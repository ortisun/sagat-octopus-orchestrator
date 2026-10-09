//! Connections killed under load: what is acknowledged is durable, what is refused leaves nothing,
//! and nothing is ever half done.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::runs::StartRun;
use sagat_domain::quota::QuotaResource;
use serde_json::json;

use crate::support::engine::{World, activity, ctx, document, task_ref, with, world};

/// Terminates every other backend of the test database over and over for `seconds`.
fn storm(w: &World, seconds: u64) -> tokio::task::JoinHandle<()> {
    let pool = w.db.pool.clone();
    tokio::spawn(async move {
        let until = std::time::Instant::now() + Duration::from_secs(seconds);
        while std::time::Instant::now() < until {
            let _ = sqlx::query(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                 WHERE datname = current_database() AND pid <> pg_backend_pid()",
            )
            .execute(&pool)
            .await;
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
}

/// Whether the failure is the kind a client retries: the request may or may not have been applied.
const fn retryable(error: &AppError) -> bool {
    matches!(error, AppError::Unavailable | AppError::Internal)
}

#[tokio::test]
async fn connection_kills_around_quota_checked_starts_never_overshoot_or_lose_an_accepted_run() {
    let w = Arc::new(world().await);
    let (wf, _) = w
        .publish(document("limited", "a", vec![activity("a")]))
        .await;
    let limit = 8;
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            limit,
            &ctx(),
        )
        .await
        .expect("limit");

    let storm = storm(&w, 3);
    let mut attempts = Vec::new();
    for i in 0..40 {
        let w = Arc::clone(&w);
        attempts.push(tokio::spawn(async move {
            // A client retries with the same Idempotency-Key until it gets an answer.
            let key = format!("fault-start-{i:03}");
            for _ in 0..200 {
                let result = w
                    .runs
                    .start(
                        &w.admin,
                        &StartRun {
                            namespace: "payments",
                            workflow: wf,
                            version: None,
                            input: b"{}",
                            correlation_id: None,
                            deadline: None,
                            key: &key,
                        },
                        &ctx(),
                    )
                    .await;
                match result {
                    Ok(record) => return (key, Ok(record.info.run_id)),
                    Err(error) if retryable(&error) => {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(error) => return (key, Err(error)),
                }
            }
            panic!("{key}: never got an answer");
        }));
    }
    let mut accepted = BTreeSet::new();
    for attempt in attempts {
        match attempt.await.expect("task") {
            (_, Ok(run)) => {
                accepted.insert(run);
            }
            (key, Err(AppError::QuotaExceeded(_))) => {
                let _ = key; // refused at the limit: the only other answer there may be
            }
            (key, Err(other)) => panic!("{key}: {other:?}"),
        }
    }
    storm.await.expect("storm");

    let stored: BTreeSet<sagat_domain::execution::RunId> =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM runs WHERE finished_at IS NULL")
            .fetch_all(&w.db.pool)
            .await
            .expect("runs")
            .into_iter()
            .map(sagat_domain::execution::RunId::new)
            .collect();
    assert_eq!(
        stored, accepted,
        "exactly the runs that were acknowledged exist, no more and no fewer"
    );
    assert_eq!(
        accepted.len(),
        usize::try_from(limit).expect("small"),
        "the limit was reached and not passed"
    );
    for run in &accepted {
        w.assert_consistent(*run).await;
    }
}

#[tokio::test]
async fn connection_kills_around_secret_writes_leave_every_version_with_its_audit_record() {
    let w = Arc::new(world().await);
    let storm = storm(&w, 3);
    let mut writers = Vec::new();
    for i in 0..24 {
        let w = Arc::clone(&w);
        writers.push(tokio::spawn(async move {
            // One secret rotated by several writers and two others, retried until acknowledged.
            let name = ["shared-key", "other-key", "third-key"][i % 3];
            for _ in 0..200 {
                match w
                    .secrets
                    .put(
                        &w.admin,
                        w.namespace.id(),
                        name,
                        format!("value-{i:03}-padding-xx").into_bytes(),
                        &ctx(),
                    )
                    .await
                {
                    Ok(_) => return true,
                    Err(error) if retryable(&error) => {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(other) => panic!("{name}: {other:?}"),
                }
            }
            false
        }));
    }
    for writer in writers {
        assert!(writer.await.expect("task"), "a writer never got an answer");
    }
    storm.await.expect("storm");

    // Every secret has its current version, and every committed version has exactly one audit
    // record: a put is one transaction, so there is no version without a record and no record
    // without a version.
    let rows: Vec<(String, i32, i64, i64)> = sqlx::query_as(
        "SELECT s.name, s.current_version, \
                (SELECT count(*) FROM secret_versions v WHERE v.secret_id = s.id AND v.version = s.current_version), \
                (SELECT count(*) FROM audit_records a WHERE a.action = 'secret.stored' \
                   AND a.resource_id = s.namespace_id::text || '/' || s.name) \
         FROM secrets s ORDER BY s.name",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("secrets");
    assert_eq!(rows.len(), 3);
    for (name, current, has_current_row, audits) in rows {
        assert_eq!(
            has_current_row, 1,
            "{name}: the current version has its row"
        );
        assert_eq!(
            i64::from(current),
            audits,
            "{name}: one audit record for each version that ever committed"
        );
    }
    let orphans: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM secret_versions v LEFT JOIN secrets s ON s.id = v.secret_id \
         WHERE s.id IS NULL",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("orphans");
    assert_eq!(orphans, 0);
}

#[tokio::test]
async fn connection_kills_around_resolutions_never_return_a_wrong_value_or_an_unaudited_one() {
    let w = Arc::new(world().await);
    w.put_secret("stripe-key", "sk-live-fault-0123456789").await;
    let (wf, _) = w
        .publish(document(
            "uses-secret",
            "a",
            vec![with(activity("a"), "secrets", json!(["stripe-key"]))],
        ))
        .await;
    w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;

    let storm = storm(&w, 3);
    let mut resolvers = Vec::new();
    for _ in 0..16 {
        let w = Arc::clone(&w);
        let task = task.clone();
        let principal = session.principal.clone();
        resolvers.push(tokio::spawn(async move {
            let mut answered = 0u32;
            for _ in 0..40 {
                match w
                    .workers
                    .resolve_secrets(&principal, &task_ref(&task), None)
                    .await
                {
                    Ok(secrets) => {
                        assert_eq!(secrets.len(), 1);
                        assert_eq!(
                            secrets[0].value.expose(),
                            b"sk-live-fault-0123456789",
                            "never a wrong or partial value"
                        );
                        answered += 1;
                    }
                    Err(error) if retryable(&error) => {
                        tokio::time::sleep(Duration::from_millis(15)).await;
                    }
                    Err(other) => panic!("{other:?}"),
                }
            }
            answered
        }));
    }
    let mut answered = 0u32;
    for resolver in resolvers {
        answered += resolver.await.expect("task");
    }
    storm.await.expect("storm");

    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = 'secret.resolved'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert!(answered > 0, "the storm let some resolutions through");
    // A resolution and its audit record commit together, and asking again within one attempt is
    // written down once: however many times the value was handed out (and however many commits had
    // their acknowledgement lost), the attempt has exactly one record, never none.
    assert_eq!(
        audited, 1,
        "{answered} values were handed out for one attempt and {audited} records were written"
    );
}

#[tokio::test]
async fn connection_kills_while_a_binding_is_replaced_never_leave_a_session_it_does_not_cover() {
    let w = Arc::new(world().await);
    let ns = w.namespace.id();
    let who = sagat_domain::identity::Principal::new(
        w.admin.tenant_id(),
        crate::support::subject("churn-worker"),
        sagat_domain::identity::Role::Worker,
    );
    let storm = storm(&w, 3);
    for round in 0..30 {
        // Alternate between a wide and a narrow binding while the worker keeps registering.
        let activities = if round % 2 == 0 { "*" } else { "payments.*" };
        for _ in 0..100 {
            match w
                .bindings
                .bind(
                    &w.admin,
                    ns,
                    "churn-worker",
                    &[activities.to_owned()],
                    &ctx(),
                )
                .await
            {
                Ok(_) => break,
                Err(error) if retryable(&error) => {
                    tokio::time::sleep(Duration::from_millis(15)).await;
                }
                Err(other) => panic!("{other:?}"),
            }
        }
        let caps = sagat_application::worker::Capabilities {
            activities: vec!["audit.log".into()],
            languages: vec![],
            pools: vec![],
            max_inflight: 4,
            protocol_version: "1".into(),
        };
        let _ = w.workers.register(&who, "payments", "churn", &caps).await;
    }
    storm.await.expect("storm");
    // Settle on the narrow binding, retried until it commits.
    for _ in 0..100 {
        if w.bindings
            .bind(
                &w.admin,
                ns,
                "churn-worker",
                &["payments.*".to_owned()],
                &ctx(),
            )
            .await
            .is_ok()
        {
            break;
        }
    }
    // Whatever order things happened in, a session that survives is covered by the binding in force:
    // `audit.log` is not under `payments.*`, so none may exist.
    let uncovered: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM worker_sessions WHERE subject = 'churn-worker' \
         AND 'audit.log' = ANY(activities)",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(uncovered, 0);
}

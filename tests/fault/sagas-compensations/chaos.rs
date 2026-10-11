//! Database connections killed under load while runs succeed and compensate: whatever is cut off,
//! every run ends in the state its history demands, every effect has happened once, and every
//! journal is complete (NFR-003-DUR).

use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::worker::WorkerSettings;
use sagat_domain::execution::RunStatus;
use serde_json::{Value, json};

use crate::common::{ExternalSystem, quick, settlement};
use crate::support::engine::{Session, World, effect_activities, world_with};

#[tokio::test]
async fn connection_kills_while_sagas_succeed_and_compensate_lose_nothing_and_duplicate_nothing() {
    // Short leases, so a task whose grant was committed but never delivered is recovered quickly.
    let w = world_with(WorkerSettings {
        lease_ttl: Duration::from_secs(1),
        ..WorkerSettings::default()
    })
    .await;
    let (wf, _) = w.publish(quick(settlement())).await;
    let total = 20usize;
    let mut runs = Vec::new();
    for i in 0..total {
        runs.push(w.start(wf, &json!({"i": i})).await);
    }
    let provider = ExternalSystem::default();

    // For three seconds, terminate every other backend over and over.
    let storm = {
        let pool = w.db.pool.clone();
        tokio::spawn(async move {
            let until = std::time::Instant::now() + Duration::from_secs(3);
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
    };

    let mut sessions = Vec::new();
    for i in 0..4 {
        sessions.push(register_with_retry(&w, i).await);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let done = futures_util::future::join_all(
        sessions
            .iter()
            .map(|s| worker_loop(&w, s, &provider, deadline)),
    )
    .await;
    storm.await.expect("storm");
    assert!(
        done.iter().all(|d| *d),
        "a worker gave up before the work was done"
    );

    // Even runs paid; odd runs were declined and released the limit.
    let mut paid = 0;
    let mut released = 0;
    for (i, run) in runs.iter().enumerate() {
        let status = w.run_status(*run).await;
        if i % 2 == 0 {
            assert_eq!(status, RunStatus::Succeeded, "run {i}");
            paid += 1;
        } else {
            assert_eq!(status, RunStatus::Compensated, "run {i}");
            released += 1;
        }
        w.assert_consistent(*run).await;
        let journal = w.journal(*run).await;
        assert_eq!(
            journal.iter().filter(|e| *e == "run.finished").count(),
            1,
            "run {i}"
        );
        let compensations = journal
            .iter()
            .filter(|e| *e == "compensation.planned")
            .count();
        assert_eq!(
            compensations,
            usize::from(i % 2 == 1),
            "a saga is planned once, only when it failed"
        );
    }
    assert_eq!((paid, released), (total / 2, total / 2));

    // At-least-once delivery, exactly-once effects: every effect is keyed, so none was duplicated.
    assert_eq!(
        provider.effects_with_prefix("pay:"),
        total / 2,
        "one payment per paying run"
    );
    assert_eq!(
        provider.effects_with_prefix("reserve:"),
        total,
        "one reservation per run"
    );
    assert_eq!(
        provider.effects_with_prefix("compensation:"),
        total / 2,
        "one release per declined run"
    );
}

/// Works for `session` until every run is over, retrying every call that fails because the
/// database was killed under it. Returns false if the deadline passes first.
async fn worker_loop(
    w: &World,
    session: &Session,
    provider: &ExternalSystem,
    deadline: std::time::Instant,
) -> bool {
    loop {
        if std::time::Instant::now() > deadline {
            return false;
        }
        w.scheduler.tick().await;
        let tasks = match w
            .workers
            .poll(
                &session.principal,
                session.worker_id,
                &session.token,
                4,
                None,
            )
            .await
        {
            Ok(tasks) => tasks,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
        };
        if tasks.is_empty() {
            let open: Result<i64, _> =
                sqlx::query_scalar("SELECT count(*) FROM runs WHERE finished_at IS NULL")
                    .fetch_one(&w.db.pool)
                    .await;
            if matches!(open, Ok(0)) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
            continue;
        }
        for task in tasks {
            let input: Value = serde_json::from_slice(&task.input).unwrap_or(Value::Null);
            let declined = input["input"]["i"].as_u64().is_some_and(|i| i % 2 == 1);
            let activity = task.activity.as_str().to_owned();
            let key = task.idempotency_key.clone();
            match activity.as_str() {
                "act.reserve" => {
                    provider.perform(&format!("reserve:{key}"));
                }
                "act.submit" if !declined => {
                    provider.perform(&format!("pay:{key}"));
                }
                "undo.reserve" => {
                    provider.perform(&key);
                }
                _ => {}
            }
            let fails = activity == "act.submit" && declined;
            // Report until the platform acknowledges (a replay counts as acknowledgement).
            for _ in 0..200 {
                let reported = if fails {
                    w.fail(session, &task, false).await
                } else {
                    w.complete(session, &task, &json!({"ok": true})).await
                };
                match reported {
                    Ok(_) | Err(AppError::StaleAttempt) => break,
                    Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        }
    }
}

async fn register_with_retry(w: &World, index: u32) -> Session {
    let mut names = effect_activities(&["reserve", "submit"]);
    names.push("act.notify".to_owned());
    for _ in 0..100 {
        let caps = sagat_application::worker::Capabilities {
            activities: names.clone(),
            languages: vec![],
            pools: vec![],
            max_inflight: 50,
            protocol_version: "1".into(),
        };
        if let Ok(registered) = w
            .workers
            .register(
                &w.worker,
                "payments",
                &format!("chaos-worker-{index}"),
                &caps,
            )
            .await
        {
            return Session {
                principal: w.worker.clone(),
                worker_id: registered.worker_id,
                token: registered.session_token.expose().to_owned(),
            };
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("could not register a worker");
}

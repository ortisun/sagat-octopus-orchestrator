//! Ordinary workers must release durable run locks during provider I/O.
use super::world;
use super::{
    ReadBarrier,
    support::engine::{activity, ctx, document, task_ref, with},
};
use sagat_application::{
    clock::SystemClock,
    error::AppError,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    runs::RunOperation,
    secret_provider::{ProviderError, ProviderFuture, ProviderReference, SecretProvider},
};
use sagat_domain::secrets::SecretValue;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

pub(super) struct Down(pub Arc<dyn SecretProvider>);
impl SecretProvider for Down {
    fn create<'a>(
        &'a self,
        r: &'a ProviderReference,
        v: &'a SecretValue,
    ) -> ProviderFuture<'a, ()> {
        self.0.create(r, v)
    }
    fn read<'a>(&'a self, _: &'a ProviderReference) -> ProviderFuture<'a, Option<SecretValue>> {
        Box::pin(async { Err(ProviderError::Unavailable) })
    }
    fn delete<'a>(&'a self, r: &'a ProviderReference) -> ProviderFuture<'a, ()> {
        self.0.delete(r)
    }
}

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let mut w = world().await;
    ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap()
    .put(
        &w.admin,
        w.namespace.id(),
        ManagedSecretWrite {
            name: "ordinary-secret",
            expected_version: 0,
            request_id: Uuid::new_v4(),
            value: SecretValue::new(b"ordinary-secret-canary".to_vec()).unwrap(),
        },
        &ctx(),
    )
    .await
    .unwrap();
    let (workflow, _) = w
        .publish(document(
            "ordinary-provider",
            "a",
            vec![with(activity("a"), "secrets", json!(["ordinary-secret"]))],
        ))
        .await;
    let session = w.session(&["act.a"]).await;
    for operation in ["resolve", "complete", "fail"] {
        let run = w.start(workflow, &json!({})).await;
        let task = w.one_task(&session).await;
        let reference = task_ref(&task);
        let gated = Arc::new(ReadBarrier {
            inner: provider.clone(),
            entered: tokio::sync::Barrier::new(2),
            resume: tokio::sync::Barrier::new(2),
        });
        w.workers = w.workers.with_secret_provider(gated.clone());
        let reading = async {
            match operation {
                "resolve" => w
                    .workers
                    .resolve_secrets(&session.principal, &reference, None)
                    .await
                    .map(|_| ()),
                "complete" => w
                    .workers
                    .complete(&session.principal, &reference, b"{}", "cancelled-complete")
                    .await
                    .map(|_| ()),
                _ => w
                    .workers
                    .fail(
                        &session.principal,
                        &reference,
                        "SAFE_ERROR",
                        false,
                        "cancelled-fail",
                    )
                    .await
                    .map(|_| ()),
            }
        };
        let cancelling = async {
            gated.entered.wait().await;
            tokio::time::timeout(
                Duration::from_secs(1),
                w.workers.heartbeat(&session.principal, &reference),
            )
            .await
            .expect("provider read must not hold the run lock")
            .unwrap();
            tokio::time::timeout(Duration::from_secs(1), w.operate(run, RunOperation::Cancel))
                .await
                .expect("cancellation must commit during provider I/O")
                .unwrap();
            gated.resume.wait().await;
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(reading, cancelling)
        })
        .await
        .unwrap();
        assert!(
            matches!(result, Err(AppError::StaleAttempt)),
            "{operation}: {result:?}"
        );
        let output: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT output_digest FROM steps WHERE run_id=$1 AND node='a'")
                .bind(run.as_uuid())
                .fetch_one(&w.db.pool)
                .await
                .unwrap();
        assert!(output.is_none());
        w.age_lease(run, "a").await;
        w.scheduler.tick().await;
    }
    // An authenticated Principal is only a snapshot. Demotion during the provider read must
    // commit immediately and invalidate the final capability even while the lease is live.
    w.workers = w.workers.with_secret_provider(provider.clone());
    let demoted_run = w.start(workflow, &json!({})).await;
    let demoted_task = w.one_task(&session).await;
    let reference = task_ref(&demoted_task);
    let gated = Arc::new(ReadBarrier {
        inner: provider.clone(),
        entered: tokio::sync::Barrier::new(2),
        resume: tokio::sync::Barrier::new(2),
    });
    w.workers = w.workers.with_secret_provider(gated.clone());
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            w.workers
                .resolve_secrets(&session.principal, &reference, None),
            async {
                gated.entered.wait().await;
                tokio::time::timeout(
                    Duration::from_secs(1),
                    sqlx::query(
                        "UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2",
                    )
                    .bind(session.principal.tenant_id().as_uuid())
                    .bind(session.principal.subject().as_str())
                    .execute(&w.db.pool),
                )
                .await
                .unwrap()
                .unwrap();
                gated.resume.wait().await;
            }
        )
    })
    .await
    .unwrap();
    assert!(
        matches!(result, Err(AppError::Forbidden { .. })),
        "demotion invalidates secret delivery"
    );
    sqlx::query("UPDATE memberships SET role='worker' WHERE tenant_id=$1 AND subject=$2")
        .bind(session.principal.tenant_id().as_uuid())
        .bind(session.principal.subject().as_str())
        .execute(&w.db.pool)
        .await
        .unwrap();
    w.operate(demoted_run, RunOperation::Cancel).await.unwrap();
    w.age_lease(demoted_run, "a").await;
    w.scheduler.tick().await;
    for operation in ["complete", "fail"] {
        w.workers = w.workers.with_secret_provider(provider.clone());
        w.start(workflow, &json!({})).await;
        let task = w.one_task(&session).await;
        let reference = task_ref(&task);
        let report = async |workers: &super::support::engine::Workers| {
            if operation == "complete" {
                workers
                    .complete(&session.principal, &reference, b"{}", "provider-replay")
                    .await
            } else {
                workers
                    .fail(
                        &session.principal,
                        &reference,
                        "SAFE_ERROR",
                        false,
                        "provider-replay",
                    )
                    .await
            }
        };
        assert!(!report(&w.workers).await.unwrap().already_applied);
        w.workers = w
            .workers
            .with_secret_provider(Arc::new(Down(provider.clone())));
        assert!(
            report(&w.workers).await.unwrap().already_applied,
            "durable {operation} receipt bypasses provider outage"
        );
    }
    // A rotation between scan and commit must require a fresh scan of the retained versions.
    w.start(workflow, &json!({})).await;
    let task = w.one_task(&session).await;
    let reference = task_ref(&task);
    let gated = Arc::new(ReadBarrier {
        inner: provider.clone(),
        entered: tokio::sync::Barrier::new(2),
        resume: tokio::sync::Barrier::new(2),
    });
    w.workers = w.workers.with_secret_provider(gated.clone());
    let scanning = w
        .workers
        .complete(&session.principal, &reference, b"{}", "rotation-race");
    let rotation = async {
        gated.entered.wait().await;
        ManagedSecretService::new(
            w.store.clone(),
            SystemClock,
            provider.clone(),
            SecretValue::new(vec![42; 32]).unwrap(),
        )
        .unwrap()
        .put(
            &w.admin,
            w.namespace.id(),
            ManagedSecretWrite {
                name: "ordinary-secret",
                expected_version: 1,
                request_id: Uuid::new_v4(),
                value: SecretValue::new(b"ordinary-rotated-canary".to_vec()).unwrap(),
            },
            &ctx(),
        )
        .await
        .unwrap();
        gated.resume.wait().await;
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(scanning, rotation)
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(AppError::Unavailable)));
    w.workers = w.workers.with_secret_provider(provider.clone());
    assert!(
        !w.workers
            .complete(&session.principal, &reference, b"{}", "rotation-race")
            .await
            .unwrap()
            .already_applied
    );
    derived_during_cancellation(provider).await;
}

async fn derived_during_cancellation(provider: Arc<dyn SecretProvider>) {
    use super::support::engine::{effect_node, saga_node};
    let mut w = world().await;
    ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap()
    .put(
        &w.admin,
        w.namespace.id(),
        ManagedSecretWrite {
            name: "saga-secret",
            expected_version: 0,
            request_id: Uuid::new_v4(),
            value: SecretValue::new(b"saga-secret-canary".to_vec()).unwrap(),
        },
        &ctx(),
    )
    .await
    .unwrap();
    w.workers = w.workers.with_secret_provider(provider);
    let (workflow, _) = w
        .publish(document(
            "secret-recovery",
            "saga",
            vec![
                saga_node("saga", "a", None),
                with(
                    effect_node("a", Some("b"), true),
                    "secrets",
                    json!(["saga-secret"]),
                ),
                effect_node("b", None, false),
            ],
        ))
        .await;
    let session = w
        .session(&["act.a", "act.b", "undo.a", "undo.b", "lookup.a"])
        .await;
    for reconcile in [false, true] {
        let run = w.start(workflow, &json!({})).await;
        let first = w.one_task(&session).await;
        if reconcile {
            w.operate_run(run, RunOperation::Cancel).await;
            w.age_lease(run, "a").await;
            w.scheduler.tick().await;
        } else {
            w.complete(&session, &first, &json!({})).await.unwrap();
            let second = w.one_task(&session).await;
            w.operate_run(run, RunOperation::Cancel).await;
            w.fail_with(&session, &second, "CANCELLED", true)
                .await
                .unwrap();
        }
        let recovery = w.one_task(&session).await;
        assert_eq!(
            recovery.activity.as_str(),
            if reconcile { "lookup.a" } else { "undo.a" }
        );
        let resolved = w
            .workers
            .resolve_secrets(&session.principal, &task_ref(&recovery), None)
            .await
            .unwrap();
        assert_eq!(resolved[0].value.expose(), b"saga-secret-canary");
        if reconcile {
            w.answer_reconciliation(&session, &recovery, "not_applied", None)
                .await
                .unwrap();
        } else {
            w.complete(&session, &recovery, &json!({})).await.unwrap();
        }
        assert_eq!(w.run_status(run).await.as_str(), "CANCELLED");
        w.assert_consistent(run).await;
    }
}

//! The transactional outbox: notifications are written with the change they describe, delivered at
//! least once, and never required for correctness (FR-002-003, FR-002-006, NFR-002-DUR).

use std::collections::BTreeSet;
use std::time::Duration;

use sagat_application::maintenance::OutboxStore;
use serde_json::{Value, json};

use crate::support::engine::{chain, world};

#[tokio::test]
async fn every_ready_step_and_every_finished_run_is_announced_in_the_same_transaction() {
    let w = world().await;
    let (wf, _) = w.publish(chain("announced")).await;
    let run = w.start(wf, &json!({})).await;
    assert_eq!(w.outbox("task.ready").await, 1, "a is ready");

    let session = w.session(&["act.a", "act.b"]).await;
    let a = w.one_task(&session).await;
    w.complete(&session, &a, &json!({})).await.expect("a");
    assert_eq!(w.outbox("task.ready").await, 2, "b became ready");
    let b = w.one_task(&session).await;
    w.complete(&session, &b, &json!({})).await.expect("b");
    assert_eq!(w.outbox("run.finished").await, 1);

    let hint: Value = sqlx::query_scalar("SELECT payload FROM event_outbox WHERE event_type = 'task.ready' ORDER BY created_at LIMIT 1")
        .fetch_one(&w.db.pool)
        .await
        .expect("payload");
    assert_eq!(hint["runId"], run.to_string());
    assert_eq!(hint["node"], "a");
    assert_eq!(hint["activity"], "act.a");
    assert_eq!(hint["tenantId"], w.admin.tenant_id().to_string());
    assert!(
        hint.get("input").is_none() && hint.get("output").is_none(),
        "a hint carries identifiers, never payloads"
    );
    let finished: Value =
        sqlx::query_scalar("SELECT payload FROM event_outbox WHERE event_type = 'run.finished'")
            .fetch_one(&w.db.pool)
            .await
            .expect("payload");
    assert_eq!(finished["status"], "SUCCEEDED");
}

#[tokio::test]
async fn a_claimed_message_reappears_after_the_visibility_timeout_until_it_is_confirmed() {
    let w = world().await;
    let (wf, _) = w.publish(chain("at-least-once")).await;
    w.start(wf, &json!({})).await;
    let visibility = Duration::from_millis(400);

    let first = w.store.claim_outbox(10, visibility).await.expect("claim");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].attempt_count, 1);
    assert!(
        w.store
            .claim_outbox(10, visibility)
            .await
            .expect("claim")
            .is_empty(),
        "invisible while claimed"
    );

    // The dispatcher crashed after publishing and before confirming: the message comes back.
    tokio::time::sleep(visibility + Duration::from_millis(200)).await;
    let again = w.store.claim_outbox(10, visibility).await.expect("claim");
    assert_eq!(again.len(), 1);
    assert_eq!(
        again[0].id, first[0].id,
        "the same message is delivered again"
    );
    assert_eq!(again[0].attempt_count, 2);

    w.store
        .mark_delivered(&[again[0].id])
        .await
        .expect("confirm");
    tokio::time::sleep(visibility + Duration::from_millis(200)).await;
    assert!(
        w.store
            .claim_outbox(10, visibility)
            .await
            .expect("claim")
            .is_empty(),
        "confirmed messages never return"
    );
    assert_eq!(w.backlog().await, 0);
}

#[tokio::test]
async fn failures_are_recorded_and_retried_later() {
    let w = world().await;
    let (wf, _) = w.publish(chain("failing-broker")).await;
    w.start(wf, &json!({})).await;
    let message = w
        .store
        .claim_outbox(1, Duration::from_secs(60))
        .await
        .expect("claim")
        .remove(0);
    w.store
        .mark_failed(message.id, "BROKER_DOWN", Duration::from_millis(300))
        .await
        .expect("fail");
    assert!(
        w.store
            .claim_outbox(1, Duration::from_secs(60))
            .await
            .expect("claim")
            .is_empty()
    );
    let code: Option<String> =
        sqlx::query_scalar("SELECT last_error_code FROM event_outbox WHERE id = $1")
            .bind(message.id)
            .fetch_one(&w.db.pool)
            .await
            .expect("code");
    assert_eq!(code.as_deref(), Some("BROKER_DOWN"));
    assert_eq!(w.backlog().await, 1, "still owed");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        w.store
            .claim_outbox(1, Duration::from_secs(60))
            .await
            .expect("claim")
            .len(),
        1
    );
}

#[tokio::test]
async fn concurrent_dispatchers_claim_disjoint_messages() {
    let w = world().await;
    let (wf, _) = w.publish(chain("many-hints")).await;
    for i in 0..40 {
        w.start(wf, &json!({"i": i})).await;
    }
    let claims = futures_util::future::join_all(
        (0..8).map(|_| w.store.claim_outbox(10, Duration::from_secs(60))),
    )
    .await;
    let ids: Vec<_> = claims
        .into_iter()
        .flat_map(|c| c.expect("claim"))
        .map(|m| m.id)
        .collect();
    let unique: BTreeSet<_> = ids.iter().collect();
    assert_eq!(ids.len(), unique.len(), "a message went to two dispatchers");
    assert_eq!(ids.len(), 40, "all messages were claimed, none twice");
}

#[tokio::test]
async fn messages_come_out_in_the_order_they_were_written() {
    let w = world().await;
    let (wf, _) = w.publish(chain("ordered-hints")).await;
    let mut runs = Vec::new();
    for _ in 0..6 {
        runs.push(w.start(wf, &json!({})).await);
    }
    let claimed = w
        .store
        .claim_outbox(20, Duration::from_secs(60))
        .await
        .expect("claim");
    let order: Vec<_> = claimed.iter().map(|m| m.aggregate_id).collect();
    let expected: Vec<_> = runs.iter().map(|r| r.as_uuid()).collect();
    assert_eq!(order, expected);
}

// ------------------------------------------------------------------------------ dispatching

mod dispatching {
    use std::sync::Arc;
    use std::time::Duration;

    use sagat_application::entropy::SystemEntropy;
    use sagat_application::error::StoreError;
    use sagat_application::maintenance::{MaintenanceStore, OutboxMessage, OutboxStore};
    use sagat_infra_postgres::{PgRunStore, PoolConfig, connect};
    use sagat_scheduler::{DispatchSettings, OutboxDispatcher};
    use serde_json::{Value, json};
    use uuid::Uuid;

    use crate::support::engine::{RecordingBroker, World, chain, world};
    use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD};

    fn dispatcher<S: OutboxStore>(
        store: S,
        broker: &RecordingBroker,
    ) -> OutboxDispatcher<S, RecordingBroker, SystemEntropy> {
        OutboxDispatcher::new(
            store,
            broker.clone(),
            SystemEntropy,
            DispatchSettings {
                visibility: Duration::from_millis(300),
                base_backoff: Duration::from_millis(100),
                max_backoff: Duration::from_millis(400),
                ..DispatchSettings::default()
            },
        )
    }

    async fn pending(w: &World) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE delivered_at IS NULL")
            .fetch_one(&w.db.pool)
            .await
            .expect("count")
    }

    #[tokio::test]
    async fn a_hint_reaches_the_broker_once_on_its_tenant_subject_with_a_stable_id() {
        let w = world().await;
        let (wf, _) = w.publish(chain("to-the-broker")).await;
        let run = w.start(wf, &json!({})).await;
        let broker = RecordingBroker::default();
        let dispatcher = dispatcher(w.store.clone(), &broker);

        let round = dispatcher.run_once().await;
        assert_eq!((round.published, round.failed), (1, 0));
        let sent = broker.published();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].subject,
            format!("sagat.v1.task.ready.{}.act.a", w.admin.tenant_id()),
            "per tenant and activity"
        );
        let id: String = sqlx::query_scalar("SELECT id::text FROM event_outbox LIMIT 1")
            .fetch_one(&w.db.pool)
            .await
            .expect("id");
        assert_eq!(sent[0].message_id, id, "the outbox id is the dedupe id");
        let payload: Value = serde_json::from_slice(&sent[0].payload).expect("json");
        assert_eq!(payload["runId"], run.to_string());
        assert_eq!(pending(&w).await, 0);
        assert!(!dispatcher.run_once().await.busy(), "nothing left");
        assert_eq!(broker.published().len(), 1, "never published twice");
    }

    #[tokio::test]
    async fn a_broker_outage_loses_nothing_and_publishing_resumes_with_backoff() {
        let w = world().await;
        let (wf, _) = w.publish(chain("outage")).await;
        w.start(wf, &json!({})).await;
        let broker = RecordingBroker::default();
        // A backoff long enough that the queries between the two rounds, on a slow machine, are well inside
        // it (the delay is at least half of the base): the test is about waiting, not about how fast a
        // database answers.
        let dispatcher = OutboxDispatcher::new(
            w.store.clone(),
            broker.clone(),
            SystemEntropy,
            DispatchSettings {
                visibility: Duration::from_millis(300),
                base_backoff: Duration::from_millis(1_500),
                max_backoff: Duration::from_millis(1_500),
                ..DispatchSettings::default()
            },
        );

        broker.set_down(true);
        let round = dispatcher.run_once().await;
        assert_eq!((round.published, round.failed), (0, 1));
        assert_eq!(pending(&w).await, 1, "the message is still owed");
        let code: Option<String> =
            sqlx::query_scalar("SELECT last_error_code FROM event_outbox LIMIT 1")
                .fetch_one(&w.db.pool)
                .await
                .expect("code");
        assert_eq!(code.as_deref(), Some("BROKER_UNAVAILABLE"));
        assert!(
            !dispatcher.run_once().await.busy(),
            "a failed message waits for its backoff instead of being hammered"
        );

        broker.set_down(false);
        tokio::time::sleep(Duration::from_millis(1_700)).await;
        let round = dispatcher.run_once().await;
        assert_eq!((round.published, round.failed), (1, 0));
        assert_eq!(pending(&w).await, 0);
        assert_eq!(broker.published().len(), 1);
    }

    /// Fails the first confirmation, as a dispatcher that crashes after publishing would.
    struct LosesConfirmation {
        inner: PgRunStore,
        armed: std::sync::atomic::AtomicBool,
    }

    impl OutboxStore for LosesConfirmation {
        async fn claim_outbox(
            &self,
            limit: u32,
            visibility: Duration,
        ) -> Result<Vec<OutboxMessage>, StoreError> {
            self.inner.claim_outbox(limit, visibility).await
        }

        async fn mark_delivered(&self, ids: &[Uuid]) -> Result<(), StoreError> {
            if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
                return Err(StoreError::Unavailable {
                    detail: "the confirmation was lost".to_owned(),
                });
            }
            self.inner.mark_delivered(ids).await
        }

        async fn mark_failed(
            &self,
            id: Uuid,
            code: &str,
            retry_in: Duration,
        ) -> Result<(), StoreError> {
            self.inner.mark_failed(id, code, retry_in).await
        }

        async fn outbox_backlog(&self) -> Result<u64, StoreError> {
            self.inner.outbox_backlog().await
        }
    }

    #[tokio::test]
    async fn a_crash_between_publishing_and_confirming_republishes_with_the_same_id() {
        let w = world().await;
        let (wf, _) = w.publish(chain("crash-after-publish")).await;
        w.start(wf, &json!({})).await;
        let broker = RecordingBroker::default();
        let dispatcher = dispatcher(
            LosesConfirmation {
                inner: w.store.clone(),
                armed: true.into(),
            },
            &broker,
        );

        dispatcher.run_once().await;
        assert_eq!(
            broker.published().len(),
            1,
            "published, but never confirmed"
        );
        assert_eq!(pending(&w).await, 1);

        tokio::time::sleep(Duration::from_millis(400)).await; // the visibility timeout
        dispatcher.run_once().await;
        let sent = broker.published();
        assert_eq!(sent.len(), 2, "at-least-once: a duplicate, never a loss");
        assert_eq!(
            sent[0].message_id, sent[1].message_id,
            "the broker can drop the duplicate by its id"
        );
        assert_eq!(pending(&w).await, 0);
    }

    #[tokio::test]
    async fn a_message_that_can_never_be_published_stays_visible_instead_of_vanishing() {
        let w = world().await;
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
             VALUES (gen_random_uuid(), $1, gen_random_uuid(), 'Bad Type*', '{}')",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("insert");
        let broker = RecordingBroker::default();
        let round = dispatcher(w.store.clone(), &broker).run_once().await;
        assert_eq!((round.published, round.failed), (0, 1));
        assert!(broker.published().is_empty(), "nothing reached the broker");
        assert_eq!(pending(&w).await, 1, "still owed, and countable");
        let code: Option<String> =
            sqlx::query_scalar("SELECT last_error_code FROM event_outbox LIMIT 1")
                .fetch_one(&w.db.pool)
                .await
                .expect("code");
        assert_eq!(code.as_deref(), Some("REJECTED"));
    }

    #[tokio::test]
    async fn only_delivered_messages_are_purged_and_the_runtime_role_may_do_it() {
        let w = world().await;
        let (wf, _) = w.publish(chain("retention")).await;
        w.start(wf, &json!({})).await;
        w.start(wf, &json!({})).await;
        let claimed = w
            .store
            .claim_outbox(1, Duration::from_secs(60))
            .await
            .expect("claim");
        w.store
            .mark_delivered(&[claimed[0].id])
            .await
            .expect("delivered");

        // Within the retention nothing goes.
        assert_eq!(
            w.store
                .purge_outbox(Duration::from_secs(3600), 100)
                .await
                .expect("purge"),
            0
        );
        sqlx::query("UPDATE event_outbox SET delivered_at = clock_timestamp() - interval '2 hours' WHERE delivered_at IS NOT NULL")
            .execute(&w.db.pool)
            .await
            .expect("age");

        // The least-privilege role the engine runs as can purge, and only delivered rows.
        let url = w.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD);
        let pool = connect(&PoolConfig::new(url)).await.expect("runtime pool");
        let runtime = PgRunStore::new(
            pool,
            sagat_application::execution::DefinitionCache::new(
                Arc::new(sagat_infra_ir::JsonIrCodec),
                8,
            ),
        );
        assert_eq!(
            runtime
                .purge_outbox(Duration::from_secs(3600), 100)
                .await
                .expect("purge as the runtime role"),
            1
        );
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM event_outbox")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
        assert_eq!(remaining, 1, "the undelivered message is untouched");
    }
}

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

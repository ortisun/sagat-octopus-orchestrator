//! The namespace ceilings at readiness (G1199; FR-022-047, FR-022-054): the live-keys ceiling refuses a step key that
//! would be a new key beyond it (`FLOW_CONTROL_LIMIT`, through the failure route, no attempt spent, never granted), and
//! the namespace's `concurrency_limit` and `flow_control_queue_timeout_ms` bound what a declaration asks. Rests on the
//! readiness gate as built (ADR-0094, proposed, O1171 pending).

use sagat_domain::quota::QuotaResource;
use serde_json::{Value, json};

use super::readiness::{granted, queued, rows, step};
use crate::support::engine::{World, activity, ctx, document, with};
use crate::support::flow_control::{gated_world, publish};

fn guarded(node: Value, limit: u32) -> Value {
    with(
        with(
            node,
            "concurrency",
            json!({"key": "input.accountId", "limit": limit}),
        ),
        "onError",
        json!("recover"),
    )
}

async fn set(w: &World, resource: QuotaResource, value: u64) {
    w.quotas
        .set(&w.admin, Some(w.namespace.id()), resource, value, &ctx())
        .await
        .unwrap_or_else(|error| panic!("{resource}: {error:?}"));
}

async fn failure(w: &World, run: sagat_domain::execution::RunId) -> (String, Option<String>, i32) {
    sqlx::query_as(
        "SELECT status, failure_code, attempt FROM steps WHERE run_id = $1 AND node = 'g'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("step")
}

/// FR-022-054: a key beyond the namespace's live keys fails its step `FLOW_CONTROL_LIMIT` through its route; a key that
/// lives already is decided as ever; a ceiling of zero refuses every new key.
#[tokio::test]
async fn a_new_step_key_beyond_the_live_keys_ceiling_fails_flow_control_limit() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "live-keys",
            "g",
            vec![guarded(activity("g"), 1), activity("recover")],
        ),
    )
    .await;
    set(&w, QuotaResource::ConcurrencyLiveKeys, 2).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second = w.start(workflow, &json!({"accountId": "acct-2"})).await;
    granted(&w, first, "g", (true, false)).await;
    granted(&w, second, "g", (true, false)).await;

    let third = w.start(workflow, &json!({"accountId": "acct-3"})).await;
    assert_eq!(
        failure(&w, third).await,
        (
            "FAILED".to_owned(),
            Some("FLOW_CONTROL_LIMIT".to_owned()),
            0
        ),
        "refused, no attempt spent"
    );
    assert_eq!(
        rows(&w, third, "g").await,
        (0, 0, 0, 0),
        "never granted nor queued"
    );
    assert_eq!(
        step(&w, third, "recover").await.0,
        "READY",
        "the failure route takes it"
    );
    w.assert_consistent(third).await;
    let keys: i64 =
        sqlx::query_scalar("SELECT count(*) FROM concurrency_semaphores WHERE namespace_id = $1")
            .bind(w.namespace.id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("keys");
    assert_eq!(keys, 2, "no key row beyond the ceiling");

    // a key that lives is not a new key
    let again = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    queued(&w, again, "g", (true, false)).await;

    set(&w, QuotaResource::ConcurrencyLiveKeys, 0).await;
    let refused = w.start(workflow, &json!({"accountId": "acct-4"})).await;
    assert_eq!(
        failure(&w, refused).await.1.as_deref(),
        Some("FLOW_CONTROL_LIMIT")
    );
    w.assert_consistent(refused).await;
}

/// FR-022-047: the effective limit and queue timeout of a step are the lower of the declaration and the namespace.
#[tokio::test]
async fn the_namespace_limit_and_queue_timeout_bound_a_declaration() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "namespace-bounds",
            "g",
            vec![guarded(activity("g"), 5), activity("recover")],
        ),
    )
    .await;
    set(&w, QuotaResource::ConcurrencyLimit, 1).await;
    set(&w, QuotaResource::FlowControlQueueTimeoutMs, 60_000).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, holder, "g", (true, false)).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    queued(&w, waiter, "g", (true, false)).await;
    let (limit, waits): (i32, f64) = sqlx::query_as(
        "SELECT declared_limit, extract(epoch FROM deadline_at - enqueued_at)::float8 \
           FROM concurrency_waiters WHERE run_id = $1",
    )
    .bind(waiter.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("waiter");
    assert_eq!(limit, 1, "the namespace's limit, not the declared 5");
    assert!(
        (59.0..=60.5).contains(&waits),
        "the namespace's queue timeout, not the declared ten minutes: {waits}"
    );
}

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

/// OD-2026-11-19-4 and OD-2026-11-20-3 (the owner accepted the bound, G022-002, D022-047): the concurrency live-keys
/// count reads the committed key rows without a namespace lock, so it misses only the keys other transactions of the
/// same moment have created and not committed (the twin of
/// `rate_gate::the_rate_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed`). With the ceiling at
/// two, one key committed and one created by a transaction still in flight, a readiness round creates a second key (it
/// counts one); once both commit the ceiling is passed by exactly the other transaction's key, and the next new key,
/// which counts every committed row, is refused.
#[tokio::test]
async fn the_concurrency_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed()
 {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document(
            "live-keys-overshoot",
            "g",
            vec![guarded(activity("g"), 1), activity("recover")],
        ),
    )
    .await;
    set(&w, QuotaResource::ConcurrencyLiveKeys, 2).await;
    let keys = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM concurrency_semaphores WHERE namespace_id = $1",
        )
        .bind(w.namespace.id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("keys")
    };
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, first, "g", (true, false)).await;
    assert_eq!(keys().await, 1);
    // another transaction of the same moment has created a key of the namespace and not committed
    let stand_in = vec![0x42_u8; 32];
    let mut other = w.db.pool.begin().await.expect("a transaction");
    sqlx::query(
        "INSERT INTO concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, \
                                             key_version, key_shard, updated_at) \
         SELECT $1, $2, 'ORDINARY', 'STEP', 'provider', $3, active, 0, clock_timestamp() \
           FROM flow_control_key_versions_in_force()",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(&stand_in)
    .execute(&mut *other)
    .await
    .expect("a key in flight");
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        w.start(workflow, &json!({"accountId": "acct-2"})),
    )
    .await
    .expect("the round takes no namespace lock");
    granted(&w, second, "g", (true, false)).await;
    other.commit().await.expect("the other key commits");
    assert_eq!(
        keys().await,
        3,
        "the ceiling of two passed by exactly the other transaction's one key"
    );
    let third = w.start(workflow, &json!({"accountId": "acct-3"})).await;
    assert_eq!(
        failure(&w, third).await,
        (
            "FAILED".to_owned(),
            Some("FLOW_CONTROL_LIMIT".to_owned()),
            0
        ),
        "every committed row is counted"
    );
    assert_eq!(keys().await, 3, "no row for the refused key");
    // the stand-in key has no holder and no waiter: it goes before the consistency check, which refuses an empty key row
    sqlx::query("DELETE FROM concurrency_semaphores WHERE key_digest = $1")
        .bind(&stand_in)
        .execute(&w.db.pool)
        .await
        .expect("the stand-in key goes");
    for run in [first, second, third] {
        w.assert_consistent(run).await;
    }
}

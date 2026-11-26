//! The flow-control alerts of the engine's monitor and the gauges it sets (task T022-082; FR-022-049; decision D022-077
//! items 4, 6 and 7; `docs/runbooks/flow-control.md` sections 8 to 10 and 14, `docs/runbooks/alerts.md`): each alert is
//! raised by the condition its runbook section names and resolved when the condition ends, one alert per group, its
//! subject the group and never a key. Time never decides: the instants an alert reads are seeded (a waiter's queueing,
//! a key's hint, a grant's instant), as the owner, with the guard of an immutable row switched off in that one
//! transaction.

use sagat_application::operations::MonitorStore;
use sagat_application::runs::RunOperation;
use sagat_domain::execution::RunId;
use sagat_domain::quota::QuotaResource;
use sagat_scheduler::{Monitor, MonitorSettings};
use serde_json::{Value, json};

use crate::run_admission::{serial_start, set};
use crate::support::engine::{World, activity, document};
use crate::support::flow_control::{gated_world, publish};

/// A one-step workflow `name` whose start takes a slot of `input.accountId` in group `accounts`, `limit` an hour.
fn rated_start(name: &str, limit: u32) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": limit, "per": "hour"});
    doc
}

/// The alerts of `kind` and their state: (subject, state, items).
async fn alerts(w: &World, kind: &str) -> Vec<(String, String, i64)> {
    sqlx::query_as(
        "SELECT subject, state, item_count FROM operational_alerts WHERE tenant_id = $1 AND kind = $2 ORDER BY subject",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(kind)
    .fetch_all(&w.db.pool)
    .await
    .expect("alerts")
}

/// Runs `sql` as the owner with the triggers of `table` that forbid changing its rows switched off in that transaction.
async fn seed(w: &World, table: &'static str, sql: &'static str, run: RunId) {
    let mut tx = w.db.pool.begin().await.expect("tx");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE {table} DISABLE TRIGGER USER"
    )))
    .execute(&mut *tx)
    .await
    .expect("the guards off in this transaction");
    sqlx::query(sql)
        .bind(run.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("seeded");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE {table} ENABLE TRIGGER USER"
    )))
    .execute(&mut *tx)
    .await
    .expect("the guards on again");
    tx.commit().await.expect("seed");
}

/// The engine's monitor over a store of the runtime role's pool, in every run of the suite (batch 5 security nit,
/// decision D022-130 item 6): the alert detection is what the engine runs as `sagat_app`, so a table its statements read
/// without a grant fails the owner's run too, as the runtime-role statements of `rate_gate.rs` and `migrations.rs` do.
async fn monitor(w: &World) -> Monitor<sagat_infra_postgres::PgRunStore> {
    Monitor::new(runtime_store(w).await, MonitorSettings::default())
}

/// A store of the runtime role's pool (see [`monitor`]).
async fn runtime_store(w: &World) -> sagat_infra_postgres::PgRunStore {
    crate::support::flow_control::gate(sagat_infra_postgres::PgRunStore::new(
        w.db.runtime_pool_with(4).await,
        w.cache.clone(),
    ))
}

/// `CONCURRENCY_QUEUE_STUCK` (section 8): a key with no room whose oldest eligible waiter queued more than the unclaimed
/// threshold (300 s) before the key's oldest holder took its permit fires one warning for the group; a key whose holder
/// is older than its waiter does not (a holder that never ends is not this alert); the waiter's cancellation resolves it.
#[tokio::test]
async fn a_waiter_passed_over_by_every_holder_raises_concurrency_queue_stuck_until_it_leaves() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("alert-stuck", 1)).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    let monitor = monitor(&w).await;
    monitor.run_once().await;
    assert!(
        alerts(&w, "CONCURRENCY_QUEUE_STUCK").await.is_empty(),
        "a waiter of a few seconds is not stuck"
    );
    // the waiter queued 400 s ago, and the holder took its permit 500 s ago, before it: a holder that does not end is
    // not this alert (its waiters time out at their deadline)
    seed(
        &w,
        "concurrency_waiters",
        "UPDATE concurrency_waiters SET enqueued_at = clock_timestamp() - interval '400 seconds', \
                deadline_at = clock_timestamp() + interval '1 hour' WHERE run_id = $1",
        waiter,
    )
    .await;
    seed(
        &w,
        "concurrency_holders",
        "UPDATE concurrency_holders SET acquired_at = clock_timestamp() - interval '500 seconds' WHERE run_id = $1",
        holder,
    )
    .await;
    monitor.run_once().await;
    assert!(
        alerts(&w, "CONCURRENCY_QUEUE_STUCK").await.is_empty(),
        "a holder older than the waiter is not a waiter passed over"
    );
    // the holder took its permit now, 400 s after the waiter queued: every permit it waited for went to another
    seed(
        &w,
        "concurrency_holders",
        "UPDATE concurrency_holders SET acquired_at = clock_timestamp() WHERE run_id = $1",
        holder,
    )
    .await;
    let round = monitor.run_once().await;
    assert_eq!(
        alerts(&w, "CONCURRENCY_QUEUE_STUCK").await,
        [("accounts".to_owned(), "FIRING".to_owned(), 1)],
        "{round:?}"
    );
    w.operate_run(waiter, RunOperation::Cancel).await;
    monitor.run_once().await;
    assert_eq!(
        alerts(&w, "CONCURRENCY_QUEUE_STUCK").await,
        [("accounts".to_owned(), "RESOLVED".to_owned(), 1)]
    );
    w.operate_run(holder, RunOperation::Cancel).await;
}

/// `RATE_LIMIT_GRANT_LAGGING` (section 9): a window with waiters due for longer than the lease grace (60 s) and no grant
/// of it in that time fires one critical alert for the group; a busy key with a recent grant does not; once the pass
/// grants the waiter the alert resolves.
#[tokio::test]
async fn a_due_window_no_pass_serves_raises_rate_limit_grant_lagging_until_it_is_granted() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("alert-lagging", 1)).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    // due for two minutes
    sqlx::query(
        "UPDATE rate_limit_windows SET next_eligible_at = clock_timestamp() - interval '120 seconds' WHERE waiters > 0",
    )
    .execute(&w.db.pool)
    .await
    .expect("due");
    let monitor = monitor(&w).await;
    monitor.run_once().await;
    assert!(
        alerts(&w, "RATE_LIMIT_GRANT_LAGGING").await.is_empty(),
        "a key granted within the grace is busy, not lagging"
    );
    // the holder's grant two minutes old: no grant of the key within the grace
    seed(
        &w,
        "rate_limit_grants",
        "UPDATE rate_limit_grants SET granted_at = granted_at - interval '120 seconds', \
                leased_at = leased_at - interval '120 seconds' WHERE run_id = $1",
        holder,
    )
    .await;
    monitor.run_once().await;
    assert_eq!(
        alerts(&w, "RATE_LIMIT_GRANT_LAGGING").await,
        [("accounts".to_owned(), "FIRING".to_owned(), 1)]
    );
    let severity: String = sqlx::query_scalar(
        "SELECT severity FROM operational_alerts WHERE kind = 'RATE_LIMIT_GRANT_LAGGING'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("severity");
    assert_eq!(severity, "critical");
    // the holder's slot out of its hour: the pass grants the waiter
    seed(
        &w,
        "rate_limit_grants",
        "UPDATE rate_limit_grants SET granted_at = granted_at - interval '2 hours', \
                leased_at = leased_at - interval '2 hours' WHERE run_id = $1",
        holder,
    )
    .await;
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    monitor.run_once().await;
    assert_eq!(
        alerts(&w, "RATE_LIMIT_GRANT_LAGGING").await,
        [("accounts".to_owned(), "RESOLVED".to_owned(), 1)]
    );
    for run in [holder, waiter] {
        w.operate_run(run, RunOperation::Cancel).await;
    }
}

/// `RATE_LIMIT_QUEUE_SATURATED` (section 10): with the namespace's `flow_control_key_waiters` at 10, a key with nine
/// buffered starts (above 80%) fires one warning for the group, with eight it does not, and the alert resolves when the
/// buffer falls to eight.
#[tokio::test]
async fn a_buffer_above_eighty_percent_raises_rate_limit_queue_saturated_until_it_drains() {
    let w = gated_world().await;
    set(&w, QuotaResource::FlowControlKeyWaiters, 10).await;
    let workflow = publish(&w, rated_start("alert-saturated", 1)).await;
    let input = json!({"accountId": "acct-1"});
    let _holder = w.start(workflow, &input).await;
    let mut waiters = Vec::new();
    for _ in 0..8 {
        waiters.push(w.start(workflow, &input).await);
    }
    let monitor = monitor(&w).await;
    monitor.run_once().await;
    assert!(
        alerts(&w, "RATE_LIMIT_QUEUE_SATURATED").await.is_empty(),
        "eight of ten is 80%, not above it"
    );
    waiters.push(w.start(workflow, &input).await);
    monitor.run_once().await;
    assert_eq!(
        alerts(&w, "RATE_LIMIT_QUEUE_SATURATED").await,
        [("accounts".to_owned(), "FIRING".to_owned(), 1)]
    );
    w.operate_run(waiters[0], RunOperation::Cancel).await;
    monitor.run_once().await;
    assert_eq!(
        alerts(&w, "RATE_LIMIT_QUEUE_SATURATED").await,
        [("accounts".to_owned(), "RESOLVED".to_owned(), 1)]
    );
}

/// `FLOW_CONTROL_WAITER_ORPHANED` (section 14; invariant 12, gap G022-005): a waiter row of a run that ended fires one
/// critical alert for its group, whatever its age; the row's removal resolves it. The waiter gauges and the unleased
/// grants the monitor samples are the counts of the key rows, by scope.
#[tokio::test]
async fn a_waiter_of_an_ended_run_raises_flow_control_waiter_orphaned_and_the_gauges_count_the_waiters()
 {
    // the process's recorder, installed before the monitor sets a gauge
    let telemetry = sagat_telemetry::Metrics::global();
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("alert-orphan", 1)).await;
    let input = json!({"accountId": "acct-1"});
    let _holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    let sample = runtime_store(&w).await.sample().await.expect("a sample");
    assert_eq!(sample.flow_control.rate_run_waiters, 1, "{sample:?}");
    assert_eq!(sample.flow_control.concurrency_run_waiters, 0);
    let monitor = monitor(&w).await;
    monitor.run_once().await;
    assert!(alerts(&w, "FLOW_CONTROL_WAITER_ORPHANED").await.is_empty());
    let page = telemetry.render();
    for series in [
        "concurrency_waiters{scope=\"run\"}",
        "concurrency_waiters{scope=\"step\"}",
        "rate_limit_waiters{scope=\"run\"}",
        "rate_limit_waiters{scope=\"step\"}",
        "rate_limit_unleased_grants ",
    ] {
        assert!(
            page.contains(series),
            "{series} is not on the page:\n{page}"
        );
    }
    // the breach, seeded: the run ended and its waiter row stayed
    sqlx::query(
        "UPDATE runs SET status = 'CANCELLED', finished_at = clock_timestamp() WHERE id = $1",
    )
    .bind(waiter.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("the run ended under its waiter row");
    monitor.run_once().await;
    assert_eq!(
        alerts(&w, "FLOW_CONTROL_WAITER_ORPHANED").await,
        [("accounts".to_owned(), "FIRING".to_owned(), 1)]
    );
    // the operator's repair, as the owner
    sqlx::query("DELETE FROM rate_limit_waiters WHERE run_id = $1")
        .bind(waiter.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("removed");
    monitor.run_once().await;
    assert_eq!(
        alerts(&w, "FLOW_CONTROL_WAITER_ORPHANED").await,
        [("accounts".to_owned(), "RESOLVED".to_owned(), 1)]
    );
}

/// Seeds `count` run-scope windows of group `group` in `namespace` of `tenant`, each with `waiters` waiters whose oldest
/// queued 10 s ago (the counters only: the monitor reads a window's `waiters`, never its waiter rows).
async fn seed_windows(
    w: &World,
    tenant: uuid::Uuid,
    namespace: uuid::Uuid,
    group: &str,
    count: i32,
    waiters: i32,
) {
    sqlx::query(
        "INSERT INTO rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, waiters, \
                                         max_period_ms, key_version, key_shard, oldest_wait_at, updated_at) \
         SELECT $1, $2, 'ORDINARY', 'RUN', $3, sha256(($3 || '/' || i)::bytea), $5, 3600000, \
                flow_control_active_key_version(), 0, clock_timestamp() - interval '10 seconds', clock_timestamp() \
           FROM generate_series(1, $4) i",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(group)
    .bind(count)
    .bind(waiters)
    .execute(&w.db.pool)
    .await
    .expect("windows seeded");
}

/// Batch 3 reviews of 2026-11-25, correctness Major 2 and security Major 1: the condition of `RATE_LIMIT_QUEUE_SATURATED`
/// is applied before any bound, and a look that stops short stops short for a tenant or a namespace, never for everyone.
/// Tenant A has more than 5,000 run-scope windows with three waiters each (well under its ceiling: not saturated) and a
/// saturated alert whose key has drained; tenant B has one key with two waiters against its `flow_control_key_waiters`
/// of 2. B's key fires, and A's drained alert resolves (after the two looks that miss a condition, correctness minor 3).
#[tokio::test]
async fn a_saturated_key_of_one_tenant_fires_beside_thousands_of_another_tenants_keys_and_drained_alerts_resolve()
 {
    let w = gated_world().await;
    let tenant_a = w.admin.tenant_id().as_uuid();
    let namespace_a = w.namespace.id().as_uuid();
    let (tenant_b, namespace_b): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT tenant_id, id FROM namespaces WHERE tenant_id = $1 AND slug = 'payments'",
    )
    .bind(w.other_admin.tenant_id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the other tenant's namespace");
    w.quotas
        .set(
            &w.other_admin,
            Some(sagat_domain::identity::NamespaceId::new(namespace_b)),
            QuotaResource::FlowControlKeyWaiters,
            2,
            &crate::support::engine::ctx(),
        )
        .await
        .expect("tenant B's key waiters");
    seed_windows(&w, tenant_a, namespace_a, "accounts", 5_001, 3).await;
    seed_windows(&w, tenant_b, namespace_b, "payees", 1, 2).await;
    sqlx::query(
        "INSERT INTO operational_alerts (id, tenant_id, namespace_id, kind, subject, state, severity, first_seen_at, \
                                         last_seen_at, fired_count, item_count, oldest_seconds) \
         VALUES (gen_random_uuid(), $1, $2, 'RATE_LIMIT_QUEUE_SATURATED', 'drained', 'FIRING', 'warning', \
                 clock_timestamp() - interval '1 hour', clock_timestamp() - interval '1 minute', 1, 1, 60)",
    )
    .bind(tenant_a)
    .bind(namespace_a)
    .execute(&w.db.pool)
    .await
    .expect("tenant A's alert of a key that drained");
    let monitor = monitor(&w).await;
    monitor.run_once().await;
    monitor.run_once().await;
    let states: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
        "SELECT tenant_id, subject, state FROM operational_alerts WHERE kind = 'RATE_LIMIT_QUEUE_SATURATED' \
          ORDER BY subject",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("alerts");
    assert_eq!(
        states,
        [
            (tenant_a, "drained".to_owned(), "RESOLVED".to_owned()),
            (tenant_b, "payees".to_owned(), "FIRING".to_owned()),
        ]
    );
}

/// Batch 3 reviews of 2026-11-25, correctness Major 2: `CONCURRENCY_QUEUE_STUCK` compares a key's holders with the lower
/// of its waiter's declaration and the namespace's effective `concurrency_limit`, resolved in the statement as the gate
/// resolves it; here the tenant's setting (1) is below the declaration (2), so one holder leaves no room.
#[tokio::test]
async fn a_stuck_key_is_judged_against_the_tenants_concurrency_ceiling_below_its_declaration() {
    let w = gated_world().await;
    w.quotas
        .set(
            &w.admin,
            None,
            QuotaResource::ConcurrencyLimit,
            1,
            &crate::support::engine::ctx(),
        )
        .await
        .expect("the tenant's concurrency limit");
    let workflow = publish(&w, serial_start("alert-stuck-ceiling", 2)).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let waiter = w.start(workflow, &input).await;
    assert_eq!(
        crate::run_admission::start_rows(&w, waiter).await,
        (0, 1),
        "the ceiling queues it"
    );
    seed(
        &w,
        "concurrency_waiters",
        "UPDATE concurrency_waiters SET enqueued_at = clock_timestamp() - interval '400 seconds', \
                deadline_at = clock_timestamp() + interval '1 hour' WHERE run_id = $1",
        waiter,
    )
    .await;
    monitor(&w).await.run_once().await;
    assert_eq!(
        alerts(&w, "CONCURRENCY_QUEUE_STUCK").await,
        [("accounts".to_owned(), "FIRING".to_owned(), 1)]
    );
    for run in [waiter, holder] {
        w.operate_run(run, RunOperation::Cancel).await;
    }
}

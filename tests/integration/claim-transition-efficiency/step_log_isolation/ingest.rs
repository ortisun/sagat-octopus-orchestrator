//! Design 041 section 2, task T041-6: the ingest beside the hot path, calling `StepLogIngest` directly.
//!
//! * SC-041-001's lock-free half: a batch commits while its run row is held, because the ingest takes no lock a
//!   report, a task lease or a heartbeat takes.
//! * The quota-lease half: with one batch of a tenant held after T8 (its charge holds its own quota lease), three more
//!   batches of that tenant, each on a backend of its own, commit while it is held, each charged to its own quota
//!   lease, and no refill runs; a refill held (the hold after a T8 that refilled holds the usage row) delays only a
//!   charge that needs a refill, while a charge into a quota lease with room commits. The carrying report's half is
//!   task T041-36's.
//! * The lock order of section 2 (L1, L1b, L2, L3, L3b, then the usage row): an ingest, a report that carries nothing,
//!   a change of a step-log resource and the purge, each against a reference transaction that takes the locks in
//!   that order (`support/lock_order.rs`), for every start; none of them deadlocks.

use sagat_application::maintenance::{MaintenanceStore as _, StepLogPurge};
use sagat_application::step_logs::{BatchAnswer, StepLogVerdict};
use sagat_domain::quota::QuotaResource;
use serde_json::json;
use sqlx::Connection as _;

use super::LIMIT;
use crate::support::bounded;
use crate::support::engine::{World, ctx, world};
use crate::support::lock_order::{Lock, Outcome, Slot, against_at};
use crate::support::step_logs::{
    Attempt, HELD_LOCK_TIMEOUT, Hook, batch, empty_quota_leases, hooked_on, ingest,
    ingest_on_own_pool, leased, pool_of, records, send, spawn_send,
};

const RETRY: BatchAnswer = BatchAnswer {
    verdict: StepLogVerdict::RetryLater { after_ms: 200 },
    stored_through_sequence: 0,
};

fn continued(through: u64) -> BatchAnswer {
    BatchAnswer {
        verdict: StepLogVerdict::Continue,
        stored_through_sequence: through,
    }
}

/// The refills counted so far (`step_log_quota_refills_refill_seq`, through the counters' accessor).
async fn refills(w: &World) -> i64 {
    sqlx::query_scalar("SELECT refills FROM sagat_quota_step_log_counters()")
        .fetch_one(&w.db.pool)
        .await
        .expect("the counters")
}

/// SC-041-001: a batch commits while its run's row is held `FOR UPDATE` (as a report holds it).
#[tokio::test]
async fn a_batch_commits_while_its_run_row_is_held() {
    bounded::within("run row held", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let mut holder = sqlx::PgConnection::connect(&w.db.url)
            .await
            .expect("a holder");
        let mut tx = holder.begin().await.expect("a transaction");
        sqlx::query("SELECT 1 FROM runs WHERE id = $1 FOR UPDATE")
            .bind(a.run.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("the run row");
        sqlx::query("SELECT 1 FROM steps WHERE run_id = $1 FOR UPDATE")
            .bind(a.run.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("the step rows");
        assert_eq!(send(&ingest(&w), &a, records([1, 2])).await, continued(2));
        tx.rollback().await.expect("release");
    })
    .await;
}

/// With one batch of the tenant held after T8, holding its own quota lease, three more batches of the tenant, each on
/// a backend of its own with a quota lease of its own, commit while it is held, and none of the four refills: a charge
/// with room never takes the usage row.
#[tokio::test]
async fn three_batches_commit_while_a_fourth_holds_its_quota_lease_and_none_refills() {
    bounded::within("quota leases", LIMIT, async {
        let w = world().await;
        let mut writers = Vec::new();
        for node in ["a", "b", "c", "d"] {
            let attempt = leased(&w, node).await;
            let pool = pool_of(&w, 1).await;
            // Each backend's first batch refills its quota lease; the second charges into it.
            let (warm, _) = hooked_on(pool.clone(), Hook::CommitThenFail);
            assert!(
                warm.append(&attempt.session.principal, &batch(&attempt, records([1])))
                    .await
                    .is_err()
            );
            writers.push((attempt, pool));
        }
        let before = refills(&w).await;
        let (held, gate) = hooked_on(writers[0].1.clone(), Hook::HoldAfterInsert);
        let first = spawn_send(&held, &writers[0].0, records([2]));
        gate.reached().await;
        for (attempt, pool) in &writers[1..] {
            let ingest = sagat_application::step_logs::StepLogIngest::new(
                sagat_infra_postgres::PgStepLogStore::new(pool.clone()),
                sagat_application::clock::SystemClock,
                true,
            );
            assert_eq!(send(&ingest, attempt, records([2])).await, continued(2));
        }
        gate.release();
        assert_eq!(
            first.await.expect("the batch").expect("answered"),
            continued(2)
        );
        assert_eq!(refills(&w).await, before, "no charge took the usage row");
        let leases: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM step_log_quota_leases WHERE tenant_id = $1 AND used > 0",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the quota leases");
        assert_eq!(leases, 4, "each writer charged its own quota lease");
    })
    .await;
}

/// What `body` counts, on a runtime of one thread with a recorder of its own (as `step-logs/counted.rs`).
fn counted<F: std::future::Future<Output = ()>>(body: F) -> String {
    let (metrics, recorder) =
        sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    metrics::with_local_recorder(&recorder, || runtime.block_on(body));
    metrics.render()
}

fn value(page: &str, line_start: &str) -> Option<f64> {
    page.lines()
        .find(|line| line.starts_with(line_start))
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse().ok())
}

/// A batch held after a T8 that refilled holds the tenant's usage row (the fixture empties every quota lease first, so
/// that its charge refills): a batch whose quota lease has room commits meanwhile, and a batch that needs a refill
/// waits for the usage row at most its budget and answers `RETRY_LATER`, counted as a lock timeout and not as a quota
/// refusal; once the refill commits, the resend is stored.
#[test]
fn a_held_refill_delays_only_a_charge_that_needs_a_refill() {
    let page = counted(async {
        bounded::within("held refill", LIMIT, async {
            let w = world().await;
            let a = leased(&w, "a").await;
            let b = leased(&w, "b").await;
            let c = leased(&w, "c").await;
            empty_quota_leases(&w).await;
            let with_room = ingest_on_own_pool(&w, 1).await;
            assert_eq!(send(&with_room, &b, records([1])).await, continued(1));
            let (held, gate) = hooked_on(pool_of(&w, 1).await, Hook::HoldAfterInsert);
            let refilling = spawn_send(&held, &a, records([1]));
            let holder = gate.reached().await;
            let locks: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks l JOIN pg_class c ON c.oid = l.relation \
                  WHERE l.pid = $1 AND c.relname = 'tenant_retained_usage' AND l.mode = 'RowShareLock'",
            )
            .bind(holder)
            .fetch_one(&w.db.pool)
            .await
            .expect("pg_locks");
            assert_eq!(locks, 1, "the held batch refilled: it locked the usage row");
            assert_eq!(send(&with_room, &b, records([2])).await, continued(2));
            let needs_refill = ingest_on_own_pool(&w, 1).await;
            assert_eq!(send(&needs_refill, &c, records([1])).await, RETRY);
            gate.release();
            assert_eq!(refilling.await.expect("the batch").expect("answered"), continued(1));
            assert_eq!(send(&needs_refill, &c, records([1])).await, continued(1));
        })
        .await;
    });
    assert_eq!(value(&page, "step_log_lock_timeouts_total "), Some(1.0));
    assert_eq!(value(&page, "step_log_quota_refusals_total"), None);
}

/// L3b is only ever tried: with another transaction holding a writer's own quota lease (only a quota reclaim does),
/// the writer's charge passes over it and its refill takes its quota lease `NOWAIT`, so the batch answers `RETRY_LATER`
/// at once, while the lease is still held, although its own lock timeout is a minute.
#[tokio::test]
async fn a_writer_never_waits_for_its_own_quota_lease_held_elsewhere() {
    bounded::within("quota lease tried", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let pool = pool_of(&w, 1).await;
        let writer = sagat_application::step_logs::StepLogIngest::new(
            sagat_infra_postgres::PgStepLogStore::new(pool.clone())
                .with_lock_timeout(HELD_LOCK_TIMEOUT),
            sagat_application::clock::SystemClock,
            true,
        );
        assert_eq!(send(&writer, &a, records([1])).await, continued(1));
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&pool)
            .await
            .expect("the writer's backend");
        let mut holder = sqlx::PgConnection::connect(&w.db.url)
            .await
            .expect("a holder");
        let mut tx = holder.begin().await.expect("a transaction");
        sqlx::query("SELECT 1 FROM step_log_quota_leases WHERE writer = $1 FOR UPDATE")
            .bind(pid)
            .execute(&mut *tx)
            .await
            .expect("the writer's quota lease");
        assert_eq!(send(&writer, &a, records([2])).await, RETRY);
        tx.rollback().await.expect("release");
        assert_eq!(send(&writer, &a, records([2])).await, continued(2));
    })
    .await;
}

// ---------------------------------------------------------------------------------- the lock order

/// The locks of section 2's order for one attempt's stream, each taken the way its strongest other holder takes it:
/// L1 a change of `step_log_attempt_records` (its namespace fence, exclusive), L1b another ingest of the attempt, L2
/// the purge or the delete route (the stream row), L3 a change of the tenant's step-log limit (its class fence,
/// exclusive), and the usage row (L4) a refill.
async fn reference(w: &World, a: &Attempt) -> Vec<(Slot, Lock)> {
    let tenant = w.admin.tenant_id();
    let stream: i64 =
        sqlx::query_scalar("SELECT stream_id FROM step_log_streams WHERE run_id = $1")
            .bind(a.run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("the stream");
    let advisory =
        |key: String| format!("SELECT pg_advisory_xact_lock(hashtextextended('{key}', 0))");
    vec![
        (
            Slot::StepLogPolicyFences,
            Lock::statement(
                "L1 the namespace fence of step_log_attempt_records",
                advisory(format!(
                    "quota-policy:step_log_attempt_records:{tenant}:{}",
                    w.namespace.id()
                )),
            ),
        ),
        (
            Slot::StepLogStreamLock,
            Lock::statement(
                "L1b the attempt's stream lock",
                advisory(sagat_infra_postgres::stream_lock_key(
                    tenant,
                    a.task.run_id,
                    a.task.step_id,
                    a.task.fence.attempt,
                    a.task.fence.generation,
                )),
            ),
        ),
        (
            Slot::StepLogStream,
            Lock::statement(
                "L2 the stream row",
                format!(
                    "SELECT 1 FROM step_log_streams WHERE tenant_id = '{tenant}' AND stream_id = {stream} FOR UPDATE"
                ),
            ),
        ),
        (
            Slot::QuotaClassFences,
            Lock::statement(
                "L3 the step-log class fence",
                advisory(format!("quota-policy:tenant_step_log_bytes:{tenant}")),
            ),
        ),
        (
            Slot::Usage,
            Lock::statement(
                "L4 the usage row",
                format!(
                    "SELECT 1 FROM tenant_retained_usage WHERE tenant_id = '{tenant}' FOR UPDATE"
                ),
            ),
        ),
    ]
}

/// An ingest (its stream created by an earlier batch, so that the row exists to be held), run against the
/// reference from each of L1, L1b, L2 and L3: it waits for the lock held and commits after the reference, which takes
/// every later lock and commits; no deadlock.
#[tokio::test]
async fn an_ingest_takes_the_step_log_locks_in_the_order_of_section_2() {
    bounded::within("ingest order", LIMIT, async {
        let w = world().await;
        let patient = sagat_application::step_logs::StepLogIngest::new(
            sagat_infra_postgres::PgStepLogStore::new(w.execution_pool.clone())
                .with_lock_timeout(HELD_LOCK_TIMEOUT),
            sagat_application::clock::SystemClock,
            true,
        );
        // The usage row exists before the reference locks it.
        for start in 0..4 {
            let a = leased(&w, &format!("n{start}")).await;
            assert_eq!(send(&patient, &a, records([1])).await, continued(1));
            let reference = reference(&w, &a).await;
            let ran = against_at(
                &w.db.pool,
                &reference,
                start,
                patient.append(&a.session.principal, &batch(&a, records([2]))),
            )
            .await;
            assert_eq!(ran.reference, Outcome::Committed, "start {start}");
            assert_eq!(ran.path.expect("answered"), continued(2), "start {start}");
        }
    })
    .await;
}

/// A report that carries nothing takes none of L1, L1b and L2: it commits while another transaction holds all three
/// for its own attempt.
#[tokio::test]
async fn a_report_that_carries_nothing_takes_no_step_log_lock() {
    bounded::within("report", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        send(&ingest(&w), &a, records([1])).await;
        let reference = reference(&w, &a).await;
        let mut holder = sqlx::PgConnection::connect(&w.db.url)
            .await
            .expect("a holder");
        let mut tx = holder.begin().await.expect("a transaction");
        for (_, lock) in &reference[..3] {
            sqlx::query(sqlx::AssertSqlSafe(lock.sql.clone()))
                .execute(&mut *tx)
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", lock.name));
        }
        w.complete(&a.session, &a.task, &json!({}))
            .await
            .expect("the report commits while L1, L1b and L2 are held");
        tx.rollback().await.expect("release");
    })
    .await;
}

/// A change of a step-log resource takes its namespace fence (L1) exclusively and nothing after it: against the
/// reference holding that fence shared, as an ingest after T3 holds it, the change waits and the reference takes
/// every later lock and commits.
#[tokio::test]
async fn a_change_of_a_step_log_resource_takes_only_its_fence() {
    bounded::within("change", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        send(&ingest(&w), &a, records([1])).await;
        let mut reference = reference(&w, &a).await;
        reference[0].1 = Lock::statement(
            "L1 the namespace fence of step_log_attempt_records, shared as an ingest holds it",
            format!(
                "SELECT pg_advisory_xact_lock_shared(hashtextextended('quota-policy:step_log_attempt_records:{}:{}', 0))",
                w.admin.tenant_id(),
                w.namespace.id()
            ),
        );
        let context = ctx();
        let ran = against_at(
            &w.db.pool,
            &reference,
            0,
            w.quotas.set(
                &w.admin,
                Some(w.namespace.id()),
                QuotaResource::StepLogAttemptRecords,
                5,
                &context,
            ),
        )
        .await;
        assert_eq!(ran.reference, Outcome::Committed);
        assert_eq!(ran.path.expect("the change").limit, 5);
    })
    .await;
}

/// T041-10's test that needs this order (D041-144 (6)): the purge locks stream rows only `SKIP LOCKED` (a row held is
/// left, never waited for) and takes the step-log class fence and the usage row only at its commit, for the release:
/// against the reference holding the class fence (L3), it waits, the reference takes the usage row and commits, and
/// the purge commits after it.
#[tokio::test]
async fn the_purge_skips_a_held_stream_row_and_releases_in_the_order_of_section_2() {
    bounded::within("purge", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        send(&ingest(&w), &a, records([1, 2])).await;
        sqlx::query("UPDATE step_log_streams SET removed_at = now() WHERE run_id = $1")
            .bind(a.run.as_uuid())
            .execute(&w.db.pool)
            .await
            .expect("removal begun");
        let reference = reference(&w, &a).await;
        // L2 held: the purge passes over it and waits for nothing.
        let mut holder = sqlx::PgConnection::connect(&w.db.url)
            .await
            .expect("a holder");
        let mut tx = holder.begin().await.expect("a transaction");
        sqlx::query(sqlx::AssertSqlSafe(reference[2].1.sql.clone()))
            .execute(&mut *tx)
            .await
            .expect("the stream row");
        assert!(matches!(
            w.store
                .purge_step_log_lines(None, None, 500)
                .await
                .expect("a pass"),
            StepLogPurge::Nothing
        ));
        tx.rollback().await.expect("release");
        // L3 held: the release at the purge's commit waits for it.
        let ran = against_at(
            &w.db.pool,
            &reference,
            3,
            w.store.purge_step_log_lines(None, None, 500),
        )
        .await;
        assert_eq!(ran.reference, Outcome::Committed);
        assert!(
            matches!(
                ran.path.expect("a pass"),
                StepLogPurge::Purged { lines: 2, .. }
            ),
            "the purge deleted the two lines"
        );
    })
    .await;
}

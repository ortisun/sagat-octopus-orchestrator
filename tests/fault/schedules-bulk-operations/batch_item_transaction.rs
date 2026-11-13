//! The effect of a batch item on its run and the record of the item are one transaction (`FR-018-030`, ADR-0050
//! decision 2, O1951 answered (b) by OD-2026-11-13-2, G1961): the run's own transaction locks the fence, the job and the
//! item after the run and records the outcome before it commits. What that closes, each forced with a held lock and seen
//! in `pg_stat_activity` (never a sleep), with the real run service:
//!
//! - a cancel of the job that commits after the executor's last look and before the effect lets no effect through;
//! - a replica whose lease is taken over in that window lets no effect through;
//! - a process that dies between the effect and the record of its item leaves neither: the item never says less than
//!   happened to its run.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::batch::{BatchService, DueJob, JobRecord};
use sagat_application::batch_executor::{
    BatchExecutor, ExecutorSettings, ItemActions, ItemEffect, ItemRequest, Stepped,
};
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedules::ShardFence;
use sagat_application::shards::Lease;
use sagat_domain::batch::JobState;
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::{PgBatchStore, PgIdentityStore};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::batch::{create, runs_started};
use crate::support::bounded::within;
use crate::support::engine::World;
use crate::support::schedules::{Fixture, TestClock, fixture};

/// The run service of the world, as the executor's single-run use cases.
#[derive(Clone)]
struct Runs(Arc<World>);

impl ItemActions for Runs {
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        ItemActions::apply(&self.0.runs, request).await
    }
}

type Executor = BatchExecutor<PgBatchStore, Runs, PgIdentityStore, TestClock>;

fn executor(f: &Fixture) -> Arc<Executor> {
    Arc::new(BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        Runs(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        f.clock.clone(),
        ExecutorSettings::default(),
    ))
}

fn service(f: &Fixture) -> BatchService<PgBatchStore, TestClock> {
    BatchService::new(PgBatchStore::new(f.w.db.pool.clone()), f.clock.clone())
}

fn due(job: &JobRecord) -> DueJob {
    DueJob {
        tenant_id: job.tenant_id,
        id: job.id,
    }
}

async fn job(f: &Fixture, id: Uuid) -> JobRecord {
    service(f).get(&f.w.admin, id).await.expect("job")
}

async fn item_state(f: &Fixture, id: Uuid) -> String {
    sqlx::query_scalar("SELECT state FROM batch_items WHERE job_id = $1")
        .bind(id)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the item")
}

async fn status_of(f: &Fixture, run: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
        .bind(run)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("status")
}

/// The requests to cancel `run` on record: one per effect.
async fn cancellations(f: &Fixture, run: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.cancel_requested' AND resource_id = $1",
    )
    .bind(run.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit")
}

/// A job that cancels one open run, its run selected: the next step applies the item.
async fn one_item_job(f: &Fixture, key: &str, fence: Option<&ShardFence>) -> (JobRecord, Uuid) {
    let runs = runs_started(f, 1).await;
    f.clock
        .set(OffsetDateTime::now_utc() + Duration::seconds(2));
    let created = create(f, &f.w.admin, &runs, key).await;
    let exec = executor(f);
    within("the selection", StdDuration::from_secs(120), async {
        loop {
            f.clock.advance(Duration::seconds(1));
            match exec
                .step(&due(&created), fence.cloned())
                .await
                .expect("select")
            {
                Stepped::Acted => break,
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await;
    assert_eq!(job(f, created.id).await.counts.selected, 1);
    f.clock.advance(Duration::seconds(1));
    (created, runs[0])
}

/// A transaction that holds the row that `sql` locks, and the id of its backend.
async fn hold(
    f: &Fixture,
    sql: &'static str,
    id: Uuid,
) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut held = f.w.db.pool.begin().await.expect("begin");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *held)
        .await
        .expect("pid");
    sqlx::query(sql)
        .bind(id)
        .execute(&mut *held)
        .await
        .expect("take the lock");
    (held, pid)
}

/// The backend that waits for a lock `holder` holds while running a statement on `table` (`pg_stat_activity`, not a time).
async fn blocked_on(f: &Fixture, holder: i32, table: &str, stage: &str) -> i32 {
    within(stage, StdDuration::from_secs(120), async {
        loop {
            let pid: Option<i32> = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity \
                 WHERE $1 = ANY(pg_blocking_pids(pid)) AND wait_event_type = 'Lock' \
                   AND strpos(query, $2) > 0 \
                 LIMIT 1",
            )
            .bind(holder)
            .bind(table)
            .fetch_optional(&f.w.db.pool)
            .await
            .expect("look at the backends");
            if let Some(pid) = pid {
                return pid;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
}

const HOLD_RUN: &str = "SELECT 1 FROM runs WHERE id = $1 FOR UPDATE";
const HOLD_ITEM: &str = "SELECT 1 FROM batch_items WHERE job_id = $1 FOR UPDATE";

#[tokio::test]
async fn a_cancel_of_the_job_between_the_last_look_and_the_effect_lets_no_effect_through() {
    let f = fixture().await;
    let (created, run) = one_item_job(&f, "one-tx-late-cancel", None).await;
    let exec = executor(&f);

    // the step looks (the job runs, the item is pending) and then waits for the run
    let (held, holder) = hold(&f, HOLD_RUN, run).await;
    let step = tokio::spawn({
        let exec = Arc::clone(&exec);
        let due = due(&created);
        async move { exec.step(&due, None).await }
    });
    blocked_on(
        &f,
        holder,
        "runs",
        "the step waits for the run after its last look",
    )
    .await;
    // the job is cancelled in that window, and the cancel commits
    service(&f)
        .cancel(
            &f.w.admin,
            created.id,
            "stop this batch job",
            &RequestContext::default(),
        )
        .await
        .expect("cancel");
    held.commit().await.expect("release the run");
    let stepped = step.await.expect("join").expect("step");

    assert!(matches!(stepped, Stepped::Refused(_)), "{stepped:?}");
    assert_ne!(
        status_of(&f, run).await,
        "CANCELLED",
        "the effect did not go through after the job was cancelled"
    );
    assert_eq!(cancellations(&f, run).await, 0);
    // the drain ends the job and the item it never applied
    for _ in 0..50 {
        if job(&f, created.id).await.state == JobState::Cancelled {
            break;
        }
        f.clock.advance(Duration::seconds(1));
        exec.step(&due(&created), None).await.expect("drain");
    }
    let done = job(&f, created.id).await;
    assert_eq!(done.state, JobState::Cancelled);
    assert_eq!((done.counts.succeeded, done.counts.cancelled), (0, 1));
    assert_eq!(item_state(&f, created.id).await, "CANCELLED");
}

#[tokio::test]
async fn a_replica_that_loses_its_lease_between_the_last_look_and_the_effect_lets_no_effect_through()
 {
    let f = fixture().await;
    // the job's id decides its shard: the old replica's lease is set once the job exists
    let (created, run) = one_item_job(&f, "one-tx-lost-lease", None).await;
    let shard = Shard::of(created.id);
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'replica-old', generation = 11, \
         expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .execute(&f.w.db.pool)
    .await
    .expect("the old replica holds the shard");
    let fence = |owner: &str, generation| ShardFence {
        owner: owner.to_owned(),
        lease: Lease { shard, generation },
    };
    let exec = executor(&f);

    let (held, holder) = hold(&f, HOLD_RUN, run).await;
    let step = tokio::spawn({
        let exec = Arc::clone(&exec);
        let due = due(&created);
        let stale = fence("replica-old", 11);
        async move { exec.step(&due, Some(stale)).await }
    });
    blocked_on(
        &f,
        holder,
        "runs",
        "the old replica waits for the run after its last look",
    )
    .await;
    // the shard changes hands in that window
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'replica-new', generation = 12, \
         expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .execute(&f.w.db.pool)
    .await
    .expect("a new owner");
    held.commit().await.expect("release the run");
    let stepped = step.await.expect("join").expect("step");

    assert!(matches!(stepped, Stepped::Refused(_)), "{stepped:?}");
    assert_ne!(
        status_of(&f, run).await,
        "CANCELLED",
        "the replica that lost its lease did not touch the run"
    );
    assert_eq!(item_state(&f, created.id).await, "PENDING");

    // the new owner applies it once, and records it with the effect
    f.clock.advance(Duration::seconds(1));
    let applied = exec
        .step(&due(&created), Some(fence("replica-new", 12)))
        .await
        .expect("the owner");
    assert_eq!(applied, Stepped::Acted);
    assert_eq!(item_state(&f, created.id).await, "SUCCEEDED");
    assert_eq!(status_of(&f, run).await, "CANCELLED");
    assert_eq!(cancellations(&f, run).await, 1);
}

#[tokio::test]
async fn a_process_that_dies_between_the_effect_and_the_record_of_its_item_leaves_neither() {
    let f = fixture().await;
    let (created, run) = one_item_job(&f, "one-tx-crash", None).await;
    let exec = executor(&f);

    // the record of the item waits for its row: everything before it in the step has been done
    let (held, holder) = hold(&f, HOLD_ITEM, created.id).await;
    let step = tokio::spawn({
        let exec = Arc::clone(&exec);
        let due = due(&created);
        async move { exec.step(&due, None).await }
    });
    let recording = blocked_on(&f, holder, "batch_items", "the step records its item").await;
    // the backend that waits for the item has written the effect already: it holds the run's row and wrote its journal
    // (a record taken before the effect would wait here with neither, and the test would prove nothing)
    let written: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT c.relname::text FROM pg_locks l JOIN pg_class c ON c.oid = l.relation \
         WHERE l.pid = $1 AND l.granted AND l.mode = 'RowExclusiveLock' \
           AND c.relname IN ('runs', 'run_events') ORDER BY 1",
    )
    .bind(recording)
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the locks of the step");
    assert_eq!(
        written,
        ["run_events", "runs"],
        "the effect is written before the item is locked"
    );
    // the process dies there: its task stops and its connection is cut inside PostgreSQL
    step.abort();
    let cut: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(recording)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("cut the connection");
    assert!(cut);
    held.commit().await.expect("release the item");
    let _ = step.await;

    let (status, item) = (status_of(&f, run).await, item_state(&f, created.id).await);
    assert!(
        status != "CANCELLED" && item == "PENDING",
        "the run is as the item says, neither the effect nor its record committed: the run {status}, the item {item}"
    );
    assert_eq!(cancellations(&f, run).await, 0);

    // a restarted process applies it once
    let restarted = executor(&f);
    for _ in 0..50 {
        if job(&f, created.id).await.state == JobState::Completed {
            break;
        }
        f.clock.advance(Duration::seconds(1));
        restarted.step(&due(&created), None).await.expect("step");
    }
    assert_eq!(job(&f, created.id).await.state, JobState::Completed);
    assert_eq!(item_state(&f, created.id).await, "SUCCEEDED");
    assert_eq!(status_of(&f, run).await, "CANCELLED");
    assert_eq!(cancellations(&f, run).await, 1);
}

// --- a replay, and a failure after the commit (review of D2219) -------------------------------------------------------

#[tokio::test]
async fn an_effect_an_older_engine_applied_is_recorded_though_the_job_was_cancelled_since() {
    let f = fixture().await;
    let (created, run) = one_item_job(&f, "one-tx-replay-cancel", None).await;
    // an older engine applied the effect under the item's key and died before it recorded the item
    let key = format!("batch:{}:{}", created.id, run);
    f.w.runs
        .operate(
            &f.w.admin,
            sagat_domain::execution::RunId::new(run),
            sagat_application::runs::RunOperation::Cancel,
            &format!("batch {}: {}", created.id, created.reason),
            None,
            &key,
            &RequestContext::default(),
        )
        .await
        .expect("the effect of the older engine");
    assert_eq!(item_state(&f, created.id).await, "PENDING");
    let exec = executor(&f);

    // the new engine looks, then waits for the run; the job is cancelled in that window
    let (held, holder) = hold(&f, HOLD_RUN, run).await;
    let step = tokio::spawn({
        let exec = Arc::clone(&exec);
        let due = due(&created);
        async move { exec.step(&due, None).await }
    });
    blocked_on(
        &f,
        holder,
        "runs",
        "the step waits for the run after its last look",
    )
    .await;
    service(&f)
        .cancel(
            &f.w.admin,
            created.id,
            "stop this batch job",
            &RequestContext::default(),
        )
        .await
        .expect("cancel");
    held.commit().await.expect("release the run");
    let stepped = step.await.expect("join").expect("step");

    assert_eq!(stepped, Stepped::Acted, "the replay is recorded");
    assert_eq!(
        item_state(&f, created.id).await,
        "SUCCEEDED",
        "the item says what happened to its run: it was cancelled before the job was"
    );
    assert_eq!(status_of(&f, run).await, "CANCELLED");
    assert_eq!(cancellations(&f, run).await, 1);
}

/// The run service, and then a failure of the call after its transaction committed (a read that follows the commit, the
/// way `operate` reads the run back).
#[derive(Clone)]
struct FailsAfterCommit(Arc<World>);

impl ItemActions for FailsAfterCommit {
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        ItemActions::apply(&self.0.runs, request).await?;
        Err(AppError::Unavailable)
    }
}

#[tokio::test]
async fn an_outcome_that_committed_stands_when_the_call_fails_after_the_commit() {
    let f = fixture().await;
    let (created, run) = one_item_job(&f, "one-tx-after-commit", None).await;
    let exec = BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        FailsAfterCommit(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        f.clock.clone(),
        ExecutorSettings::default(),
    );

    let stepped = exec.step(&due(&created), None).await.expect("step");

    assert_eq!(
        stepped,
        Stepped::Acted,
        "the item was recorded with its effect: the failure that came after changes nothing"
    );
    assert_eq!(item_state(&f, created.id).await, "SUCCEEDED");
    assert_eq!(status_of(&f, run).await, "CANCELLED");
}

#[tokio::test]
async fn an_item_is_recorded_only_with_its_own_run() {
    let f = fixture().await;
    let (created, run) = one_item_job(&f, "one-tx-own-run", None).await;
    let other = runs_started(&f, 1).await[0];
    let finished = sagat_domain::audit::AuditRecord::by(
        &f.w.admin,
        sagat_domain::audit::AuditAction::BatchFinished,
        sagat_domain::audit::ResourceType::BatchJob,
        created.id.to_string(),
        None,
        OffsetDateTime::now_utc(),
    );
    // the item of `run`, asked of another run: the record must refuse it and the effect must not commit
    let ordinal: i32 = sqlx::query_scalar("SELECT ordinal FROM batch_items WHERE job_id = $1")
        .bind(created.id)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the ordinal");
    let settlement = sagat_application::batch::ItemSettlement::new(
        created.tenant_id,
        created.id,
        u32::try_from(ordinal).expect("ordinal"),
        run,
        None,
        OffsetDateTime::now_utc(),
        finished,
    );
    let key = format!("batch:{}:{}", created.id, other);
    let ctx = RequestContext::default();
    let applied = ItemActions::apply(
        &f.w.runs,
        &ItemRequest {
            principal: &f.w.admin,
            job: &created,
            run: other,
            key: &key,
            payload: None,
            ctx: &ctx,
            settlement: &settlement,
        },
    )
    .await;

    assert!(applied.is_err(), "{applied:?}");
    assert_ne!(
        status_of(&f, other).await,
        "CANCELLED",
        "the other run is untouched"
    );
    assert_eq!(item_state(&f, created.id).await, "PENDING");
}

// --- every single-run path rolls its effect back when its record is refused ---------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Path {
    Signal,
    Review,
    Rerun,
    AdHocRerun,
    Update,
}

/// A run the action of `path` applies to, and the job over it, selected.
async fn job_for(f: &Fixture, path: Path) -> (JobRecord, Uuid) {
    use crate::support::engine::{
        DECLARED_OPERATION, activity, declared_waiter, document, wait_node, with,
    };
    use sagat_application::batch::{BatchWrite, NamedPayload};
    use sagat_domain::batch::{Action, Justification, Selector};
    use serde_json::json;

    let tag = Uuid::now_v7().simple().to_string();
    let (run, action, step_id, signal, update) = match path {
        Path::Signal => {
            let (flow, _) =
                f.w.publish(document(
                    &format!("payments-{tag}"),
                    "w",
                    vec![
                        with(
                            wait_node("w", "payment.confirmed", "input.paymentId", 300_000, None),
                            "next",
                            json!("notify"),
                        ),
                        activity("notify"),
                    ],
                ))
                .await;
            let run = f.w.start(flow, &json!({"paymentId": "p-1"})).await;
            let signal = NamedPayload {
                name: "payment.confirmed".to_owned(),
                body: br#"{"ok": true}"#.to_vec(),
            };
            (run, Action::Signal, None, Some(signal), None)
        }
        Path::Review => {
            let (flow, _) =
                f.w.publish(document(
                    &format!("held-{tag}"),
                    "charge",
                    vec![activity("charge")],
                ))
                .await;
            let run = f.w.start(flow, &json!({})).await;
            (
                run,
                Action::MarkManualReview,
                Some("charge".to_owned()),
                None,
                None,
            )
        }
        Path::Rerun => {
            let run = f.w.start(f.workflow, &json!({})).await;
            f.w.operate(run, sagat_application::runs::RunOperation::Cancel)
                .await
                .expect("end it");
            (run, Action::Rerun, None, None, None)
        }
        Path::AdHocRerun => {
            let run = crate::support::schedules::ad_hoc_run(&f.w, &tag[..12]).await;
            (run, Action::Rerun, None, None, None)
        }
        Path::Update => {
            let (flow, _) =
                f.w.publish(declared_waiter(&format!("accounts-{tag}")))
                    .await;
            let run =
                f.w.start(flow, &json!({"account": {"balance": 10}, "k": "s"}))
                    .await;
            let update = NamedPayload {
                name: DECLARED_OPERATION.to_owned(),
                body: json!({"allowed": true, "nextState": {"balance": 25}})
                    .to_string()
                    .into_bytes(),
            };
            (run, Action::Update, None, None, Some(update))
        }
    };
    f.clock
        .set(OffsetDateTime::now_utc() + Duration::seconds(2));
    let created = service(f)
        .create(
            &f.w.admin,
            f.w.namespace.id(),
            BatchWrite {
                selector: Selector::run_ids(vec![run.as_uuid()]).expect("selector"),
                action,
                step_id,
                instance_path: Vec::new(),
                signal,
                update,
                drop_business_id: false,
                repin: false,
                reason: Justification::new("apply the action to the run").expect("reason"),
                max_items: None,
                rate: None,
            },
            &format!("one-tx-{path:?}-{tag}"),
            &RequestContext::default(),
        )
        .await
        .expect("create the job");
    let exec = executor(f);
    within("the selection", StdDuration::from_secs(120), async {
        loop {
            f.clock.advance(Duration::seconds(1));
            if exec.step(&due(&created), None).await.expect("select") == Stepped::Acted {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(job(f, created.id).await.counts.selected, 1, "{path:?}");
    f.clock.advance(Duration::seconds(1));
    (created, run.as_uuid())
}

/// What the effect of any action on `run` leaves: the audit records that name it and the runs that repeat it.
async fn effects_on(f: &Fixture, run: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM audit_records \
                 WHERE action IN ('run.signalled', 'run.marked_for_review', 'run.updated', 'run.rerun') \
                   AND (resource_id = $1::text OR resource_id LIKE $1::text || '/%')) \
              + (SELECT count(*) FROM runs WHERE rerun_of = $1)",
    )
    .bind(run)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the effects")
}

async fn a_refused_record_rolls_the_effect_of(path: Path) {
    let f = fixture().await;
    let (created, run) = job_for(&f, path).await;
    let before = effects_on(&f, run).await;
    let exec = executor(&f);

    let (held, holder) = hold(&f, HOLD_RUN, run).await;
    let step = tokio::spawn({
        let exec = Arc::clone(&exec);
        let due = due(&created);
        async move { exec.step(&due, None).await }
    });
    blocked_on(
        &f,
        holder,
        "runs",
        &format!("{path:?}: the step waits for the run"),
    )
    .await;
    service(&f)
        .cancel(
            &f.w.admin,
            created.id,
            "stop this batch job",
            &RequestContext::default(),
        )
        .await
        .expect("cancel");
    held.commit().await.expect("release the run");
    let stepped = step.await.expect("join").expect("step");

    assert!(
        matches!(stepped, Stepped::Refused(_)),
        "{path:?}: {stepped:?}"
    );
    assert_eq!(
        effects_on(&f, run).await,
        before,
        "{path:?}: the effect did not commit after the job was cancelled"
    );
    assert_eq!(item_state(&f, created.id).await, "PENDING", "{path:?}");
}

#[tokio::test]
async fn a_refused_record_rolls_back_a_signal() {
    a_refused_record_rolls_the_effect_of(Path::Signal).await;
}

#[tokio::test]
async fn a_refused_record_rolls_back_a_review() {
    a_refused_record_rolls_the_effect_of(Path::Review).await;
}

#[tokio::test]
async fn a_refused_record_rolls_back_a_rerun() {
    a_refused_record_rolls_the_effect_of(Path::Rerun).await;
}

#[tokio::test]
async fn a_refused_record_rolls_back_an_ad_hoc_rerun() {
    a_refused_record_rolls_the_effect_of(Path::AdHocRerun).await;
}

#[tokio::test]
async fn a_refused_record_rolls_back_an_update() {
    a_refused_record_rolls_the_effect_of(Path::Update).await;
}

// --- the order of a RERUN that keeps its business id, and of an Update (D2223) -------------------------------------------

/// The writing statements, in order, of the transaction that records the job's one item, as the tripwire saw them.
async fn writes_of_the_item(
    f: &Fixture,
    wire: &crate::tripwire::Tripwire,
    created: &JobRecord,
) -> Vec<String> {
    let exec = executor(f);
    // the log of the tripwire keeps what earlier watches saw: only what this step writes is looked at
    let seen = wire.writes().await.len();
    wire.watch().await;
    let stepped = exec.step(&due(created), None).await.expect("step");
    wire.disarm().await;
    assert_eq!(stepped, Stepped::Acted);
    let writes: Vec<_> = wire.writes().await.into_iter().skip(seen).collect();
    let xid = writes
        .iter()
        .find(|(_, _, table)| table == "batch_items")
        .map(|(xid, _, _)| *xid)
        .expect("a transaction that records the item");
    writes
        .into_iter()
        .filter(|(other, _, _)| *other == xid)
        .map(|(_, _, table)| table)
        .collect()
}

fn first(writes: &[String], table: &str) -> usize {
    writes
        .iter()
        .position(|written| written == table)
        .unwrap_or_else(|| panic!("no write of {table}: {writes:?}"))
}

#[tokio::test]
async fn a_rerun_that_keeps_its_id_and_an_update_write_in_the_order_of_the_slots() {
    use sagat_application::runs::{BusinessStart, StartOptions, StartRun};
    use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};

    let f = fixture().await;
    let wire = crate::tripwire::Tripwire::install(&f.w.db.pool).await;

    // RERUN with KEEP: the source held its id and ended, so the rerun takes the binding over
    let id = format!("order-{}", Uuid::now_v7());
    let source =
        f.w.runs
            .start_with_options(
                &f.w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow: f.workflow,
                    version: None,
                    input: b"{}",
                    correlation_id: None,
                    deadline: None,
                    key: &format!("source-{}", Uuid::now_v7()),
                },
                StartOptions {
                    business: Some(BusinessStart {
                        id: BusinessId::new(&id).expect("an id"),
                        conflict: ConflictPolicy::Fail,
                        reuse: ReusePolicy::AllowDuplicate,
                        window: ReuseWindow::default(),
                    }),
                    ..StartOptions::default()
                },
                &RequestContext::default(),
            )
            .await
            .expect("the source")
            .run
            .info
            .run_id;
    f.w.operate(source, sagat_application::runs::RunOperation::Cancel)
        .await
        .expect("end it");
    let created = {
        use sagat_application::batch::BatchWrite;
        use sagat_domain::batch::{Action, Justification, Selector};
        f.clock
            .set(OffsetDateTime::now_utc() + Duration::seconds(2));
        let created = service(&f)
            .create(
                &f.w.admin,
                f.w.namespace.id(),
                BatchWrite {
                    selector: Selector::run_ids(vec![source.as_uuid()]).expect("selector"),
                    action: Action::Rerun,
                    step_id: None,
                    instance_path: Vec::new(),
                    signal: None,
                    update: None,
                    drop_business_id: false,
                    repin: false,
                    reason: Justification::new("start the run again").expect("reason"),
                    max_items: None,
                    rate: None,
                },
                "one-tx-order-rerun",
                &RequestContext::default(),
            )
            .await
            .expect("the job");
        let exec = executor(&f);
        within("the selection", StdDuration::from_secs(120), async {
            loop {
                f.clock.advance(Duration::seconds(1));
                if exec.step(&due(&created), None).await.expect("select") == Stepped::Acted {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        f.clock.advance(Duration::seconds(1));
        created
    };
    let rerun = writes_of_the_item(&f, &wire, &created).await;
    println!("RERUN keeping its id writes {rerun:?}");
    let (binding, run, item, job_row) = (
        first(&rerun, "business_ids"),
        first(&rerun, "runs"),
        first(&rerun, "batch_items"),
        first(&rerun, "batch_jobs"),
    );
    assert!(
        binding < run && run < item && item < job_row,
        "the binding (slot 3), the new run (6), then the item and the job (8a): {rerun:?}"
    );
    // the tenant's usage row is the shared row of slot 8b, flushed at the commit; `quota_transaction_deltas` is a row of
    // this transaction's own (keyed by its xid), written all along, which no other transaction ever waits for
    let usage = first(&rerun, "tenant_retained_usage");
    assert!(job_row < usage, "the usage rows last (8b): {rerun:?}");

    // UPDATE
    let (created, _) = job_for(&f, Path::Update).await;
    let update = writes_of_the_item(&f, &wire, &created).await;
    println!("UPDATE writes {update:?}");
    let item = first(&update, "batch_items");
    let effect_after_item = update[item..].iter().any(|t| {
        matches!(
            t.as_str(),
            "run_events" | "runs" | "workflow_operation_states" | "operation_update_receipts"
        )
    });
    assert!(
        !effect_after_item,
        "the effect (journal, state, receipt, run), then the item: {update:?}"
    );
    assert!(
        first(&update, "batch_jobs") < first(&update, "tenant_retained_usage"),
        "the usage rows last (8b): {update:?}"
    );
}

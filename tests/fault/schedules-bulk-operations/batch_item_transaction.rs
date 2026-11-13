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

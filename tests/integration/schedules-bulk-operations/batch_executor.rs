//! The executor of batch jobs (`T018-046`, `T018-047`, `FR-018-029..031`, `FR-018-033`): the selection
//! of the runs that waits for the transactions that were open at creation, the action applied to one
//! item per slot with its effect, its outcome and the counters of the job, what is skipped and what
//! fails, the loss of the creator's authority, the cancellation, a refusal that is tried again and one
//! that is not, and a lease that was lost. Real PostgreSQL, the real run service, a clock moved by hand.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use sagat_application::batch::DueJob;
use sagat_application::batch::{BatchService, BatchWrite, JobRecord};
use sagat_application::batch_executor::{
    BatchExecutor, ExecutorSettings, ItemActions, ItemEffect, ItemRequest, Stepped,
};
use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedules::ShardFence;
use sagat_application::shards::Lease;
use sagat_domain::batch::{Action, Filter, JobState, Justification, Limits, Rate, Selector};
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::{PgBatchStore, PgIdentityStore};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::firing::{Fixture, fixture};
use crate::support::engine::World;
use crate::support::schedules::TestClock;

/// The run service of the world, as the executor's single-run use cases.
#[derive(Clone)]
pub(crate) struct Runs(pub(crate) Arc<World>);

impl ItemActions for Runs {
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        ItemActions::apply(&self.0.runs, request).await
    }
}

pub(crate) type Executor = BatchExecutor<PgBatchStore, Runs, PgIdentityStore, TestClock>;

fn executor_with(f: &Fixture, settings: ExecutorSettings) -> Executor {
    BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        Runs(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        f.clock.clone(),
        settings,
    )
}

pub(crate) fn executor(f: &Fixture) -> Executor {
    executor_with(f, ExecutorSettings::default())
}

pub(crate) fn service(f: &Fixture) -> BatchService<PgBatchStore, TestClock> {
    BatchService::new(PgBatchStore::new(f.w.db.pool.clone()), f.clock.clone())
}

/// The clock of the test follows the database's time, which the jobs are stamped with, and then
/// moves by hand.
pub(crate) fn live(f: &Fixture) {
    f.clock
        .set(OffsetDateTime::now_utc() + Duration::seconds(2));
}

pub(crate) fn due(job: &JobRecord) -> DueJob {
    DueJob {
        tenant_id: job.tenant_id,
        id: job.id,
    }
}

pub(crate) fn write(action: Action, selector: Selector) -> BatchWrite {
    BatchWrite {
        selector,
        action,
        step_id: None,
        instance_path: Vec::new(),
        signal: None,
        update: None,
        drop_business_id: false,
        repin: false,
        reason: Justification::new("clean up the runs that are stuck").expect("reason"),
        max_items: None,
        rate: None,
    }
}

pub(crate) fn everything() -> Selector {
    Selector::filter(Filter::default()).expect("selector")
}

pub(crate) async fn create(f: &Fixture, write: BatchWrite, key: &str) -> JobRecord {
    service(f)
        .create(
            &f.w.admin,
            f.w.namespace.id(),
            write,
            key,
            &RequestContext::default(),
        )
        .await
        .expect("create the job")
}

pub(crate) async fn runs_started(f: &Fixture, n: usize) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for _ in 0..n {
        ids.push(f.w.start(f.workflow, &json!({})).await.as_uuid());
    }
    ids.sort_unstable();
    ids
}

pub(crate) async fn job(f: &Fixture, id: Uuid) -> JobRecord {
    service(f).get(&f.w.admin, id).await.expect("job")
}

/// `(ordinal, state, reason, error code)` of the items, in order.
pub(crate) async fn items(
    f: &Fixture,
    id: Uuid,
) -> Vec<(i32, String, Option<String>, Option<String>)> {
    sqlx::query_as(
        "SELECT ordinal, state, reason, error_code FROM batch_items WHERE job_id = $1 ORDER BY ordinal",
    )
    .bind(id)
    .fetch_all(&f.w.db.pool)
    .await
    .expect("items")
}

pub(crate) async fn status_of(f: &Fixture, run: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
        .bind(run)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("status")
}

/// Steps until the selection has taken a page. It waits, as it should, for a transaction of the time of
/// creation that is still open: the world has loops of its own, and the horizon it waits for is the
/// whole server's, so a transaction of another test on the same server holds it too. The bound is
/// therefore a time, not a number of round trips (500 of them passed in a few milliseconds on a busy
/// runner and the test gave up while the other test's transaction was still open).
pub(crate) async fn select(exec: &Executor, job: &JobRecord) {
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if exec.step(&due(job), None).await.expect("step") == Stepped::Acted {
            return;
        }
        assert!(
            std::time::Instant::now() < give_up,
            "the selection never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Steps the job until it has nothing more to do now; returns how many steps acted.
pub(crate) async fn settle(exec: &Executor, job: &JobRecord) -> u32 {
    let mut acted = 0;
    for _ in 0..200 {
        match exec.step(&due(job), None).await.expect("step") {
            Stepped::Acted => acted += 1,
            Stepped::Idle | Stepped::Refused(_) => return acted,
        }
    }
    panic!("the job never became idle");
}

/// Finishes the only open run, the way a worker does, so that it is `SUCCEEDED`.
pub(crate) async fn finish_the_only_open_run(w: &World) {
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("a completes");
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("b completes");
}

// --- the selection --------------------------------------------------------------------------------

#[tokio::test]
async fn the_selection_takes_the_runs_in_pages_by_id_and_stops_at_max_items() {
    let f = fixture().await;
    let all = runs_started(&f, 5).await;
    live(&f);
    let exec = executor_with(
        &f,
        ExecutorSettings {
            select_page: 2,
            ..ExecutorSettings::default()
        },
    );
    let capped = create(
        &f,
        BatchWrite {
            max_items: Some(3),
            ..write(Action::Cancel, everything())
        },
        "select-capped",
    )
    .await;

    select(&exec, &capped).await;
    assert_eq!(
        job(&f, capped.id).await.state,
        JobState::Selecting,
        "two of three"
    );
    assert_eq!(
        exec.step(&due(&capped), None).await.expect("page 2"),
        Stepped::Acted
    );

    let running = job(&f, capped.id).await;
    assert_eq!(running.state, JobState::Running, "maxItems reached");
    assert_eq!(running.counts.selected, 3);
    assert!(running.started_at.is_some());
    let selected: Vec<Uuid> =
        sqlx::query_scalar("SELECT run_id FROM batch_items WHERE job_id = $1 ORDER BY ordinal")
            .bind(capped.id)
            .fetch_all(&f.w.db.pool)
            .await
            .expect("items");
    assert_eq!(
        selected,
        all[..3].to_vec(),
        "the first three by id, in order of ordinal"
    );
}

#[tokio::test]
async fn a_job_over_no_run_runs_and_completes_with_no_item() {
    let f = fixture().await;
    live(&f);
    let exec = executor(&f);
    let empty = create(&f, write(Action::Cancel, everything()), "select-empty").await;

    select(&exec, &empty).await;
    assert!(settle(&exec, &empty).await >= 1, "the end of the job");

    let done = job(&f, empty.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(done.counts.selected, 0);
    assert!(done.finished_at.is_some());
}

#[tokio::test]
async fn the_selection_waits_for_the_transactions_that_were_open_when_the_job_was_created() {
    let f = fixture().await;
    runs_started(&f, 2).await;
    live(&f);
    let exec = executor(&f);
    // a transaction that has an identity and is still open
    let mut open = f.w.db.pool.begin().await.expect("begin");
    sqlx::query("SELECT txid_current()")
        .fetch_one(&mut *open)
        .await
        .expect("take an identity");
    let waiting = create(&f, write(Action::Cancel, everything()), "select-horizon").await;

    assert_eq!(
        exec.step(&due(&waiting), None).await.expect("step"),
        Stepped::Idle,
        "a transaction of the time of creation could still commit a run"
    );
    assert_eq!(job(&f, waiting.id).await.state, JobState::Selecting);
    assert_eq!(job(&f, waiting.id).await.counts.selected, 0);

    open.rollback().await.expect("the transaction ends");
    select(&exec, &waiting).await;
    assert_eq!(job(&f, waiting.id).await.counts.selected, 2);
}

// --- one item per slot ----------------------------------------------------------------------------

#[tokio::test]
async fn an_item_is_applied_per_slot_with_its_effect_its_outcome_and_the_counters() {
    let f = fixture().await;
    let all = runs_started(&f, 3).await;
    live(&f);
    let exec = executor(&f);
    let job_ = create(
        &f,
        BatchWrite {
            rate: Some(Rate::per_second(1).expect("rate")),
            ..write(Action::Cancel, everything())
        },
        "apply-per-slot",
    )
    .await;
    select(&exec, &job_).await;

    assert_eq!(
        exec.step(&due(&job_), None).await.expect("first"),
        Stepped::Acted
    );
    let after_one = job(&f, job_.id).await;
    assert_eq!(
        (after_one.counts.succeeded, after_one.counts.selected),
        (1, 3)
    );
    assert_eq!(status_of(&f, all[0]).await, "CANCELLED");
    assert_eq!(status_of(&f, all[1]).await, "RUNNING", "one item, one slot");
    assert_eq!(
        exec.step(&due(&job_), None).await.expect("same instant"),
        Stepped::Idle,
        "the next slot has not come"
    );
    assert!(after_one.next_slot_at >= f.clock.now() + Duration::milliseconds(900));

    f.clock.advance(Duration::seconds(1));
    assert_eq!(
        exec.step(&due(&job_), None).await.expect("second"),
        Stepped::Acted
    );
    f.clock.advance(Duration::seconds(1));
    assert_eq!(
        exec.step(&due(&job_), None).await.expect("third"),
        Stepped::Acted
    );
    assert_eq!(
        job(&f, job_.id).await.state,
        JobState::Completed,
        "the last item ends the job in its own transaction"
    );

    let done = job(&f, job_.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(
        (
            done.counts.selected,
            done.counts.succeeded,
            done.counts.skipped,
            done.counts.failed
        ),
        (3, 3, 0, 0)
    );
    for run in &all {
        assert_eq!(status_of(&f, *run).await, "CANCELLED");
    }
    let audits: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, actor FROM audit_records WHERE action IN ('run.cancel_requested', 'batch.finished') \
         ORDER BY created_at, id",
    )
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        audits
            .iter()
            .filter(|a| a.0 == "run.cancel_requested" && a.1 == "alice")
            .count(),
        3
    );
    assert_eq!(audits.iter().filter(|a| a.0 == "batch.finished").count(), 1);
}

#[tokio::test]
async fn a_run_that_has_ended_or_is_not_there_is_skipped_with_its_reason() {
    let f = fixture().await;
    let ended = runs_started(&f, 1).await[0];
    finish_the_only_open_run(&f.w).await;
    let open = runs_started(&f, 1).await[0];
    let nobody = Uuid::new_v4();
    live(&f);
    let exec = executor(&f);
    let mut ids = vec![ended, open, nobody];
    ids.sort_unstable();
    let job_ = create(
        &f,
        write(Action::Cancel, Selector::run_ids(ids).expect("selector")),
        "skip-reasons",
    )
    .await;

    // the unknown id selects nothing: only the runs that exist become items
    select(&exec, &job_).await;
    for _ in 0..4 {
        f.clock.advance(Duration::seconds(1));
        settle(&exec, &job_).await;
    }

    let done = job(&f, job_.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(
        (
            done.counts.selected,
            done.counts.succeeded,
            done.counts.skipped
        ),
        (2, 1, 1)
    );
    let rows = items(&f, job_.id).await;
    let skipped: Vec<_> = rows.iter().filter(|r| r.1 == "SKIPPED").collect();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].2.as_deref(), Some("ALREADY_TERMINAL"));
    assert_eq!(
        status_of(&f, ended).await,
        "SUCCEEDED",
        "a finished run is left as it ended"
    );
    assert_eq!(status_of(&f, open).await, "CANCELLED");
}

#[tokio::test]
async fn a_crash_between_the_effect_and_its_record_costs_a_repeat_that_changes_nothing() {
    let f = fixture().await;
    let runs = runs_started(&f, 1).await;
    live(&f);
    let exec = executor(&f);
    let job_ = create(
        &f,
        write(Action::Cancel, everything()),
        "crash-after-effect",
    )
    .await;
    select(&exec, &job_).await;
    // the effect committed, the process died before the outcome was recorded
    let creator = sagat_domain::identity::Principal::new(
        job_.tenant_id,
        sagat_domain::identity::Subject::parse("alice").expect("subject"),
        sagat_domain::identity::Role::Admin,
    );
    let key = format!("batch:{}:{}", job_.id, runs[0]);
    let ctx = RequestContext::default();
    Runs(Arc::clone(&f.w))
        .apply(&ItemRequest {
            principal: &creator,
            job: &job_,
            run: runs[0],
            key: &key,
            payload: None,
            ctx: &ctx,
        })
        .await
        .expect("the effect");
    assert_eq!(items(&f, job_.id).await[0].1, "PENDING");

    settle(&exec, &job_).await;

    assert_eq!(job(&f, job_.id).await.state, JobState::Completed);
    assert_eq!(items(&f, job_.id).await[0].1, "SUCCEEDED");
    let cancellations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.cancel_requested' AND resource_id = $1",
    )
    .bind(runs[0].to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(cancellations, 1, "the repeat of the key changed nothing");
}

// --- authority, cancellation ----------------------------------------------------------------------

#[tokio::test]
async fn a_creator_who_loses_a_permission_fails_the_job_and_nothing_more_is_applied() {
    let f = fixture().await;
    let creator =
        f.w.member("batch-creator", sagat_domain::identity::Role::Operator)
            .await;
    let all = runs_started(&f, 3).await;
    live(&f);
    let exec = executor(&f);
    let job_ = service(&f)
        .create(
            &creator,
            f.w.namespace.id(),
            write(Action::Cancel, everything()),
            "authority-lost",
            &RequestContext::default(),
        )
        .await
        .expect("create");
    select(&exec, &job_).await;
    assert_eq!(
        exec.step(&due(&job_), None).await.expect("first"),
        Stepped::Acted
    );
    sqlx::query(
        "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = 'batch-creator'",
    )
    .bind(job_.tenant_id.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("demote");

    f.clock.advance(Duration::seconds(1));
    assert_eq!(
        exec.step(&due(&job_), None).await.expect("next"),
        Stepped::Acted
    );

    let failed = job(&f, job_.id).await;
    assert_eq!(failed.state, JobState::Failed);
    assert_eq!(
        failed.failure_reason.map(|r| r.as_str()),
        Some("AUTHORITY_REVOKED")
    );
    assert_eq!(
        (
            failed.counts.succeeded,
            failed.counts.cancelled,
            failed.counts.selected
        ),
        (1, 2, 3),
        "what was applied stays, the rest is cancelled, the counters equal the items"
    );
    assert_eq!(status_of(&f, all[0]).await, "CANCELLED");
    assert_eq!(status_of(&f, all[1]).await, "RUNNING");
    assert_eq!(status_of(&f, all[2]).await, "RUNNING");
}

#[tokio::test]
async fn cancelling_a_job_marks_the_items_that_have_not_run_and_undoes_nothing() {
    let f = fixture().await;
    let all = runs_started(&f, 4).await;
    live(&f);
    let exec = executor_with(
        &f,
        ExecutorSettings {
            drain_page: 2,
            ..ExecutorSettings::default()
        },
    );
    let job_ = create(&f, write(Action::Cancel, everything()), "cancel-midway").await;
    select(&exec, &job_).await;
    assert_eq!(
        exec.step(&due(&job_), None).await.expect("first"),
        Stepped::Acted
    );
    service(&f)
        .cancel(
            &f.w.admin,
            job_.id,
            "stop this batch job",
            &RequestContext::default(),
        )
        .await
        .expect("cancel");
    f.clock.advance(Duration::seconds(1));

    assert_eq!(
        exec.step(&due(&job_), None).await.expect("drain 1"),
        Stepped::Acted
    );
    assert_eq!(
        job(&f, job_.id).await.state,
        JobState::Cancelling,
        "two of three marked"
    );
    assert_eq!(
        exec.step(&due(&job_), None).await.expect("drain 2"),
        Stepped::Acted
    );

    let done = job(&f, job_.id).await;
    assert_eq!(done.state, JobState::Cancelled);
    assert_eq!((done.counts.succeeded, done.counts.cancelled), (1, 3));
    assert_eq!(
        status_of(&f, all[0]).await,
        "CANCELLED",
        "the item that ran stays done"
    );
    assert_eq!(status_of(&f, all[1]).await, "RUNNING");
    assert_eq!(
        items(&f, job_.id)
            .await
            .iter()
            .filter(|r| r.1 == "CANCELLED")
            .count(),
        3
    );
}

// --- refusals ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_refusal_for_now_is_tried_again_after_a_backoff_and_succeeds_once_there_is_room() {
    let f = fixture().await;
    let source = runs_started(&f, 1).await[0];
    finish_the_only_open_run(&f.w).await;
    live(&f);
    let exec = executor(&f);
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'active_runs', 0, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("no room");
    let job_ = create(
        &f,
        write(
            Action::Rerun,
            Selector::run_ids(vec![source]).expect("selector"),
        ),
        "rerun-deferred",
    )
    .await;
    select(&exec, &job_).await;

    assert_eq!(
        exec.step(&due(&job_), None).await.expect("refused"),
        Stepped::Acted
    );

    let deferred = job(&f, job_.id).await;
    assert_eq!(
        (deferred.counts.deferred, deferred.counts.succeeded),
        (1, 0)
    );
    let row = &items(&f, job_.id).await[0];
    assert_eq!(
        (row.1.as_str(), row.2.as_deref()),
        ("DEFERRED", Some("QUOTA_EXCEEDED"))
    );
    f.clock.advance(Duration::seconds(1));
    assert_eq!(
        exec.step(&due(&job_), None).await.expect("backing off"),
        Stepped::Idle,
        "not before the retry"
    );

    sqlx::query("DELETE FROM quota_limits WHERE tenant_id = $1 AND resource = 'active_runs'")
        .bind(f.w.admin.tenant_id().as_uuid())
        .execute(&f.w.db.pool)
        .await
        .expect("room again");
    f.clock.advance(Duration::seconds(10));
    settle(&exec, &job_).await;

    let done = job(&f, job_.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!((done.counts.succeeded, done.counts.deferred), (1, 0));
    let new_run: Option<Uuid> =
        sqlx::query_scalar("SELECT new_run_id FROM batch_items WHERE job_id = $1")
            .bind(job_.id)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("new run");
    assert!(new_run.is_some(), "a rerun says which run it made");
}

#[tokio::test]
async fn an_item_refused_too_often_fails_instead_of_waiting_for_ever() {
    let f = fixture().await;
    let source = runs_started(&f, 1).await[0];
    finish_the_only_open_run(&f.w).await;
    live(&f);
    let exec = executor_with(
        &f,
        ExecutorSettings {
            limits: Limits {
                item_retries: 2,
                ..Limits::default()
            },
            ..ExecutorSettings::default()
        },
    );
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'active_runs', 0, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("no room, ever");
    let job_ = create(
        &f,
        write(
            Action::Rerun,
            Selector::run_ids(vec![source]).expect("selector"),
        ),
        "rerun-exhausted",
    )
    .await;

    select(&exec, &job_).await;
    for _ in 0..12 {
        f.clock.advance(Duration::minutes(6));
        settle(&exec, &job_).await;
    }

    let done = job(&f, job_.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(
        (
            done.counts.failed,
            done.counts.deferred,
            done.counts.succeeded
        ),
        (1, 0, 0)
    );
    let row = &items(&f, job_.id).await[0];
    assert_eq!(
        (row.1.as_str(), row.3.as_deref()),
        ("FAILED", Some("RETRIES_EXHAUSTED"))
    );
}

// --- the lease --------------------------------------------------------------------------------------

#[tokio::test]
async fn a_replica_that_lost_its_lease_records_nothing_and_the_new_owner_goes_on() {
    let f = fixture().await;
    let all = runs_started(&f, 1).await;
    live(&f);
    let exec = executor(&f);
    let job_ = create(&f, write(Action::Cancel, everything()), "stale-lease-job").await;
    let shard = Shard::of(job_.id);
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'replica-new', generation = 9, \
         expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .execute(&f.w.db.pool)
    .await
    .expect("the shard has a new owner");
    let stale = ShardFence {
        owner: "replica-old".to_owned(),
        lease: Lease {
            shard,
            generation: 8,
        },
    };
    let holder = ShardFence {
        owner: "replica-new".to_owned(),
        lease: Lease {
            shard,
            generation: 9,
        },
    };

    let refused = exec.step(&due(&job_), Some(stale)).await.expect("step");

    assert!(matches!(refused, Stepped::Refused(_)), "{refused:?}");
    assert_eq!(
        job(&f, job_.id).await.counts.selected,
        0,
        "nothing was selected"
    );
    for _ in 0..500 {
        f.clock.advance(Duration::seconds(1));
        exec.step(&due(&job_), Some(holder.clone()))
            .await
            .expect("step");
        if job(&f, job_.id).await.state == JobState::Completed {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(job(&f, job_.id).await.state, JobState::Completed);
    assert_eq!(status_of(&f, all[0]).await, "CANCELLED");
}

#[tokio::test]
async fn a_replica_that_lost_its_lease_neither_applies_nor_records_an_item_and_the_new_owner_goes_on()
 {
    let f = fixture().await;
    let all = runs_started(&f, 1).await;
    live(&f);
    let exec = executor(&f);
    let job_ = create(&f, write(Action::Cancel, everything()), "stale-item-record").await;
    select(&exec, &job_).await;
    let shard = Shard::of(job_.id);
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'replica-new', generation = 12, \
         expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .execute(&f.w.db.pool)
    .await
    .expect("the shard has a new owner");
    let stale = ShardFence {
        owner: "replica-old".to_owned(),
        lease: Lease {
            shard,
            generation: 11,
        },
    };
    let holder = ShardFence {
        owner: "replica-new".to_owned(),
        lease: Lease {
            shard,
            generation: 12,
        },
    };

    // the old replica is stopped before it touches the run
    let refused = exec.step(&due(&job_), Some(stale)).await.expect("step");

    assert!(matches!(refused, Stepped::Refused(_)), "{refused:?}");
    assert_eq!(
        items(&f, job_.id).await[0].1,
        "PENDING",
        "nothing is recorded without the lease"
    );
    assert_eq!(job(&f, job_.id).await.counts.succeeded, 0);
    assert_ne!(
        status_of(&f, all[0]).await,
        "CANCELLED",
        "the replica that lost its lease did not apply the action"
    );

    f.clock.advance(Duration::seconds(1));
    exec.step(&due(&job_), Some(holder.clone()))
        .await
        .expect("holder");
    f.clock.advance(Duration::seconds(1));
    exec.step(&due(&job_), Some(holder)).await.expect("end");

    assert_eq!(job(&f, job_.id).await.state, JobState::Completed);
    assert_eq!(items(&f, job_.id).await[0].1, "SUCCEEDED");
    let cancellations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.cancel_requested' AND resource_id = $1",
    )
    .bind(all[0].to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        cancellations, 1,
        "the old replica and the new owner cancelled the run once between them"
    );
}

#[tokio::test]
async fn the_last_look_before_an_effect_refuses_an_ended_item_a_stale_lease_and_a_cancelled_job() {
    use sagat_application::batch::BatchStore;
    use sagat_application::error::StoreError;

    let f = fixture().await;
    runs_started(&f, 2).await;
    live(&f);
    let exec = executor(&f);
    let job_ = create(&f, write(Action::Cancel, everything()), "last-look").await;
    select(&exec, &job_).await;
    let store = PgBatchStore::new(f.w.db.pool.clone());
    let ordinals: Vec<u32> = items(&f, job_.id)
        .await
        .into_iter()
        .map(|(n, ..)| u32::try_from(n).expect("ordinal"))
        .collect();
    let (first, second) = (ordinals[0], ordinals[1]);
    let conflict =
        |result: Result<(), StoreError>| matches!(result, Err(StoreError::StateConflict(_)));

    store
        .ensure_live(job_.tenant_id, job_.id, first, None)
        .await
        .expect("a pending item of a running job is live");

    // an item that has ended is not applied again
    assert_eq!(
        exec.step(&due(&job_), None).await.expect("first item"),
        Stepped::Acted
    );
    assert!(conflict(
        store
            .ensure_live(job_.tenant_id, job_.id, first, None)
            .await
    ));
    store
        .ensure_live(job_.tenant_id, job_.id, second, None)
        .await
        .expect("the other is still live");

    // a replica whose lease was taken over
    let shard = Shard::of(job_.id);
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'replica-new', generation = 12, \
         expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .execute(&f.w.db.pool)
    .await
    .expect("a new owner");
    let stale = ShardFence {
        owner: "replica-old".to_owned(),
        lease: Lease {
            shard,
            generation: 11,
        },
    };
    let holder = ShardFence {
        owner: "replica-new".to_owned(),
        lease: Lease {
            shard,
            generation: 12,
        },
    };
    let lost = store
        .ensure_live(job_.tenant_id, job_.id, second, Some(&stale))
        .await;
    assert!(
        matches!(lost, Err(StoreError::StateConflict(reason)) if reason.contains("lease")),
        "{lost:?}"
    );
    store
        .ensure_live(job_.tenant_id, job_.id, second, Some(&holder))
        .await
        .expect("the owner may");

    // a job that was cancelled stops its items before the run is touched
    service(&f)
        .cancel(
            &f.w.admin,
            job_.id,
            "stop this batch job",
            &RequestContext::default(),
        )
        .await
        .expect("cancel");
    assert!(conflict(
        store
            .ensure_live(job_.tenant_id, job_.id, second, Some(&holder))
            .await
    ));
}

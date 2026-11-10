//! The batch pass (`T018-046`, `FR-018-029..031`): which jobs a replica looks at, how much it does in a
//! round, and that the lease it acts under is checked in the transaction that records the step. Real
//! PostgreSQL, a real keeper, the clock of the database (the one the engine gives the pass), and a
//! rate high enough that the tests wait for the time of a slot and not for a long one.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::batch::{BatchService, BatchStore, BatchWrite, DueJob, DueJobs, JobRecord};
use sagat_application::batch_executor::{
    BatchExecutor, ExecutorSettings, ItemActions, ItemEffect, ItemRequest, Stepped,
};
use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_domain::batch::{Action, JobState, Rate, Selector};
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::{DatabaseClock, PgBatchStore, PgIdentityStore, PgRunStore};
use sagat_scheduler::{BatchPass, BatchRound, BatchSettings, Keeper, KeeperSettings, Placement};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::batch_executor::{Runs, everything, job, runs_started, status_of, write};
use crate::firing::{Fixture, fixture};
use crate::gap_store::SlotComesBetween;
use crate::support::bounded::within;
use crate::support::schedules::TestClock;

type Clock = DatabaseClock<SystemClock>;
type Pass = BatchPass<PgBatchStore, Runs, PgIdentityStore, Clock>;

/// One replica: the keeper that holds its shards and the pass that works on them.
struct Replica {
    keeper: Keeper<PgRunStore>,
    pass: Pass,
    placement: Placement,
}

async fn clock(f: &Fixture) -> Clock {
    let clock = DatabaseClock::new(f.w.db.pool.clone(), SystemClock);
    clock.sync().await.expect("the database clock");
    clock
}

fn replica_with(f: &Fixture, name: &str, clock: &Clock, settings: BatchSettings) -> Replica {
    let placement = Placement::new(format!("{name}-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    );
    let executor = BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        Runs(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        clock.clone(),
        ExecutorSettings::default(),
    );
    Replica {
        keeper,
        pass: BatchPass::new(executor, placement.clone(), settings),
        placement,
    }
}

fn replica(f: &Fixture, name: &str, clock: &Clock) -> Replica {
    replica_with(f, name, clock, BatchSettings::default())
}

impl Replica {
    async fn keep(&self) {
        self.keeper.round().await.expect("keeper round");
    }
}

async fn create(f: &Fixture, clock: &Clock, write: BatchWrite, key: &str) -> JobRecord {
    BatchService::new(PgBatchStore::new(f.w.db.pool.clone()), clock.clone())
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

/// Room for `jobs` jobs in progress at once: the deployment ceiling first, then the limit.
async fn room_for(f: &Fixture, jobs: i64) {
    for resource in ["batch_jobs", "tenant_batch_jobs"] {
        sqlx::query(
            "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
             VALUES ($1, $3, $2, 'operator')",
        )
        .bind(f.w.admin.tenant_id().as_uuid())
        .bind(jobs)
        .bind(resource)
        .execute(&f.w.db.pool)
        .await
        .expect("a higher ceiling");
    }
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'batch_jobs', $3, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .bind(jobs)
    .execute(&f.w.db.pool)
    .await
    .expect("room for the jobs");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, NULL, 'tenant_batch_jobs', $2, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(jobs)
    .execute(&f.w.db.pool)
    .await
    .expect("room for the jobs of the tenant");
}

/// A job at the highest rate, so that the wait for the next slot is a few milliseconds.
fn fast(action: Action, selector: Selector) -> BatchWrite {
    BatchWrite {
        rate: Some(Rate::MAX),
        ..write(action, selector)
    }
}

async fn state_of(f: &Fixture, id: Uuid) -> JobState {
    job(f, id).await.state
}

/// Runs rounds of the replicas (together, as they would run) until every job has ended, and returns
/// what each replica did, summed. The wait is for the time of a slot and for the transactions that
/// were open when the job was created; it ends in a panic that names the jobs left.
async fn until_ended(f: &Fixture, replicas: &[&Replica], jobs: &[Uuid]) -> Vec<BatchRound> {
    let mut totals = vec![BatchRound::default(); replicas.len()];
    for _ in 0..3_000 {
        let rounds =
            futures_util::future::join_all(replicas.iter().map(|replica| replica.pass.round()))
                .await;
        for (total, round) in totals.iter_mut().zip(rounds) {
            total.due += round.due;
            total.steps += round.steps;
            total.refused += round.refused;
            total.failed += round.failed;
        }
        let mut open = 0;
        for id in jobs {
            if !matches!(
                state_of(f, *id).await,
                JobState::Completed | JobState::Cancelled | JobState::Failed
            ) {
                open += 1;
            }
        }
        if open == 0 {
            return totals;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    panic!("the jobs never ended: {jobs:?}");
}

async fn cancellations_of(f: &Fixture, run: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.cancel_requested' AND resource_id = $1",
    )
    .bind(run.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit")
}

#[tokio::test]
async fn a_replica_that_holds_the_shards_selects_and_cancels_the_runs_of_a_job() {
    let f = fixture().await;
    let runs = runs_started(&f, 3).await;
    let clock = clock(&f).await;
    let a = replica(&f, "a", &clock);
    a.keep().await;
    let created = create(&f, &clock, fast(Action::Cancel, everything()), "pass-apply").await;

    let totals = until_ended(&f, &[&a], &[created.id]).await;

    let done = job(&f, created.id).await;
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(
        (
            done.counts.selected,
            done.counts.succeeded,
            done.counts.failed
        ),
        (3, 3, 0)
    );
    for run in &runs {
        assert_eq!(status_of(&f, *run).await, "CANCELLED");
        assert_eq!(cancellations_of(&f, *run).await, 1);
    }
    assert_eq!(totals[0].failed, 0);
    assert_eq!(
        totals[0].steps, 4,
        "one page of the selection, then one step for each of the three items"
    );
}

#[tokio::test]
async fn a_replica_that_holds_no_shard_does_nothing() {
    let f = fixture().await;
    let runs = runs_started(&f, 1).await;
    let clock = clock(&f).await;
    let idle = replica(&f, "idle", &clock);
    let created = create(&f, &clock, fast(Action::Cancel, everything()), "pass-idle").await;

    assert_eq!(idle.pass.round().await, BatchRound::default());
    assert_eq!(state_of(&f, created.id).await, JobState::Selecting);
    assert_ne!(status_of(&f, runs[0]).await, "CANCELLED");
}

#[tokio::test]
async fn two_replicas_split_the_jobs_by_shard_and_every_run_is_cancelled_once() {
    let f = fixture().await;
    let runs = runs_started(&f, 12).await;
    room_for(&f, 16).await;
    let clock = clock(&f).await;
    let mut jobs = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        let selector = Selector::run_ids(vec![*run]).expect("selector");
        jobs.push(
            create(
                &f,
                &clock,
                fast(Action::Cancel, selector),
                &format!("pass-split-{i}"),
            )
            .await,
        );
    }
    let a = replica(&f, "a", &clock);
    let b = replica(&f, "b", &clock);
    // each announces itself, the first gives back what is beyond its share, the second takes it
    a.keep().await;
    b.keep().await;
    a.keep().await;
    b.keep().await;
    assert_eq!(
        a.placement.owned().len() + b.placement.owned().len(),
        64,
        "every shard has an owner"
    );
    assert!(!a.placement.owned().is_empty() && !b.placement.owned().is_empty());
    let owners: BTreeMap<i16, String> = sqlx::query_as("SELECT shard, owner FROM scheduler_shards")
        .fetch_all(&f.w.db.pool)
        .await
        .expect("owners")
        .into_iter()
        .collect();
    let owned_by_a = jobs
        .iter()
        .filter(|j| {
            owners.get(&i16::from(Shard::of(j.id).index())) == Some(&a.placement.owner().to_owned())
        })
        .count();

    let ids: Vec<Uuid> = jobs.iter().map(|j| j.id).collect();
    let totals = until_ended(&f, &[&a, &b], &ids).await;

    assert_eq!((totals[0].failed, totals[1].failed), (0, 0));
    assert_eq!(
        usize::try_from(totals[0].steps).expect("steps"),
        2 * owned_by_a,
        "a replica acts on the jobs of its own shards: a page, then the item"
    );
    assert_eq!(
        usize::try_from(totals[0].steps + totals[1].steps).expect("steps"),
        2 * jobs.len(),
        "between them the replicas act on every job"
    );
    for run in &runs {
        assert_eq!(status_of(&f, *run).await, "CANCELLED");
        assert_eq!(
            cancellations_of(&f, *run).await,
            1,
            "cancelled once, not twice"
        );
    }
}

#[tokio::test]
async fn a_replica_that_lost_its_lease_is_refused_where_it_records_and_the_job_waits() {
    let f = fixture().await;
    let runs = runs_started(&f, 1).await;
    let clock = clock(&f).await;
    let a = replica(&f, "a", &clock);
    a.keep().await;
    let created = create(&f, &clock, fast(Action::Cancel, everything()), "pass-lease").await;
    let shard = i16::from(Shard::of(created.id).index());
    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner, generation FROM scheduler_shards WHERE shard = $1")
            .bind(shard)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the lease");
    assert_eq!(owner, a.placement.owner());

    // another replica takes the shard over; the first one's snapshot has not run out yet
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'usurper', generation = generation + 1 WHERE shard = $1",
    )
    .bind(shard)
    .execute(&f.w.db.pool)
    .await
    .expect("take the shard over");
    // a job whose selection still waits for a transaction is not due: the usurped replica is refused
    // in the first round in which it is
    let round = within(
        "the job is due and the usurped replica meets the new owner",
        StdDuration::from_secs(60),
        async {
            loop {
                let round = a.pass.round().await;
                if round.due > 0 {
                    return round;
                }
                tokio::time::sleep(StdDuration::from_millis(20)).await;
            }
        },
    )
    .await;
    assert_eq!((round.steps, round.refused, round.failed), (0, 1, 0));
    let waiting = job(&f, created.id).await;
    assert_eq!(waiting.state, JobState::Selecting);
    assert_eq!(waiting.counts.selected, 0, "nothing of the job changed");
    assert_ne!(status_of(&f, runs[0]).await, "CANCELLED");

    // the lease comes back to its holder: the job goes on
    sqlx::query("UPDATE scheduler_shards SET owner = $2, generation = $3 WHERE shard = $1")
        .bind(shard)
        .bind(&owner)
        .bind(generation)
        .execute(&f.w.db.pool)
        .await
        .expect("give the shard back");
    until_ended(&f, &[&a], &[created.id]).await;
    assert_eq!(state_of(&f, created.id).await, JobState::Completed);
    assert_eq!(status_of(&f, runs[0]).await, "CANCELLED");
}

#[tokio::test]
async fn a_round_takes_no_more_steps_than_its_ceiling_and_the_next_one_carries_on() {
    let f = fixture().await;
    let runs = runs_started(&f, 3).await;
    let clock = clock(&f).await;
    let a = replica_with(
        &f,
        "a",
        &clock,
        BatchSettings {
            round_actions: 1,
            ..BatchSettings::default()
        },
    );
    a.keep().await;
    let created = create(
        &f,
        &clock,
        fast(Action::Cancel, everything()),
        "pass-ceiling",
    )
    .await;

    let mut total = 0;
    for _ in 0..3_000 {
        let round = a.pass.round().await;
        assert!(
            round.steps <= 1,
            "a round takes one step at most: {round:?}"
        );
        total += round.steps;
        if state_of(&f, created.id).await == JobState::Completed {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }

    assert_eq!(state_of(&f, created.id).await, JobState::Completed);
    assert_eq!(total, 4, "the page and the three items, one step a round");
    for run in &runs {
        assert_eq!(status_of(&f, *run).await, "CANCELLED");
    }
}

// --- what is due -------------------------------------------------------------------------------------

/// An action that is always refused for now, as when the platform is at one of its limits.
#[derive(Clone)]
struct AtItsLimit;

impl ItemActions for AtItsLimit {
    async fn apply(&self, _: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        Err(AppError::QuotaExceeded("a limit of the test"))
    }
}

async fn ids_due(f: &Fixture, jobs: &[&JobRecord], limit: u32) -> Vec<Uuid> {
    let shards: Vec<Shard> = jobs.iter().map(|job| Shard::of(job.id)).collect();
    PgBatchStore::new(f.w.db.pool.clone())
        .due_jobs(&shards, limit)
        .await
        .expect("due jobs")
        .jobs
        .into_iter()
        .map(|due| due.id)
        .collect()
}

#[tokio::test]
async fn a_job_that_has_nothing_to_do_yet_does_not_keep_the_head_of_the_round() {
    let f = fixture().await;
    let (stuck_run, ready_run) = (
        f.w.start(f.workflow, &json!({})).await.as_uuid(),
        f.w.start(f.workflow, &json!({})).await.as_uuid(),
    );
    room_for(&f, 4).await;
    let clock = clock(&f).await;

    // the older job has an item the platform refused for now: it waits for its time
    let stuck = create(
        &f,
        &clock,
        fast(
            Action::Cancel,
            Selector::run_ids(vec![stuck_run]).expect("s"),
        ),
        "head-stuck",
    )
    .await;
    let refusing: BatchExecutor<PgBatchStore, AtItsLimit, PgIdentityStore, Clock> =
        BatchExecutor::new(
            PgBatchStore::new(f.w.db.pool.clone()),
            AtItsLimit,
            PgIdentityStore::new(f.w.db.pool.clone()),
            clock.clone(),
            ExecutorSettings::default(),
        );
    let due = |job: &JobRecord| DueJob {
        tenant_id: job.tenant_id,
        id: job.id,
    };
    for _ in 0..500 {
        if refusing.step(&due(&stuck), None).await.expect("step") == Stepped::Acted {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(5)).await;
    }
    // (the first step took the page of the selection; the next one meets the refusal)
    assert_eq!(
        refusing
            .step(&due(&stuck), None)
            .await
            .expect("refused item"),
        Stepped::Acted
    );
    assert_eq!(job(&f, stuck.id).await.counts.deferred, 1);

    // The slot of the stuck job (a few milliseconds after the refused item) passes, and then it is in neither answer of the
    // scan: not due, and not a wake-up either. A job that has nothing to do until its retry, said to be a slot ahead, would
    // have the loop look again at it and again, and a slot already past at the start of the scan is what keeps it out.
    let scan = within(
        "the slot of the stuck job passes",
        StdDuration::from_secs(30),
        async {
            loop {
                let scan = PgBatchStore::new(f.w.db.pool.clone())
                    .due_jobs(&[Shard::of(stuck.id)], 10)
                    .await
                    .expect("scan");
                if scan.next_slot_in.is_none() {
                    return scan;
                }
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        },
    )
    .await;
    assert!(
        scan.jobs.is_empty(),
        "nothing to do until the retry: not due, and no slot ahead: {scan:?}"
    );

    // the newer one has a run to cancel
    let ready = create(
        &f,
        &clock,
        fast(
            Action::Cancel,
            Selector::run_ids(vec![ready_run]).expect("s"),
        ),
        "head-ready",
    )
    .await;
    let working: BatchExecutor<PgBatchStore, Runs, PgIdentityStore, Clock> = BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        Runs(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        clock.clone(),
        ExecutorSettings::default(),
    );
    for _ in 0..500 {
        if working.step(&due(&ready), None).await.expect("step") == Stepped::Acted {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(5)).await;
    }

    // a round that can look at one job looks at the one that can do something
    assert_eq!(ids_due(&f, &[&stuck, &ready], 1).await, vec![ready.id]);
    assert_eq!(ids_due(&f, &[&stuck, &ready], 10).await, vec![ready.id]);

    // when the time of the refused item comes, the older job is due again, and first
    sqlx::query(
        "UPDATE batch_items SET next_attempt_at = clock_timestamp() - interval '1 second' \
         WHERE job_id = $1",
    )
    .bind(stuck.id)
    .execute(&f.w.db.pool)
    .await
    .expect("the time has come");
    within(
        "the older job is due again",
        StdDuration::from_secs(30),
        async {
            while ids_due(&f, &[&stuck, &ready], 10).await != vec![stuck.id, ready.id] {
                tokio::time::sleep(StdDuration::from_millis(20)).await;
            }
        },
    )
    .await;
}

#[tokio::test]
async fn a_selection_that_waits_for_a_transaction_is_not_due_until_it_can_go_on() {
    let f = fixture().await;
    f.w.start(f.workflow, &json!({})).await;
    let clock = clock(&f).await;
    // a transaction that has an identity, open when the job is created
    let mut held = f.w.db.pool.begin().await.expect("begin");
    sqlx::query("SELECT pg_current_xact_id()")
        .execute(&mut *held)
        .await
        .expect("an identity");
    let waiting = create(
        &f,
        &clock,
        fast(Action::Cancel, everything()),
        "waits-for-a-transaction",
    )
    .await;

    assert_eq!(ids_due(&f, &[&waiting], 10).await, Vec::<Uuid>::new());
    held.rollback().await.expect("end it");
    within(
        "the selection may go on",
        StdDuration::from_secs(60),
        async {
            while ids_due(&f, &[&waiting], 10).await != vec![waiting.id] {
                tokio::time::sleep(StdDuration::from_millis(20)).await;
            }
        },
    )
    .await;
}

#[tokio::test]
async fn a_job_being_cancelled_is_always_due() {
    let f = fixture().await;
    let clock = clock(&f).await;
    f.w.start(f.workflow, &json!({})).await;
    let job_ = create(
        &f,
        &clock,
        fast(Action::Cancel, everything()),
        "cancelled-and-due",
    )
    .await;
    BatchService::new(PgBatchStore::new(f.w.db.pool.clone()), clock.clone())
        .cancel(
            &f.w.admin,
            job_.id,
            "stop this batch job",
            &RequestContext::default(),
        )
        .await
        .expect("cancel");

    within("the drain is due", StdDuration::from_secs(30), async {
        while ids_due(&f, &[&job_], 10).await != vec![job_.id] {
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
    })
    .await;
}

// --- the wake-up for the next slot (G2043) ---------------------------------------------------------------

/// A job whose items are 1,000 seconds apart (`rate_milli` 1, the slowest the code names): after an item the next slot is
/// further ahead than any test lasts, so a test can look at the time to the slot, and at a round that finds nothing due,
/// without racing the slot.
fn slowest(action: Action, selector: Selector) -> BatchWrite {
    BatchWrite {
        rate: Some(Rate::from_milli(1).expect("the slowest rate")),
        ..write(action, selector)
    }
}

/// Moves the slot of a job to `seconds` from now by the clock of the database (negative: past), as the owner.
async fn move_slot(f: &Fixture, id: Uuid, seconds: i32) {
    sqlx::query(
        "UPDATE batch_jobs SET next_slot_at = clock_timestamp() + make_interval(secs => $2) WHERE id = $1",
    )
    .bind(id)
    .bind(f64::from(seconds))
    .execute(&f.w.db.pool)
    .await
    .expect("move the slot");
}

/// How long a job may take to select its runs. A selecting job waits for the oldest open transaction of the WHOLE PostgreSQL
/// server (the horizon is server-wide), and the tests of a suite share one server, so a test beside this one that holds a
/// transaction can hold the selection back: CI failed `the_engines_loop_applies_...` here once at 60 s on a loaded Linux runner
/// (the `counted` tests have the same wait and give it 120 s). The bound is a time, not a number of rounds.
const SELECTION_BOUND: StdDuration = StdDuration::from_secs(180);

/// Rounds of `replica`, one at a time, until the selection of the job is over and it is running; returns the round that
/// ended the selection (it goes on to apply the first item, whose slot has come: a step after a step in one round).
async fn until_running(f: &Fixture, replica: &Replica, id: Uuid) -> BatchRound {
    within("the job selects its runs", SELECTION_BOUND, async {
        let mut last = BatchRound::default();
        while state_of(f, id).await != JobState::Running {
            last = replica.pass.round().await;
            tokio::time::sleep(StdDuration::from_millis(10)).await;
        }
        last
    })
    .await
}

#[tokio::test]
async fn the_scan_of_the_due_jobs_says_how_long_until_the_soonest_slot_that_is_ahead_of_its_start()
{
    let f = fixture().await;
    runs_started(&f, 2).await;
    let clock = clock(&f).await;
    let a = replica(&f, "a", &clock);
    a.keep().await;
    let store = PgBatchStore::new(f.w.db.pool.clone());
    let first = Shard::new(0).expect("a shard");
    assert_eq!(
        store.due_jobs(&[first], 10).await.expect("none"),
        DueJobs::default(),
        "no job at all"
    );

    // a running job whose first item has been applied: its next slot is the whole interval of the rate ahead (1,000 s)
    let created = create(
        &f,
        &clock,
        slowest(Action::Cancel, everything()),
        "wake-store-ahead",
    )
    .await;
    until_running(&f, &a, created.id).await;
    let shard = Shard::of(created.id);
    let elsewhere = Shard::new((shard.index() + 1) % 64).expect("another shard");
    let seconds = |due: DueJobs| due.next_slot_in.map(|d| d.as_secs_f64());
    let scan = store.due_jobs(&[shard], 10).await.expect("ahead");
    assert!(scan.jobs.is_empty(), "its slot is ahead: it is not due");
    let ahead = scan.next_slot_in.expect("a slot ahead");
    assert!(
        ahead > StdDuration::from_secs(900) && ahead <= StdDuration::from_secs(1_000),
        "the interval of the slowest rate, by the clock of the database: {ahead:?}"
    );

    move_slot(&f, created.id, 30).await;
    let ahead = seconds(store.due_jobs(&[shard], 10).await.expect("30 s")).expect("a slot ahead");
    assert!(
        ahead > 29.0 && ahead <= 30.0,
        "30 s ahead by the clock of the database: {ahead}"
    );
    assert_eq!(
        store.due_jobs(&[elsewhere], 10).await.expect("elsewhere"),
        DueJobs::default(),
        "a job of a shard the replica does not own is not its to wake for"
    );
    assert_eq!(
        store.due_jobs(&[], 10).await.expect("no shard"),
        DueJobs::default()
    );
    assert_eq!(
        seconds(store.due_jobs(&[elsewhere, shard], 10).await.expect("both")).map(|s| s.floor()),
        Some(29.0),
        "the soonest over the shards it is asked about"
    );

    // a slot that has come is a due job and not a wake-up: the scan that finds the job due says no slot ahead
    move_slot(&f, created.id, -1).await;
    let scan = store.due_jobs(&[shard], 10).await.expect("due");
    assert_eq!(
        (
            scan.jobs.iter().map(|j| j.id).collect::<Vec<_>>(),
            scan.next_slot_in
        ),
        (vec![created.id], None)
    );

    // a job that is over has a slot ahead too (the last item moves it) and never calls for a wake-up
    sqlx::query(
        "UPDATE batch_jobs SET state = 'CANCELLED', finished_at = clock_timestamp(), \
                next_slot_at = clock_timestamp() + interval '30 seconds' WHERE id = $1",
    )
    .bind(created.id)
    .execute(&f.w.db.pool)
    .await
    .expect("end the job");
    assert_eq!(
        store.due_jobs(&[shard], 10).await.expect("over"),
        DueJobs::default(),
        "a finished job wakes nobody"
    );
}

/// Only an applied item moves a slot ahead, and only a running job has one: a job that is selecting is due from its creation
/// (or waits for a transaction, which no slot is for) and one that is being cancelled is moved back to its request. A slot
/// that is ahead on such a job is nobody's wake-up: the statement does not look for it.
#[tokio::test]
async fn a_slot_ahead_of_a_job_that_is_not_running_is_not_a_wake_up() {
    let f = fixture().await;
    f.w.start(f.workflow, &json!({})).await;
    let clock = clock(&f).await;
    let store = PgBatchStore::new(f.w.db.pool.clone());
    let created = create(
        &f,
        &clock,
        slowest(Action::Cancel, everything()),
        "wake-store-selecting",
    )
    .await;
    let shard = Shard::of(created.id);
    move_slot(&f, created.id, 30).await;
    assert_eq!(
        store.due_jobs(&[shard], 10).await.expect("selecting"),
        DueJobs::default(),
        "selecting"
    );
    sqlx::query(
        "UPDATE batch_jobs SET state = 'CANCELLING', next_slot_at = clock_timestamp() + interval '30 seconds' WHERE id = $1",
    )
    .bind(created.id)
    .execute(&f.w.db.pool)
    .await
    .expect("cancelling");
    assert_eq!(
        store.due_jobs(&[shard], 10).await.expect("cancelling"),
        DueJobs::default(),
        "cancelling"
    );
}

#[tokio::test]
async fn a_round_that_did_nothing_says_when_the_next_slot_comes_and_one_that_worked_does_not() {
    let f = fixture().await;
    runs_started(&f, 2).await;
    let clock = clock(&f).await;
    let a = replica(&f, "a", &clock);
    a.keep().await;
    assert_eq!(
        a.pass.round().await.next_due_in,
        None,
        "no job, nothing to wake for"
    );
    let created = create(
        &f,
        &clock,
        slowest(Action::Cancel, everything()),
        "wake-round-says",
    )
    .await;
    let applying = until_running(&f, &a, created.id).await;
    assert_eq!(
        (
            applying.due,
            applying.steps,
            applying.refused,
            applying.failed
        ),
        (1, 2, 0, 0),
        "the round that selected the runs applied the first item too, its slot having come"
    );
    assert_eq!(
        applying.next_due_in, None,
        "a round that worked is followed at once and does not ask"
    );

    let idle = a.pass.round().await;
    assert_eq!(
        (idle.due, idle.steps),
        (0, 0),
        "the next slot is a thousand seconds away"
    );
    let wait = idle.next_due_in.expect("the slot of the job is ahead");
    assert!(
        wait > StdDuration::from_secs(900) && wait <= StdDuration::from_secs(1_000),
        "1,000 seconds at the most, the interval of the rate: {wait:?}"
    );

    // the second and last item, once its slot has come: the job is over, its slot (ahead again) is nobody's
    move_slot(&f, created.id, -1).await;
    let last = a.pass.round().await;
    assert_eq!(last.steps, 1);
    assert_eq!(state_of(&f, created.id).await, JobState::Completed);
    let after = a.pass.round().await;
    assert_eq!(
        (after.due, after.steps, after.next_due_in),
        (0, 0, None),
        "a finished job's slot does not hold the loop's attention"
    );
}

/// The time between the two statements of a round (G2043): the scan of the due jobs found the job's slot ahead, and by the
/// time the round asks when the next slot comes it has come. A round that reads the time of its own start and not of its
/// second statement still has the slot; one that asks again, with a bound taken later, finds none and says nothing, and the
/// loop sleeps its whole poll while an item is due. The store hands the answer back only after the slot has come.
#[tokio::test]
async fn a_slot_that_comes_between_the_statements_of_an_idle_round_is_not_lost() {
    let f = fixture().await;
    runs_started(&f, 2).await;
    let clock = clock(&f).await;
    let a = replica(&f, "a", &clock);
    a.keep().await;
    let created = create(
        &f,
        &clock,
        slowest(Action::Cancel, everything()),
        "wake-gap-between",
    )
    .await;
    until_running(&f, &a, created.id).await;
    // the slot is five seconds ahead when the round's scan looks; the store makes it come before the answer is handed back
    move_slot(&f, created.id, 5).await;

    let executor = BatchExecutor::new(
        SlotComesBetween::new(f.w.db.pool.clone(), created.id),
        Runs(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        clock.clone(),
        ExecutorSettings::default(),
    );
    let pass = BatchPass::new(executor, a.placement.clone(), BatchSettings::default());
    let round = pass.round().await;

    assert_eq!(
        (round.due, round.steps, round.refused, round.failed),
        (0, 0, 0, 0),
        "the slot was ahead when the scan looked"
    );
    let wait = round
        .next_due_in
        .expect("the slot that came while the round was between its statements was lost");
    assert!(
        wait > StdDuration::ZERO && wait <= StdDuration::from_secs(5),
        "the time to the slot as the scan saw it: {wait:?}"
    );
    assert!(
        job_slot_has_come(&f, created.id).await,
        "and the slot did come, as the test made it"
    );
}

/// A slot that comes while the scan itself is running (G2043). The statement reads the database's clock once, at its start,
/// and every slot after that instant is ahead of the scan, whenever the scan looks at it: one that comes while the statement
/// runs is zero, not lost. The scan is given the instant it is decided at (`due_jobs_as_of`), taken before a job's slot
/// comes: what the slot coming while the statement runs is, with nothing in the way of the test but the order of two
/// statements (a lock on the table of items would hold the statement back before it reads the clock, not after).
#[tokio::test]
async fn a_slot_that_comes_while_the_scan_runs_is_a_zero_and_not_lost() {
    let f = fixture().await;
    let runs = runs_started(&f, 2).await;
    let clock = clock(&f).await;
    let a = replica(&f, "a", &clock);
    a.keep().await;
    // two items, the first applied: running, with the next slot 1,000 s away, and an item still to do
    let created = create(
        &f,
        &clock,
        slowest(
            Action::Cancel,
            Selector::run_ids(runs.clone()).expect("selector"),
        ),
        "barrier-zero",
    )
    .await;
    until_running(&f, &a, created.id).await;
    let shard = Shard::of(created.id);
    let store = PgBatchStore::new(f.w.db.pool.clone());

    let before: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the clock of the database");
    move_slot(&f, created.id, 0).await;
    let after: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the clock of the database");

    // decided at `before`, the slot that came afterwards is ahead of the scan, and it has come by the end of the statement
    let scan = store
        .due_jobs_as_of(&[shard], 10, Some(before))
        .await
        .expect("the scan");
    assert_eq!(
        (scan.jobs.len(), scan.next_slot_in),
        (0, Some(StdDuration::ZERO)),
        "the slot came after the instant the scan was decided at: not due by it, and zero, not lost"
    );
    // decided at `after`, the same slot had come by then: it is a due job (it has an item to do), and no slot ahead
    let scan = store
        .due_jobs_as_of(&[shard], 10, Some(after))
        .await
        .expect("the scan");
    assert_eq!(
        (
            scan.jobs.iter().map(|job| job.id).collect::<Vec<_>>(),
            scan.next_slot_in
        ),
        (vec![created.id], None),
        "a slot that had come by the instant of the scan is the job's turn, not a wake-up"
    );
}

async fn job_slot_has_come(f: &Fixture, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT next_slot_at <= clock_timestamp() FROM batch_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the slot")
}

/// The clock disagreement (G2043): the scan says a job is due by the clock of the database, the executor decides by its own
/// (`DatabaseClock`, which reads behind by up to half a round trip, and by whatever it has drifted since its last read). A due
/// job that the executor finds not due yet is a round with a due job and no step: the next round must come in a few
/// milliseconds, not after the poll, or the stall comes back whenever the clocks differ by a hair.
#[tokio::test]
async fn a_due_job_that_the_executor_finds_not_due_yet_is_looked_at_again_in_a_few_milliseconds() {
    let f = fixture().await;
    runs_started(&f, 2).await;
    let clock = clock(&f).await;
    let a = replica(&f, "a", &clock);
    a.keep().await;
    // the executor's clock reads an hour before the database's: the job is due by the one and not by the other
    let behind = TestClock::at(OffsetDateTime::now_utc() - time::Duration::hours(1));
    let executor = BatchExecutor::new(
        PgBatchStore::new(f.w.db.pool.clone()),
        Runs(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.db.pool.clone()),
        behind,
        ExecutorSettings::default(),
    );
    let pass = BatchPass::new(executor, a.placement.clone(), BatchSettings::default());
    let created = create(
        &f,
        &clock,
        write(Action::Cancel, everything()),
        "wake-gap-behind",
    )
    .await;
    // the selection does not look at the slot, so the job gets to running whatever the clock says
    within("the job selects its runs", SELECTION_BOUND, async {
        while state_of(&f, created.id).await != JobState::Running {
            pass.round().await;
            tokio::time::sleep(StdDuration::from_millis(10)).await;
        }
    })
    .await;

    let round = pass.round().await;
    assert_eq!(
        (round.due, round.steps, round.refused, round.failed),
        (1, 0, 0, 0),
        "due by the database's clock, not by the executor's: a round with a due job and no step"
    );
    let wait = round.next_due_in.expect(
        "a due job that the executor found not due yet would be looked at after the whole poll",
    );
    assert!(
        wait > StdDuration::ZERO && wait <= StdDuration::from_millis(50),
        "a few milliseconds, not the poll: {wait:?}"
    );
}

/// The engine's own loop, with a poll far longer than the test lets it run: the items of a lone job at the default rate
/// follow each other by their slots (50 ms). A loop that slept its poll after the round that found the next slot not yet
/// come (the measured defect: one item a second at the poll of 1 s) would have applied one item and waited for 8 s or more.
#[tokio::test]
async fn the_engines_loop_applies_the_items_of_a_lone_job_at_its_slots_and_not_at_its_poll() {
    let f = fixture().await;
    runs_started(&f, 4).await;
    let clock = clock(&f).await;
    let a = replica(&f, "a", &clock);
    a.keep().await;
    let created = create(
        &f,
        &clock,
        write(Action::Cancel, everything()),
        "wake-lone-job",
    )
    .await;
    until_running(&f, &a, created.id).await;

    let (stop, stopped) = tokio::sync::watch::channel(false);
    let looping = sagat_execution_engine::loops::run_batches(
        || sagat_execution_engine::loops::batch_round(&clock, &a.pass),
        StdDuration::from_secs(10),
        stopped,
    );
    let watching = async {
        while state_of(&f, created.id).await != JobState::Completed {
            tokio::time::sleep(StdDuration::from_millis(10)).await;
        }
        stop.send(true).expect("stop the loop");
    };
    within(
        "four items at the default rate, with a poll of 10 s",
        StdDuration::from_secs(5),
        async { tokio::join!(looping, watching) },
    )
    .await;

    let done = job(&f, created.id).await;
    assert_eq!(
        (
            done.counts.selected,
            done.counts.succeeded,
            done.counts.failed
        ),
        (4, 4, 0)
    );
}

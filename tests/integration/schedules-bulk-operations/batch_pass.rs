//! The batch pass (`T018-046`, `FR-018-029..031`): which jobs a replica looks at, how much it does in a
//! round, and that the lease it acts under is checked in the transaction that records the step. Real
//! PostgreSQL, a real keeper, the clock of the database (the one the engine gives the pass), and a
//! rate high enough that the tests wait for the time of a slot and not for a long one.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::batch::{BatchService, BatchWrite, JobRecord};
use sagat_application::batch_executor::{BatchExecutor, ExecutorSettings};
use sagat_application::clock::SystemClock;
use sagat_application::identity::RequestContext;
use sagat_domain::batch::{Action, JobState, Rate, Selector};
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::{DatabaseClock, PgBatchStore, PgIdentityStore, PgRunStore};
use sagat_scheduler::{BatchPass, BatchRound, BatchSettings, Keeper, KeeperSettings, Placement};
use uuid::Uuid;

use crate::batch_executor::{Runs, everything, job, runs_started, status_of, write};
use crate::firing::{Fixture, fixture};

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
    let round = a.pass.round().await;
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

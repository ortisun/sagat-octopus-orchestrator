//! The schedule pass (`T018-017`, `FR-018-005`): which schedules a replica looks at, and that the
//! lease it acts under is checked in the transaction that acts. Real PostgreSQL, a real keeper, a
//! clock the test moves by hand.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use sagat_application::schedule_firing::ScheduleFiring;
use sagat_domain::schedules::Ceilings;
use sagat_domain::sharding::Shard;
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{PgIdentityStore, PgRunStore, PgScheduleStore};
use sagat_scheduler::{
    Keeper, KeeperSettings, Placement, SchedulePass, ScheduleRound, ScheduleSettings,
};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::firing::{Fixture, Starts, TestClock, fixture_from, hourly};

type Pass = SchedulePass<PgScheduleStore, SpecCalendar, TestClock, PgIdentityStore, Starts>;

/// One replica: the keeper that holds its shards and the pass that works on them.
struct Replica {
    keeper: Keeper<PgRunStore>,
    pass: Pass,
    placement: Placement,
}

fn replica(f: &Fixture, name: &str) -> Replica {
    let placement = Placement::new(format!("{name}-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(3_600),
        },
    );
    let firing = ScheduleFiring::new(
        PgScheduleStore::new(f.w.db.pool.clone()),
        SpecCalendar,
        f.clock.clone(),
        PgIdentityStore::new(f.w.db.pool.clone()),
        Starts(Arc::clone(&f.w)),
        Ceilings::default(),
    );
    Replica {
        keeper,
        pass: SchedulePass::new(firing, placement.clone(), ScheduleSettings::default()),
        placement,
    }
}

impl Replica {
    async fn keep(&self) {
        self.keeper.round().await.expect("keeper round");
    }
}

/// An hour boundary a few hours ago. The pass finds what is due by the database clock and the
/// firing decides by the clock of the test, so the test clock starts where the first time of an
/// hourly schedule is already in the past for both.
fn some_hours_ago() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    OffsetDateTime::from_unix_timestamp(now - now % 3_600 - 3 * 3_600).expect("instant")
}

async fn runs_of(f: &Fixture, schedule: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM runs WHERE schedule_id = $1")
        .bind(schedule)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("runs")
}

#[tokio::test]
async fn a_replica_that_holds_the_shards_starts_the_run_of_a_due_schedule() {
    let f = fixture_from(some_hours_ago()).await;
    let schedule = f.create("hourly", &hourly(), &json!({}), None).await;
    let a = replica(&f, "a");
    a.keep().await;

    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(
        a.pass.round().await,
        ScheduleRound {
            due: 1,
            steps: 1,
            refused: 0,
            failed: 0
        }
    );
    assert_eq!(f.runs(&schedule).await.len(), 1);
    assert_eq!(
        a.pass.round().await.steps,
        0,
        "the same time is not started twice"
    );
    assert_eq!(f.runs(&schedule).await.len(), 1);
}

#[tokio::test]
async fn a_replica_that_holds_no_shard_does_nothing() {
    let f = fixture_from(some_hours_ago()).await;
    let schedule = f.create("hourly", &hourly(), &json!({}), None).await;
    let idle = replica(&f, "idle");

    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(idle.pass.round().await, ScheduleRound::default());
    assert_eq!(runs_of(&f, schedule.id.as_uuid()).await, 0);
}

#[tokio::test]
async fn two_replicas_split_the_schedules_by_shard_and_every_time_starts_one_run() {
    let f = fixture_from(some_hours_ago()).await;
    let mut schedules = Vec::new();
    for i in 0..12 {
        schedules.push(
            f.create(&format!("s{i}"), &hourly(), &json!({}), None)
                .await,
        );
    }
    let a = replica(&f, "a");
    let b = replica(&f, "b");
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
    let owned_by_a = schedules
        .iter()
        .filter(|s| {
            owners.get(&i16::from(Shard::of(s.id.as_uuid()).index()))
                == Some(&a.placement.owner().to_owned())
        })
        .count();

    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    let (ra, rb) = tokio::join!(a.pass.round(), b.pass.round());
    assert_eq!((ra.failed, rb.failed), (0, 0));
    assert_eq!(
        usize::try_from(ra.steps).expect("steps"),
        owned_by_a,
        "a replica acts on the schedules of its own shards"
    );
    assert_eq!(
        usize::try_from(ra.steps + rb.steps).expect("steps"),
        schedules.len(),
        "between them the replicas act on every schedule"
    );
    for schedule in &schedules {
        assert_eq!(
            runs_of(&f, schedule.id.as_uuid()).await,
            1,
            "{} started one run",
            schedule.name.as_str()
        );
    }
}

#[tokio::test]
async fn a_replica_that_lost_its_lease_is_refused_where_it_acts_and_the_time_stays_owed() {
    let f = fixture_from(some_hours_ago()).await;
    let schedule = f.create("hourly", &hourly(), &json!({}), None).await;
    let a = replica(&f, "a");
    a.keep().await;
    let shard = i16::from(Shard::of(schedule.id.as_uuid()).index());
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
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    let round = a.pass.round().await;
    assert_eq!((round.steps, round.refused), (0, 1));
    assert_eq!(runs_of(&f, schedule.id.as_uuid()).await, 0);
    assert_eq!(
        f.read(&schedule).await.state_version,
        schedule.state_version,
        "nothing of the schedule changed"
    );

    // the lease comes back to its holder: the owed time is acted on
    sqlx::query("UPDATE scheduler_shards SET owner = $2, generation = $3 WHERE shard = $1")
        .bind(shard)
        .bind(&owner)
        .bind(generation)
        .execute(&f.w.db.pool)
        .await
        .expect("give the shard back");
    assert_eq!(a.pass.round().await.steps, 1);
    assert_eq!(runs_of(&f, schedule.id.as_uuid()).await, 1);
}

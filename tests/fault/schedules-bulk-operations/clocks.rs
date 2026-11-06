//! Process clocks that disagree with the database's (`T018-020`, `FR-018-005`: "due-ness uses the
//! database clock"). The pass finds what is due by the database, but the firing plans the times it
//! acts on with the clock it is given: a process that runs hours ahead would start, or swallow into
//! a summary, times the database has not reached, and one that runs behind would not act on times
//! that are owed. The engine therefore gives the firing a [`DatabaseClock`], moved to the database's
//! time before every round.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use sagat_application::clock::Clock;
use sagat_application::schedule_firing::{Fired, ScheduleFiring};
use sagat_application::schedules::ScheduleRecord;
use sagat_domain::schedules::Ceilings;
use sagat_execution_engine::loops::schedule_round;
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{DatabaseClock, PgIdentityStore, PgScheduleStore};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, SchedulePass, ScheduleSettings};
use serde_json::json;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::schedules::{Fixture, Starts, TestClock, fixture_from};

type Skewed = ScheduleFiring<
    PgScheduleStore,
    SpecCalendar,
    DatabaseClock<TestClock>,
    PgIdentityStore,
    Starts,
>;

async fn database_now(pool: &PgPool) -> OffsetDateTime {
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("the database clock")
}

/// A process whose own clock runs `by` ahead of the database's (behind, if negative) and whose
/// firing reads the time from a [`DatabaseClock`] over it.
struct Process {
    clock: DatabaseClock<TestClock>,
    firing: Skewed,
}

fn process(f: &Fixture, now: OffsetDateTime, by: Duration) -> Process {
    let clock = DatabaseClock::new(f.w.db.pool.clone(), TestClock::at(now + by));
    let firing = ScheduleFiring::new(
        PgScheduleStore::new(f.w.db.pool.clone()),
        SpecCalendar,
        clock.clone(),
        PgIdentityStore::new(f.w.db.pool.clone()),
        Starts(Arc::clone(&f.w)),
        Ceilings::default(),
    );
    Process { clock, firing }
}

/// An hourly schedule whose times fall 30 minutes before and after the database's `now` (and every
/// hour from there), so that no time sits within 29 minutes of the moment the test runs.
async fn schedule_around(f: &Fixture, now: OffsetDateTime) -> ScheduleRecord {
    let ms = now.unix_timestamp_nanos() / 1_000_000;
    let offset = (ms + 1_800_000).rem_euclid(3_600_000);
    // created two hours back: the first time is 90 minutes ago, the second 30 minutes ago
    f.clock.set(now - Duration::hours(2));
    f.create(
        "around-now",
        &json!({"intervals": [{"everyMs": 3_600_000, "offsetMs": i64::try_from(offset).expect("offset")}]}),
        &json!({"catchupMode": "ONE"}),
        None,
    )
    .await
}

async fn drain(process: &Process, f: &Fixture, schedule: &ScheduleRecord) {
    let due = f.due(schedule);
    for _ in 0..50 {
        match process.firing.fire(&due, None).await.expect("fire") {
            Fired::Acted | Fired::Refused(_) => {}
            Fired::Idle => return,
        }
    }
    panic!("the schedule never became idle");
}

/// The invariants of acting on the database's time, whichever way the process clock is wrong.
async fn assert_acted_on_database_time(
    f: &Fixture,
    schedule: &ScheduleRecord,
    begun: OffsetDateTime,
) {
    let runs = f.runs(schedule).await;
    assert_eq!(
        runs.len(),
        1,
        "the time the database had reached is started once (catch-up ONE)"
    );
    let latest: OffsetDateTime = sqlx::query_scalar(
        "SELECT max(greatest(nominal_at, coalesce(summary_last_at, nominal_at))) \
         FROM schedule_actions WHERE schedule_id = $1",
    )
    .bind(schedule.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("latest");
    assert!(
        latest <= begun,
        "an action was taken for {latest}, a time the database had not reached when the test began ({begun})"
    );
    assert_eq!(
        f.read(schedule).await.next_action_at,
        Some(begun + Duration::minutes(30)),
        "the next time is the first one after the database's now, not one the process clock reached"
    );
}

#[tokio::test]
async fn a_process_whose_clock_runs_hours_ahead_does_not_act_on_times_the_database_has_not_reached()
{
    let f = fixture_from(OffsetDateTime::now_utc()).await;
    let begun = database_now(&f.w.db.pool).await;
    let begun = begun - Duration::nanoseconds(i64::from(begun.nanosecond() % 1_000_000));
    let schedule = schedule_around(&f, begun).await;
    let ahead = process(&f, begun, Duration::hours(3));
    ahead.clock.sync().await.expect("sync");

    drain(&ahead, &f, &schedule).await;
    assert_acted_on_database_time(&f, &schedule, begun).await;
}

#[tokio::test]
async fn a_process_whose_clock_runs_hours_behind_still_acts_on_the_times_that_are_owed() {
    let f = fixture_from(OffsetDateTime::now_utc()).await;
    let begun = database_now(&f.w.db.pool).await;
    let begun = begun - Duration::nanoseconds(i64::from(begun.nanosecond() % 1_000_000));
    let schedule = schedule_around(&f, begun).await;
    let behind = process(&f, begun, -Duration::hours(3));
    behind.clock.sync().await.expect("sync");

    drain(&behind, &f, &schedule).await;
    assert_acted_on_database_time(&f, &schedule, begun).await;
}

#[tokio::test]
async fn a_synced_clock_reads_the_database_whatever_the_process_clock_says() {
    let f = fixture_from(OffsetDateTime::now_utc()).await;
    // How far this host's clock is from the database's own (a container's clock may differ by a
    // little), measured with a process clock that is not skewed on purpose.
    let baseline = DatabaseClock::new(
        f.w.db.pool.clone(),
        TestClock::at(OffsetDateTime::now_utc()),
    )
    .sync()
    .await
    .expect("sync");
    for by in [Duration::hours(3), -Duration::hours(3), Duration::ZERO] {
        let skewed = DatabaseClock::new(
            f.w.db.pool.clone(),
            TestClock::at(OffsetDateTime::now_utc() + by),
        );
        let offset = skewed.sync().await.expect("sync");
        assert!(
            (offset + by - baseline).abs() < Duration::seconds(1),
            "a process {by} off from the database measured {offset} (baseline {baseline})"
        );
        let apart = (skewed.now() - database_now(&f.w.db.pool).await).abs();
        assert!(apart < Duration::seconds(1), "{by}: still {apart} apart");
    }
}

#[tokio::test]
async fn a_clock_that_cannot_reach_the_database_keeps_the_last_time_it_knew() {
    let f = fixture_from(OffsetDateTime::now_utc()).await;
    let own = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&f.w.db.url)
        .await
        .expect("a pool of its own");
    let local = TestClock::at(OffsetDateTime::now_utc() + Duration::hours(3));
    let clock = DatabaseClock::new(own.clone(), local.clone());
    clock.sync().await.expect("sync");
    let known = clock.now();
    own.close().await;

    assert!(clock.sync().await.is_err(), "a closed pool cannot be asked");
    assert_eq!(clock.now(), known, "the offset it knew is kept");
    local.advance(Duration::seconds(2));
    assert_eq!(
        clock.now(),
        known + Duration::seconds(2),
        "and time goes on"
    );
}

/// A replica: its keeper has taken the shards, and its pass fires with `firing`.
async fn replica(
    f: &Fixture,
    firing: Skewed,
) -> SchedulePass<PgScheduleStore, SpecCalendar, DatabaseClock<TestClock>, PgIdentityStore, Starts>
{
    let placement = Placement::new(format!("skewed-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(3_600),
        },
    );
    keeper.round().await.expect("the replica takes its shards");
    SchedulePass::new(firing, placement, ScheduleSettings::default())
}

#[tokio::test]
async fn the_round_of_the_engine_acts_on_the_database_time_of_a_replica_whose_clock_is_hours_fast()
{
    // `schedule_round` is what the engine runs each time: it moves the clock to the database's time
    // and then runs the pass. Nothing here syncs by hand.
    let f = fixture_from(OffsetDateTime::now_utc()).await;
    let begun = database_now(&f.w.db.pool).await;
    let begun = begun - Duration::nanoseconds(i64::from(begun.nanosecond() % 1_000_000));
    let schedule = schedule_around(&f, begun).await;
    let Process { clock, firing } = process(&f, begun, Duration::hours(3));
    assert!(!clock.is_synced(), "the clock starts as the process clock");
    let pass = replica(&f, firing).await;

    let round = schedule_round(&clock, &pass).await;
    assert!(clock.is_synced());
    assert_eq!((round.due, round.failed), (1, 0), "{round:?}");
    assert_acted_on_database_time(&f, &schedule, begun).await;
}

#[tokio::test]
async fn a_round_whose_clock_was_never_measured_does_nothing() {
    // The database cannot be asked for the time (this clock's pool is closed) although the pass
    // can read its schedules: acting on the process clock, 3 hours fast, is what must not happen.
    let f = fixture_from(OffsetDateTime::now_utc()).await;
    let begun = database_now(&f.w.db.pool).await;
    let begun = begun - Duration::nanoseconds(i64::from(begun.nanosecond() % 1_000_000));
    let schedule = schedule_around(&f, begun).await;
    let unreachable = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&f.w.db.url)
        .await
        .expect("a pool of its own");
    unreachable.close().await;
    let clock = DatabaseClock::new(unreachable, TestClock::at(begun + Duration::hours(3)));
    let firing = ScheduleFiring::new(
        PgScheduleStore::new(f.w.db.pool.clone()),
        SpecCalendar,
        clock.clone(),
        PgIdentityStore::new(f.w.db.pool.clone()),
        Starts(Arc::clone(&f.w)),
        Ceilings::default(),
    );
    let pass = replica(&f, firing).await;
    let before = f.read(&schedule).await;

    let round = schedule_round(&clock, &pass).await;
    assert!(!clock.is_synced());
    assert_eq!(
        (round.due, round.steps, round.failed),
        (0, 0, 1),
        "{round:?}"
    );
    assert!(f.runs(&schedule).await.is_empty());
    assert_eq!(f.outcomes(&schedule).await, Vec::<(String, i64)>::new());
    let after = f.read(&schedule).await;
    assert_eq!(
        (after.next_action_at, after.state_version),
        (before.next_action_at, before.state_version),
        "the schedule was not touched"
    );
}

//! A process that dies while it acts on a time of a schedule (`T018-020`, `SC-018-002`,
//! `FR-018-004`, `FR-018-005`, `NFR-018-001`): whichever write it was cut off at, nothing of the
//! action is left behind, the time stays owed, and the next pass acts on it exactly once. The
//! matrix runs over the two transactions of the pass that write the most: the one that starts a run
//! and the one that records a skipped time next to an open run.
//!
//! A stale fence, a duplicated pass, a lost reply and a duplicated backlog are here too, each ending
//! in one run and one action per nominal time. There is no lost hint to test: a schedule is found by
//! the pass's own poll of the database and no broker message announces it (the pass tests run with
//! no broker at all), so nothing a broker loses can delay or repeat a firing.
//!
//! Not killed at each write here (the other decision-only steps share the structure of the skipped
//! time: one transaction with the fence, the compare-and-swap, the action row and the new state):
//! buffer, coalesce, drop, reconcile, auto-pause and the backfill step. See gap G1018.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;

use sagat_application::schedule_firing::{Fired, ScheduleFiring};
use sagat_application::schedules::{DueSchedule, ScheduleRecord};
use sagat_domain::execution::RunId;
use sagat_domain::schedules::Ceilings;
use sagat_domain::sharding::Shard;
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{PgIdentityStore, PgScheduleStore};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, SchedulePass, ScheduleSettings};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use tokio::sync::Barrier;
use uuid::Uuid;

use crate::support::schedules::{
    Fixture, Starts, fixture_from, hourly, outcomes_of, some_hours_ago,
};
use crate::tripwire::{Tripwire, table_digests};

/// The run a schedule started for `nominal`, and everything that has to be true of it: its
/// provenance, the scope and key it was admitted under, who started it, and a journal that replays
/// to its projections.
async fn assert_one_scheduled_run(f: &Fixture, schedule: &ScheduleRecord, nominal: OffsetDateTime) {
    let runs = f.runs(schedule).await;
    assert_eq!(runs.len(), 1, "exactly one run for the time");
    let (owner, at, kind): (Uuid, OffsetDateTime, String) = sqlx::query_as(
        "SELECT schedule_id, schedule_nominal_at, schedule_action_kind FROM runs WHERE id = $1",
    )
    .bind(runs[0])
    .fetch_one(&f.w.db.pool)
    .await
    .expect("provenance");
    assert_eq!(
        (owner, at, kind.as_str()),
        (schedule.id.as_uuid(), nominal, "SCHEDULED")
    );
    let actor: String = sqlx::query_scalar(
        "SELECT actor FROM audit_records WHERE action = 'run.started' AND resource_id = $1",
    )
    .bind(runs[0].to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(actor, format!("system:schedule:{}", schedule.id));
    let (scope, key): (String, String) =
        sqlx::query_as("SELECT scope, idempotency_key FROM command_dedupe WHERE resource_id = $1")
            .bind(runs[0].to_string())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the admission key of the run");
    assert_eq!(
        (scope, key),
        (
            format!("schedule:{}", schedule.id),
            format!("nominal-{}", nominal.unix_timestamp_nanos() / 1_000_000)
        ),
        "FR-018-004: scope schedule:{{id}}, key nominal-{{epoch ms}}"
    );
    f.w.assert_consistent(RunId::new(runs[0])).await;
}

/// A transaction of the pass whose every write is cut off in turn.
#[derive(Clone, Copy)]
enum Step {
    /// The time falls due with nothing open: the run starts.
    Start,
    /// The time falls due next to an open run under `SKIP`: the time is recorded as skipped.
    Skip,
}

impl Step {
    fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Skip => "skip",
        }
    }

    /// A schedule created at `start`, brought to the instant just before the step under test.
    async fn prepare(self, f: &Fixture, start: OffsetDateTime, name: &str) -> ScheduleRecord {
        let first = start + Duration::hours(1);
        f.clock.set(start);
        let policies = match self {
            Self::Start => json!({}),
            Self::Skip => json!({"overlap": "SKIP"}),
        };
        let schedule = f.create(name, &hourly(), &policies, None).await;
        if matches!(self, Self::Skip) {
            f.clock.set(first + Duration::seconds(5));
            f.drain(&schedule).await; // the run that is open when the next time falls due
        }
        f.clock.set(self.nominal(start) + Duration::seconds(5));
        schedule
    }

    /// The time the step acts on.
    fn nominal(self, start: OffsetDateTime) -> OffsetDateTime {
        match self {
            Self::Start => start + Duration::hours(1),
            Self::Skip => start + Duration::hours(2),
        }
    }

    fn outcomes(self) -> Vec<(String, i64)> {
        match self {
            Self::Start => outcomes_of(&[("STARTED", 1)]),
            Self::Skip => outcomes_of(&[("SKIPPED_OVERLAP", 1), ("STARTED", 1)]),
        }
    }
}

async fn die_at_every_write(step: Step) {
    let start = some_hours_ago();
    let first = start + Duration::hours(1);
    let nominal = step.nominal(start);
    let f = fixture_from(start).await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // Which writes does the step make? Watch a normal one.
    let probe = step.prepare(&f, start, "probe").await;
    wire.watch().await;
    assert_eq!(f.fire(&probe).await, Fired::Acted);
    wire.disarm().await;
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "{}: a step is one transaction: {writes:?}",
        step.name()
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in ["schedule_actions", "schedules"] {
        assert!(
            tables.contains(table),
            "{}: the step is expected to write {table}: {writes:?}",
            step.name()
        );
    }
    if matches!(step, Step::Start) {
        for table in ["runs", "run_events", "audit_records"] {
            assert!(
                tables.contains(table),
                "the start is expected to write {table}: {writes:?}"
            );
        }
        assert_one_scheduled_run(&f, &probe, first).await;
    }
    assert_eq!(f.outcomes(&probe).await, step.outcomes());
    println!(
        "{}: the step makes {} writing statements in one transaction, on {} tables: {tables:?}",
        step.name(),
        writes.len(),
        tables.len()
    );

    // Die after each of those writes, on a schedule of its own, and start again.
    for (_, position, table) in &writes {
        let schedule = step
            .prepare(&f, start, &format!("dies-at-{position}"))
            .await;
        let digests = table_digests(&f.w.db.pool).await;
        let before = f.read(&schedule).await;
        let runs_before = f.runs(&schedule).await;
        let outcomes_before = f.outcomes(&schedule).await;

        wire.arm(*position).await;
        let result = f.firing.fire(&f.due(&schedule), None).await;
        wire.disarm().await;

        // The cut-off was seen: the step either failed or was refused as an admission the store
        // could not take, never reported as done and never refused for another reason.
        let cut_off = match &result {
            Err(_) => true,
            Ok(Fired::Refused(reason)) => reason.contains("admission was refused"),
            Ok(_) => false,
        };
        assert!(
            cut_off,
            "{}: cut off at write {position} ({table}), the step answered {result:?}",
            step.name()
        );
        assert_eq!(
            table_digests(&f.w.db.pool).await,
            digests,
            "{}: cut off at write {position} ({table}), a table changed",
            step.name()
        );
        let after = f.read(&schedule).await;
        assert_eq!(
            (
                after.next_action_at,
                after.remaining_actions,
                after.buffered,
                &after.tracked_runs
            ),
            (
                before.next_action_at,
                before.remaining_actions,
                before.buffered,
                &before.tracked_runs
            ),
            "{}: cut off at write {position} ({table}) moved the schedule on",
            step.name()
        );
        assert_eq!(f.runs(&schedule).await, runs_before);
        assert_eq!(f.outcomes(&schedule).await, outcomes_before);

        // The process restarts: whatever backoff the failure asked for has passed.
        f.clock.advance(Duration::minutes(10));
        f.drain(&schedule).await;
        assert_eq!(
            f.outcomes(&schedule).await,
            step.outcomes(),
            "{}: after write {position} ({table}) the time is acted on once",
            step.name()
        );
        assert_one_scheduled_run(&f, &schedule, first).await;
        assert_eq!(
            f.read(&schedule).await.next_action_at,
            Some(nominal + Duration::hours(1)),
            "{}: after write {position} ({table}) the next time follows the one acted on",
            step.name()
        );
    }

    // The matrix is the whole transaction: a step armed past its last write is not cut off.
    let beyond = step.prepare(&f, start, "dies-after-the-last").await;
    wire.arm(writes.len() + 1).await;
    let result = f.fire(&beyond).await;
    wire.disarm().await;
    assert_eq!(
        result,
        Fired::Acted,
        "{}: the step makes more than the {} writes that were cut off",
        step.name(),
        writes.len()
    );
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_start_commits_nothing_and_the_time_starts_one_run() {
    die_at_every_write(Step::Start).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_skip_commits_nothing_and_the_time_is_skipped_once() {
    die_at_every_write(Step::Skip).await;
}

/// A firing built from nothing but the database: what a restarted process has.
fn restarted(
    f: &Fixture,
) -> ScheduleFiring<
    PgScheduleStore,
    SpecCalendar,
    crate::support::schedules::TestClock,
    PgIdentityStore,
    Starts,
> {
    ScheduleFiring::new(
        PgScheduleStore::new(f.w.db.pool.clone()),
        SpecCalendar,
        f.clock.clone(),
        PgIdentityStore::new(f.w.db.pool.clone()),
        Starts(Arc::clone(&f.w)),
        Ceilings::default(),
    )
}

#[tokio::test]
async fn a_reply_that_never_arrives_does_not_make_the_restarted_process_start_the_time_again() {
    // The firing commits and the process dies before it learns so. The restarted process knows
    // nothing of the step: it is built from the database alone and asks the store again.
    let start = some_hours_ago();
    let first = start + Duration::hours(1);
    let f = fixture_from(start).await;
    let schedule = f.create("hourly", &hourly(), &json!({}), None).await;
    f.clock.set(first + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    // (the result is dropped here: that is the lost reply)
    let after_restart = restarted(&f);
    for _ in 0..3 {
        assert_eq!(
            after_restart
                .fire(&f.due(&schedule), None)
                .await
                .expect("fire"),
            Fired::Idle,
            "a pass after the restart finds nothing owed"
        );
    }
    assert_one_scheduled_run(&f, &schedule, first).await;
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 1)]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eight_passes_racing_over_a_backlog_start_each_time_once() {
    let start = some_hours_ago();
    let f = fixture_from(start).await;
    let schedule = f
        .create(
            "backlog",
            &hourly(),
            &json!({"catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock
        .set(start + Duration::hours(3) + Duration::seconds(5));
    let pool = f.w.db.pool.clone();
    let due = DueSchedule {
        tenant_id: schedule.tenant_id,
        id: schedule.id,
    };
    let firing = Arc::new(restarted(&f));
    // All eight start together, so that the steps meet in the store and not one after the other.
    let gate = Arc::new(Barrier::new(8));
    let mut passes = Vec::new();
    for _ in 0..8 {
        let (firing, due, gate) = (Arc::clone(&firing), due.clone(), Arc::clone(&gate));
        passes.push(tokio::spawn(async move {
            gate.wait().await;
            let mut lost = 0_u32;
            // A pass keeps stepping while there is something to do; a step that lost the swap to
            // another pass looks again from what is stored.
            for _ in 0..50 {
                match firing.fire(&due, None).await.expect("fire") {
                    Fired::Idle => return lost,
                    Fired::Acted => {}
                    Fired::Refused(_) => lost += 1,
                }
            }
            panic!("the pass never became idle");
        }));
    }
    let mut lost = 0;
    for pass in passes {
        lost += pass.await.expect("pass");
    }
    println!("{lost} steps lost the swap to another pass");

    let nominals: Vec<OffsetDateTime> = sqlx::query_scalar(
        "SELECT schedule_nominal_at FROM runs WHERE schedule_id = $1 ORDER BY schedule_nominal_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("nominals");
    assert_eq!(
        nominals,
        vec![
            start + Duration::hours(1),
            start + Duration::hours(2),
            start + Duration::hours(3)
        ],
        "each time has one run, none twice"
    );
    let actions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM schedule_actions WHERE schedule_id = $1")
            .bind(schedule.id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("actions");
    assert_eq!(actions, 3, "and one action each");
}

#[tokio::test]
async fn a_replica_with_a_stale_lease_and_a_duplicate_pass_leave_one_run_and_one_action() {
    let start = some_hours_ago();
    let first = start + Duration::hours(1);
    let f = fixture_from(start).await;
    let schedule = f.create("hourly", &hourly(), &json!({}), None).await;
    let placement = Placement::new(format!("a-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        f.w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(3_600),
        },
    );
    keeper.round().await.expect("the replica takes its shards");
    let pass = || {
        SchedulePass::new(
            restarted(&f),
            placement.clone(),
            ScheduleSettings::default(),
        )
    };
    let (one, twin) = (pass(), pass());

    // Another replica takes the shard over while this one still holds a snapshot that says it owns it.
    let shard = i16::from(Shard::of(schedule.id.as_uuid()).index());
    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner, generation FROM scheduler_shards WHERE shard = $1")
            .bind(shard)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("the lease");
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'usurper', generation = generation + 1 WHERE shard = $1",
    )
    .bind(shard)
    .execute(&f.w.db.pool)
    .await
    .expect("take the shard over");
    f.clock.set(first + Duration::seconds(5));
    let stale = one.round().await;
    assert_eq!(
        (stale.steps, stale.refused),
        (0, 1),
        "the stale replica is refused where it acts"
    );
    assert!(f.runs(&schedule).await.is_empty());
    assert_eq!(
        f.outcomes(&schedule).await,
        Vec::<(String, i64)>::new(),
        "and nothing of the action was recorded"
    );

    // The lease comes back and the same pass runs twice at once.
    sqlx::query("UPDATE scheduler_shards SET owner = $2, generation = $3 WHERE shard = $1")
        .bind(shard)
        .bind(&owner)
        .bind(generation)
        .execute(&f.w.db.pool)
        .await
        .expect("give the shard back");
    let (a, b) = tokio::join!(one.round(), twin.round());
    assert_eq!((a.failed, b.failed), (0, 0));
    assert_eq!(
        a.steps + b.steps,
        1,
        "one of the two passes acted: {a:?} {b:?}"
    );
    assert_one_scheduled_run(&f, &schedule, first).await;
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 1)]));
}

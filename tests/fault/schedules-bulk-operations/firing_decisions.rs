//! The steps of the firing that start no run, killed after each of their writes (`T018-020`, `SC-018-002`, `FR-018-004`,
//! `FR-018-005`, `NFR-018-001`, gap G1018): `firing.rs` cuts the start and the skipped time; the others decide and record
//! and are cut here. A time that finds its buffer with room is buffered and one that finds it full is dropped, the
//! times a recovery has no action left for are coalesced in one summary, the times older than the window are missed, a
//! schedule with no action left is exhausted, the runs it tracks that have ended are forgotten (and the schedule is paused
//! when one failed and it says to pause), and a backfill takes a time that is skipped, buffered, found acted on already, or
//! the last one of its range.
//!
//! For each: one step is watched to learn its writing statements (the count must be the same for two schedules in the
//! same state: a step whose count is not stable cannot be cut at each of its writes, and that is a finding), then the
//! connection is cut after each of them in turn, on a schedule of its own in that state. The step answers an error, nothing
//! in any table has changed, and the same step taken again does what the watched one did, once: the same outcomes, the same
//! times and counters, one row of action for a time, the same audit records, and the runs the schedule started replay
//! to their projection.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use sagat_application::clock::Clock as _;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::Fired;
use sagat_application::schedules::{BackfillRecord, BackfillRequest, DueBackfill, ScheduleRecord};
use sagat_domain::execution::RunId;
use sagat_domain::schedules::{Overlap, RemainingActions};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};

use crate::support::digests::{all_table_digests, changed};
use crate::support::schedules::{Fixture, fixture_from, hourly};
use crate::tripwire::Tripwire;

/// An hour boundary six hours ago: the times these steps act on are all in the past of the database's clock and of the test's.
fn long_ago() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    OffsetDateTime::from_unix_timestamp(now - now % 3_600 - 6 * 3_600).expect("instant")
}

fn minutely() -> Value {
    json!({"intervals": [{"everyMs": 60_000}]})
}

/// A schedule, and the backfill of the case when it has one.
struct Prepared {
    schedule: ScheduleRecord,
    backfill: Option<BackfillRecord>,
}

#[derive(Debug, Clone, Copy)]
enum Case {
    /// The time falls due next to an open run under `BUFFER_ONE` with room: it waits.
    Buffer,
    /// A waiting time that an open run still holds back is looked at again, five seconds after it began to wait.
    Recheck,
    /// The same with the buffer full: the time is dropped.
    Drop,
    /// A recovery with no action left and times owed: one summary of them all.
    Coalesce,
    /// Times older than the window of catch-up: one summary of them.
    Missed,
    /// The last action the schedule may take is taken and the next time falls due.
    Exhausted,
    /// A run the schedule tracks has ended: it is forgotten.
    Reconcile,
    /// A run the schedule tracks has failed and the schedule pauses on failure: forgotten, and the schedule is paused.
    AutoPause,
    /// A backfill finds an open run under `SKIP`.
    BackfillSkip,
    /// A backfill finds an open run under `BUFFER_ONE`.
    BackfillBuffer,
    /// A backfill finds a time the schedule acted on already.
    BackfillSeen,
    /// A backfill has no time left in its range.
    BackfillFinish,
}

impl Case {
    fn name(self) -> String {
        format!("{self:?}")
    }

    /// A schedule created at `start`, brought to the instant just before the step under test.
    async fn prepare(self, f: &Fixture, start: OffsetDateTime, name: &str) -> Prepared {
        let first = start + Duration::hours(1);
        let at = |hours: i64| start + Duration::hours(hours) + Duration::seconds(5);
        f.clock.set(start);
        // the first time starts a run, which stays open: nothing works on it
        let open_run = |policies: Value| async move {
            let schedule = f.create(name, &hourly(), &policies, None).await;
            f.clock.set(first + Duration::seconds(5));
            f.drain(&schedule).await;
            schedule
        };
        match self {
            Self::Buffer => {
                let schedule = open_run(json!({"overlap": "BUFFER_ONE"})).await;
                f.clock.set(at(2));
                Prepared {
                    schedule,
                    backfill: None,
                }
            }
            Self::Recheck => {
                let schedule = open_run(json!({"overlap": "BUFFER_ONE"})).await;
                f.clock.set(at(2));
                assert_eq!(f.fire(&schedule).await, Fired::Acted, "the time waits");
                // five seconds after the time was buffered, the open run that holds it back is looked at again
                f.clock.set(at(2) + Duration::seconds(6));
                Prepared {
                    schedule: f.read(&schedule).await,
                    backfill: None,
                }
            }
            Self::Drop => {
                let schedule = open_run(json!({"overlap": "BUFFER_ONE"})).await;
                f.clock.set(at(2));
                assert_eq!(f.fire(&schedule).await, Fired::Acted, "the time waits");
                // a waiting action that an open run holds back is looked at again soon: that is the step that comes
                // first when the next time falls due, and it is taken, so that the one under test is the drop
                f.clock.set(at(3));
                assert_eq!(f.fire(&schedule).await, Fired::Acted, "the look again");
                Prepared {
                    schedule,
                    backfill: None,
                }
            }
            Self::Coalesce => {
                let schedule = f
                    .create(
                        name,
                        &minutely(),
                        &json!({"catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
                        None,
                    )
                    .await;
                f.clock
                    .set(start + Duration::minutes(20) + Duration::seconds(30));
                // a recovery that has used up its actions, with twenty times still owed
                sqlx::query(
                    "UPDATE schedules SET recovery_until = $2, recovery_acts = 0 WHERE id = $1",
                )
                .bind(schedule.id.as_uuid())
                .bind(f.clock.now())
                .execute(&f.w.db.pool)
                .await
                .expect("a recovery with no action left");
                Prepared {
                    schedule: f.read(&schedule).await,
                    backfill: None,
                }
            }
            Self::Missed => {
                let schedule = f
                    .create(
                        name,
                        &hourly(),
                        &json!({"catchupWindowMs": 1_800_000, "catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
                        None,
                    )
                    .await;
                f.clock.set(at(3));
                Prepared {
                    schedule,
                    backfill: None,
                }
            }
            Self::Exhausted => {
                let mut write = f.write_of(name, &hourly(), &json!({}), None);
                write.remaining_actions = Some(RemainingActions::new(1).expect("one action"));
                let schedule = f.try_create(write).await.expect("the schedule");
                f.clock.set(first + Duration::seconds(5));
                f.drain(&schedule).await;
                f.clock.set(at(2));
                Prepared {
                    schedule,
                    backfill: None,
                }
            }
            Self::Reconcile | Self::AutoPause => {
                let policies = if matches!(self, Self::AutoPause) {
                    json!({"pauseOnFailure": true})
                } else {
                    json!({})
                };
                let schedule = open_run(policies).await;
                let runs = f.runs(&schedule).await;
                assert_eq!(runs.len(), 1);
                // a worker ends the run, so that its journal says so (a status written to the table would not replay)
                let session = f.w.session(&["act.a", "act.b"]).await;
                let task = f.w.one_task_of(&session, "act.a").await;
                if matches!(self, Self::AutoPause) {
                    f.w.fail(&session, &task, false)
                        .await
                        .expect("the step fails for good");
                    assert_eq!(
                        f.w.run_status(RunId::new(runs[0])).await,
                        sagat_domain::execution::RunStatus::Failed
                    );
                } else {
                    f.w.complete(&session, &task, &json!({}))
                        .await
                        .expect("a completes");
                    let task = f.w.one_task_of(&session, "act.b").await;
                    f.w.complete(&session, &task, &json!({}))
                        .await
                        .expect("b completes");
                    assert_eq!(
                        f.w.run_status(RunId::new(runs[0])).await,
                        sagat_domain::execution::RunStatus::Succeeded
                    );
                }
                f.clock.set(at(2));
                Prepared {
                    schedule: f.read(&schedule).await,
                    backfill: None,
                }
            }
            Self::BackfillSkip | Self::BackfillBuffer => {
                let schedule = open_run(json!({})).await;
                let overlap = if matches!(self, Self::BackfillSkip) {
                    Overlap::Skip
                } else {
                    Overlap::BufferOne
                };
                let backfill = f
                    .service
                    .create_backfill(
                        &f.w.admin,
                        schedule.id,
                        &BackfillRequest {
                            start: start - Duration::hours(2),
                            end: start,
                            overlap: Some(overlap),
                        },
                        &RequestContext::default(),
                    )
                    .await
                    .expect("the backfill");
                Prepared {
                    schedule: f.read(&schedule).await,
                    backfill: Some(backfill),
                }
            }
            Self::BackfillSeen => {
                let schedule = open_run(json!({"overlap": "ALLOW_ALL"})).await;
                // the time the schedule acted on is the only one of the range
                let backfill = f
                    .service
                    .create_backfill(
                        &f.w.admin,
                        schedule.id,
                        &BackfillRequest {
                            start: first,
                            end: first + Duration::seconds(1),
                            overlap: Some(Overlap::AllowAll),
                        },
                        &RequestContext::default(),
                    )
                    .await
                    .expect("the backfill");
                Prepared {
                    schedule: f.read(&schedule).await,
                    backfill: Some(backfill),
                }
            }
            Self::BackfillFinish => {
                let schedule = f.create(name, &hourly(), &json!({}), None).await;
                f.clock.set(first + Duration::seconds(5));
                // a range between two times of the schedule holds none
                let backfill = f
                    .service
                    .create_backfill(
                        &f.w.admin,
                        schedule.id,
                        &BackfillRequest {
                            start: start + Duration::minutes(10),
                            end: start + Duration::minutes(20),
                            overlap: Some(Overlap::AllowAll),
                        },
                        &RequestContext::default(),
                    )
                    .await
                    .expect("the backfill");
                Prepared {
                    schedule: f.read(&schedule).await,
                    backfill: Some(backfill),
                }
            }
        }
    }

    /// The step.
    async fn act(self, f: &Fixture, prepared: &Prepared) -> Result<Fired, AppError> {
        match &prepared.backfill {
            None => f.firing.fire(&f.due(&prepared.schedule), None).await,
            Some(backfill) => {
                f.firing
                    .backfill_step(
                        &DueBackfill {
                            tenant_id: backfill.tenant_id,
                            schedule: backfill.schedule,
                            id: backfill.id,
                        },
                        None,
                    )
                    .await
            }
        }
    }
}

/// Everything a step may change, as the database holds it, without what differs between two schedules (their ids).
#[derive(Debug, PartialEq, Eq)]
struct State {
    schedule: String,
    actions: Vec<(String, String, String, i32, String, bool)>,
    audits: Vec<(String, i64)>,
    backfill: Option<String>,
    runs: usize,
}

async fn state_of(f: &Fixture, prepared: &Prepared) -> State {
    let schedule = f.read(&prepared.schedule).await;
    let backfill = match &prepared.backfill {
        None => None,
        Some(wanted) => {
            let found = f
                .service
                .backfills(&f.w.admin, prepared.schedule.id)
                .await
                .expect("the backfills")
                .into_iter()
                .find(|b| b.id == wanted.id)
                .expect("the backfill");
            Some(format!(
                "{:?} {:?} {:?} {}",
                found.state,
                found.counters,
                found.cursor,
                found.finished_at.is_some()
            ))
        }
    };
    State {
        schedule: format!(
            "{:?} next {:?} actual {:?} remaining {:?} retry {:?} recheck {:?} recovery {:?} buffered {} tracked {}",
            schedule.state,
            schedule.next_action_at,
            schedule.next_actual_at,
            schedule.remaining_actions,
            schedule.retry_at,
            schedule.recheck_at,
            schedule.recovery,
            schedule.buffered,
            schedule.tracked_runs.len(),
        ),
        actions: sqlx::query_as(
            "SELECT nominal_at::text, kind, outcome, coalesce(summary_count, 0), \
                    coalesce(summary_last_at::text, ''), backfill_id IS NOT NULL \
             FROM schedule_actions WHERE schedule_id = $1 ORDER BY nominal_at, kind",
        )
        .bind(prepared.schedule.id.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the actions"),
        audits: sqlx::query_as(
            "SELECT action, count(*) FROM audit_records WHERE resource_id = $1 GROUP BY action ORDER BY action",
        )
        .bind(prepared.schedule.id.to_string())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the audit"),
        backfill,
        runs: f.runs(&prepared.schedule).await.len(),
    }
}

/// The writing statements of one step of `case` on a schedule of its own, and the state it leaves.
async fn watch(
    f: &Fixture,
    wire: &Tripwire,
    case: Case,
    start: OffsetDateTime,
    name: &str,
) -> (Vec<(usize, String)>, State) {
    let prepared = Box::pin(case.prepare(f, start, name)).await;
    let before = wire.writes().await.len();
    wire.watch().await;
    let done = Box::pin(case.act(f, &prepared)).await;
    wire.disarm().await;
    assert!(
        matches!(done, Ok(Fired::Acted)),
        "{}: the step of the probe acts: {done:?}",
        case.name()
    );
    let writes: Vec<(i64, usize, String)> = wire.writes().await.split_off(before);
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    assert_eq!(
        transactions.len(),
        1,
        "{}: a step is one transaction: {writes:?}",
        case.name()
    );
    let state = state_of(f, &prepared).await;
    release(f, &prepared).await;
    (
        writes
            .into_iter()
            .map(|(_, position, table)| (position, table))
            .collect(),
        state,
    )
}

/// Stops the backfill of a schedule that has served its purpose: a namespace has room for four at once, and each of the
/// schedules of this matrix has one.
async fn release(f: &Fixture, prepared: &Prepared) {
    if let Some(backfill) = &prepared.backfill {
        // one that has ended cannot be stopped, and holds no room
        let _ = f
            .service
            .cancel_backfill(
                &f.w.admin,
                prepared.schedule.id,
                backfill.id,
                &RequestContext::default(),
            )
            .await;
    }
}

async fn die_at_every_write(case: Case) {
    let start = long_ago();
    let f = fixture_from(start).await;
    let wire = Tripwire::install(&f.w.db.pool).await;

    // Which writes does the step make? Watch it on two schedules in the same state: the count has to be the same.
    let (writes, expected) = watch(&f, &wire, case, start, "probe-a").await;
    let (again, expected_again) = watch(&f, &wire, case, start, "probe-b").await;
    assert_eq!(
        writes,
        again,
        "{}: the writing statements of a step are not the same on two schedules in one state, so it cannot be cut at each of them",
        case.name()
    );
    assert_eq!(
        expected,
        expected_again,
        "{}: the same step leaves another state on a schedule in the same state",
        case.name()
    );
    assert!(
        !writes.is_empty(),
        "{}: a step that decides writes",
        case.name()
    );
    let tables: BTreeSet<&str> = writes.iter().map(|(_, table)| table.as_str()).collect();
    assert!(
        tables.contains("schedules"),
        "{}: the step moves the schedule: {writes:?}",
        case.name()
    );
    println!(
        "{}: the step makes {} writing statements in one transaction, on {} tables: {tables:?}",
        case.name(),
        writes.len(),
        tables.len()
    );

    // Die after each of those writes, on a schedule of its own, and take the step again.
    for (position, table) in &writes {
        let prepared = Box::pin(case.prepare(&f, start, &format!("dies-at-{position}"))).await;
        let digests = all_table_digests(&f.w.db.pool).await;
        let before = state_of(&f, &prepared).await;

        wire.arm(*position).await;
        let result = Box::pin(case.act(&f, &prepared)).await;
        wire.disarm().await;

        // the cut-off was seen: the step failed, it was not reported as done and not refused for another reason
        assert!(
            result.is_err(),
            "{}: cut off at write {position} ({table}), the step answered {result:?}",
            case.name()
        );
        let after_digests = all_table_digests(&f.w.db.pool).await;
        assert_eq!(
            changed(&digests, &after_digests),
            Vec::<String>::new(),
            "{}: cut off at write {position} ({table}), a table changed",
            case.name()
        );
        assert_eq!(
            state_of(&f, &prepared).await,
            before,
            "{}: cut off at write {position} ({table}) moved the schedule on",
            case.name()
        );

        // the next pass takes the step again, once, and it is the step that was watched
        let again = Box::pin(case.act(&f, &prepared)).await;
        assert!(
            matches!(again, Ok(Fired::Acted)),
            "{}: after write {position} ({table}) the step is taken again: {again:?}",
            case.name()
        );
        assert_eq!(
            state_of(&f, &prepared).await,
            expected,
            "{}: after write {position} ({table}) the step does what it did",
            case.name()
        );
        // one row of action for a time, and the runs it started replay to what the database holds of them
        let doubled: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT nominal_at::text, kind, count(*) FROM schedule_actions WHERE schedule_id = $1 \
             GROUP BY nominal_at, kind HAVING count(*) > 1",
        )
        .bind(prepared.schedule.id.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("the actions");
        assert!(
            doubled.is_empty(),
            "{}: after write {position} ({table}) a time has two actions: {doubled:?}",
            case.name()
        );
        for run in f.runs(&prepared.schedule).await {
            f.w.assert_consistent(RunId::new(run)).await;
        }
        release(&f, &prepared).await;
    }

    // The matrix is the whole transaction: a step armed past its last write is not cut off.
    let beyond = Box::pin(case.prepare(&f, start, "dies-after-the-last")).await;
    wire.arm(writes.len() + 1).await;
    let result = Box::pin(case.act(&f, &beyond)).await;
    wire.disarm().await;
    assert!(
        matches!(result, Ok(Fired::Acted)),
        "{}: the step makes more than the {} writes that were cut off: {result:?}",
        case.name(),
        writes.len()
    );
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_buffered_time_commits_nothing_and_the_time_is_buffered_once()
 {
    Box::pin(die_at_every_write(Case::Buffer)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_the_look_again_at_a_waiting_time_commits_nothing_and_it_is_looked_at_once()
 {
    Box::pin(die_at_every_write(Case::Recheck)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_dropped_time_commits_nothing_and_the_time_is_dropped_once()
 {
    Box::pin(die_at_every_write(Case::Drop)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_coalesced_recovery_commits_nothing_and_the_summary_is_one()
 {
    Box::pin(die_at_every_write(Case::Coalesce)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_missed_summary_commits_nothing_and_the_times_are_counted_once()
 {
    Box::pin(die_at_every_write(Case::Missed)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_the_exhaustion_commits_nothing_and_the_schedule_is_exhausted_once()
 {
    Box::pin(die_at_every_write(Case::Exhausted)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_the_forgetting_of_an_ended_run_commits_nothing_and_the_run_is_forgotten_once()
 {
    Box::pin(die_at_every_write(Case::Reconcile)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_an_auto_pause_commits_nothing_and_the_schedule_is_paused_once()
 {
    Box::pin(die_at_every_write(Case::AutoPause)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_skipped_time_of_a_backfill_commits_nothing_and_the_time_is_skipped_once()
 {
    Box::pin(die_at_every_write(Case::BackfillSkip)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_buffered_time_of_a_backfill_commits_nothing_and_the_time_is_buffered_once()
 {
    Box::pin(die_at_every_write(Case::BackfillBuffer)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_a_time_a_backfill_finds_acted_on_commits_nothing_and_it_is_counted_once()
 {
    Box::pin(die_at_every_write(Case::BackfillSeen)).await;
}

#[tokio::test]
async fn a_process_killed_after_any_write_of_the_end_of_a_backfill_commits_nothing_and_it_ends_once()
 {
    Box::pin(die_at_every_write(Case::BackfillFinish)).await;
}

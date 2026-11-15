//! What the third review of the trigger-owned schedules asked for (`T018-060`, `T018-061`; D2069 and after), written before the
//! fixes and seen failing: one definition of "converged" for the reconciliation and the handover, a cron that never fires, a
//! pause in `LEGACY`, the order and the bound of the locks of the command, a state the function does not know, the name of the
//! operator as it is printed.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::time::Duration;

use sagat_application::schedule_triggers::TriggerScheduleStore as _;
use sagat_control_plane::handover::{HandoverOutcome, HandoverRequest, schedule_trigger_handover};
use uuid::Uuid;

use crate::support::trigger_schedules::{Triggers, cron_workflow, triggers};
use crate::trigger_firing::due;
use crate::trigger_followup::{command, database, declined, engine, state, until_a_backend_waits};
use crate::trigger_reconcile::{old_control_plane_publishes, replica};

async fn dry_run(
    t: &Triggers,
    live: u32,
) -> Result<HandoverOutcome, sagat_control_plane::RunError> {
    schedule_trigger_handover(
        &database(t),
        &HandoverRequest {
            operator: None,
            expect_live: Some(live),
            dry_run: true,
        },
    )
    .await
}

// --- A1: one definition of "converged" -------------------------------------------------------------------------------------------

/// Every state a trigger's schedule can be in, as the owner can write it, with whether it is converged while the handover has
/// not happened and after it.
const STATES: &[(&str, &str, bool, bool)] = &[
    // name, the statement that makes the state from a planned, active schedule ($1 is the schedule), converged in LEGACY, after
    ("planned", "SELECT $1::uuid", true, true),
    (
        "only the actual time",
        "UPDATE schedules SET next_action_at = NULL WHERE id = $1",
        false,
        false,
    ),
    (
        "only the nominal time",
        "UPDATE schedules SET next_actual_at = NULL WHERE id = $1",
        false,
        false,
    ),
    (
        "no time and not exhausted",
        "UPDATE schedules SET next_action_at = NULL, next_actual_at = NULL WHERE id = $1",
        false,
        false,
    ),
    (
        "exhausted: no time and no actions left",
        "UPDATE schedules SET next_action_at = NULL, next_actual_at = NULL, remaining_actions = 0 WHERE id = $1",
        true,
        true,
    ),
    (
        "paused",
        "UPDATE schedules SET state = 'PAUSED', next_action_at = NULL, next_actual_at = NULL WHERE id = $1",
        false,
        true,
    ),
    (
        "deleted",
        "UPDATE schedules SET state = 'DELETED', deleted_at = now(), next_action_at = NULL, next_actual_at = NULL WHERE id = $1",
        false,
        false,
    ),
];

#[tokio::test]
async fn the_reconciliation_has_nothing_to_repair_exactly_when_the_handover_would_count_none_over_every_state()
 {
    let t = triggers().await;
    let mut wanted = Vec::new();
    for (n, (name, statement, _, _)) in STATES.iter().enumerate() {
        let workflow = format!("state-{n}");
        t.publish(cron_workflow(&workflow, "0 * * * *", "UTC"))
            .await;
        let (trigger, ..) = t.trigger_of(&workflow).await;
        wanted.push((*name, trigger, *statement));
    }
    // the planned schedules first, so that every state is made from a schedule that is equal to its trigger
    t.reconcile().await;
    for (name, trigger, statement) in &wanted {
        let schedule = t.schedule_of(*trigger).await.expect("schedule").id;
        sqlx::query(*statement)
            .bind(schedule.as_uuid())
            .execute(&t.w.db.pool)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    // a trigger an older control plane published has no schedule at all
    old_control_plane_publishes(&t, "state-0", Uuid::from_u128(0x5151), "30 * * * *", "UTC").await;

    for (phase, after_handover) in [("LEGACY", false), ("HANDED_OVER", true)] {
        if after_handover {
            t.force_handover_row("HANDED_OVER", Some("test-operator"))
                .await;
        }
        let drift: BTreeSet<Uuid> = t
            .drift()
            .await
            .iter()
            .filter(|d| {
                !matches!(
                    d,
                    sagat_application::schedule_triggers::Drift::Orphan { .. }
                )
            })
            .map(sagat_application::schedule_triggers::Drift::trigger)
            .collect();
        let mut expected = BTreeSet::new();
        expected.insert(Uuid::from_u128(0x5151));
        for (n, (name, trigger, _)) in wanted.iter().enumerate() {
            let converged = if after_handover {
                STATES[n].3
            } else {
                STATES[n].2
            };
            if !converged {
                expected.insert(*trigger);
            }
            assert_eq!(
                drift.contains(trigger),
                !converged,
                "{phase}: the reconciliation on `{name}`"
            );
        }
        assert_eq!(drift, expected, "{phase}: what the reconciliation repairs");
        let counted = t.store.convergence().await.expect("convergence");
        assert_eq!(
            usize::try_from(counted.unconverged).expect("count"),
            expected.len(),
            "{phase}: what the handover counts is what the reconciliation repairs"
        );
        assert_eq!(drift.is_empty(), counted.unconverged == 0);
    }
}

// --- A1: a cron that parses and never fires ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_cron_that_never_fires_is_converged_as_exhausted_and_does_not_block_the_handover() {
    let t = triggers().await;
    // publication refuses the 31st of February (feature 004 validates the calendar there), so the rows that reach the
    // reconciliation are the ones an older build or another calendar wrote: one with no schedule, one whose schedule was a
    // cron that fired and whose trigger now never does
    t.publish(cron_workflow("daily", "0 9 * * *", "UTC")).await;
    t.publish(cron_workflow("changed", "0 9 * * *", "UTC"))
        .await;
    engine(&t).await;
    let (changed, ..) = t.trigger_of("changed").await;
    t.reconcile().await;
    let never = Uuid::from_u128(0x31_02);
    old_control_plane_publishes(&t, "daily", never, "0 0 31 2 *", "UTC").await;
    sqlx::query("UPDATE workflow_triggers SET cron = '0 0 30 2 *' WHERE id = $1")
        .bind(changed)
        .execute(&t.w.db.pool)
        .await
        .expect("an older build rewrote the expression");

    t.reconcile().await;
    let again = t.reconcile().await;
    assert_eq!(
        again, 0,
        "a second pass has nothing to do: the first one planned them as they are"
    );
    for trigger in [never, changed] {
        let schedule = t
            .schedule_of(trigger)
            .await
            .expect("the schedule of the trigger");
        assert_eq!(schedule.next_action_at, None, "it has no next time");
        let remaining: Option<i32> =
            sqlx::query_scalar("SELECT remaining_actions FROM schedules WHERE trigger_id = $1")
                .bind(trigger)
                .fetch_one(&t.w.db.pool)
                .await
                .expect("remaining actions");
        assert_eq!(
            remaining,
            Some(0),
            "it is marked exhausted: the trigger never fires"
        );
    }
    assert_eq!(
        t.store
            .convergence()
            .await
            .expect("convergence")
            .unconverged,
        0
    );

    let dry = dry_run(&t, 1)
        .await
        .expect("a cron that never fires does not stop the handover");
    assert!(
        matches!(dry, HandoverOutcome::WouldHandOver { .. }),
        "{dry:?}"
    );
    let done = command(&t, Some(1)).await.expect("the handover");
    assert!(
        matches!(done, HandoverOutcome::HandedOver { .. }),
        "{done:?}"
    );
}

#[tokio::test]
async fn the_refusal_for_a_trigger_that_is_not_planned_says_which_trigger_and_what_to_do() {
    let t = triggers().await;
    t.publish(cron_workflow("unplanned", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("unplanned").await;
    let message = declined(dry_run(&t, 0).await);
    assert!(message.contains(&trigger.to_string()), "{message}");
    assert!(message.contains("no schedule or no next time"), "{message}");
    assert!(message.contains("wait"), "{message}");
}

// --- A2: a paused trigger schedule before the handover -----------------------------------------------------------------------------

#[tokio::test]
async fn a_paused_trigger_schedule_in_legacy_stops_the_dry_run_and_the_reconciliation_resumes_it() {
    let t = triggers().await;
    t.publish(cron_workflow("accident", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("accident").await;
    t.reconcile().await;
    engine(&t).await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    sqlx::query(
        "UPDATE schedules SET state = 'PAUSED', state_version = state_version + 1, next_action_at = NULL, next_actual_at = NULL \
          WHERE id = $1",
    )
    .bind(schedule.id.as_uuid())
    .execute(&t.w.db.pool)
    .await
    .expect("an accident");

    let message = declined(dry_run(&t, 1).await);
    assert!(message.contains(&trigger.to_string()), "{message}");

    assert_eq!(
        t.reconcile().await,
        1,
        "the reconciliation has the paused schedule to repair"
    );
    let resumed = t.read(&schedule).await;
    assert_eq!(
        resumed.state,
        sagat_application::schedules::ScheduleState::Active
    );
    assert!(resumed.next_action_at.is_some(), "and plans it");
    dry_run(&t, 1).await.expect("converged now");

    // after the handover a pause is the operator's, and the reconciliation leaves it
    let done = command(&t, Some(1)).await.expect("the handover");
    assert!(
        matches!(done, HandoverOutcome::HandedOver { .. }),
        "{done:?}"
    );
    sqlx::query(
        "UPDATE schedules SET state = 'PAUSED', state_version = state_version + 1, next_action_at = NULL, next_actual_at = NULL \
          WHERE id = $1",
    )
    .bind(schedule.id.as_uuid())
    .execute(&t.w.db.pool)
    .await
    .expect("the operator's pause");
    assert_eq!(
        t.reconcile().await,
        0,
        "a pause after the handover is not drift"
    );
    assert_eq!(
        t.read(&schedule).await.state,
        sagat_application::schedules::ScheduleState::Paused
    );
}

// --- B3: the order of the locks of the function -------------------------------------------------------------------------------------

#[tokio::test]
async fn the_function_takes_the_table_of_the_triggers_before_the_handover_row() {
    use sagat_application::instances::{Activation, Capability, InstanceKind};
    let t = triggers().await;
    t.publish(cron_workflow("ordered", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    engine(&t).await;
    // the requirement is recorded (as the activation records it), so the function gets as far as its locks
    let actor = sagat_domain::identity::Subject::parse("somebody").expect("subject");
    sagat_infra_postgres::PgInstanceStore::new(t.w.db.pool.clone())
        .activate(
            Activation {
                capability: Capability::SchedulesV1,
                kinds: &[InstanceKind::ExecutionEngine],
                actor: &actor,
            },
            |_| Box::pin(async { Ok(()) }),
        )
        .await
        .expect("recorded");

    // somebody holds the table of the triggers: the function waits at the first lock it takes
    let mut holder = t.w.db.pool.begin().await.expect("holder");
    sqlx::query("LOCK TABLE workflow_triggers IN ROW EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .expect("a writer in flight");
    let mut caller = t.w.db.pool.acquire().await.expect("caller");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *caller)
        .await
        .expect("pid");
    let waiting = tokio::spawn(async move {
        let result =
            sqlx::query_scalar::<_, String>("SELECT schedule_trigger_handover_complete('ana')")
                .fetch_one(&mut *caller)
                .await;
        (result, caller)
    });
    until_a_backend_waits(&t, "the function waits for the table of the triggers").await;
    let holding_the_row: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_locks l JOIN pg_class c ON c.oid = l.relation \
          WHERE l.pid = $1 AND l.granted AND c.relname = 'schedule_trigger_handover' AND l.mode = 'RowShareLock'",
    )
    .bind(pid)
    .fetch_one(&t.w.db.pool)
    .await
    .expect("locks");
    assert_eq!(
        holding_the_row, 0,
        "the function holds the handover row while it waits for the table: the census takes the table first, so two operators can deadlock"
    );
    holder.rollback().await.expect("release");
    let (result, _caller) = waiting.await.expect("the function");
    assert_eq!(result.expect("it hands over"), "HANDED_OVER");
}

// --- B4: the table lock is short ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_long_publication_makes_the_command_try_again_and_does_not_hold_the_writers_behind_it() {
    let t = triggers().await;
    t.publish(cron_workflow("held", "0 * * * *", "UTC")).await;
    t.publish(cron_workflow("bystander", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    engine(&t).await;
    let (held, ..) = t.trigger_of("held").await;
    let (bystander, ..) = t.trigger_of("bystander").await;
    // a publication that stays open: it has written a trigger and has not committed
    let mut publication = t.w.db.pool.begin().await.expect("publication");
    sqlx::query("UPDATE workflow_triggers SET retry_at = NULL WHERE id = $1")
        .bind(held)
        .execute(&mut *publication)
        .await
        .expect("a write in flight");

    let url = t.w.db.url.clone();
    let running = tokio::spawn(async move {
        schedule_trigger_handover(
            &sagat_control_plane::config::DatabaseConfig {
                url,
                max_connections: 2,
                // longer than the lock timeout: if the lock had no bound of its own the statement timeout would end the wait
                statement_timeout: Duration::from_secs(20),
            },
            &HandoverRequest {
                operator: Some("ana".to_owned()),
                expect_live: Some(1),
                dry_run: false,
            },
        )
        .await
    });
    until_a_backend_waits(&t, "the command waits for the table").await;
    // a later writer queues behind the command's request for the table ...
    let pool = t.w.db.pool.clone();
    let writer = tokio::spawn(async move {
        sqlx::query("UPDATE workflow_triggers SET retry_at = NULL WHERE id = $1")
            .bind(bystander)
            .execute(&pool)
            .await
    });
    // ... and the command gives up in words, long before the statement timeout, with nothing recorded
    let message = declined(running.await.expect("the command"));
    assert!(message.contains("run the command again"), "{message}");
    assert_eq!(state(&t).await, "LEGACY");
    let history: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_requirement_history")
        .fetch_one(&t.w.db.pool)
        .await
        .expect("history");
    assert_eq!(history, 0, "nothing was recorded");
    // the writer is let through while the publication is still open
    crate::support::bounded::within(
        "the writer behind the command",
        Duration::from_secs(30),
        async {
            writer
                .await
                .expect("the writer")
                .expect("the later writer is not held behind the publication");
        },
    )
    .await;
    publication.rollback().await.expect("the publication ends");
}

// --- B7: a state the function does not know ---------------------------------------------------------------------------------------

#[tokio::test]
async fn the_function_moves_only_the_state_legacy() {
    use sagat_application::instances::{Activation, Capability, InstanceKind};
    let t = triggers().await;
    t.publish(cron_workflow("newer", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    let actor = sagat_domain::identity::Subject::parse("somebody").expect("subject");
    sagat_infra_postgres::PgInstanceStore::new(t.w.db.pool.clone())
        .activate(
            Activation {
                capability: Capability::SchedulesV1,
                kinds: &[InstanceKind::ExecutionEngine],
                actor: &actor,
            },
            |_| Box::pin(async { Ok(()) }),
        )
        .await
        .expect("recorded");
    // a newer build knows another state: the check that lists the two is dropped, as its migration would drop it
    sqlx::query("ALTER TABLE schedule_trigger_handover DROP CONSTRAINT schedule_trigger_handover_state_valid")
        .execute(&t.w.db.pool)
        .await
        .expect("drop the check");
    t.force_handover_row("ARMED", Some("a newer build")).await;
    let refused =
        sqlx::query_scalar::<_, String>("SELECT schedule_trigger_handover_complete('ana')")
            .fetch_one(&t.w.db.pool)
            .await;
    let error = refused.expect_err("a state this build does not know is not moved");
    assert!(error.to_string().contains("does not know"), "{error}");
    assert_eq!(state(&t).await, "ARMED");
}

// --- B8: the name as it is stored --------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_database_refuses_a_name_that_could_make_the_record_read_as_somebody_else() {
    let t = triggers().await;
    t.publish(cron_workflow("named", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    engine(&t).await;
    use sagat_application::instances::{Activation, Capability, InstanceKind};
    let actor = sagat_domain::identity::Subject::parse("somebody").expect("subject");
    sagat_infra_postgres::PgInstanceStore::new(t.w.db.pool.clone())
        .activate(
            Activation {
                capability: Capability::SchedulesV1,
                kinds: &[InstanceKind::ExecutionEngine],
                actor: &actor,
            },
            |_| Box::pin(async { Ok(()) }),
        )
        .await
        .expect("recorded");
    for name in [
        "ana\u{202E}evil",
        "ana\u{200B}",
        "ana\u{0007}",
        "ana\nroot",
        "\u{FEFF}ana",
        "ana\u{1b}[2J",
    ] {
        let refused =
            sqlx::query_scalar::<_, String>("SELECT schedule_trigger_handover_complete($1)")
                .bind(name)
                .fetch_one(&t.w.db.pool)
                .await;
        assert!(refused.is_err(), "{name:?} must not be recorded");
        assert_eq!(state(&t).await, "LEGACY", "{name:?}");
    }
    let accepted = sqlx::query_scalar::<_, String>("SELECT schedule_trigger_handover_complete($1)")
        .bind("Jos\u{e9} M\u{fc}ller")
        .fetch_one(&t.w.db.pool)
        .await
        .expect("a name with letters of other alphabets is fine");
    assert_eq!(accepted, "HANDED_OVER");
}

// --- B9: the command as the binary runs it -------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_apply_path_of_the_command_hands_over_and_says_what_the_binary_prints() {
    use sagat_control_plane::cli::HandoverArgs;
    use sagat_control_plane::handover::run_handover_command;
    let t = triggers().await;
    t.publish(cron_workflow("applied", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    engine(&t).await;
    let args = |dry_run, apply| HandoverArgs {
        operator: Some("ana".to_owned()),
        expect_live: Some(1),
        dry_run,
        apply,
    };
    let db = database(&t);

    // neither flag: it verifies, hands nothing over, and says so
    let said = run_handover_command(&db, &args(false, false))
        .await
        .expect("verifies");
    assert!(said.contains("--apply was not given"), "{said}");
    assert_eq!(state(&t).await, "LEGACY");
    // --dry-run: the same verification, and it is not told that --apply is missing
    let said = run_handover_command(&db, &args(true, false))
        .await
        .expect("verifies");
    assert!(!said.contains("--apply was not given"), "{said}");
    assert_eq!(state(&t).await, "LEGACY");
    // --apply: it hands over, and the live count is the one counted under the lock
    let said = run_handover_command(&db, &args(false, true))
        .await
        .expect("hands over");
    assert!(said.starts_with("handed over"), "{said}");
    assert!(said.contains("1 live execution engine"), "{said}");
    assert!(!said.contains("dry run"), "{said}");
    assert_eq!(state(&t).await, "HANDED_OVER");
    // afterwards, whichever flags: it is already done, and it does not say that this was a dry run
    for (dry_run, apply) in [(false, false), (true, false), (false, true)] {
        let said = run_handover_command(&db, &args(dry_run, apply))
            .await
            .expect("already");
        assert!(said.starts_with("already handed over"), "{said}");
        assert!(
            !said.contains("dry run") && !said.contains("--apply was not given"),
            "{said}"
        );
    }
}

// --- C10: the legacy pass reads the state before each trigger --------------------------------------------------------------------

#[tokio::test]
async fn a_slow_start_of_the_legacy_pass_cannot_be_followed_by_another_after_the_handover() {
    let t = triggers().await;
    let (first, ..) = due(&t, "slow-first").await;
    let (second, ..) = due(&t, "slow-second").await;
    let legacy =
        t.w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default())
            .with_handover_bound(Duration::ZERO);
    // every start is held at the table of the runs: the first one blocks there, and the state changes meanwhile
    let mut hold = t.w.db.pool.begin().await.expect("hold");
    sqlx::query("LOCK TABLE runs IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *hold)
        .await
        .expect("the runs are locked");
    let round = tokio::spawn(async move { legacy.tick().await });
    until_a_backend_waits(&t, "a start waits at the runs").await;
    t.hand_over().await;
    hold.rollback().await.expect("release");
    round.await.expect("the round");
    let started = t.runs_of(first).await.len() + t.runs_of(second).await.len();
    assert_eq!(
        started, 1,
        "the start that was under way finishes; the next trigger is not fired after the handover"
    );
}

// --- C11: the window of the engines and the window of the control ------------------------------------------------------------------

#[tokio::test]
async fn an_engine_with_the_default_bound_never_fires_after_the_commit_while_the_control_is_open() {
    let t = triggers().await;
    due(&t, "default-bound-first").await;
    // the default bound: LEGACY is believed for two seconds
    let legacy =
        t.w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default());
    assert_eq!(
        legacy.tick().await.runs_started,
        1,
        "LEGACY: it fires, and remembers LEGACY"
    );
    t.hand_over_now().await;
    due(&t, "default-bound-second").await;
    // however long the engine takes to read the state again, the control is closed for as long as it may still fire: either it
    // fired inside its bound (and the control is closed) or it has read the state (and it fired nothing)
    let fired = legacy.tick().await.runs_started;
    let open = t.service.trigger_control_open().await.expect("read");
    assert!(
        !(fired > 0 && open),
        "the legacy pass fired after the commit ({fired}) while the control of the schedules was open"
    );
}

// --- C12: a handover that cannot be read ---------------------------------------------------------------------------------------------

fn plain(page: &str, name: &str) -> Option<f64> {
    page.lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(' ')?.parse().ok())
}

async fn unreadable_rounds(
    t: &Triggers,
    legacy: &crate::support::engine::EventLoop,
    rounds: usize,
) -> Vec<u32> {
    sqlx::query("ALTER TABLE schedule_trigger_handover RENAME TO schedule_trigger_handover_away")
        .execute(&t.w.db.pool)
        .await
        .expect("the state cannot be read");
    let mut started = Vec::new();
    for _ in 0..rounds {
        started.push(legacy.tick().await.runs_started);
    }
    sqlx::query("ALTER TABLE schedule_trigger_handover_away RENAME TO schedule_trigger_handover")
        .execute(&t.w.db.pool)
        .await
        .expect("put it back");
    started
}

#[test]
fn a_handover_read_that_keeps_failing_is_counted_and_the_pass_goes_on_until_it_can_be_read() {
    use crate::counted::recorded;
    let page = recorded(|| async {
        let t = triggers().await;
        due(&t, "unreadable-counted").await;
        let legacy =
            t.w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default())
                .with_handover_bound(Duration::ZERO);
        let started = unreadable_rounds(&t, &legacy, 3).await;
        assert_eq!(
            started[0], 1,
            "the pass goes on: a tick fired twice starts once, one not fired is lost"
        );
        // never put back inside the block: the metrics are read as they stand while it fails
        sqlx::query(
            "ALTER TABLE schedule_trigger_handover RENAME TO schedule_trigger_handover_away",
        )
        .execute(&t.w.db.pool)
        .await
        .expect("fail again");
        legacy.tick().await;
    });
    let failures = plain(&page, "cron_handover_read_failures_total").expect("the counter");
    assert!(failures >= 4.0, "every failed read is counted: {failures}");
    let consecutive = plain(&page, "cron_handover_read_consecutive_failures").expect("the gauge");
    assert!(
        consecutive >= 4.0,
        "and the run of failures is a gauge: {consecutive}"
    );
}

#[test]
fn after_the_reads_fail_and_recover_the_handover_and_a_pause_are_honoured() {
    use crate::counted::recorded;
    use sagat_domain::schedules::Note;
    let page = recorded(|| async {
        let t = triggers().await;
        let (workflow, _trigger, schedule, _) = due(&t, "unreadable-then-paused").await;
        let legacy =
            t.w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default())
                .with_handover_bound(Duration::ZERO);
        let started = unreadable_rounds(&t, &legacy, 3).await;
        assert_eq!(
            started.iter().sum::<u32>(),
            1,
            "one tick, fired once, however often the read failed"
        );
        t.hand_over().await;
        t.service
            .pause(
                &t.w.admin,
                schedule.id,
                Note::new("hold").expect("note"),
                &sagat_application::identity::RequestContext::default(),
            )
            .await
            .expect("pause");
        assert_eq!(
            legacy.tick().await.runs_started,
            0,
            "the read works again: the pass is stopped"
        );
        assert_eq!(
            t.runs_of(workflow).await.len(),
            1,
            "and the pause is honoured"
        );
    });
    assert_eq!(
        plain(&page, "cron_handover_read_consecutive_failures"),
        Some(0.0),
        "a read that works again resets the run of failures"
    );
}

// --- C13, C14: the tripwire survives a restart; a replica with no shard is alive --------------------------------------------------------

#[tokio::test]
async fn a_restart_does_not_tell_again_of_legacy_fires_already_told_and_a_new_one_is_told_once() {
    let t = triggers().await;
    t.publish(cron_workflow("restarted", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("restarted").await;
    t.reconcile().await;
    t.hand_over().await;
    let r = replica(&t, sagat_scheduler::TriggerSettings::default());
    r.keep().await;
    let fire = |seconds: i64| {
        let pool = t.w.db.pool.clone();
        async move {
            sqlx::query(
                "UPDATE workflow_triggers SET last_fire_at = now() + make_interval(secs => $2::float8) WHERE id = $1",
            )
            .bind(trigger)
            .bind(seconds)
            .execute(&pool)
            .await
            .expect("an old binary fires");
        }
    };
    fire(1).await;
    assert_eq!(
        r.pass.round().await.legacy_fires,
        0,
        "the first look after a start is the baseline: what is there is history"
    );
    fire(2).await;
    assert_eq!(r.pass.round().await.legacy_fires, 1, "a new fire is told");
    assert_eq!(r.pass.round().await.legacy_fires, 0, "once");
    let restarted = r.restarted(&t, sagat_scheduler::TriggerSettings::default());
    assert_eq!(
        restarted.round().await.legacy_fires,
        0,
        "a restart does not tell of the fires again"
    );
    fire(3).await;
    assert_eq!(
        restarted.round().await.legacy_fires,
        1,
        "and the next fire is told"
    );
}

#[test]
fn a_replica_that_holds_no_shard_still_says_its_loop_is_alive() {
    use crate::counted::recorded;
    let page = recorded(|| async {
        let t = triggers().await;
        // a replica that has not taken a lease: it holds no shard, and its round has nothing to do
        let r = replica(&t, sagat_scheduler::TriggerSettings::default());
        let round = r.pass.round().await;
        assert_eq!(round.drift, 0);
    });
    let stamp = plain(&page, "trigger_schedule_last_round_timestamp_seconds")
        .expect("the gauge of a loop that is alive, shard or none");
    assert!(stamp > 1.0e9, "{stamp}");
}

// --- C16: a trigger that is gone -----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_backfill_or_a_run_now_of_a_schedule_whose_trigger_is_gone_is_refused_with_a_documented_reason()
 {
    use sagat_application::schedule_firing::{Fired, TRIGGER_GONE, TriggerRequest};
    let t = triggers().await;
    t.publish(cron_workflow("gone", "0 * * * *", "UTC")).await;
    let (trigger, ..) = t.trigger_of("gone").await;
    t.reconcile().await;
    t.hand_over().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let now = time::OffsetDateTime::now_utc();
    let hour = now.unix_timestamp() - now.unix_timestamp() % 3_600;
    let start = time::OffsetDateTime::from_unix_timestamp(hour - 3 * 3_600).expect("instant");
    let end = time::OffsetDateTime::from_unix_timestamp(hour - 2 * 3_600).expect("instant");
    let backfill = t
        .service
        .create_backfill(
            &t.w.admin,
            schedule.id,
            &sagat_application::schedules::BackfillRequest {
                start,
                end,
                overlap: None,
            },
            &sagat_application::identity::RequestContext::default(),
        )
        .await
        .expect("backfill");
    // an older control plane drops the trigger
    sqlx::query("DELETE FROM workflow_triggers WHERE id = $1")
        .bind(trigger)
        .execute(&t.w.db.pool)
        .await
        .expect("dropped");
    let due = sagat_application::schedules::DueBackfill {
        tenant_id: schedule.tenant_id,
        schedule: schedule.id,
        id: backfill.id,
    };
    let stepped = t
        .firing
        .backfill_step(&due, None)
        .await
        .expect("a trigger that is gone is a refusal, not an error to repeat every round");
    assert_eq!(stepped, Fired::Refused(TRIGGER_GONE));

    let run_now = t
        .firing
        .trigger(
            &t.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: "run-now-gone-1",
            },
            &sagat_application::identity::RequestContext::default(),
        )
        .await;
    assert!(
        matches!(
            &run_now,
            Err(sagat_application::error::AppError::ScheduleConflict {
                reason: "SCHEDULE_TRIGGER_GONE"
            })
        ),
        "{run_now:?}"
    );
}

// --- D17: an existing name is not refused when it is not changed -------------------------------------------------------------------------

#[tokio::test]
async fn a_schedule_with_a_reserved_looking_name_can_be_updated_but_not_renamed_to_one() {
    let t = triggers().await;
    let hex = "0123456789abcdef0123456789abcdef";
    let reserved = format!("trigger-{hex}");
    t.api_schedule("before").await.expect("an API schedule");
    // it was made before the name was reserved
    sqlx::query("UPDATE schedules SET name = $1 WHERE name = 'before'")
        .bind(&reserved)
        .execute(&t.w.db.pool)
        .await
        .expect("an old name");
    let id: Uuid = sqlx::query_scalar("SELECT id FROM schedules WHERE name = $1")
        .bind(&reserved)
        .fetch_one(&t.w.db.pool)
        .await
        .expect("id");
    let update = t
        .service
        .update(
            &t.w.admin,
            sagat_domain::schedules::ScheduleId::new(id),
            t.api_write(&reserved).await,
            1,
            &sagat_application::identity::RequestContext::default(),
        )
        .await;
    assert!(update.is_ok(), "the name is not changed: {update:?}");
    let renamed = t
        .service
        .update(
            &t.w.admin,
            sagat_domain::schedules::ScheduleId::new(id),
            t.api_write(&format!("trigger-{}", "f".repeat(32))).await,
            2,
            &sagat_application::identity::RequestContext::default(),
        )
        .await;
    assert!(
        matches!(
            &renamed,
            Err(sagat_application::error::AppError::InvalidSchedule {
                reason: "SCHEDULE_NAME_RESERVED",
                ..
            })
        ),
        "{renamed:?}"
    );
}

// --- C16: the documented limit of a backfill over old legacy ticks -------------------------------------------------------------------

/// The legacy pass leaves no action row, only the deduplication key of the run (`trigger:{id}`, `cron-{epoch}`), and the keys
/// are purged after seven days (`idempotency_retention`). A backfill of such a tick after the purge cannot tell that the tick
/// was acted on and starts a second run. It is the platform's documented limit (G2021, the runbook 6.4); this test is what says it
/// is a limit and not a surprise: when the lookup is made to find those runs, this test is the one to turn into the opposite.
#[tokio::test]
async fn a_backfill_over_a_tick_the_legacy_pass_started_before_the_purge_of_its_key_starts_a_second_run_which_is_the_documented_limit()
 {
    use sagat_application::schedule_firing::Fired;
    let t = triggers().await;
    let (workflow, _) = t.publish(cron_workflow("purged", "0 * * * *", "UTC")).await;
    let (trigger, ..) = t.trigger_of("purged").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let now = time::OffsetDateTime::now_utc();
    let hour = now.unix_timestamp() - now.unix_timestamp() % 3_600;
    let start = time::OffsetDateTime::from_unix_timestamp(hour - 3 * 3_600).expect("instant");
    let end = time::OffsetDateTime::from_unix_timestamp(hour - 2 * 3_600).expect("instant");
    t.clock.set(now);
    sqlx::query("UPDATE workflow_triggers SET next_fire_at = $2 WHERE id = $1")
        .bind(trigger)
        .bind(start)
        .execute(&t.w.db.pool)
        .await
        .expect("due");
    t.w.dispatcher.tick().await;
    assert_eq!(
        t.runs_of(workflow).await.len(),
        1,
        "the legacy pass started the tick"
    );
    // the key is purged, as maintenance does when it is older than the retention
    sqlx::query("DELETE FROM command_dedupe WHERE scope = $1")
        .bind(format!("trigger:{trigger}"))
        .execute(&t.w.db.pool)
        .await
        .expect("the purge");
    t.hand_over().await;
    let backfill = t
        .service
        .create_backfill(
            &t.w.admin,
            schedule.id,
            &sagat_application::schedules::BackfillRequest {
                start,
                end: end + time::Duration::seconds(1),
                overlap: Some(sagat_domain::schedules::Overlap::AllowAll),
            },
            &sagat_application::identity::RequestContext::default(),
        )
        .await
        .expect("backfill");
    let due = sagat_application::schedules::DueBackfill {
        tenant_id: schedule.tenant_id,
        schedule: schedule.id,
        id: backfill.id,
    };
    for _ in 0..5 {
        if t.firing.backfill_step(&due, None).await.expect("step") == Fired::Idle {
            break;
        }
    }
    assert_eq!(
        t.runs_of(workflow).await.len(),
        3,
        "the two ticks of the range start, the first of them for the second time: the key that would have said otherwise is gone"
    );
}

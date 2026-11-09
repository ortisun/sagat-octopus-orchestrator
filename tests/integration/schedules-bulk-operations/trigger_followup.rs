//! What the reviews of the first build asked for (`T018-060`, `T018-061`; D2046 and after), written before the fixes and seen
//! failing: the handover refuses triggers that have not converged, the control of a trigger-owned schedule waits for the
//! engines to learn of the handover, a buffered run-now does not pause the cron trigger, a cron row this build cannot parse
//! fails one entry and not the round, the registry cannot be told to forget `schedules-v1` after the handover, an
//! activation recorded by hand is completed and not refused, a publication and the reconciliation never deadlock or undo
//! each other, and the names of the schedules of the triggers are reserved.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::ScheduleState;
use sagat_control_plane::RunError;
use sagat_control_plane::config::DatabaseConfig;
use sagat_control_plane::handover::{HandoverOutcome, HandoverRequest, schedule_trigger_handover};
use sagat_domain::schedules::{Note, Overlap};
use sagat_scheduler::TriggerSettings;
use time::Duration as TimeDuration;
use uuid::Uuid;

use crate::support::trigger_schedules::{Triggers, cron_workflow, second_version, triggers};
use crate::trigger_reconcile::{old_control_plane_publishes, replica};

pub(crate) fn database(t: &Triggers) -> DatabaseConfig {
    DatabaseConfig {
        url: t.w.db.url.clone(),
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    }
}

pub(crate) async fn command(
    t: &Triggers,
    expect_live: Option<u32>,
) -> Result<HandoverOutcome, RunError> {
    schedule_trigger_handover(
        &database(t),
        &HandoverRequest {
            operator: Some("ana".to_owned()),
            expect_live,
            dry_run: false,
        },
    )
    .await
}

pub(crate) fn declined(result: Result<HandoverOutcome, RunError>) -> String {
    match result {
        Err(RunError::Declined(message)) => message,
        other => panic!("expected a refusal in words, got {other:?}"),
    }
}

pub(crate) async fn state(t: &Triggers) -> String {
    sqlx::query_scalar("SELECT state FROM schedule_trigger_handover")
        .fetch_one(&t.w.db.pool)
        .await
        .expect("state")
}

/// A live engine of the registry that lists `schedules-v1`, written by the owner.
pub(crate) async fn engine(t: &Triggers) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO serving_instances (instance_id, kind, binary_version, capabilities, started_at, last_seen, token_hash) \
         VALUES (gen_random_uuid(), 'EXECUTION_ENGINE', 'test', ARRAY['schedules-v1'], clock_timestamp(), clock_timestamp(), \
                 sha256(decode(repeat('09', 32), 'hex'))) RETURNING instance_id",
    )
    .fetch_one(&t.w.db.pool)
    .await
    .expect("an engine")
}

/// Waits (a backend on a lock, not a time) until `waiting` is true or `finished` says the work already ended.
pub(crate) async fn until_a_backend_waits(t: &Triggers, stage: &str) {
    crate::support::bounded::within(stage, Duration::from_secs(60), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND pid <> pg_backend_pid() AND wait_event_type = 'Lock'",
            )
            .fetch_one(&t.w.db.pool)
            .await
            .expect("backends");
            if waiting > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

// --- A: the handover refuses what has not converged ----------------------------------------------------------------------

#[tokio::test]
async fn the_handover_refuses_a_trigger_with_no_next_time_or_no_schedule_and_passes_once_they_converge()
 {
    let t = triggers().await;
    t.publish(cron_workflow("converges", "0 * * * *", "UTC"))
        .await;
    // publication has no calendar: the schedule has no next time until the reconciliation plans it
    let message = declined(command(&t, Some(0)).await);
    assert!(
        message.contains("1 cron trigger(s)") && message.contains("no schedule or no next time"),
        "{message}"
    );
    let (trigger, ..) = t.trigger_of("converges").await;
    assert!(
        message.contains(&trigger.to_string()),
        "it names the trigger: {message}"
    );
    assert_eq!(state(&t).await, "LEGACY", "nothing was recorded");

    t.reconcile().await;
    // a trigger an older control plane published has no schedule at all
    let old = Uuid::now_v7();
    old_control_plane_publishes(&t, "converges", old, "15 * * * *", "UTC").await;
    let message = declined(command(&t, Some(0)).await);
    assert!(message.contains(&old.to_string()), "{message}");
    assert_eq!(state(&t).await, "LEGACY");
    let history: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_requirement_history")
        .fetch_one(&t.w.db.pool)
        .await
        .expect("history");
    assert_eq!(history, 0, "the requirement was not recorded either");

    t.reconcile().await;
    let done = command(&t, Some(0))
        .await
        .expect("everything has converged");
    assert!(
        matches!(done, HandoverOutcome::HandedOver { .. }),
        "{done:?}"
    );
}

#[tokio::test]
async fn a_dry_run_reports_the_triggers_that_have_not_converged() {
    let t = triggers().await;
    t.publish(cron_workflow("dry", "0 * * * *", "UTC")).await;
    let dry = schedule_trigger_handover(
        &database(&t),
        &HandoverRequest {
            operator: None,
            expect_live: Some(0),
            dry_run: true,
        },
    )
    .await;
    let message = declined(dry);
    assert!(message.contains("no schedule or no next time"), "{message}");
}

// --- I: a requirement recorded by hand is completed, not refused ---------------------------------------------------------

#[tokio::test]
async fn a_requirement_recorded_by_hand_while_legacy_is_completed_by_the_command_after_its_checks()
{
    use sagat_application::instances::{Activation, Capability, InstanceKind};
    let t = triggers().await;
    t.publish(cron_workflow("by-hand", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    engine(&t).await;
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
        .expect("recorded by hand");
    assert_eq!(state(&t).await, "LEGACY");

    let done = command(&t, Some(1))
        .await
        .expect("the command completes it");
    assert!(
        matches!(done, HandoverOutcome::HandedOver { live: 1, .. }),
        "{done:?}"
    );
    assert_eq!(state(&t).await, "HANDED_OVER");
    let by: String = sqlx::query_scalar("SELECT changed_by FROM schedule_trigger_handover")
        .fetch_one(&t.w.db.pool)
        .await
        .expect("who");
    assert_eq!(by, "ana");
}

#[tokio::test]
async fn a_requirement_recorded_by_hand_is_still_refused_when_the_checks_fail() {
    use sagat_application::instances::{Activation, Capability, InstanceKind};
    let t = triggers().await;
    t.publish(cron_workflow("by-hand-bad", "0 * * * *", "UTC"))
        .await;
    engine(&t).await;
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
        .expect("recorded by hand");
    // the trigger has no next time yet: the same refusal as without the requirement
    let message = declined(command(&t, Some(1)).await);
    assert!(message.contains("no schedule or no next time"), "{message}");
    assert_eq!(state(&t).await, "LEGACY");
}

// --- O: an engine that goes silent between the check and the lock --------------------------------------------------------

#[tokio::test]
async fn an_engine_that_goes_silent_between_the_check_and_the_lock_stops_the_handover_with_nothing_recorded()
 {
    let t = triggers().await;
    t.publish(cron_workflow("race", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    engine(&t).await;
    let second = engine(&t).await;

    // hold the requirement epoch: the command passes its first check and then waits for the lock of the activation
    let mut held = t.w.db.pool.begin().await.expect("hold");
    sqlx::query("SELECT platform_requirement_lock()")
        .execute(&mut *held)
        .await
        .expect("the epoch");
    let url = t.w.db.url.clone();
    let running = tokio::spawn(async move {
        schedule_trigger_handover(
            &DatabaseConfig {
                url,
                max_connections: 2,
                statement_timeout: Duration::from_secs(5),
            },
            &HandoverRequest {
                operator: Some("ana".to_owned()),
                expect_live: Some(2),
                dry_run: false,
            },
        )
        .await
    });
    until_a_backend_waits(&t, "the command waits for the epoch").await;
    // the engine goes silent while the command waits: it is not live any more when the lock is taken
    sqlx::query("UPDATE serving_instances SET last_seen = clock_timestamp() - interval '60 seconds' WHERE instance_id = $1")
        .bind(second)
        .execute(&t.w.db.pool)
        .await
        .expect("age it");
    held.rollback().await.expect("release the epoch");

    let message = declined(running.await.expect("the command"));
    assert!(
        message.contains("changed while the handover was being made"),
        "{message}"
    );
    assert_eq!(state(&t).await, "LEGACY");
    let history: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_requirement_history")
        .fetch_one(&t.w.db.pool)
        .await
        .expect("history");
    assert_eq!(history, 0);
}

/// A: a publication of an older control plane that is in flight when the handover counts is waited for and then counted:
/// the refusal names the trigger (the census under the table lock), it is not left to the generic refusal of the function.
#[tokio::test]
async fn a_trigger_published_while_the_handover_counts_is_waited_for_and_named_in_the_refusal() {
    let t = triggers().await;
    t.publish(cron_workflow("in-flight", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    engine(&t).await;
    let late = Uuid::from_u128(0x99);
    let mut publishing = t.w.db.pool.begin().await.expect("an older control plane");
    sqlx::query(
        "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
         SELECT $2, tenant_id, namespace_id, workflow_id, version_id, 'cron', '0 0 * * *', 'UTC' \
           FROM workflow_triggers WHERE id = (SELECT t.id FROM workflow_triggers t JOIN workflows w \
                                              ON w.id = t.workflow_id WHERE w.name = $1 \
                                              ORDER BY t.id LIMIT 1)",
    )
    .bind("in-flight")
    .bind(late)
    .execute(&mut *publishing)
    .await
    .expect("a trigger without a schedule, uncommitted");
    let url = t.w.db.url.clone();
    let running = tokio::spawn(async move {
        schedule_trigger_handover(
            &DatabaseConfig {
                url,
                max_connections: 2,
                statement_timeout: Duration::from_secs(5),
            },
            &HandoverRequest {
                operator: Some("ana".to_owned()),
                expect_live: Some(1),
                dry_run: false,
            },
        )
        .await
    });
    until_a_backend_waits(&t, "the census waits for the publication").await;
    publishing.commit().await.expect("the publication commits");
    let message = declined(running.await.expect("the command"));
    assert!(
        message.contains(&late.to_string()),
        "the refusal names the trigger the census counted after the wait: {message}"
    );
    assert_eq!(state(&t).await, "LEGACY");
}

// --- M: nothing makes the fleet forget the requirement after the handover ------------------------------------------------

#[tokio::test]
async fn the_requirement_cannot_be_removed_after_the_handover() {
    use sagat_application::instances::Capability;
    let t = triggers().await;
    t.publish(cron_workflow("removal", "0 * * * *", "UTC"))
        .await;
    t.hand_over().await;
    let actor = sagat_domain::identity::Subject::parse("somebody").expect("subject");
    let removed = sagat_infra_postgres::PgInstanceStore::new(t.w.db.pool.clone())
        .remove(Capability::SchedulesV1, &actor, |_| {
            Box::pin(async { Ok(()) })
        })
        .await;
    assert!(
        removed.is_err(),
        "an engine without the schedule pass could register and nothing would fire the triggers: {removed:?}"
    );
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform_requirements WHERE capability = 'schedules-v1'",
    )
    .fetch_one(&t.w.db.pool)
    .await
    .expect("requirements");
    assert_eq!(left, 1, "the requirement stays");
}

// --- N: the control waits until the engines have learned of the handover ---------------------------------------------------

#[tokio::test]
async fn the_control_of_a_trigger_owned_schedule_opens_after_the_engines_have_had_time_to_learn_of_the_handover()
 {
    let t = triggers().await;
    t.publish(cron_workflow("window", "0 * * * *", "UTC")).await;
    t.hand_over_now().await;
    let (trigger, ..) = t.trigger_of("window").await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let pause = || async {
        t.service
            .pause(
                &t.w.admin,
                schedule.id,
                Note::new("hold").expect("note"),
                &RequestContext::default(),
            )
            .await
            .map(|_| ())
    };
    let pending = Err(AppError::ScheduleConflict {
        reason: "SCHEDULE_TRIGGER_HANDOVER_PENDING",
    });
    assert_eq!(
        pause().await,
        pending,
        "at the commit an engine may still believe LEGACY for up to two seconds"
    );
    t.move_the_handover_back(1).await;
    assert_eq!(pause().await, pending, "one second later it still may");
    t.move_the_handover_back(10).await;
    assert!(
        pause().await.is_ok(),
        "after the window the control is open"
    );
}

// --- D: a buffered run-now does not pause the cron trigger -----------------------------------------------------------------

#[tokio::test]
async fn a_buffered_run_now_of_a_trigger_owned_schedule_starts_when_the_open_run_ends_and_pauses_nothing()
 {
    let t = triggers().await;
    let (workflow, _) = t
        .publish(cron_workflow("buffered", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("buffered").await;
    t.hand_over().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let run_now = |key: &'static str, overlap| {
        let (firing, admin, id) = (&t.firing, &t.w.admin, schedule.id);
        async move {
            firing
                .trigger(
                    admin,
                    id,
                    &TriggerRequest { overlap, key },
                    &RequestContext::default(),
                )
                .await
                .expect("run now")
        }
    };
    let first = run_now("run-now-0001", None).await;
    assert_eq!(first.outcome, "STARTED");
    t.clock.advance(TimeDuration::seconds(1));
    let second = run_now("run-now-0002", Some(Overlap::BufferOne)).await;
    assert_eq!(
        second.outcome, "BUFFERED",
        "a run is open: the second waits"
    );

    // the open run ends and the waiting action is released by the pass
    sqlx::query("UPDATE runs SET status = 'SUCCEEDED', finished_at = now() WHERE id = $1")
        .bind(first.run_id.expect("run"))
        .execute(&t.w.db.pool)
        .await
        .expect("the run ends");
    t.clock.advance(TimeDuration::seconds(30));
    let mut steps = 0;
    for _ in 0..5 {
        match t.fire(&schedule).await {
            Fired::Acted => steps += 1,
            _ => break,
        }
    }
    assert!(steps >= 1);
    let after = t.read(&schedule).await;
    assert_eq!(
        after.state,
        ScheduleState::Active,
        "the cron trigger is not paused by its own engine"
    );
    assert_eq!(
        t.runs_of(workflow).await.len(),
        2,
        "the waiting run started"
    );
    let outcomes: Vec<String> = t
        .actions_of(&schedule)
        .await
        .into_iter()
        .map(|(_, outcome, _)| outcome)
        .collect();
    assert!(outcomes.iter().all(|o| o == "STARTED"), "{outcomes:?}");
    assert!(
        after.next_action_at.is_some(),
        "its next time is still planned"
    );
}

// --- E: a cron row this build cannot parse fails its entry, not the round -------------------------------------------------

#[tokio::test]
async fn a_cron_row_this_build_cannot_parse_fails_its_own_entry_and_not_the_round() {
    let t = triggers().await;
    t.publish(cron_workflow("base", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    // six fields: it passes the shape check of an older control plane, and this build's grammar refuses it
    old_control_plane_publishes(&t, "base", Uuid::from_u128(1), "0 0 * * * *", "UTC").await;
    let good = Uuid::now_v7();
    old_control_plane_publishes(&t, "base", good, "5 * * * *", "UTC").await;

    let r = replica(&t, TriggerSettings::default());
    r.keep().await;
    let round = r.pass.round().await;
    assert_eq!(
        round.failed, 1,
        "the entry that cannot be parsed fails and backs off: {round:?}"
    );
    assert_eq!(
        round.created, 1,
        "and the entry beside it is made in the same round: {round:?}"
    );
    assert!(t.schedule_of(good).await.is_some());
    let next = r.pass.round().await;
    assert_eq!(
        next.failed, 0,
        "the stuck entry is left out for a while: {next:?}"
    );
}

// --- L: the names of the schedules of the triggers are reserved --------------------------------------------------------------

#[tokio::test]
async fn an_api_schedule_cannot_take_a_name_a_trigger_schedule_may_have() {
    let t = triggers().await;
    let hex = "0123456789abcdef0123456789abcdef";
    for name in [
        format!("trigger-{hex}"),
        format!("trigger-{hex}-1"),
        format!("trigger-{hex}-3"),
    ] {
        let refused = t.api_schedule(&name).await;
        assert!(
            matches!(&refused, Err(AppError::InvalidSchedule { reason: "SCHEDULE_NAME_RESERVED", field: Some(f) }) if f == "/name"),
            "{name}: {refused:?}"
        );
    }
    // other names that start the same way are the tenant's
    for name in [
        "trigger-happy",
        "trigger-0123",
        &format!("trigger-{}", &hex[..31]),
    ] {
        t.api_schedule(name)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    }
}

// --- Q: a publication that is in flight is not undone by the reconciliation ---------------------------------------------------

#[tokio::test]
async fn a_publication_serialises_with_the_removal_of_the_schedule_it_keeps() {
    let t = triggers().await;
    let (workflow, _) = t.publish(cron_workflow("serial", "0 * * * *", "UTC")).await;
    let (trigger, ..) = t.trigger_of("serial").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    // an older control plane dropped the trigger: the schedule is an orphan, and the reconciliation is at the point of
    // tombstoning it (it holds the row and has read the trigger as absent) when a publication brings the trigger back
    sqlx::query("DELETE FROM workflow_triggers WHERE id = $1")
        .bind(trigger)
        .execute(&t.w.db.pool)
        .await
        .expect("dropped");
    let mut removal =
        t.w.db
            .pool
            .begin()
            .await
            .expect("the reconciliation, mid-flight");
    sqlx::query("SELECT id FROM schedules WHERE id = $1 FOR UPDATE")
        .bind(schedule.id.as_uuid())
        .fetch_one(&mut *removal)
        .await
        .expect("its lock");
    sqlx::query(
        "UPDATE schedules SET state = 'DELETED', deleted_at = now(), next_action_at = NULL, next_actual_at = NULL, \
         state_version = state_version + 1 WHERE id = $1",
    )
    .bind(schedule.id.as_uuid())
    .execute(&mut *removal)
    .await
    .expect("the tombstone, uncommitted");

    let w = t.w.clone();
    let publication = tokio::spawn(async move {
        w.publish_new_version(workflow, second_version("serial", "0 * * * *", "UTC"))
            .await
    });
    // the publication keeps the trigger: it must wait for the row of the schedule instead of reading it as it was
    crate::support::bounded::within(
        "the publication reaches the schedule row",
        Duration::from_secs(60),
        async {
            loop {
                if publication.is_finished() {
                    break;
                }
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND pid <> pg_backend_pid() AND wait_event_type = 'Lock'",
                )
                .fetch_one(&t.w.db.pool)
                .await
                .expect("backends");
                if waiting > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    assert!(
        !publication.is_finished(),
        "the publication did not wait for the row of the schedule: it read it as ACTIVE and left a tombstone beside a live trigger"
    );
    removal.commit().await.expect("the reconciliation commits");
    publication.await.expect("the publication");

    let (again, ..) = t.trigger_of("serial").await;
    assert_eq!(again, trigger);
    let after = t.read(&schedule).await;
    assert_eq!(
        after.state,
        ScheduleState::Active,
        "a trigger that is in force has an alive schedule: the publication saw the tombstone and revived it"
    );
}

/// Q: the removal of the schedule of a trigger that is gone looks at the trigger under the trigger's row lock, so that a
/// trigger somebody is deleting or rewriting at that moment is waited for and not read as it was.
#[tokio::test]
async fn the_removal_of_a_schedule_waits_for_a_trigger_that_is_being_deleted_and_then_removes_it() {
    use sagat_application::schedule_triggers::{Change, TriggerScheduleStore as _};
    let t = triggers().await;
    t.publish(cron_workflow("being-deleted", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("being-deleted").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let mut deleting =
        t.w.db
            .pool
            .begin()
            .await
            .expect("an older control plane, mid-flight");
    sqlx::query("DELETE FROM workflow_triggers WHERE id = $1")
        .bind(trigger)
        .execute(&mut *deleting)
        .await
        .expect("the trigger is being deleted, uncommitted");

    let store = t.store.clone();
    let change = Change::Remove {
        tenant_id: schedule.tenant_id,
        schedule: schedule.id,
        trigger,
        expected_state_version: schedule.state_version,
    };
    let removal = tokio::spawn(async move { store.apply(&change, None).await });
    crate::support::bounded::within(
        "the removal reaches the row of the trigger",
        Duration::from_secs(60),
        async {
            loop {
                if removal.is_finished() {
                    break;
                }
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND pid <> pg_backend_pid() AND wait_event_type = 'Lock'",
                )
                .fetch_one(&t.w.db.pool)
                .await
                .expect("backends");
                if waiting > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    assert!(
        !removal.is_finished(),
        "the removal did not wait for the row of the trigger: it read the trigger as it was"
    );
    deleting.commit().await.expect("the deletion commits");
    removal
        .await
        .expect("the removal")
        .expect("the trigger is gone: the schedule is removed");
    assert_eq!(
        t.read(&schedule).await.state,
        ScheduleState::Deleted,
        "the schedule of the deleted trigger is a tombstone"
    );
}

// --- C: the lock order of the reconciliation and of a publication -------------------------------------------------------------
// (the order of the slots of `tests/support/lock_order.rs` is checked against real locks here and in `trigger_round3.rs`, not
// against a list written in the test)

#[tokio::test]
async fn a_change_of_the_reconciliation_and_a_publication_that_hold_the_trigger_row_never_deadlock()
{
    use sagat_application::schedule_triggers::{HandoverState, Reconciled};
    let t = triggers().await;
    t.publish(cron_workflow("order", "0 * * * *", "UTC")).await;
    let (trigger, ..) = t.trigger_of("order").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    // the two changes that take both rows: the expression drifted (Respec), and an active schedule has lost its next time (SetNext)
    for case in ["respec", "set-next"] {
        if case == "respec" {
            sqlx::query("UPDATE workflow_triggers SET cron = '30 * * * *' WHERE id = $1")
                .bind(trigger)
                .execute(&t.w.db.pool)
                .await
                .expect("drift");
        } else {
            sqlx::query(
                "UPDATE schedules SET next_action_at = NULL, next_actual_at = NULL WHERE id = $1",
            )
            .bind(schedule.id.as_uuid())
            .execute(&t.w.db.pool)
            .await
            .expect("no next time");
        }
        let drift = t.drift().await;
        assert_eq!(drift.len(), 1, "{case}: {drift:?}");
        // a publication holds the trigger's row first (it writes the trigger) ...
        let mut publication =
            t.w.db
                .pool
                .begin()
                .await
                .expect("the publication, mid-flight");
        sqlx::query("SELECT id FROM workflow_triggers WHERE id = $1 FOR UPDATE")
            .bind(trigger)
            .fetch_one(&mut *publication)
            .await
            .expect("the trigger row");
        let reconciler_store = t.store.clone();
        let handover = HandoverState::Legacy;
        let one = drift[0].clone();
        let r = t.reconciler_for_task();
        let running = tokio::spawn(async move {
            let _ = &reconciler_store;
            r.reconcile(&one, &handover, None).await
        });
        until_a_backend_waits(&t, "the reconciliation waits for the trigger row").await;
        // ... and its second lock is the schedule's row: the reconciliation must not be holding it while it waits for the first
        sqlx::query("SELECT id FROM schedules WHERE id = $1 FOR UPDATE")
            .bind(schedule.id.as_uuid())
            .fetch_one(&mut *publication)
            .await
            .unwrap_or_else(|e| panic!("{case}: the publication was a deadlock victim: {e}"));
        publication.commit().await.expect("the publication commits");
        let done = running
            .await
            .expect("the reconciliation")
            .unwrap_or_else(|e| panic!("{case}: {e:?}"));
        assert!(
            matches!(done, Reconciled::Repaired | Reconciled::Refused(_)),
            "{case}: {done:?}"
        );
        t.reconcile().await;
    }
}

// --- K: a legacy pass that fires after the handover is seen ---------------------------------------------------------------------

#[tokio::test]
async fn a_cron_trigger_a_legacy_pass_fires_after_the_handover_settled_is_told_of_once() {
    let t = triggers().await;
    t.publish(cron_workflow("old-binary", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("old-binary").await;
    t.reconcile().await;
    let r = replica(&t, TriggerSettings::default());
    r.keep().await;
    // before the handover a legacy fire is the normal state of things, however late it is: a minute after the row of the
    // handover was made, so that only the state of the handover (and not the age of the row) keeps it out of the tripwire
    sqlx::query(
        "UPDATE workflow_triggers SET last_fire_at = now() + interval '1 minute' WHERE id = $1",
    )
    .bind(trigger)
    .execute(&t.w.db.pool)
    .await
    .expect("a legacy fire");
    assert_eq!(
        r.pass.round().await.legacy_fires,
        0,
        "while the handover has not happened"
    );
    sqlx::query("UPDATE workflow_triggers SET last_fire_at = now() WHERE id = $1")
        .bind(trigger)
        .execute(&t.w.db.pool)
        .await
        .expect("a legacy fire from before the handover");
    t.hand_over().await;
    assert_eq!(
        r.pass.round().await.legacy_fires,
        0,
        "a fire from before the handover is not one after it"
    );
    // an old binary fires the trigger now: what its pass writes is the tick's time on the trigger
    sqlx::query(
        "UPDATE workflow_triggers SET last_fire_at = now() + interval '1 second' WHERE id = $1",
    )
    .bind(trigger)
    .execute(&t.w.db.pool)
    .await
    .expect("the old binary fires");
    assert_eq!(r.pass.round().await.legacy_fires, 1, "the tripwire sees it");
    assert_eq!(r.pass.round().await.legacy_fires, 0, "and says it once");
}

// --- H: what a reconciliation that never converges looks like ------------------------------------------------------------------

#[tokio::test]
async fn a_stuck_entry_shows_as_backing_off_and_as_unconverged_even_while_it_is_left_out_of_the_drift()
 {
    let t = triggers().await;
    t.publish(cron_workflow("base", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    old_control_plane_publishes(&t, "base", Uuid::from_u128(1), "0 0 * * * *", "UTC").await;
    let r = replica(&t, TriggerSettings::default());
    r.keep().await;
    let first = r.pass.round().await;
    assert_eq!(first.failed, 1);
    assert_eq!(
        first.unconverged,
        Some(1),
        "the first round measures how far the schedules are from the triggers: {first:?}"
    );
    let second = r.pass.round().await;
    assert_eq!(second.drift, 0, "the stuck entry is left out of the drift");
    assert_eq!(
        second.backing_off, 1,
        "and is counted as backing off, so that the gauge does not read zero: {second:?}"
    );
    assert_eq!(
        second.unconverged, None,
        "the convergence is measured every sixth round"
    );
    for _ in 0..4 {
        r.pass.round().await;
    }
    let sixth = r.pass.round().await;
    assert_eq!(
        sixth.unconverged,
        Some(1),
        "and again: it never reads zero while the trigger has no schedule: {sixth:?}"
    );
}

// --- G: the indexes the reconciliation reads by -------------------------------------------------------------------------------

#[tokio::test]
async fn the_reconciliation_reads_the_triggers_and_the_schedules_by_shard_through_partial_indexes()
{
    let t = triggers().await;
    for index in [
        "workflow_triggers_cron_shard_idx",
        "schedules_trigger_shard_idx",
        "workflow_triggers_cron_last_fire_idx",
    ] {
        let found: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_indexes WHERE indexname = $1 AND indexdef LIKE '% WHERE %'",
        )
        .bind(index)
        .fetch_one(&t.w.db.pool)
        .await
        .expect("index");
        assert_eq!(found, 1, "{index} is a partial index");
    }
}

#[tokio::test]
async fn the_api_says_whether_the_control_of_the_schedules_of_the_triggers_is_open() {
    let t = triggers().await;
    t.publish(cron_workflow("controllable", "0 * * * *", "UTC"))
        .await;
    assert!(
        !t.service.trigger_control_open().await.expect("read"),
        "before the handover"
    );
    t.hand_over_now().await;
    assert!(
        !t.service.trigger_control_open().await.expect("read"),
        "at the commit: the engines may still believe LEGACY"
    );
    t.move_the_handover_back(10).await;
    assert!(
        t.service.trigger_control_open().await.expect("read"),
        "after the window"
    );
}

/// J over HTTP: the flag is on the schedules a cron trigger owns and only on them, and it follows the window of the handover.
#[tokio::test]
async fn the_schedules_over_http_say_whether_the_control_of_a_trigger_owned_one_is_open_and_say_nothing_of_the_others()
 {
    use crate::support::worker::{chain_ir, platform};
    use reqwest::Method;
    use serde_json::{Value, json};

    let p = platform().await;
    let namespace = p.namespace_id().await;
    let target = p.publish("target", &chain_ir("target")).await;
    let mut ir = chain_ir("cronned");
    ir["triggers"] = json!([{"kind": "cron", "cron": "0 * * * *", "timezone": "UTC"}]);
    p.publish("cronned", &ir).await;
    // publication makes the schedule without a next time (it has no calendar); the reconciliation of an engine plans it, which
    // this API has none of, so the plan is written as the pass would
    sqlx::query(
        "UPDATE schedules SET next_action_at = now() + interval '1 hour', next_actual_at = now() + interval '1 hour' \
          WHERE origin = 'TRIGGER'",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("the next time the reconciliation plans");
    let base = format!("/namespaces/{namespace}/schedules");
    let (status, made) = p
        .rest(
            Method::POST,
            &base,
            Some(&json!({
                "name": "mine",
                "spec": {"intervals": [{"everyMs": 3_600_000}]},
                "action": {"workflowId": target, "input": {"region": "eu"}}
            })),
        )
        .await;
    assert_eq!(status, 201, "{made}");
    assert!(
        made.get("controllable").is_none(),
        "an API schedule: {made}"
    );
    let api_id = made["scheduleId"].as_str().expect("id").to_owned();

    // the trigger-owned one and the API one, as a list and as a description
    async fn both(
        p: &crate::support::worker::Platform,
        base: &str,
        api_id: &str,
    ) -> (Value, Value, Value, Value) {
        let (status, page) = p.rest(Method::GET, base, None).await;
        assert_eq!(status, 200, "{page}");
        let items = page["items"].as_array().expect("items").clone();
        let pick = |origin: &str| -> Value {
            items
                .iter()
                .find(|item| item["origin"] == origin)
                .unwrap_or_else(|| panic!("a {origin} schedule in {page}"))
                .clone()
        };
        let (owned, own) = (pick("TRIGGER"), pick("API"));
        let (status, described) = p
            .rest(
                Method::GET,
                &format!("{base}/{}", owned["scheduleId"].as_str().expect("id")),
                None,
            )
            .await;
        assert_eq!(status, 200, "{described}");
        let (status, described_own) = p.rest(Method::GET, &format!("{base}/{api_id}"), None).await;
        assert_eq!(status, 200, "{described_own}");
        (owned, own, described, described_own)
    }
    let (owned, own, described, described_own) = both(&p, &base, &api_id).await;
    assert_eq!(owned["controllable"], json!(false), "before the handover");
    assert_eq!(described["controllable"], json!(false), "described, before");
    assert!(own.get("controllable").is_none(), "{own}");
    assert!(
        described_own.get("controllable").is_none(),
        "{described_own}"
    );

    // the handover, as the operator command makes it, on the owner's connection
    use sagat_application::instances::{Activation, Capability, InstanceKind};
    let operator = sagat_domain::identity::Subject::parse("test-operator").expect("subject");
    sagat_infra_postgres::PgInstanceStore::new(p.api.db.pool.clone())
        .activate(
            Activation {
                capability: Capability::SchedulesV1,
                kinds: &[InstanceKind::ExecutionEngine],
                actor: &operator,
            },
            |conn| {
                Box::pin(async move {
                    sagat_infra_postgres::complete_handover(conn, "test-operator")
                        .await
                        .map(|_| ())
                })
            },
        )
        .await
        .expect("the handover");
    let (owned, _, described, _) = both(&p, &base, &api_id).await;
    assert_eq!(owned["controllable"], json!(false), "at the commit");
    assert_eq!(
        described["controllable"],
        json!(false),
        "described, at the commit"
    );
    let owned_id = owned["scheduleId"].as_str().expect("id").to_owned();
    let (status, refused) = p
        .rest(
            Method::POST,
            &format!("{base}/{owned_id}/pause"),
            Some(&json!({"note": "too early"})),
        )
        .await;
    assert_eq!(status, 409, "{refused}");
    assert_eq!(
        refused["details"]["reason"],
        "SCHEDULE_TRIGGER_HANDOVER_PENDING"
    );

    sqlx::query(
        "ALTER TABLE schedule_trigger_handover DISABLE TRIGGER schedule_trigger_handover_guard",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("disable the guard");
    sqlx::query(
        "UPDATE schedule_trigger_handover SET changed_at = changed_at - interval '10 seconds'",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("move it back");
    sqlx::query("ALTER TABLE schedule_trigger_handover ENABLE ALWAYS TRIGGER schedule_trigger_handover_guard")
        .execute(&p.api.db.pool)
        .await
        .expect("enable the guard");
    let (owned, own, described, described_own) = both(&p, &base, &api_id).await;
    assert_eq!(owned["controllable"], json!(true), "after the window");
    assert_eq!(described["controllable"], json!(true), "described, after");
    assert!(own.get("controllable").is_none(), "{own}");
    assert!(
        described_own.get("controllable").is_none(),
        "{described_own}"
    );
    let (status, paused) = p
        .rest(
            Method::POST,
            &format!("{base}/{owned_id}/pause"),
            Some(&json!({"note": "maintenance"})),
        )
        .await;
    assert_eq!(status, 200, "{paused}");
    assert_eq!(paused["state"], "PAUSED");
    assert_eq!(paused["controllable"], json!(true));
}

/// L: a trigger none of whose names can be used (a control plane older than the reservation let a tenant take them) gets no
/// schedule, the pass says so and counts it, and the next round tries again. A recorder of the test's own, on a runtime of
/// one thread.
#[test]
fn a_trigger_whose_every_schedule_name_is_taken_is_counted_and_left_for_the_next_round() {
    use crate::counted::{recorded, series};
    use sagat_application::schedule_triggers::trigger_schedule_name;

    let page = recorded(|| async {
        let t = triggers().await;
        t.publish(cron_workflow("crowded", "0 * * * *", "UTC"))
            .await;
        t.reconcile().await;
        let id = Uuid::from_u128(0x77);
        let base = trigger_schedule_name(id).as_str().to_owned();
        let names = [
            base.clone(),
            format!("{base}-1"),
            format!("{base}-2"),
            format!("{base}-3"),
        ];
        for (n, name) in names.iter().enumerate() {
            let filler = format!("filler-{n}");
            t.api_schedule(&filler).await.expect("a schedule");
            sqlx::query("UPDATE schedules SET name = $1 WHERE name = $2")
                .bind(name)
                .bind(&filler)
                .execute(&t.w.db.pool)
                .await
                .expect("a name an old control plane let a tenant take");
        }
        old_control_plane_publishes(&t, "crowded", id, "0 0 * * *", "UTC").await;
        let r = replica(&t, TriggerSettings::default());
        r.keep().await;
        let round = r.pass.round().await;
        assert_eq!(
            (round.created, round.refused),
            (0, 1),
            "the entry is refused, not made: {round:?}"
        );
        assert!(t.schedule_of(id).await.is_none(), "no schedule was made");
    });
    assert_eq!(
        series(
            &page,
            "trigger_schedules_unmade_total",
            &[("reason", "names_taken")]
        ),
        Some(1.0),
        "{page}"
    );
}

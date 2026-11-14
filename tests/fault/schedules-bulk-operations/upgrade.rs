//! The rolling upgrade of the cron triggers (`T018-061`, `SC-018-005`, `FR-018-017`, `NFR-018-004`): an old dispatcher (the
//! legacy cron pass) and the new schedule pass reach the same tick, in either order, with the connection of the first one
//! killed after each write it makes; whatever happens, one run starts, with feature 004's key and document, and the schedule
//! records `STARTED` once or `ALREADY_ACTED`. The reconciliation and the handover are cut off at each of their writes too:
//! nothing is half done and the next attempt finishes it.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use sagat_application::schedules::ScheduleRecord;
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::RunId;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::support::trigger_schedules::{Triggers, cron_workflow, triggers};
use crate::tripwire::Tripwire;

#[derive(Clone, Copy, Debug)]
enum Pass {
    /// The cron pass of an old (or new, while `LEGACY`) engine.
    Legacy,
    /// The schedule pass of a new engine.
    Schedule,
}

async fn run(t: &Triggers, pass: Pass, schedule: &ScheduleRecord) {
    match pass {
        Pass::Legacy => {
            t.w.dispatcher.tick().await;
        }
        Pass::Schedule => {
            let _ = t.firing.fire(&t.due(schedule), None).await;
        }
    }
}

/// A cron workflow whose tick (the last whole hour) is due for both passes.
async fn due(t: &Triggers, name: &str) -> (WorkflowId, Uuid, ScheduleRecord, OffsetDateTime) {
    let (workflow, _) = t.publish(cron_workflow(name, "0 * * * *", "UTC")).await;
    let (trigger, ..) = t.trigger_of(name).await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let tick = Triggers::last_hour();
    t.due_at(trigger, &schedule, tick).await;
    t.clock.set(OffsetDateTime::now_utc());
    let schedule = t.read(&schedule).await;
    (workflow, trigger, schedule, tick)
}

/// The backoff a failure asked for has passed: the process that was killed has restarted.
async fn restart(t: &Triggers) {
    sqlx::query("UPDATE workflow_triggers SET retry_at = NULL")
        .execute(&t.w.db.pool)
        .await
        .expect("trigger backoff");
    // a refused admission moves the time to its retry; the backoff has passed, so the time is due again
    sqlx::query("UPDATE schedules SET retry_at = NULL, next_actual_at = next_action_at WHERE next_action_at IS NOT NULL")
        .execute(&t.w.db.pool)
        .await
        .expect("schedule backoff");
}

/// What has to be true of a tick both passes have reached, whichever started it.
async fn assert_one_run(
    t: &Triggers,
    workflow: WorkflowId,
    trigger: Uuid,
    schedule: &ScheduleRecord,
    tick: OffsetDateTime,
    context: &str,
) {
    let runs = t.runs_of(workflow).await;
    assert_eq!(runs.len(), 1, "{context}: exactly one run for the tick");
    let keys: Vec<(String, String)> =
        sqlx::query_as("SELECT scope, idempotency_key FROM command_dedupe WHERE scope = $1")
            .bind(format!("trigger:{trigger}"))
            .fetch_all(&t.w.db.pool)
            .await
            .expect("keys");
    assert_eq!(
        keys,
        [(
            format!("trigger:{trigger}"),
            format!("cron-{}", tick.unix_timestamp())
        )],
        "{context}: one key, feature 004's"
    );
    let (started_by, input): (String, Vec<u8>) = sqlx::query_as(
        "SELECT r.started_by, p.content FROM runs r JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest WHERE r.id = $1",
    )
    .bind(runs[0])
    .fetch_one(&t.w.db.pool)
    .await
    .expect("run");
    assert_eq!(started_by, "system:engine", "{context}");
    let input: serde_json::Value = serde_json::from_slice(&input).expect("json");
    assert_eq!(input["event"]["type"], "cron.fired", "{context}");

    // the schedule's decision for the tick: one row, the run it stands for, STARTED when it started the run
    let actions = t.actions_of(schedule).await;
    if actions.len() != 1 {
        let row: (String, String) = sqlx::query_as(
            "SELECT state, concat_ws(' | ', next_action_at::text, next_actual_at::text, retry_at::text, recheck_at::text) \
             FROM schedules WHERE id = $1",
        )
        .bind(schedule.id.as_uuid())
        .fetch_one(&t.w.db.pool)
        .await
        .expect("row");
        panic!("{context}: the schedule is {row:?}, the tick is {tick}, the actions {actions:?}");
    }
    assert_eq!(
        actions.len(),
        1,
        "{context}: one decision for the tick: {actions:?}"
    );
    let (nominal, outcome, run) = &actions[0];
    assert_eq!(*nominal, tick, "{context}");
    assert_eq!(
        *run,
        Some(runs[0]),
        "{context}: the decision stands for the run"
    );
    let provenance: Option<Uuid> = sqlx::query_scalar("SELECT schedule_id FROM runs WHERE id = $1")
        .bind(runs[0])
        .fetch_one(&t.w.db.pool)
        .await
        .expect("provenance");
    match outcome.as_str() {
        "STARTED" => assert_eq!(provenance, Some(schedule.id.as_uuid()), "{context}"),
        "ALREADY_ACTED" => assert_eq!(provenance, None, "{context}: the legacy pass started it"),
        other => panic!("{context}: the decision is {other}"),
    }
    let schedule_next = t.read(schedule).await.next_action_at.expect("next");
    assert!(schedule_next > tick, "{context}: the schedule moved on");
    let trigger_next: OffsetDateTime =
        sqlx::query_scalar("SELECT next_fire_at FROM workflow_triggers WHERE id = $1")
            .bind(trigger)
            .fetch_one(&t.w.db.pool)
            .await
            .expect("trigger");
    assert!(trigger_next > tick, "{context}: the trigger moved on");
    t.w.assert_consistent(RunId::new(runs[0])).await;
}

/// How many writing statements the first pass makes in the transaction that starts the run, and on which tables.
async fn writes_of(t: &Triggers, wire: &Tripwire, pass: Pass, name: &str) -> usize {
    let (_, _, schedule, _) = due(t, name).await;
    wire.watch().await;
    run(t, pass, &schedule).await;
    wire.disarm().await;
    let writes = wire.writes().await;
    let transactions: BTreeSet<i64> = writes.iter().map(|(xid, _, _)| *xid).collect();
    let biggest = transactions
        .iter()
        .map(|xid| writes.iter().filter(|(x, ..)| x == xid).count())
        .max()
        .expect("a transaction");
    assert!(
        writes.iter().any(|(_, _, table)| table == "runs"),
        "{pass:?} starts a run: {writes:?}"
    );
    biggest
}

async fn matrix(first: Pass, then: [Pass; 2]) {
    let t = triggers().await;
    let wire = Tripwire::install(&t.w.db.pool).await;
    // the first pass of the sequence is cut off after each write of the transaction that starts the run
    let writes = writes_of(
        &t,
        &wire,
        first,
        &format!("probe-{}", format!("{first:?}").to_lowercase()),
    )
    .await;
    assert!(
        writes >= 8,
        "the transaction of a start makes many writes: {writes}"
    );
    for position in 1..=writes {
        let name = format!("cut-{}-{position}", format!("{first:?}").to_lowercase());
        let (workflow, trigger, schedule, tick) = due(&t, &name).await;
        wire.arm(position).await;
        run(&t, first, &schedule).await;
        wire.disarm().await;
        assert!(
            t.runs_of(workflow).await.is_empty(),
            "{first:?} cut off at write {position}: nothing of the run is left behind"
        );
        restart(&t).await;
        for pass in then {
            run(&t, pass, &schedule).await;
        }
        // a pass that finds nothing more to do on a later round changes nothing
        for pass in [Pass::Legacy, Pass::Schedule] {
            run(&t, pass, &schedule).await;
        }
        assert_one_run(
            &t,
            workflow,
            trigger,
            &schedule,
            tick,
            &format!("{first:?} cut off at write {position}, then {then:?}"),
        )
        .await;
    }
}

#[tokio::test]
async fn the_legacy_pass_killed_after_any_write_and_then_both_passes_start_the_tick_once() {
    matrix(Pass::Legacy, [Pass::Schedule, Pass::Legacy]).await;
}

#[tokio::test]
async fn the_legacy_pass_killed_after_any_write_and_the_legacy_pass_again_before_the_schedule_pass()
{
    matrix(Pass::Legacy, [Pass::Legacy, Pass::Schedule]).await;
}

#[tokio::test]
async fn the_schedule_pass_killed_after_any_write_and_then_both_passes_start_the_tick_once() {
    matrix(Pass::Schedule, [Pass::Legacy, Pass::Schedule]).await;
}

#[tokio::test]
async fn the_schedule_pass_killed_after_any_write_and_the_schedule_pass_again_before_the_legacy_pass()
 {
    matrix(Pass::Schedule, [Pass::Schedule, Pass::Legacy]).await;
}

/// The legacy pass dies after its run committed and before it moves the trigger on (they are two transactions): the run is
/// there and the trigger is where it was.
async fn legacy_killed_after_the_run_committed(then: [Pass; 2]) {
    let t = triggers().await;
    let wire = Tripwire::install(&t.w.db.pool).await;
    let (workflow, trigger, schedule, tick) = due(&t, "committed-then-killed").await;
    wire.arm_on("workflow_triggers", 1).await;
    run(&t, Pass::Legacy, &schedule).await;
    wire.disarm().await;
    assert_eq!(
        t.runs_of(workflow).await.len(),
        1,
        "the run committed before the process died"
    );
    let still: Option<OffsetDateTime> =
        sqlx::query_scalar("SELECT next_fire_at FROM workflow_triggers WHERE id = $1")
            .bind(trigger)
            .fetch_one(&t.w.db.pool)
            .await
            .expect("trigger");
    assert_eq!(still, Some(tick), "and the trigger was not moved on");
    restart(&t).await;
    for pass in then {
        run(&t, pass, &schedule).await;
    }
    for pass in [Pass::Legacy, Pass::Schedule] {
        run(&t, pass, &schedule).await;
    }
    assert_one_run(
        &t,
        workflow,
        trigger,
        &schedule,
        tick,
        &format!("the legacy pass killed after its run committed, then {then:?}"),
    )
    .await;
}

#[tokio::test]
async fn the_legacy_pass_killed_after_its_run_committed_then_the_schedule_pass_and_the_legacy_pass()
{
    legacy_killed_after_the_run_committed([Pass::Schedule, Pass::Legacy]).await;
}

#[tokio::test]
async fn the_legacy_pass_killed_after_its_run_committed_then_the_legacy_pass_and_the_schedule_pass()
{
    legacy_killed_after_the_run_committed([Pass::Legacy, Pass::Schedule]).await;
}

// The schedule pass has no such state: its run, its action and the schedule's move are one transaction, so a death after the
// commit is a pass that had finished (the next round finds nothing to do, asserted by every matrix above).

#[tokio::test]
async fn both_passes_racing_on_one_tick_start_one_run_whoever_commits_first() {
    use tokio::sync::Barrier;
    let t = std::sync::Arc::new(triggers().await);
    let (workflow, trigger, schedule, tick) = due(&t, "race").await;
    let barrier = std::sync::Arc::new(Barrier::new(2));
    let legacy = {
        let (t, barrier) = (t.clone(), barrier.clone());
        tokio::spawn(async move {
            barrier.wait().await;
            t.w.dispatcher.tick().await;
        })
    };
    let new = {
        let (t, barrier, schedule) = (t.clone(), barrier.clone(), schedule.clone());
        tokio::spawn(async move {
            barrier.wait().await;
            let _ = t.firing.fire(&t.due(&schedule), None).await;
        })
    };
    legacy.await.expect("legacy");
    new.await.expect("new");
    restart(&t).await;
    // whoever lost the race finds the other's run on its next round
    for pass in [Pass::Schedule, Pass::Legacy, Pass::Schedule] {
        run(&t, pass, &schedule).await;
    }
    assert_one_run(&t, workflow, trigger, &schedule, tick, "a race on one tick").await;
}

/// One round of the cron pass of an engine binary older than the registry release, which never reads the handover: for it
/// the state is always `LEGACY`. The test makes the database say so for the length of the round, with the guard of the
/// table disabled on purpose (the guard exists to stop exactly this, and a test that stands for an old binary has to
/// get around it), and puts it back.
async fn old_binary_round(t: &Triggers) -> u32 {
    let pool = &t.w.db.pool;
    sqlx::query(
        "ALTER TABLE schedule_trigger_handover DISABLE TRIGGER schedule_trigger_handover_guard",
    )
    .execute(pool)
    .await
    .expect("disable the guard");
    sqlx::query("UPDATE schedule_trigger_handover SET state = 'LEGACY', changed_by = NULL, changed_at = NULL, changed_db_role = NULL")
        .execute(pool)
        .await
        .expect("legacy for the old binary");
    let started =
        t.w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default())
            .with_handover_bound(std::time::Duration::ZERO)
            .tick()
            .await
            .runs_started;
    sqlx::query("UPDATE schedule_trigger_handover SET state = 'HANDED_OVER', changed_by = 'ana', changed_at = now(), changed_db_role = 'owner'")
        .execute(pool)
        .await
        .expect("handed over again");
    sqlx::query("ALTER TABLE schedule_trigger_handover ENABLE ALWAYS TRIGGER schedule_trigger_handover_guard")
        .execute(pool)
        .await
        .expect("enable the guard");
    started
}

#[tokio::test]
async fn an_old_engine_binary_keeps_firing_after_the_handover_ignoring_a_pause_but_never_starting_a_tick_twice()
 {
    // G1011: below the registry release nothing stops an old binary from running its cron pass (it does not read the
    // handover): it ignores a pause, which is the hazard the runbook states, and it cannot start a tick twice, because its
    // key is the one the schedule pass uses. A registry-aware engine without schedules-v1 refuses to run instead.
    let t = triggers().await;
    let (workflow, trigger, schedule, tick) = due(&t, "old-binary").await;
    t.hand_over().await;
    t.service
        .pause(
            &t.w.admin,
            schedule.id,
            sagat_domain::schedules::Note::new("hold the trigger").expect("note"),
            &sagat_application::identity::RequestContext::default(),
        )
        .await
        .expect("pause");
    assert_eq!(
        t.firing.fire(&t.due(&schedule), None).await.expect("fire"),
        sagat_application::schedule_firing::Fired::Idle,
        "the new engines honour the pause"
    );
    assert_eq!(
        old_binary_round(&t).await,
        1,
        "the old binary starts the tick all the same"
    );
    assert_eq!(old_binary_round(&t).await, 0, "and never a second time");
    assert_eq!(t.runs_of(workflow).await.len(), 1);
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT idempotency_key FROM command_dedupe WHERE scope = $1")
            .bind(format!("trigger:{trigger}"))
            .fetch_all(&t.w.db.pool)
            .await
            .expect("keys");
    assert_eq!(keys, [format!("cron-{}", tick.unix_timestamp())]);
}

// --- the reconciliation, cut off at each of its writes ----------------------------------------------------------------

#[tokio::test]
async fn the_reconciliation_cut_off_at_any_write_leaves_nothing_half_done_and_the_next_round_finishes()
 {
    use sagat_application::schedule_triggers::Drift;
    let t = triggers().await;
    let wire = Tripwire::install(&t.w.db.pool).await;
    t.publish(cron_workflow("base", "0 * * * *", "UTC")).await;
    t.reconcile().await;

    async fn drifts(t: &Triggers) -> Vec<Drift> {
        t.drift().await
    }

    // what an older control plane does, three ways: it adds a trigger, drops one, and the expression of one drifts
    for (case, position_limit) in [("create", 8usize), ("remove", 8), ("respec", 8)] {
        let trigger = Uuid::now_v7();
        match case {
            "create" => {
                sqlx::query(
                    "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
                     SELECT $1, tenant_id, namespace_id, workflow_id, version_id, 'cron', '5 * * * *', 'UTC' FROM workflow_triggers LIMIT 1",
                )
                .bind(trigger)
                .execute(&t.w.db.pool)
                .await
                .expect("added");
            }
            "remove" => {
                sqlx::query(
                    "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
                     SELECT $1, tenant_id, namespace_id, workflow_id, version_id, 'cron', '6 * * * *', 'UTC' FROM workflow_triggers LIMIT 1",
                )
                .bind(trigger)
                .execute(&t.w.db.pool)
                .await
                .expect("added");
                t.reconcile().await;
                sqlx::query("DELETE FROM workflow_triggers WHERE id = $1")
                    .bind(trigger)
                    .execute(&t.w.db.pool)
                    .await
                    .expect("dropped");
            }
            _ => {
                sqlx::query(
                    "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
                     SELECT $1, tenant_id, namespace_id, workflow_id, version_id, 'cron', '7 * * * *', 'UTC' FROM workflow_triggers LIMIT 1",
                )
                .bind(trigger)
                .execute(&t.w.db.pool)
                .await
                .expect("added");
                t.reconcile().await;
                sqlx::query("UPDATE workflow_triggers SET cron = '8 * * * *' WHERE id = $1")
                    .bind(trigger)
                    .execute(&t.w.db.pool)
                    .await
                    .expect("drifted");
            }
        }
        let expected = drifts(&t).await;
        assert_eq!(expected.len(), 1, "{case}: {expected:?}");
        // how many writes does the change make? watch one
        wire.watch().await;
        let schedules_before: i64 = sqlx::query_scalar("SELECT count(*) FROM schedules")
            .fetch_one(&t.w.db.pool)
            .await
            .expect("count");
        let snapshot = |label: &'static str| {
            let pool = t.w.db.pool.clone();
            async move {
                let rows: Vec<(String, i32, i64)> = sqlx::query_as(
                    "SELECT state, head_revision, state_version FROM schedules ORDER BY id",
                )
                .fetch_all(&pool)
                .await
                .unwrap_or_else(|e| panic!("{label}: {e}"));
                rows
            }
        };
        let before = snapshot("before").await;
        t.reconcile().await;
        wire.disarm().await;
        let writes = wire.writes().await;
        let transactions: BTreeSet<i64> = writes.iter().map(|(x, ..)| *x).collect();
        let writes_of_the_change = transactions
            .iter()
            .map(|x| writes.iter().filter(|(y, ..)| y == x).count())
            .max()
            .expect("a transaction");
        assert!(
            writes_of_the_change >= 2 && writes_of_the_change <= position_limit + 4,
            "{case}: {writes_of_the_change}"
        );
        // undo what the probe did so that the same drift can be cut off: delete and add again
        let _ = (schedules_before, before);

        // now cut it off at each write, on a drift of its own, until the transaction has no write left to cut
        let mut cut = 0;
        for position in 1..=40 {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO workflow_triggers (id, tenant_id, namespace_id, workflow_id, version_id, kind, cron, timezone) \
                 SELECT $1, tenant_id, namespace_id, workflow_id, version_id, 'cron', '9 * * * *', 'UTC' FROM workflow_triggers LIMIT 1",
            )
            .bind(id)
            .execute(&t.w.db.pool)
            .await
            .expect("a trigger without a schedule");
            if case != "create" {
                t.reconcile().await;
                if case == "remove" {
                    sqlx::query("DELETE FROM workflow_triggers WHERE id = $1")
                        .bind(id)
                        .execute(&t.w.db.pool)
                        .await
                        .expect("drop");
                } else {
                    sqlx::query("UPDATE workflow_triggers SET cron = '10 * * * *' WHERE id = $1")
                        .bind(id)
                        .execute(&t.w.db.pool)
                        .await
                        .expect("drift");
                }
            }
            let state_before = snapshot("cut").await;
            let audits_before: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM audit_records WHERE action LIKE 'schedule.%'",
            )
            .fetch_one(&t.w.db.pool)
            .await
            .expect("audits");
            wire.arm(position).await;
            let handover = sagat_application::schedule_triggers::HandoverState::Legacy;
            for one in &t.drift().await {
                // a cut-off change fails or is refused; it never half-commits
                let _ = t.reconciler.reconcile(one, &handover, None).await;
            }
            wire.disarm().await;
            let during = snapshot("during").await;
            let audits_during: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM audit_records WHERE action LIKE 'schedule.%'",
            )
            .fetch_one(&t.w.db.pool)
            .await
            .expect("audits");
            if during != state_before || audits_during != audits_before {
                // armed past its last write: the change was not cut off and is whole
                assert!(
                    cut >= 3,
                    "{case}: the change has more writes than {cut}, and was not cut off before write {position}"
                );
                assert!(t.drift().await.is_empty(), "{case}: whole once it commits");
                break;
            }
            cut += 1;
            // the next round finishes it, and finishes it once
            assert_eq!(t.reconcile().await, 1, "{case}: after write {position}");
            assert!(
                t.drift().await.is_empty(),
                "{case}: after write {position}, nothing differs"
            );
        }
        assert!(
            cut >= 3,
            "{case}: the matrix cut the transaction at {cut} writes"
        );
        let _ = writes_of_the_change;
    }
}

// --- the handover, cut off at each of its writes ------------------------------------------------------------------------

#[tokio::test]
async fn the_handover_cut_off_at_any_write_records_nothing_and_the_next_run_records_it_once() {
    use crate::support::instances::registry;
    use sagat_control_plane::config::DatabaseConfig;
    use sagat_control_plane::handover::{
        HandoverOutcome, HandoverRequest, schedule_trigger_handover,
    };
    let r = registry().await;
    r.serving(
        sagat_application::instances::InstanceKind::ExecutionEngine,
        &[sagat_application::instances::Capability::SchedulesV1],
    )
    .await;
    let wire = Tripwire::install(&r.db.pool).await;
    let database = || DatabaseConfig {
        url: r.db.url.clone(),
        max_connections: 2,
        statement_timeout: std::time::Duration::from_secs(5),
    };
    let request = HandoverRequest {
        operator: Some("ana".to_owned()),
        expect_live: Some(1),
        dry_run: false,
    };
    let recorded = || async {
        let state: String = sqlx::query_scalar("SELECT state FROM schedule_trigger_handover")
            .fetch_one(&r.db.pool)
            .await
            .expect("state");
        let requirements: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_requirements")
            .fetch_one(&r.db.pool)
            .await
            .expect("requirements");
        let history: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_requirement_history")
            .fetch_one(&r.db.pool)
            .await
            .expect("history");
        let epoch: i64 = sqlx::query_scalar("SELECT epoch FROM platform_requirement_epoch")
            .fetch_one(&r.db.pool)
            .await
            .expect("epoch");
        (state, requirements, history, epoch)
    };

    // how many writes the activation makes: watch it on a schema of its own
    let probe = crate::support::instances::registry().await;
    probe
        .serving(
            sagat_application::instances::InstanceKind::ExecutionEngine,
            &[sagat_application::instances::Capability::SchedulesV1],
        )
        .await;
    let probe_wire = Tripwire::install(&probe.db.pool).await;
    probe_wire.watch().await;
    schedule_trigger_handover(
        &DatabaseConfig {
            url: probe.db.url.clone(),
            max_connections: 2,
            statement_timeout: std::time::Duration::from_secs(5),
        },
        &request,
    )
    .await
    .expect("the probe");
    probe_wire.disarm().await;
    let writes = probe_wire.writes().await;
    let tables: BTreeSet<&str> = writes.iter().map(|(_, _, t)| t.as_str()).collect();
    for table in [
        "platform_requirements",
        "platform_requirement_epoch",
        "platform_requirement_history",
        "schedule_trigger_handover",
    ] {
        assert!(
            tables.contains(table),
            "the activation writes {table}: {writes:?}"
        );
    }
    let transactions: BTreeSet<i64> = writes.iter().map(|(x, ..)| *x).collect();
    assert_eq!(
        transactions.len(),
        1,
        "the handover is one transaction: {writes:?}"
    );

    for position in 1..=writes.len() {
        wire.arm(position).await;
        let killed = schedule_trigger_handover(&database(), &request).await;
        wire.disarm().await;
        assert!(killed.is_err(), "cut off at write {position}: {killed:?}");
        assert_eq!(
            recorded().await,
            ("LEGACY".to_owned(), 0, 0, 0),
            "cut off at write {position}: neither the requirement, its history, the epoch nor the state"
        );
    }
    let done = schedule_trigger_handover(&database(), &request)
        .await
        .expect("the next run");
    assert!(
        matches!(done, HandoverOutcome::HandedOver { live: 1, .. }),
        "{done:?}"
    );
    let (state, requirements, history, epoch) = recorded().await;
    assert_eq!(
        (state.as_str(), requirements, history, epoch),
        ("HANDED_OVER", 1, 1, 1),
        "recorded once"
    );
}

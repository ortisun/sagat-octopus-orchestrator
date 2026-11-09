//! The firing of a trigger-owned schedule (`T018-060`, `T018-061`, `SC-018-005`, `FR-018-017`): the run of a tick is THE run
//! the legacy cron pass starts (feature 004's scope `trigger:{id}`, key `cron-{epoch}`, the `cron.fired` document, started
//! by `system:engine`, the version in force), so that when both passes reach a tick one run starts and the other records
//! `ALREADY_ACTED`; after the handover the legacy pass starts nothing. Real PostgreSQL, the real legacy pass of the world.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::dispatch::ClaimedEvent;
use sagat_application::execution::Engine;
use sagat_application::schedule_firing::Fired;
use sagat_application::schedules::ScheduleRecord;
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::RunId;
use sagat_infra_ir::JsonEvaluator;
use serde_json::Value;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::trigger_schedules::{Triggers, cron_workflow, triggers};

/// A published cron workflow whose tick is due for both passes: the trigger planned by the legacy pass would fire it, and
/// the schedule too.
pub(crate) async fn due(
    t: &Triggers,
    name: &str,
) -> (WorkflowId, Uuid, ScheduleRecord, OffsetDateTime) {
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

async fn input_of(t: &Triggers, run: Uuid) -> Value {
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM runs r JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest WHERE r.id = $1",
    )
    .bind(run)
    .fetch_one(&t.w.db.pool)
    .await
    .expect("input");
    serde_json::from_slice(&bytes).expect("json")
}

/// Everything feature 004 says of the run of a tick, whichever pass started it.
async fn assert_the_run_of_the_tick(t: &Triggers, trigger: Uuid, tick: OffsetDateTime, run: Uuid) {
    let epoch = tick.unix_timestamp();
    let (started_by, version): (String, Uuid) =
        sqlx::query_as("SELECT started_by, workflow_version_id FROM runs WHERE id = $1")
            .bind(run)
            .fetch_one(&t.w.db.pool)
            .await
            .expect("the run");
    assert_eq!(
        started_by, "system:engine",
        "started as the legacy pass starts it"
    );
    let in_force: Uuid =
        sqlx::query_scalar("SELECT version_id FROM workflow_triggers WHERE id = $1")
            .bind(trigger)
            .fetch_one(&t.w.db.pool)
            .await
            .expect("trigger");
    assert_eq!(version, in_force, "the version the trigger names");

    let (scope, key): (String, String) =
        sqlx::query_as("SELECT scope, idempotency_key FROM command_dedupe WHERE resource_id = $1")
            .bind(run.to_string())
            .fetch_one(&t.w.db.pool)
            .await
            .expect("the key of the run");
    assert_eq!(
        (scope, key),
        (format!("trigger:{trigger}"), format!("cron-{epoch}")),
        "feature 004's scope and key"
    );

    let input = input_of(t, run).await;
    let at = ClaimedEvent::rfc3339(tick);
    assert_eq!(input["event"]["source"], "cron");
    assert_eq!(input["event"]["type"], "cron.fired");
    assert_eq!(input["event"]["id"], format!("cron:{trigger}:{epoch}"));
    assert_eq!(input["event"]["occurredAt"], at);
    assert_eq!(input["data"]["scheduledAt"], at);
    // the instant the pass acted on, after the time it was meant for (the legacy pass's database clock, or the clock of the pass)
    let fired = OffsetDateTime::parse(
        input["data"]["firedAt"].as_str().expect("firedAt"),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("an instant");
    assert!(fired > tick, "firedAt {fired} is after scheduledAt {tick}");
    assert_eq!(input["event"]["receivedAt"], input["data"]["firedAt"]);

    let (actor, reason): (String, Option<String>) = sqlx::query_as(
        "SELECT actor, reason FROM audit_records WHERE action = 'run.started' AND resource_id = $1",
    )
    .bind(run.to_string())
    .fetch_one(&t.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(actor, "system:engine");
    assert_eq!(
        reason.as_deref(),
        Some(format!("cron trigger {trigger} scheduled for {at}").as_str())
    );
    let journal: String = sqlx::query_scalar(
        "SELECT actor FROM run_events WHERE run_id = $1 AND event_type = 'run.started'",
    )
    .bind(run)
    .fetch_one(&t.w.db.pool)
    .await
    .expect("journal");
    assert_eq!(
        journal, "system:event-dispatcher",
        "the dispatcher's journal, as the legacy pass writes it"
    );
    t.w.assert_consistent(RunId::new(run)).await;
}

#[tokio::test]
async fn the_legacy_pass_first_starts_the_run_and_the_schedule_pass_records_that_it_was_already_acted()
 {
    let t = triggers().await;
    let (workflow, trigger, schedule, tick) = due(&t, "legacy-first").await;

    let round = t.w.dispatcher.tick().await;
    assert_eq!(round.runs_started, 1, "the legacy pass fires the tick");
    let runs = t.runs_of(workflow).await;
    assert_eq!(runs.len(), 1);
    assert_the_run_of_the_tick(&t, trigger, tick, runs[0]).await;
    let (provenance,): (Option<Uuid>,) =
        sqlx::query_as("SELECT schedule_id FROM runs WHERE id = $1")
            .bind(runs[0])
            .fetch_one(&t.w.db.pool)
            .await
            .expect("provenance");
    assert_eq!(
        provenance, None,
        "the legacy pass writes no schedule provenance"
    );

    // the schedule pass reaches the same tick: no second run, and the decision says whose it was
    assert_eq!(t.fire(&schedule).await, Fired::Acted);
    assert_eq!(t.runs_of(workflow).await, runs, "one run for the tick");
    assert_eq!(
        t.actions_of(&schedule).await,
        vec![(tick, "ALREADY_ACTED".to_owned(), Some(runs[0]))]
    );
    let after = t.read(&schedule).await;
    assert!(
        after.next_action_at.expect("next") > tick,
        "the schedule moved on"
    );
    assert!(
        after.tracked_runs.is_empty(),
        "a tick of a trigger is never tracked"
    );
    let counted: serde_json::Value =
        sqlx::query_scalar("SELECT counters FROM schedules WHERE id = $1")
            .bind(schedule.id.as_uuid())
            .fetch_one(&t.w.db.pool)
            .await
            .expect("counters");
    assert_eq!(counted["ALREADY_ACTED"], 1);
    assert_eq!(t.fire(&schedule).await, Fired::Idle, "nothing more is due");
    assert_eq!(t.w.dispatcher.tick().await.runs_started, 0);
    assert_eq!(t.runs_of(workflow).await, runs);
}

#[tokio::test]
async fn the_schedule_pass_first_starts_the_run_and_the_legacy_pass_starts_nothing_more() {
    let t = triggers().await;
    let (workflow, trigger, schedule, tick) = due(&t, "schedule-first").await;

    assert_eq!(t.fire(&schedule).await, Fired::Acted);
    let runs = t.runs_of(workflow).await;
    assert_eq!(runs.len(), 1);
    assert_the_run_of_the_tick(&t, trigger, tick, runs[0]).await;
    assert_eq!(
        t.actions_of(&schedule).await,
        vec![(tick, "STARTED".to_owned(), Some(runs[0]))]
    );
    let (kind, at): (Option<String>, Option<OffsetDateTime>) =
        sqlx::query_as("SELECT schedule_action_kind, schedule_nominal_at FROM runs WHERE id = $1")
            .bind(runs[0])
            .fetch_one(&t.w.db.pool)
            .await
            .expect("provenance");
    assert_eq!(
        (kind.as_deref(), at),
        (Some("SCHEDULED"), Some(tick)),
        "the schedule that started it says so"
    );

    // the legacy pass reaches the same tick: the key is known, nothing starts, and its trigger moves on
    let round = t.w.dispatcher.tick().await;
    assert_eq!(round.runs_started, 0);
    assert_eq!(t.runs_of(workflow).await, runs, "one run for the tick");
    let next: OffsetDateTime =
        sqlx::query_scalar("SELECT next_fire_at FROM workflow_triggers WHERE id = $1")
            .bind(trigger)
            .fetch_one(&t.w.db.pool)
            .await
            .expect("trigger");
    assert!(next > tick, "its trigger moved on");
    assert_eq!(t.fire(&schedule).await, Fired::Idle);
}

#[tokio::test]
async fn the_input_document_of_a_tick_is_what_the_legacy_pass_builds() {
    use sagat_application::dispatch::EventView;
    use sagat_application::execution::ExpressionEvaluator as _;
    let trigger = Uuid::from_u128(0xabcd);
    for (nominal, fired) in [
        (1_793_188_800_i64, 1_793_188_803_i64),
        (1_799_999_999, 1_800_000_000),
        (0, 1),
    ] {
        let (nominal, fired) = (
            OffsetDateTime::from_unix_timestamp(nominal).expect("instant"),
            OffsetDateTime::from_unix_timestamp(fired).expect("instant"),
        );
        // exactly what `cron_pass` hands the evaluator
        let at = ClaimedEvent::rfc3339(nominal);
        let fired_text = ClaimedEvent::rfc3339(fired);
        let data = format!(r#"{{"scheduledAt":"{at}","firedAt":"{fired_text}"}}"#);
        let id = format!("cron:{trigger}:{}", nominal.unix_timestamp());
        let legacy = Engine::new(JsonEvaluator)
            .evaluator()
            .event_document(&EventView {
                id: &id,
                source: "cron",
                event_type: "cron.fired",
                subject: None,
                correlation_key: None,
                occurred_at: Some(&at),
                received_at: &fired_text,
                data: data.as_bytes(),
            });
        assert_eq!(
            sagat_application::schedule_triggers::cron_document(trigger, nominal, fired),
            legacy
        );
    }
}

#[tokio::test]
async fn the_ticks_of_a_trigger_schedule_are_the_ticks_of_the_cron_trigger_in_every_zone() {
    use sagat_application::dispatch::CronSchedule as _;
    use sagat_application::schedules::ScheduleCalendar as _;
    use sagat_domain::triggers::CronSpec;
    let legacy = sagat_infra_ir::cron::CronCalendar;
    let schedules = sagat_infra_ir::schedule_calendar::SpecCalendar;
    // across the daylight-saving changes of 2026 in three hemispheres, a gap, a repeated hour and a leap day
    let from = OffsetDateTime::from_unix_timestamp(1_772_323_200).expect("2026-03-01");
    for (expression, zone) in [
        ("*/7 * * * *", "UTC"),
        ("30 2 * * *", "America/New_York"),
        ("30 1 * * *", "America/New_York"),
        ("0 2 * * *", "Europe/Lisbon"),
        ("15 2 * * *", "Australia/Sydney"),
        ("0 3 * * 1-5", "America/Sao_Paulo"),
        ("0 0 29 2 *", "UTC"),
        ("45 23 31 * *", "Asia/Kolkata"),
        // a half-hour offset that also shifts by half an hour, a quarter-hour offset, and the other southern change
        ("45 1 * * *", "Australia/Lord_Howe"),
        ("10 0 * * *", "Asia/Kathmandu"),
        ("30 2 * * *", "Pacific/Auckland"),
    ] {
        let cron = CronSpec::parse(expression, zone).expect("cron");
        let spec = sagat_application::schedule_triggers::trigger_spec(&cron).expect("spec");
        let mut after = from;
        let mut seen = 0;
        for _ in 0..400 {
            let Ok(expected) = legacy.next_after(&cron, after) else {
                break;
            };
            let got = schedules
                .next_after(&spec, after)
                .expect("calendar")
                .expect("an occurrence");
            assert_eq!(got, expected, "{expression} in {zone} after {after}");
            after = expected;
            seen += 1;
            if after > from + Duration::days(900) {
                break;
            }
        }
        assert!(seen > 0, "{expression} in {zone} fires");
    }
}

#[tokio::test]
async fn a_tick_starts_the_version_its_trigger_names_as_the_legacy_pass_does() {
    let t = triggers().await;
    let (workflow, first) = t
        .publish(cron_workflow("versioned", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("versioned").await;
    // a second version keeps the trigger and moves it to the version in force; the trigger is then put back on the first,
    // as it is when an activation of an earlier version registers it (feature 011)
    t.w.publish_new_version(
        workflow,
        crate::support::trigger_schedules::second_version("versioned", "0 * * * *", "UTC"),
    )
    .await;
    sqlx::query("UPDATE workflow_triggers SET version_id = $2 WHERE id = $1")
        .bind(trigger)
        .bind(first.as_uuid())
        .execute(&t.w.db.pool)
        .await
        .expect("the trigger names the first version");
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let tick = Triggers::last_hour();
    t.due_at(trigger, &schedule, tick).await;
    t.clock.set(OffsetDateTime::now_utc());
    assert_eq!(t.fire(&schedule).await, Fired::Acted);
    let runs = t.runs_of(workflow).await;
    assert_eq!(runs.len(), 1);
    let started: Uuid = sqlx::query_scalar("SELECT workflow_version_id FROM runs WHERE id = $1")
        .bind(runs[0])
        .fetch_one(&t.w.db.pool)
        .await
        .expect("version");
    assert_eq!(
        started,
        first.as_uuid(),
        "the version the trigger names, not the active one"
    );
}

#[tokio::test]
async fn a_tick_never_waits_for_an_open_run_and_is_not_tracked() {
    let t = triggers().await;
    let (workflow, _) = t
        .publish(cron_workflow("overlap", "0 * * * *", "UTC"))
        .await;
    let (trigger, ..) = t.trigger_of("overlap").await;
    t.reconcile().await;
    let schedule = t.schedule_of(trigger).await.expect("schedule");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let first =
        OffsetDateTime::from_unix_timestamp(now - now % 3_600 - 2 * 3_600).expect("instant");
    t.due_at(trigger, &schedule, first).await;

    // the first tick starts a run that stays open; the second finds it open and starts one all the same
    t.clock.set(first + Duration::seconds(5));
    assert_eq!(t.fire(&schedule).await, Fired::Acted);
    t.clock
        .set(first + Duration::hours(1) + Duration::seconds(5));
    assert_eq!(t.fire(&schedule).await, Fired::Acted);
    assert_eq!(t.runs_of(workflow).await.len(), 2, "ALLOW_ALL");
    let after = t.read(&schedule).await;
    assert!(
        after.tracked_runs.is_empty(),
        "no run of a trigger-owned tick is tracked"
    );
    let outcomes: Vec<(String, i64)> = sqlx::query_as(
        "SELECT outcome, count(*) FROM schedule_actions WHERE schedule_id = $1 GROUP BY outcome",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&t.w.db.pool)
    .await
    .expect("outcomes");
    assert_eq!(outcomes, vec![("STARTED".to_owned(), 2)]);
}

#[tokio::test]
async fn a_schedule_whose_trigger_was_dropped_by_an_older_control_plane_starts_nothing_and_goes_away()
 {
    let t = triggers().await;
    let (workflow, trigger, schedule, _) = due(&t, "dropped-by-old").await;
    // an older control plane republishes without the trigger: it deletes the trigger and knows nothing of the schedule
    sqlx::query("DELETE FROM workflow_triggers WHERE id = $1")
        .bind(trigger)
        .execute(&t.w.db.pool)
        .await
        .expect("the older control plane drops the trigger");

    assert_eq!(t.fire(&schedule).await, Fired::Acted, "the step removes it");
    assert!(
        t.runs_of(workflow).await.is_empty(),
        "nothing started for a trigger nobody asked for"
    );
    assert_eq!(
        t.read(&schedule).await.state,
        sagat_application::schedules::ScheduleState::Deleted
    );
    assert_eq!(t.fire(&schedule).await, Fired::Idle);
}

#[tokio::test]
async fn after_the_handover_the_legacy_pass_starts_nothing_even_when_it_still_runs_and_the_schedule_pass_does()
 {
    let t = triggers().await;
    let (workflow, trigger, schedule, tick) = due(&t, "handed-over").await;
    // a dispatcher that reads the state at every round, as an engine does every two seconds at most
    let legacy =
        t.w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default())
            .with_handover_bound(std::time::Duration::ZERO);

    t.hand_over().await;
    let round = legacy.tick().await;
    assert_eq!(round.runs_started, 0, "the legacy pass has stopped");
    assert!(t.runs_of(workflow).await.is_empty());
    let still: Option<OffsetDateTime> =
        sqlx::query_scalar("SELECT next_fire_at FROM workflow_triggers WHERE id = $1")
            .bind(trigger)
            .fetch_one(&t.w.db.pool)
            .await
            .expect("trigger");
    assert_eq!(still, Some(tick), "its trigger was not even looked at");

    // the schedule pass fires the tick, as the engine would
    assert_eq!(t.fire(&schedule).await, Fired::Acted);
    let runs = t.runs_of(workflow).await;
    assert_eq!(runs.len(), 1);
    assert_the_run_of_the_tick(&t, trigger, tick, runs[0]).await;
}

#[tokio::test]
async fn a_pause_after_the_handover_stops_the_trigger_and_nothing_else_fires_it() {
    use sagat_domain::schedules::Note;
    let t = triggers().await;
    let (workflow, _trigger, schedule, _) = due(&t, "paused-trigger").await;
    t.hand_over().await;
    let legacy =
        t.w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default())
            .with_handover_bound(std::time::Duration::ZERO);
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
        t.fire(&schedule).await,
        Fired::Idle,
        "a paused schedule fires nothing"
    );
    assert_eq!(
        legacy.tick().await.runs_started,
        0,
        "and the legacy pass is stopped"
    );
    assert!(t.runs_of(workflow).await.is_empty());
}

#[tokio::test]
async fn a_handover_state_that_cannot_be_read_leaves_the_legacy_pass_running() {
    // a tick fired twice starts once, and a tick not fired is lost: when the state cannot be read the pass goes on
    let t = triggers().await;
    let (workflow, _trigger, _schedule, _tick) = due(&t, "unreadable").await;
    let legacy =
        t.w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default())
            .with_handover_bound(std::time::Duration::ZERO);
    sqlx::query("ALTER TABLE schedule_trigger_handover RENAME TO schedule_trigger_handover_away")
        .execute(&t.w.db.pool)
        .await
        .expect("the state cannot be read");
    let started = legacy.tick().await.runs_started;
    sqlx::query("ALTER TABLE schedule_trigger_handover_away RENAME TO schedule_trigger_handover")
        .execute(&t.w.db.pool)
        .await
        .expect("put it back");
    assert_eq!(started, 1, "the legacy pass fired the tick");
    assert_eq!(t.runs_of(workflow).await.len(), 1);
}

#[tokio::test]
async fn before_the_handover_the_legacy_pass_goes_on_and_the_state_is_read_from_the_database_at_every_round()
 {
    let t = triggers().await;
    let (workflow, ..) = due(&t, "legacy-goes-on").await;
    let legacy =
        t.w.dispatcher_with(sagat_scheduler::EventDispatchSettings::default())
            .with_handover_bound(std::time::Duration::ZERO);
    assert_eq!(
        legacy.tick().await.runs_started,
        1,
        "LEGACY: the pass fires"
    );
    assert_eq!(t.runs_of(workflow).await.len(), 1);

    // the state changes between two rounds of the same dispatcher, and the second round reads it from the row: a pass that
    // read the state once, or never, would start the tick of the second workflow
    let (second, ..) = due(&t, "legacy-then-handed-over").await;
    t.hand_over().await;
    assert_eq!(
        legacy.tick().await.runs_started,
        0,
        "HANDED_OVER: the same dispatcher starts nothing at its next round"
    );
    assert!(t.runs_of(second).await.is_empty());
}

//! Design 043 against a real PostgreSQL, rows 6, 7, 8 and 14 of the creators (a schedule's action, backfill, trigger-now
//! and write) and the outcome `INPUT_INVALID` (T043-10, T043-11, FR-043-006).
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::InputValidationMode;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::{BackfillRequest, BackfillState, DueBackfill, ScheduleState};
use serde_json::{Value, json};
use time::Duration;

use crate::firing::{Fixture, fixture, hourly, outcomes_of, t0};
use crate::support::engine::{activity, document};
use crate::support::run_input::{order_schema, publish_in, unusable_schema, with_schema};

fn flow(name: &str) -> Value {
    document(name, "a", vec![activity("a")])
}

fn target(workflow: sagat_domain::definition::WorkflowId) -> Value {
    json!({"workflowId": workflow.to_string()})
}

async fn reasons(f: &Fixture, schedule: sagat_domain::schedules::ScheduleId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT coalesce(reason, '') FROM schedule_actions WHERE schedule_id = $1 AND outcome = 'INPUT_INVALID' ORDER BY nominal_at",
    )
    .bind(schedule.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("reasons")
}

async fn auto_paused(f: &Fixture, schedule: sagat_domain::schedules::ScheduleId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'schedule.auto_paused' AND resource_id = $1",
    )
    .bind(schedule.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audits")
}

/// Row 14: a write whose input the target version refuses is refused `SCHEDULE_INPUT_INVALID` under `/action/input`, and
/// one that fits is stored.
#[tokio::test]
async fn row_14_a_write_whose_input_the_target_refuses_is_refused_under_action_input() {
    let f = fixture().await;
    let (workflow, _) =
        f.w.publish(with_schema(flow("row-14"), order_schema()))
            .await;
    let mut write = f.write_of("row-14", &hourly(), &json!({}), Some(target(workflow)));
    write.input = json!({"orderId": 5});
    let refused = f.try_create(write.clone()).await.expect_err("refused");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "SCHEDULE_INPUT_INVALID",
            field: Some("/action/input/orderId".to_owned()),
            rule: Some("type"),
        }
    );
    write.input = json!({"orderId": "o-1"});
    f.try_create(write)
        .await
        .expect("a matching input is stored");
}

/// Row 14: `nominalTimeField` must be a declared top-level string property of a version with an `inputSchema`, and a
/// schema outside the subset refuses the write `RUN_INPUT_SCHEMA_UNUSABLE`.
#[tokio::test]
async fn row_14_the_nominal_time_field_must_be_declared_and_an_unusable_schema_refuses() {
    let f = fixture().await;
    let (workflow, _) =
        f.w.publish(with_schema(flow("row-14-nominal"), order_schema()))
            .await;
    let mut write = f.write_of(
        "row-14-nominal",
        &hourly(),
        &json!({}),
        Some(json!({"workflowId": workflow.to_string(), "nominalTimeField": "at"})),
    );
    write.input = json!({"orderId": "o-1"});
    let refused = f.try_create(write).await.expect_err("refused");
    assert_eq!(
        refused,
        AppError::InvalidSchedule {
            reason: "NOMINAL_TIME_FIELD_INVALID",
            field: Some("/action/nominalTimeField".to_owned()),
        }
    );

    let (unusable, _) = publish_in(
        &f.w,
        InputValidationMode::Report,
        &with_schema(flow("row-14-unusable"), unusable_schema()),
    )
    .await;
    let write = f.write_of(
        "row-14-unusable",
        &hourly(),
        &json!({}),
        Some(target(unusable)),
    );
    let refused = f.try_create(write).await.expect_err("refused");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "RUN_INPUT_SCHEMA_UNUSABLE",
            field: None,
            rule: None,
        }
    );
}

/// Row 6, CUJ 2: a new version that refuses the schedule's input makes the next time `INPUT_INVALID`, pauses the schedule
/// with its audit in the same decision, starts nothing, and the next tick does nothing more.
#[tokio::test]
async fn row_6_an_action_the_new_version_refuses_is_input_invalid_and_pauses_once() {
    let f = fixture().await;
    let (workflow, _) = f.w.publish(flow("row-6")).await;
    let schedule = f
        .create(
            "row-6",
            &hourly(),
            &json!({"pauseOnFailure": true}),
            Some(target(workflow)),
        )
        .await;
    f.w.publish_new_version(workflow, with_schema(flow("row-6"), order_schema()))
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("INPUT_INVALID", 1)])
    );
    assert_eq!(
        reasons(&f, schedule.id).await,
        vec!["SCHEDULE_INPUT_INVALID"]
    );
    assert!(f.runs(&schedule).await.is_empty());
    assert_eq!(f.read(&schedule).await.state, ScheduleState::Paused);
    assert_eq!(auto_paused(&f, schedule.id).await, 1);
    // the refused time is acted on, not owed: resumed, the schedule goes on to the next time and decides that one only
    let paused = f.read(&schedule).await;
    assert_eq!(paused.retry_at, None, "nothing is owed");
    f.service
        .resume(
            &f.w.admin,
            schedule.id,
            sagat_domain::schedules::Note::new("fixed the input").expect("note"),
            &RequestContext::default(),
        )
        .await
        .expect("resumed");
    assert_eq!(
        f.fire(&schedule).await,
        Fired::Idle,
        "the refused time is not decided again"
    );
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("INPUT_INVALID", 1)])
    );
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("INPUT_INVALID", 2)]),
        "the next time is decided, once"
    );
}

/// Row 6 without `pause_on_failure`: the time is acted on and not owed, and the schedule goes on to its next time; an
/// unusable schema records `RUN_INPUT_SCHEMA_UNUSABLE`.
#[tokio::test]
async fn row_6_without_pause_the_refused_time_is_not_owed_and_an_unusable_schema_says_so() {
    let f = fixture().await;
    let (workflow, _) = f.w.publish(flow("row-6-on")).await;
    let schedule = f
        .create("row-6-on", &hourly(), &json!({}), Some(target(workflow)))
        .await;
    let defs = sagat_application::definitions::DefinitionService::new(
        f.w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_infra_ir::JsonIrCodec,
    )
    .with_input_validation(InputValidationMode::Report);
    let record = defs
        .get_workflow(&f.w.admin, workflow, &crate::support::engine::ctx())
        .await
        .expect("workflow");
    let draft = defs
        .update_draft(
            &f.w.admin,
            workflow,
            record.workflow.draft_revision(),
            &serde_json::to_vec(&with_schema(flow("row-6-on"), unusable_schema())).expect("json"),
            &crate::support::engine::ctx(),
        )
        .await
        .expect("draft");
    defs.publish(
        &f.w.admin,
        workflow,
        draft.workflow.draft_revision(),
        None,
        "publish-unusable-row-6",
        &crate::support::engine::ctx(),
    )
    .await
    .expect("published in report mode");
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("INPUT_INVALID", 2)])
    );
    assert_eq!(
        reasons(&f, schedule.id).await,
        vec!["RUN_INPUT_SCHEMA_UNUSABLE", "RUN_INPUT_SCHEMA_UNUSABLE"]
    );
    assert_eq!(f.read(&schedule).await.state, ScheduleState::Active);
    assert_eq!(f.read(&schedule).await.retry_at, None, "nothing is owed");
}

/// Row 7: a backfill step whose input is refused records `INPUT_INVALID` and ends the backfill `CANCELLED` in the same
/// decision.
#[tokio::test]
async fn row_7_a_backfill_whose_input_is_refused_ends_cancelled() {
    let f = fixture().await;
    let (workflow, _) = f.w.publish(flow("row-7")).await;
    let schedule = f
        .create("row-7", &hourly(), &json!({}), Some(target(workflow)))
        .await;
    f.w.publish_new_version(workflow, with_schema(flow("row-7"), order_schema()))
        .await;
    let backfill = f
        .service
        .create_backfill(
            &f.w.admin,
            schedule.id,
            &BackfillRequest {
                start: t0() - Duration::hours(5),
                end: t0(),
                overlap: None,
            },
            &RequestContext::default(),
        )
        .await
        .expect("a backfill");
    let due = DueBackfill {
        tenant_id: backfill.tenant_id,
        schedule: backfill.schedule,
        id: backfill.id,
    };
    let mut steps = 0;
    while steps < 10 {
        match f.firing.backfill_step(&due, None).await.expect("a step") {
            Fired::Acted => steps += 1,
            Fired::Idle | Fired::Refused(_) => break,
        }
    }
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("INPUT_INVALID", 1)])
    );
    let backfills = f
        .service
        .backfills(&f.w.admin, schedule.id)
        .await
        .expect("backfills");
    assert_eq!(backfills[0].state, BackfillState::Cancelled);
    assert!(f.runs(&schedule).await.is_empty());
}

/// Row 8: trigger-now answers `422` before it decides and records nothing.
#[tokio::test]
async fn row_8_trigger_now_answers_422_and_records_nothing() {
    let f = fixture().await;
    let (workflow, _) = f.w.publish(flow("row-8")).await;
    let schedule = f
        .create("row-8", &hourly(), &json!({}), Some(target(workflow)))
        .await;
    f.w.publish_new_version(workflow, with_schema(flow("row-8"), order_schema()))
        .await;
    let refused = f
        .firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: "trigger-row-8-0001",
            },
            &RequestContext::default(),
        )
        .await
        .expect_err("refused");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "SCHEDULE_INPUT_INVALID",
            // the stored input comes out only through the audited read of FR-018-042: nothing of it is told here
            field: Some("/".to_owned()),
            rule: Some("other"),
        }
    );
    assert!(f.outcomes(&schedule).await.is_empty(), "nothing recorded");
}

/// Row 6, a buffered action checked again at its release: valid when it was buffered, refused when it is released, it
/// becomes `INPUT_INVALID` (the guard of the table allows `BUFFERED` to any final outcome).
#[tokio::test]
async fn a_buffered_action_is_checked_again_at_its_release() {
    let f = fixture().await;
    let (workflow, _) = f.w.publish(flow("row-6-buffer")).await;
    let schedule = f
        .create(
            "row-6-buffer",
            &hourly(),
            &json!({"overlap": "BUFFER_ONE"}),
            Some(target(workflow)),
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // starts
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // waits
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("BUFFERED", 1), ("STARTED", 1)])
    );
    f.w.publish_new_version(workflow, with_schema(flow("row-6-buffer"), order_schema()))
        .await;
    let first = f.runs(&schedule).await[0];
    f.end(first, "SUCCEEDED").await;
    f.clock.advance(Duration::seconds(6));
    f.drain(&schedule).await;
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("INPUT_INVALID", 1), ("STARTED", 1)])
    );
    assert_eq!(f.runs(&schedule).await.len(), 1);
}

/// Row 3 (FR-043-005, OD-2026-11-14-10): a Signal-with-Start whose start input does not match is refused also when an open
/// holder would answer, and the holder receives no signal.
#[tokio::test]
async fn row_3_with_start_is_refused_for_its_input_even_with_an_open_holder() {
    use sagat_application::runs::{SignalAtStart, StartRun};
    use sagat_domain::business_ids::ReusePolicy;

    let f = fixture().await;
    let schema = json!({
        "type": "object",
        "required": ["k"],
        "properties": {"k": {"type": "string"}}
    });
    let mut waiter = document(
        &format!("row-3-{}", uuid::Uuid::now_v7()),
        "w",
        vec![
            crate::support::engine::with(
                crate::support::engine::wait_node(
                    "w",
                    crate::with_start::SIGNAL,
                    "input.k",
                    300_000,
                    None,
                ),
                "next",
                json!("notify"),
            ),
            activity("notify"),
        ],
    );
    waiter["inputSchema"] = schema;
    let (workflow, _) = f.w.publish(waiter).await;
    let send = |input: &'static [u8], key: String| {
        let f = &f;
        async move {
            f.w.runs
                .signal_with_start(
                    &f.w.admin,
                    &StartRun {
                        namespace: "payments",
                        workflow,
                        version: None,
                        input,
                        correlation_id: None,
                        deadline: None,
                        key: &key,
                    },
                    crate::with_start::business("row-3-cart", ReusePolicy::AllowDuplicate),
                    SignalAtStart {
                        name: crate::with_start::SIGNAL,
                        payload: b"{}",
                    },
                    &crate::support::engine::ctx(),
                )
                .await
        }
    };
    let holder = send(br#"{"k": "s"}"#, crate::business_ids::key())
        .await
        .expect("started and signalled");
    let signals = |run: sagat_domain::execution::RunId| {
        let pool = f.w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type LIKE 'signal%'",
            )
            .bind(run.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("signals")
        }
    };
    let before = signals(holder.run.info.run_id).await;
    let refused = send(br#"{"k": 5}"#, crate::business_ids::key())
        .await
        .expect_err("refused although a holder is open");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "RUN_INPUT_INVALID",
            field: Some("/k".to_owned()),
            rule: Some("type"),
        }
    );
    assert_eq!(
        signals(holder.run.info.run_id).await,
        before,
        "the holder got nothing"
    );
}

/// Row 4 (OD-2026-11-14-11): a rerun of a run whose stored input its version refuses (created before the check was on,
/// here in `report` mode) is refused, and says `/` and `other` (D043-037).
#[tokio::test]
async fn row_4_a_rerun_of_an_input_the_version_refuses_says_the_root_and_other() {
    use sagat_application::clock::SystemClock;
    use sagat_application::execution::Engine;
    use sagat_application::runs::{RunService, StartRun};
    use sagat_infra_schema::run_input::RunInputValidator;

    let f = fixture().await;
    let (workflow, _) =
        f.w.publish(with_schema(flow("row-4"), order_schema()))
            .await;
    let before_the_check = RunService::new(
        f.w.store.clone(),
        SystemClock,
        Engine::new(sagat_infra_ir::JsonEvaluator),
        f.w.cache.clone(),
    )
    .with_run_inputs(RunInputValidator::gate(InputValidationMode::Report));
    let source = before_the_check
        .start(
            &f.w.admin,
            &StartRun {
                namespace: "payments",
                workflow,
                version: None,
                input: br#"{"orderId": 9}"#,
                correlation_id: None,
                deadline: None,
                key: &crate::business_ids::key(),
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("admitted in report mode");
    crate::business_ids::cancel(&f, source.info.run_id).await;
    let refused =
        f.w.runs
            .rerun(
                &f.w.admin,
                source.info.run_id,
                "start it again after the fix",
                &crate::business_ids::key(),
                &crate::support::engine::ctx(),
            )
            .await
            .expect_err("refused");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "RUN_INPUT_INVALID",
            field: Some("/".to_owned()),
            rule: Some("other"),
        }
    );
}

/// A run service whose gate refuses every input it is asked about, final: a start that the check would refuse today,
/// retried under the key of a run created before the check was on (design 043, correctness review M1).
pub(crate) fn refusing(w: &crate::support::engine::World) -> crate::support::engine::Runs {
    use std::sync::Arc;

    use sagat_application::run_input::test_support::Scripted;
    use sagat_application::run_input::{PathSegment, Rule, RunInputGate, SchemaRefusal};

    let refusals = (0..16).map(|_| {
        Err(SchemaRefusal::Invalid {
            segments: vec![PathSegment::Property("k".into())],
            rule: Rule::Type,
        })
    });
    sagat_application::runs::RunService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(Arc::new(RunInputGate::new(
        Arc::new(Scripted::answering(refusals)),
        InputValidationMode::Enforce,
    )))
    .with_declared_updates(Arc::new(sagat_infra_ir::JsonDeclaredOperationEvaluator))
    .with_reset_reapply(true)
    .with_reset_cancel_source(true)
}

/// M1 for Signal-with-Start: the retry answers what the first request did, the signal delivered at creation included.
#[tokio::test]
async fn a_signal_with_start_retry_the_gate_refuses_replays_what_the_first_request_delivered() {
    let f = fixture().await;
    let mut waiter = document(
        &format!("row-3-replay-{}", uuid::Uuid::now_v7()),
        "w",
        vec![
            crate::support::engine::with(
                crate::support::engine::wait_node(
                    "w",
                    crate::with_start::SIGNAL,
                    "input.k",
                    300_000,
                    None,
                ),
                "next",
                json!("notify"),
            ),
            activity("notify"),
        ],
    );
    waiter["inputSchema"] = json!({"type": "object"});
    let (workflow, _) = f.w.publish(waiter).await;
    let key = crate::business_ids::key();
    let first = signal(&f, &f.w.runs, workflow, &key)
        .await
        .expect("started and signalled");
    let again = signal(&f, &refusing(&f.w), workflow, &key)
        .await
        .expect("a replay of the first request");
    assert_eq!(again.run.info.run_id, first.run.info.run_id);
    assert_eq!(again.started, first.started);
    assert_eq!(
        again.delivered_to, first.delivered_to,
        "what the first request delivered"
    );
}

async fn signal(
    f: &Fixture,
    runs: &crate::support::engine::Runs,
    workflow: sagat_domain::definition::WorkflowId,
    key: &str,
) -> Result<sagat_application::runs::SignalStarted, AppError> {
    runs.signal_with_start(
        &f.w.admin,
        &sagat_application::runs::StartRun {
            namespace: "payments",
            workflow,
            version: None,
            input: br#"{"k": "s"}"#,
            correlation_id: None,
            deadline: None,
            key,
        },
        crate::with_start::business(
            "row-3-replay",
            sagat_domain::business_ids::ReusePolicy::AllowDuplicate,
        ),
        sagat_application::runs::SignalAtStart {
            name: crate::with_start::SIGNAL,
            payload: b"{}",
        },
        &crate::support::engine::ctx(),
    )
    .await
}

/// M1 for a pending reset: an engine whose gate refuses the source's input, completing an entry another engine
/// completed first, is told `RESET_ALREADY_COMPLETED`, as it is when nothing is refused.
#[tokio::test]
async fn a_pending_reset_the_gate_refuses_after_another_engine_completed_it_is_already_completed() {
    use sagat_application::reset_completion::{Completed, PendingResetStore as _};
    use sagat_application::runs::RESET_ALREADY_COMPLETED;

    let f = fixture().await;
    let open =
        crate::cancel_source::open_chain_with_schema(&f, "", Some(json!({"type": "object"}))).await;
    let made = crate::cancel_source::request(
        &f,
        open.run,
        crate::cancel_source::asking(open.after_a, true),
        &crate::business_ids::key(),
    )
    .await
    .expect("the request");
    crate::cancel_source::end(&f, &open).await;
    let stale =
        f.w.store
            .pending_reset(f.w.admin.tenant_id(), made.reset.id)
            .await
            .expect("read")
            .expect("it is pending");
    assert_eq!(
        crate::cancel_source::completer(&f)
            .step(&crate::cancel_source::due_of(&f, made.reset.id), None)
            .await
            .expect("the other engine"),
        Completed::Created
    );
    let refused = refusing(&f.w)
        .complete_reset(&f.w.admin, &stale, None, &crate::support::engine::ctx())
        .await;
    assert!(
        matches!(refused, Err(AppError::StateConflict(reason)) if reason == RESET_ALREADY_COMPLETED),
        "{refused:?}"
    );
}

/// Correctness review M2: a schedule whose remaining actions are used up is exhausted at its next time even when a later
/// version refuses its input; it records no `INPUT_INVALID`, then or afterwards.
#[tokio::test]
async fn an_exhausted_schedule_records_no_input_invalid_for_a_version_that_refuses_its_input() {
    let f = fixture().await;
    let (workflow, _) = f.w.publish(flow("row-6-exhausted")).await;
    let mut write = f.write_of(
        "row-6-exhausted",
        &hourly(),
        &json!({}),
        Some(target(workflow)),
    );
    write.remaining_actions = Some(sagat_domain::schedules::RemainingActions::new(1).expect("one"));
    let schedule = f.try_create(write).await.expect("created");
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 1)]));
    f.w.publish_new_version(
        workflow,
        with_schema(flow("row-6-exhausted"), order_schema()),
    )
    .await;
    for _ in 0..3 {
        f.clock.advance(Duration::hours(1));
        f.drain(&schedule).await;
    }
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("STARTED", 1)]),
        "nothing is left to give: no time is decided, refused or not"
    );
    assert_eq!(f.read(&schedule).await.next_action_at, None);
}

/// Starts whose check before the decision passes and whose start refuses: a version promoted between the two.
#[derive(Clone)]
struct PromotedBetween {
    refusing: std::sync::Arc<crate::support::engine::Runs>,
}

impl sagat_application::schedule_firing::ScheduledStarts for PromotedBetween {
    async fn start_scheduled(
        &self,
        principal: &sagat_domain::identity::Principal,
        request: &sagat_application::schedule_firing::ScheduledStart<'_>,
        admission: sagat_application::schedules::ScheduleAdmission,
        ctx: &RequestContext,
    ) -> Result<sagat_application::schedule_firing::ScheduledRun, AppError> {
        sagat_application::schedule_firing::ScheduledStarts::start_scheduled(
            self.refusing.as_ref(),
            principal,
            request,
            admission,
            ctx,
        )
        .await
    }

    async fn check_scheduled(
        &self,
        _: &sagat_domain::identity::Principal,
        _: &sagat_application::schedule_firing::ScheduledStart<'_>,
    ) -> Result<(), AppError> {
        Ok(())
    }

    async fn cancel_run(
        &self,
        _: &sagat_domain::identity::Principal,
        _: uuid::Uuid,
        _: &str,
        _: &str,
        _: &RequestContext,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

/// Security review of phase 2: a trigger-now whose start is refused after its check passed (a version promoted between
/// the two) answers `422` and records nothing, as one refused by the check does.
#[tokio::test]
async fn a_trigger_now_refused_at_its_start_answers_422_and_records_nothing() {
    let f = fixture().await;
    let mut checked = flow("row-8-promoted");
    checked["inputSchema"] = json!({"type": "object"});
    let (workflow, _) = f.w.publish(checked).await;
    let schedule = f
        .create(
            "row-8-promoted",
            &hourly(),
            &json!({}),
            Some(target(workflow)),
        )
        .await;
    let firing = sagat_application::schedule_firing::ScheduleFiring::new(
        sagat_infra_postgres::PgScheduleStore::new(f.w.db.pool.clone()),
        sagat_infra_ir::schedule_calendar::SpecCalendar,
        f.clock.clone(),
        sagat_infra_postgres::PgIdentityStore::new(f.w.db.pool.clone()),
        PromotedBetween {
            refusing: std::sync::Arc::new(refusing(&f.w)),
        },
        sagat_domain::schedules::Ceilings::default(),
    );
    let refused = firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: "trigger-row-8-promoted",
            },
            &RequestContext::default(),
        )
        .await
        .expect_err("refused");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "SCHEDULE_INPUT_INVALID",
            field: Some("/".to_owned()),
            rule: Some("other"),
        },
        "D043-144"
    );
    assert!(f.outcomes(&schedule).await.is_empty(), "nothing recorded");
    let triggered: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'schedule.triggered' AND resource_id = $1",
    )
    .bind(schedule.id.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audits");
    assert_eq!(
        triggered, 0,
        "no trigger-now is audited for a request that did nothing"
    );
}

/// Row 8 where the decision would buffer: an open run and `BUFFER_ONE`; the check comes first, so trigger-now answers
/// `422` instead of `BUFFERED`.
#[tokio::test]
async fn row_8_trigger_now_answers_422_where_the_decision_would_buffer() {
    let f = fixture().await;
    let (workflow, _) = f.w.publish(flow("row-8-buffer")).await;
    let schedule = f
        .create(
            "row-8-buffer",
            &hourly(),
            &json!({"overlap": "BUFFER_ONE"}),
            Some(target(workflow)),
        )
        .await;
    let ctx = RequestContext::default();
    let first = TriggerRequest {
        overlap: None,
        key: "trigger-buffer-0001",
    };
    let second = TriggerRequest {
        overlap: None,
        key: "trigger-buffer-0002",
    };
    f.firing
        .trigger(&f.w.admin, schedule.id, &first, &ctx)
        .await
        .expect("starts a run that stays open");
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 1)]));
    f.w.publish_new_version(workflow, with_schema(flow("row-8-buffer"), order_schema()))
        .await;
    let refused = f
        .firing
        .trigger(&f.w.admin, schedule.id, &second, &ctx)
        .await
        .expect_err("refused, not buffered");
    assert!(
        matches!(
            refused,
            AppError::InvalidRunInput {
                reason: "SCHEDULE_INPUT_INVALID",
                ..
            }
        ),
        "{refused:?}"
    );
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("STARTED", 1)]),
        "no BUFFERED"
    );
}

/// The run service `runs` as the starts of a firing.
#[derive(Clone)]
struct Through(std::sync::Arc<crate::support::engine::Runs>);

impl sagat_application::schedule_firing::ScheduledStarts for Through {
    async fn start_scheduled(
        &self,
        principal: &sagat_domain::identity::Principal,
        request: &sagat_application::schedule_firing::ScheduledStart<'_>,
        admission: sagat_application::schedules::ScheduleAdmission,
        ctx: &RequestContext,
    ) -> Result<sagat_application::schedule_firing::ScheduledRun, AppError> {
        sagat_application::schedule_firing::ScheduledStarts::start_scheduled(
            self.0.as_ref(),
            principal,
            request,
            admission,
            ctx,
        )
        .await
    }

    async fn check_scheduled(
        &self,
        principal: &sagat_domain::identity::Principal,
        request: &sagat_application::schedule_firing::ScheduledStart<'_>,
    ) -> Result<(), AppError> {
        sagat_application::schedule_firing::ScheduledStarts::check_scheduled(
            self.0.as_ref(),
            principal,
            request,
        )
        .await
    }

    async fn cancel_run(
        &self,
        principal: &sagat_domain::identity::Principal,
        run: uuid::Uuid,
        reason: &str,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        sagat_application::schedule_firing::ScheduledStarts::cancel_run(
            self.0.as_ref(),
            principal,
            run,
            reason,
            key,
            ctx,
        )
        .await
    }
}

fn scripted_runs(
    w: &crate::support::engine::World,
    answers: Vec<Result<(), sagat_application::run_input::SchemaRefusal>>,
) -> crate::support::engine::Runs {
    use std::sync::Arc;
    sagat_application::runs::RunService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(Arc::new(sagat_application::run_input::RunInputGate::new(
        Arc::new(sagat_application::run_input::test_support::Scripted::answering(answers)),
        InputValidationMode::Enforce,
    )))
}

/// T043-21 for row 6: no room in the validator keeps the time owed (no outcome, a retry set); the work budget is final
/// the first time (`INPUT_INVALID`).
#[tokio::test]
async fn row_6_no_room_keeps_the_time_owed_and_the_budget_is_final() {
    use sagat_application::run_input::SchemaRefusal;

    let f = fixture().await;
    let mut checked = flow("row-6-budget");
    checked["inputSchema"] = json!({"type": "object"});
    let (workflow, _) = f.w.publish(checked).await;
    let schedule = f
        .create(
            "row-6-budget",
            &hourly(),
            &json!({}),
            Some(target(workflow)),
        )
        .await;
    let firing = |answers| {
        sagat_application::schedule_firing::ScheduleFiring::new(
            sagat_infra_postgres::PgScheduleStore::new(f.w.db.pool.clone()),
            sagat_infra_ir::schedule_calendar::SpecCalendar,
            f.clock.clone(),
            sagat_infra_postgres::PgIdentityStore::new(f.w.db.pool.clone()),
            Through(std::sync::Arc::new(scripted_runs(&f.w, answers))),
            sagat_domain::schedules::Ceilings::default(),
        )
    };
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    let busy = firing(vec![Err(SchemaRefusal::Busy)]);
    assert!(matches!(
        busy.fire(&f.due(&schedule), None).await.expect("fire"),
        Fired::Refused(_)
    ));
    assert!(f.outcomes(&schedule).await.is_empty(), "nothing decided");
    assert!(
        f.read(&schedule).await.retry_at.is_some(),
        "the time stays owed"
    );

    f.clock.advance(Duration::minutes(10));
    let costly = firing(vec![Err(SchemaRefusal::WorkLimit)]);
    assert_eq!(
        costly.fire(&f.due(&schedule), None).await.expect("fire"),
        Fired::Acted
    );
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("INPUT_INVALID", 1)])
    );
    assert!(f.runs(&schedule).await.is_empty());
}

/// Counts the counters of one test thread by name and labels.
#[derive(Default)]
struct Counted(std::sync::Mutex<std::collections::BTreeMap<String, u64>>);

struct One(String, std::sync::Arc<Counted>);

impl metrics::CounterFn for One {
    fn increment(&self, value: u64) {
        *self
            .1
            .0
            .lock()
            .expect("counts")
            .entry(self.0.clone())
            .or_default() += value;
    }
    fn absolute(&self, _: u64) {}
}

struct Recorder(std::sync::Arc<Counted>);

impl metrics::Recorder for Recorder {
    fn describe_counter(
        &self,
        _: metrics::KeyName,
        _: Option<metrics::Unit>,
        _: metrics::SharedString,
    ) {
    }
    fn describe_gauge(
        &self,
        _: metrics::KeyName,
        _: Option<metrics::Unit>,
        _: metrics::SharedString,
    ) {
    }
    fn describe_histogram(
        &self,
        _: metrics::KeyName,
        _: Option<metrics::Unit>,
        _: metrics::SharedString,
    ) {
    }
    fn register_counter(&self, key: &metrics::Key, _: &metrics::Metadata<'_>) -> metrics::Counter {
        let labels: Vec<String> = key
            .labels()
            .map(|l| format!("{}={}", l.key(), l.value()))
            .collect();
        metrics::Counter::from_arc(std::sync::Arc::new(One(
            format!("{}{{{}}}", key.name(), labels.join(",")),
            std::sync::Arc::clone(&self.0),
        )))
    }
    fn register_gauge(&self, _: &metrics::Key, _: &metrics::Metadata<'_>) -> metrics::Gauge {
        metrics::Gauge::noop()
    }
    fn register_histogram(
        &self,
        _: &metrics::Key,
        _: &metrics::Metadata<'_>,
    ) -> metrics::Histogram {
        metrics::Histogram::noop()
    }
}

/// Re-review items 1 and 3 (D043-147): one scheduled action of a version with an `inputSchema` is one validation in
/// `run_input_validations_total{creator=schedule}`, though it is checked before its decision and again at its start; both
/// checks run in a `run_input.check` span that records its outcome.
#[tokio::test]
async fn one_scheduled_action_is_one_validation_in_two_spans() {
    let f = fixture().await;
    let mut checked = flow("row-6-counted");
    checked["inputSchema"] = json!({"type": "object"});
    let (workflow, _) = f.w.publish(checked).await;
    let schedule = f
        .create(
            "row-6-counted",
            &hourly(),
            &json!({}),
            Some(target(workflow)),
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    let counted = std::sync::Arc::new(Counted::default());
    let recorder = Recorder(std::sync::Arc::clone(&counted));
    let (_spans_guard, spans) = crate::support::logs::capture_with_spans(tracing::Level::TRACE);
    {
        let _guard = metrics::set_default_local_recorder(&recorder);
        assert_eq!(f.fire(&schedule).await, Fired::Acted);
    }
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 1)]));
    let counts = counted.0.lock().expect("counts").clone();
    let schedule_validations: u64 = counts
        .iter()
        .filter(|(name, _)| name.starts_with("run_input_validations_total{creator=schedule,"))
        .map(|(_, n)| n)
        .sum();
    assert_eq!(schedule_validations, 1, "{counts:?}");
    let text = spans.text();
    assert!(
        text.matches("run_input.check").count() >= 2
            && text.contains("sagat.run_input.outcome=\"accepted\"")
            || text.contains("sagat.run_input.outcome=accepted"),
        "{text}"
    );
    let spans_with_outcome = text
        .lines()
        .filter(|line| line.contains("run_input.check") && line.contains("sagat.run_input.outcome"))
        .count();
    assert!(
        spans_with_outcome >= 2,
        "both checks record their outcome in their span: {text}"
    );
}

/// `RUN_INPUT_SCHEMA_UNUSABLE` on rows 3, 4 and 8: a Signal-with-Start, a rerun and a trigger-now of a version whose schema
/// is outside the subset are refused with it (OD-2026-11-14-11).
#[tokio::test]
async fn an_unusable_schema_refuses_with_start_rerun_and_trigger_now() {
    let f = fixture().await;
    let unusable = AppError::InvalidRunInput {
        reason: "RUN_INPUT_SCHEMA_UNUSABLE",
        field: None,
        rule: None,
    };
    // row 3
    let mut waiter = document(
        &format!("row-3-unusable-{}", uuid::Uuid::now_v7()),
        "w",
        vec![
            crate::support::engine::with(
                crate::support::engine::wait_node(
                    "w",
                    crate::with_start::SIGNAL,
                    "input.k",
                    300_000,
                    None,
                ),
                "next",
                json!("notify"),
            ),
            activity("notify"),
        ],
    );
    waiter["inputSchema"] = unusable_schema();
    let (with_start, _) = publish_in(&f.w, InputValidationMode::Report, &waiter).await;
    assert_eq!(
        signal(&f, &f.w.runs, with_start, &crate::business_ids::key())
            .await
            .expect_err("refused"),
        unusable
    );
    // row 4: a run of the version admitted before the check, started again
    let (workflow, _) = publish_in(
        &f.w,
        InputValidationMode::Report,
        &with_schema(flow("row-4-unusable"), unusable_schema()),
    )
    .await;
    let source = crate::support::run_input::start_in_report(&f.w, workflow, &json!({})).await;
    crate::business_ids::cancel(&f, source).await;
    let refused =
        f.w.runs
            .rerun(
                &f.w.admin,
                source,
                "start it again after the fix",
                &crate::business_ids::key(),
                &crate::support::engine::ctx(),
            )
            .await
            .expect_err("refused");
    assert_eq!(refused, unusable);
    // row 8: a schedule written while its target was usable, whose target then took a version outside the subset
    let (later, _) = f.w.publish(flow("row-8-later")).await;
    let schedule = f
        .create("row-8-unusable", &hourly(), &json!({}), Some(target(later)))
        .await;
    let defs = sagat_application::definitions::DefinitionService::new(
        f.w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_infra_ir::JsonIrCodec,
    )
    .with_input_validation(InputValidationMode::Report);
    let record = defs
        .get_workflow(&f.w.admin, later, &crate::support::engine::ctx())
        .await
        .expect("workflow");
    let draft = defs
        .update_draft(
            &f.w.admin,
            later,
            record.workflow.draft_revision(),
            &serde_json::to_vec(&with_schema(flow("row-8-later"), unusable_schema()))
                .expect("json"),
            &crate::support::engine::ctx(),
        )
        .await
        .expect("draft");
    defs.publish(
        &f.w.admin,
        later,
        draft.workflow.draft_revision(),
        None,
        "publish-unusable-row-8",
        &crate::support::engine::ctx(),
    )
    .await
    .expect("published in report mode");
    let refused = f
        .firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: "trigger-unusable-01",
            },
            &RequestContext::default(),
        )
        .await
        .expect_err("refused");
    assert_eq!(refused, unusable);
    assert!(f.outcomes(&schedule).await.is_empty());
}

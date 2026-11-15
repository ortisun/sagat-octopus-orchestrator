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
    f.clock.advance(Duration::hours(1));
    assert_eq!(
        f.drain(&schedule).await,
        0,
        "a paused schedule does nothing more"
    );
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("INPUT_INVALID", 1)])
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
            field: Some("/orderId".to_owned()),
            rule: Some("required"),
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

//! Design 043, T043-17 (FR-043-014, SC-043-002) on the rows the first sentinel test leaves: a Signal-with-Start (row 3), a
//! reset (row 5), a schedule's action, backfill and trigger-now (rows 6, 7 and 8), a continuation (row 12) and a schedule
//! write (row 14). A sentinel value and a sentinel key, in inputs that fail at them, appear in none of the answers, the logs and spans
//! captured at TRACE, the audit rows, the schedule's action records, the journal and the metrics text. Row 10 (a cron
//! tick) has no caller-chosen value: its document is the platform's. Rows 2 and 13 are covered by
//! `row_2_an_uploaded_input_start_checks_the_descriptor_and_a_refusal_claims_nothing` and the row 13 scenario of
//! `integration_database_connections`. The metrics are recorded on this test's thread, so the text holds only what it did.

use std::time::Duration;

use sagat_application::definitions::InputValidationMode;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::run_input::RunInputGate;
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReuseWindow};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{World, activity, ctx, document};
use crate::support::logs::capture_with_spans;
use crate::support::schedules::{Fixture, fixture, hourly};

const VALUE: &str = "sentinel-043-rows-value-5e2a";
const KEY: &str = "sentinel-043-rows-key-91cd";

/// `orderId` a string of at most 8, other keys only integers: the sentinel value fails under `properties`, the sentinel
/// key under `additionalProperties`.
fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["orderId"],
        "properties": {"orderId": {"type": "string", "maxLength": 8}},
        "additionalProperties": {"type": "integer"}
    })
}

fn inputs() -> [Value; 2] {
    [
        json!({"orderId": VALUE}),
        json!({"orderId": "o-1", KEY: VALUE}),
    ]
}

fn clean(label: &str, text: &str) {
    assert!(
        !text.contains(VALUE),
        "{label}: the sentinel value leaked: {text}"
    );
    assert!(
        !text.contains(KEY),
        "{label}: the sentinel key leaked: {text}"
    );
}

async fn texts(w: &World, sql: &'static str) -> String {
    let rows: Vec<Option<String>> = sqlx::query_scalar(sql)
        .fetch_all(&w.db.pool)
        .await
        .expect("rows");
    rows.into_iter().flatten().collect::<Vec<_>>().join("\n")
}

fn named(tag: &str) -> String {
    format!("sentinel-{tag}-{}", Uuid::now_v7())
}

/// A run service in `report` mode: it admits what the version refuses, as a run created before 0.3.0 was.
fn reported(w: &World) -> crate::support::engine::Runs {
    sagat_application::runs::RunService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(std::sync::Arc::new(RunInputGate::new(
        sagat_infra_schema::run_input::RunInputValidator::process(),
        InputValidationMode::Report,
    )))
}

#[test]
fn no_sentinel_reaches_an_answer_a_log_an_audit_a_record_or_a_metric_on_the_other_rows() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (telemetry, recorder) = sagat_telemetry::Metrics::detached(Duration::from_secs(3600));
    metrics::with_local_recorder(&recorder, || {
        runtime.block_on(Box::pin(the_other_rows(&telemetry)));
    });
}

async fn the_other_rows(telemetry: &sagat_telemetry::Metrics) {
    let f = fixture().await;
    let w = &f.w;
    let mut order = document(&named("order"), "a", vec![activity("a")]);
    order["inputSchema"] = schema();
    let (workflow, _) = w.publish(order).await;

    let scheduler = chain_scheduler(w).await;
    let (_guard, captured) = capture_with_spans(tracing::Level::TRACE);
    let mut answers = String::new();
    for (n, input) in inputs().into_iter().enumerate() {
        let bytes = serde_json::to_vec(&input).expect("json");
        // row 3: a Signal-with-Start whose start input is refused
        let refused = w
            .runs
            .signal_with_start(
                &w.admin,
                &sagat_application::runs::StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: &bytes,
                    correlation_id: None,
                    deadline: None,
                    key: &format!("sws-{}", Uuid::now_v7()),
                },
                sagat_application::runs::BusinessStart {
                    id: BusinessId::new(&format!("sentinel-order-{n}")).expect("id"),
                    conflict: ConflictPolicy::UseExisting,
                    reuse: sagat_domain::business_ids::ReusePolicy::AllowDuplicate,
                    window: ReuseWindow::default(),
                },
                sagat_application::runs::SignalAtStart {
                    name: "payment.confirmed",
                    payload: b"{}",
                },
                &ctx(),
            )
            .await
            .expect_err("refused");
        assert!(
            matches!(refused, AppError::InvalidRunInput { .. }),
            "row 3: {refused:?}"
        );
        answers.push_str(&format!("{refused:?} {refused}\n"));

        // row 5: a reset of a run admitted before the check with the sentinel
        let source = reported(w)
            .start(
                &w.admin,
                &sagat_application::runs::StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: &bytes,
                    correlation_id: None,
                    deadline: None,
                    key: &format!("start-{}", Uuid::now_v7()),
                },
                &ctx(),
            )
            .await
            .expect("admitted in report mode")
            .info
            .run_id;
        // the source runs to its end, so that the completion of `a` is a point to reset to
        let session = w.session(&["act.a"]).await;
        let task = w.one_task_of(&session, "act.a").await;
        w.complete(&session, &task, &json!({}))
            .await
            .expect("a completes");
        let started: i64 = sqlx::query_scalar(
            "SELECT seq FROM run_events WHERE run_id = $1 AND event_type = 'step.completed'",
        )
        .bind(source.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the completion");
        let refused = w
            .runs
            .reset(
                &w.admin,
                source,
                sagat_application::runs::ResetRequest {
                    sequence: u64::try_from(started).expect("small"),
                    reason: "start it again after the fix",
                    acknowledge_reexecution: true,
                    business: sagat_application::runs::RerunBusinessId::Keep,
                    reapply: sagat_domain::execution::ResetReapply::None,
                    source_policy: sagat_application::runs::ResetSourcePolicy::RequireTerminal,
                },
                &format!("reset-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .map(|_| ())
            .expect_err("refused");
        assert!(
            matches!(refused, AppError::InvalidRunInput { .. }),
            "row 5: {refused:?}"
        );
        answers.push_str(&format!("{refused:?} {refused}\n"));

        schedule_rows(&f, &input, &mut answers).await;
        continuation_row(w, &scheduler, &input).await;
    }

    clean("the answers", &answers);
    clean("the logs and spans", &captured.text());
    clean(
        "the audit rows",
        &texts(w, "SELECT reason FROM audit_records").await,
    );
    clean(
        "the schedule's records",
        &texts(
            w,
            "SELECT coalesce(reason, '') || ' ' || coalesce(summary_count::text, '') FROM schedule_actions",
        )
        .await,
    );
    clean(
        "the journal's stops",
        &texts(
            w,
            "SELECT detail::text FROM run_events WHERE event_type IN ('run.stop_requested', 'run.finished')",
        )
        .await,
    );
    let rendered = telemetry.render();
    assert!(
        rendered.contains("run_input_validations_total"),
        "the refusals were counted: {rendered}"
    );
    clean("the metrics", &rendered);
}

/// Rows 14, 6, 8 and 7: a write whose input the target refuses; an action whose input a later version refuses;
/// trigger-now on that schedule; a backfill step of it.
async fn schedule_rows(f: &Fixture, input: &Value, answers: &mut String) {
    let strict = named("strict");
    let mut doc = document(&strict, "a", vec![activity("a")]);
    doc["inputSchema"] = schema();
    let (target, _) = f.w.publish(doc).await;
    let mut write = f.write_of(
        &named("row-14"),
        &hourly(),
        &json!({}),
        Some(json!({"workflowId": target.to_string()})),
    );
    write.input = input.clone();
    let refused = f.try_create(write).await.expect_err("refused");
    answers.push_str(&format!("{refused:?} {refused}\n"));

    let lenient = named("lenient");
    let (workflow, _) =
        f.w.publish(document(&lenient, "a", vec![activity("a")]))
            .await;
    let mut write = f.write_of(
        &named("row-6"),
        &hourly(),
        &json!({}),
        Some(json!({"workflowId": workflow.to_string()})),
    );
    write.input = input.clone();
    let schedule = f.try_create(write).await.expect("no schema yet");
    let mut doc = document(&lenient, "a", vec![activity("a")]);
    doc["inputSchema"] = schema();
    f.w.publish_new_version(workflow, doc).await;
    let refused = f
        .firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: &format!("trigger-{}", Uuid::now_v7()),
            },
            &RequestContext::default(),
        )
        .await
        .expect_err("refused");
    answers.push_str(&format!("{refused:?} {refused}\n"));
    f.clock
        .advance(time::Duration::hours(1) + time::Duration::seconds(1));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert_eq!(
        f.outcomes(&schedule).await,
        vec![("INPUT_INVALID".to_owned(), 1)]
    );
    // row 7: a backfill of earlier times, whose first step is refused and ends it `CANCELLED`
    let backfill = f
        .service
        .create_backfill(
            &f.w.admin,
            schedule.id,
            &sagat_application::schedules::BackfillRequest {
                start: crate::support::schedules::t0() - time::Duration::hours(3),
                end: crate::support::schedules::t0(),
                overlap: None,
            },
            &RequestContext::default(),
        )
        .await
        .expect("a backfill");
    let due = sagat_application::schedules::DueBackfill {
        tenant_id: backfill.tenant_id,
        schedule: backfill.schedule,
        id: backfill.id,
    };
    for _ in 0..10 {
        if f.firing.backfill_step(&due, None).await.expect("a step") != Fired::Acted {
            break;
        }
    }
    assert_eq!(
        f.outcomes(&schedule).await,
        vec![("INPUT_INVALID".to_owned(), 2)]
    );
}

/// Row 12: a continuation whose evaluated input carries the sentinel.
async fn continuation_row(w: &World, scheduler: &crate::support::engine::Sched, input: &Value) {
    let name = named("chain");
    let (workflow, _) = w
        .publish(json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": name},
            "entrypoint": "work",
            "inputSchema": {
                "type": "object",
                "properties": {"next": {"type": "object"}, "orderId": {"type": "string", "maxLength": 8}},
                "additionalProperties": {"type": "integer"}
            },
            "nodes": [
                {"id": "work", "type": "activity", "activity": "sentinel.chain", "next": "rollover"},
                {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
            ],
            "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
        }))
        .await;
    let chain = w.start(workflow, &json!({"next": input})).await;
    let session = w.session(&["sentinel.chain"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("complete");
    scheduler.tick().await;
    assert_eq!(
        w.run_status(chain).await,
        sagat_domain::execution::RunStatus::Failed
    );
}

/// One scheduler for the test, holding every shard: a second one with another placement would hold none for a minute.
async fn chain_scheduler(w: &World) -> crate::support::engine::Sched {
    let placement = sagat_scheduler::Placement::new(format!("sentinel-{}", Uuid::now_v7()));
    sagat_scheduler::Keeper::new(
        w.store.clone(),
        placement.clone(),
        sagat_scheduler::KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("shards");
    sagat_scheduler::Scheduler::new(
        w.store.clone(),
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_scheduler::SchedulerSettings::default(),
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        InputValidationMode::Enforce,
    ))
    .with_children(w.cache.clone())
    .with_placement(placement)
}

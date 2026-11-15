//! Design 043, T043-17 (FR-043-014, SC-043-002): a sentinel value, and a sentinel key under `additionalProperties` (the
//! only place a caller's key can reach the subset), sent in inputs that fail at them on rows 1, 4, 9 and 11, appear in
//! none of the answer, the logs and spans captured at TRACE, and the audit rows.

use sagat_application::definitions::InputValidationMode;
use sagat_application::run_input::RunInputGate;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{World, activity, child_node, ctx, document, world};
use crate::support::logs::capture_with_spans;

const VALUE: &str = "sentinel-043-value-0d1c";
const KEY: &str = "sentinel-043-key-77ab";

/// Declares `orderId` (a string) and allows other keys only as integers: the sentinel key fails under
/// `additionalProperties`, the sentinel value under `properties`.
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

async fn audit_text(w: &World) -> String {
    let rows: Vec<Option<String>> = sqlx::query_scalar("SELECT reason FROM audit_records")
        .fetch_all(&w.db.pool)
        .await
        .expect("audit");
    rows.into_iter().flatten().collect::<Vec<_>>().join("\n")
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

#[tokio::test]
async fn no_sentinel_reaches_an_answer_a_log_a_span_or_an_audit_row_on_any_row() {
    let w = world().await;
    let mut order = document(
        &format!("sentinel-order-{}", Uuid::now_v7()),
        "a",
        vec![activity("a")],
    );
    order["inputSchema"] = schema();
    let order_name = order["metadata"]["name"].as_str().expect("name").to_owned();
    let (workflow, _) = w.publish(order.clone()).await;
    let mut triggered = document(
        &format!("sentinel-trigger-{}", Uuid::now_v7()),
        "a",
        vec![activity("a")],
    );
    triggered["triggers"] = json!([{"kind": "event", "eventType": "sentinel.happened"}]);
    triggered["inputSchema"] = json!({
        "type": "object",
        "properties": {"data": schema()}
    });
    w.publish(triggered).await;
    let (parent, _) = w
        .publish(document(
            &format!("sentinel-parent-{}", Uuid::now_v7()),
            "c",
            vec![child_node("c", &order_name, None)],
        ))
        .await;

    let (_guard, captured) = capture_with_spans(tracing::Level::TRACE);
    let mut answers = String::new();
    for input in inputs() {
        // row 1: a start
        let refused = w
            .start_with(workflow, &input, &format!("start-{}", Uuid::now_v7()), None)
            .await
            .expect_err("refused");
        answers.push_str(&format!("{refused:?} {refused}\n"));
        // row 11: a child of a parent without a schema, given the same input
        let run = w.start(parent, &input).await;
        w.scheduler.tick().await;
        let code: Option<String> =
            sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = 'c'")
                .bind(run.as_uuid())
                .fetch_one(&w.db.pool)
                .await
                .expect("code");
        assert_eq!(code.as_deref(), Some("CHILD_INPUT_INVALID"));
        // row 9: an event whose data carries it
        w.events
            .publish(
                &w.admin,
                sagat_application::events::PublishRequest {
                    namespace: "payments".to_owned(),
                    event_id: Some(
                        sagat_domain::events::EventId::parse(&format!(
                            "sentinel-{}",
                            Uuid::now_v7()
                        ))
                        .expect("id"),
                    ),
                    event_type: sagat_domain::workflow::EventType::parse("sentinel.happened")
                        .expect("type"),
                    subject: None,
                    correlation_key: None,
                    occurred_at: None,
                    data: serde_json::to_vec(&input).expect("json"),
                    dedupe_ttl: None,
                },
                &ctx(),
            )
            .await
            .expect("published");
        w.dispatcher.tick().await;
    }
    // row 4: a rerun of a run admitted in `report` mode with the sentinel, refused when started again
    let reported = sagat_application::runs::RunService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(std::sync::Arc::new(RunInputGate::new(
        sagat_infra_schema::run_input::RunInputValidator::process(),
        InputValidationMode::Report,
    )));
    let bytes = serde_json::to_vec(&inputs()[1]).expect("json");
    let source = reported
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
        .expect("admitted in report mode");
    w.operate_run(
        source.info.run_id,
        sagat_application::runs::RunOperation::Cancel,
    )
    .await;
    let refused = w
        .runs
        .rerun(
            &w.admin,
            source.info.run_id,
            "a rerun after the fix",
            &format!("rerun-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect_err("refused");
    answers.push_str(&format!("{refused:?} {refused}\n"));

    clean("the answers", &answers);
    clean("the logs and spans", &captured.text());
    clean("the audit rows", &audit_text(&w).await);
    assert!(
        audit_text(&w).await.contains("RUN_INPUT_INVALID rule="),
        "the trigger refusals were audited"
    );
}

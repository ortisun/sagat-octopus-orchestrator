//! What the tests of design 043 (run input validation) share: the schema of the examples, a publication in the mode a test
//! asks for (an `inputSchema` outside the supported subset is published only in `report`, as a version stored before 0.3.0
//! would be), and counts read from the database rather than from the service under test.

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{DefinitionService, InputValidationMode};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};
use uuid::Uuid;

use super::engine::{World, ctx};

/// A value that must never appear in a refusal, a log, a span, an audit row or a metric (FR-043-014).
pub const SENTINEL: &str = "sentinel-043-value-that-must-not-leak";

/// `{"orderId": string}` required, nothing else declared.
#[must_use]
pub fn order_schema() -> Value {
    json!({
        "type": "object",
        "required": ["orderId"],
        "properties": {"orderId": {"type": "string", "maxLength": 64}}
    })
}

/// A schema outside the subset: `patternProperties` (refuses every start of its version, OD-2026-11-14-11).
#[must_use]
pub fn unusable_schema() -> Value {
    json!({"type": "object", "patternProperties": {"^x-": {"type": "string"}}})
}

/// `document` with `schema` as its `inputSchema`.
#[must_use]
pub fn with_schema(mut document: Value, schema: Value) -> Value {
    document["inputSchema"] = schema;
    document
}

/// Creates the workflow of `document` and publishes it with a definition service in `mode`: `report` publishes a schema
/// outside the subset, as a version stored before 0.3.0 would be.
pub async fn publish_in(
    w: &World,
    mode: InputValidationMode,
    document: &Value,
) -> (WorkflowId, WorkflowVersionId) {
    let defs = DefinitionService::new(w.store.clone(), SystemClock, JsonIrCodec)
        .with_input_validation(mode);
    let name = document["metadata"]["name"].as_str().expect("name");
    let created = defs
        .create_workflow(
            &w.admin,
            "payments",
            name,
            &serde_json::to_vec(document).expect("json"),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("a draft");
    let published = defs
        .publish(
            &w.admin,
            created.workflow.id(),
            created.workflow.draft_revision(),
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("published");
    (created.workflow.id(), published.version.id())
}

/// Runs of `workflow`, read as the owner.
pub async fn runs_of(w: &World, workflow: WorkflowId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM runs WHERE workflow_id = $1")
        .bind(workflow.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count runs")
}

/// Audit rows with `action` whose reason contains `needle`, read as the owner.
pub async fn audits(w: &World, action: &str, needle: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = $1 AND coalesce(reason, '') LIKE '%' || $2 || '%'",
    )
    .bind(action)
    .bind(needle)
    .fetch_one(&w.db.pool)
    .await
    .expect("count audits")
}

/// Whether `text` holds the sentinel.
#[must_use]
pub fn leaks(text: &str) -> bool {
    text.contains(SENTINEL)
}

/// Starts a run of `workflow` with `input` through a run service in `report` mode: a run admitted before the check was
/// on, as a version stored before 0.3.0 would have, whatever its schema says of the input.
pub async fn start_in_report(
    w: &World,
    workflow: WorkflowId,
    input: &Value,
) -> sagat_domain::execution::RunId {
    let bytes = serde_json::to_vec(input).expect("json");
    sagat_application::runs::RunService::new(
        w.store.clone(),
        SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        InputValidationMode::Report,
    ))
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
    .run_id
}

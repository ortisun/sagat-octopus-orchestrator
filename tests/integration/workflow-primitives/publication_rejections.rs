//! WP-02/13 publication failures are HTTP contracts; SDKs do not expose a publisher.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::api::{Options, json_of, start_with};

fn collection(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "line.process", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines", "body": "line",
             "result": "bodyResult", "maxItems": 3, "maxConcurrency": 1, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

async fn assert_rejected(document: Value, expected_reason: &str) -> Value {
    let api = start_with(Options {
        runtime_role: true,
        ..Options::default()
    })
    .await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "publication").await;
    api.create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": document["metadata"]["name"], "ir": document}))
        .send()
        .await
        .expect("save an editable draft");
    let status = created.status();
    let created = json_of(created).await;
    assert_eq!(status, StatusCode::CREATED, "draft: {created}");
    let workflow = created["id"].as_str().expect("workflow id");
    let validation = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/validate"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .json(&json!({}))
        .send()
        .await
        .expect("draft validation response");
    let validation_status = validation.status();
    let validation = json_of(validation).await;
    assert_eq!(validation_status, StatusCode::OK, "{validation}");
    assert_eq!(validation["valid"], false, "{validation}");
    assert!(
        !validation["problems"]
            .as_array()
            .expect("problems")
            .is_empty()
    );
    let response = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("publication response");
    let status = response.status();
    let header_trace = response
        .headers()
        .get("x-trace-id")
        .expect("trace header")
        .to_str()
        .expect("trace header text")
        .to_owned();
    let body = json_of(response).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "VALIDATION_FAILED", "{body}");
    assert_eq!(body["retryable"], false, "{body}");
    let trace = body["traceId"].as_str().expect("trace id");
    assert_eq!(trace, header_trace);
    assert_eq!(trace.len(), 32);
    assert!(trace.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let persisted: (Option<Uuid>, i64) = sqlx::query_as(
        "SELECT active_version_id, (SELECT count(*) FROM workflow_versions \
           WHERE tenant_id=$1 AND workflow_id=$2) FROM workflows WHERE tenant_id=$1 AND id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(Uuid::parse_str(workflow).expect("workflow UUID"))
    .fetch_one(&api.db.pool)
    .await
    .expect("independent publication projection");
    assert_eq!(
        persisted,
        (None, 0),
        "a rejected draft must not publish a version"
    );
    assert_eq!(body["details"]["reason"], expected_reason, "{body}");
    validation
}

#[tokio::test]
async fn a_terminal_body_node_is_rejected_by_the_canonical_schema_over_http() {
    let mut document = collection("publication-body-terminal");
    document["bodies"]["line"]["nodes"] = json!([{"id": "work", "type": "end"}]);
    assert_rejected(document, "SCHEMA").await;
}

#[tokio::test]
async fn a_body_route_into_the_root_returns_invalid_scope_over_http() {
    let mut document = collection("publication-body-escape");
    document["bodies"]["line"]["nodes"][0]["next"] = json!("done");
    let validation = assert_rejected(document, "INVALID_SCOPE").await;
    assert_eq!(validation["problems"][0]["code"], "UNKNOWN_TARGET");
    assert_eq!(validation["problems"][0]["nodeId"], "work");
    assert_eq!(validation["problems"][0]["path"], "/bodies/line");
}

#[tokio::test]
async fn a_recursive_body_returns_invalid_scope_over_http() {
    let mut document = collection("publication-body-recursion");
    document["bodies"]["line"]["nodes"] = json!([
        {"id": "work", "type": "forEach", "items": "input.lines", "body": "line",
         "result": "bodyResult", "maxItems": 3, "maxConcurrency": 1, "next": "returned"},
        {"id": "returned", "type": "return", "result": "steps.work.output"}
    ]);
    assert_rejected(document, "INVALID_SCOPE").await;
}

#[tokio::test]
async fn a_body_edge_cycle_returns_invalid_scope_over_http() {
    let mut document = collection("publication-body-edge-cycle");
    document["bodies"]["line"]["nodes"][0]["next"] = json!("work");
    assert_rejected(document, "INVALID_SCOPE").await;
}

#[tokio::test]
async fn overlapping_parallel_branches_return_invalid_scope_over_http() {
    let mut document = collection("publication-overlapping-branches");
    document.as_object_mut().expect("document").remove("bodies");
    document["entrypoint"] = json!("fork");
    document["nodes"] = json!([
        {"id": "fork", "type": "parallel", "branches": ["left", "right"], "join": "all", "next": "done"},
        {"id": "left", "type": "activity", "activity": "line.left", "next": "shared"},
        {"id": "right", "type": "activity", "activity": "line.right", "next": "shared"},
        {"id": "shared", "type": "activity", "activity": "line.shared"},
        {"id": "done", "type": "end"}
    ]);
    assert_rejected(document, "INVALID_SCOPE").await;
}

#[tokio::test]
async fn an_object_literal_is_rejected_at_http_publication_before_execution() {
    let mut document = collection("publication-object-literal");
    document["nodes"][0]["items"] = json!("[{'a': 1}]");
    assert_rejected(document, "EXPRESSION").await;
}

#[tokio::test]
async fn an_oversized_multibyte_expression_returns_expression_instead_of_scope() {
    let mut document = collection("publication-multibyte-expression");
    // 602 characters pass schema maxLength; 1202 UTF-8 bytes exceed the effective 1024-byte limit.
    document["bodies"]["line"]["nodes"][1]["result"] = json!(format!("'{}'", "é".repeat(600)));
    let validation = assert_rejected(document, "EXPRESSION").await;
    assert_eq!(validation["problems"][0]["code"], "EXPRESSION");
    assert_eq!(validation["problems"][0]["nodeId"], "returned");
}

#[tokio::test]
async fn an_oversized_multibyte_switch_predicate_returns_expression_over_http() {
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "publication-multibyte-switch"}, "entrypoint": "choose",
        "nodes": [
            {"id": "choose", "type": "switch", "cases": [
                {"id": "large", "when": format!("'{}'", "é".repeat(600)), "target": "matched"}
            ], "default": "fallback"},
            {"id": "matched", "type": "end"}, {"id": "fallback", "type": "end"}
        ]
    });
    let validation = assert_rejected(document, "EXPRESSION").await;
    assert_eq!(validation["problems"][0]["code"], "EXPRESSION");
    assert_eq!(validation["problems"][0]["nodeId"], "choose");
}

#[tokio::test]
async fn a_multibyte_expression_above_the_legacy_wrapper_limit_returns_expression() {
    let mut document = collection("publication-above-wrapper-limit");
    // 702 characters pass schema maxLength; 2102 bytes also exceed Expression's legacy wrapper.
    document["bodies"]["line"]["nodes"][1]["result"] = json!(format!("'{}'", "€".repeat(700)));
    let validation = assert_rejected(document, "EXPRESSION").await;
    assert_eq!(validation["problems"][0]["code"], "EXPRESSION");
    assert_eq!(validation["problems"][0]["nodeId"], "returned");
}

#[tokio::test]
async fn an_ordinary_missing_root_target_keeps_the_existing_graph_reason() {
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "publication-missing-root-target"}, "entrypoint": "work",
        "nodes": [{"id": "work", "type": "activity", "activity": "line.work", "next": "ghost"}]
    });
    assert_rejected(document, "GRAPH").await;
}

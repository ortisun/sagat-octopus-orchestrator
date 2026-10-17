//! Validate is an authoring read: bounded effective levels, no persistence or audit side effect.
use crate::support::{contract::OpenApi, worker::platform};
use reqwest::Method;
use serde_json::{Value, json};

fn source(name: &str) -> Value {
    json!({"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow",
        "metadata":{"name":name},"entrypoint":"done","nodes":[{"id":"done","type":"end"}],
        "inputSchema":{"type":"object","x-sagat-classification":"public","properties":{"receipt":{"type":"string"}}},
        "outputSchema":{"type":"object"}})
}

#[tokio::test]
async fn validate_returns_the_closed_report_and_does_not_store_or_audit_it() {
    let p = platform().await;
    let name = "classification-report";
    let workflow = p.publish(name, &source(name)).await;
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_records")
        .fetch_one(&p.api.db.pool)
        .await
        .expect("audit count");
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/workflows/{workflow}/validate"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    OpenApi::load()
        .validate_body("POST", "/workflows/{workflowId}/validate", status, &body)
        .expect("documented body")
        .expect("contract");
    assert_eq!(body["valid"], true);
    let report = body["classification"]
        .as_array()
        .expect("classification report");
    assert!(report.iter().any(|entry| entry["carrier"] == "inputSchema"
        && entry["path"] == ""
        && entry["level"] == "public"
        && entry["declared"] == true));
    assert!(report.iter().any(|entry| entry["carrier"] == "inputSchema"
        && entry["path"] == "/receipt"
        && entry["level"] == "public"
        && entry["declared"] == false));
    assert!(report.iter().any(|entry| entry["carrier"] == "outputSchema"
        && entry["path"] == ""
        && entry["level"] == "pii"));
    assert!(
        body["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|problem| problem["code"] == "CLASSIFICATION_INHERITED_OPEN")
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_records")
        .fetch_one(&p.api.db.pool)
        .await
        .expect("audit count");
    assert_eq!(before, after);
    let mut invalid = source(name);
    invalid["inputSchema"]["x-sagat-classification"] = json!("pii");
    invalid["inputSchema"]["properties"]["receipt"]["x-sagat-classification"] = json!("public");
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/workflows/{workflow}/validate"),
            Some(&json!({"ir":invalid})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["valid"], false);
    assert!(
        body["problems"]
            .as_array()
            .expect("problems")
            .iter()
            .any(|problem| problem["code"] == "CLASSIFICATION_LOOSENED")
    );
}

#[tokio::test]
async fn publication_returns_classification_reason_instead_of_generic_schema_failure() {
    let p = platform().await;
    let name = "classification-error";
    let mut invalid = source(name);
    invalid["metadata"]["x-sagat-classification"] = json!("secret");
    let (status, created) = p
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name":name,"ir":invalid})),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    let workflow = created["id"].as_str().expect("workflow id");
    let (status, body) = p
        .rest(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            Some(&json!({"draftRevision":1})),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], "VALIDATION_FAILED");
    assert_eq!(body["details"]["reason"], "CLASSIFICATION");
    assert!(
        body["details"]["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .any(|issue| issue["code"] == "CLASSIFICATION_NOT_APPLICABLE"
                && issue["path"] == "/metadata/x-sagat-classification")
    );
    assert!(
        !body["message"]
            .as_str()
            .expect("safe message")
            .contains("secret")
    );
}

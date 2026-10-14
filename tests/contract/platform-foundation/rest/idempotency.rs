//! `Idempotency-Key` on the two creating operations of the Identity context (gap G8): optional, so
//! existing clients are unaffected, and when sent a retry is answered with the first response.

use reqwest::{Method, Response};
use serde_json::{Value, json};

use crate::support::api::{TestApi, json_of, start};
use crate::support::contract::OpenApi;

const KEY: &str = "create-platform-0001";

async fn post(api: &TestApi, path: &str, key: Option<&str>, body: Value) -> Response {
    let mut request = api
        .request(Method::POST, path)
        .bearer_auth(api.idp.token("alice", "acme").build())
        .json(&body);
    if let Some(key) = key {
        request = request.header("Idempotency-Key", key);
    }
    request.send().await.expect("send")
}

#[tokio::test]
async fn a_retried_workspace_creation_gets_the_same_workspace_back() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    let spec = OpenApi::load();

    let first = post(&api, "/workspaces", Some(KEY), json!({"slug": "platform"})).await;
    assert_eq!(first.status(), 201);
    let first = json_of(first).await;
    let again = post(&api, "/workspaces", Some(KEY), json!({"slug": "platform"})).await;
    assert_eq!(
        again.status(),
        201,
        "the retry gets the status of the original"
    );
    let again = json_of(again).await;

    assert_eq!(again, first, "the same workspace, byte for byte");
    spec.validate_body("POST", "/workspaces", 201, &first)
        .expect("a documented body")
        .expect("conforms");
    let page = json_of(api.get_as("/workspaces", "alice", "acme").await).await;
    assert_eq!(page["items"].as_array().expect("items").len(), 1);
}

#[tokio::test]
async fn the_same_key_for_a_different_request_is_a_conflict_with_a_reason() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    post(&api, "/workspaces", Some(KEY), json!({"slug": "platform"})).await;

    let other = post(&api, "/workspaces", Some(KEY), json!({"slug": "billing"})).await;

    assert_eq!(other.status(), 409);
    let body = json_of(other).await;
    assert_eq!(body["code"], "CONFLICT");
    assert_eq!(body["details"]["reason"], "IDEMPOTENCY_KEY_REUSED");
    assert!(
        body["remediation"]
            .as_str()
            .is_some_and(|s| s.contains("Idempotency-Key")),
        "{body}"
    );
}

#[tokio::test]
async fn without_a_key_nothing_changes_for_existing_clients() {
    let api = start().await;
    api.tenant("acme", "alice").await;

    let first = post(&api, "/workspaces", None, json!({"slug": "platform"})).await;
    let again = post(&api, "/workspaces", None, json!({"slug": "platform"})).await;

    assert_eq!(first.status(), 201);
    assert_eq!(again.status(), 409);
    assert_eq!(json_of(again).await["details"]["field"], "slug");
}

#[tokio::test]
async fn a_malformed_key_is_a_bad_request_not_silently_ignored() {
    let api = start().await;
    api.tenant("acme", "alice").await;

    for key in ["short", &"k".repeat(161), "has a space in it"] {
        let response = post(&api, "/workspaces", Some(key), json!({"slug": "platform"})).await;
        assert_eq!(response.status(), 400, "{key:?}");
        assert_eq!(json_of(response).await["code"], "BAD_REQUEST");
    }
    let page = json_of(api.get_as("/workspaces", "alice", "acme").await).await;
    assert!(page["items"].as_array().expect("items").is_empty());
}

#[tokio::test]
async fn a_retried_namespace_creation_gets_the_same_namespace_back() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "platform").await;
    let path = format!("/workspaces/{workspace}/namespaces");
    let body = json!({"slug": "dev", "environment": "dev"});

    let first = post(&api, &path, Some(KEY), body.clone()).await;
    assert_eq!(first.status(), 201);
    let first = json_of(first).await;
    let again = post(&api, &path, Some(KEY), body).await;
    assert_eq!(again.status(), 201);

    assert_eq!(json_of(again).await, first);
    let other = post(
        &api,
        &path,
        Some(KEY),
        json!({"slug": "dev", "environment": "prod"}),
    )
    .await;
    assert_eq!(other.status(), 409);
    assert_eq!(
        json_of(other).await["details"]["reason"],
        "IDEMPOTENCY_KEY_REUSED"
    );
}

#[tokio::test]
async fn replay_never_bypasses_a_revoked_permission() {
    let api = start().await;
    let tenant = api.tenant("acme", "alice").await;
    assert_eq!(
        post(&api, "/workspaces", Some(KEY), json!({"slug": "platform"}))
            .await
            .status(),
        201
    );
    sqlx::query(
        "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = 'alice'",
    )
    .bind(tenant.as_uuid())
    .execute(&api.db.pool)
    .await
    .expect("demote");
    assert_eq!(
        post(&api, "/workspaces", Some(KEY), json!({"slug": "platform"}))
            .await
            .status(),
        403
    );
    sqlx::query("DELETE FROM memberships WHERE tenant_id = $1 AND subject = 'alice'")
        .bind(tenant.as_uuid())
        .execute(&api.db.pool)
        .await
        .expect("remove membership");
    assert_eq!(
        post(&api, "/workspaces", Some(KEY), json!({"slug": "platform"}))
            .await
            .status(),
        401
    );
}

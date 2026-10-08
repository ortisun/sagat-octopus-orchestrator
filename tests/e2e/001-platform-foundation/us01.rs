//! US01 — Create a workspace and namespace (FR-001-001, FR-001-002, FR-001-006).

use reqwest::Method;
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::support::api::{Options, json_of, start, start_with};
use crate::{trace_id, without_trace};

#[tokio::test]
async fn an_administrator_creates_a_workspace_and_a_dev_namespace_and_both_are_visible() {
    // Given an authenticated administrator
    let api = start().await;
    api.tenant("acme", "alice").await;

    // When they create a workspace and a dev namespace
    let created = api
        .post_as("/workspaces", "alice", "acme", &json!({"slug": "platform"}))
        .await;
    assert_eq!(created.status(), 201);
    trace_id(&created);
    let workspace = json_of(created).await;
    let workspace_id = workspace["id"].as_str().expect("id").to_owned();
    assert_eq!(workspace["slug"], "platform");
    OffsetDateTime::parse(
        workspace["createdAt"].as_str().expect("createdAt"),
        &Rfc3339,
    )
    .expect("RFC 3339 timestamp");
    assert!(
        workspace.get("tenantId").is_none(),
        "the tenant is implied, never echoed"
    );

    let created = api
        .post_as(
            &format!("/workspaces/{workspace_id}/namespaces"),
            "alice",
            "acme",
            &json!({"slug": "dev", "environment": "dev"}),
        )
        .await;
    assert_eq!(created.status(), 201);
    let namespace = json_of(created).await;
    assert_eq!(namespace["workspaceId"], workspace_id);
    assert_eq!(namespace["environment"], "dev");
    let namespace_id = namespace["id"].as_str().expect("id").to_owned();

    // Then the scope appears in the API ...
    let got = json_of(
        api.get_as(&format!("/workspaces/{workspace_id}"), "alice", "acme")
            .await,
    )
    .await;
    assert_eq!(got, workspace);
    let got = json_of(
        api.get_as(&format!("/namespaces/{namespace_id}"), "alice", "acme")
            .await,
    )
    .await;
    assert_eq!(got, namespace);
    let listed = json_of(api.get_as("/namespaces", "alice", "acme").await).await;
    assert_eq!(listed["items"], json!([namespace]));
    assert_eq!(listed["nextCursor"], json!(null));

    // ... it is persisted, and each mutation was audited with its request's trace id
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT slug, environment FROM namespaces")
        .fetch_all(&api.db.pool)
        .await
        .expect("rows");
    assert_eq!(rows, vec![("dev".to_owned(), "dev".to_owned())]);
    let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_records ORDER BY id")
        .fetch_all(&api.db.pool)
        .await
        .expect("audit");
    assert_eq!(actions, ["workspace.created", "namespace.created"]);
}

#[tokio::test]
async fn invalid_input_is_a_422_with_the_standard_envelope() {
    let api = start().await;
    api.tenant("acme", "alice").await;

    for bad_slug in ["Bad Slug", "ab", "-x-", "a--b", ""] {
        let response = api
            .post_as("/workspaces", "alice", "acme", &json!({"slug": bad_slug}))
            .await;
        assert_eq!(response.status(), 422, "{bad_slug:?}");
        let tid = trace_id(&response);
        let body = json_of(response).await;
        assert_eq!(body["code"], "VALIDATION_FAILED");
        assert_eq!(body["retryable"], false);
        assert_eq!(
            body["traceId"], tid,
            "body and header carry the same trace id"
        );
        assert!(body["message"].as_str().expect("message").contains("slug"));
    }
}

#[tokio::test]
async fn duplicates_are_a_409_and_slugs_are_scoped_to_the_tenant() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    api.create_namespace("alice", "acme", &ws, "dev", "dev")
        .await;

    let dup_ws = api
        .post_as("/workspaces", "alice", "acme", &json!({"slug": "platform"}))
        .await;
    assert_eq!(dup_ws.status(), 409);
    let body = json_of(dup_ws).await;
    assert_eq!(body["code"], "CONFLICT");
    assert_eq!(body["details"], json!({"field": "slug"}));

    let dup_ns = api
        .post_as(
            &format!("/workspaces/{ws}/namespaces"),
            "alice",
            "acme",
            &json!({"slug": "dev", "environment": "prod"}),
        )
        .await;
    assert_eq!(dup_ns.status(), 409);

    // The same names are free in another tenant.
    api.create_workspace("bob", "globex", "platform").await;
}

#[tokio::test]
async fn roles_without_the_permission_get_a_403_that_names_it() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    // Grant alice's colleague the developer role directly in the database.
    let tenant_id: sqlx::types::Uuid =
        sqlx::query_scalar("SELECT id FROM tenants WHERE slug = 'acme'")
            .fetch_one(&api.db.pool)
            .await
            .expect("tenant");
    sqlx::query("INSERT INTO memberships (tenant_id, subject, role, created_at) VALUES ($1, 'dan', 'developer', now())")
        .bind(tenant_id)
        .execute(&api.db.pool)
        .await
        .expect("grant");

    let response = api
        .post_as("/workspaces", "dan", "acme", &json!({"slug": "mine"}))
        .await;

    assert_eq!(response.status(), 403);
    let body = json_of(response).await;
    assert_eq!(body["code"], "FORBIDDEN");
    assert_eq!(
        body["details"],
        json!({"requiredPermission": "workspace:create"})
    );
}

#[tokio::test]
async fn body_problems_are_reported_in_the_same_envelope() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    let token = api.idp.token("alice", "acme").build();
    let post = |content_type: &'static str, body: String| {
        api.request(Method::POST, "/workspaces")
            .bearer_auth(&token)
            .header("content-type", content_type)
            .body(body)
    };

    // Not JSON at all -> 400
    let r = post("application/json", "{not json".into())
        .send()
        .await
        .expect("send");
    assert_eq!(r.status(), 400);
    assert_eq!(json_of(r).await["code"], "BAD_REQUEST");

    // Wrong content type -> 415
    let r = post("text/plain", r#"{"slug":"platform"}"#.into())
        .send()
        .await
        .expect("send");
    assert_eq!(r.status(), 415);
    assert_eq!(json_of(r).await["code"], "BAD_REQUEST");

    // Unknown field (including a tenant id!) -> 422, never silently ignored
    let r = post(
        "application/json",
        r#"{"slug":"platform","tenantId":"x"}"#.into(),
    )
    .send()
    .await
    .expect("send");
    assert_eq!(r.status(), 422);
    assert_eq!(json_of(r).await["code"], "VALIDATION_FAILED");

    // Missing field -> 422
    let r = post("application/json", "{}".into())
        .send()
        .await
        .expect("send");
    assert_eq!(r.status(), 422);

    // Oversized body -> 413, nothing persisted
    let r = post(
        "application/json",
        format!(r#"{{"slug":"{}"}}"#, "a".repeat(200_000)),
    )
    .send()
    .await
    .expect("send");
    assert_eq!(r.status(), 413);
    assert_eq!(json_of(r).await["code"], "BAD_REQUEST");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces")
        .fetch_one(&api.db.pool)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn an_inbound_traceparent_is_continued_in_the_response_and_the_error_body() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    let wanted = "4bf92f3577b34da6a3ce929d0e0e4736";

    let ok = api
        .as_user(Method::GET, "/auth/context", "alice", "acme")
        .header("traceparent", format!("00-{wanted}-00f067aa0ba902b7-01"))
        .send()
        .await
        .expect("send");
    assert_eq!(trace_id(&ok), wanted);

    let missing = api
        .as_user(Method::GET, "/workspaces/not-a-uuid", "alice", "acme")
        .header("traceparent", format!("00-{wanted}-00f067aa0ba902b7-01"))
        .send()
        .await
        .expect("send");
    assert_eq!(missing.status(), 404);
    assert_eq!(json_of(missing).await["traceId"], wanted);
}

#[tokio::test]
async fn requests_that_take_too_long_get_a_retryable_408_envelope() {
    // Hold a table lock so the handler blocks, with a short request timeout.
    let api = start_with(Options {
        limits: sagat_api_http::HttpLimits {
            request_timeout: std::time::Duration::from_millis(400),
            ..sagat_api_http::HttpLimits::default()
        },
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    let mut blocker = api.db.pool.acquire().await.expect("conn");
    sqlx::query("BEGIN")
        .execute(&mut *blocker)
        .await
        .expect("begin");
    sqlx::query("LOCK TABLE workspaces IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("lock");

    let response = api.get_as("/workspaces", "alice", "acme").await;

    assert_eq!(response.status(), 408);
    let body = without_trace(json_of(response).await);
    assert_eq!(body["code"], "UNAVAILABLE");
    assert_eq!(body["retryable"], true);
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .expect("release");
}

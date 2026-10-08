//! US04 — Deny cross-tenant access (NFR-001-SEC, FR-001-007).
//!
//! Given a user of tenant A, when they request a resource of tenant B, then the API does not reveal
//! tenant B data and records the attempt.

use reqwest::Method;
use serde_json::json;
use sqlx::types::Uuid;

use crate::support::api::{TestApi, json_of, start};
use crate::{trace_id, without_trace};

struct Setup {
    api: TestApi,
    ws_b: String,
    ns_b: String,
}

async fn two_tenants() -> Setup {
    let api = start().await;
    api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    let ws_b = api
        .create_workspace("bob", "globex", "secret-workspace")
        .await;
    let ns_b = api
        .create_namespace("bob", "globex", &ws_b, "secret-namespace", "prod")
        .await;
    Setup { api, ws_b, ns_b }
}

#[tokio::test]
async fn another_tenants_resources_look_exactly_like_missing_ones() {
    let Setup { api, ws_b, ns_b } = two_tenants().await;
    let missing = Uuid::nil().to_string().replace('0', "7"); // a well-formed id that exists nowhere

    // Same question, three ways: tenant B's id, a non-existent id, and a malformed id.
    let mut answers = Vec::new();
    for id in [&ws_b, &missing, &"not-a-uuid".to_owned()] {
        let response = api
            .get_as(&format!("/workspaces/{id}"), "alice", "acme")
            .await;
        assert_eq!(response.status(), 404, "{id}");
        trace_id(&response);
        answers.push(without_trace(json_of(response).await));
    }
    assert_eq!(
        answers[0], answers[1],
        "tenant B's workspace is indistinguishable from a missing one"
    );
    assert_eq!(answers[1], answers[2]);
    assert_eq!(answers[0]["code"], "NOT_FOUND");
    let rendered = answers[0].to_string();
    assert!(
        !rendered.contains("secret") && !rendered.contains("globex"),
        "{rendered}"
    );

    for path in [
        format!("/namespaces/{ns_b}"),
        format!("/namespaces/{missing}"),
    ] {
        assert_eq!(
            api.get_as(&path, "alice", "acme").await.status(),
            404,
            "{path}"
        );
    }
}

#[tokio::test]
async fn nothing_of_tenant_b_leaks_through_lists_or_writes() {
    let Setup { api, ws_b, .. } = two_tenants().await;

    for path in ["/workspaces", "/namespaces"] {
        let page = json_of(api.get_as(path, "alice", "acme").await).await;
        assert_eq!(page["items"], json!([]), "{path} must not include tenant B");
    }

    // Writing into tenant B's workspace is the same 404.
    let response = api
        .post_as(
            &format!("/workspaces/{ws_b}/namespaces"),
            "alice",
            "acme",
            &json!({"slug": "sneaky", "environment": "dev"}),
        )
        .await;
    assert_eq!(response.status(), 404);

    let created: i64 = sqlx::query_scalar("SELECT count(*) FROM namespaces WHERE slug = 'sneaky'")
        .fetch_one(&api.db.pool)
        .await
        .expect("count");
    assert_eq!(created, 0, "nothing was written");
}

#[tokio::test]
async fn each_attempt_is_audited_in_the_actors_tenant_and_correlates_with_its_trace_id() {
    let Setup { api, ws_b, ns_b } = two_tenants().await;

    let mut expected: Vec<(String, String)> = Vec::new(); // (trace id, resource id)
    for path in [format!("/workspaces/{ws_b}"), format!("/namespaces/{ns_b}")] {
        let response = api.get_as(&path, "alice", "acme").await;
        assert_eq!(response.status(), 404);
        expected.push((
            trace_id(&response),
            path.rsplit('/').next().expect("id").to_owned(),
        ));
    }
    // A plain miss is NOT a security event and must not be recorded.
    let nobody = api
        .get_as(
            &format!("/workspaces/{}", Uuid::nil().to_string().replace('0', "7")),
            "alice",
            "acme",
        )
        .await;
    assert_eq!(nobody.status(), 404);

    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT a.action, a.actor, a.resource_id, a.trace_id, t.slug \
         FROM audit_records a JOIN tenants t ON t.id = a.tenant_id \
         WHERE a.action = 'access.denied.cross_tenant' ORDER BY a.id",
    )
    .fetch_all(&api.db.pool)
    .await
    .expect("audit");

    assert_eq!(rows.len(), 2, "exactly the two real attempts");
    for ((action, actor, resource_id, trace, tenant), (want_trace, want_resource)) in
        rows.iter().zip(&expected)
    {
        assert_eq!(action, "access.denied.cross_tenant");
        assert_eq!(actor, "alice");
        assert_eq!(
            tenant, "acme",
            "recorded in the ACTOR's tenant, not the victim's"
        );
        assert_eq!(
            resource_id, want_resource,
            "the id alice sent, nothing else"
        );
        assert_eq!(
            trace, want_trace,
            "the audit row can be joined to the response by trace id"
        );
    }

    // Tenant B's trail has only its own two creations.
    let b_actions: Vec<String> = sqlx::query_scalar(
        "SELECT a.action FROM audit_records a JOIN tenants t ON t.id = a.tenant_id WHERE t.slug = 'globex' ORDER BY a.id",
    )
    .fetch_all(&api.db.pool)
    .await
    .expect("audit b");
    assert_eq!(b_actions, ["workspace.created", "namespace.created"]);
}

#[tokio::test]
async fn claiming_another_tenant_does_not_grant_access_to_it() {
    let Setup { api, ws_b, .. } = two_tenants().await;

    // Alice's own, validly signed token but with the tenant claim set to globex: she is no member.
    let spoofed = api
        .get_as(&format!("/workspaces/{ws_b}"), "alice", "globex")
        .await;
    assert_eq!(spoofed.status(), 401);

    // Bob (a member of globex) cannot act inside acme by claiming it either.
    let reverse = api.get_as("/workspaces", "bob", "acme").await;
    assert_eq!(reverse.status(), 401);
}

#[tokio::test]
async fn a_tenant_supplied_by_the_client_is_never_honoured() {
    let Setup { api, ws_b, .. } = two_tenants().await;

    // Header and query attempts are ignored: the answer is still scoped to alice's tenant.
    let response = api
        .as_user(
            Method::GET,
            "/workspaces?tenant=globex&tenantId=globex",
            "alice",
            "acme",
        )
        .header("x-tenant-id", "globex")
        .header("x-octopus-tenant", "globex")
        .send()
        .await
        .expect("send");
    // Unknown query parameters are rejected rather than silently ignored.
    assert_eq!(response.status(), 400);

    let ignored_header = api
        .as_user(Method::GET, "/workspaces", "alice", "acme")
        .header("x-tenant-id", "globex")
        .send()
        .await
        .expect("send");
    assert_eq!(ignored_header.status(), 200);
    assert_eq!(
        json_of(ignored_header).await["items"],
        json!([]),
        "header has no effect"
    );

    // A body that tries to name a tenant is rejected by the schema.
    let body = api
        .post_as(
            "/workspaces",
            "alice",
            "acme",
            &json!({"slug": "mine", "tenantId": ws_b}),
        )
        .await;
    assert_eq!(body.status(), 422);
}

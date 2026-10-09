//! Live conformance of the operations marked `x-implemented: '009'`.

use std::collections::BTreeSet;

use sagat_domain::identity::{Membership, Role};
use serde_json::json;

use crate::support::api::start;
use crate::support::contract::{As, OpenApi, check, scenario};
use crate::support::{now, subject};

async fn grant(
    api: &crate::support::api::TestApi,
    tenant: sagat_domain::identity::TenantId,
    sub: &str,
    role: Role,
) {
    api.owner_store
        .insert_membership(&Membership::restore(tenant, subject(sub), role, now()))
        .await
        .expect("grant");
}

#[tokio::test]
async fn every_governance_operation_conforms_to_the_contract_on_its_normal_and_error_paths() {
    let spec = OpenApi::load();
    let api = start().await;
    let acme = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    grant(&api, acme, "vera", Role::Viewer).await;
    grant(&api, acme, "dave", Role::Developer).await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    let ns = api
        .create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;
    let globex_ws = api.create_workspace("bob", "globex", "platform").await;
    let globex_ns = api
        .create_namespace("bob", "globex", &globex_ws, "payments", "dev")
        .await;

    let alice = As::User("alice", "acme");
    let vera = As::User("vera", "acme");
    let dave = As::User("dave", "acme");
    let bob = As::User("bob", "globex");
    let missing = "019b5d6e-0000-7000-8000-000000000000";

    let tenant_t = "/quotas";
    let tenant_one_t = "/quotas/{resource}";
    let ns_t = "/namespaces/{namespaceId}/quotas";
    let ns_one_t = "/namespaces/{namespaceId}/quotas/{resource}";
    let ns_path = format!("/namespaces/{ns}/quotas");
    let ns_one = |r: &str| format!("/namespaces/{ns}/quotas/{r}");

    let scenarios = vec![
        // ---- the tenant level
        scenario("GET", tenant_t, "/quotas", alice.clone(), None, 200),
        scenario("GET", tenant_t, "/quotas", vera.clone(), None, 200),
        scenario("GET", tenant_t, "/quotas", As::Anonymous, None, 401),
        scenario("GET", tenant_t, "/quotas", As::Garbage, None, 401),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            alice.clone(),
            Some(json!({"limit": 40})),
            200,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/worker_sessions",
            alice.clone(),
            Some(json!({"limit": 8})),
            200,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            dave.clone(),
            Some(json!({"limit": 40})),
            403,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            vera.clone(),
            Some(json!({"limit": 40})),
            403,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            As::Anonymous,
            Some(json!({"limit": 40})),
            401,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/builds",
            alice.clone(),
            Some(json!({"limit": 40})),
            404,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            alice.clone(),
            Some(json!({"limit": 1_000_000_001_u64})),
            422,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            alice.clone(),
            Some(json!({"limit": -1})),
            422,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            alice.clone(),
            Some(json!({"limit": 5, "extra": true})),
            422,
        ),
        scenario(
            "DELETE",
            tenant_one_t,
            "/quotas/worker_sessions",
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "DELETE",
            tenant_one_t,
            "/quotas/worker_sessions",
            dave.clone(),
            None,
            403,
        ),
        // ---- the namespace level
        scenario("GET", ns_t, &ns_path, alice.clone(), None, 200),
        scenario("GET", ns_t, &ns_path, dave.clone(), None, 200),
        scenario("GET", ns_t, &ns_path, As::Anonymous, None, 401),
        scenario(
            "GET",
            ns_t,
            &format!("/namespaces/{missing}/quotas"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            ns_t,
            "/namespaces/not-a-uuid/quotas",
            alice.clone(),
            None,
            404,
        ),
        // a namespace of another tenant is exactly as absent as one that does not exist
        scenario("GET", ns_t, &ns_path, bob.clone(), None, 404),
        scenario(
            "GET",
            ns_t,
            &format!("/namespaces/{globex_ns}/quotas"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "PUT",
            ns_one_t,
            &ns_one("active_runs"),
            alice.clone(),
            Some(json!({"limit": 25})),
            200,
        ),
        scenario(
            "PUT",
            ns_one_t,
            &ns_one("active_runs"),
            bob.clone(),
            Some(json!({"limit": 25})),
            404,
        ),
        scenario(
            "PUT",
            ns_one_t,
            &ns_one("active_runs"),
            vera.clone(),
            Some(json!({"limit": 25})),
            403,
        ),
        scenario(
            "PUT",
            ns_one_t,
            &ns_one("worker_sessions"),
            alice.clone(),
            Some(json!({"limit": 3})),
            422,
        ),
        scenario(
            "PUT",
            ns_one_t,
            &ns_one("unknown"),
            alice.clone(),
            Some(json!({"limit": 3})),
            404,
        ),
        scenario(
            "DELETE",
            ns_one_t,
            &ns_one("active_runs"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "DELETE",
            ns_one_t,
            &ns_one("active_runs"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "DELETE",
            ns_one_t,
            &ns_one("active_runs"),
            dave.clone(),
            None,
            403,
        ),
    ];

    let mut covered = BTreeSet::new();
    for s in &scenarios {
        let (method, template, _) = check(&api, &spec, s).await;
        covered.insert((method, template));
    }

    // Every operation marked as implemented by this feature was exercised.
    let marked: BTreeSet<_> = spec
        .operations_marked("x-implemented", "009")
        .into_iter()
        .collect();
    assert!(
        !marked.is_empty(),
        "the contract marks the governance operations"
    );
    let untested: Vec<_> = marked.difference(&covered).collect();
    assert!(
        untested.is_empty(),
        "operations without a conformance scenario: {untested:?}"
    );
}

#[tokio::test]
async fn the_quota_endpoints_show_what_the_engine_enforces() {
    let api = start().await;
    api.tenant("acme", "alice").await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    let ns = api
        .create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;

    let put = |path: String, limit: u64| {
        let request = api
            .as_user(reqwest::Method::PUT, &path, "alice", "acme")
            .json(&json!({"limit": limit}));
        async move { request.send().await.expect("send") }
    };
    let response = put(format!("/namespaces/{ns}/quotas/workflows"), 1).await;
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["limit"], 1);
    assert_eq!(body["source"], "namespace");
    assert_eq!(body["used"], 0);

    // The quota is real: the second workflow of the namespace is refused with the documented code.
    let create = |name: &str| {
        api.as_user(reqwest::Method::POST, "/workflows", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", format!("key-{name}-0001"))
            .json(&json!({"name": name, "ir": {
                "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
                "metadata": {"name": name}, "entrypoint": "a",
                "nodes": [{"id": "a", "type": "activity", "activity": "act.a"}]
            }}))
    };
    assert_eq!(create("first").send().await.expect("send").status(), 201);
    let refused = create("second").send().await.expect("send");
    assert_eq!(refused.status(), 429);
    let body: serde_json::Value = refused.json().await.expect("json");
    assert_eq!(body["code"], "QUOTA_EXCEEDED");
    assert_eq!(
        body["retryable"], false,
        "waiting does not help; raising the quota does"
    );
    assert!(
        !body.to_string().contains("first"),
        "the refusal names the limit, not the data: {body}"
    );

    let listed: serde_json::Value = api
        .get_as(&format!("/namespaces/{ns}/quotas"), "alice", "acme")
        .await
        .json()
        .await
        .expect("json");
    let workflows = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|q| q["resource"] == "workflows")
        .expect("workflows");
    assert_eq!(
        (workflows["limit"].as_u64(), workflows["used"].as_u64()),
        (Some(1), Some(1))
    );
}

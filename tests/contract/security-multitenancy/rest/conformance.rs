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
    let activity_t = "/namespaces/{namespaceId}/activity-quotas";
    let activity_one_t = "/namespaces/{namespaceId}/activity-quotas/{activity}";
    let activity_path = format!("/namespaces/{ns}/activity-quotas");
    let activity_one = |activity: &str| format!("/namespaces/{ns}/activity-quotas/{activity}");
    let bind_t = "/namespaces/{namespaceId}/worker-bindings";
    let bind_one_t = "/namespaces/{namespaceId}/worker-bindings/{subject}";
    let bind_path = format!("/namespaces/{ns}/worker-bindings");
    let bind_one = |subject: &str| format!("/namespaces/{ns}/worker-bindings/{subject}");
    let egress_t = "/namespaces/{namespaceId}/egress-policy";
    let egress_path = format!("/namespaces/{ns}/egress-policy");
    let sec_t = "/namespaces/{namespaceId}/secrets";
    let sec_one_t = "/namespaces/{namespaceId}/secrets/{secretName}";
    let sec_access_t = "/namespaces/{namespaceId}/secrets/{secretName}/access";
    let sec_pools_t = "/namespaces/{namespaceId}/secrets/{secretName}/pools";
    let sec_grant_t = "/namespaces/{namespaceId}/secrets/{secretName}/access/{subject}";
    let sec_path = format!("/namespaces/{ns}/secrets");
    let sec_one = |name: &str| format!("/namespaces/{ns}/secrets/{name}");
    let sec_access = |name: &str| format!("/namespaces/{ns}/secrets/{name}/access");
    let sec_pools = |name: &str| format!("/namespaces/{ns}/secrets/{name}/pools");
    let sec_grant = |name: &str, who: &str| format!("/namespaces/{ns}/secrets/{name}/access/{who}");
    let incident_t = "/script-runner/incidents";

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
        // The ceiling the platform grants the tenant (the default of `workflows` is 1000): the
        // value at it is accepted, one above it is not, and neither is the largest a limit can be.
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            alice.clone(),
            Some(json!({"limit": 1000})),
            200,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            alice.clone(),
            Some(json!({"limit": 1001})),
            422,
        ),
        scenario(
            "PUT",
            tenant_one_t,
            "/quotas/workflows",
            alice.clone(),
            Some(json!({"limit": 1_000_000_000})),
            422,
        ),
        scenario(
            "PUT",
            ns_one_t,
            &ns_one("active_runs"),
            alice.clone(),
            Some(json!({"limit": 10_001})),
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
        // ---- activity-specific concurrency
        scenario("GET", activity_t, &activity_path, alice.clone(), None, 200),
        scenario("GET", activity_t, &activity_path, dave.clone(), None, 200),
        scenario("GET", activity_t, &activity_path, bob.clone(), None, 404),
        scenario("GET", activity_t, &activity_path, As::Anonymous, None, 401),
        scenario(
            "PUT",
            activity_one_t,
            &activity_one("payment.capture"),
            alice.clone(),
            Some(json!({"limit": 2})),
            200,
        ),
        scenario(
            "PUT",
            activity_one_t,
            &activity_one("payment.capture"),
            vera.clone(),
            Some(json!({"limit": 2})),
            403,
        ),
        scenario(
            "PUT",
            activity_one_t,
            &activity_one("bad%20activity"),
            alice.clone(),
            Some(json!({"limit": 2})),
            404,
        ),
        scenario(
            "PUT",
            activity_one_t,
            &activity_one("payment.capture"),
            alice.clone(),
            Some(json!({"limit": 1_000_000_001_u64})),
            422,
        ),
        scenario(
            "DELETE",
            activity_one_t,
            &activity_one("payment.capture"),
            dave.clone(),
            None,
            403,
        ),
        scenario(
            "DELETE",
            activity_one_t,
            &activity_one("payment.capture"),
            alice.clone(),
            None,
            200,
        ),
        // ---- worker bindings
        scenario("GET", bind_t, &bind_path, alice.clone(), None, 200),
        scenario("GET", bind_t, &bind_path, dave.clone(), None, 403),
        scenario("GET", bind_t, &bind_path, As::Anonymous, None, 401),
        scenario("GET", bind_t, &bind_path, bob.clone(), None, 404),
        scenario(
            "GET",
            bind_t,
            &format!("/namespaces/{missing}/worker-bindings"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            Some(json!({"activities": ["payments.*", "audit.log"]})),
            200,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            Some(json!({"activities": ["payments.*"], "pools": ["trusted", "gpu"]})),
            200,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            Some(json!({"activities": ["payments.*"], "pools": []})),
            200,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            Some(json!({"activities": ["payments.*"], "pools": ["has space"]})),
            422,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            Some(json!({"activities": ["payments.*"], "pools": "trusted"})),
            422,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            dave.clone(),
            Some(json!({"activities": ["*"], "pools": ["trusted"]})),
            403,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            dave.clone(),
            Some(json!({"activities": ["*"]})),
            403,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            bob.clone(),
            Some(json!({"activities": ["*"]})),
            404,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            Some(json!({"activities": []})),
            422,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            Some(json!({"activities": ["has space"]})),
            422,
        ),
        scenario(
            "PUT",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            Some(json!({"activities": ["*"], "extra": 1})),
            422,
        ),
        scenario("GET", bind_t, &bind_path, vera.clone(), None, 403),
        scenario(
            "DELETE",
            bind_one_t,
            &bind_one("worker-7"),
            dave.clone(),
            None,
            403,
        ),
        scenario(
            "DELETE",
            bind_one_t,
            &bind_one("worker-7"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "DELETE",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "DELETE",
            bind_one_t,
            &bind_one("worker-7"),
            alice.clone(),
            None,
            404,
        ),
        // ---- secrets
        scenario("GET", sec_t, &sec_path, alice.clone(), None, 200),
        scenario("GET", sec_t, &sec_path, dave.clone(), None, 200),
        scenario("GET", sec_t, &sec_path, vera.clone(), None, 403),
        scenario("GET", sec_t, &sec_path, As::Anonymous, None, 401),
        scenario("GET", sec_t, &sec_path, bob.clone(), None, 404),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            Some(json!({"value": "sk-live-conformance-0001"})),
            200,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            Some(json!({"value": "sk-live-conformance-0002"})),
            200,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            Some(json!({"value": "sk-live-conformance-0003", "pools": ["trusted"]})),
            200,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            Some(json!({"value": "sk-live-conformance-0004", "pools": ["has space"]})),
            422,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            Some(json!({"value": "sk-live-conformance-0005", "pools": []})),
            200,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            dave.clone(),
            Some(json!({"value": "x-value-from-a-developer"})),
            403,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            bob.clone(),
            Some(json!({"value": "x-value-from-another-tenant"})),
            404,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("Stripe-Key"),
            alice.clone(),
            Some(json!({"value": "some-value-0001"})),
            422,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            Some(json!({"value": ""})),
            422,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            Some(json!({"value": "ok-value-0001", "extra": 1})),
            422,
        ),
        scenario(
            "PUT",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            Some(json!({})),
            422,
        ),
        scenario(
            "GET",
            sec_access_t,
            &sec_access("stripe-key"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            sec_access_t,
            &sec_access("stripe-key"),
            dave.clone(),
            None,
            403,
        ),
        scenario(
            "GET",
            sec_access_t,
            &sec_access("stripe-key"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            sec_access_t,
            &sec_access("missing-key"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "PUT",
            sec_grant_t,
            &sec_grant("stripe-key", "dave"),
            alice.clone(),
            None,
            204,
        ),
        scenario(
            "PUT",
            sec_grant_t,
            &sec_grant("stripe-key", "dave"),
            dave.clone(),
            None,
            403,
        ),
        scenario(
            "PUT",
            sec_grant_t,
            &sec_grant("stripe-key", "dave"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "PUT",
            sec_grant_t,
            &sec_grant("missing-key", "dave"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "DELETE",
            sec_grant_t,
            &sec_grant("stripe-key", "dave"),
            dave.clone(),
            None,
            403,
        ),
        scenario(
            "DELETE",
            sec_grant_t,
            &sec_grant("stripe-key", "dave"),
            alice.clone(),
            None,
            204,
        ),
        scenario(
            "DELETE",
            sec_grant_t,
            &sec_grant("stripe-key", "dave"),
            alice.clone(),
            None,
            404,
        ),
        // ---- the worker pools allowed to receive a secret
        scenario(
            "GET",
            sec_pools_t,
            &sec_pools("stripe-key"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "PUT",
            sec_pools_t,
            &sec_pools("stripe-key"),
            alice.clone(),
            Some(json!({"pools": ["trusted", "gpu"]})),
            200,
        ),
        scenario(
            "GET",
            sec_pools_t,
            &sec_pools("stripe-key"),
            alice.clone(),
            None,
            200,
        ),
        scenario("GET", sec_t, &sec_path, alice.clone(), None, 200),
        scenario(
            "GET",
            sec_pools_t,
            &sec_pools("stripe-key"),
            dave.clone(),
            None,
            403,
        ),
        scenario(
            "GET",
            sec_pools_t,
            &sec_pools("stripe-key"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            sec_pools_t,
            &sec_pools("missing-key"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "PUT",
            sec_pools_t,
            &sec_pools("stripe-key"),
            dave.clone(),
            Some(json!({"pools": ["x"]})),
            403,
        ),
        scenario(
            "PUT",
            sec_pools_t,
            &sec_pools("stripe-key"),
            bob.clone(),
            Some(json!({"pools": ["x"]})),
            404,
        ),
        scenario(
            "PUT",
            sec_pools_t,
            &sec_pools("missing-key"),
            alice.clone(),
            Some(json!({"pools": ["x"]})),
            404,
        ),
        scenario(
            "PUT",
            sec_pools_t,
            &sec_pools("stripe-key"),
            alice.clone(),
            Some(json!({"pools": ["has space"]})),
            422,
        ),
        scenario(
            "PUT",
            sec_pools_t,
            &sec_pools("stripe-key"),
            alice.clone(),
            Some(json!({"pools": ["x"], "extra": 1})),
            422,
        ),
        scenario(
            "PUT",
            sec_pools_t,
            &sec_pools("stripe-key"),
            alice.clone(),
            Some(json!({})),
            422,
        ),
        scenario(
            "PUT",
            sec_pools_t,
            &sec_pools("stripe-key"),
            As::Anonymous,
            Some(json!({"pools": []})),
            401,
        ),
        scenario(
            "DELETE",
            sec_one_t,
            &sec_one("stripe-key"),
            dave.clone(),
            None,
            403,
        ),
        scenario(
            "DELETE",
            sec_one_t,
            &sec_one("stripe-key"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "DELETE",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            None,
            204,
        ),
        scenario(
            "DELETE",
            sec_one_t,
            &sec_one("stripe-key"),
            alice.clone(),
            None,
            404,
        ),
        // ---- egress policy
        scenario("GET", egress_t, &egress_path, alice.clone(), None, 200),
        scenario("GET", egress_t, &egress_path, dave.clone(), None, 403),
        scenario("GET", egress_t, &egress_path, As::Anonymous, None, 401),
        scenario("GET", egress_t, &egress_path, bob.clone(), None, 404),
        scenario(
            "PUT",
            egress_t,
            &egress_path,
            alice.clone(),
            Some(
                json!({"rules": ["API.example.com", "*.partner.example", "203.0.113.0/24"], "ports": [8443, 443], "allowHttp": false}),
            ),
            200,
        ),
        scenario(
            "PUT",
            egress_t,
            &egress_path,
            alice.clone(),
            Some(json!({"rules": []})),
            200,
        ),
        scenario(
            "PUT",
            egress_t,
            &egress_path,
            dave.clone(),
            Some(json!({"rules": ["a.example.com"]})),
            403,
        ),
        scenario(
            "PUT",
            egress_t,
            &egress_path,
            bob.clone(),
            Some(json!({"rules": ["a.example.com"]})),
            404,
        ),
        scenario(
            "PUT",
            egress_t,
            &egress_path,
            alice.clone(),
            Some(json!({"rules": ["not a host!"]})),
            422,
        ),
        scenario(
            "PUT",
            egress_t,
            &egress_path,
            alice.clone(),
            Some(json!({"rules": ["a.example.com"], "ports": [0]})),
            422,
        ),
        scenario(
            "PUT",
            egress_t,
            &egress_path,
            alice.clone(),
            Some(json!({"rules": ["a.example.com"], "ports": [-1]})),
            422,
        ),
        scenario(
            "PUT",
            egress_t,
            &egress_path,
            alice.clone(),
            Some(json!({"rules": ["a.example.com"], "extra": true})),
            422,
        ),
        scenario("DELETE", egress_t, &egress_path, dave.clone(), None, 403),
        scenario("DELETE", egress_t, &egress_path, bob.clone(), None, 404),
        scenario("DELETE", egress_t, &egress_path, alice.clone(), None, 204),
        scenario("DELETE", egress_t, &egress_path, alice.clone(), None, 204),
        // ---- runner incidents: authentication is checked before the fenced target is resolved.
        // Stateful success and stale-fence paths use real leases in the integration suite.
        scenario(
            "POST",
            incident_t,
            incident_t,
            As::Anonymous,
            Some(json!({
                "target": {
                    "kind": "production",
                    "namespaceId": ns,
                    "runId": missing,
                    "stepId": missing,
                    "attempt": 1,
                    "generation": 1,
                    "leaseToken": "not-a-real-token"
                },
                "category": "NOT_ALLOWLISTED"
            })),
            401,
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
    assert_eq!(
        body["ceiling"], 1000,
        "the view says how far the tenant may go"
    );

    // Beyond that the tenant's own administrator cannot go, and nothing is stored when refused.
    let over = put(format!("/namespaces/{ns}/quotas/workflows"), 1001).await;
    assert_eq!(over.status(), 422);
    let refused: serde_json::Value = over.json().await.expect("json");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|m| m.contains("ceiling")),
        "{refused}"
    );
    let unchanged: serde_json::Value = api
        .get_as(&format!("/namespaces/{ns}/quotas"), "alice", "acme")
        .await
        .json()
        .await
        .expect("json");
    let workflows = unchanged["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|q| q["resource"] == "workflows")
        .expect("workflows");
    assert_eq!(workflows["limit"], 1, "the refused value was not stored");

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

    // An activity override is a separate resource. Clearing it returns the generic
    // activity_concurrency value that the same namespace inherits.
    assert_eq!(
        put(format!("/namespaces/{ns}/quotas/activity_concurrency"), 5,)
            .await
            .status(),
        200
    );
    let override_response = put(
        format!("/namespaces/{ns}/activity-quotas/payment.capture"),
        2,
    )
    .await;
    assert_eq!(override_response.status(), 200);
    let override_body: serde_json::Value = override_response.json().await.expect("json");
    assert_eq!(override_body["activity"], "payment.capture");
    assert_eq!(override_body["limit"], 2);
    assert_eq!(override_body["used"], 0);
    assert_eq!(override_body["overridden"], true);

    let activity_list: serde_json::Value = api
        .get_as(
            &format!("/namespaces/{ns}/activity-quotas"),
            "alice",
            "acme",
        )
        .await
        .json()
        .await
        .expect("activity quotas");
    assert_eq!(activity_list["items"], json!([override_body]));

    let cleared = api
        .as_user(
            reqwest::Method::DELETE,
            &format!("/namespaces/{ns}/activity-quotas/payment.capture"),
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("clear activity quota");
    assert_eq!(cleared.status(), 200);
    let cleared: serde_json::Value = cleared.json().await.expect("json");
    assert_eq!(cleared["limit"], 5);
    assert_eq!(cleared["overridden"], false);
}

#[tokio::test]
async fn no_secret_operation_ever_returns_the_value() {
    const VALUE: &str = "sk-live-canary-7f3a9c1e5b2d";
    let api = start().await;
    api.tenant("acme", "alice").await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    let ns = api
        .create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;

    let put = api
        .as_user(
            reqwest::Method::PUT,
            &format!("/namespaces/{ns}/secrets/stripe-key"),
            "alice",
            "acme",
        )
        .json(&json!({"value": VALUE}))
        .send()
        .await
        .expect("send");
    assert_eq!(put.status(), 200);
    let mut seen = vec![put.text().await.expect("body")];
    for path in [
        format!("/namespaces/{ns}/secrets"),
        format!("/namespaces/{ns}/secrets/stripe-key/access"),
    ] {
        let response = api.get_as(&path, "alice", "acme").await;
        assert_eq!(response.status(), 200);
        seen.push(response.text().await.expect("body"));
    }
    // A refused attempt echoes nothing either: wrong role, malformed name, unknown field.
    api.tenant("globex", "bob").await;
    for (who, tenant, path, body) in [
        (
            "bob",
            "globex",
            format!("/namespaces/{ns}/secrets/stripe-key"),
            json!({"value": VALUE}),
        ),
        (
            "alice",
            "acme",
            format!("/namespaces/{ns}/secrets/Bad-Name"),
            json!({"value": VALUE}),
        ),
        (
            "alice",
            "acme",
            format!("/namespaces/{ns}/secrets/stripe-key"),
            json!({"value": VALUE, "extra": 1}),
        ),
    ] {
        let response = api
            .as_user(reqwest::Method::PUT, &path, who, tenant)
            .json(&body)
            .send()
            .await
            .expect("send");
        assert!(response.status().is_client_error());
        seen.push(response.text().await.expect("body"));
    }
    for body in &seen {
        assert!(
            !body.contains(VALUE),
            "a response carried the value: {body}"
        );
    }
}

//! Live conformance of the operations marked `x-implemented: '002'`.

use std::collections::BTreeSet;
use std::time::Duration;

use reqwest::Method;
use sagat_application::blobs::BlobLimits;
use sagat_domain::identity::{Membership, Role};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::api::{Options, TestApi, json_of, start, start_with};
use crate::support::contract::{As, OpenApi, check, scenario};
use crate::support::{now, subject};

fn chain_ir(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "a",
        "nodes": [
            {"id": "a", "type": "activity", "activity": "act.a", "next": "b"},
            {"id": "b", "type": "activity", "activity": "act.b"}
        ]
    })
}

fn waiting_ir(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "w",
        "nodes": [
            {"id": "w", "type": "waitEvent", "eventType": "payment.confirmed",
             "correlationExpr": "input.paymentId", "timeoutMs": 600000, "next": "done"},
            {"id": "done", "type": "activity", "activity": "act.done"}
        ]
    })
}

fn key() -> String {
    format!("key-{}", Uuid::now_v7())
}

/// A request as `sub` of acme, with the namespace and a fresh idempotency key.
fn call(
    api: &TestApi,
    method: Method,
    path: &str,
    sub: &str,
    body: Option<&Value>,
) -> reqwest::RequestBuilder {
    let mut request = api
        .as_user(method, path, sub, "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", key());
    if let Some(body) = body {
        request = request.json(body);
    }
    request
}

async fn send_json(request: reqwest::RequestBuilder) -> (u16, Value) {
    let response = request.send().await.expect("send");
    let status = response.status().as_u16();
    (status, json_of(response).await)
}

async fn grant(api: &TestApi, tenant: sagat_domain::identity::TenantId, sub: &str, role: Role) {
    api.owner_store
        .insert_membership(&Membership::restore(tenant, subject(sub), role, now()))
        .await
        .expect("grant");
}

#[tokio::test]
async fn every_execution_operation_conforms_to_the_contract_on_its_normal_and_error_paths() {
    let spec = OpenApi::load();
    let api = start_with(Options {
        blobs: Some(BlobLimits {
            max_bytes: 4 * 1024 * 1024,
            max_pending: 8,
            upload_ttl: Duration::from_secs(24 * 3600),
            abandoned_after: Duration::from_secs(3600),
        }),
        ..Options::default()
    })
    .await;
    let acme = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    grant(&api, acme, "vera", Role::Viewer).await;
    grant(&api, acme, "olga", Role::Operator).await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    api.create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;

    // Fixtures created through the API itself.
    let (status, chain) = send_json(call(
        &api,
        Method::POST,
        "/workflows",
        "alice",
        Some(&json!({"name": "chain-flow", "ir": chain_ir("chain-flow")})),
    ))
    .await;
    assert_eq!(status, 201, "{chain}");
    let chain_id = chain["id"].as_str().expect("id").to_owned();
    let (status, waiting) = send_json(call(
        &api,
        Method::POST,
        "/workflows",
        "alice",
        Some(&json!({"name": "wait-flow", "ir": waiting_ir("wait-flow")})),
    ))
    .await;
    assert_eq!(status, 201);
    let wait_id = waiting["id"].as_str().expect("id").to_owned();
    let (status, draft_only) = send_json(call(
        &api,
        Method::POST,
        "/workflows",
        "alice",
        Some(&json!({"name": "draft-only", "ir": chain_ir("draft-only")})),
    ))
    .await;
    assert_eq!(status, 201);
    let draft_only_id = draft_only["id"].as_str().expect("id").to_owned();
    for id in [&chain_id, &wait_id] {
        let (status, _) = send_json(call(
            &api,
            Method::POST,
            &format!("/workflows/{id}/publish"),
            "alice",
            Some(&json!({"draftRevision": 1})),
        ))
        .await;
        assert_eq!(status, 201);
    }
    let (status, run) = send_json(call(
        &api,
        Method::POST,
        "/runs",
        "alice",
        Some(&json!({"workflowId": chain_id, "input": {"n": 1}})),
    ))
    .await;
    assert_eq!(status, 202, "{run}");
    let run_id = run["runId"].as_str().expect("run id").to_owned();
    let (status, waiting_run) = send_json(call(
        &api,
        Method::POST,
        "/runs",
        "alice",
        Some(&json!({"workflowId": wait_id, "input": {"paymentId": "p-1"}})),
    ))
    .await;
    assert_eq!(status, 202);
    let waiting_run_id = waiting_run["runId"].as_str().expect("run id").to_owned();
    let missing = "019b5d6e-0000-7000-8000-000000000000";
    let alice = As::User("alice", "acme");
    let vera = As::User("vera", "acme");
    let olga = As::User("olga", "acme");
    let bob = As::User("bob", "globex");

    let ns =
        |s: crate::support::contract::Scenario| s.with_header("x-octopus-namespace", "payments");
    let idem = |s: crate::support::contract::Scenario| s.with_header("idempotency-key", key());
    let mutating = |s| idem(ns(s));

    let wf_t = "/workflows/{workflowId}";
    let versions_t = "/workflows/{workflowId}/versions";
    let version_t = "/workflows/{workflowId}/versions/{versionNumber}";
    let publish_t = "/workflows/{workflowId}/publish";
    let run_t = "/runs/{runId}";
    let ops_t = "/runs/{runId}/operations";
    let signal_t = "/runs/{runId}/signals/{signalName}";
    let upload_t = "/run-input-blobs";

    let scenarios = vec![
        // ---- workflows
        mutating(scenario(
            "POST",
            "/workflows",
            "/workflows",
            alice.clone(),
            Some(json!({"name": "second-flow", "ir": chain_ir("second-flow")})),
            201,
        )),
        ns(scenario(
            "POST",
            "/workflows",
            "/workflows",
            alice.clone(),
            Some(json!({"name": "no-key", "ir": chain_ir("no-key")})),
            400,
        )),
        mutating(scenario(
            "POST",
            "/workflows",
            "/workflows",
            alice.clone(),
            Some(json!({"name": "chain-flow", "ir": chain_ir("chain-flow")})),
            409,
        )),
        mutating(scenario(
            "POST",
            "/workflows",
            "/workflows",
            vera.clone(),
            Some(json!({"name": "viewer-flow", "ir": chain_ir("viewer-flow")})),
            403,
        )),
        mutating(scenario(
            "POST",
            "/workflows",
            "/workflows",
            alice.clone(),
            Some(json!({"name": "Bad Name", "ir": chain_ir("bad-name")})),
            422,
        )),
        mutating(scenario(
            "POST",
            "/workflows",
            "/workflows",
            alice.clone(),
            Some(json!({"name": "not-object", "ir": [1, 2]})),
            422,
        )),
        mutating(scenario(
            "POST",
            "/workflows",
            "/workflows",
            As::Anonymous,
            None,
            401,
        )),
        ns(scenario(
            "GET",
            "/workflows",
            "/workflows",
            alice.clone(),
            None,
            200,
        )),
        ns(scenario(
            "GET",
            "/workflows",
            "/workflows",
            vera.clone(),
            None,
            200,
        )),
        scenario("GET", "/workflows", "/workflows", alice.clone(), None, 400),
        scenario(
            "GET",
            wf_t,
            &format!("/workflows/{chain_id}"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            wf_t,
            &format!("/workflows/{missing}"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            wf_t,
            &format!("/workflows/{chain_id}"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "PUT",
            wf_t,
            &format!("/workflows/{draft_only_id}"),
            alice.clone(),
            Some(json!({"draftRevision": 1, "ir": chain_ir("draft-only")})),
            200,
        ),
        scenario(
            "PUT",
            wf_t,
            &format!("/workflows/{draft_only_id}"),
            alice.clone(),
            Some(json!({"draftRevision": 1, "ir": chain_ir("draft-only")})),
            409,
        ),
        scenario(
            "PUT",
            wf_t,
            &format!("/workflows/{missing}"),
            alice.clone(),
            Some(json!({"draftRevision": 1, "ir": chain_ir("x")})),
            404,
        ),
        scenario(
            "PUT",
            wf_t,
            &format!("/workflows/{draft_only_id}"),
            vera.clone(),
            Some(json!({"draftRevision": 2, "ir": chain_ir("x")})),
            403,
        ),
        scenario(
            "PUT",
            wf_t,
            &format!("/workflows/{draft_only_id}"),
            alice.clone(),
            Some(json!({"draftRevision": 2, "ir": "text"})),
            422,
        ),
        scenario(
            "PUT",
            wf_t,
            &format!("/workflows/{draft_only_id}"),
            alice.clone(),
            Some(json!({"draftRevision": 2})),
            422,
        ),
        // ---- publishing and versions
        mutating(scenario(
            "POST",
            publish_t,
            &format!("/workflows/{draft_only_id}/publish"),
            alice.clone(),
            Some(json!({"draftRevision": 1})),
            409,
        )),
        mutating(scenario(
            "POST",
            publish_t,
            &format!("/workflows/{missing}/publish"),
            alice.clone(),
            Some(json!({"draftRevision": 1})),
            404,
        )),
        mutating(scenario(
            "POST",
            publish_t,
            &format!("/workflows/{draft_only_id}/publish"),
            vera.clone(),
            Some(json!({"draftRevision": 2})),
            403,
        )),
        mutating(scenario(
            "POST",
            publish_t,
            &format!("/workflows/{draft_only_id}/publish"),
            alice.clone(),
            Some(json!({"draftRevision": 2})),
            201,
        )),
        scenario(
            "GET",
            versions_t,
            &format!("/workflows/{chain_id}/versions"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            versions_t,
            &format!("/workflows/{missing}/versions"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            version_t,
            &format!("/workflows/{chain_id}/versions/1"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            version_t,
            &format!("/workflows/{chain_id}/versions/99"),
            alice.clone(),
            None,
            404,
        ),
        // ---- runs
        mutating(scenario(
            "POST",
            "/runs",
            "/runs",
            alice.clone(),
            Some(json!({"workflowId": chain_id, "input": {"n": 2}})),
            202,
        )),
        mutating(scenario(
            "POST",
            "/runs",
            "/runs",
            olga.clone(),
            Some(json!({"workflowId": chain_id, "input": {}})),
            202,
        )),
        mutating(scenario(
            "POST",
            "/runs",
            "/runs",
            vera.clone(),
            Some(json!({"workflowId": chain_id, "input": {}})),
            403,
        )),
        mutating(scenario(
            "POST",
            "/runs",
            "/runs",
            alice.clone(),
            Some(json!({"workflowId": missing, "input": {}})),
            404,
        )),
        mutating(scenario(
            "POST",
            "/runs",
            "/runs",
            alice.clone(),
            Some(json!({"workflowId": chain_id, "input": [1]})),
            422,
        )),
        mutating(scenario(
            "POST",
            "/runs",
            "/runs",
            alice.clone(),
            Some(json!({"workflowId": chain_id, "input": {}, "unknown": 1})),
            422,
        )),
        mutating(scenario(
            "POST",
            "/runs",
            "/runs",
            alice.clone(),
            Some(json!({"workflowId": chain_id, "input": {}, "deadline": "2001-01-01T00:00:00Z"})),
            422,
        )),
        ns(scenario(
            "POST",
            "/runs",
            "/runs",
            alice.clone(),
            Some(json!({"workflowId": chain_id, "input": {}})),
            400,
        )),
        ns(scenario("GET", "/runs", "/runs", alice.clone(), None, 200)),
        ns(scenario(
            "GET",
            "/runs",
            "/runs?status=RUNNING",
            vera.clone(),
            None,
            200,
        )),
        ns(scenario(
            "GET",
            "/runs",
            "/runs?status=NOPE",
            alice.clone(),
            None,
            422,
        )),
        scenario(
            "GET",
            run_t,
            &format!("/runs/{run_id}"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            run_t,
            &format!("/runs/{run_id}"),
            vera.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            run_t,
            &format!("/runs/{run_id}"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            run_t,
            &format!("/runs/{missing}"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            "/runs/{runId}/events",
            &format!("/runs/{run_id}/events"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            "/runs/{runId}/events",
            &format!("/runs/{missing}/events"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            "/runs/{runId}/graph",
            &format!("/runs/{run_id}/graph"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            "/runs/{runId}/graph",
            &format!("/runs/{missing}/graph"),
            alice.clone(),
            None,
            404,
        ),
        // ---- streamed run input staging
        ns(scenario(
            "POST",
            upload_t,
            upload_t,
            alice.clone(),
            Some(json!({"large": "run input"})),
            201,
        )),
        ns(scenario(
            "POST",
            upload_t,
            upload_t,
            vera.clone(),
            Some(json!({"large": "forbidden"})),
            403,
        )),
        ns(scenario(
            "POST",
            upload_t,
            upload_t,
            alice.clone(),
            Some(json!({"large": "wrong digest"})),
            422,
        ))
        .with_header("x-octopus-blob-sha256", "0".repeat(64)),
        // ---- operator actions
        mutating(scenario(
            "POST",
            ops_t,
            &format!("/runs/{run_id}/operations"),
            olga.clone(),
            Some(json!({"action": "PAUSE", "reason": "maintenance window"})),
            202,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &format!("/runs/{run_id}/operations"),
            olga.clone(),
            Some(json!({"action": "PAUSE", "reason": "maintenance window"})),
            409,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &format!("/runs/{run_id}/operations"),
            olga.clone(),
            Some(json!({"action": "RESUME", "reason": "window is over"})),
            202,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &format!("/runs/{run_id}/operations"),
            vera.clone(),
            Some(json!({"action": "CANCEL", "reason": "I am a viewer"})),
            403,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &format!("/runs/{run_id}/operations"),
            olga.clone(),
            Some(json!({"action": "RETRY_STEP", "reason": "not built yet"})),
            422,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &format!("/runs/{run_id}/operations"),
            olga.clone(),
            Some(json!({"action": "CANCEL", "reason": "short"})),
            422,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &format!("/runs/{missing}/operations"),
            olga.clone(),
            Some(json!({"action": "CANCEL", "reason": "does not exist"})),
            404,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &format!("/runs/{run_id}/operations"),
            olga.clone(),
            Some(json!({"action": "CANCEL", "reason": "no longer needed"})),
            202,
        )),
        mutating(scenario(
            "POST",
            ops_t,
            &format!("/runs/{run_id}/operations"),
            olga.clone(),
            Some(json!({"action": "CANCEL", "reason": "already done?"})),
            409,
        )),
        // ---- signals
        mutating(scenario(
            "POST",
            signal_t,
            &format!("/runs/{waiting_run_id}/signals/payment.confirmed"),
            olga.clone(),
            Some(json!({"amount": 10})),
            202,
        )),
        mutating(scenario(
            "POST",
            signal_t,
            &format!("/runs/{waiting_run_id}/signals/payment.confirmed"),
            olga.clone(),
            Some(json!({"amount": 10})),
            409,
        )),
        mutating(scenario(
            "POST",
            signal_t,
            &format!("/runs/{waiting_run_id}/signals/payment.confirmed"),
            vera.clone(),
            Some(json!({})),
            403,
        )),
        mutating(scenario(
            "POST",
            signal_t,
            &format!("/runs/{missing}/signals/payment.confirmed"),
            olga.clone(),
            Some(json!({})),
            404,
        )),
    ];

    let mut exercised: BTreeSet<(String, String, u16)> = BTreeSet::new();
    for s in &scenarios {
        exercised.insert(check(&api, &spec, s).await);
    }

    // Every implemented operation has a successful scenario, and the contract marks the right set.
    let covered: BTreeSet<(String, String)> = exercised
        .iter()
        .filter(|(_, _, st)| (200..300).contains(st))
        .map(|(m, t, _)| (m.clone(), t.clone()))
        .collect();
    let implemented = spec.operations_marked("x-implemented", "002");
    assert!(
        implemented.len() >= 14,
        "the contract marks {} operations",
        implemented.len()
    );
    for (method, template) in implemented {
        assert!(
            covered.contains(&(method.clone(), template.clone())),
            "{method} {template} has no conformance scenario"
        );
    }
}

#[tokio::test]
async fn the_unauthenticated_and_cross_tenant_responses_leak_nothing_and_conform() {
    let spec = OpenApi::load();
    let api = start().await;
    api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    api.create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;
    let (_, created) = send_json(call(
        &api,
        Method::POST,
        "/workflows",
        "alice",
        Some(&json!({"name": "private-flow", "ir": chain_ir("private-flow")})),
    ))
    .await;
    let id = created["id"].as_str().expect("id");
    send_json(call(
        &api,
        Method::POST,
        &format!("/workflows/{id}/publish"),
        "alice",
        Some(&json!({"draftRevision": 1})),
    ))
    .await;

    for (method, template, path) in [
        ("GET", "/workflows", "/workflows".to_owned()),
        ("GET", "/runs", "/runs".to_owned()),
    ] {
        let s = scenario(method, template, &path, As::Anonymous, None, 401)
            .with_header("x-octopus-namespace", "payments");
        check(&api, &spec, &s).await;
        let garbage = scenario(method, template, &path, As::Garbage, None, 401)
            .with_header("x-octopus-namespace", "payments");
        check(&api, &spec, &garbage).await;
    }
    // Another tenant addressing the same namespace slug sees its own (empty) world, not acme's.
    let (status, body) = send_json(
        api.as_user(Method::GET, "/workflows", "bob", "globex")
            .header("x-octopus-namespace", "payments"),
    )
    .await;
    assert_eq!(
        status, 404,
        "globex has no namespace called payments: {body}"
    );
}

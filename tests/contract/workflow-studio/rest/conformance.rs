//! Live conformance of `validateWorkflowDraft`, `diffWorkflow`, `getRuntimeGraphChanges` and
//! `streamRunChanges`. The stream is open-ended, so its successful answer is played by the user
//! stories (`tests/e2e/workflow-studio`); here its refusals are held to the contract.

use std::collections::BTreeSet;

use sagat_domain::identity::Role;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::contract::{As, OpenApi, check, scenario};
use crate::support::worker::platform;

fn ir(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "a",
        "inputSchema": {"type": "object", "required": ["x"], "properties": {"x": {"type": "string"}}},
        "nodes": [
            {"id": "a", "type": "activity", "activity": "act.a", "next": "b"},
            {"id": "b", "type": "activity", "activity": "act.b"}
        ]
    })
}

fn key() -> String {
    format!("key-{}", Uuid::now_v7())
}

#[tokio::test]
async fn the_studio_operations_conform_to_the_contract_on_their_normal_and_error_paths() {
    let spec = OpenApi::load();
    let p = platform().await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    let wf = p.publish("order-flow", &ir("order-flow")).await;
    let run = p.start(&wf, &json!({"x": "1"})).await;

    let missing = "019b5d6e-0000-7000-8000-000000000000";
    let alice = As::User("alice", "acme");
    let vera = As::User("vera", "acme");
    let bob = As::User("bob", "globex");
    let ns =
        |s: crate::support::contract::Scenario| s.with_header("x-octopus-namespace", "payments");
    let validate_t = "/workflows/{workflowId}/validate";
    let diff_t = "/workflows/{workflowId}/diff";
    let changes_t = "/runs/{runId}/graph/changes";
    let stream_t = "/runs/{runId}/stream";
    let validate = format!("/workflows/{wf}/validate");
    let diff = format!("/workflows/{wf}/diff");
    let changes = |since: &str| format!("/runs/{run}/graph/changes{since}");
    let stream = format!("/runs/{run}/stream");

    let mut broken = ir("order-flow");
    broken["nodes"][0]["next"] = json!("nowhere");

    let scenarios = vec![
        // ---- validateWorkflowDraft
        ns(scenario(
            "POST",
            validate_t,
            &validate,
            alice.clone(),
            Some(json!({})),
            200,
        )),
        ns(scenario(
            "POST",
            validate_t,
            &validate,
            alice.clone(),
            Some(json!({"ir": broken})),
            200,
        )),
        ns(scenario(
            "POST",
            validate_t,
            &validate,
            alice.clone(),
            Some(json!({"ir": {"kind": "x"}})),
            200,
        )),
        ns(scenario(
            "POST",
            validate_t,
            &validate,
            alice.clone(),
            Some(json!({"nope": 1})),
            422,
        )),
        ns(scenario(
            "POST",
            validate_t,
            &validate,
            vera.clone(),
            Some(json!({})),
            403,
        )),
        ns(scenario(
            "POST",
            validate_t,
            &validate,
            As::Anonymous,
            Some(json!({})),
            401,
        )),
        ns(scenario(
            "POST",
            validate_t,
            &validate,
            As::Garbage,
            Some(json!({})),
            401,
        )),
        ns(scenario(
            "POST",
            validate_t,
            &validate,
            bob.clone(),
            Some(json!({})),
            404,
        )),
        ns(scenario(
            "POST",
            validate_t,
            &format!("/workflows/{missing}/validate"),
            alice.clone(),
            Some(json!({})),
            404,
        )),
        // ---- diffWorkflow
        ns(scenario("GET", diff_t, &diff, alice.clone(), None, 200)),
        ns(scenario("GET", diff_t, &diff, vera.clone(), None, 200)),
        ns(scenario(
            "GET",
            diff_t,
            &format!("{diff}?from=1&to=draft"),
            alice.clone(),
            None,
            200,
        )),
        ns(scenario(
            "GET",
            diff_t,
            &format!("{diff}?from=draft&to=draft"),
            alice.clone(),
            None,
            200,
        )),
        ns(scenario(
            "GET",
            diff_t,
            &format!("{diff}?from=latest"),
            alice.clone(),
            None,
            400,
        )),
        ns(scenario(
            "GET",
            diff_t,
            &format!("{diff}?from=9"),
            alice.clone(),
            None,
            404,
        )),
        ns(scenario("GET", diff_t, &diff, As::Anonymous, None, 401)),
        ns(scenario("GET", diff_t, &diff, bob.clone(), None, 404)),
        ns(scenario(
            "GET",
            diff_t,
            &format!("/workflows/{missing}/diff"),
            alice.clone(),
            None,
            404,
        )),
        // ---- getRuntimeGraphChanges
        ns(scenario(
            "GET",
            changes_t,
            &changes("?since=0"),
            alice.clone(),
            None,
            200,
        )),
        ns(scenario(
            "GET",
            changes_t,
            &changes("?since=1"),
            vera.clone(),
            None,
            200,
        )),
        ns(scenario(
            "GET",
            changes_t,
            &changes("?since=99999"),
            alice.clone(),
            None,
            200,
        )),
        ns(scenario(
            "GET",
            changes_t,
            &changes(""),
            alice.clone(),
            None,
            400,
        )),
        ns(scenario(
            "GET",
            changes_t,
            &changes("?since=abc"),
            alice.clone(),
            None,
            400,
        )),
        ns(scenario(
            "GET",
            changes_t,
            &changes("?since=0"),
            As::Anonymous,
            None,
            401,
        )),
        ns(scenario(
            "GET",
            changes_t,
            &changes("?since=0"),
            bob.clone(),
            None,
            404,
        )),
        ns(scenario(
            "GET",
            changes_t,
            &format!("/runs/{missing}/graph/changes?since=0"),
            alice.clone(),
            None,
            404,
        )),
        // ---- streamRunChanges: only the refusals (the answer never ends)
        ns(scenario("GET", stream_t, &stream, As::Anonymous, None, 401)),
        ns(scenario("GET", stream_t, &stream, As::Garbage, None, 401)),
        ns(scenario("GET", stream_t, &stream, bob.clone(), None, 404)),
        ns(scenario(
            "GET",
            stream_t,
            &format!("/runs/{missing}/stream"),
            alice.clone(),
            None,
            404,
        )),
        ns(scenario(
            "GET",
            stream_t,
            &format!("{stream}?after=abc"),
            alice.clone(),
            None,
            400,
        )),
    ];
    let _ = key();
    let mut exercised: BTreeSet<(String, String, u16)> = BTreeSet::new();
    for s in &scenarios {
        exercised.insert(check(&p.api, &spec, s).await);
    }

    // Everything the feature added is documented and exercised; all but the stream, successfully.
    let added = spec.operations_marked("x-implemented", "007");
    assert_eq!(added.len(), 4, "{added:?}");
    for (method, template) in &added {
        if template == stream_t {
            continue;
        }
        assert!(
            exercised
                .iter()
                .any(|(m, t, st)| m == method && t == template && *st == 200),
            "{method} {template} has no successful conformance scenario"
        );
    }
    assert!(
        exercised.iter().any(|(_, t, _)| t == stream_t),
        "the stream's refusals are in the contract"
    );
}

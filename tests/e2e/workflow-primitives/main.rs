//! Workflow-primitive public-interface scenarios against the real platform. A missing PostgreSQL, broker or
//! language toolchain is a test failure when a scenario requires it, never a successful substitute.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use support::api::{TestApi, json_of};

fn routing_workflow(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "choose",
        "nodes": [
            {
                "id": "choose",
                "type": "switch",
                "cases": [{"id": "positive", "when": "input.amount > 0", "target": "accepted"}],
                "default": "rejected"
            },
            {"id": "accepted", "type": "end", "outputExpr": "input.amount"},
            {"id": "rejected", "type": "end", "outputExpr": "input.amount"}
        ]
    })
}

async fn start(api: &TestApi, workflow: &str, input: Value) -> String {
    let started = api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": workflow, "input": input}))
        .send()
        .await
        .expect("start routed run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .to_owned()
}

#[tokio::test]
async fn public_routing_finishes_both_outcomes_and_fails_on_an_invalid_predicate() {
    let api = support::api::start().await;
    api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "orders").await;
    api.create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let name = format!("e2e-routing-{}", Uuid::now_v7());
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": routing_workflow(&name)}))
        .send()
        .await
        .expect("create routed workflow");
    assert_eq!(created.status(), StatusCode::CREATED);
    let workflow = json_of(created).await["id"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    let published = api
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
        .expect("publish routed workflow");
    assert_eq!(published.status(), StatusCode::CREATED);

    for (amount, selected) in [(4, "accepted"), (-1, "rejected")] {
        let run = start(&api, &workflow, json!({"amount": amount})).await;
        let response = api
            .as_user(Method::GET, &format!("/runs/{run}/graph"), "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .send()
            .await
            .expect("read routed run graph");
        assert_eq!(response.status(), StatusCode::OK);
        let graph = json_of(response).await;
        assert_eq!(graph["status"], "SUCCEEDED", "{graph}");
        assert!(
            graph["nodes"]
                .as_array()
                .expect("graph nodes")
                .iter()
                .any(|node| node["id"] == selected && node["status"] == "SUCCEEDED"),
            "selected outcome missing: {graph}"
        );
    }

    let failed_run = start(&api, &workflow, json!({})).await;
    let response = api
        .as_user(
            Method::GET,
            &format!("/runs/{failed_run}/graph"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .send()
        .await
        .expect("read failed run graph");
    assert_eq!(response.status(), StatusCode::OK);
    let graph = json_of(response).await;
    assert_eq!(graph["status"], "FAILED", "{graph}");
    assert!(
        graph["nodes"]
            .as_array()
            .expect("graph nodes")
            .iter()
            .all(|node| {
                node["id"] != "accepted" && node["id"] != "rejected"
                    || node["status"] == "NOT_STARTED"
            }),
        "predicate failure must not select an outcome: {graph}"
    );
}

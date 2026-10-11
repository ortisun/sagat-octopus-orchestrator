//! US02 — keep the old run: a run waiting for an event when a new version is published resumes with
//! the semantics of the version it started on, and a run started afterwards uses the new one
//! (US02-AC01).

use std::time::Duration;

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::worker::platform;
use crate::{Dir, cli, engine, rest_in, within, workflow_id};

const NS: &str = "payments";

/// Waits for an order to be paid, then ships with `ship`.
fn waiting(ship: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "checkout"}, "entrypoint": "pay",
        "nodes": [
            {"id": "pay", "type": "waitEvent", "eventType": "payment.confirmed",
             "correlationExpr": "input.orderId", "timeoutMs": 300000, "next": "ship"},
            {"id": "ship", "type": "activity", "activity": ship}
        ]
    })
}

async fn paid(p: &crate::support::worker::Platform, event: &str, order: &str) {
    let (status, body) = rest_in(
        p,
        "alice",
        NS,
        Method::POST,
        "/events",
        Some(&json!({"eventId": event, "type": "payment.confirmed", "correlationKey": order, "data": {"paid": 99}})),
    )
    .await;
    assert_eq!(status, 202, "{body}");
}

#[tokio::test]
async fn us02_a_run_waiting_on_version_one_resumes_with_version_one_after_two_is_published() {
    let p = platform().await;
    let engine = engine(&p).await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    let dir = Dir::new();
    dir.document("checkout", &waiting("act.ship-v1"));
    dir.manifest(NS, &["checkout"]);
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(0)
        .mentions("published  checkout v1");
    let checkout = workflow_id(&p, NS, "checkout").await;

    // An order is waiting for its payment on version 1.
    let (status, started) = rest_in(
        &p,
        "alice",
        NS,
        Method::POST,
        "/runs",
        Some(&json!({"workflowId": checkout, "input": {"orderId": "ord-1"}})),
    )
    .await;
    assert_eq!(status, 202, "{started}");
    let old = started["runId"].as_str().expect("run").to_owned();
    within("the first run to wait", 20, || async {
        let (_, run) = rest_in(&p, "alice", NS, Method::GET, &format!("/runs/{old}"), None).await;
        (run["status"] == "WAITING").then_some(())
    })
    .await;

    // Version 2 ships with another activity and is published while the order waits.
    dir.document("checkout", &waiting("act.ship-v2"));
    cli(&p, &dir, "dev1", &["publish"])
        .await
        .expect(0)
        .mentions("published  checkout v2")
        .mentions("prepare   ACTIVITY_CHANGED");
    let (_, started) = rest_in(
        &p,
        "alice",
        NS,
        Method::POST,
        "/runs",
        Some(&json!({"workflowId": checkout, "input": {"orderId": "ord-2"}})),
    )
    .await;
    let new = started["runId"].as_str().expect("run").to_owned();

    // Both orders are paid. The old run asks for version 1's activity, the new one for version 2's.
    paid(&p, "e-1", "ord-1").await;
    paid(&p, "e-2", "ord-2").await;
    let mut worker = p.worker_at(&engine.grpc_url, "wendy").await;
    worker.register(&["act.ship-v1", "act.ship-v2"]).await;
    let mut asked = std::collections::BTreeMap::new();
    while asked.len() < 2 {
        for task in worker.poll(2).await {
            asked.insert(task.run_id.clone(), task.activity_name.clone());
            worker
                .complete(&task, &json!({"shipped": true}))
                .await
                .expect("complete");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        asked[&old], "act.ship-v1",
        "the run keeps the semantics it started with"
    );
    assert_eq!(asked[&new], "act.ship-v2");
    for run in [&old, &new] {
        within("the run to finish", 20, || async {
            let (_, body) =
                rest_in(&p, "alice", NS, Method::GET, &format!("/runs/{run}"), None).await;
            (body["status"] == "SUCCEEDED").then_some(())
        })
        .await;
    }

    // The old run is still bound to version 1, which was not touched.
    let (_, first) = rest_in(&p, "alice", NS, Method::GET, &format!("/runs/{old}"), None).await;
    let (_, v1) = rest_in(
        &p,
        "alice",
        NS,
        Method::GET,
        &format!("/workflows/{checkout}/versions/1"),
        None,
    )
    .await;
    assert_eq!(first["versionId"], v1["versionId"]);
}

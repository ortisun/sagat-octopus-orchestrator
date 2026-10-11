//! US02 — wake a suspended run on an event: a run waits for a correlated event, the event arrives,
//! the engine releases the step once and the evidence is kept (US02-AC01).

use std::time::Duration;

use reqwest::Method;
use serde_json::json;

use crate::support::worker::platform;
use crate::{engine, within};

fn waiting(name: &str, lookback_ms: u64) -> serde_json::Value {
    let mut wait = json!({
        "id": "pay", "type": "waitEvent", "eventType": "payment.confirmed",
        "correlationExpr": "input.orderId", "timeoutMs": 300000, "next": "ship"
    });
    if lookback_ms > 0 {
        wait["lookbackMs"] = json!(lookback_ms);
    }
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "pay",
        "nodes": [wait, {"id": "ship", "type": "activity", "activity": "act.ship"}]
    })
}

async fn publish_event(
    p: &crate::support::worker::Platform,
    id: &str,
    order: &str,
) -> serde_json::Value {
    let (status, body) = p
        .rest(
            Method::POST,
            "/events",
            Some(&json!({
                "eventId": id, "type": "payment.confirmed", "correlationKey": order,
                "data": {"order": order, "paid": 99}
            })),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    body
}

#[tokio::test]
async fn us02_the_correlated_event_releases_the_waiting_step_once_and_the_evidence_is_kept() {
    let p = platform().await;
    let engine = engine(&p, &[]).await;
    let workflow = p.publish("payment-flow", &waiting("payment-flow", 0)).await;
    let run = p.start(&workflow, &json!({"orderId": "ord-1"})).await;
    let other = p.start(&workflow, &json!({"orderId": "ord-2"})).await;
    assert_eq!(p.status(&run).await, "WAITING");

    // An event for another order wakes only that order's run, and it is delivered once even if the
    // publisher sends it twice.
    publish_event(&p, "e-1", "ord-1").await;
    assert_eq!(
        publish_event(&p, "e-1", "ord-1").await["status"],
        "DUPLICATE"
    );
    within("the run to leave its wait", 20, || async {
        (p.status(&run).await == "RUNNING").then_some(())
    })
    .await;
    assert_eq!(
        p.status(&other).await,
        "WAITING",
        "the other order is still waiting"
    );

    // The journal says a wait was satisfied, once, and the inbox keeps the evidence.
    let (_, journal) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=200"), None)
        .await;
    let kinds: Vec<&str> = journal["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|e| e["eventType"].as_str())
        .collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == "wait.satisfied").count(),
        1,
        "{kinds:?}"
    );
    let ns = p.namespace_id().await;
    let (_, event) = p
        .rest(
            Method::GET,
            &format!("/namespaces/{ns}/events/api/e-1"),
            None,
        )
        .await;
    let consumer = event["deliveries"][0]["consumer"]
        .as_str()
        .expect("consumer")
        .to_owned();
    assert_eq!(consumer, format!("wait:{run}/pay"));

    // The step after the wait is the worker's, and the run finishes.
    let mut worker = p.worker_at(&engine.grpc_url, "wendy").await;
    worker.register(&["act.ship"]).await;
    let task = loop {
        if let Some(task) = worker.poll(1).await.into_iter().next() {
            break task;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    worker
        .complete(&task, &json!({"shipped": true}))
        .await
        .expect("complete");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}

#[tokio::test]
async fn us02_an_event_that_came_just_before_the_wait_is_delivered_when_the_wait_asks_to_look_back()
{
    let p = platform().await;
    let _engine = engine(&p, &[]).await;
    let early = p
        .publish("looks-back", &waiting("looks-back", 60_000))
        .await;
    let strict = p.publish("strict", &waiting("strict", 0)).await;

    // The event is admitted and dispatched while nobody waits for it.
    publish_event(&p, "e-early", "ord-9").await;
    let ns = p.namespace_id().await;
    within("the event to be dispatched", 20, || async {
        let (_, event) = p
            .rest(
                Method::GET,
                &format!("/namespaces/{ns}/events/api/e-early"),
                None,
            )
            .await;
        (event["status"] == "DISPATCHED").then_some(())
    })
    .await;

    // Then the runs begin to wait: the one that asked to look back gets it, the other does not.
    let looking = p.start(&early, &json!({"orderId": "ord-9"})).await;
    let not_looking = p.start(&strict, &json!({"orderId": "ord-9"})).await;
    within("the run that looks back to be woken", 20, || async {
        (p.status(&looking).await == "RUNNING").then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        p.status(&not_looking).await,
        "WAITING",
        "an event from before its wait is not its event"
    );
}

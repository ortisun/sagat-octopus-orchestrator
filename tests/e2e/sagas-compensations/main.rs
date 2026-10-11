//! The four user stories of feature 003 through the public interfaces: definitions, runs and
//! operator decisions over REST, the worker protocol over gRPC, the engine as a process, real
//! PostgreSQL. The workflow is the shipped example, `examples/payment-saga.workflow.json`.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod telemetry;
mod us01;
mod us02;
mod us03;
mod us04;

use reqwest::Method;
use serde_json::{Value, json};

use support::worker::Platform;

/// The example every story starts from.
pub fn payment_saga() -> Value {
    serde_json::from_str(include_str!("../../../examples/payment-saga.workflow.json"))
        .expect("the example is JSON")
}

/// The example with its compensation limited to `attempts`, so a failure reaches an operator
/// without waiting through minutes of back-off.
pub fn with_compensation_attempts(mut ir: Value, attempts: u64) -> Value {
    for node in ir["nodes"].as_array_mut().expect("nodes") {
        if node.get("compensationRetry").is_some() {
            node["compensationRetry"]["maxAttempts"] = json!(attempts);
        }
    }
    ir
}

pub const PAYMENT: &str = r#"{"paymentId":"p-1","amount":125.5}"#;

pub fn input() -> Value {
    serde_json::from_str(PAYMENT).expect("json")
}

pub async fn journal(p: &Platform, run: &str) -> Vec<(String, Option<String>)> {
    let (status, page) = p
        .rest(Method::GET, &format!("/runs/{run}/events?limit=200"), None)
        .await;
    assert_eq!(status, 200, "{page}");
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|event| {
            (
                event["eventType"].as_str().expect("type").to_owned(),
                event["stepId"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

pub async fn graph(p: &Platform, run: &str) -> Value {
    let (status, body) = p
        .rest(Method::GET, &format!("/runs/{run}/graph"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    body
}

pub fn node<'a>(graph: &'a Value, id: &str) -> &'a Value {
    graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node {id} in {graph}"))
}

/// An operator's decision about a step, over REST.
pub async fn decide(p: &Platform, who: Option<&str>, run: &str, body: &Value) -> (u16, Value) {
    let path = format!("/runs/{run}/operations");
    match who {
        Some(sub) => p.rest_as(sub, Method::POST, &path, Some(body)).await,
        None => p.rest(Method::POST, &path, Some(body)).await,
    }
}

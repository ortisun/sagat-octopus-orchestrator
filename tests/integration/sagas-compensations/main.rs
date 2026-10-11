//! Feature 003 integration suite against a real PostgreSQL: the compensation plan stored with a
//! version, compensations as steps of a run, reconciliation of an unknown outcome, and the operator
//! commands, with the journal checked against its projections in every scenario.
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../support/mod.rs"]
mod support;

mod breaker;
mod children;
mod compensation;
mod plan;
mod reconcile;
mod review;
mod stopping;

use serde_json::{Value, json};

use support::engine::{
    Session, World, activity, document, effect_activities, effect_node, saga_node, with,
};

/// `settlement`: a saga over `reserve` then `submit`, followed by `notify`.
pub fn settlement() -> Value {
    document(
        "settlement",
        "settlement",
        vec![
            saga_node("settlement", "reserve", Some("notify")),
            effect_node("reserve", Some("submit"), false),
            effect_node("submit", None, true),
            activity("notify"),
        ],
    )
}

/// A worker for everything `settlement` can ask for.
pub async fn worker(w: &World) -> Session {
    let mut names = effect_activities(&["reserve", "submit"]);
    names.push("act.notify".to_owned());
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    w.session(&names).await
}

/// A `settlement` whose steps retry fast, so a test does not wait for backoff.
pub fn quick(mut doc: Value) -> Value {
    for node in doc["nodes"].as_array_mut().expect("nodes") {
        if node["type"] == "activity" {
            *node = with(
                node.clone(),
                "retry",
                json!({ "maxAttempts": 2, "initialDelayMs": 100, "maxDelayMs": 100,
                        "strategy": "fixed", "jitter": false }),
            );
        }
    }
    doc
}

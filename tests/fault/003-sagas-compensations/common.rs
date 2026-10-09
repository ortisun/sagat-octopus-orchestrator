//! What the fault tests share.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use sagat_domain::execution::RunId;
use serde_json::{Value, json};

use crate::support::engine::{
    Session, World, activity, document, effect_activities, effect_node, saga_node, with,
};

/// `settlement`: a saga over `reserve` then `submit` (which can be looked up), then `notify`.
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

/// Every attempt, compensation and lookup of `settlement` retries fast.
pub fn quick(mut doc: Value) -> Value {
    let fast = json!({ "maxAttempts": 4, "initialDelayMs": 100, "maxDelayMs": 100,
                       "strategy": "fixed", "jitter": false });
    for node in doc["nodes"].as_array_mut().expect("nodes") {
        if node["type"] == "activity" {
            *node = with(node.clone(), "retry", fast.clone());
            if node.get("compensateWith").is_some() {
                *node = with(node.clone(), "compensationRetry", fast.clone());
            }
        }
    }
    doc
}

/// A worker for everything `settlement` can ask for.
pub async fn worker(w: &World) -> Session {
    let mut names = effect_activities(&["reserve", "submit"]);
    names.push("act.notify".to_owned());
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    w.session(&names).await
}

/// A stand-in for an external system that deduplicates by idempotency key, as a payment API does.
#[derive(Default, Clone)]
pub struct ExternalSystem {
    performed: Arc<Mutex<BTreeMap<String, u32>>>,
}

impl ExternalSystem {
    /// Performs the side effect unless this key already did; returns whether it ran.
    pub fn perform(&self, key: &str) -> bool {
        let mut performed = self.performed.lock().expect("lock");
        let count = performed.entry(key.to_owned()).or_insert(0);
        *count += 1;
        *count == 1
    }

    pub fn effects_with_prefix(&self, prefix: &str) -> usize {
        self.performed
            .lock()
            .expect("lock")
            .keys()
            .filter(|key| key.starts_with(prefix))
            .count()
    }

    pub fn calls(&self, key: &str) -> u32 {
        self.performed
            .lock()
            .expect("lock")
            .get(key)
            .copied()
            .unwrap_or(0)
    }
}

/// Everything observable about a saga run's rows, to compare before and after.
pub async fn snapshot(
    w: &World,
    run: RunId,
) -> (
    Vec<String>,
    Vec<(String, String, i32, i64)>,
    Vec<(String, String, Option<String>)>,
    i64,
    String,
) {
    let journal = w.journal(run).await;
    let steps: Vec<(String, String, i32, i64)> = sqlx::query_as(
        "SELECT node, status, attempt, generation FROM steps WHERE run_id = $1 ORDER BY node",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("steps");
    let sagas: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT node, phase, outcome FROM run_sagas WHERE run_id = $1 ORDER BY node",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("sagas");
    let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM event_outbox")
        .fetch_one(&w.db.pool)
        .await
        .expect("outbox");
    let status = w.run_status(run).await.as_str().to_owned();
    (journal, steps, sagas, outbox, status)
}

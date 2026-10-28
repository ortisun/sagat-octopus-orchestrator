//! Shared fault-plan accounting. Proxies inject failures without modifying SDK semantics.
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, Mutex};
#[path = "faults/grpc.rs"]
mod grpc;
#[path = "faults/http.rs"]
mod http;
pub struct Proxies {
    pub config: Value,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Proxies {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Proxies {
    pub async fn start(config: &Value, plan: &Plan) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = config.clone();
        let mut tasks = Vec::new();
        if plan.has("http") {
            let (url, task) = http::start(
                config["apiUrl"].as_str().ok_or("missing API URL")?,
                plan.clone(),
                super::control::client()?,
            )
            .await?;
            config["apiUrl"] = url.into();
            tasks.push(task);
        }
        if plan.has("grpc") {
            let (url, task) = grpc::start(
                config["grpcUrl"].as_str().ok_or("missing gRPC URL")?,
                plan.clone(),
            )
            .await?;
            config["grpcUrl"] = url.into();
            tasks.push(task);
        }
        Ok(Self { config, tasks })
    }
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fault {
    pub channel: String,
    #[serde(rename = "match")]
    pub prefix: String,
    pub action: String,
    #[serde(default)]
    pub after: usize,
    pub times: Option<usize>,
    pub status: Option<u16>,
    pub code: Option<String>,
    pub reason: Option<String>,
    pub retry_after_ms: Option<u64>,
    pub body: Option<Value>,
}
#[derive(Default)]
struct State {
    faults: Vec<Fault>,
    seen: Vec<usize>,
    answered: Vec<usize>,
}
#[derive(Clone, Default)]
pub struct Plan(Arc<Mutex<State>>);
impl Plan {
    pub fn new(value: &Value) -> Result<Self, Box<dyn std::error::Error>> {
        let faults: Vec<Fault> = serde_json::from_value(value.clone())?;
        let count = faults.len();
        Ok(Self(Arc::new(Mutex::new(State {
            faults,
            seen: vec![0; count],
            answered: vec![0; count],
        }))))
    }
    pub fn has(&self, channel: &str) -> bool {
        self.0
            .lock()
            .is_ok_and(|s| s.faults.iter().any(|f| f.channel == channel))
    }
    pub fn next(&self, channel: &str, key: &str) -> Option<Fault> {
        let mut state = self.0.lock().ok()?;
        let mut due = None;
        for i in 0..state.faults.len() {
            let fault = &state.faults[i];
            if fault.channel != channel || !key.starts_with(&fault.prefix) {
                continue;
            }
            let count = state.seen[i];
            let eligible =
                count >= fault.after && fault.times.is_none_or(|n| count - fault.after < n);
            state.seen[i] = count.saturating_add(1);
            if due.is_none() && eligible {
                due = Some(i);
            }
        }
        let index = due?;
        state.answered[index] += 1;
        Some(state.faults[index].clone())
    }
    pub fn applied(&self) -> Vec<usize> {
        self.0
            .lock()
            .map(|s| s.answered.clone())
            .unwrap_or_default()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_every_matching_rule_but_applies_only_the_first_due_one() {
        let plan = Plan::new(&serde_json::json!([
            {"channel":"http","match":"POST /runs","action":"failBefore","times":1},
            {"channel":"http","match":"POST /runs","action":"dropResponse","after":1,"times":1},
            {"channel":"grpc","match":"CompleteStep","action":"dropResponse","times":1}
        ]))
        .unwrap();
        assert!(plan.has("http"));
        assert_eq!(
            plan.next("http", "POST /runs").unwrap().action,
            "failBefore"
        );
        assert_eq!(
            plan.next("http", "POST /runs/one").unwrap().action,
            "dropResponse"
        );
        assert!(plan.next("http", "POST /runs").is_none());
        assert_eq!(plan.applied(), [1, 1, 0]);
        assert_eq!(
            plan.next("grpc", "CompleteStep").unwrap().action,
            "dropResponse"
        );
        assert_eq!(plan.applied(), [1, 1, 1]);
    }
}

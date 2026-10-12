//! The server side of the four user stories of feature 007 (the Studio), through the public
//! interfaces: real PostgreSQL, the REST API, the worker protocol over gRPC. What the Studio
//! shows and does in a browser is played by the journeys of `frontend/` (Playwright), which a
//! harness runs against this same platform (see `studio_journeys.rs`).
//!
//! US01: a draft is saved whatever its state, validated as publish would and compared with the
//! version it came from; US02: a script node is published only when its artifact was built;
//! US03: a run's graph is followed by journal sequence, by polling or by a stream, without
//! missing a transition; US04: an operator's decision shows in the stream as it happens.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod payload_inspection;
mod studio_journeys;
mod us01;
mod us02;
mod us03;
mod us04;

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::Method;
use serde_json::{Value, json};

use support::worker::{HUMAN, NAMESPACE, Platform, TENANT};

/// A chain of `n` activities `a0 -> a1 -> ...`, the activity of node `i` being `act.i`.
pub fn chain(name: &str, n: usize) -> Value {
    let nodes: Vec<Value> = (0..n)
        .map(|i| {
            let mut node =
                json!({"id": format!("a{i}"), "type": "activity", "activity": format!("act.{i}")});
            if i + 1 < n {
                node["next"] = json!(format!("a{}", i + 1));
            }
            node
        })
        .collect();
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "a0", "nodes": nodes
    })
}

/// The names of the activities of [`chain`], to register a worker with.
pub fn activities(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("act.{i}")).collect()
}

pub async fn changes(p: &Platform, run: &str, since: u64) -> Value {
    let (status, body) = p
        .rest(
            Method::GET,
            &format!("/runs/{run}/graph/changes?since={since}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    body
}

pub async fn graph(p: &Platform, run: &str) -> Value {
    let (status, body) = p
        .rest(Method::GET, &format!("/runs/{run}/graph"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    body
}

/// The nodes of a graph or of a delta by id.
pub fn by_id(body: &Value) -> BTreeMap<String, Value> {
    body["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .map(|n| (n["id"].as_str().expect("id").to_owned(), n.clone()))
        .collect()
}

/// What a Studio does with the changes it receives: replace everything on a reset, else replace
/// the nodes that came.
pub fn apply(model: &mut BTreeMap<String, Value>, seq: &mut u64, delta: &Value) {
    if delta["reset"] == true {
        *model = by_id(delta);
    } else {
        model.extend(by_id(delta));
    }
    *seq = delta["projectionSeq"].as_u64().expect("projectionSeq");
}

/// One server-sent event.
#[derive(Debug, Clone)]
pub struct Message {
    pub event: String,
    pub id: Option<String>,
    pub data: Value,
}

/// A stream being read.
pub struct Stream {
    response: reqwest::Response,
    buffer: String,
}

impl Stream {
    /// Opens `/runs/{run}/stream` as `sub` of `tenant`. `last_event_id` is what a reconnecting
    /// client sends.
    pub async fn open(
        p: &Platform,
        run: &str,
        sub: &str,
        tenant: &str,
        last_event_id: Option<&str>,
    ) -> Result<Self, (u16, Value)> {
        let mut request = p
            .api
            .as_user(Method::GET, &format!("/runs/{run}/stream"), sub, tenant)
            .header("x-octopus-namespace", NAMESPACE)
            .header("accept", "text/event-stream");
        if let Some(id) = last_event_id {
            request = request.header("last-event-id", id);
        }
        let response = request.send().await.expect("send");
        let status = response.status().as_u16();
        if status != 200 {
            return Err((status, support::api::json_of(response).await));
        }
        Ok(Self {
            response,
            buffer: String::new(),
        })
    }

    /// Opens it as the usual human.
    pub async fn open_as_alice(p: &Platform, run: &str) -> Self {
        Self::open(p, run, HUMAN, TENANT, None)
            .await
            .expect("the stream opens")
    }

    /// The next event, `None` when the server closed the stream or `within` passed.
    pub async fn next(&mut self, within: Duration) -> Option<Message> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if let Some(message) = self.take() {
                return Some(message);
            }
            let remaining = deadline.checked_duration_since(tokio::time::Instant::now())?;
            match tokio::time::timeout(remaining, self.response.chunk()).await {
                Ok(Ok(Some(chunk))) => self.buffer.push_str(&String::from_utf8_lossy(&chunk)),
                Ok(Ok(None) | Err(_)) | Err(_) => return None,
            }
        }
    }

    /// Reads events until one is called `name`, returning it and what came before.
    pub async fn until(&mut self, name: &str, within: Duration) -> Option<(Message, Vec<Message>)> {
        let deadline = tokio::time::Instant::now() + within;
        let mut before = Vec::new();
        loop {
            let remaining = deadline.checked_duration_since(tokio::time::Instant::now())?;
            let message = self.next(remaining).await?;
            if message.event == name {
                return Some((message, before));
            }
            before.push(message);
        }
    }

    /// A complete event out of the buffer (comments are dropped).
    fn take(&mut self) -> Option<Message> {
        loop {
            let end = self.buffer.find("\n\n")?;
            let block: String = self.buffer.drain(..end + 2).collect();
            let (mut event, mut id, mut data) = (String::new(), None, String::new());
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("event:") {
                    rest.trim().clone_into(&mut event);
                } else if let Some(rest) = line.strip_prefix("id:") {
                    id = Some(rest.trim().to_owned());
                } else if let Some(rest) = line.strip_prefix("data:") {
                    data.push_str(rest.trim_start());
                }
            }
            if event.is_empty() && data.is_empty() {
                continue; // a comment: `: ping`
            }
            return Some(Message {
                event,
                id,
                data: serde_json::from_str(&data).unwrap_or(Value::Null),
            });
        }
    }
}

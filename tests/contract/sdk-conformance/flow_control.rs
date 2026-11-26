//! Feature 022's independent assertions (T022-053, T022-084; `contracts/conformance.md`): what the journals of the runs
//! a language started say about the gates, checked from the platform's own rows and instants, not from the golden. The
//! golden holds only the normalized event order and states; the intervals are computed here.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

/// One `concurrency.*`, `rate_limit.*` or `step.leased` event of a run, as `run_events` holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateEvent {
    pub run: String,
    pub seq: i64,
    pub kind: String,
    pub node: Option<String>,
    /// `RUN` or `STEP` (`detail.scope`); `None` for `step.leased`.
    pub scope: Option<String>,
    /// `detail.group` and `detail.key_label`: which semaphore or window the event is about.
    pub group: Option<String>,
    pub key: Option<String>,
    /// The instant the event occurred (database clock, microseconds).
    pub at_us: i64,
    /// `rate_limit.granted` only: the instant the slot counts from (`detail.granted_at_us`).
    pub granted_at_us: Option<i64>,
    /// `rate_limit.granted` only (`detail.from_queue`).
    pub from_queue: Option<bool>,
}

impl GateEvent {
    fn at_run_scope(&self) -> bool {
        self.scope.as_deref() == Some("RUN")
    }

    fn semaphore(&self) -> (String, String) {
        (
            self.group.clone().unwrap_or_default(),
            self.key.clone().unwrap_or_default(),
        )
    }
}

/// The run-scope permits of each key never had more than `limit` holders at once, and the runs a key queued were
/// granted in the order they were queued (`flow-concurrency`). A holder holds from its `concurrency.acquired` to its
/// `concurrency.released`; an interval is half-open, so a grant at the instant of a release does not overlap it.
pub fn check_concurrency(events: &[GateEvent], limit: usize) -> Vec<String> {
    let mut failures = Vec::new();
    let mut keys: BTreeMap<(String, String), Vec<&GateEvent>> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.at_run_scope() && event.kind.starts_with("concurrency."))
    {
        keys.entry(event.semaphore()).or_default().push(event);
    }
    for ((group, key), events) in keys {
        // (instant, +1 for an acquisition, -1 for a release): releases first at one instant
        let mut changes: Vec<(i64, i64)> = Vec::new();
        for event in &events {
            match event.kind.as_str() {
                "concurrency.acquired" => changes.push((event.at_us, 1)),
                "concurrency.released" => changes.push((event.at_us, -1)),
                _ => {}
            }
        }
        changes.sort_unstable();
        let (mut holders, mut peak) = (0_i64, 0_i64);
        for (_, change) in changes {
            holders += change;
            peak = peak.max(holders);
        }
        if peak > i64::try_from(limit).unwrap_or(i64::MAX) {
            failures.push(format!(
                "group {group} key {key}: {peak} runs held a permit at once, the limit is {limit}"
            ));
        }
        let first = |kind: &str| -> BTreeMap<&str, i64> {
            let mut at = BTreeMap::new();
            for event in events.iter().filter(|event| event.kind == kind) {
                at.entry(event.run.as_str()).or_insert(event.at_us);
            }
            at
        };
        let (queued, acquired) = (first("concurrency.queued"), first("concurrency.acquired"));
        let mut granted: Vec<(&str, i64, i64)> = queued
            .iter()
            .filter_map(|(run, queued_at)| acquired.get(run).map(|at| (*run, *queued_at, *at)))
            .collect();
        granted.sort_by_key(|(_, queued_at, _)| *queued_at);
        for pair in granted.windows(2) {
            if pair[1].2 < pair[0].2 {
                failures.push(format!(
                    "group {group} key {key}: run {} was queued before run {} but granted after it",
                    pair[0].0, pair[1].0
                ));
            }
        }
        for (run, at) in &acquired {
            if queued.get(run).is_some_and(|queued_at| at < queued_at) {
                failures.push(format!("run {run} was granted before it was queued"));
            }
        }
    }
    failures
}

/// No window of `per_us` holds more than `limit` of `instants` (a sliding window, as the gate counts it).
pub fn check_window(what: &str, instants: &[i64], limit: usize, per_us: i64) -> Vec<String> {
    let mut sorted = instants.to_vec();
    sorted.sort_unstable();
    let mut failures = Vec::new();
    for (i, start) in sorted.iter().enumerate() {
        let within = sorted[i..]
            .iter()
            .take_while(|at| **at < start + per_us)
            .count();
        if within > limit {
            failures.push(format!(
                "{what}: {within} in the window of {per_us} µs from {start}, the limit is {limit}"
            ));
        }
    }
    failures
}

/// The rate limits of `flow-rate-limit`: the run-scope grants of each key and the leases of each rate-limited node
/// stay within their windows, a run granted without waiting never queued and one granted after waiting did, and the
/// runs a key buffered were granted in the order they were buffered.
pub fn check_rate(
    events: &[GateEvent],
    run_limit: (usize, i64),
    node_limits: &BTreeMap<String, (usize, i64)>,
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut keys: BTreeMap<(String, String), Vec<&GateEvent>> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.at_run_scope() && event.kind.starts_with("rate_limit."))
    {
        keys.entry(event.semaphore()).or_default().push(event);
    }
    for ((group, key), events) in &keys {
        let grants: Vec<&&GateEvent> = events
            .iter()
            .filter(|event| event.kind == "rate_limit.granted")
            .collect();
        let instants: Vec<i64> = grants
            .iter()
            .map(|event| event.granted_at_us.unwrap_or(event.at_us))
            .collect();
        failures.extend(check_window(
            &format!("run grants of group {group} key {key}"),
            &instants,
            run_limit.0,
            run_limit.1,
        ));
        let mut buffered: Vec<(&str, i64, i64)> = Vec::new();
        for grant in &grants {
            let queued = events
                .iter()
                .find(|event| event.run == grant.run && event.kind == "rate_limit.queued");
            match (queued, grant.from_queue) {
                (Some(queued), Some(true)) => buffered.push((
                    grant.run.as_str(),
                    queued.at_us,
                    grant.granted_at_us.unwrap_or(grant.at_us),
                )),
                (None, Some(false)) => {}
                (queued, from_queue) => failures.push(format!(
                    "run {}: granted with from_queue {from_queue:?} but {} queued",
                    grant.run,
                    if queued.is_some() { "it" } else { "never" }
                )),
            }
        }
        buffered.sort_by_key(|(_, queued_at, _)| *queued_at);
        for pair in buffered.windows(2) {
            if pair[1].2 < pair[0].2 {
                failures.push(format!(
                    "group {group} key {key}: run {} was buffered before run {} but granted after it",
                    pair[0].0, pair[1].0
                ));
            }
        }
        for event in events
            .iter()
            .filter(|event| event.kind == "rate_limit.dequeued")
        {
            if grants.iter().any(|grant| grant.run == event.run) {
                failures.push(format!("run {} was dequeued and granted", event.run));
            }
        }
    }
    for (node, (limit, per_us)) in node_limits {
        let leases: Vec<i64> = events
            .iter()
            .filter(|event| event.kind == "step.leased" && event.node.as_deref() == Some(node))
            .map(|event| event.at_us)
            .collect();
        failures.extend(check_window(
            &format!("leases of {node}"),
            &leases,
            *limit,
            *per_us,
        ));
    }
    failures
}

/// The length of a window in microseconds.
pub fn per_us(per: &str) -> Option<i64> {
    match per {
        "second" => Some(1_000_000),
        "minute" => Some(60_000_000),
        "hour" => Some(3_600_000_000),
        _ => None,
    }
}

/// The normalized evidence of the golden: per run, its gate events in order with their scope, node, whether a grant
/// came from the queue and which key (named by order of first appearance, since a label is a keyed digest), and no
/// instant.
pub fn normalized(events: &[GateEvent]) -> Value {
    let mut labels: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut runs: Vec<(String, Vec<Value>)> = Vec::new();
    for event in events {
        if event.kind == "step.leased" {
            continue;
        }
        let next = format!("k{}", labels.len() + 1);
        let label = labels.entry(event.semaphore()).or_insert(next).clone();
        let mut entry = Map::new();
        entry.insert("event".into(), json!(event.kind));
        entry.insert("key".into(), json!(label));
        if let Some(scope) = &event.scope {
            entry.insert("scope".into(), json!(scope));
        }
        if let Some(node) = &event.node {
            entry.insert("node".into(), json!(node));
        }
        if let Some(from_queue) = event.from_queue {
            entry.insert("fromQueue".into(), json!(from_queue));
        }
        match runs.iter_mut().find(|(run, _)| *run == event.run) {
            Some((_, list)) => list.push(Value::Object(entry)),
            None => runs.push((event.run.clone(), vec![Value::Object(entry)])),
        }
    }
    Value::Array(
        runs.into_iter()
            .map(|(run, events)| json!({"runId": run, "events": events}))
            .collect(),
    )
}

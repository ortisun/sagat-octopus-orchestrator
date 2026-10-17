//! What is compared across SDKs: the results with everything that legitimately differs (ids, instants,
//! trace ids, the SDK's own words) replaced by placeholders.

use std::collections::HashMap;

use regex::Regex;
use serde_json::{Map, Value};

pub struct Normalizer {
    uuid: Regex,
    trace: Regex,
    time: Regex,
    uuids: HashMap<String, usize>,
    traces: HashMap<String, usize>,
    unique: String,
    given_traces: Vec<String>,
}

impl Normalizer {
    pub fn new(unique: &str, given_traces: Vec<String>) -> Self {
        Self {
            uuid: Regex::new(
                r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
            )
            .expect("regex"),
            trace: Regex::new(r"\b[0-9a-f]{32}\b").expect("regex"),
            time: Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})")
                .expect("regex"),
            uuids: HashMap::new(),
            traces: HashMap::new(),
            unique: unique.to_owned(),
            given_traces,
        }
    }

    pub fn value(&mut self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.text(text)),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.value(v)).collect()),
            Value::Object(map) => {
                // What is masked is masked where it is known to be the SDK's own: the words of an error (it
                // carries a remediation), the journal counter of a Run. A `message` or a `version` in a payload
                // the scenario sent is compared like any other value.
                let is_error = map.contains_key("remediation");
                let is_run = map.contains_key("runId") && map.contains_key("createdAt");
                let deadline_after = if is_run {
                    deadline_after_creation(map)
                } else {
                    None
                };
                let mut out = Map::new();
                for (key, item) in map {
                    match key.as_str() {
                        // The SDK's own words: present, not compared.
                        "message" if is_error => {
                            let present = item.as_str().is_some_and(|m| !m.is_empty());
                            out.insert(
                                key.clone(),
                                Value::String(if present {
                                    "<text>".into()
                                } else {
                                    "<empty>".into()
                                }),
                            );
                        }
                        // A journal counter: depends on when it was read.
                        "version" if is_run && item.is_number() => {
                            out.insert(key.clone(), Value::String("<n>".into()));
                        }
                        // The deadline of a run is "so long after it was created": that is what an SDK that
                        // sends it in the wrong unit would get wrong, so that is what is compared.
                        "deadline" if deadline_after.is_some() => {
                            out.insert(
                                key.clone(),
                                Value::String(format!(
                                    "<createdAt+{}s>",
                                    deadline_after.unwrap_or_default()
                                )),
                            );
                        }
                        _ => {
                            out.insert(key.clone(), self.value(item));
                        }
                    }
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }

    fn text(&mut self, text: &str) -> String {
        let mut text = if self.unique.is_empty() {
            text.to_owned()
        } else {
            text.replace(&self.unique, "{{unique}}")
        };
        // Instants first, so that digits inside them are not taken for anything else.
        text = self.time.replace_all(&text, "<time>").into_owned();
        let found: Vec<String> = self
            .uuid
            .find_iter(&text)
            .map(|m| m.as_str().to_lowercase())
            .collect();
        for id in found {
            let next = self.uuids.len() + 1;
            let n = *self.uuids.entry(id.clone()).or_insert(next);
            let pattern = Regex::new(&format!("(?i){}", regex::escape(&id))).expect("regex");
            text = pattern
                .replace_all(&text, format!("<uuid:{n}>"))
                .into_owned();
        }
        let traces: Vec<String> = self
            .trace
            .find_iter(&text)
            .map(|m| m.as_str().to_owned())
            .collect();
        for trace in traces {
            let replacement = if self.given_traces.contains(&trace) {
                "<given-trace>".to_owned()
            } else {
                let next = self.traces.len() + 1;
                format!(
                    "<trace:{}>",
                    *self.traces.entry(trace.clone()).or_insert(next)
                )
            };
            text = text.replace(&trace, &replacement);
        }
        text
    }
}

/// Seconds between `createdAt` and `deadline` of a Run, rounded, if it has a deadline.
fn deadline_after_creation(run: &Map<String, Value>) -> Option<i64> {
    let created = chrono::DateTime::parse_from_rfc3339(run.get("createdAt")?.as_str()?).ok()?;
    let deadline = chrono::DateTime::parse_from_rfc3339(run.get("deadline")?.as_str()?).ok()?;
    let millis = (deadline - created).num_milliseconds();
    Some((millis as f64 / 1000.0).round() as i64)
}

/// The part of a runner's output that is compared.
pub fn compared(out: &Value, normalizer: &mut Normalizer) -> Value {
    let mut doc = Map::new();
    for key in [
        "observations",
        "workerLog",
        "faultsApplied",
        "serverState",
        "blobEvidence",
        "runInputBlobEvidence",
        "scopeEvidence",
        "auditEvidence",
    ] {
        if let Some(value) = out.get(key) {
            doc.insert(key.to_owned(), normalizer.value(value));
        }
    }
    Value::Object(doc)
}

/// The first place two documents differ, as a JSON pointer, with both values.
pub fn first_difference(a: &Value, b: &Value) -> Option<String> {
    fn walk(a: &Value, b: &Value, path: &str) -> Option<String> {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                for key in x.keys().chain(y.keys()) {
                    let here = format!("{path}/{key}");
                    match (x.get(key), y.get(key)) {
                        (Some(l), Some(r)) => {
                            if let Some(found) = walk(l, r, &here) {
                                return Some(found);
                            }
                        }
                        (l, r) => return Some(format!("{here}: {} != {}", show(l), show(r))),
                    }
                }
                None
            }
            (Value::Array(x), Value::Array(y)) => {
                for i in 0..x.len().max(y.len()) {
                    let here = format!("{path}/{i}");
                    match (x.get(i), y.get(i)) {
                        (Some(l), Some(r)) => {
                            if let Some(found) = walk(l, r, &here) {
                                return Some(found);
                            }
                        }
                        (l, r) => return Some(format!("{here}: {} != {}", show(l), show(r))),
                    }
                }
                None
            }
            (l, r) if l == r => None,
            (l, r) => Some(format!("{path}: {l} != {r}")),
        }
    }
    fn show(v: Option<&Value>) -> String {
        v.map_or_else(|| "(absent)".to_owned(), ToString::to_string)
    }
    walk(a, b, "")
}

//! The messages of `events.asyncapi.yaml` only grow (FR-004-007), and the real messages the platform
//! produces conform to the shapes the document promises.
//!
//! The baseline `events.baseline.json` is the shape of every message when it was last released. A
//! property may be added; it may not disappear, change its type, become required or lose an
//! enumeration value. When a message is *intentionally* extended, regenerate the baseline with
//! `SAGAT_UPDATE_BASELINE=1 cargo test -p sagat-tests --test contract_asyncapi_004` and review the
//! diff: a removed line would be breaking.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod compatibility;
mod conformance;

use std::collections::BTreeMap;

use serde_json::{Value, json};

pub const DOCUMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../specs/001-platform-foundation/contracts/events.asyncapi.yaml"
);
pub const BASELINE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../specs/001-platform-foundation/contracts/events.baseline.json"
);

pub fn load() -> Value {
    serde_yaml_ng::from_str(&std::fs::read_to_string(DOCUMENT).expect("the document"))
        .expect("YAML")
}

/// What a consumer depends on in one schema: for every property path its type and enumeration, and
/// which paths are required.
pub fn shape(schema: &Value) -> Value {
    let mut properties = BTreeMap::new();
    let mut required = Vec::new();
    walk(schema, "", &mut properties, &mut required);
    required.sort();
    json!({ "properties": properties, "required": required })
}

fn walk(
    schema: &Value,
    path: &str,
    properties: &mut BTreeMap<String, Value>,
    required: &mut Vec<String>,
) {
    let kind = match &schema["type"] {
        Value::String(t) => vec![t.clone()],
        Value::Array(list) => {
            let mut names: Vec<String> = list
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            names.sort();
            names
        }
        _ => Vec::new(),
    };
    let mut entry = json!({ "type": kind });
    if let Some(values) = schema["enum"].as_array() {
        let mut names: Vec<String> = values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        names.sort();
        entry["enum"] = json!(names);
    }
    if !path.is_empty() {
        properties.insert(path.to_owned(), entry);
    }
    if let Some(object) = schema["properties"].as_object() {
        for (name, child) in object {
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            walk(child, &child_path, properties, required);
        }
    }
    if let Some(list) = schema["required"].as_array() {
        for name in list.iter().filter_map(Value::as_str) {
            required.push(if path.is_empty() {
                name.to_owned()
            } else {
                format!("{path}.{name}")
            });
        }
    }
    if schema["items"].is_object() {
        walk(&schema["items"], &format!("{path}[]"), properties, required);
    }
}

/// The shape of every message: its headers and its payload.
pub fn shapes(document: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for (name, message) in document["components"]["messages"]
        .as_object()
        .expect("messages")
    {
        out.insert(
            name.clone(),
            json!({
                "headers": shape(&message["headers"]),
                "payload": shape(&message["payload"]),
            }),
        );
    }
    Value::Object(out)
}

/// What a change from `before` to `after` breaks, in words.
pub fn breakages(before: &Value, after: &Value) -> Vec<String> {
    let mut found = Vec::new();
    for (message, parts) in before.as_object().expect("baseline messages") {
        let Some(now) = after.get(message) else {
            found.push(format!("{message}: the message is gone"));
            continue;
        };
        for part in ["headers", "payload"] {
            let (old, new) = (&parts[part], &now[part]);
            for (path, entry) in old["properties"].as_object().expect("properties") {
                let Some(current) = new["properties"].get(path) else {
                    found.push(format!("{message}.{part}: {path} was removed"));
                    continue;
                };
                if entry["type"] != current["type"]
                    && !entry["type"].as_array().is_some_and(Vec::is_empty)
                {
                    found.push(format!(
                        "{message}.{part}: {path} changed type from {} to {}",
                        entry["type"], current["type"]
                    ));
                }
                let old_values: Vec<&Value> = entry["enum"]
                    .as_array()
                    .map(|a| a.iter().collect())
                    .unwrap_or_default();
                for value in old_values {
                    if !current["enum"]
                        .as_array()
                        .is_some_and(|a| a.contains(value))
                    {
                        found.push(format!("{message}.{part}: {path} lost the value {value}"));
                    }
                }
            }
            let was: Vec<&Value> = old["required"]
                .as_array()
                .expect("required")
                .iter()
                .collect();
            for path in new["required"].as_array().expect("required") {
                if !was.contains(&path) {
                    found.push(format!("{message}.{part}: {path} became required"));
                }
            }
        }
    }
    found
}

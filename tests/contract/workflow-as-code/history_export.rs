//! The history export format (design 044, Technical Plan 7.4, Requirement 044-025): the lines after the header are
//! `HistoryEvent` messages in the proto3 JSON mapping, so the format is the closure of `HistoryEvent` in
//! `worker.proto`. The baseline `specs/044-workflow-as-code/contracts/history-export.baseline.json` names every
//! message and enumeration of that closure; a change of the protocol that would change what a replayer reads fails
//! here until the baseline is changed in a reviewed commit (a new `exportFormat` when it is not additive).

use std::collections::{BTreeMap, BTreeSet};

use prost_types::field_descriptor_proto::Type;
use serde_json::{Value, json};

use crate::{read_json, worker_proto};

fn short(type_name: &str) -> &str {
    type_name.rsplit('.').next().unwrap_or(type_name)
}

/// The messages (with the JSON names of their fields) and the enumerations (with their values) reachable from
/// `HistoryEvent`.
fn closure() -> (BTreeMap<String, Value>, BTreeMap<String, Value>) {
    let set = worker_proto();
    let file = set
        .file
        .iter()
        .find(|f| f.name() == "worker.proto")
        .expect("worker.proto");
    let mut messages = BTreeMap::new();
    let mut enums = BTreeMap::new();
    let mut pending = vec!["HistoryEvent".to_owned()];
    let mut seen = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(enumeration) = file.enum_type.iter().find(|e| e.name() == name) {
            let values: Vec<&str> = enumeration.value.iter().map(|v| v.name()).collect();
            enums.insert(name, json!(values));
            continue;
        }
        let message = file
            .message_type
            .iter()
            .find(|m| m.name() == name)
            .unwrap_or_else(|| panic!("{name} is a message or an enumeration of worker.proto"));
        let mut fields: Vec<String> = message
            .field
            .iter()
            .map(|f| {
                if f.json_name().is_empty() {
                    lower_camel(f.name())
                } else {
                    f.json_name().to_owned()
                }
            })
            .collect();
        fields.sort();
        messages.insert(name, json!(fields));
        for field in &message.field {
            if matches!(field.r#type(), Type::Message | Type::Enum) {
                pending.push(short(field.type_name()).to_owned());
            }
        }
    }
    (messages, enums)
}

fn lower_camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn the_history_export_format_matches_its_baseline() {
    let baseline = read_json("specs/044-workflow-as-code/contracts/history-export.baseline.json");
    assert_eq!(baseline["exportFormat"], json!(1));
    assert_eq!(baseline["contentType"], json!("application/x-ndjson"));
    assert_eq!(
        baseline["line"],
        json!("sagat_octopus_orchestrator.v1.HistoryEvent")
    );
    let header: Vec<&str> = baseline["header"]["fields"]
        .as_array()
        .expect("header fields")
        .iter()
        .map(|f| f.as_str().expect("a name"))
        .collect();
    for required in [
        "exportFormat",
        "gatedEntries",
        "stateMaxBytes",
        "seed",
        "codeDigest",
    ] {
        assert!(header.contains(&required), "the header carries {required}");
    }

    let (messages, enums) = closure();
    let expected_messages: BTreeMap<String, Value> =
        serde_json::from_value(baseline["messages"].clone()).expect("messages");
    let expected_enums: BTreeMap<String, Value> =
        serde_json::from_value(baseline["enums"].clone()).expect("enums");
    assert_eq!(
        messages, expected_messages,
        "the messages of the export changed"
    );
    assert_eq!(
        enums, expected_enums,
        "the enumerations of the export changed"
    );

    // What the export never carries (Technical Plan 4.2 and 4.5): no actor subject, no body digest, no node
    // specification, no fragment.
    for (message, fields) in &messages {
        for field in fields.as_array().expect("fields") {
            let field = field.as_str().expect("a name");
            for forbidden in [
                "subject",
                "actor",
                "bodyDigest",
                "spec",
                "fragment",
                "fragmentJson",
            ] {
                assert_ne!(field, forbidden, "{message}.{field} must not be exported");
            }
        }
    }
}

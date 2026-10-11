//! The worker protocol may only grow (NFR-002-COMP): a worker built against any earlier version of
//! `worker.proto` must keep working. This is a hand-rolled `buf breaking` for the rules that matter:
//! nothing the baseline declares may disappear or change meaning. New messages, new fields with new
//! numbers and new rpcs are allowed.
//!
//! The baseline is `worker.baseline.json` next to the contract. When the protocol is *intentionally*
//! extended, regenerate it with
//! `SAGAT_UPDATE_BASELINE=1 cargo test -p sagat-api-grpc --test compatibility`
//! and review the diff: removed lines would be breaking.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use prost_types::field_descriptor_proto::Label;
use prost_types::{FileDescriptorSet, ServiceDescriptorProto};
use serde_json::{Value, json};

fn contracts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../specs/001-platform-foundation/contracts")
}

fn current() -> FileDescriptorSet {
    protox::compile(["worker.proto"], [contracts()]).expect("worker.proto compiles")
}

/// Everything compatibility depends on, in a stable shape.
fn snapshot(set: &FileDescriptorSet) -> Value {
    let mut messages = BTreeMap::new();
    let mut services = BTreeMap::new();
    for file in &set.file {
        let package = file.package();
        for message in &file.message_type {
            let fields: BTreeMap<String, Value> = message
                .field
                .iter()
                .map(|f| {
                    (
                        f.number().to_string(),
                        json!({
                            "name": f.name(),
                            "type": format!(
                                "{:?}{}",
                                f.r#type(),
                                if f.type_name().is_empty() { String::new() } else { format!(":{}", f.type_name()) }
                            ),
                            "repeated": f.label() == Label::Repeated,
                        }),
                    )
                })
                .collect();
            messages.insert(format!("{package}.{}", message.name()), json!(fields));
        }
        for service in &file.service {
            services.insert(format!("{package}.{}", service.name()), rpcs(service));
        }
    }
    json!({ "messages": messages, "services": services })
}

fn rpcs(service: &ServiceDescriptorProto) -> Value {
    let map: BTreeMap<String, Value> = service
        .method
        .iter()
        .map(|m| {
            (
                m.name().to_owned(),
                json!({
                    "input": m.input_type(),
                    "output": m.output_type(),
                    "clientStreaming": m.client_streaming(),
                    "serverStreaming": m.server_streaming(),
                }),
            )
        })
        .collect();
    json!(map)
}

/// What `newer` breaks relative to `baseline`; empty when it is compatible.
fn breakages(baseline: &Value, newer: &Value) -> Vec<String> {
    let mut found = Vec::new();
    for (message, fields) in baseline["messages"].as_object().expect("messages") {
        let Some(now) = newer["messages"].get(message) else {
            found.push(format!("message {message} was removed"));
            continue;
        };
        for (number, field) in fields.as_object().expect("fields") {
            match now.get(number) {
                None => found.push(format!(
                    "{message}: field {number} ({}) was removed",
                    field["name"]
                )),
                Some(f) if f["type"] != field["type"] || f["repeated"] != field["repeated"] => {
                    found.push(format!(
                        "{message}: field {number} changed its type or cardinality"
                    ));
                }
                Some(f) if f["name"] != field["name"] => {
                    found.push(format!(
                        "{message}: field {number} was renamed (breaks JSON and code generation)"
                    ));
                }
                Some(_) => {}
            }
        }
    }
    for (service, methods) in baseline["services"].as_object().expect("services") {
        let Some(now) = newer["services"].get(service) else {
            found.push(format!("service {service} was removed"));
            continue;
        };
        for (name, method) in methods.as_object().expect("methods") {
            if now.get(name) != Some(method) {
                found.push(format!(
                    "{service}.{name} was removed or changed its signature"
                ));
            }
        }
    }
    found
}

fn baseline_path() -> PathBuf {
    contracts().join("worker.baseline.json")
}

#[test]
fn the_current_contract_is_compatible_with_the_published_baseline() {
    let now = snapshot(&current());
    if std::env::var_os("SAGAT_UPDATE_BASELINE").is_some() {
        std::fs::write(
            baseline_path(),
            serde_json::to_string_pretty(&now).expect("json") + "\n",
        )
        .expect("write the baseline");
    }
    let baseline: Value = serde_json::from_str(
        &std::fs::read_to_string(baseline_path()).expect("worker.baseline.json exists"),
    )
    .expect("valid json");
    let broken = breakages(&baseline, &now);
    assert!(
        broken.is_empty(),
        "worker.proto broke compatibility:\n  {}",
        broken.join("\n  ")
    );
}

#[test]
fn the_checker_notices_each_kind_of_breaking_change() {
    let now = snapshot(&current());
    let mut baseline = now.clone();
    // A field the "old" contract had that the current one lacks.
    baseline["messages"]["sagat_octopus_orchestrator.v1.HeartbeatRequest"]["99"] =
        json!({"name": "gone", "type": "String", "repeated": false});
    // A field whose type differs.
    baseline["messages"]["sagat_octopus_orchestrator.v1.PollTasksRequest"]["3"]["type"] =
        json!("String");
    // A renamed field.
    baseline["messages"]["sagat_octopus_orchestrator.v1.TaskLease"]["1"]["name"] =
        json!("old_name");
    // A removed rpc and a removed message.
    baseline["services"]["sagat_octopus_orchestrator.v1.WorkerService"]["Retired"] = json!({"input": ".x.A", "output": ".x.B", "clientStreaming": false, "serverStreaming": false});
    baseline["messages"]["sagat_octopus_orchestrator.v1.Retired"] = json!({});
    let broken = breakages(&baseline, &now);
    assert_eq!(broken.len(), 5, "{broken:#?}");

    // Growing is fine: a baseline that lacks the newer fields and messages is satisfied.
    let mut older = now.clone();
    older["messages"]["sagat_octopus_orchestrator.v1.HeartbeatRequest"]
        .as_object_mut()
        .expect("fields")
        .retain(|number, _| number.parse::<u32>().expect("number") <= 4);
    older["messages"]
        .as_object_mut()
        .expect("messages")
        .remove("sagat_octopus_orchestrator.v1.StepCommandResponse");
    assert!(breakages(&older, &now).is_empty());
}

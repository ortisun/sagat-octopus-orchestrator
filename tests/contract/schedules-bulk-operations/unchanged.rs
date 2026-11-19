//! What feature 018 leaves alone (`T018-010`, `FR-018-046`, `G2065`): the worker protocol is unchanged and the workflow IR
//! gains one optional field, `reapplyOnReset` on an Update declaration (`T018-078`, ADR-0032).
//!
//! A digest of the two files cannot say this, because it would fail for every later feature that changes them on purpose
//! (the worker protocol and the workflow IR have their own guards: `crates/api-grpc/tests/compatibility.rs` with
//! `worker.baseline.json`, and the contract suites of the IR). What these tests pin is the feature's own vocabulary: no
//! message, field, enum, service or method of the worker protocol, and no definition or property of the workflow IR, carries
//! a name of this feature (a schedule, a business id, a reset, a nominal time, a backfill, a batch job) but the one declared
//! field. A later feature that names something with one of these words on purpose changes the list here, in the pull request
//! that does it and with the decision that says why.
//!
//! Each check is a function of a parsed document, and the last test plants a name of the feature in a copy of each document
//! and sees it found, so a check that finds nothing because it looks at the wrong place is a red test, not a green one.

use std::fs;
use std::path::PathBuf;

use prost_types::{
    DescriptorProto, EnumDescriptorProto, FileDescriptorSet, ServiceDescriptorProto,
};
use serde_json::{Value, json};

const CONTRACTS: &str = "specs/001-platform-foundation/contracts";

/// The words of this feature, lower case and without `_`: a name is the feature's when it holds one.
const WORDS: [&str; 10] = [
    "schedule",
    "business",
    "reset",
    "nominal",
    "backfill",
    "batchjob",
    "batchitem",
    "batchselector",
    "startdelay",
    "obligation",
];

fn contracts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(CONTRACTS)
}

fn read_json(file: &str) -> Value {
    let path = contracts().join(file);
    let text =
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn is_the_features(name: &str) -> bool {
    let name = name.replace('_', "").to_ascii_lowercase();
    WORDS.iter().any(|word| name.contains(word))
}

/// The pointers of the definitions and properties of a JSON Schema whose name is the feature's.
fn schema_names(schema: &Value) -> Vec<String> {
    fn walk(value: &Value, at: &str, found: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let here = format!("{at}/{key}");
                    if matches!(
                        at.rsplit('/').next(),
                        Some("properties" | "$defs" | "definitions")
                    ) && is_the_features(key)
                    {
                        found.push(here.clone());
                    }
                    walk(child, &here, found);
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    walk(child, &format!("{at}/{index}"), found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    walk(schema, "", &mut found);
    found.sort();
    found
}

/// The names of `worker.baseline.json` that are the feature's: a message, a field of it, a service, a method.
fn baseline_names(baseline: &Value) -> Vec<String> {
    let mut found = Vec::new();
    for (message, fields) in baseline["messages"].as_object().expect("messages") {
        if is_the_features(message.rsplit('.').next().unwrap_or(message)) {
            found.push(format!("message {message}"));
        }
        for (number, field) in fields.as_object().expect("fields") {
            if is_the_features(field["name"].as_str().expect("a field name")) {
                found.push(format!("field {message}.{number}"));
            }
        }
    }
    for (service, methods) in baseline["services"].as_object().expect("services") {
        if is_the_features(service.rsplit('.').next().unwrap_or(service)) {
            found.push(format!("service {service}"));
        }
        for method in methods.as_object().expect("methods").keys() {
            if is_the_features(method) {
                found.push(format!("method {service}/{method}"));
            }
        }
    }
    found.sort();
    found
}

fn enum_names(owner: &str, e: &EnumDescriptorProto, found: &mut Vec<String>) {
    if is_the_features(e.name()) {
        found.push(format!("enum {owner}.{}", e.name()));
    }
    for value in &e.value {
        if is_the_features(value.name()) {
            found.push(format!("enum value {owner}.{}.{}", e.name(), value.name()));
        }
    }
}

fn message_names(owner: &str, message: &DescriptorProto, found: &mut Vec<String>) {
    let here = format!("{owner}.{}", message.name());
    if is_the_features(message.name()) {
        found.push(format!("message {here}"));
    }
    for field in &message.field {
        if is_the_features(field.name()) {
            found.push(format!("field {here}.{}", field.name()));
        }
    }
    for nested in &message.nested_type {
        message_names(&here, nested, found);
    }
    for e in &message.enum_type {
        enum_names(&here, e, found);
    }
}

fn service_names(service: &ServiceDescriptorProto, found: &mut Vec<String>) {
    if is_the_features(service.name()) {
        found.push(format!("service {}", service.name()));
    }
    for method in &service.method {
        if is_the_features(method.name()) {
            found.push(format!("method {}/{}", service.name(), method.name()));
        }
    }
}

/// The names of the compiled `worker.proto` that are the feature's.
fn proto_names(set: &FileDescriptorSet) -> Vec<String> {
    let mut found = Vec::new();
    for file in &set.file {
        let package = file.package().to_owned();
        for message in &file.message_type {
            message_names(&package, message, &mut found);
        }
        for e in &file.enum_type {
            enum_names(&package, e, &mut found);
        }
        for service in &file.service {
            service_names(service, &mut found);
        }
    }
    found.sort();
    found
}

fn worker_proto() -> FileDescriptorSet {
    protox::compile(["worker.proto"], [contracts()]).expect("worker.proto compiles")
}

#[test]
fn the_workflow_schema_holds_one_name_of_this_feature_the_optional_field_of_an_update() {
    let schema = read_json("workflow.schema.json");

    assert_eq!(
        schema_names(&schema),
        vec!["/$defs/update/properties/reapplyOnReset".to_owned()],
        "the workflow IR gains only `reapplyOnReset` from this feature (FR-018-046, T018-078)"
    );

    let update = &schema["$defs"]["update"];
    let field = &update["properties"]["reapplyOnReset"];
    assert_eq!(field["type"], "boolean");
    assert_eq!(field["default"], json!(false));
    assert!(
        !update["required"]
            .as_array()
            .expect("an Update declaration lists what it requires")
            .contains(&json!("reapplyOnReset")),
        "an optional field: a document that does not write it stays valid"
    );
}

#[test]
fn the_worker_protocol_and_its_baseline_hold_no_name_of_this_feature() {
    assert_eq!(
        baseline_names(&read_json("worker.baseline.json")),
        Vec::<String>::new(),
        "worker.baseline.json is the worker protocol that was published: this feature adds nothing to it (FR-018-046)"
    );
    assert_eq!(
        proto_names(&worker_proto()),
        Vec::<String>::new(),
        "worker.proto: this feature adds nothing to the worker protocol (FR-018-046)"
    );
}

#[test]
fn the_checks_find_a_name_of_the_feature_planted_in_a_copy_of_each_document() {
    // the IR: a property named after the feature, beside the declared one and under another definition
    let mut schema = read_json("workflow.schema.json");
    schema["$defs"]["update"]["properties"]["scheduleId"] = json!({"type": "string"});
    schema["properties"]["businessId"] = json!({"type": "string"});
    schema["$defs"]["nominalTime"] = json!({"type": "string"});
    assert_eq!(
        schema_names(&schema),
        vec![
            "/$defs/nominalTime".to_owned(),
            "/$defs/update/properties/reapplyOnReset".to_owned(),
            "/$defs/update/properties/scheduleId".to_owned(),
            "/properties/businessId".to_owned(),
        ]
    );

    // the baseline: a message, a field of another message, a service and a method
    let mut baseline = read_json("worker.baseline.json");
    baseline["messages"]["sagat_octopus_orchestrator.v1.ResetToken"] = json!({});
    baseline["messages"]["sagat_octopus_orchestrator.v1.HeartbeatRequest"]["77"] =
        json!({"name": "nominal_time", "repeated": false, "type": "String"});
    baseline["services"]["sagat_octopus_orchestrator.v1.ScheduleService"] = json!({});
    baseline["services"]["sagat_octopus_orchestrator.v1.WorkerService"]["Backfill"] = json!({"input": ".x.A", "output": ".x.B", "clientStreaming": false, "serverStreaming": false});
    assert_eq!(
        baseline_names(&baseline),
        vec![
            "field sagat_octopus_orchestrator.v1.HeartbeatRequest.77".to_owned(),
            "message sagat_octopus_orchestrator.v1.ResetToken".to_owned(),
            "method sagat_octopus_orchestrator.v1.WorkerService/Backfill".to_owned(),
            "service sagat_octopus_orchestrator.v1.ScheduleService".to_owned(),
        ]
    );

    // the compiled protocol: a field, a nested message, an enum value and a method
    let mut set = worker_proto();
    let file = set
        .file
        .iter_mut()
        .find(|file| file.package() == "sagat_octopus_orchestrator.v1")
        .expect("the worker package");
    let heartbeat = file
        .message_type
        .iter_mut()
        .find(|message| message.name() == "HeartbeatRequest")
        .expect("HeartbeatRequest");
    heartbeat.field.push(prost_types::FieldDescriptorProto {
        name: Some("business_id".to_owned()),
        ..Default::default()
    });
    heartbeat.nested_type.push(DescriptorProto {
        name: Some("ResetPoint".to_owned()),
        ..Default::default()
    });
    file.service[0]
        .method
        .push(prost_types::MethodDescriptorProto {
            name: Some("RunSchedule".to_owned()),
            ..Default::default()
        });
    assert_eq!(
        proto_names(&set),
        vec![
            "field sagat_octopus_orchestrator.v1.HeartbeatRequest.business_id".to_owned(),
            "message sagat_octopus_orchestrator.v1.HeartbeatRequest.ResetPoint".to_owned(),
            "method WorkerService/RunSchedule".to_owned(),
        ]
    );
}

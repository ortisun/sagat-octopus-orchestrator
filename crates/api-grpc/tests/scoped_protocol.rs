#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use prost::Message;
use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{DescriptorProto, FieldDescriptorProto, FileDescriptorSet};
use sagat_api_grpc::pb;

fn contract() -> FileDescriptorSet {
    let contracts = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../specs/001-platform-foundation/contracts");
    protox::compile(["worker.proto"], [contracts]).expect("worker.proto compiles")
}

fn message<'a>(set: &'a FileDescriptorSet, name: &str) -> &'a DescriptorProto {
    set.file
        .iter()
        .flat_map(|file| &file.message_type)
        .find(|message| message.name() == name)
        .unwrap_or_else(|| panic!("missing message {name}"))
}

fn fields(message: &DescriptorProto) -> BTreeMap<i32, &FieldDescriptorProto> {
    message
        .field
        .iter()
        .map(|field| (field.number(), field))
        .collect()
}

fn assert_message_field(
    fields: &BTreeMap<i32, &FieldDescriptorProto>,
    number: i32,
    name: &str,
    type_name: &str,
    repeated: bool,
) {
    let field = fields
        .get(&number)
        .unwrap_or_else(|| panic!("missing field {number}"));
    assert_eq!(field.name(), name);
    assert_eq!(field.r#type(), Type::Message);
    assert_eq!(field.type_name(), type_name);
    assert_eq!(field.label() == Label::Repeated, repeated);
}

#[test]
fn scoped_identity_has_stable_structural_field_numbers() {
    let contract = contract();
    let segments = fields(message(&contract, "ScopePathSegment"));
    assert_eq!(segments[&1].name(), "scope_node_id");
    assert_eq!(segments[&1].r#type(), Type::String);
    assert_eq!(segments[&2].name(), "iteration_index");
    assert_eq!(segments[&2].r#type(), Type::Uint32);

    let identity = fields(message(&contract, "ExecutionInstanceIdentity"));
    assert_message_field(
        &identity,
        1,
        "instance_path",
        ".sagat_octopus_orchestrator.v1.ScopePathSegment",
        true,
    );
    assert_eq!(identity[&2].name(), "definition_node_id");
    assert_eq!(identity[&2].r#type(), Type::String);
}

#[test]
fn every_attempt_capability_carries_the_same_instance_identity() {
    let contract = contract();
    let identity = ".sagat_octopus_orchestrator.v1.ExecutionInstanceIdentity";
    for (message_name, field_number) in [
        ("HeartbeatRequest", 8),
        ("CompleteStepRequest", 12),
        ("FailStepRequest", 12),
        ("ResolveSecretsRequest", 8),
        ("WriteBlobHeader", 10),
        ("ReadBlobRequest", 8),
    ] {
        assert_message_field(
            &fields(message(&contract, message_name)),
            field_number,
            "instance",
            identity,
            false,
        );
    }

    let lease = fields(message(&contract, "TaskLease"));
    assert_message_field(&lease, 17, "instance", identity, false);
    assert_message_field(&lease, 18, "target_instance", identity, false);
}

#[test]
fn generated_messages_preserve_scoped_paths_and_legacy_absence() {
    let scoped = pb::CompleteStepRequest {
        task_id: "0198c5a0-0000-7000-8000-000000000001/charge/3".into(),
        run_id: "0198c5a0-0000-7000-8000-000000000001".into(),
        step_id: "0198c5a0-0000-7000-8000-000000000002".into(),
        attempt: 2,
        generation: 3,
        lease_token: "lease".into(),
        idempotency_key: "attempt-2".into(),
        output_json: "null".into(),
        output_blob_ref: String::new(),
        output_sha256: String::new(),
        traceparent: String::new(),
        instance: Some(pb::ExecutionInstanceIdentity {
            instance_path: vec![
                pb::ScopePathSegment {
                    scope_node_id: "lines".into(),
                    iteration_index: 7,
                },
                pb::ScopePathSegment {
                    scope_node_id: "retries".into(),
                    iteration_index: 2,
                },
            ],
            definition_node_id: "charge".into(),
        }),
    };
    let decoded = pb::CompleteStepRequest::decode(scoped.encode_to_vec().as_slice())
        .expect("generated message round trips");
    assert_eq!(decoded, scoped);

    let legacy = pb::CompleteStepRequest {
        instance: None,
        ..scoped
    };
    let decoded = pb::CompleteStepRequest::decode(legacy.encode_to_vec().as_slice())
        .expect("legacy message round trips");
    assert!(decoded.instance.is_none());
}

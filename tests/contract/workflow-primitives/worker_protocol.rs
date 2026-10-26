//! The additive worker wire shape for a logical instance. These tests compile the canonical
//! protobuf directly so they do not mistake generated adapter behavior for the public contract.

use std::collections::BTreeMap;
use std::path::PathBuf;

use prost_types::{DescriptorProto, FileDescriptorSet, field_descriptor_proto};

const PACKAGE: &str = "sagat_octopus_orchestrator.v1";

fn contracts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../specs/001-platform-foundation/contracts")
}

fn descriptors() -> FileDescriptorSet {
    protox::compile(["worker.proto"], [contracts()]).expect("canonical worker.proto compiles")
}

fn messages(set: &FileDescriptorSet) -> BTreeMap<String, &DescriptorProto> {
    set.file
        .iter()
        .flat_map(|file| {
            let package = file.package();
            file.message_type
                .iter()
                .map(move |message| (format!("{package}.{}", message.name()), message))
        })
        .collect()
}

fn assert_field(
    message: &DescriptorProto,
    number: i32,
    name: &str,
    kind: field_descriptor_proto::Type,
    type_name: &str,
    repeated: bool,
) {
    let field = message
        .field
        .iter()
        .find(|field| field.number() == number)
        .unwrap_or_else(|| panic!("{} has no field {number}", message.name()));
    assert_eq!(field.name(), name);
    assert_eq!(field.r#type(), kind);
    assert_eq!(field.type_name(), type_name);
    assert_eq!(
        field.label(),
        if repeated {
            field_descriptor_proto::Label::Repeated
        } else {
            field_descriptor_proto::Label::Optional
        }
    );
}

#[test]
fn scoped_identity_is_typed_and_attempt_independent() {
    let set = descriptors();
    let messages = messages(&set);
    let segment = messages
        .get(&format!("{PACKAGE}.ScopePathSegment"))
        .expect("typed scope path segment");
    assert_field(
        segment,
        1,
        "scope_node_id",
        field_descriptor_proto::Type::String,
        "",
        false,
    );
    assert_field(
        segment,
        2,
        "iteration_index",
        field_descriptor_proto::Type::Uint32,
        "",
        false,
    );

    let identity = messages
        .get(&format!("{PACKAGE}.ExecutionInstanceIdentity"))
        .expect("logical execution identity");
    assert_field(
        identity,
        1,
        "instance_path",
        field_descriptor_proto::Type::Message,
        &format!(".{PACKAGE}.ScopePathSegment"),
        true,
    );
    assert_field(
        identity,
        2,
        "definition_node_id",
        field_descriptor_proto::Type::String,
        "",
        false,
    );
    assert!(
        identity.field.iter().all(|field| !matches!(
            field.name(),
            "attempt" | "generation" | "lease_token" | "task_id" | "step_id"
        )),
        "retry and lease fencing are separate from logical identity"
    );
}

#[test]
fn leases_reports_and_capabilities_carry_the_same_identity_message() {
    let set = descriptors();
    let messages = messages(&set);
    let identity_type = format!(".{PACKAGE}.ExecutionInstanceIdentity");
    for (message_name, field_number) in [
        ("TaskLease", 17),
        ("HeartbeatRequest", 8),
        ("CompleteStepRequest", 12),
        ("FailStepRequest", 12),
        ("ResolveSecretsRequest", 8),
        ("WriteBlobHeader", 10),
        ("ReadBlobRequest", 8),
    ] {
        let message = messages
            .get(&format!("{PACKAGE}.{message_name}"))
            .unwrap_or_else(|| panic!("{message_name} exists"));
        assert_field(
            message,
            field_number,
            "instance",
            field_descriptor_proto::Type::Message,
            &identity_type,
            false,
        );
    }

    let lease = messages
        .get(&format!("{PACKAGE}.TaskLease"))
        .expect("TaskLease exists");
    assert_field(
        lease,
        18,
        "target_instance",
        field_descriptor_proto::Type::Message,
        &identity_type,
        false,
    );
}

//! The worker protocol additions of feature 035 (contracts/worker-protocol.md): `ResolvedDatabase` gains three
//! connection kinds, additively.
//!
//! That no existing number is reused, renumbered or retyped is the job of the compatibility test of the protocol
//! (`crates/api-grpc/tests/compatibility.rs`, with `worker.baseline.json` next to the contract), which this feature's
//! messages are now part of. The twelve fields `ResolvedDatabase` had before the feature are written out below, so a
//! change to one of them fails here with the feature's name on it.

use prost_types::{DescriptorProto, FileDescriptorSet, field_descriptor_proto};

use super::repository_root;

const PACKAGE: &str = "sagat_octopus_orchestrator.v1";

/// `ResolvedDatabase` before feature 035: number and name.
const OLD_FIELDS: [(i32, &str); 12] = [
    (1, "connection_id"),
    (2, "connection_revision"),
    (3, "postgres"),
    (4, "operation"),
    (5, "statement"),
    (6, "parameters"),
    (7, "limits"),
    (8, "credential"),
    (9, "lease_expires_at_epoch_ms"),
    (10, "namespace_id"),
    (11, "connection_name"),
    (12, "tenant_id"),
];

fn descriptors() -> FileDescriptorSet {
    let contracts = repository_root().join("specs/001-platform-foundation/contracts");
    protox::compile(["worker.proto"], [contracts]).expect("canonical worker.proto compiles")
}

fn message<'a>(set: &'a FileDescriptorSet, name: &str) -> &'a DescriptorProto {
    set.file
        .iter()
        .flat_map(|file| file.message_type.iter())
        .find(|message| message.name() == name)
        .unwrap_or_else(|| panic!("worker.proto has no message {name}"))
}

#[test]
fn resolved_database_keeps_its_old_fields_and_gains_the_three_connection_kinds() {
    let set = descriptors();
    let resolved = message(&set, "ResolvedDatabase");

    for (number, name) in OLD_FIELDS {
        let field = resolved
            .field
            .iter()
            .find(|field| field.number() == number)
            .unwrap_or_else(|| panic!("ResolvedDatabase lost field {number} ({name})"));
        assert_eq!(field.name(), name, "number {number} keeps its name");
    }

    for (field_name, type_name) in [
        ("iceberg", "IcebergConnection"),
        ("object_storage", "ObjectStorageConnection"),
        ("clickhouse", "ClickHouseConnection"),
    ] {
        let field = resolved
            .field
            .iter()
            .find(|field| field.name() == field_name)
            .unwrap_or_else(|| panic!("ResolvedDatabase has no field {field_name}"));
        assert_eq!(
            field.r#type(),
            field_descriptor_proto::Type::Message,
            "{field_name}"
        );
        assert_eq!(field.type_name(), format!(".{PACKAGE}.{type_name}"));
        assert_eq!(
            field.label(),
            field_descriptor_proto::Label::Optional,
            "{field_name} is optional: an old runner never sets it"
        );
        assert!(
            OLD_FIELDS
                .iter()
                .all(|(number, _)| *number != field.number()),
            "{field_name} takes number {}, which an old field already holds",
            field.number()
        );
        assert!(
            field.number() > 12,
            "{field_name} takes a number after the old ones"
        );

        // Each holds the pinned non-secret settings and `map<string, bytes> credentials`.
        let connection = message(&set, type_name);
        let credentials = connection
            .field
            .iter()
            .find(|field| field.name() == "credentials")
            .unwrap_or_else(|| panic!("{type_name} has no credentials map"));
        assert_eq!(credentials.label(), field_descriptor_proto::Label::Repeated);
        let entry = connection
            .nested_type
            .iter()
            .find(|nested| nested.options.as_ref().is_some_and(|o| o.map_entry()))
            .unwrap_or_else(|| panic!("{type_name}.credentials is not a map"));
        let key = entry
            .field
            .iter()
            .find(|f| f.number() == 1)
            .expect("map key");
        let value = entry
            .field
            .iter()
            .find(|f| f.number() == 2)
            .expect("map value");
        assert_eq!(key.r#type(), field_descriptor_proto::Type::String);
        assert_eq!(value.r#type(), field_descriptor_proto::Type::Bytes);
    }

    let numbers: Vec<i32> = resolved.field.iter().map(|field| field.number()).collect();
    let mut unique = numbers.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), numbers.len(), "no number is used twice");
}

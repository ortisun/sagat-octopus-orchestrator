//! The worker protocol additions of feature 035 (contracts/worker-protocol.md): `ResolvedDatabase` gains three
//! connection kinds, additively, and no field or enum number that exists today is reused, renumbered or retyped.
//!
//! The baseline (`fixtures/worker-proto-fields.json`) lists every message field and enum value of the protocol
//! before the feature. A number added later is fine; a number that moved is not. After a deliberate change, read
//! the diff and rerun with `SAGAT_CONTRACT_BASELINE_BLESS=1`, then without it.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use prost_types::{DescriptorProto, FileDescriptorSet, field_descriptor_proto};
use serde_json::json;

use super::{fixtures, read_json, repository_root};

const PACKAGE: &str = "sagat_octopus_orchestrator.v1";

type Numbers = BTreeMap<String, BTreeMap<String, String>>;

fn contracts() -> PathBuf {
    repository_root().join("specs/001-platform-foundation/contracts")
}

fn descriptors() -> FileDescriptorSet {
    protox::compile(["worker.proto"], [contracts()]).expect("canonical worker.proto compiles")
}

fn message<'a>(set: &'a FileDescriptorSet, name: &str) -> &'a DescriptorProto {
    set.file
        .iter()
        .flat_map(|file| file.message_type.iter())
        .find(|message| message.name() == name)
        .unwrap_or_else(|| panic!("worker.proto has no message {name}"))
}

fn collect_message(prefix: &str, message: &DescriptorProto, numbers: &mut Numbers) {
    let name = format!("message {prefix}.{}", message.name());
    let fields = message
        .field
        .iter()
        .map(|field| {
            (
                field.number().to_string(),
                format!(
                    "{} {:?} {:?} {}",
                    field.name(),
                    field.label(),
                    field.r#type(),
                    field.type_name()
                ),
            )
        })
        .collect();
    numbers.insert(name.clone(), fields);
    let scope = format!("{prefix}.{}", message.name());
    for nested in &message.nested_type {
        collect_message(&scope, nested, numbers);
    }
    for enumeration in &message.enum_type {
        collect_enum(&scope, enumeration, numbers);
    }
}

fn collect_enum(
    prefix: &str,
    enumeration: &prost_types::EnumDescriptorProto,
    numbers: &mut Numbers,
) {
    numbers.insert(
        format!("enum {prefix}.{}", enumeration.name()),
        enumeration
            .value
            .iter()
            .map(|value| (value.number().to_string(), value.name().to_string()))
            .collect(),
    );
}

/// Every message field and enum value of the protocol: container, then number to what it is called and typed.
fn numbers(set: &FileDescriptorSet) -> Numbers {
    let mut numbers = Numbers::new();
    for file in &set.file {
        for message in &file.message_type {
            collect_message(file.package(), message, &mut numbers);
        }
        for enumeration in &file.enum_type {
            collect_enum(file.package(), enumeration, &mut numbers);
        }
    }
    numbers
}

#[test]
fn no_existing_number_is_reused_renumbered_or_retyped() {
    let current = numbers(&descriptors());
    let baseline_path = fixtures().join("worker-proto-fields.json");
    if std::env::var_os("SAGAT_CONTRACT_BASELINE_BLESS").is_some() {
        let text = serde_json::to_string_pretty(&json!({"numbers": current})).expect("JSON");
        fs::write(&baseline_path, text + "\n").expect("write the baseline");
    }
    let baseline: Numbers = serde_json::from_value(read_json(&baseline_path)["numbers"].clone())
        .expect("the baseline maps containers to numbers");
    assert!(
        baseline.len() >= 28,
        "the baseline holds the whole protocol as it was"
    );

    let mut broken = Vec::new();
    for (container, fields) in &baseline {
        let Some(now) = current.get(container) else {
            broken.push(format!("removed: {container}"));
            continue;
        };
        for (number, what) in fields {
            match now.get(number) {
                None => broken.push(format!("{container}: number {number} ({what}) is gone")),
                Some(found) if found != what => broken.push(format!(
                    "{container}: number {number} was `{what}`, is now `{found}`"
                )),
                Some(_) => {}
            }
        }
    }
    assert!(
        broken.is_empty(),
        "the protocol is additive:\n{}",
        broken.join("\n")
    );
}

#[test]
fn resolved_database_gains_the_three_connection_kinds_without_touching_the_old_fields() {
    let set = descriptors();
    let resolved = message(&set, "ResolvedDatabase");

    let baseline: Numbers = serde_json::from_value(
        read_json(&fixtures().join("worker-proto-fields.json"))["numbers"].clone(),
    )
    .expect("the baseline maps containers to numbers");
    let old = &baseline[&format!("message {PACKAGE}.ResolvedDatabase")];
    assert_eq!(old.len(), 12, "twelve fields before the feature");

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
            !old.contains_key(&field.number().to_string()),
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

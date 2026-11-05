//! The lake connections of the worker protocol (feature 035) carry credentials: `Debug` must never print them.
//!
//! A resolved secret and a resolved database already print as `<redacted>`; the three connection messages hold the
//! credentials of their secrets in a map of bytes, so a derived `Debug` would put them in a log line or a panic
//! message. The generated derive is skipped for them and replaced by a redacting one (see `build.rs` and `lib.rs`).
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use sagat_api_grpc::pb;

const SECRET: &str = "never-print-this-credential";

fn credentials() -> HashMap<String, Vec<u8>> {
    HashMap::from([
        ("catalog".to_string(), SECRET.as_bytes().to_vec()),
        ("storage".to_string(), SECRET.as_bytes().to_vec()),
    ])
}

fn printed(connection: &impl std::fmt::Debug) -> String {
    format!("{connection:?} / {connection:#?}")
}

#[test]
fn an_iceberg_connection_never_prints_its_credentials() {
    let connection = pb::IcebergConnection {
        warehouse_locations: vec!["s3://warehouse/".to_string()],
        credentials: credentials(),
        ..Default::default()
    };
    let text = printed(&connection);
    assert!(!text.contains(SECRET), "{text}");
    assert!(text.contains("<redacted>"), "{text}");
}

#[test]
fn an_object_storage_connection_never_prints_its_credentials() {
    let connection = pb::ObjectStorageConnection {
        prefix: "s3://exports/".to_string(),
        credentials: credentials(),
        ..Default::default()
    };
    let text = printed(&connection);
    assert!(!text.contains(SECRET), "{text}");
    assert!(text.contains("<redacted>"), "{text}");
}

#[test]
fn a_clickhouse_connection_never_prints_its_credentials() {
    let connection = pb::ClickHouseConnection {
        url: "https://clickhouse.example.com".to_string(),
        credentials: credentials(),
        ..Default::default()
    };
    let text = printed(&connection);
    assert!(!text.contains(SECRET), "{text}");
    assert!(text.contains("<redacted>"), "{text}");
}

/// The wrapper already redacts everything; this keeps it so while the new fields exist.
#[test]
fn a_resolved_database_with_a_lake_connection_prints_nothing_of_it() {
    let resolved = pb::ResolvedDatabase {
        iceberg: Some(pb::IcebergConnection {
            credentials: credentials(),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!printed(&resolved).contains(SECRET));
}

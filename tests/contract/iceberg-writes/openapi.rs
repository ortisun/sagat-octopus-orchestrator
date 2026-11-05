//! The REST additions of feature 035 (contracts/connections.md), merged into the canonical
//! `specs/001-platform-foundation/contracts/openapi.yaml` by T035-008. Every change is additive: the operations
//! that exist today keep their exact definition.
//!
//! The baseline of existing operations (`fixtures/openapi-operations.json`) is a digest of each operation and the
//! parameters of its path. When an operation changes on purpose, read the diff and rerun with
//! `SAGAT_CONTRACT_BASELINE_BLESS=1 cargo test -p sagat-tests --test contract_iceberg_writes`, then without it.

use std::collections::BTreeMap;
use std::fs;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{fixtures, read_json, repository_root};

const METHODS: [&str; 7] = ["get", "put", "post", "delete", "patch", "head", "options"];

fn openapi() -> Value {
    let path = repository_root().join("specs/001-platform-foundation/contracts/openapi.yaml");
    serde_saphyr::from_str(&fs::read_to_string(path).expect("canonical OpenAPI exists"))
        .expect("canonical OpenAPI is YAML")
}

fn schema<'a>(openapi: &'a Value, name: &str) -> &'a Value {
    &openapi["components"]["schemas"][name]
}

/// `METHOD /path` to the digest of the operation and the parameters declared once for its path.
fn operation_digests(openapi: &Value) -> BTreeMap<String, String> {
    let mut digests = BTreeMap::new();
    for (path, item) in openapi["paths"].as_object().expect("paths") {
        for method in METHODS {
            if let Some(operation) = item.get(method) {
                let definition =
                    json!({"parameters": item.get("parameters"), "operation": operation});
                let digest = Sha256::digest(serde_json::to_vec(&definition).expect("JSON"));
                digests.insert(
                    format!("{} {path}", method.to_uppercase()),
                    digest.iter().map(|byte| format!("{byte:02x}")).collect(),
                );
            }
        }
    }
    digests
}

/// Follows a `$ref` into `components.schemas`, as many times as it takes.
fn resolved<'a>(openapi: &'a Value, node: &'a Value) -> &'a Value {
    match node["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix("#/components/schemas/"))
    {
        Some(name) => resolved(openapi, schema(openapi, name)),
        None => node,
    }
}

fn blessing() -> bool {
    std::env::var_os("SAGAT_CONTRACT_BASELINE_BLESS").is_some()
}

#[test]
fn existing_operations_keep_their_exact_definition() {
    let current = operation_digests(&openapi());
    let baseline_path = fixtures().join("openapi-operations.json");
    if blessing() {
        let text = serde_json::to_string_pretty(&json!({"operations": current})).expect("JSON");
        fs::write(&baseline_path, text + "\n").expect("write the baseline");
    }
    let baseline: BTreeMap<String, String> =
        serde_json::from_value(read_json(&baseline_path)["operations"].clone())
            .expect("the baseline maps operations to digests");
    assert!(
        baseline.len() > 100,
        "the baseline holds every operation that existed before feature 035"
    );

    let mut changed = Vec::new();
    for (operation, digest) in &baseline {
        match current.get(operation) {
            None => changed.push(format!("removed: {operation}")),
            Some(now) if now != digest => changed.push(format!("changed: {operation}")),
            Some(_) => {}
        }
    }
    assert!(
        changed.is_empty(),
        "feature 035 is additive; these operations differ from the baseline:\n{}",
        changed.join("\n")
    );
}

#[test]
fn database_connection_config_is_a_one_of_discriminated_by_driver() {
    let openapi = openapi();
    let config = resolved(
        &openapi,
        &schema(&openapi, "DatabaseConnection")["properties"]["config"],
    );
    let request = resolved(
        &openapi,
        &schema(&openapi, "PutDatabaseConnectionRequest")["properties"]["config"],
    );
    assert_eq!(
        request, config,
        "a connection is written with every variant it can be read with"
    );
    let branches: Vec<String> = config["oneOf"]
        .as_array()
        .unwrap_or_else(|| panic!("DatabaseConnection.config is not a oneOf: {config}"))
        .iter()
        .map(|branch| {
            branch["$ref"]
                .as_str()
                .expect("a reference")
                .trim_start_matches("#/components/schemas/")
                .to_string()
        })
        .collect();
    assert_eq!(
        branches,
        [
            "PostgresConnectionConfig",
            "IcebergConnectionConfig",
            "ObjectStorageConnectionConfig",
            "ClickHouseConnectionConfig"
        ]
    );
    assert_eq!(config["discriminator"]["propertyName"], "driver");
}

#[test]
fn every_connection_config_is_closed_and_names_its_driver() {
    let openapi = openapi();
    for (name, driver) in [
        ("PostgresConnectionConfig", "postgresql"),
        ("IcebergConnectionConfig", "iceberg"),
        ("ObjectStorageConnectionConfig", "object_storage"),
        ("ClickHouseConnectionConfig", "clickhouse"),
    ] {
        let config = schema(&openapi, name);
        assert_eq!(config["type"], "object", "{name} exists");
        assert_eq!(config["additionalProperties"], false, "{name} is closed");
        assert_eq!(
            config["properties"]["driver"]["const"], driver,
            "{name} names its driver"
        );
        assert_eq!(
            resolved(&openapi, &config["properties"]["egressCeiling"])["enum"],
            json!(["public", "internal", "pii"]),
            "{name} carries the egress ceiling, and `secret` is not a value"
        );
    }
}

#[test]
fn the_driver_list_names_all_four_drivers() {
    let openapi = openapi();
    let list = &schema(&openapi, "DatabaseDriverList")["properties"]["items"];
    assert_eq!(list["maxItems"], 4, "{list}");
    let ids: Vec<Value> = list["items"]["oneOf"]
        .as_array()
        .unwrap_or_else(|| panic!("a driver is one of two shapes: {list}"))
        .iter()
        .flat_map(|branch| {
            resolved(&openapi, branch)["properties"]["id"]["enum"]
                .as_array()
                .expect("the ids a shape names")
                .clone()
        })
        .collect();
    assert_eq!(
        ids,
        [
            json!("postgresql"),
            json!("iceberg"),
            json!("object_storage"),
            json!("clickhouse")
        ]
    );
}

/// An operation is `x-planned: '035'` while the contract is merged and no route serves it (the cross-tenant battery
/// attacks every `x-implemented` operation live, and a planned one is listed there by name), and
/// `x-implemented: '035'` from the task that adds its route (D035-073).
fn belongs_to_035(operation: &Value) -> bool {
    operation["x-planned"] == "035" || operation["x-implemented"] == "035"
}

#[test]
fn the_catalog_browse_operations_exist_and_belong_to_035() {
    let openapi = openapi();
    let start = &openapi["paths"]["/namespaces/{namespaceId}/connections/{connectionName}/catalog-browse"]
        ["post"];
    assert!(belongs_to_035(start), "start a browse: {start}");
    assert!(
        start["responses"].get("202").is_some(),
        "a browse is admitted, not answered"
    );
    let read = &openapi["paths"]["/namespaces/{namespaceId}/catalog-browses/{browseId}"]["get"];
    assert!(belongs_to_035(read), "read a browse: {read}");
    for (what, operation) in [("start", start), ("read", read)] {
        assert!(
            operation["responses"].get("404").is_some(),
            "{what}: another tenant's connection and an unknown browse are a 404"
        );
    }
}

#[test]
fn subscriptions_carry_a_purpose_and_an_egress_ceiling_when_created_and_read() {
    let openapi = openapi();
    for name in ["CreateWebhookSubscriptionRequest", "WebhookSubscription"] {
        let properties = &schema(&openapi, name)["properties"];
        assert_eq!(
            properties["purpose"]["enum"],
            json!(["RUN_EVENTS", "STEP_OUTPUT"]),
            "{name}.purpose"
        );
        assert_eq!(
            resolved(&openapi, &properties["egressCeiling"])["enum"],
            json!(["public", "internal", "pii"]),
            "{name}.egressCeiling"
        );
    }
    assert_eq!(
        schema(&openapi, "CreateWebhookSubscriptionRequest")["properties"]["purpose"]["default"],
        "RUN_EVENTS",
        "an existing client that sends no purpose keeps run events"
    );
}

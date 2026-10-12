//! The runner incident request is deliberately too small to carry a URL, hostname, headers,
//! payload, actor or free-form reason into the durable audit trail.

use serde_json::{Value, json};

fn contract() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../specs/001-platform-foundation/contracts/openapi.yaml"
    );
    serde_yaml_ng::from_str(&std::fs::read_to_string(path).expect("OpenAPI")).expect("valid YAML")
}

fn request_schema(document: &Value) -> Value {
    let mut schema = document["paths"]["/script-runner/incidents"]["post"]["requestBody"]
        ["content"]["application/json"]["schema"]
        .clone();
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let path = reference.strip_prefix("#/").expect("local reference");
        schema = path
            .split('/')
            .fold(document, |node, key| &node[key])
            .clone();
    }
    schema
        .as_object_mut()
        .expect("schema object")
        .insert("components".into(), document["components"].clone());
    schema
}

fn validates(schema: &Value, value: &Value) -> bool {
    jsonschema::validator_for(schema)
        .expect("valid schema")
        .is_valid(value)
}

#[test]
fn incident_contract_is_tagged_closed_and_rejects_attacker_detail() {
    let document = contract();
    let schema = request_schema(&document);
    let production = json!({
        "target": {
            "kind": "production",
            "namespaceId": "019b5d6e-0000-7000-8000-000000000001",
            "runId": "019b5d6e-0000-7000-8000-000000000002",
            "stepId": "019b5d6e-0000-7000-8000-000000000003",
            "attempt": 1,
            "generation": 2,
            "leaseToken": "redacted-token"
        },
        "category": "NOT_ALLOWLISTED"
    });
    let test = json!({
        "target": {
            "kind": "test",
            "namespaceId": "019b5d6e-0000-7000-8000-000000000001",
            "testRunId": "019b5d6e-0000-7000-8000-000000000004",
            "leaseToken": "redacted-token"
        },
        "category": "EGRESS_DISABLED"
    });
    assert!(validates(&schema, &production));
    assert!(validates(&schema, &test));

    for field in [
        "actor", "digest", "reason", "host", "url", "headers", "payload",
    ] {
        let mut hostile = production.clone();
        hostile[field] = json!("secret-canary");
        assert!(!validates(&schema, &hostile), "accepted {field}");
    }
    let mut open_category = production;
    open_category["category"] = json!("fetch https://secret.example/private?canary=1");
    assert!(!validates(&schema, &open_category));
}

#[test]
fn incident_ack_and_typed_failures_are_documented() {
    let document = contract();
    let responses = &document["paths"]["/script-runner/incidents"]["post"]["responses"];
    for status in ["204", "401", "403", "409", "422", "429", "503", "default"] {
        assert!(responses.get(status).is_some(), "missing response {status}");
    }
}

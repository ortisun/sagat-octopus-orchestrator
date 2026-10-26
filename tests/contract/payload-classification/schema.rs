//! Additive workflow-IR carrier contract (T016-005/T016-006).

use std::fs;
use std::sync::OnceLock;

use jsonschema::Validator;
use serde_json::{Value, json};

fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let path = super::repository_root()
            .join("specs/001-platform-foundation/contracts/workflow.schema.json");
        serde_json::from_str(&fs::read_to_string(path).expect("canonical workflow schema exists"))
            .expect("workflow schema is JSON")
    })
}

fn validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| jsonschema::validator_for(schema()).expect("schema compiles"))
}

fn workflow(node: Value) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "classification-contract"},
        "entrypoint": node["id"].clone(),
        "nodes": [node]
    })
}

fn node(kind: &str, output_schema: Value) -> Value {
    match kind {
        "activity" => json!({
            "id": "work", "type": "activity", "activity": "orders.read",
            "outputSchema": output_schema
        }),
        "script" => json!({
            "id": "work", "type": "script",
            "artifactDigest": format!("sha256:{}", "0".repeat(64)),
            "outputSchema": output_schema
        }),
        "waitEvent" => json!({
            "id": "work", "type": "waitEvent", "eventType": "order.ready",
            "correlationExpr": "input.orderId", "timeoutMs": 1_000,
            "outputSchema": output_schema
        }),
        "child" => json!({
            "id": "work", "type": "child", "workflowId": "child-flow",
            "outputSchema": output_schema
        }),
        other => panic!("unsupported test node {other}"),
    }
}

fn errors(instance: &Value) -> Vec<String> {
    validator()
        .iter_errors(instance)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect()
}

#[test]
fn only_the_four_output_producers_declare_node_output_schema_carriers() {
    let definitions = schema()["$defs"].as_object().expect("$defs object");
    let declared = definitions
        .iter()
        .filter_map(|(name, definition)| {
            (definition
                .pointer("/allOf/1/properties/outputSchema")
                .is_some()
                || definition.pointer("/properties/outputSchema").is_some())
            .then_some(name.as_str())
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        declared,
        [
            "activity",
            "bodyActivity",
            "bodyChild",
            "bodyScript",
            "bodyWaitEvent",
            "child",
            "script",
            "waitEvent",
        ]
        .into()
    );
    assert!(
        !declared.contains("database"),
        "feature 017 owns database outputs"
    );
}

#[test]
fn each_node_output_schema_carrier_accepts_an_object_and_refuses_non_objects() {
    for kind in ["activity", "script", "waitEvent", "child"] {
        let valid = workflow(node(
            kind,
            json!({
                "type": "object",
                "x-sagat-classification": "public",
                "properties": {"receipt": {"type": "string"}}
            }),
        ));
        assert!(errors(&valid).is_empty(), "{kind}: {:?}", errors(&valid));

        for invalid_schema in [json!(null), json!("object"), json!([]), json!(42)] {
            let invalid = workflow(node(kind, invalid_schema));
            assert!(
                !errors(&invalid).is_empty(),
                "{kind} accepted a non-object outputSchema: {invalid}"
            );
        }
    }
}

#[test]
fn existing_sdk_workflow_corpus_remains_valid() {
    let directory = super::repository_root().join("tests/contract/sdk-conformance/expected");
    let mut checked = 0usize;
    for entry in fs::read_dir(directory).expect("SDK expected corpus") {
        let path = entry.expect("directory entry").path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".workflows.json"))
        {
            continue;
        }
        let fixture: Value = serde_json::from_str(
            &fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
        )
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let Some(workflows) = fixture.get("workflows").and_then(Value::as_object) else {
            continue;
        };
        for (name, document) in workflows {
            let failures = errors(document);
            assert!(
                failures.is_empty(),
                "{} workflow {name}: {failures:?}",
                path.display()
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 20,
        "expected a substantial existing workflow corpus"
    );
}

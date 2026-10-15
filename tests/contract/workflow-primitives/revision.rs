//! Schema shape checks do not prove graph closure, expression validity or runtime support.

use jsonschema::Validator;
use serde_json::{Value, json};
use std::sync::OnceLock;

fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let path = super::repository_root()
            .join("specs/001-platform-foundation/contracts/workflow.schema.json");
        let source = std::fs::read_to_string(path).expect("canonical workflow schema exists");
        serde_json::from_str(&source).expect("workflow schema is JSON")
    })
}

fn validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        jsonschema::options()
            .build(schema())
            .expect("workflow schema compiles without external retrieval")
    })
}

fn assert_valid(document: &Value) {
    let errors: Vec<String> = validator()
        .iter_errors(document)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

fn minimal() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow", "metadata": {"name": "bounded-orders"},
        "entrypoint": "done", "nodes": [{"id": "done", "type": "end"}]
    })
}

fn collection() -> Value {
    let mut document = minimal();
    document["entrypoint"] = json!("lines");
    document["nodes"] = json!([
        {"id": "lines", "type": "forEach", "items": "input.items", "body": "line",
         "result": "bodyResult", "maxItems": 17, "maxConcurrency": 3, "next": "done"},
        {"id": "done", "type": "end"}
    ]);
    document["bodies"] = json!({"line": {"entrypoint": "work", "nodes": [
        {"id": "work", "type": "activity", "activity": "line.process", "next": "return"},
        {"id": "return", "type": "return", "result": "steps.work.output"}
    ]}});
    document
}

fn operations() -> Value {
    let mut document = minimal();
    document["state"] = json!({
        "schema": {"type": "integer"}, "initial": "0",
        "queries": {"count": {"argsSchema": {"type": "object"},
            "resultSchema": {"type": "integer"}, "expression": "state",
            "disclosure": "declared-business-result-v1"}},
        "updates": {"increment": {"argsSchema": {"type": "integer"},
            "validate": "args > 0", "state": "state + args", "result": "newState",
            "resultSchema": {"type": "integer"},
            "disclosure": "declared-business-result-v1"}}
    });
    document
}

#[test]
fn additive_capabilities_keep_the_existing_api_version() {
    assert_valid(&minimal());
    assert_valid(&collection());
    assert_valid(&operations());
    let mut unsupported = minimal();
    unsupported["apiVersion"] = json!("sagat-octopus-orchestrator.io/v2");
    assert!(!validator().is_valid(&unsupported));
    unsupported["apiVersion"] = json!("sagat-octopus-orchestrator.io/v1");
    unsupported["nodes"][0]["type"] = json!("unknownNode");
    assert!(!validator().is_valid(&unsupported));
    assert_eq!(schema()["properties"]["limits"]["$ref"], "#/$defs/limits");
    assert_eq!(
        schema()["$defs"]["limits"]["properties"]["maxInstances"]["default"],
        100_000
    );
    assert_eq!(
        schema()["$defs"]["limits"]["properties"]["maxDepth"]["default"],
        8
    );
}

#[test]
fn existing_documents_and_schema_forms_remain_accepted() {
    for name in ["routing", "fulfillment", "reminder"] {
        let path = super::repository_root().join(format!(
            "examples/order-processing/workflows/{name}.workflow.json"
        ));
        let source = std::fs::read_to_string(path).expect("existing example exists");
        let document = serde_json::from_str(&source).expect("existing example is JSON");
        assert_valid(&document);
    }
    let legacy_nodes = [
        json!({"id": "done", "type": "activity", "activity": "order.process"}),
        json!({"id": "done", "type": "timer", "durationMs": 1}),
        json!({"id": "done", "type": "waitEvent", "eventType": "approved",
            "correlationExpr": "input.order", "timeoutMs": 1}),
        json!({"id": "done", "type": "condition", "expression": "true",
            "ifTrue": "yes", "ifFalse": "no"}),
        json!({"id": "done", "type": "parallel", "branches": ["left", "right"], "join": "all"}),
        json!({"id": "done", "type": "child", "workflowId": "order-process"}),
        json!({"id": "done", "type": "saga", "begin": "start"}),
        json!({"id": "done", "type": "script", "artifactDigest": format!("sha256:{}", "a".repeat(64))}),
        json!({"id": "done", "type": "end", "outputExpr": "input"}),
        json!({"id": "done", "type": "foreach", "collectionExpr": "input.items",
            "itemKeyExpr": "item.id", "body": "work", "maxItems": 100_000}),
        json!({"id": "done", "type": "humanApproval", "approverPolicy": "operator",
            "timeoutMs": 1, "formSchema": {}}),
    ];
    for node in legacy_nodes {
        let mut document = minimal();
        document["nodes"] = json!([node]);
        assert_valid(&document);
    }
    // Schema compatibility does not enable historical placeholder kinds in the codec.
    let mut legacy = minimal();
    legacy["nodes"][0]["outputExpr"] = json!("x".repeat(2048));
    assert_valid(&legacy);
}

#[test]
fn collection_bodies_have_a_single_return_and_no_workflow_terminal() {
    assert_valid(&collection());
    for forbidden in ["end", "continueAsNew"] {
        let mut document = collection();
        document["bodies"]["line"]["nodes"][1] = json!({"id": "return", "type": forbidden});
        assert!(!validator().is_valid(&document), "{forbidden}");
    }
    let mut missing = collection();
    missing["bodies"]["line"]["nodes"]
        .as_array_mut()
        .expect("nodes")
        .pop();
    assert!(!validator().is_valid(&missing));
    let mut duplicate = collection();
    duplicate["bodies"]["line"]["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(json!({"id": "other", "type": "return", "result": "null"}));
    assert!(!validator().is_valid(&duplicate));
    let mut root_return = minimal();
    root_return["nodes"][0] = json!({"id": "done", "type": "return", "result": "null"});
    assert!(!validator().is_valid(&root_return));
}

#[test]
fn publication_shape_enforces_hard_limits_and_rejects_unknown_fields() {
    let mut maximum = collection();
    maximum["nodes"][0]["maxItems"] = json!(10_000);
    maximum["nodes"][0]["maxConcurrency"] = json!(256);
    maximum["limits"] = json!({"maxInstances": 100_000, "maxDepth": 8});
    assert_valid(&maximum);
    for (pointer, value) in [
        ("/nodes/0/maxItems", json!(0)),
        ("/nodes/0/maxItems", json!(10_001)),
        ("/nodes/0/maxConcurrency", json!(0)),
        ("/nodes/0/maxConcurrency", json!(257)),
        ("/nodes/0/id", json!("x".repeat(65))),
        ("/nodes/0/id", json!("scope/node")),
        ("/nodes/0/items", json!("x".repeat(1025))),
        ("/limits/maxInstances", json!(100_001)),
        ("/limits/maxDepth", json!(9)),
    ] {
        let mut invalid = maximum.clone();
        *invalid.pointer_mut(pointer).expect("field") = value;
        assert!(!validator().is_valid(&invalid), "{pointer}");
    }
    let mut unknown = collection();
    unknown["nodes"][0]["itemKeyExpr"] = json!("item.id");
    assert!(!validator().is_valid(&unknown));
    let mut excessive = minimal();
    excessive["nodes"] = json!(vec![json!({"id": "done", "type": "end"}); 10_001]);
    assert!(!validator().is_valid(&excessive));
}

#[test]
fn loops_switches_and_approvals_require_their_bounded_configuration() {
    for kind in ["while", "repeatUntil"] {
        let mut document = collection();
        document["nodes"][0] = json!({"id": "lines", "type": kind, "body": "line",
            "initial": "0", "carry": "carry + 1", "condition": "carry < 3",
            "maxIterations": 10_000, "timeoutMs": 31_622_400_000_u64,
            "delayMs": 1, "next": "done"});
        assert_valid(&document);
        for field in ["maxIterations", "timeoutMs"] {
            let mut invalid = document.clone();
            invalid["nodes"][0]
                .as_object_mut()
                .expect("node")
                .remove(field);
            assert!(!validator().is_valid(&invalid), "{kind}/{field}");
        }
        document["nodes"][0]["delayMs"] = json!(0);
        assert!(!validator().is_valid(&document));
    }
    let mut switch = minimal();
    switch["nodes"][0] = json!({"id": "done", "type": "switch",
        "cases": [{"id": "positive", "when": "input.amount > 0", "target": "ok"}],
        "default": "no"});
    assert_valid(&switch);
    switch["nodes"][0]["cases"] =
        json!(vec![json!({"id": "x", "when": "true", "target": "ok"}); 33]);
    assert!(!validator().is_valid(&switch));
    let mut approval = minimal();
    approval["nodes"][0] = json!({"id": "done", "type": "approval", "timeoutMs": 1,
        "policy": {"subjects": ["approver"], "roles": ["operator"], "excludeInitiator": true},
        "onApprove": "yes", "onReject": "no", "onExpire": "late"});
    assert_valid(&approval);
    approval["nodes"][0]["policy"]["roles"] = json!(["viewer"]);
    assert!(!validator().is_valid(&approval));
    approval["nodes"][0]["policy"] = json!({"subjects": []});
    assert!(!validator().is_valid(&approval));
    approval["nodes"][0]["policy"] = json!({"subjects": (0..32).map(|n| format!("person{n}")).collect::<Vec<_>>(), "roles": ["operator"]});
    assert!(
        !validator().is_valid(&approval),
        "total principals exceed the combined limit"
    );
}

#[test]
fn declared_operations_require_explicit_disclosure_and_closed_bounded_results() {
    assert_valid(&operations());
    let mut undeclared = operations();
    undeclared["state"]["queries"]["count"]
        .as_object_mut()
        .expect("query")
        .remove("disclosure");
    assert!(!validator().is_valid(&undeclared));
    for result_schema in [
        json!({"type": "object", "properties": {}, "additionalProperties": true}),
        json!({"type": "string"}),
        json!({"type": "array", "items": {"type": "integer"}}),
        json!({"$ref": "https://example.invalid/data"}),
    ] {
        let mut invalid = operations();
        invalid["state"]["queries"]["count"]["resultSchema"] = result_schema;
        assert!(!validator().is_valid(&invalid));
    }
    let mut closed = operations();
    closed["state"]["queries"]["count"]["resultSchema"] = json!({"type": "object",
        "properties": {"count": {"type": "integer"}, "label": {"type": "string", "maxLength": 64}},
        "required": ["count"], "additionalProperties": false});
    assert_valid(&closed);
    let mut excessive = operations();
    let query = excessive["state"]["queries"]["count"].clone();
    let queries = excessive["state"]["queries"]
        .as_object_mut()
        .expect("queries");
    for index in 0..32 {
        queries.insert(format!("query{index}"), query.clone());
    }
    assert!(!validator().is_valid(&excessive));
}

#[test]
fn continuation_requires_chain_budget_and_explicit_declared_state_transfer() {
    let mut document = minimal();
    document["nodes"][0] = json!({"id": "done", "type": "continueAsNew", "input": "input"});
    assert!(!validator().is_valid(&document));
    document["chainPolicy"] = json!({"maxSegments": 10_000, "timeoutMs": 31_622_400_000_u64});
    assert_valid(&document);
    document["state"] = operations()["state"].clone();
    assert!(!validator().is_valid(&document));
    document["nodes"][0]["state"] = json!("state");
    assert_valid(&document);
    document["nodes"][0]["next"] = json!("elsewhere");
    assert!(!validator().is_valid(&document));
}

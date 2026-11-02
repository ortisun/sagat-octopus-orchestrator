#![allow(clippy::expect_used)]
use sagat_octopus_orchestrator_sdk::{
    json,
    workflow::{Builder, Join, StepOptions, canonical_json},
};

#[test]
fn typed_activity_timer_and_terminal_emit_the_common_ir() {
    let workflow = Builder::new("order-flow");
    workflow
        .step(
            "charge",
            "payments.charge",
            StepOptions {
                timeout_ms: Some(5000),
                ..Default::default()
            },
        )
        .sleep("pause", 2000)
        .end("done", Some("steps.charge.output"));
    assert_eq!(
        workflow.build().expect("valid workflow"),
        json!({
            "apiVersion":"sagat-octopus-orchestrator.io/v1", "kind":"Workflow",
            "metadata":{"name":"order-flow"}, "entrypoint":"charge", "nodes":[
                {"id":"charge","type":"activity","activity":"payments.charge","timeoutMs":5000,"next":"pause"},
                {"id":"pause","type":"timer","durationMs":2000,"next":"done"},
                {"id":"done","type":"end","outputExpr":"steps.charge.output"}
            ]
        })
    );
}

#[test]
fn joined_flows_are_read_at_build_and_flatten_after_top_level_nodes() {
    let left = Builder::flow();
    left.step("left", "one", StepOptions::default());
    let right = Builder::flow();
    right.step("right", "two", StepOptions::default());
    let workflow = Builder::new("parallel-flow");
    workflow
        .parallel("fan", Join::All, &[left.clone(), right])
        .end("done", None);
    left.sleep("late", 1);
    let doc = workflow.build().expect("joined workflow");
    assert_eq!(
        doc["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .map(|n| n["id"].as_str().expect("id"))
            .collect::<Vec<_>>(),
        ["fan", "done", "left", "late", "right"]
    );
    assert_eq!(doc["nodes"][2]["next"], "late");
    left.step("done", "duplicate", StepOptions::default());
    assert!(
        workflow
            .build()
            .expect_err("global duplicate")
            .message
            .contains("used twice")
    );
}

#[test]
fn invalid_names_references_saga_mutations_and_join_cycles_fail_locally() {
    assert_eq!(
        Builder::new("Bad Name").build().expect_err("bad name").code,
        "VALIDATION_FAILED"
    );
    let dangling = Builder::new("dangling-flow");
    dangling.condition("choice", "true", "missing", "missing");
    assert!(
        dangling
            .build()
            .expect_err("dangling reference")
            .message
            .contains("not a node")
    );
    let body = Builder::flow();
    body.step("charge", "charge", StepOptions::default());
    let saga = Builder::new("saga-flow");
    saga.saga("transaction", &body, Default::default());
    body.end("bad", None);
    assert!(
        saga.build()
            .expect_err("late end")
            .message
            .contains("end node")
    );
    let recursive = Builder::flow();
    recursive.step("one", "activity", StepOptions::default());
    recursive.saga("recursive", &recursive, Default::default());
    assert!(
        recursive
            .build()
            .expect_err("cycle")
            .message
            .contains("cycle")
    );
}

#[test]
fn canonical_text_uses_utf16_and_ecmascript_numbers_with_safe_integer_policy() {
    let tree = json!({"～":1,"😀":2,"numbers":[-0.0,1.0,600000.0,1e21,-1e-7,1e-6]});
    assert_eq!(
        canonical_json(&tree).expect("canonical"),
        "{\"numbers\":[0,1,600000,1e+21,-1e-7,0.000001],\"😀\":2,\"～\":1}"
    );
    for text in ["9007199254740993", "1e20", "1e400"] {
        let number = serde_json::from_str(text).expect("JSON number");
        assert_eq!(
            canonical_json(&number)
                .expect_err("unsafe canonical number")
                .code,
            "VALIDATION_FAILED"
        );
    }
}

#[test]
fn build_preserves_numbers_and_only_canonical_text_rejects_unsafe_whole_values() {
    let builder = Builder::new("exact-document");
    builder
        .input_schema(json!({"maximum":9007199254740993_u64}))
        .step("one", "one", Default::default());
    assert_eq!(
        builder.build().expect("plain JSON document")["inputSchema"]["maximum"].as_u64(),
        Some(9007199254740993)
    );
    assert_eq!(
        builder.json().expect_err("canonical integer policy").code,
        "VALIDATION_FAILED"
    );
}

#[test]
fn root_only_settings_on_nested_flows_are_refused_instead_of_dropped() {
    let branch = Builder::flow();
    branch
        .input_schema(json!({"type":"object"}))
        .step("one", "one", Default::default());
    let other = Builder::flow();
    other.step("two", "two", Default::default());
    let builder = Builder::new("root-options");
    builder.parallel("fork", Join::All, &[branch, other]);
    assert!(
        builder
            .build()
            .expect_err("root-only settings")
            .message
            .contains("top-level")
    );
}

#[test]
fn managed_database_empty_error_route_is_rejected_with_an_opaque_error() {
    use sagat_octopus_orchestrator_sdk::workflow::{
        ConnectionRef, DatabaseOperation, DatabaseOptions,
    };
    let builder = Builder::new("database-route");
    builder.database(
        "query",
        DatabaseOptions {
            connection_ref: ConnectionRef {
                name: "orders-db".into(),
                revision: None,
            },
            operation: DatabaseOperation::Query,
            statement: "SELECT 'private'".into(),
            parameters: None,
            limits: None,
            timeout_ms: None,
            on_error: Some(String::new()),
        },
    );
    assert_eq!(
        builder.build().expect_err("empty route").message,
        "invalid managed database operation"
    );
}

#[test]
fn state_schema_byte_budget_measures_canonical_text() {
    use sagat_octopus_orchestrator_sdk::workflow::StateDefinition;
    let properties = (0..1470)
        .map(|i| {
            (
                format!("field-{i:04}"),
                json!({"type":"number","minimum":1.0}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let schema = json!({"type":"object","properties":properties,"additionalProperties":false});
    assert!(canonical_json(&schema).expect("canonical schema").len() <= 65536);
    assert!(serde_json::to_vec(&schema).expect("ordinary schema").len() > 65536);
    let builder = Builder::new("schema-budget");
    builder
        .state(StateDefinition {
            schema,
            initial: "input".into(),
            ..Default::default()
        })
        .end("done", None);
    builder.build().expect("canonical schema fits the budget");
}

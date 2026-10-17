//! Public identity and evaluator ceilings shared by additive v1 primitives.
//!
//! These checks prove the current contract/model boundary. They do not prove runtime admission,
//! tenant quota accounting or persistence of a dynamic collection.

use sagat_application::definitions::IrCodec;
use sagat_application::execution::{ExpressionEvaluator, RunData, instance_step_id};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ExecutionInstanceId, InstanceIdentityError, InstancePath, MAX_ITERATION_INDEX, MAX_SCOPE_DEPTH,
    RunId,
};
use sagat_domain::workflow::{Expression, NodeId, WorkflowLimits};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use serde_json::{Value, json};

fn condition(source: &str, input: &Value) -> Result<bool, String> {
    let expression = Expression::parse(source).map_err(|error| error.to_string())?;
    let data = RunData {
        input: serde_json::to_vec(input).expect("fixture input is JSON"),
        ..RunData::default()
    };
    JsonEvaluator
        .condition(&expression, &data)
        .map_err(|error| error.to_string())
}

#[test]
fn old_root_identity_is_the_empty_path_and_scoped_members_are_distinct() {
    let node = NodeId::parse("process").expect("fixture node");
    let old_root = ExecutionInstanceId::root(node.clone());
    let explicit_root = ExecutionInstanceId::in_path(InstancePath::root(), node.clone());
    assert_eq!(old_root, explicit_root);
    assert!(old_root.path().is_root());

    let scope = NodeId::parse("items").expect("fixture scope");
    let first_path = InstancePath::root()
        .child(scope.clone(), 0)
        .expect("zero is the first stable index");
    let last_path = InstancePath::root()
        .child(scope.clone(), MAX_ITERATION_INDEX)
        .expect("9,999 is the last stable index");
    assert_ne!(first_path, last_path);
    assert_eq!(first_path.segments()[0].scope_node(), &scope);
    assert_eq!(first_path.segments()[0].iteration_index(), 0);
    assert_eq!(last_path.segments()[0].iteration_index(), 9_999);
    assert_eq!(
        InstancePath::root().child(scope.clone(), 10_000),
        Err(InstanceIdentityError::IterationIndexOutOfRange)
    );

    let mut deepest = InstancePath::root();
    for index in 0..MAX_SCOPE_DEPTH {
        deepest = deepest
            .child(
                NodeId::parse(&format!("scope{index}")).expect("fixture scope"),
                u32::try_from(index).expect("bounded fixture index"),
            )
            .expect("eight nested scopes are accepted");
    }
    assert_eq!(deepest.depth(), 8);
    assert_eq!(
        deepest.child(NodeId::parse("tooDeep").expect("fixture scope"), 0),
        Err(InstanceIdentityError::ScopeDepthExceeded)
    );

    let scoped = ExecutionInstanceId::in_path(first_path, node);
    assert_ne!(scoped, old_root);
    assert_eq!(scoped.compensation().path(), scoped.path());
    assert_eq!(scoped.reconciliation().path(), scoped.path());

    let version = WorkflowVersionId::generate();
    let run = RunId::generate();
    assert_eq!(
        instance_step_id(version, run, &old_root),
        instance_step_id(version, run, &explicit_root),
        "an upgraded reader maps a legacy root to the same durable step identity"
    );
    assert_ne!(
        instance_step_id(version, run, &old_root),
        instance_step_id(version, run, &scoped),
        "a runtime iteration cannot alias the legacy root"
    );
}

#[test]
fn domain_and_additive_v1_schema_publish_the_same_identity_limits() {
    let defaults = WorkflowLimits::default();
    assert_eq!(defaults.max_instances(), 100_000);
    assert_eq!(defaults.max_depth(), 8);
    assert!(WorkflowLimits::new(1, 1).is_ok());
    assert!(WorkflowLimits::new(100_000, 8).is_ok());
    for (instances, depth) in [(0, 1), (100_001, 1), (1, 0), (1, 9)] {
        assert!(
            WorkflowLimits::new(instances, depth).is_err(),
            "invalid limits {instances}/{depth}"
        );
    }
    assert!(NodeId::parse(&"n".repeat(64)).is_ok());
    assert!(NodeId::parse(&"n".repeat(65)).is_err());

    let path = super::repository_root()
        .join("specs/001-platform-foundation/contracts/workflow.schema.json");
    let source = std::fs::read_to_string(path).expect("canonical workflow schema exists");
    let schema: Value = serde_json::from_str(&source).expect("workflow schema is JSON");
    assert_eq!(schema["properties"]["nodes"]["maxItems"], 10_000);
    assert_eq!(
        schema["$defs"]["body"]["properties"]["nodes"]["maxItems"],
        10_000
    );
    assert_eq!(schema["$defs"]["nodeId"]["minLength"], 1);
    assert_eq!(schema["$defs"]["nodeId"]["maxLength"], 64);
    assert_eq!(schema["$defs"]["expression"]["x-maxUtf8Bytes"], 1_024);
    assert_eq!(
        schema["$defs"]["limits"]["properties"]["maxInstances"]["minimum"],
        1
    );
    assert_eq!(
        schema["$defs"]["limits"]["properties"]["maxInstances"]["maximum"],
        100_000
    );
    assert_eq!(
        schema["$defs"]["limits"]["properties"]["maxDepth"]["minimum"],
        1
    );
    assert_eq!(
        schema["$defs"]["limits"]["properties"]["maxDepth"]["maximum"],
        8
    );

    let legacy = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "legacy-root"},
        "entrypoint": "done",
        "nodes": [{"id": "done", "type": "end"}]
    });
    let parsed = JsonIrCodec
        .parse(&serde_json::to_vec(&legacy).expect("legacy fixture JSON"))
        .expect("an old root-only v1 definition remains accepted");
    assert_eq!(parsed.definition.limits(), WorkflowLimits::default());
}

#[test]
fn publication_lets_an_author_lower_the_limits_and_never_raise_them() {
    let document = |limits: Option<Value>| {
        let mut document = json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": "lowered-limits"},
            "entrypoint": "work",
            "nodes": [
                {"id": "work", "type": "activity", "activity": "orders.work", "next": "done"},
                {"id": "done", "type": "end"}
            ]
        });
        if let Some(limits) = limits {
            document["limits"] = limits;
        }
        serde_json::to_vec(&document).expect("fixture JSON")
    };
    let old = JsonIrCodec.parse(&document(None)).expect("old document");
    assert_eq!(old.definition.limits(), WorkflowLimits::default());
    assert_eq!(
        old.digest,
        JsonIrCodec.parse(&document(None)).expect("again").digest,
        "a document without the block keeps its digest"
    );

    let lowered = JsonIrCodec
        .parse(&document(Some(json!({"maxInstances": 3, "maxDepth": 1}))))
        .expect("an author may lower both ceilings");
    assert_eq!(lowered.definition.limits().max_instances(), 3);
    assert_eq!(lowered.definition.limits().max_depth(), 1);
    assert_ne!(lowered.digest, old.digest);

    for raised in [
        json!({"maxInstances": 100_001}),
        json!({"maxDepth": 9}),
        json!({"maxInstances": 0}),
        json!({"maxDepth": 0}),
    ] {
        assert!(
            JsonIrCodec.parse(&document(Some(raised.clone()))).is_err(),
            "{raised} must not publish"
        );
    }
}

#[test]
fn effective_expression_limits_remain_bounded_for_additive_v1() {
    let at_source_limit = format!("true{}", " ".repeat(1_020));
    let over_source_limit = format!("true{}", " ".repeat(1_021));
    assert_eq!(at_source_limit.len(), 1_024);
    assert_eq!(condition(&at_source_limit, &json!({})), Ok(true));
    assert!(condition(&over_source_limit, &json!({})).is_err());

    let literal_64 = format!("[{}].size() == 64", vec!["0"; 64].join(","));
    let literal_65 = format!("[{}].size() == 65", vec!["0"; 65].join(","));
    assert_eq!(condition(&literal_64, &json!({})), Ok(true));
    assert!(condition(&literal_65, &json!({})).is_err());
    assert!(condition("{'value': 1} == {'value': 1}", &json!({})).is_err());

    let within = json!({"left": vec![0; 2_048], "right": vec![0; 2_048]});
    assert_eq!(
        condition("(input.left + input.right).size() == 4096", &within),
        Ok(true)
    );
    let over = json!({"left": vec![0; 2_048], "right": vec![0; 2_049]});
    assert!(condition("(input.left + input.right).size() == 4097", &over).is_err());
}

#[test]
fn ten_thousand_items_are_read_by_reference_without_expanding_expression_limits() {
    let input = json!({"items": vec![0; 10_000]});
    assert_eq!(condition("input.items.size() == 10000", &input), Ok(true));
}

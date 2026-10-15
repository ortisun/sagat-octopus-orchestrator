//! Internal additive-v1 parsing for continuation chains.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use sagat_application::definitions::{IrCodec, IrError};
use sagat_domain::workflow::{NodeId, NodeKind};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

const CONTINUATION_GOLDEN: &str = include_str!(
    "../../../tests/contract/sdk-conformance/expected/continuation-builders.workflows.json"
);

fn stateful_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "continuation-stateful"},
        "entrypoint": "prepare",
        "nodes": [
            {
                "id": "prepare",
                "type": "activity",
                "activity": "segment.prepare",
                "next": "rollover"
            },
            {
                "id": "rollover",
                "type": "continueAsNew",
                "input": "result.nextInput",
                "state": "state"
            }
        ],
        "state": {
            "schema": {"type": "integer"},
            "initial": "input.initial"
        },
        "chainPolicy": {"maxSegments": 10000, "timeoutMs": 31622400000_u64}
    })
}

fn published_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "continuation-published"},
        "entrypoint": "work",
        "nodes": [
            {
                "id": "work",
                "type": "activity",
                "activity": "segment.work",
                "next": "rollover"
            },
            {
                "id": "rollover",
                "type": "continueAsNew",
                "input": "input.next"
            }
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 86_400_000}
    })
}

fn conditional_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "conditional-continuation-published"},
        "entrypoint": "work",
        "nodes": [
            {
                "id": "work",
                "type": "activity",
                "activity": "segment.work",
                "next": "again"
            },
            {
                "id": "again",
                "type": "condition",
                "expression": "input.continue",
                "ifTrue": "rollover",
                "ifFalse": "done"
            },
            {
                "id": "rollover",
                "type": "continueAsNew",
                "input": "input.next"
            },
            {"id": "done", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 86_400_000}
    })
}

#[test]
fn publishes_only_the_proven_activity_to_continuation_shape() {
    let document = published_document();
    let parsed = JsonIrCodec
        .parse(document.to_string().as_bytes())
        .expect("bounded continuation is publishable");
    let policy = parsed
        .definition
        .continuation_policy()
        .expect("continuation policy");
    assert_eq!(policy.max_segments(), 3);
    assert_eq!(policy.timeout(), Duration::from_secs(86_400));
    assert!(matches!(
        parsed
            .definition
            .node(&NodeId::parse("rollover").expect("node"))
            .expect("continuation")
            .kind(),
        NodeKind::ContinueAsNew { input, state }
            if input.as_str() == "input.next" && state.is_none()
    ));
}

#[test]
fn publishes_a_single_condition_between_continuation_and_successful_end() {
    let document = conditional_document();
    let parsed = JsonIrCodec
        .parse(document.to_string().as_bytes())
        .expect("conditional continuation is publishable");
    assert_eq!(
        parsed
            .definition
            .continuation_policy()
            .expect("continuation policy")
            .max_segments(),
        3
    );
    assert!(matches!(
        parsed
            .definition
            .node(&NodeId::parse("again").expect("node"))
            .expect("condition")
            .kind(),
        NodeKind::Condition { if_true, if_false, .. }
            if if_true.as_str() == "rollover" && if_false.as_str() == "done"
    ));
}

#[test]
fn publication_rejects_unproven_continuation_combinations() {
    let mut entrypoint = published_document();
    entrypoint["entrypoint"] = json!("rollover");
    assert!(matches!(
        JsonIrCodec.parse(entrypoint.to_string().as_bytes()),
        Err(IrError::Unsupported { kind, .. }) if kind == "continueAsNew"
    ));

    let mut state = published_document();
    state["state"] = json!({"schema": {"type": "integer"}, "initial": "0"});
    state["nodes"][1]["state"] = json!("state");
    assert!(matches!(
        JsonIrCodec.parse(state.to_string().as_bytes()),
        Err(IrError::UnsupportedDeclaration {
            field: "chainPolicy",
            ..
        })
    ));

    let mut extra_node = published_document();
    extra_node["nodes"]
        .as_array_mut()
        .expect("nodes")
        .insert(1, json!({"id": "middle", "type": "end"}));
    assert!(matches!(
        JsonIrCodec.parse(extra_node.to_string().as_bytes()),
        Err(IrError::Unsupported { .. })
    ));

    let mut trigger = published_document();
    trigger["triggers"] = json!([{"kind": "manual"}]);
    assert!(matches!(
        JsonIrCodec.parse(trigger.to_string().as_bytes()),
        Err(IrError::UnsupportedDeclaration {
            field: "triggers",
            ..
        })
    ));
}

#[test]
fn validates_and_preserves_policy_transfer_and_graph_without_enabling_publication() {
    let document = stateful_document();
    let bytes = serde_json::to_vec(&document).expect("document");
    let parsed = JsonIrCodec
        .parse_continuation(&bytes)
        .expect("continuation validates");

    assert_eq!(parsed.workflow_name().as_str(), "continuation-stateful");
    assert_eq!(parsed.entrypoint().as_str(), "prepare");
    assert_eq!(parsed.policy().max_segments(), 10_000);
    assert_eq!(
        parsed.policy().timeout(),
        Duration::from_millis(31_622_400_000)
    );
    let domain_policy = parsed
        .definition()
        .continuation_policy()
        .expect("domain continuation policy");
    assert_eq!(domain_policy.max_segments(), 10_000);
    assert_eq!(domain_policy.timeout(), parsed.policy().timeout());
    assert!(matches!(
        parsed
            .definition()
            .node(&NodeId::parse("rollover").expect("node"))
            .expect("domain continuation")
            .kind(),
        NodeKind::ContinueAsNew { input, state }
            if input.as_str() == "result.nextInput"
                && state.as_ref().is_some_and(|state| state.as_str() == "state")
    ));
    let rollover = parsed
        .continuation(&NodeId::parse("rollover").expect("node"))
        .expect("continuation");
    assert_eq!(rollover.input().as_str(), "result.nextInput");
    assert_eq!(rollover.state().expect("state transfer").as_str(), "state");
    assert_eq!(parsed.continuations().count(), 1);
    assert_eq!(
        parsed.state().expect("declared state").initial().as_str(),
        "input.initial"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(parsed.canonical()).expect("canonical"),
        document
    );

    assert!(matches!(
        JsonIrCodec.parse(&bytes),
        Err(IrError::UnsupportedDeclaration {
            field: "chainPolicy",
            ..
        })
    ));
    assert!(matches!(
        JsonIrCodec.parse_structured(&bytes),
        Err(IrError::UnsupportedDeclaration { field: "state", .. })
    ));
}

#[test]
fn input_only_sdk_golden_reparses_to_the_reviewed_canonical_v1_bytes() {
    let golden: Value = serde_json::from_str(CONTINUATION_GOLDEN).expect("golden JSON");
    let document = &golden["workflows"]["input-only"];
    let expected = golden["canonical"]["input-only"]
        .as_str()
        .expect("canonical workflow");

    let parsed = JsonIrCodec
        .parse_continuation(document.to_string().as_bytes())
        .expect("input-only continuation");
    assert_eq!(parsed.canonical(), expected.as_bytes());
    assert_eq!(parsed.policy().max_segments(), 3);
    assert_eq!(parsed.policy().timeout(), Duration::from_secs(86_400));
    assert_eq!(
        parsed
            .continuation(&NodeId::parse("rollover").expect("node"))
            .expect("continuation")
            .input()
            .as_str(),
        "input.next"
    );
    assert!(parsed.state().is_none());

    assert!(matches!(
        JsonIrCodec.parse(document.to_string().as_bytes()),
        Err(IrError::Unsupported { kind, .. }) if kind == "continueAsNew"
    ));
}

#[test]
fn stateful_sdk_golden_reparses_to_the_reviewed_canonical_v1_bytes() {
    let golden: Value = serde_json::from_str(CONTINUATION_GOLDEN).expect("golden JSON");
    let document = &golden["workflows"]["input-and-state-maximum"];
    let expected = golden["canonical"]["input-and-state-maximum"]
        .as_str()
        .expect("canonical workflow");

    let parsed = JsonIrCodec
        .parse_continuation(document.to_string().as_bytes())
        .expect("stateful continuation");
    assert_eq!(parsed.canonical(), expected.as_bytes());
    assert_eq!(parsed.policy().max_segments(), 10_000);
    assert_eq!(
        parsed.policy().timeout(),
        Duration::from_millis(31_622_400_000)
    );
    let state = parsed.state().expect("declared state");
    assert_eq!(state.initial().as_str(), "input.state");
    assert_eq!(
        serde_json::from_slice::<Value>(state.schema().canonical()).expect("state schema"),
        document["state"]["schema"]
    );
    let rollover = parsed
        .continuation(&NodeId::parse("rollover").expect("node"))
        .expect("continuation");
    assert_eq!(rollover.input().as_str(), "result.nextInput");
    assert_eq!(rollover.state().expect("state transfer").as_str(), "state");

    assert!(matches!(
        JsonIrCodec.parse(document.to_string().as_bytes()),
        Err(IrError::UnsupportedDeclaration {
            field: "chainPolicy",
            ..
        })
    ));
}

#[test]
fn rejects_unavailable_transfer_variables_and_invalid_graphs() {
    let mut unavailable = stateful_document();
    unavailable["nodes"][1]["input"] = json!("secret.token");
    assert!(matches!(
        JsonIrCodec.parse_continuation(unavailable.to_string().as_bytes()),
        Err(IrError::Expression { field: "input", .. })
    ));

    let mut broken_graph = stateful_document();
    broken_graph["nodes"][0]["next"] = json!("missing");
    assert!(matches!(
        JsonIrCodec.parse_continuation(broken_graph.to_string().as_bytes()),
        Err(IrError::Invalid(_))
    ));

    let mut missing_state_transfer = stateful_document();
    missing_state_transfer["nodes"][1]
        .as_object_mut()
        .expect("continuation")
        .remove("state");
    assert!(matches!(
        JsonIrCodec.parse_continuation(missing_state_transfer.to_string().as_bytes()),
        Err(IrError::Schema(_))
    ));
}

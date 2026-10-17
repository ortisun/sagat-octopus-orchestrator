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

fn waiting_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "waiting-continuation-published"},
        "entrypoint": "prepare",
        "nodes": [
            {"id": "prepare", "type": "activity", "activity": "chain.prepare", "next": "release"},
            {
                "id": "release", "type": "waitEvent", "eventType": "chain.release",
                "correlationExpr": "input.key", "timeoutMs": 600000, "next": "rollover"
            },
            {"id": "rollover", "type": "continueAsNew", "input": "input"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 86_400_000}
    })
}

#[test]
fn publishes_an_event_wait_after_an_activity_followed_by_continuation() {
    let parsed = JsonIrCodec
        .parse(waiting_document().to_string().as_bytes())
        .expect("waiting continuation is publishable");
    assert!(matches!(
        parsed
            .definition
            .node(&NodeId::parse("release").expect("node"))
            .expect("wait")
            .kind(),
        NodeKind::WaitEvent { .. }
    ));
    assert!(matches!(
        parsed
            .definition
            .node(&NodeId::parse("rollover").expect("node"))
            .expect("continuation")
            .kind(),
        NodeKind::ContinueAsNew { state: None, .. }
    ));
}

#[test]
fn publication_keeps_other_waiting_continuation_shapes_closed() {
    let mut timeout_route = waiting_document();
    timeout_route["nodes"][1]["onTimeout"] = json!("rollover");
    assert!(
        JsonIrCodec
            .parse(timeout_route.to_string().as_bytes())
            .is_err()
    );

    let mut lookback = waiting_document();
    lookback["nodes"][1]["lookbackMs"] = json!(1);
    assert!(JsonIrCodec.parse(lookback.to_string().as_bytes()).is_err());

    let mut stateful = waiting_document();
    stateful["state"] = json!({"schema": {"type": "integer"}, "initial": "input.initial"});
    stateful["nodes"][2]["state"] = json!("state");
    assert!(JsonIrCodec.parse(stateful.to_string().as_bytes()).is_err());

    let mut extra_node = waiting_document();
    extra_node["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(json!({"id": "done", "type": "end"}));
    assert!(
        JsonIrCodec
            .parse(extra_node.to_string().as_bytes())
            .is_err()
    );
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
fn publishes_direct_continuation_with_exact_declared_state_transfer() {
    let bytes = stateful_document().to_string();
    let parsed = JsonIrCodec
        .parse(bytes.as_bytes())
        .expect("stateful continuation is publishable");
    let operation_view = JsonIrCodec
        .parse_declared_state(bytes.as_bytes())
        .expect("operations read the published continuation declaration");
    assert_eq!(operation_view.parsed().digest, parsed.digest);
    assert!(matches!(
        parsed
            .definition
            .node(&NodeId::parse("rollover").expect("node"))
            .expect("continuation")
            .kind(),
        NodeKind::ContinueAsNew { state: Some(state), .. } if state.as_str() == "state"
    ));
}

#[test]
fn publishes_conditional_continuation_with_exact_declared_state_transfer() {
    let mut document = conditional_document();
    document["state"] = json!({"schema": {"type": "integer"}, "initial": "input.initial"});
    document["nodes"][2]["state"] = json!("state");
    let bytes = document.to_string();
    let parsed = JsonIrCodec
        .parse(bytes.as_bytes())
        .expect("conditional stateful continuation");
    let operation_view = JsonIrCodec
        .parse_declared_state(bytes.as_bytes())
        .expect("operations read conditional continuation state");
    assert_eq!(operation_view.parsed().digest, parsed.digest);
    assert!(matches!(
        parsed.definition.node(&NodeId::parse("rollover").expect("node")).expect("continuation").kind(),
        NodeKind::ContinueAsNew { state: Some(state), .. } if state.as_str() == "state"
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

    let mut state = stateful_document();
    state["nodes"][1]["state"] = json!("input.initial");
    assert!(matches!(
        JsonIrCodec.parse(state.to_string().as_bytes()),
        Err(IrError::Unsupported { .. })
    ));

    let mut missing_transfer = stateful_document();
    missing_transfer["nodes"][1]
        .as_object_mut()
        .expect("continuation")
        .remove("state");
    assert!(
        JsonIrCodec
            .parse(missing_transfer.to_string().as_bytes())
            .is_err()
    );

    let mut conditional_state = conditional_document();
    conditional_state["state"] = json!({"schema": {"type": "integer"}, "initial": "0"});
    conditional_state["nodes"][2]["state"] = json!("input.initial");
    assert!(
        JsonIrCodec
            .parse(conditional_state.to_string().as_bytes())
            .is_err()
    );

    conditional_state["nodes"][2]
        .as_object_mut()
        .expect("conditional continuation")
        .remove("state");
    assert!(
        JsonIrCodec
            .parse(conditional_state.to_string().as_bytes())
            .is_err()
    );

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
fn validates_and_preserves_policy_transfer_and_graph() {
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

    JsonIrCodec.parse(&bytes).expect("publishable continuation");
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
        Err(IrError::Unsupported { .. })
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

fn approval_gated_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "continuation-approval"},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "review"},
            {
                "id": "review",
                "type": "approval",
                "policy": {"roles": ["operator"]},
                "timeoutMs": 60_000,
                "onApprove": "rollover",
                "onReject": "rejected",
                "onExpire": "expired"
            },
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"},
            {"id": "rejected", "type": "end"},
            {"id": "expired", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    })
}

#[test]
fn publishes_an_approval_that_rolls_the_chain_over_and_ends_it_on_rejection_or_expiry() {
    let parsed = JsonIrCodec
        .parse(approval_gated_document().to_string().as_bytes())
        .expect("an approval and a continuation publish together (D295)");
    assert!(
        parsed
            .definition
            .node(&NodeId::parse("review").expect("approval"))
            .is_some_and(|node| matches!(node.kind(), NodeKind::Approval { .. }))
    );
    assert!(parsed.definition.continuation_policy().is_some());

    // One end node may serve both the rejection and the expiry.
    let mut shared = approval_gated_document();
    shared["nodes"][1]["onExpire"] = json!("rejected");
    shared["nodes"].as_array_mut().expect("nodes").pop();
    JsonIrCodec
        .parse(shared.to_string().as_bytes())
        .expect("a shared end node");

    // The exact declared state travels with the chain, as for the other published shapes.
    let mut stateful = approval_gated_document();
    stateful["nodes"][2]["state"] = json!("state");
    stateful["state"] = json!({"schema": {"type": "integer"}, "initial": "0"});
    JsonIrCodec
        .parse(stateful.to_string().as_bytes())
        .expect("approval, state and continuation together");
}

#[test]
fn publication_keeps_other_approval_continuation_shapes_closed() {
    let closed = |edit: &dyn Fn(&mut Value)| {
        let mut document = approval_gated_document();
        edit(&mut document);
        assert!(
            matches!(
                JsonIrCodec.parse(document.to_string().as_bytes()),
                Err(IrError::Unsupported { .. } | IrError::Invalid(_))
            ),
            "{document}"
        );
    };
    // The approval is the entrypoint: successor creation assumes an activity entrypoint.
    closed(&|document| document["entrypoint"] = json!("review"));
    // Rejection continues instead of ending, or the approval routes elsewhere than the boundary.
    closed(&|document| document["nodes"][1]["onReject"] = json!("rollover"));
    closed(&|document| document["nodes"][1]["onApprove"] = json!("rejected"));
    // An approval error edge, and an end that carries an output.
    closed(&|document| document["nodes"][1]["onError"] = json!("rejected"));
    closed(&|document| document["nodes"][3]["outputExpr"] = json!("'rejected'"));
    // A second approval in the segment.
    closed(&|document| {
        let second = json!({
            "id": "second", "type": "approval", "policy": {"roles": ["operator"]},
            "timeoutMs": 60_000, "onApprove": "rejected", "onReject": "rejected",
            "onExpire": "rejected"
        });
        document["nodes"]
            .as_array_mut()
            .expect("nodes")
            .push(second);
    });
}

fn root_wait_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "root-wait-continuation-published"},
        "entrypoint": "release",
        "nodes": [
            {
                "id": "release", "type": "waitEvent", "eventType": "chain.release",
                "correlationExpr": "input.key", "timeoutMs": 600000, "next": "rollover"
            },
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 86_400_000}
    })
}

fn root_wait_conditional_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "root-wait-conditional-published"},
        "entrypoint": "release",
        "nodes": [
            {
                "id": "release", "type": "waitEvent", "eventType": "chain.release",
                "correlationExpr": "input.key", "timeoutMs": 600000, "next": "again"
            },
            {
                "id": "again", "type": "condition", "expression": "input.continue",
                "ifTrue": "rollover", "ifFalse": "done"
            },
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"},
            {"id": "done", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 86_400_000}
    })
}

#[test]
fn publishes_an_event_wait_entrypoint_followed_by_a_continuation_or_one_condition() {
    for document in [root_wait_document(), root_wait_conditional_document()] {
        let parsed = JsonIrCodec
            .parse(document.to_string().as_bytes())
            .expect("a wait entrypoint is publishable");
        assert!(matches!(
            parsed
                .definition
                .node(&NodeId::parse("release").expect("node"))
                .expect("entry")
                .kind(),
            NodeKind::WaitEvent { .. }
        ));
        assert_eq!(parsed.definition.entrypoint().as_str(), "release");
    }
}

#[test]
fn publication_keeps_every_other_wait_entrypoint_shape_closed_with_a_typed_refusal() {
    let closed = |edit: &dyn Fn(&mut Value)| {
        let mut document = root_wait_document();
        edit(&mut document);
        assert!(
            matches!(
                JsonIrCodec.parse(document.to_string().as_bytes()),
                Err(IrError::Unsupported { .. } | IrError::Invalid(_))
            ),
            "{document}"
        );
    };
    // The wait routes its own timeout, or looks back at an event that may be the one that ended the
    // predecessor's wait.
    closed(&|document| document["nodes"][0]["onTimeout"] = json!("rollover"));
    closed(&|document| document["nodes"][0]["lookbackMs"] = json!(1));
    // Declared state does not travel to a successor that begins by waiting.
    closed(&|document| {
        document["state"] = json!({"schema": {"type": "integer"}, "initial": "0"});
        document["nodes"][1]["state"] = json!("state");
    });
    // A second wait, an activity, an approval or an extra node between the wait and the boundary.
    closed(&|document| {
        document["nodes"][0]["next"] = json!("second");
        document["nodes"].as_array_mut().expect("nodes").insert(
            1,
            json!({
                "id": "second", "type": "waitEvent", "eventType": "chain.again",
                "correlationExpr": "input.key", "timeoutMs": 600000, "next": "rollover"
            }),
        );
    });
    closed(&|document| {
        document["nodes"]
            .as_array_mut()
            .expect("nodes")
            .push(json!({"id": "done", "type": "end"}));
    });
    // Other node kinds as the entrypoint stay closed with the message that names the two allowed.
    let mut timer = root_wait_document();
    timer["nodes"][0] =
        json!({"id": "release", "type": "timer", "durationMs": 1000, "next": "rollover"});
    assert!(matches!(
        JsonIrCodec.parse(timer.to_string().as_bytes()),
        Err(IrError::Unsupported { kind, reason, .. })
            if kind == "timer" && reason.contains("activity or an event wait")
    ));
    let mut stateful_condition = root_wait_conditional_document();
    stateful_condition["state"] = json!({"schema": {"type": "integer"}, "initial": "0"});
    stateful_condition["nodes"][2]["state"] = json!("state");
    assert!(matches!(
        JsonIrCodec.parse(stateful_condition.to_string().as_bytes()),
        Err(IrError::Unsupported { kind, .. }) if kind == "waitEvent"
    ));
}

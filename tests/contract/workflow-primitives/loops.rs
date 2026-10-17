//! The shared SDK loop document must agree with the server's structured v1 model.

use sagat_application::definitions::IrCodec;
use sagat_domain::workflow::{NodeId, NodeKind};
use sagat_infra_ir::JsonIrCodec;
use serde_json::Value;

fn loop_golden() -> (Value, Vec<u8>) {
    let path = super::repository_root()
        .join("tests/contract/sdk-conformance/expected/loop-builders.workflows.json");
    let source = std::fs::read(path).expect("reviewed SDK golden");
    let golden: Value = serde_json::from_slice(&source).expect("golden JSON");
    let bytes =
        serde_json::to_vec(&golden["workflows"]["pre-and-post-test"]).expect("workflow JSON");
    (golden, bytes)
}

#[test]
fn five_sdk_loop_golden_is_a_valid_structured_v1_definition() {
    let (golden, bytes) = loop_golden();

    let parsed = JsonIrCodec
        .parse_structured(&bytes)
        .expect("SDK output matches the structured server model");
    let pre = parsed
        .definition
        .node(&NodeId::parse("pre").expect("node id"))
        .expect("pre-test loop");
    let post = parsed
        .definition
        .node(&NodeId::parse("post").expect("node id"))
        .expect("post-test loop");
    assert!(matches!(pre.kind(), NodeKind::While { .. }));
    assert!(matches!(post.kind(), NodeKind::RepeatUntil { .. }));
    assert_eq!(parsed.definition.bodies().count(), 1);
    assert_eq!(
        String::from_utf8(parsed.canonical.clone()).expect("canonical UTF-8"),
        golden["canonical"]["pre-and-post-test"]
            .as_str()
            .expect("reviewed canonical text")
    );

    let published = JsonIrCodec
        .parse(&bytes)
        .expect("the proven sequential-loop subset is publishable");
    assert_eq!(published.digest, parsed.digest);
    assert_eq!(published.canonical, parsed.canonical);
}

#[test]
fn structured_codec_preserves_the_complete_sequential_loop_contract() {
    let (_, bytes) = loop_golden();
    let parsed = JsonIrCodec
        .parse_structured(&bytes)
        .expect("structured codec accepts the reviewed loop contract");
    let pre = parsed
        .definition
        .node(&NodeId::parse("pre").expect("node id"))
        .expect("pre-test loop");
    let post = parsed
        .definition
        .node(&NodeId::parse("post").expect("node id"))
        .expect("post-test loop");

    match pre.kind() {
        NodeKind::While {
            condition,
            body,
            initial,
            carry,
            max_iterations,
            delay,
        } => {
            assert_eq!(condition.as_str(), "carry < 3");
            assert_eq!(body.as_str(), "iteration");
            assert_eq!(initial.as_str(), "input.start");
            assert_eq!(carry.as_str(), "bodyResult");
            assert_eq!(*max_iterations, 3);
            assert_eq!(*delay, Some(std::time::Duration::from_millis(250)));
        }
        kind => panic!("expected while, got {kind:?}"),
    }

    match post.kind() {
        NodeKind::RepeatUntil {
            condition,
            body,
            initial,
            carry,
            max_iterations,
            delay,
        } => {
            assert_eq!(condition.as_str(), "carry >= 5");
            assert_eq!(body.as_str(), "iteration");
            assert_eq!(initial.as_str(), "steps.pre.output");
            assert_eq!(carry.as_str(), "bodyResult");
            assert_eq!(*max_iterations, 10_000);
            assert_eq!(*delay, None);
        }
        kind => panic!("expected repeatUntil, got {kind:?}"),
    }
}

/// The loop bounds of the contract (T014-018): `timeoutMs` and `delayMs` are 1..31622400000 ms,
/// `maxIterations` is required and 1..10000, and the codec refuses anything outside them before a
/// run can exist.
#[test]
fn loop_bounds_are_refused_at_publication() {
    let (golden, _) = loop_golden();
    let workflow = &golden["workflows"]["pre-and-post-test"];
    let with = |field: &str, value: Option<Value>| {
        let mut document = workflow.clone();
        let node = document["nodes"]
            .as_array_mut()
            .expect("nodes")
            .iter_mut()
            .find(|node| node["id"] == "pre")
            .expect("pre-test loop");
        match value {
            Some(value) => node[field] = value,
            None => {
                node.as_object_mut().expect("node object").remove(field);
            }
        }
        serde_json::to_vec(&document).expect("workflow JSON")
    };
    let accepted = |bytes: Vec<u8>| JsonIrCodec.parse(&bytes).is_ok();

    for (field, value) in [
        ("timeoutMs", 1_u64),
        ("timeoutMs", 31_622_400_000),
        ("delayMs", 1),
        ("delayMs", 31_622_400_000),
        ("maxIterations", 1),
        ("maxIterations", 10_000),
    ] {
        assert!(
            accepted(with(field, Some(value.into()))),
            "{field} = {value}"
        );
    }
    for (field, value) in [
        ("timeoutMs", 0_u64),
        ("timeoutMs", 31_622_400_001),
        ("delayMs", 0),
        ("delayMs", 31_622_400_001),
        ("maxIterations", 0),
        ("maxIterations", 10_001),
    ] {
        assert!(
            !accepted(with(field, Some(value.into()))),
            "{field} = {value}"
        );
    }
    assert!(
        !accepted(with("maxIterations", None)),
        "maxIterations is required"
    );
    assert!(!accepted(with("timeoutMs", None)), "timeoutMs is required");
}

/// A one-loop workflow whose `onError` edge recovers through an activity (G340).
fn recovery_document(edit: impl FnOnce(&mut Vec<Value>)) -> Vec<u8> {
    let mut nodes = vec![
        serde_json::json!({
            "id": "poll", "type": "while", "condition": "carry < 1", "body": "iteration",
            "initial": "0", "carry": "bodyResult", "maxIterations": 3, "timeoutMs": 60000,
            "next": "done", "onError": "recover"
        }),
        serde_json::json!({"id": "done", "type": "end"}),
        serde_json::json!({
            "id": "recover", "type": "activity", "activity": "recover.cleanup",
            "next": "recovered"
        }),
        serde_json::json!({"id": "recovered", "type": "end"}),
    ];
    edit(&mut nodes);
    serde_json::to_vec(&serde_json::json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "loop-recovery"},
        "entrypoint": "poll",
        "bodies": {"iteration": {"entrypoint": "increment", "nodes": [
            {"id": "increment", "type": "activity", "activity": "counter.increment",
             "next": "return-value"},
            {"id": "return-value", "type": "return", "result": "steps.increment.output"}
        ]}},
        "nodes": nodes
    }))
    .expect("workflow JSON")
}

fn node_mut<'a>(nodes: &'a mut [Value], id: &str) -> &'a mut Value {
    nodes
        .iter_mut()
        .find(|node| node["id"] == id)
        .expect("declared node")
}

#[test]
fn a_loop_error_edge_may_recover_through_a_chain_of_activities_ending_at_end() {
    let single = JsonIrCodec
        .parse(&recovery_document(|_| {}))
        .expect("an onError edge to an activity is publishable");
    let chained = JsonIrCodec
        .parse(&recovery_document(|nodes| {
            node_mut(nodes, "recover")["next"] = "notify".into();
            nodes.push(serde_json::json!({
                "id": "notify", "type": "activity", "activity": "recover.notify",
                "next": "recovered"
            }));
        }))
        .expect("a chain of recovery activities is publishable");
    assert_ne!(single.digest, chained.digest);
    // The same document parsed twice is the same canonical definition.
    let again = JsonIrCodec
        .parse(&recovery_document(|_| {}))
        .expect("publishable again");
    assert_eq!(
        (single.digest, &single.canonical),
        (again.digest, &again.canonical)
    );
}

#[test]
fn a_loop_recovery_activity_stays_as_plain_as_the_runtime_proves() {
    let refused = |edit: &dyn Fn(&mut Vec<Value>), reason: &str| {
        let bytes = recovery_document(|nodes| edit(nodes));
        let error = JsonIrCodec
            .parse(&bytes)
            .expect_err("the shape is not enabled by the runtime");
        assert!(format!("{error:?}").contains(reason), "{reason}: {error:?}");
    };
    let plain = "no onError edge, compensation, reconciliation or effect policy";
    refused(
        &|nodes| node_mut(nodes, "recover")["onError"] = "recovered".into(),
        plain,
    );
    refused(
        &|nodes| node_mut(nodes, "recover")["compensateWith"] = "recover.undo".into(),
        plain,
    );
    refused(
        &|nodes| node_mut(nodes, "recover")["irreversible"] = true.into(),
        plain,
    );
    refused(
        &|nodes| node_mut(nodes, "recover")["highRisk"] = true.into(),
        plain,
    );
    refused(
        &|nodes| {
            node_mut(nodes, "recover")["next"] = "again".into();
            nodes.push(serde_json::json!({
                "id": "again", "type": "while", "condition": "carry < 1", "body": "iteration",
                "initial": "0", "carry": "bodyResult", "maxIterations": 3, "timeoutMs": 60000,
                "next": "recovered"
            }));
        },
        "chain ends at end",
    );
}

#[test]
fn an_activity_is_enabled_only_as_a_loop_recovery() {
    let refused = |edit: &dyn Fn(&mut Vec<Value>), reason: &str| {
        let bytes = recovery_document(|nodes| edit(nodes));
        let error = JsonIrCodec
            .parse(&bytes)
            .expect_err("the shape is not enabled by the runtime");
        assert!(format!("{error:?}").contains(reason), "{reason}: {error:?}");
    };
    // After the success edge of a loop.
    refused(
        &|nodes| {
            node_mut(nodes, "poll")["next"] = "recover".into();
            nodes.retain(|node| node["id"] != "done");
        },
        "after a loop's success edge",
    );
    // Declared but reached by nothing: the structure check refuses it before the subset check.
    refused(
        &|nodes| {
            node_mut(nodes, "poll")
                .as_object_mut()
                .expect("loop object")
                .remove("onError");
        },
        "Unreachable",
    );
}

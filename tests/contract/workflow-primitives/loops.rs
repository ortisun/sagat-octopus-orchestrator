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

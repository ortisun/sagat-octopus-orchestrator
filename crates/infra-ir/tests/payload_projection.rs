//! ADR-0028's projector contract: useful shape without retaining any original key or scalar.
#![allow(clippy::expect_used)]

use sagat_application::payloads::{PayloadProjector, PayloadStructure, StructureType};
use sagat_infra_ir::JsonPayloadProjector;

fn count(node: &PayloadStructure) -> usize {
    1 + node.children().iter().map(count).sum::<usize>()
}

#[test]
fn every_key_and_scalar_is_absent_while_shape_and_counts_remain() {
    let canaries = r#"{
        "password":"swordfish",
        "secret-key-without-a-credential-shape": 79228162514264337593543950335,
        "truth":true,
        "nothing":null,
        "nested":[{"schema-public-name":"unicode-value-☃"},false]
    }"#
    .as_bytes();

    let projected = JsonPayloadProjector
        .project(canaries)
        .expect("valid JSON is projected");

    assert!(!projected.truncated);
    assert_eq!(projected.tree.kind(), StructureType::Object);
    assert_eq!(projected.tree.child_count(), Some(5));
    assert_eq!(projected.tree.children().len(), 5);
    assert_eq!(count(&projected.tree), 9);
    let debug = format!("{projected:?}");
    for canary in [
        "password",
        "swordfish",
        "secret-key-without-a-credential-shape",
        "79228162514264337593543950335",
        "truth",
        "schema-public-name",
        "unicode-value",
        "☃",
    ] {
        assert!(!debug.contains(canary), "projection retained {canary:?}");
    }
}

#[test]
fn wide_and_deep_documents_are_truncated_during_traversal() {
    let wide = format!(
        "[{}]",
        std::iter::repeat_n("null", 100)
            .collect::<Vec<_>>()
            .join(",")
    );
    let projected = JsonPayloadProjector
        .project(wide.as_bytes())
        .expect("wide JSON");
    assert!(projected.truncated);
    assert_eq!(projected.tree.child_count(), Some(100));
    assert_eq!(projected.tree.children().len(), 32);
    assert_eq!(projected.tree.omitted_children(), Some(68));

    let mut deep = "null".to_owned();
    for _ in 0..40 {
        deep = format!("[{deep}]");
    }
    let projected = JsonPayloadProjector
        .project(deep.as_bytes())
        .expect("JSON below serde's parser depth");
    assert!(projected.truncated);
    assert_eq!(
        count(&projected.tree),
        17,
        "root depth is zero and depth 16 is retained"
    );
    let leaf = (0..16).fold(&projected.tree, |node, _| &node.children()[0]);
    assert_eq!(leaf.child_count(), Some(1));
    assert_eq!(leaf.children().len(), 0);
    assert_eq!(leaf.omitted_children(), Some(1));
}

#[test]
fn the_global_node_cap_is_applied_before_a_projection_can_grow() {
    let rows = std::iter::repeat_n("[null,null,null,null,null,null,null,null]", 32)
        .collect::<Vec<_>>()
        .join(",");
    let projected = JsonPayloadProjector
        .project(format!("[{rows}]").as_bytes())
        .expect("bounded JSON");
    assert!(projected.truncated);
    assert_eq!(count(&projected.tree), 256);
}

#[test]
fn malformed_or_overly_nested_json_is_an_opaque_unsupported_format() {
    assert!(JsonPayloadProjector.project(br#"{"key":"CANARY""#).is_err());

    let mut too_deep = "null".to_owned();
    for _ in 0..140 {
        too_deep = format!("[{too_deep}]");
    }
    assert!(JsonPayloadProjector.project(too_deep.as_bytes()).is_err());
}

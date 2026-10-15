//! The shared SDK loop document must agree with the server's structured v1 model.

use sagat_application::definitions::{IrCodec, IrError};
use sagat_domain::workflow::{NodeId, NodeKind};
use sagat_infra_ir::JsonIrCodec;
use serde_json::Value;

#[test]
fn five_sdk_loop_golden_is_a_valid_but_unpublished_structured_v1_definition() {
    let path = super::repository_root()
        .join("tests/contract/sdk-conformance/expected/loop-builders.workflows.json");
    let source = std::fs::read(path).expect("reviewed SDK golden");
    let golden: Value = serde_json::from_slice(&source).expect("golden JSON");
    let document = &golden["workflows"]["pre-and-post-test"];
    let bytes = serde_json::to_vec(document).expect("workflow JSON");

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
        String::from_utf8(parsed.canonical).expect("canonical UTF-8"),
        golden["canonical"]["pre-and-post-test"]
            .as_str()
            .expect("reviewed canonical text")
    );

    assert!(matches!(
        JsonIrCodec.parse(&bytes),
        Err(IrError::UnsupportedDeclaration {
            field: "bodies",
            ..
        })
    ));
}

use sagat_application::definitions::IrCodec;
use sagat_domain::workflow::NodeKind;
use sagat_infra_ir::JsonIrCodec;
use serde_json::Value;

#[test]
fn approval_golden_maps_to_the_public_structured_v1_definition() {
    let path = super::repository_root()
        .join("tests/contract/sdk-conformance/expected/approval-builders.workflows.json");
    let golden: Value = serde_json::from_slice(&std::fs::read(path).expect("reviewed SDK golden"))
        .expect("golden JSON");
    let workflows = golden["workflows"].as_object().expect("workflow documents");
    let canonical = golden["canonical"]
        .as_object()
        .expect("canonical documents");
    assert_eq!(workflows.len(), 2);

    for (name, document) in workflows {
        let bytes = serde_json::to_vec(document).expect("workflow JSON");
        let parsed = JsonIrCodec
            .parse_structured(&bytes)
            .expect("approval maps to structured v1");
        assert!(parsed.definition.is_structured());
        assert_eq!(
            std::str::from_utf8(&parsed.canonical).expect("canonical UTF-8"),
            canonical[name].as_str().expect("reviewed canonical text")
        );
        let review = parsed
            .definition
            .nodes()
            .find(|node| node.id().as_str() == "review")
            .expect("approval node");
        let NodeKind::Approval { policy, .. } = review.kind() else {
            panic!("review must be approval")
        };
        assert_eq!(policy.excludes_initiator(), name == "default-exclusion");
        let published = JsonIrCodec
            .parse(&bytes)
            .expect("native approval is publishable");
        assert!(published.definition.is_structured());
        assert_eq!(published.canonical, parsed.canonical);
        assert_eq!(published.digest, parsed.digest);
    }
}

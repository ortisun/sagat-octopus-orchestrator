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

/// Publication enforces the principal bound on subjects and roles together (T014-022): 32 is
/// accepted, 33 is refused before a version exists.
#[test]
fn publication_counts_subjects_and_roles_together() {
    let path = super::repository_root()
        .join("tests/contract/sdk-conformance/expected/approval-builders.workflows.json");
    let golden: Value = serde_json::from_slice(&std::fs::read(path).expect("reviewed SDK golden"))
        .expect("golden JSON");
    let document = golden["workflows"]
        .as_object()
        .and_then(|workflows| workflows.values().next())
        .expect("an approval workflow")
        .clone();
    let with_subjects = |count: usize| {
        let mut document = document.clone();
        let review = document["nodes"]
            .as_array_mut()
            .expect("nodes")
            .iter_mut()
            .find(|node| node["id"] == "review")
            .expect("approval node");
        review["policy"]["subjects"] = (0..count)
            .map(|index| Value::from(format!("approver-{index}")))
            .collect();
        review["policy"]["roles"] = serde_json::json!(["admin", "developer", "operator"]);
        serde_json::to_vec(&document).expect("workflow JSON")
    };
    assert!(
        JsonIrCodec.parse(&with_subjects(29)).is_ok(),
        "29 subjects + 3 roles"
    );
    assert!(
        JsonIrCodec.parse(&with_subjects(30)).is_err(),
        "30 subjects + 3 roles"
    );
}

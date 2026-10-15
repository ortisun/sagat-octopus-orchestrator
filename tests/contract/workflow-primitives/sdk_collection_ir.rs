//! The shared SDK collection documents must agree with the server's structured v1 model.

use sagat_application::definitions::IrCodec;
use sagat_infra_ir::JsonIrCodec;
use serde_json::Value;

#[test]
fn five_sdk_collection_golden_is_a_valid_structured_v1_definition() {
    let path = super::repository_root()
        .join("tests/contract/sdk-conformance/expected/collection-builders.workflows.json");
    let source = std::fs::read(path).expect("reviewed SDK golden");
    let golden: Value = serde_json::from_slice(&source).expect("golden JSON");
    let workflows = golden["workflows"].as_object().expect("workflow documents");
    let canonical = golden["canonical"].as_object().expect("canonical text");
    assert_eq!(workflows.len(), 2);

    for (name, document) in workflows {
        let bytes = serde_json::to_vec(document).expect("workflow JSON");
        let parsed = JsonIrCodec
            .parse_structured(&bytes)
            .expect("SDK output matches the structured server model");
        assert!(parsed.definition.is_structured());
        assert_eq!(
            parsed.definition.bodies().count(),
            if name == "map" { 1 } else { 2 }
        );
        assert_eq!(
            String::from_utf8(parsed.canonical).expect("canonical UTF-8"),
            canonical[name].as_str().expect("reviewed canonical text"),
            "SDK and server canonical forms differ for {name}"
        );
        assert!(
            JsonIrCodec.parse(&bytes).is_err(),
            "server publication stays gated for {name}"
        );
    }
}

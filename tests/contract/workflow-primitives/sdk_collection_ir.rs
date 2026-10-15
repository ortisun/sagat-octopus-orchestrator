//! The shared SDK collection documents must agree with the public structured v1 model.

use sagat_application::definitions::IrCodec;
use sagat_infra_ir::JsonIrCodec;
use serde_json::Value;

#[test]
fn five_sdk_collection_golden_is_a_public_structured_v1_definition() {
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
            .parse(&bytes)
            .expect("SDK output matches the public structured server model");
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
    }
}

#[test]
fn collection_publication_rejects_an_unproven_body_shape() {
    let path = super::repository_root()
        .join("tests/contract/sdk-conformance/expected/collection-builders.workflows.json");
    let source = std::fs::read(path).expect("reviewed SDK golden");
    let golden: Value = serde_json::from_slice(&source).expect("golden JSON");
    let mut document = golden["workflows"]["map"].clone();
    document["bodies"]["line"]["nodes"] = serde_json::json!([
        {"id": "price", "type": "activity", "activity": "line.price", "next": "discount"},
        {"id": "discount", "type": "activity", "activity": "line.discount", "next": "return-price"},
        {"id": "return-price", "type": "return", "result": "steps.discount.output"}
    ]);
    let bytes = serde_json::to_vec(&document).expect("workflow JSON");

    JsonIrCodec
        .parse_structured(&bytes)
        .expect("the private model retains the broader body shape");
    assert!(
        JsonIrCodec.parse(&bytes).is_err(),
        "publication must stay closed for body shapes without end-to-end evidence"
    );
}

#[test]
fn collection_publication_keeps_nested_bodies_closed() {
    let path = super::repository_root()
        .join("tests/contract/sdk-conformance/expected/collection-builders.workflows.json");
    let source = std::fs::read(path).expect("reviewed SDK golden");
    let golden: Value = serde_json::from_slice(&source).expect("golden JSON");
    let mut document = golden["workflows"]["map"].clone();
    document["bodies"]["line"] = serde_json::json!({
        "entrypoint": "nested",
        "nodes": [
            {
                "id": "nested",
                "type": "forEach",
                "items": "item.parts",
                "body": "part",
                "result": "bodyResult",
                "maxItems": 17,
                "maxConcurrency": 3,
                "next": "return-price"
            },
            {"id": "return-price", "type": "return", "result": "steps.nested.output"}
        ]
    });
    document["bodies"]["part"] = serde_json::json!({
        "entrypoint": "price",
        "nodes": [
            {"id": "price", "type": "activity", "activity": "part.price", "next": "return-part"},
            {"id": "return-part", "type": "return", "result": "steps.price.output"}
        ]
    });
    let bytes = serde_json::to_vec(&document).expect("workflow JSON");

    JsonIrCodec
        .parse_structured(&bytes)
        .expect("the private model retains nested collections");
    assert!(
        JsonIrCodec.parse(&bytes).is_err(),
        "publication must stay closed for nested collections"
    );
}

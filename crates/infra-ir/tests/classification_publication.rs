//! The production publication codec enforces classification before graph conversion.
#![allow(clippy::expect_used)]
use sagat_application::definitions::{IrCodec, problems_of};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn document() -> Value {
    json!({"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow",
        "metadata":{"name":"classification-publication"}, "entrypoint":"done",
        "nodes":[{"id":"done","type":"end"}]})
}

#[test]
fn production_codec_refuses_unsupported_annotations_with_closed_classification_problems() {
    let mut source = document();
    source["metadata"]["x-sagat-classification"] = json!("secret");
    let error = JsonIrCodec.parse(&serde_json::to_vec(&source).expect("bytes"))
        .expect_err("metadata is not a carrier");
    let problems = problems_of(&error);
    assert_eq!(problems[0].code, "CLASSIFICATION_NOT_APPLICABLE");
    assert_eq!(problems[0].path.as_deref(), Some("/metadata/x-sagat-classification"));
    assert!(!problems[0].message.contains("secret"));
}

#[test]
fn production_codec_refuses_lowered_levels_and_keeps_sound_digests_stable() {
    let mut source = document();
    source["inputSchema"] = json!({"type":"object","x-sagat-classification":"pii",
        "properties":{"reference":{"type":"string","x-sagat-classification":"public"}}});
    let bytes = serde_json::to_vec(&source).expect("bytes");
    let error = JsonIrCodec.parse(&bytes).expect_err("lowered declaration");
    assert!(problems_of(&error).iter().any(|p| p.code == "CLASSIFICATION_LOOSENED"));
    source["inputSchema"]["properties"]["reference"]["x-sagat-classification"] = json!("secret");
    let bytes = serde_json::to_vec(&source).expect("bytes");
    let a = JsonIrCodec.parse(&bytes).expect("sound carrier");
    let b = JsonIrCodec.parse(&bytes).expect("same carrier");
    assert_eq!(a.digest, b.digest);
    assert_eq!(a.canonical, b.canonical);
}

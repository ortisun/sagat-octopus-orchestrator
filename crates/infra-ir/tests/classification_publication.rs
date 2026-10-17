//! The production publication codec enforces classification before graph conversion.
#![allow(clippy::expect_used)]
use sagat_application::definitions::{IrCodec, IrError, problems_of};
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
    let error = JsonIrCodec
        .parse(&serde_json::to_vec(&source).expect("bytes"))
        .expect_err("metadata is not a carrier");
    let problems = problems_of(&error);
    assert_eq!(problems[0].code, "CLASSIFICATION_NOT_APPLICABLE");
    assert_eq!(
        problems[0].path.as_deref(),
        Some("/metadata/x-sagat-classification")
    );
    assert!(!problems[0].message.contains("secret"));
}

#[test]
fn production_codec_refuses_lowered_levels_and_keeps_sound_digests_stable() {
    let mut source = document();
    source["inputSchema"] = json!({"type":"object","x-sagat-classification":"pii",
        "properties":{"reference":{"type":"string","x-sagat-classification":"public"}}});
    let bytes = serde_json::to_vec(&source).expect("bytes");
    let error = JsonIrCodec.parse(&bytes).expect_err("lowered declaration");
    assert!(
        problems_of(&error)
            .iter()
            .any(|p| p.code == "CLASSIFICATION_LOOSENED")
    );
    source["inputSchema"]["properties"]["reference"]["x-sagat-classification"] = json!("secret");
    let bytes = serde_json::to_vec(&source).expect("bytes");
    let a = JsonIrCodec.parse(&bytes).expect("sound carrier");
    let b = JsonIrCodec.parse(&bytes).expect("same carrier");
    assert_eq!(a.digest, b.digest);
    assert_eq!(a.canonical, b.canonical);
}

#[test]
fn production_compiles_annotated_root_schemas_but_preserves_unannotated_legacy_roots() {
    let invalid_schema = json!({"type":"not-a-json-schema-type"});
    let mut legacy = document();
    legacy["inputSchema"] = invalid_schema.clone();
    JsonIrCodec
        .parse(&serde_json::to_vec(&legacy).expect("bytes"))
        .expect("unannotated legacy root remains uncompiled");

    legacy["inputSchema"]["x-sagat-classification"] = json!("pii");
    let error = JsonIrCodec
        .parse(&serde_json::to_vec(&legacy).expect("bytes"))
        .expect_err("annotated root must compile");
    let IrError::Schema(issues) = error else {
        panic!("expected schema error");
    };
    assert_eq!(issues[0].path, "/inputSchema");
    assert_eq!(
        issues[0].message,
        "classification carrier schema is not a supported JSON Schema"
    );
}

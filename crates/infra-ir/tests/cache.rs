//! The definition cache is bounded by weight as well as by count (a security-review finding: 1024
//! entries of 8 MB each are 8 GB in a 512 MB container).
#![allow(clippy::expect_used)]

use std::sync::Arc;

use sagat_application::execution::DefinitionCache;
use sagat_domain::definition::WorkflowVersionId;
use sagat_infra_ir::JsonIrCodec;
use serde_json::json;

/// A chain of `n` activities as a workflow document.
fn chain(name: &str, n: usize) -> Vec<u8> {
    let nodes: Vec<_> = (0..n)
        .map(|i| {
            let mut node = json!({"id": format!("n{i}"), "type": "activity", "activity": "act.x"});
            if i + 1 < n {
                node["next"] = json!(format!("n{}", i + 1));
            }
            node
        })
        .collect();
    serde_json::to_vec(&json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "n0", "nodes": nodes,
    }))
    .expect("json")
}

#[test]
fn old_entries_are_evicted_by_weight_not_only_by_count() {
    let codec = Arc::new(JsonIrCodec);
    // Room for about two thousand-node definitions, though the count limit would allow a hundred.
    let one = JsonIrCodec;
    let sample = sagat_application::definitions::IrCodec::parse(&one, &chain("sample", 1000))
        .expect("parses")
        .definition
        .footprint();
    let cache = DefinitionCache::with_budget(codec, 100, sample * 5 / 2);

    let (a, b, c) = (
        WorkflowVersionId::generate(),
        WorkflowVersionId::generate(),
        WorkflowVersionId::generate(),
    );
    for (id, name) in [(a, "aaa-flow"), (b, "bbb-flow"), (c, "ccc-flow")] {
        cache.get_or_parse(id, &chain(name, 1000)).expect("parses");
    }
    assert!(
        cache.get(a).is_none(),
        "the least recently used was evicted to stay within the weight"
    );
    assert!(cache.get(b).is_some() && cache.get(c).is_some());

    // Using an entry protects it: `b` is now the most recent, so the next insertion evicts `c`.
    assert!(cache.get(b).is_some());
    let d = WorkflowVersionId::generate();
    cache
        .get_or_parse(d, &chain("ddd-flow", 1000))
        .expect("parses");
    assert!(cache.get(c).is_none());
    assert!(cache.get(b).is_some() && cache.get(d).is_some());
}

#[test]
fn a_definition_heavier_than_the_whole_budget_still_works_and_is_the_only_one_kept() {
    let cache = DefinitionCache::with_budget(Arc::new(JsonIrCodec), 100, 1);
    let (a, b) = (WorkflowVersionId::generate(), WorkflowVersionId::generate());
    cache
        .get_or_parse(a, &chain("aaa-flow", 50))
        .expect("parses");
    assert!(
        cache.get(a).is_some(),
        "the entry just parsed is always usable"
    );
    cache
        .get_or_parse(b, &chain("bbb-flow", 50))
        .expect("parses");
    assert!(cache.get(b).is_some());
    assert!(cache.get(a).is_none(), "and it displaced the other");
}

#[test]
fn stored_legacy_schema_extensions_do_not_change_execution_restore() {
    use sagat_application::definitions::{IrCodec, IrError};
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/payload-classification/fixtures");
    // Exact release v0.1.0 contract at 6ede1c7849c4ac5a209c08da8ad25294fbdd7795.
    let old_schema_bytes =
        std::fs::read(fixtures.join("v0.1.0-workflow.schema.json")).expect("frozen release schema");
    use sha2::{Digest as _, Sha256};
    assert_eq!(
        sagat_domain::workflow::Digest::new(Sha256::digest(&old_schema_bytes).into()).to_hex(),
        "5035e781a2a531da0317594cd8b082fb9291d8e3f0a7b0322625c2260d89bff4",
        "the frozen release contract must not drift"
    );
    let old_schema: serde_json::Value =
        serde_json::from_slice(&old_schema_bytes).expect("release schema JSON");
    let bytes = std::fs::read(fixtures.join("legacy-annotation-extension.workflow.json"))
        .expect("legacy document");
    let doc: serde_json::Value = serde_json::from_slice(&bytes).expect("legacy JSON");
    assert!(
        jsonschema::validator_for(&old_schema)
            .expect("release schema compiles")
            .is_valid(&doc),
        "the release contract accepted the extension as metadata"
    );
    assert!(
        matches!(JsonIrCodec.parse(&bytes), Err(IrError::Classification(_))),
        "new publications must still validate new classification annotations"
    );
    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 8);
    cache
        .get_or_parse(WorkflowVersionId::generate(), &bytes)
        .expect(
            "restoring a stored definition must not apply new authoring-only classification rules",
        );
}

#[test]
fn stored_parsing_keeps_structural_validation_and_current_canonical_bytes() {
    use sagat_application::definitions::IrCodec;
    let mut value: serde_json::Value =
        serde_json::from_slice(&chain("stored-current", 2)).expect("workflow");
    value["inputSchema"] = json!({"type":"object","x-sagat-classification":"public","additionalProperties":false,"properties":{"reference":{"type":"string","maxLength":128}}});
    let bytes = serde_json::to_vec(&value).expect("current bytes");
    let publication = JsonIrCodec.parse(&bytes).expect("current publication");
    let stored = JsonIrCodec.parse_stored(&bytes).expect("stored definition");
    assert_eq!(stored.canonical, publication.canonical);
    assert_eq!(stored.digest, publication.digest);
    assert_eq!(
        stored.definition.total_len(),
        publication.definition.total_len()
    );
    value["nodes"][0]["next"] = json!("n0");
    assert!(
        JsonIrCodec
            .parse_stored(&serde_json::to_vec(&value).expect("cycle bytes"))
            .is_err(),
        "restoration still rejects a business graph cycle"
    );
    value["nodes"][0]["type"] = json!("unknown-node-kind");
    assert!(
        JsonIrCodec
            .parse_stored(&serde_json::to_vec(&value).expect("unknown bytes"))
            .is_err(),
        "restoration still rejects unsupported IR"
    );
}

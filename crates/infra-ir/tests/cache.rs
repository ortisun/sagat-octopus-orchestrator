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

/// T043-1, FR-043-001: the parsed definition carries the canonical bytes of its `inputSchema` and their SHA-256,
/// whatever the spacing and key order of the document, on publication and on restore, so every creator that holds
/// the definition holds the schema (no creator reads the database for it).
#[test]
fn the_parsed_definition_carries_the_canonical_input_schema_and_its_sha256() {
    use sagat_application::definitions::IrCodec;
    use sha2::{Digest as _, Sha256};
    let mut value: serde_json::Value =
        serde_json::from_slice(&chain("input-schema", 2)).expect("workflow");
    assert!(
        JsonIrCodec
            .parse(&serde_json::to_vec(&value).expect("bytes"))
            .expect("parses")
            .definition
            .input_schema()
            .is_none(),
        "a document without an inputSchema has none"
    );
    let written = r#"{ "required" : ["orderId"],"type":"object",
        "properties": {"orderId": {"type": "string", "maxLength": 64}} }"#;
    value["inputSchema"] = serde_json::from_str(written).expect("schema");
    let bytes = serde_json::to_vec(&value).expect("bytes");
    let canonical =
        br#"{"properties":{"orderId":{"maxLength":64,"type":"string"}},"required":["orderId"],"type":"object"}"#;
    let expected: [u8; 32] = Sha256::digest(canonical).into();
    for parsed in [
        JsonIrCodec.parse(&bytes).expect("publication"),
        JsonIrCodec.parse_stored(&bytes).expect("restore"),
    ] {
        let schema = parsed
            .definition
            .input_schema()
            .expect("the definition carries the schema");
        assert_eq!(schema.bytes(), canonical, "the canonical bytes");
        assert_eq!(schema.digest(), &expected, "their SHA-256");
        assert!(
            !format!("{schema:?}").contains("orderId"),
            "the debug form shows the size and digest, not the schema"
        );
    }
    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 8);
    let restored = cache
        .get_or_parse(WorkflowVersionId::generate(), &bytes)
        .expect("restores");
    assert_eq!(
        restored.input_schema().map(|schema| schema.digest()),
        Some(&expected),
        "the definition cache hands the schema to every creator"
    );
}

/// T043-1: a continuation document (continue-as-new holds the definition and not the document) carries it too.
#[test]
fn a_continuation_definition_carries_its_input_schema() {
    use sagat_application::definitions::IrCodec;
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "continuation-schema"}, "entrypoint": "work",
        "inputSchema": {"type": "object", "required": ["next"]},
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 86_400_000}
    });
    let bytes = serde_json::to_vec(&document).expect("bytes");
    for parsed in [
        JsonIrCodec.parse(&bytes).expect("publication"),
        JsonIrCodec.parse_stored(&bytes).expect("restore"),
    ] {
        assert_eq!(
            parsed.definition.input_schema().map(|s| s.bytes().to_vec()),
            Some(br#"{"required":["next"],"type":"object"}"#.to_vec())
        );
    }
}

/// T043-1, NFR-043-PERF: the schema bytes are counted by the footprint, so the byte budget of the definition cache
/// still bounds memory when definitions carry large schemas.
#[test]
fn the_input_schema_is_counted_by_the_definition_footprint() {
    use sagat_application::definitions::IrCodec;
    let mut value: serde_json::Value =
        serde_json::from_slice(&chain("footprint-schema", 2)).expect("workflow");
    let without = JsonIrCodec
        .parse(&serde_json::to_vec(&value).expect("bytes"))
        .expect("parses")
        .definition
        .footprint();
    let names: Vec<String> = (0..2_000).map(|i| format!("field{i:05}")).collect();
    value["inputSchema"] = json!({"type": "object", "required": names});
    let parsed = JsonIrCodec
        .parse(&serde_json::to_vec(&value).expect("bytes"))
        .expect("parses");
    let schema_len = parsed.definition.input_schema().expect("schema").len();
    assert!(schema_len > 20_000, "{schema_len}");
    assert!(
        parsed.definition.footprint() >= without + schema_len,
        "footprint {} must count the {schema_len} schema bytes over {without}",
        parsed.definition.footprint()
    );
}

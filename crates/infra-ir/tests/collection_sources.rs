//! Publication boundaries for ordinary activity prefixes producing collection membership.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::{IrCodec, IrError};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn source_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "source-prefix"}, "entrypoint": "fetch",
        "limits": {"maxInstances": 16, "maxDepth": 1},
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "line.work", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "fetch", "type": "activity", "activity": "source.fetch", "next": "items"},
            {"id": "items", "type": "forEach", "items": "steps.fetch.output", "body": "line",
             "result": "bodyResult", "maxItems": 3, "maxConcurrency": 2, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

#[test]
fn source_activity_effect_and_error_combinations_remain_explicitly_unsupported() {
    for (field, value) in [
        ("onError", json!("done")),
        ("compensateWith", json!("source.undo")),
        ("reconcileWith", json!("source.lookup")),
        ("irreversible", json!(true)),
        ("highRisk", json!(true)),
    ] {
        let mut document = source_document();
        document["nodes"][0][field] = value;
        let error = JsonIrCodec
            .parse(document.to_string().as_bytes())
            .expect_err(field);
        assert!(
            matches!(error, IrError::Unsupported { node, reason, .. }
            if node == "fetch" && reason.contains("collection source activities")),
            "{field}"
        );
    }
}

#[test]
fn accepting_a_source_prefix_does_not_enable_activity_successors_of_collections() {
    let mut document = source_document();
    document["nodes"][1]["next"] = json!("post");
    document["nodes"]
        .as_array_mut()
        .expect("root nodes")
        .push(json!({"id": "post", "type": "activity", "activity": "source.post", "next": "done"}));
    let error = JsonIrCodec
        .parse(document.to_string().as_bytes())
        .expect_err("successor gate");
    assert!(matches!(error, IrError::Unsupported { node, .. } if node == "post"));
}

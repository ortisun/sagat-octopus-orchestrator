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
    // Return-only entry has a separate admission/budget gate; control entry is now proven.
    document["bodies"]["line"]["nodes"] = serde_json::json!([
        {"id": "price", "type": "return", "result": "item"}
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
fn five_sdk_control_entry_documents_match_the_public_canonical_definition() {
    let path = super::repository_root()
        .join("tests/contract/sdk-conformance/expected/collection-control-entry.workflows.json");
    let source = std::fs::read(path).expect("reviewed all-five control entry reference");
    let golden: Value = serde_json::from_slice(&source).expect("golden JSON");
    let workflows = golden["workflows"].as_object().expect("workflows");
    assert_eq!(workflows.len(), 2);
    for (name, document) in workflows {
        let parsed = JsonIrCodec
            .parse(&serde_json::to_vec(document).expect("workflow JSON"))
            .expect("the runtime-proven control entry is publishable");
        assert_eq!(
            String::from_utf8(parsed.canonical).expect("canonical UTF-8"),
            golden["canonical"][name]
                .as_str()
                .expect("reviewed canonical text")
        );
    }
}

fn chain_of(activities: usize, extra: &serde_json::Value) -> serde_json::Value {
    let mut nodes = Vec::new();
    for position in 0..activities {
        let mut node = serde_json::json!({
            "id": format!("step{position}"), "type": "activity",
            "activity": format!("line.step{position}"),
            "next": if position + 1 == activities {
                "return-line".to_owned()
            } else {
                format!("step{}", position + 1)
            }
        });
        if let Some(fields) = extra.as_object() {
            for (key, value) in fields {
                node[key] = value.clone();
            }
        }
        nodes.push(node);
    }
    nodes.push(serde_json::json!({"id": "return-line", "type": "return", "result": "item"}));
    serde_json::json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "compensating-chain"},
        "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "step0", "nodes": nodes}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines", "body": "line",
             "result": "bodyResult", "maxItems": 8, "maxConcurrency": 2, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn publishes(document: &serde_json::Value) -> bool {
    JsonIrCodec
        .parse(&serde_json::to_vec(document).expect("workflow JSON"))
        .is_ok()
}

#[test]
fn collection_publication_accepts_a_chain_of_compensable_activities_and_stops_at_eight() {
    let compensable =
        serde_json::json!({"compensateWith": "line.undo", "reconcileWith": "line.lookup"});
    assert!(publishes(&chain_of(1, &compensable)));
    assert!(publishes(&chain_of(3, &compensable)));
    assert!(publishes(&chain_of(8, &compensable)));
    assert!(
        !publishes(&chain_of(9, &compensable)),
        "nine activities are more than the runtime tests exercise"
    );
    // The effect declarations publish with the chain: the derived companions are not nodes of it.
    assert!(publishes(&chain_of(
        2,
        &serde_json::json!({"irreversible": true})
    )));
    assert!(publishes(&chain_of(
        2,
        &serde_json::json!({"highRisk": true, "reconcileWith": "line.lookup"})
    )));
    assert!(publishes(&chain_of(
        2,
        &serde_json::json!({
            "compensateWith": "line.undo",
            "compensationRetry": {"maxAttempts": 3, "initialDelayMs": 10, "maxDelayMs": 10,
                                  "strategy": "fixed", "jitter": false},
            "compensationTimeoutMs": 5000
        })
    )));
    // An effect that cannot be undone has no compensation (the definition rules, unchanged).
    assert!(!publishes(&chain_of(
        2,
        &serde_json::json!({"irreversible": true, "compensateWith": "line.undo"})
    )));
}

#[test]
fn loop_publication_accepts_a_chain_of_compensable_activities_and_stops_at_eight() {
    let mut document = chain_of(3, &serde_json::json!({"compensateWith": "line.undo"}));
    document["entrypoint"] = serde_json::json!("poll");
    document["nodes"] = serde_json::json!([
        {"id": "poll", "type": "while", "condition": "carry < 3", "body": "line",
         "initial": "0", "carry": "bodyResult", "maxIterations": 5, "timeoutMs": 60000,
         "next": "done"},
        {"id": "done", "type": "end"}
    ]);
    assert!(publishes(&document));
    let mut long = chain_of(9, &serde_json::json!({"compensateWith": "line.undo"}));
    long["entrypoint"] = serde_json::json!("poll");
    long["nodes"] = document["nodes"].clone();
    assert!(!publishes(&long));
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

#[test]
fn bounded_collection_event_waits_publish_at_entry_and_after_an_activity() {
    for at_entry in [true, false] {
        let mut document = chain_of(1, &serde_json::json!({}));
        let wait = serde_json::json!({"id":"notified","type":"waitEvent","eventType":"payment.notified","correlationExpr":"item.key","timeoutMs":60000,"lookbackMs":0,"next":"return-line"});
        if at_entry {
            document["bodies"]["line"]["entrypoint"] = serde_json::json!("notified");
            document["bodies"]["line"]["nodes"] =
                serde_json::json!([wait,{"id":"return-line","type":"return","result":"item"}]);
        } else {
            document["bodies"]["line"]["nodes"][0]["next"] = serde_json::json!("notified");
            document["bodies"]["line"]["nodes"]
                .as_array_mut()
                .expect("body nodes")
                .insert(1, wait);
        }
        let bytes = serde_json::to_vec(&document).expect("definition");
        JsonIrCodec
            .parse_structured(&bytes)
            .expect("the internal model retains the event shape");
        JsonIrCodec
            .parse(&bytes)
            .expect("bounded public scoped event wait");
    }
}

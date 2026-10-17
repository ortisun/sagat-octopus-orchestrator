//! The data-model ceilings of T014-004, asserted at both gates a definition passes: the canonical
//! JSON Schema and the publication codec (`JsonIrCodec`, the parse the server runs). Each ceiling
//! is checked at its last accepted value and its first rejected one.
//!
//! `nodeId` 1..64, definition nodes 10000, nesting depth 1..8, effective expressions of 1024
//! bytes, list literals of 64 and constructed lists of 4096, no object literals, `maxItems` and
//! `maxIterations` 1..10000, `maxConcurrency` 1..256.

use jsonschema::Validator;
use sagat_application::definitions::IrCodec;
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};
use std::sync::OnceLock;

fn validator() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let path = super::repository_root()
            .join("specs/001-platform-foundation/contracts/workflow.schema.json");
        let source = std::fs::read_to_string(path).expect("canonical workflow schema exists");
        let schema: Value = serde_json::from_str(&source).expect("workflow schema is JSON");
        jsonschema::options()
            .build(&schema)
            .expect("workflow schema compiles without external retrieval")
    })
}

fn collection(items: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "ceilings"}, "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "line.process", "next": "return"},
            {"id": "return", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": items, "body": "line",
             "result": "bodyResult", "maxItems": 17, "maxConcurrency": 3, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn loop_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "ceilings"}, "entrypoint": "again",
        "bodies": {"iteration": {"entrypoint": "step", "nodes": [
            {"id": "step", "type": "activity", "activity": "loop.step", "next": "return"},
            {"id": "return", "type": "return", "result": "steps.step.output"}
        ]}},
        "nodes": [
            {"id": "again", "type": "while", "condition": "carry < 3", "body": "iteration",
             "initial": "0", "carry": "bodyResult", "maxIterations": 5, "timeoutMs": 600_000,
             "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

/// Accepted by both gates, or by neither: the schema and the codec must agree on each ceiling.
fn gates(document: &Value) -> (bool, bool) {
    let bytes = serde_json::to_vec(document).expect("fixture JSON");
    (
        validator().is_valid(document),
        JsonIrCodec.parse(&bytes).is_ok(),
    )
}

fn assert_accepted(label: &str, document: &Value) {
    assert_eq!(gates(document), (true, true), "{label} must be accepted");
}

fn assert_rejected(label: &str, document: &Value) {
    // The schema is the editor-time gate: it may accept what the codec refuses (expression syntax,
    // graph closure), so the publication codec is the one that must say no.
    let (_, codec) = gates(document);
    assert!(!codec, "{label} must be rejected at publication");
}

fn set(document: &Value, pointer: &str, value: Value) -> Value {
    let mut changed = document.clone();
    *changed.pointer_mut(pointer).expect("field") = value;
    changed
}

#[test]
fn node_ids_are_one_to_sixty_four_characters() {
    let base = collection("input.lines");
    for (label, id) in [
        ("1 character", "a".to_owned()),
        ("64 characters", "a".repeat(64)),
    ] {
        let mut document = set(&base, "/nodes/1/id", json!(id));
        document["nodes"][0]["next"] = json!(id);
        assert_accepted(label, &document);
    }
    for (label, id) in [("empty", String::new()), ("65 characters", "a".repeat(65))] {
        let mut document = set(&base, "/nodes/1/id", json!(id.clone()));
        document["nodes"][0]["next"] = json!(id);
        assert_rejected(label, &document);
        assert!(
            !validator().is_valid(&document),
            "{label}: the schema agrees"
        );
    }
    // The same bounds apply inside a body.
    let long = "b".repeat(64);
    let mut document = base.clone();
    document["bodies"]["line"]["entrypoint"] = json!(long);
    document["bodies"]["line"]["nodes"][0]["id"] = json!(long);
    assert_accepted("a 64 character body node", &document);
    let too_long = "b".repeat(65);
    document["bodies"]["line"]["entrypoint"] = json!(too_long);
    document["bodies"]["line"]["nodes"][0]["id"] = json!(too_long);
    assert_rejected("a 65 character body node", &document);
}

#[test]
fn a_definition_has_at_most_ten_thousand_nodes() {
    // A chain of activities ending at `end`: n - 1 activities and the end node.
    let chain = |nodes: usize| {
        let mut list: Vec<Value> = (0..nodes - 1)
            .map(|n| {
                json!({"id": format!("n{n}"), "type": "activity", "activity": "chain.step",
                       "next": if n + 2 == nodes { "done".to_owned() } else { format!("n{}", n + 1) }})
            })
            .collect();
        list.push(json!({"id": "done", "type": "end"}));
        json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
            "metadata": {"name": "ceilings"}, "entrypoint": "n0", "nodes": list
        })
    };
    assert_accepted("10,000 nodes", &chain(10_000));
    let over = chain(10_001);
    assert_rejected("10,001 nodes", &over);
    assert!(!validator().is_valid(&over), "the schema agrees");
}

#[test]
fn item_and_iteration_limits_are_one_to_ten_thousand_and_concurrency_one_to_256() {
    let base = collection("input.lines");
    let loop_base = loop_document();
    for (label, document, pointer, accepted, rejected) in [
        (
            "maxItems",
            &base,
            "/nodes/0/maxItems",
            [1_u64, 10_000],
            [0_u64, 10_001],
        ),
        (
            "maxConcurrency",
            &base,
            "/nodes/0/maxConcurrency",
            [1, 256],
            [0, 257],
        ),
        (
            "maxIterations",
            &loop_base,
            "/nodes/0/maxIterations",
            [1, 10_000],
            [0, 10_001],
        ),
    ] {
        for value in accepted {
            assert_accepted(
                &format!("{label} {value}"),
                &set(document, pointer, json!(value)),
            );
        }
        for value in rejected {
            let invalid = set(document, pointer, json!(value));
            assert_rejected(&format!("{label} {value}"), &invalid);
            assert!(
                !validator().is_valid(&invalid),
                "{label} {value}: the schema agrees"
            );
        }
    }
    for text in [json!("10"), json!(1.5), json!(null)] {
        assert_rejected(
            "a non-integer maxItems",
            &set(&base, "/nodes/0/maxItems", text),
        );
    }
}

#[test]
fn nesting_depth_is_one_to_eight() {
    let base = collection("input.lines");
    for depth in [1_u64, 8] {
        let mut document = base.clone();
        document["limits"] = json!({"maxDepth": depth});
        assert_accepted(&format!("maxDepth {depth}"), &document);
    }
    for depth in [0_u64, 9] {
        let mut document = base.clone();
        document["limits"] = json!({"maxDepth": depth});
        assert_rejected(&format!("maxDepth {depth}"), &document);
    }
}

#[test]
fn an_items_expression_is_at_most_1024_bytes_and_cannot_build_objects() {
    // Padding keeps the expression valid while reaching the byte limit exactly.
    let exact = format!("input.lines{}", " ".repeat(1024 - "input.lines".len()));
    assert_eq!(exact.len(), 1024);
    assert_accepted("a 1024 byte expression", &collection(&exact));
    let over = format!("{exact} ");
    assert_eq!(over.len(), 1025);
    assert_rejected("a 1025 byte expression", &collection(&over));

    for literal in [
        "[{'a': 1}]",
        "{'a': 1}",
        "[1, {'b': 2}]",
        "input.lines + [{'a': 1}]",
    ] {
        assert_rejected(
            &format!("the object literal in `{literal}`"),
            &collection(literal),
        );
    }
}

/// The 4096-element ceiling of constructed (concatenated) lists is a bound of evaluation, not of
/// parsing; it is asserted where it acts, in `identity_limits::effective_expression_limits_remain_bounded_for_additive_v1`.
#[test]
fn list_literals_have_at_most_64_elements_at_publication() {
    let literal = |n: usize| format!("[{}]", vec!["0"; n].join(","));
    assert_accepted("a 64 element literal", &collection(&literal(64)));
    assert_rejected("a 65 element literal", &collection(&literal(65)));
}

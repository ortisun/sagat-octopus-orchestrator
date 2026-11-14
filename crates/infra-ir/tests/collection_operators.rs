//! The `operators` of a `forEach` node in the IR codec (design 042, T042-6): every earlier document still decodes and
//! encodes byte for byte, a chain decodes on every path, and publication's refusals name the node, the stage and the
//! operator (FR-042-001, FR-042-002, FR-042-005, NFR-042-004).
#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use proptest::prelude::*;
use sagat_application::definitions::{IrCodec, IrError, OperatorRefusal, ParsedDefinition};
use sagat_domain::operators::{Operator, OperatorChain};
use sagat_domain::workflow::{NodeId, NodeKind, WorkflowDefinition};
use sagat_infra_ir::{JsonIrCodec, canonical_chain, chain_digest};
use serde_json::{Value, json};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden(name: &str) -> Value {
    let path = repository()
        .join("tests/contract/sdk-conformance/expected")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).expect("reviewed golden")).expect("golden JSON")
}

/// The publishable collection of the SDK goldens, with `operators` on its `forEach`.
fn chained(operators: Value) -> Value {
    let mut document = golden("collection-builders.workflows.json")["workflows"]["map"].clone();
    document["nodes"][0]["operators"] = operators;
    document
}

fn bytes(document: &Value) -> Vec<u8> {
    serde_json::to_vec(document).expect("JSON")
}

fn chain_of(definition: &WorkflowDefinition) -> Option<&OperatorChain> {
    match definition
        .node(&NodeId::parse("prices").expect("id"))
        .expect("the collection")
        .kind()
    {
        NodeKind::ForEach { operators, .. } => operators.as_ref(),
        other => panic!("expected a collection, got {other:?}"),
    }
}

/// Every path through the codec: publication, the restore of a stored version, and the structured validation.
fn every_path(document: &Value) -> [Result<ParsedDefinition, IrError>; 3] {
    let bytes = bytes(document);
    [
        JsonIrCodec.parse(&bytes),
        JsonIrCodec.parse_stored(&bytes),
        JsonIrCodec.parse_structured(&bytes),
    ]
}

fn for_each_nodes(definition: &WorkflowDefinition) -> Vec<&NodeKind> {
    definition
        .nodes()
        .chain(definition.bodies().flat_map(|body| body.nodes()))
        .map(|node| node.kind())
        .filter(|kind| matches!(kind, NodeKind::ForEach { .. }))
        .collect()
}

// --- earlier documents ------------------------------------------------------------------------------------------------

/// Every document of the reviewed SDK goldens that the codec restores still does, with no chain, and its canonical bytes
/// are exactly the reviewed ones written before the field existed: a `forEach` without `operators` encodes byte for byte
/// as before, so its definition digest is unchanged.
#[test]
fn every_earlier_document_still_decodes_and_encodes_byte_for_byte() {
    let directory = repository().join("tests/contract/sdk-conformance/expected");
    let mut checked = 0;
    let mut refused = 0;
    let mut collections = 0;
    let mut files: Vec<_> = std::fs::read_dir(&directory)
        .expect("goldens")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.to_string_lossy().ends_with(".workflows.json"))
        .collect();
    files.sort();
    for path in files {
        let golden: Value =
            serde_json::from_slice(&std::fs::read(&path).expect("golden")).expect("golden JSON");
        let (Some(workflows), Some(canonical)) = (
            golden["workflows"].as_object(),
            golden["canonical"].as_object(),
        ) else {
            continue;
        };
        for (name, document) in workflows {
            let Some(expected) = canonical.get(name).and_then(Value::as_str) else {
                continue;
            };
            let parsed = match JsonIrCodec.parse_stored(&bytes(document)) {
                Ok(parsed) => parsed,
                // Some goldens are shapes the server keeps closed (an unproven continuation, say). They stay refused
                // for their own reason, never for the new field.
                Err(
                    error @ (IrError::Schema(_)
                    | IrError::Operator { .. }
                    | IrError::CollectionOperatorsDisabled { .. }),
                ) => panic!("{} {name} no longer decodes: {error}", path.display()),
                Err(_) => {
                    refused += 1;
                    continue;
                }
            };
            // This one golden holds the SDKs' ECMAScript number text (`1e+21`, `-1e-7`), which the server writes in its
            // own form; it has no collection, and it decodes like the others.
            if !(path.ends_with("builders.workflows.json") && name == "canonical-edges") {
                assert_eq!(
                    String::from_utf8(parsed.canonical).expect("UTF-8"),
                    expected,
                    "{} {name}: the canonical bytes changed",
                    path.display()
                );
            }
            let kinds = for_each_nodes(&parsed.definition);
            assert!(
                kinds.iter().all(|kind| matches!(
                    kind,
                    NodeKind::ForEach {
                        operators: None,
                        ..
                    }
                )),
                "{} {name}",
                path.display()
            );
            collections += usize::from(!kinds.is_empty());
            checked += 1;
        }
    }
    assert!(
        checked >= 100,
        "only {checked} documents checked ({refused} refused)"
    );
    assert!(collections >= 20, "only {collections} collections checked");
}

/// The digest of a `forEach` without operators, pinned: the digest a stored version is identified by. The value is the
/// SHA-256 of the reviewed canonical text of the golden, computed outside the codec.
#[test]
fn a_collection_without_operators_keeps_its_digest() {
    let document = golden("collection-builders.workflows.json")["workflows"]["map"].clone();
    let parsed = JsonIrCodec.parse(&bytes(&document)).expect("publishable");
    assert_eq!(
        parsed.digest.to_hex(),
        "4a88df1e9412a7029b31ad7f6c8222fd7b03b3da0b8083c7f964f7e80f49205b",
        "the digest of an unchanged collection moved"
    );
}

// --- a chain ----------------------------------------------------------------------------------------------------------

#[test]
fn a_chain_of_the_seven_operators_decodes_on_every_path() {
    let document = chained(json!([
        {"filter": {"where": "element.total > 0 && position < 900"}},
        {"distinct": {"by": "element.customer"}},
        {"map": {"select": "element.lines"}},
        {"flatMap": {"select": "element"}},
        {"skip": {"count": 0}},
        {"take": {"count": 1000}},
        {"distinct": {}},
        {"window": {"count": 50}}
    ]));
    for parsed in every_path(&document) {
        let parsed = parsed.expect("a chained collection decodes");
        let chain = chain_of(&parsed.definition).expect("the chain");
        let names: Vec<&str> = chain.stages().iter().map(|s| s.name().as_str()).collect();
        assert_eq!(
            names,
            [
                "filter", "distinct", "map", "flatMap", "skip", "take", "distinct", "window"
            ]
        );
        assert_eq!(chain.stages()[6], Operator::Distinct { by: None });
        assert_eq!(chain.stages()[7], Operator::Window { count: 50 });
    }
}

#[test]
fn stage_expressions_may_read_the_element_its_position_and_what_items_reads() {
    for variable in ["element", "position", "input", "steps", "run", "state"] {
        let document = chained(json!([{"filter": {"where": format!("has({variable}.x)")}}]));
        JsonIrCodec
            .parse(&bytes(&document))
            .unwrap_or_else(|e| panic!("{variable}: {e}"));
    }
}

// --- refusals ---------------------------------------------------------------------------------------------------------

/// The refusal every path gives, the same on each: publication, restore and structured validation.
fn refusal(document: &Value) -> (String, usize, String, OperatorRefusal, String) {
    let refusals: Vec<_> = every_path(document)
        .into_iter()
        .map(|result| match result {
            Err(IrError::Operator {
                node,
                stage,
                operator,
                refusal,
                message,
            }) => {
                assert_eq!(node, "prices", "the error names the node");
                (node, stage, operator, refusal, message)
            }
            other => panic!("expected a refused stage, got {other:?}"),
        })
        .collect();
    assert!(
        refusals.windows(2).all(|pair| pair[0] == pair[1]),
        "{refusals:?}"
    );
    refusals.into_iter().next().expect("three paths")
}

#[test]
fn an_unknown_operator_is_refused_naming_the_stage_and_the_name() {
    let (_, stage, operator, kind, _) = refusal(&chained(
        json!([{"take": {"count": 2}}, {"throttle": {"ms": 10}}]),
    ));
    assert_eq!(
        (stage, operator.as_str(), kind),
        (1, "throttle", OperatorRefusal::Unknown)
    );
}

#[test]
fn a_trigger_operator_on_a_collection_is_refused() {
    for (operator, params) in [
        ("debounce", json!({"quietMs": 1000, "maxWaitMs": 5000})),
        ("ignoreElements", json!({})),
    ] {
        let mut stage = serde_json::Map::new();
        stage.insert(operator.to_owned(), params);
        let (_, index, named, kind, message) = refusal(&chained(json!([
            {"map": {"select": "element"}},
            {"filter": {"where": "true"}},
            Value::Object(stage)
        ])));
        assert_eq!(
            (index, named.as_str(), kind),
            (2, operator, OperatorRefusal::Surface)
        );
        assert!(message.contains("trigger"), "{message}");
    }
}

#[test]
fn a_stage_that_reads_an_iteration_variable_is_refused() {
    for variable in ["item", "index", "iteration", "carry", "bodyResult"] {
        let (_, stage, operator, kind, message) = refusal(&chained(json!([
            {"take": {"count": 5}},
            {"map": {"select": format!("[element, {variable}]")}}
        ])));
        assert_eq!(
            (stage, operator.as_str(), kind),
            (1, "map", OperatorRefusal::Variable),
            "{variable}"
        );
        assert!(message.contains(variable), "{message}");
    }
    // A name no stage has is refused too: a stage sees a closed set.
    let (_, _, _, kind, _) = refusal(&chained(json!([{"distinct": {"by": "node.id"}}])));
    assert_eq!(kind, OperatorRefusal::Variable);
}

#[test]
fn a_stage_expression_is_held_to_the_present_limits() {
    // Does not compile.
    let (_, stage, operator, kind, _) =
        refusal(&chained(json!([{"filter": {"where": "element >"}}])));
    assert_eq!(
        (stage, operator.as_str(), kind),
        (0, "filter", OperatorRefusal::Expression)
    );
    // Within the schema's 1,024 characters, over the 1,024 bytes.
    let long = format!("element == '{}'", "é".repeat(600));
    let (_, _, operator, kind, _) = refusal(&chained(json!([{"flatMap": {"select": long}}])));
    assert_eq!(
        (operator.as_str(), kind),
        ("flatMap", OperatorRefusal::Expression)
    );
    // Over the token ceiling.
    let tokens = vec!["1"; 130].join("+");
    let (_, _, _, kind, _) = refusal(&chained(json!([{"map": {"select": tokens}}])));
    assert_eq!(kind, OperatorRefusal::Expression);
}

fn schema_issue_paths(document: &Value) -> Vec<String> {
    match JsonIrCodec.parse(&bytes(document)) {
        Err(IrError::Schema(issues)) => issues.into_iter().map(|issue| issue.path).collect(),
        other => panic!("expected a schema refusal, got {other:?}"),
    }
}

#[test]
fn parameters_out_of_range_and_malformed_chains_are_refused_by_the_schema() {
    for operators in [
        json!([{"window": {"count": 0}}]),
        json!([{"window": {"count": 1001}}]),
        json!([{"skip": {"count": 1001}}]),
        json!([{"take": {"count": -1}}]),
        json!([{"filter": {"where": "true", "extra": 1}}]),
        json!([{"filter": {"where": "true"}, "map": {"select": "element"}}]),
        json!([{"map": {}}]),
        json!([]),
        json!(vec![json!({"take": {"count": 1}}); 9]),
    ] {
        let paths = schema_issue_paths(&chained(operators.clone()));
        assert!(
            paths.iter().any(|p| p.starts_with("/nodes/0")),
            "{operators}: {paths:?}"
        );
    }
}

// --- the canonical encoding -------------------------------------------------------------------------------------------

#[test]
fn the_chain_encoding_is_the_canonical_declaration() {
    let document = chained(json!([
        {"filter": {"where": "element.total > 0"}},
        {"distinct": {}},
        {"window": {"count": 50}}
    ]));
    let parsed = JsonIrCodec.parse(&bytes(&document)).expect("chain");
    let chain = chain_of(&parsed.definition).expect("chain");
    assert_eq!(
        String::from_utf8(canonical_chain(chain)).expect("UTF-8"),
        r#"[{"filter":{"where":"element.total > 0"}},{"distinct":{}},{"window":{"count":50}}]"#
    );
    assert_eq!(
        chain_digest(chain).to_hex(),
        "bcd47919ab414f309b4c83ca80ddb019197613364911a7cfec9f58a0c714379f"
    );
}

fn any_expression() -> impl Strategy<Value = String> {
    "[a-z]{1,8}".prop_map(|field| format!("element.k{field}"))
}

fn any_stage() -> impl Strategy<Value = Value> {
    prop_oneof![
        any_expression().prop_map(|e| json!({"filter": {"where": e}})),
        any_expression().prop_map(|e| json!({"map": {"select": e}})),
        any_expression().prop_map(|e| json!({"flatMap": {"select": e}})),
        (0u32..=1000).prop_map(|c| json!({"skip": {"count": c}})),
        (0u32..=1000).prop_map(|c| json!({"take": {"count": c}})),
        proptest::option::of(any_expression()).prop_map(|by| match by {
            Some(by) => json!({"distinct": {"by": by}}),
            None => json!({"distinct": {}}),
        }),
        (1u32..=1000).prop_map(|c| json!({"window": {"count": c}})),
    ]
}

fn decoded(operators: Value) -> OperatorChain {
    let parsed = JsonIrCodec
        .parse(&bytes(&chained(operators)))
        .expect("a valid chain");
    chain_of(&parsed.definition).expect("chain").clone()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Decoding the encoding gives the chain back, and two chains have one digest only when they are the same chain.
    #[test]
    fn the_encoding_round_trips_and_the_digest_tells_chains_apart(
        a in proptest::collection::vec(any_stage(), 1..=8),
        b in proptest::collection::vec(any_stage(), 1..=8),
    ) {
        let (a, b) = (decoded(Value::Array(a)), decoded(Value::Array(b)));
        let again: Value = serde_json::from_slice(&canonical_chain(&a)).expect("JSON");
        prop_assert_eq!(&decoded(again), &a);
        prop_assert_eq!(chain_digest(&a) == chain_digest(&b), a == b);
    }
}

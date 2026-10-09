//! The IR codec against the repository's own example and against hostile documents
//! (T002-002, T002-009; FR-001-002, FR-002-001, NFR-002-SEC).
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use proptest::prelude::*;
use sagat_application::definitions::{IrCodec, IrError, MAX_IR_BYTES, ParsedDefinition};
use sagat_domain::workflow::{DefinitionWarning, NodeId, NodeKind, RetryPolicy, WorkflowError};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("../../../examples/payment-saga.workflow.json");

fn parse(doc: &Value) -> Result<ParsedDefinition, IrError> {
    JsonIrCodec.parse(doc.to_string().as_bytes())
}

fn example() -> Value {
    serde_json::from_str(EXAMPLE).expect("the example is JSON")
}

fn minimal(nodes: Value, entrypoint: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "demo-flow"},
        "entrypoint": entrypoint,
        "nodes": nodes
    })
}

fn activity(id: &str) -> Value {
    json!({"id": id, "type": "activity", "activity": format!("act.{id}")})
}

fn issues_of(doc: &Value) -> Vec<(String, String)> {
    match parse(doc) {
        Err(IrError::Schema(issues)) => issues.into_iter().map(|i| (i.path, i.message)).collect(),
        other => panic!("expected schema issues, got {other:?}"),
    }
}

// ------------------------------------------------------------------ the shipped example

#[test]
fn the_repository_example_is_a_valid_definition() {
    let parsed = JsonIrCodec
        .parse(EXAMPLE.as_bytes())
        .expect("the example parses");
    let def = &parsed.definition;
    assert_eq!(def.name().as_str(), "corporate-payment");
    assert_eq!(def.entrypoint().as_str(), "validate");
    assert_eq!(def.len(), 6);
    let wait = def
        .node(&NodeId::parse("wait_confirmation").expect("id"))
        .expect("node");
    assert!(
        matches!(wait.kind(), NodeKind::WaitEvent { on_timeout: Some(t), timeout, .. }
        if t.as_str() == "manual_review" && *timeout == Duration::from_millis(300_000))
    );
    assert_eq!(wait.next().map(NodeId::as_str), Some("notify"));
}

#[test]
fn saga_fields_in_the_example_are_reported_not_silently_dropped() {
    let parsed = JsonIrCodec.parse(EXAMPLE.as_bytes()).expect("parses");
    let nodes: Vec<&str> = parsed
        .warnings
        .iter()
        .map(|w| match w {
            DefinitionWarning::SagaFieldsNotExecuted { node } => node.as_str(),
            other @ DefinitionWarning::RecursiveChild { .. } => panic!("unexpected: {other}"),
        })
        .collect();
    assert_eq!(nodes, ["reserve", "submit"]);
}

#[test]
fn the_example_digest_is_pinned() {
    // If this changes, every published version's identity changes: that must be a decision.
    // The value was cross-checked against an independent implementation (Python:
    // `json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)` then SHA-256).
    let parsed = JsonIrCodec.parse(EXAMPLE.as_bytes()).expect("parses");
    assert_eq!(
        parsed.digest.to_hex(),
        "9c6523ce4998834d69cdd3c3ffc87eaa8283d14629cbf2238c685f16a81b04c5"
    );
}

#[test]
fn the_idempotency_templates_of_the_example_compile() {
    let parsed = JsonIrCodec.parse(EXAMPLE.as_bytes()).expect("parses");
    let reserve = parsed
        .definition
        .node(&NodeId::parse("reserve").expect("id"))
        .expect("node");
    assert!(
        matches!(reserve.kind(), NodeKind::Activity { idempotency_template: Some(t), .. }
        if t.as_str() == "reserve:${run.id}")
    );
}

// ------------------------------------------------------------------ canonical form and digest

#[test]
fn whitespace_and_key_order_do_not_change_the_digest() {
    let a = JsonIrCodec.parse(EXAMPLE.as_bytes()).expect("a");
    let pretty = serde_json::to_string_pretty(&example()).expect("pretty");
    let b = JsonIrCodec.parse(pretty.as_bytes()).expect("b");
    // Reverse every object's key order.
    fn reverse(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .rev()
                    .map(|(k, v)| (k.clone(), reverse(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(reverse).collect()),
            other => other.clone(),
        }
    }
    let c = JsonIrCodec
        .parse(reverse(&example()).to_string().as_bytes())
        .expect("c");
    assert_eq!(a.digest, b.digest);
    assert_eq!(a.digest, c.digest);
    assert_eq!(a.canonical, b.canonical);
    assert_eq!(a.canonical, c.canonical);
}

#[test]
fn the_canonical_form_is_compact_sorted_and_stable_under_reparsing() {
    let parsed = JsonIrCodec.parse(EXAMPLE.as_bytes()).expect("parses");
    let text = String::from_utf8(parsed.canonical.clone()).expect("utf-8");
    assert!(text.starts_with(r#"{"apiVersion":"sagat-octopus-orchestrator.io/v1","entrypoint":"validate","inputSchema":"#), "{text}");
    assert!(!text.contains(": ") && !text.contains('\n'));
    let again = JsonIrCodec
        .parse(&parsed.canonical)
        .expect("the canonical form is itself valid");
    assert_eq!(again.canonical, parsed.canonical);
    assert_eq!(again.digest, parsed.digest);
}

#[test]
fn any_change_of_meaning_changes_the_digest() {
    let base = JsonIrCodec.parse(EXAMPLE.as_bytes()).expect("base").digest;
    let mut changed = example();
    changed["nodes"][3]["timeoutMs"] = json!(300_001);
    assert_ne!(parse(&changed).expect("changed").digest, base);

    // Array order is significant.
    let mut swapped = example();
    let nodes = swapped["nodes"].as_array_mut().expect("nodes");
    let last = nodes.len() - 1;
    nodes.swap(last - 1, last);
    assert_ne!(parse(&swapped).expect("swapped").digest, base);

    // So is a description, which is part of the document even though the engine ignores it.
    let mut described = example();
    described["metadata"]["description"] = json!("changed");
    assert_ne!(parse(&described).expect("described").digest, base);
}

#[test]
fn strings_with_unusual_characters_survive_canonicalisation() {
    let mut doc = example();
    doc["metadata"]["description"] =
        json!("quote \" backslash \\ tab \t newline \n unicode é世 \u{1F600}");
    let parsed = parse(&doc).expect("parses");
    let reparsed: Value = serde_json::from_slice(&parsed.canonical).expect("canonical is JSON");
    assert_eq!(reparsed, doc);
}

// ------------------------------------------------------------------ schema problems

#[test]
fn schema_problems_are_reported_with_json_pointers() {
    let mut doc = example();
    doc["apiVersion"] = json!("v2");
    doc["nodes"][0]["next"] = json!(7);
    let issues = issues_of(&doc);
    assert!(
        issues.iter().any(|(path, _)| path == "/apiVersion"),
        "{issues:?}"
    );
    assert!(!issues.is_empty());
}

#[test]
fn structural_schema_violations_are_rejected() {
    let cases: Vec<(&str, Value)> = vec![
        ("no nodes", minimal(json!([]), "a")),
        ("unknown top-level field", {
            let mut d = minimal(json!([activity("a")]), "a");
            d["surprise"] = json!(1);
            d
        }),
        ("missing entrypoint", {
            let mut d = minimal(json!([activity("a")]), "a");
            d.as_object_mut().expect("object").remove("entrypoint");
            d
        }),
        ("bad name", {
            let mut d = minimal(json!([activity("a")]), "a");
            d["metadata"]["name"] = json!("Bad Name");
            d
        }),
        (
            "unknown node type",
            minimal(json!([{"id": "a", "type": "teleport"}]), "a"),
        ),
        (
            "activity without a name",
            minimal(json!([{"id": "a", "type": "activity"}]), "a"),
        ),
        (
            "timer of zero",
            minimal(json!([{"id": "a", "type": "timer", "durationMs": 0}]), "a"),
        ),
        (
            "retry of 101 attempts",
            minimal(
                json!([{"id": "a", "type": "activity", "activity": "x", "retry": {"maxAttempts": 101}}]),
                "a",
            ),
        ),
        (
            "parallel with one branch",
            minimal(
                json!([{"id": "a", "type": "parallel", "branches": ["b"], "join": "all"}, activity("b")]),
                "a",
            ),
        ),
        (
            "parallel with a bad join",
            minimal(
                json!([{"id": "a", "type": "parallel", "branches": ["b", "c"], "join": "most"}, activity("b"), activity("c")]),
                "a",
            ),
        ),
    ];
    for (what, doc) in cases {
        assert!(
            matches!(parse(&doc), Err(IrError::Schema(_))),
            "{what}: {:?}",
            parse(&doc)
        );
    }
}

#[test]
fn the_number_and_size_of_reported_issues_is_bounded() {
    let nodes: Vec<Value> = (0..200)
        .map(|i| json!({"id": format!("n{i}"), "type": "teleport"}))
        .collect();
    let issues = issues_of(&minimal(Value::Array(nodes), "n0"));
    assert!(issues.len() <= 20, "{}", issues.len());
    let huge = "x".repeat(5_000);
    let mut doc = minimal(json!([activity("a")]), "a");
    doc["apiVersion"] = json!(huge);
    for (_, message) in issues_of(&doc) {
        assert!(
            message.chars().count() <= 201,
            "{} chars",
            message.chars().count()
        );
    }
}

// ------------------------------------------------------------------ graph problems come from the domain

#[test]
fn graph_violations_are_reported_by_the_domain() {
    let dangling = minimal(
        json!([{"id": "a", "type": "activity", "activity": "x", "next": "ghost"}]),
        "a",
    );
    assert!(matches!(
        parse(&dangling),
        Err(IrError::Invalid(WorkflowError::UnknownTarget { .. }))
    ));

    let cycle = minimal(
        json!([
            {"id": "a", "type": "activity", "activity": "x", "next": "b"},
            {"id": "b", "type": "activity", "activity": "y", "next": "a"}
        ]),
        "a",
    );
    assert!(matches!(
        parse(&cycle),
        Err(IrError::Invalid(WorkflowError::Cycle(_)))
    ));

    let orphan = minimal(json!([activity("a"), activity("orphan")]), "a");
    assert!(matches!(
        parse(&orphan),
        Err(IrError::Invalid(WorkflowError::Unreachable(_)))
    ));

    let duplicate = minimal(json!([activity("a"), activity("a")]), "a");
    assert!(matches!(
        parse(&duplicate),
        Err(IrError::Invalid(WorkflowError::DuplicateNode(_)))
    ));

    let missing_entry = minimal(json!([activity("a")]), "zzz");
    assert!(matches!(
        parse(&missing_entry),
        Err(IrError::Invalid(WorkflowError::UnknownEntrypoint(_)))
    ));

    let escaping_branch = minimal(
        json!([
            {"id": "f", "type": "parallel", "branches": ["l", "r"], "join": "all", "next": "after"},
            {"id": "l", "type": "activity", "activity": "x", "next": "after"},
            activity("r"),
            activity("after")
        ]),
        "f",
    );
    assert!(matches!(
        parse(&escaping_branch),
        Err(IrError::Invalid(WorkflowError::BranchBoundary { .. }))
    ));
}

#[test]
fn a_timer_beyond_a_year_is_rejected() {
    let doc = minimal(
        json!([{"id": "t", "type": "timer", "durationMs": 400u64 * 24 * 3600 * 1000}]),
        "t",
    );
    assert!(matches!(
        parse(&doc),
        Err(IrError::Invalid(WorkflowError::DurationOutOfRange { .. }))
    ));
}

// ------------------------------------------------------------------ what is not executed yet

#[test]
fn node_kinds_without_an_engine_are_refused_with_the_reason() {
    let cases = [
        (
            "saga",
            json!({"id": "c", "type": "saga", "begin": "x"}),
            "feature 003",
        ),
        (
            "script",
            json!({"id": "c", "type": "script", "artifactDigest": "sha256:0123456789abcdef"}),
            "feature 005",
        ),
        (
            "foreach",
            json!({"id": "c", "type": "foreach", "collectionExpr": "input.items", "itemKeyExpr": "item.id", "body": "x", "maxItems": 10}),
            "any planned feature",
        ),
        (
            "humanApproval",
            json!({"id": "c", "type": "humanApproval", "approverPolicy": "p", "timeoutMs": 1000, "formSchema": {}}),
            "any planned feature",
        ),
    ];
    for (kind, node, feature) in cases {
        match parse(&minimal(json!([node]), "c")) {
            Err(IrError::Unsupported {
                node,
                kind: found,
                reason,
            }) => {
                assert_eq!((node.as_str(), found.as_str()), ("c", kind));
                assert!(reason.contains(feature), "{kind}: {reason}");
            }
            other => panic!("{kind}: {other:?}"),
        }
    }
}

// ------------------------------------------------------------------ child workflows

fn child_document(node: serde_json::Value) -> serde_json::Value {
    minimal(json!([node, activity("after")]), "c")
}

#[test]
fn a_child_node_carries_its_workflow_version_input_and_cancel_policy() {
    use sagat_domain::workflow::{CancelPolicy, NodeKind};
    let doc = child_document(json!({
        "id": "c", "type": "child", "workflowId": "payment-flow", "version": 3,
        "input": "input.order", "cancelPolicy": "request_cancel", "next": "after",
        "onError": "after", "timeoutMs": 60000
    }));
    let parsed = parse(&doc).expect("a child node is executable now");
    let node = parsed
        .definition
        .node(&NodeId::parse("c").expect("id"))
        .expect("node");
    let NodeKind::Child {
        workflow,
        version,
        input,
        cancel_policy,
    } = node.kind()
    else {
        panic!("expected a child node, got {:?}", node.kind());
    };
    assert_eq!(workflow.as_str(), "payment-flow");
    assert_eq!(*version, Some(3));
    assert_eq!(input.as_ref().map(|e| e.as_str()), Some("input.order"));
    assert_eq!(*cancel_policy, CancelPolicy::RequestCancel);
    assert_eq!(node.timeout(), Some(std::time::Duration::from_secs(60)));
}

#[test]
fn a_child_node_defaults_to_the_active_version_the_parents_input_and_propagation() {
    use sagat_domain::workflow::{CancelPolicy, NodeKind};
    let doc = child_document(json!({
        "id": "c", "type": "child", "workflowId": "payment-flow", "next": "after"
    }));
    let parsed = parse(&doc).expect("valid");
    let node = parsed
        .definition
        .node(&NodeId::parse("c").expect("id"))
        .expect("node");
    assert!(matches!(
        node.kind(),
        NodeKind::Child {
            version: None,
            input: None,
            cancel_policy: CancelPolicy::Propagate,
            ..
        }
    ));
}

#[test]
fn a_malformed_child_node_is_reported_at_the_offending_field() {
    let cases = [
        (json!({"id": "c", "type": "child"}), "/nodes/0"),
        (
            json!({"id": "c", "type": "child", "workflowId": "Not A Slug", "next": "after"}),
            "/nodes/0/workflowId",
        ),
        (
            json!({"id": "c", "type": "child", "workflowId": "ok-flow", "version": 0, "next": "after"}),
            // The schema's `oneOf` reports a node it rejects at the node itself.
            "/nodes/0",
        ),
        (
            json!({"id": "c", "type": "child", "workflowId": "ok-flow", "cancelPolicy": "kill", "next": "after"}),
            "/nodes/0",
        ),
        (
            json!({"id": "c", "type": "child", "workflowId": "ok-flow", "input": "input.((", "next": "after"}),
            "/nodes/0/input",
        ),
    ];
    for (node, at) in cases {
        match parse(&child_document(node.clone())) {
            Err(IrError::Schema(issues)) => {
                assert!(
                    issues.iter().any(|i| i.path.starts_with(at)),
                    "{node}: {issues:?}"
                );
            }
            Err(IrError::Expression { field, .. }) => {
                assert!(at.ends_with("/input") && field == "input", "{node}");
            }
            other => panic!("{node}: {other:?}"),
        }
    }
}

// ------------------------------------------------------------------ expressions

#[test]
fn expressions_must_compile_in_the_bounded_subset() {
    let condition = |expr: &str| {
        minimal(
            json!([
                {"id": "c", "type": "condition", "expression": expr, "ifTrue": "a", "ifFalse": "b"},
                activity("a"),
                activity("b")
            ]),
            "c",
        )
    };
    assert!(parse(&condition("input.amount > 100 && has(input.country)")).is_ok());
    for (bad, field) in [
        ("input.amount >", "expression"),
        ("input.items.all(i, i > 0)", "expression"),
        ("input.name.matches('x.*')", "expression"),
        ("", "expression"),
    ] {
        match parse(&condition(bad)) {
            Err(IrError::Expression { node, field: f, .. }) => {
                assert_eq!((node.as_str(), f), ("c", field), "{bad}")
            }
            Err(IrError::Schema(_) | IrError::Invalid(_)) if bad.is_empty() => {}
            other => panic!("{bad:?}: {other:?}"),
        }
    }
}

#[test]
fn correlation_end_output_and_idempotency_templates_are_checked_too() {
    let wait = |expr: &str| {
        minimal(
            json!([{"id": "w", "type": "waitEvent", "eventType": "e", "correlationExpr": expr, "timeoutMs": 1000}]),
            "w",
        )
    };
    assert!(parse(&wait("input.paymentId")).is_ok());
    assert!(matches!(
        parse(&wait("input.")),
        Err(IrError::Expression {
            field: "correlationExpr",
            ..
        })
    ));

    let end = |expr: &str| {
        minimal(
            json!([activity("a"), {"id": "e", "type": "end", "outputExpr": expr}]),
            "a",
        )
    };
    let mut with_end = end("steps.a.output");
    with_end["nodes"][0]["next"] = json!("e");
    assert!(parse(&with_end).is_ok());
    let mut bad_end = end("steps.a.output +");
    bad_end["nodes"][0]["next"] = json!("e");
    assert!(matches!(
        parse(&bad_end),
        Err(IrError::Expression {
            field: "outputExpr",
            ..
        })
    ));

    let template = |t: &str| {
        minimal(
            json!([{"id": "a", "type": "activity", "activity": "x", "idempotencyTemplate": t}]),
            "a",
        )
    };
    assert!(parse(&template("pay:${input.id}:${run.id}")).is_ok());
    assert!(matches!(
        parse(&template("pay:${input.id")),
        Err(IrError::Expression {
            field: "idempotencyTemplate",
            ..
        })
    ));
    assert!(matches!(
        parse(&template("pay:${input.all(x, x)}")),
        Err(IrError::Expression { .. })
    ));
}

#[test]
fn a_hostile_expression_is_refused_not_evaluated() {
    let nested = format!("{}1{}", "(".repeat(300), ")".repeat(300));
    let doc = minimal(
        json!([
            {"id": "c", "type": "condition", "expression": nested, "ifTrue": "a", "ifFalse": "b"},
            activity("a"),
            activity("b")
        ]),
        "c",
    );
    assert!(matches!(parse(&doc), Err(IrError::Expression { .. })));
}

// ------------------------------------------------------------------ retry policy mapping

fn retry_of(spec: Value) -> Result<RetryPolicy, IrError> {
    let doc = minimal(
        json!([{"id": "a", "type": "activity", "activity": "x", "retry": spec}]),
        "a",
    );
    parse(&doc).map(|p| {
        p.definition
            .node(&NodeId::parse("a").expect("id"))
            .expect("node")
            .retry_policy()
    })
}

#[test]
fn omitted_retry_fields_take_the_documented_defaults() {
    let default = RetryPolicy::DEFAULT;
    let none = parse(&minimal(json!([activity("a")]), "a")).expect("parses");
    assert_eq!(
        none.definition
            .node(&NodeId::parse("a").expect("id"))
            .expect("n")
            .retry_policy(),
        default
    );

    let partial = retry_of(json!({"maxAttempts": 5})).expect("partial");
    assert_eq!(partial.max_attempts(), 5);
    assert_eq!(partial.initial_delay(), default.initial_delay());
    assert_eq!(partial.max_delay(), default.max_delay());
    assert!(partial.jitter());

    let slow = retry_of(json!({"initialDelayMs": 120_000})).expect("slow start");
    assert_eq!(
        slow.max_delay(),
        Duration::from_secs(120),
        "the ceiling never sits below the start"
    );

    let explicit = retry_of(json!({"maxAttempts": 1, "initialDelayMs": 10, "maxDelayMs": 20, "strategy": "fixed", "jitter": false})).expect("explicit");
    assert_eq!((explicit.max_attempts(), explicit.jitter()), (1, false));
}

#[test]
fn an_inconsistent_retry_policy_is_rejected_by_the_domain() {
    let result = retry_of(json!({"initialDelayMs": 1000, "maxDelayMs": 10}));
    assert!(
        matches!(
            result,
            Err(IrError::Invalid(WorkflowError::InvalidRetry(_)))
        ),
        "{result:?}"
    );
}

// ------------------------------------------------------------------ hostile input

#[test]
fn oversized_malformed_and_deeply_nested_documents_are_refused_safely() {
    assert!(matches!(
        JsonIrCodec.parse(&vec![b' '; MAX_IR_BYTES + 1]),
        Err(IrError::TooLarge)
    ));
    for bad in [
        &b""[..],
        b"not json",
        b"{",
        b"[1,2",
        b"\xff\xfe",
        b"null",
        b"[]",
        b"1",
    ] {
        let result = JsonIrCodec.parse(bad);
        assert!(
            matches!(result, Err(IrError::Malformed | IrError::Schema(_))),
            "{bad:?}: {result:?}"
        );
    }
    let bomb = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(matches!(
        JsonIrCodec.parse(bomb.as_bytes()),
        Err(IrError::Malformed)
    ));
    let object_bomb = format!("{}1{}", r#"{"a":"#.repeat(50_000), "}".repeat(50_000));
    assert!(matches!(
        JsonIrCodec.parse(object_bomb.as_bytes()),
        Err(IrError::Malformed)
    ));
}

#[test]
fn a_wide_but_valid_document_is_accepted() {
    let nodes: Vec<Value> = (0..2_000)
        .map(|i| {
            let mut n = activity(&format!("n{i}"));
            if i + 1 < 2_000 {
                n["next"] = json!(format!("n{}", i + 1));
            }
            n
        })
        .collect();
    let parsed = parse(&minimal(Value::Array(nodes), "n0")).expect("a long chain is valid");
    assert_eq!(parsed.definition.len(), 2_000);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        let _ = JsonIrCodec.parse(&bytes);
    }

    #[test]
    fn a_corrupted_example_never_panics_and_never_yields_a_different_valid_digest_silently(
        flips in proptest::collection::vec((0usize..2_000, any::<u8>()), 1..6),
    ) {
        let original = JsonIrCodec.parse(EXAMPLE.as_bytes()).expect("parses");
        let mut bytes = EXAMPLE.as_bytes().to_vec();
        for (at, byte) in flips {
            let len = bytes.len();
            bytes[at % len] = byte;
        }
        if let Ok(parsed) = JsonIrCodec.parse(&bytes) {
            // A mutated but still valid document either is the same document (digest equal) or a
            // different one (digest different): the digest must follow the canonical bytes.
            prop_assert_eq!(parsed.digest == original.digest, parsed.canonical == original.canonical);
        }
    }
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
}

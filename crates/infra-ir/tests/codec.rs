//! The IR codec against the repository's own example and hostile documents.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use proptest::prelude::*;
use sagat_application::definitions::{IrCodec, IrError, MAX_IR_BYTES, ParsedDefinition};
use sagat_domain::execution::{ExecutionInstanceId, InstancePath};
use sagat_domain::saga::{OnCompensationFailure, SagaCancelPolicy};
use sagat_domain::workflow::{NodeId, NodeKind, RetryPolicy, WorkflowError};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

const EXAMPLE: &str = include_str!("../../../examples/payment-saga.workflow.json");
const LOOP_BUILDERS_GOLDEN: &str =
    include_str!("../../../tests/contract/sdk-conformance/expected/loop-builders.workflows.json");

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

#[test]
fn unsupported_additive_declarations_remain_unpublishable() {
    let mut document = minimal(json!([{"id": "done", "type": "end"}]), "done");
    // The block is publishable (D297); `crates/infra-ir/tests/limits.rs` covers it.
    document["limits"] = json!({"maxInstances": 100, "maxDepth": 2});
    parse(&document).expect("lowered limits are publishable");

    document.as_object_mut().expect("document").remove("limits");
    document["state"] = json!({"schema": {"type": "integer"}, "initial": "0"});
    parse(&document).expect("basic declared state is supported");
    document.as_object_mut().expect("document").remove("state");
    document["chainPolicy"] = json!({"maxSegments": 2, "timeoutMs": 1_000});
    assert!(matches!(
        parse(&document),
        Err(IrError::UnsupportedDeclaration {
            field: "chainPolicy",
            ..
        })
    ));

    let mut collection = minimal(
        json!([
            {"id": "lines", "type": "forEach", "items": "input.items", "body": "line",
             "result": "bodyResult", "maxItems": 17, "maxConcurrency": 3, "next": "done"},
            {"id": "done", "type": "end"}
        ]),
        "lines",
    );
    collection["bodies"] = json!({"line": {"entrypoint": "return", "nodes": [
        {"id": "return", "type": "return", "result": "input.item"}
    ]}});
    assert!(matches!(
        parse(&collection),
        Err(IrError::Unsupported { kind, .. }) if kind == "body"
    ));
    collection
        .as_object_mut()
        .expect("document")
        .remove("bodies");
    assert!(matches!(
        parse(&collection),
        Err(IrError::Invalid(WorkflowError::UnknownBody { .. }))
    ));
}

#[test]
fn native_approval_is_publishable_beside_declared_state_and_apart_from_parallel() {
    let document = minimal(
        json!([
            {
                "id": "review",
                "type": "approval",
                "policy": {"roles": ["operator"], "excludeInitiator": true},
                "timeoutMs": 60_000,
                "onApprove": "approved",
                "onReject": "rejected",
                "onExpire": "expired"
            },
            {"id": "approved", "type": "end"},
            {"id": "rejected", "type": "end"},
            {"id": "expired", "type": "end"}
        ]),
        "review",
    );
    let parsed = parse(&document).expect("native approval is publishable");
    assert!(matches!(
        parsed
            .definition
            .node(&NodeId::parse("review").expect("approval node"))
            .map(|node| node.kind()),
        Some(NodeKind::Approval { .. })
    ));

    let mut combined = document.clone();
    combined["state"] = json!({"schema": {"type": "integer"}, "initial": "0"});
    parse(&combined).expect("an approval and declared state publish together (D295)");

    let mut parallel = document.clone();
    parallel["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(json!({"id": "fork", "type": "parallel", "branches": ["approved", "rejected"], "join": "all", "next": "expired"}));
    assert!(matches!(
        parse(&parallel),
        Err(IrError::Unsupported { kind, .. }) if kind == "parallel"
    ));
}

#[test]
fn structured_collection_can_be_validated_canonicalized_and_published() {
    let mut document = minimal(
        json!([
            {"id": "lines", "type": "forEach", "items": "input.items", "body": "line",
             "result": "bodyResult", "maxItems": 17, "maxConcurrency": 3, "next": "done"},
            {"id": "done", "type": "end", "outputExpr": "steps.lines.output"}
        ]),
        "lines",
    );
    document["limits"] = json!({"maxInstances": 200, "maxDepth": 2});
    document["bodies"] = json!({"line": {"entrypoint": "work", "nodes": [
        {"id": "work", "type": "activity", "activity": "line.process", "next": "return"},
        {"id": "return", "type": "return", "result": "steps.work.output"}
    ]}});

    let parsed = JsonIrCodec
        .parse_structured(document.to_string().as_bytes())
        .expect("structured validation succeeds");
    assert!(parsed.definition.is_structured());
    assert_eq!(parsed.definition.limits().max_instances(), 200);
    assert_eq!(parsed.definition.limits().max_depth(), 2);
    let lines = parsed
        .definition
        .node(&NodeId::parse("lines").expect("node id"))
        .expect("collection node");
    assert!(matches!(
        lines.kind(),
        NodeKind::ForEach {
            max_items: 17,
            max_concurrency: 3,
            ..
        }
    ));
    let body = parsed
        .definition
        .body(&NodeId::parse("line").expect("body id"))
        .expect("named body");
    assert_eq!(body.entrypoint().as_str(), "work");
    assert!(matches!(
        body.node(&NodeId::parse("return").expect("return id"))
            .expect("return node")
            .kind(),
        NodeKind::Return { .. }
    ));

    let canonical: Value = serde_json::from_slice(&parsed.canonical).expect("canonical JSON");
    assert_eq!(canonical["apiVersion"], "sagat-octopus-orchestrator.io/v1");
    let reparsed = JsonIrCodec
        .parse_structured(&parsed.canonical)
        .expect("canonical structured document reparses");
    assert_eq!(reparsed.canonical, parsed.canonical);
    assert_eq!(reparsed.digest, parsed.digest);

    let published = JsonIrCodec
        .parse(document.to_string().as_bytes())
        .expect("proven collection is publishable");
    assert_eq!(published.canonical, parsed.canonical);
    assert_eq!(published.digest, parsed.digest);
}

#[test]
fn structured_validation_compiles_collection_and_body_expressions() {
    let mut document = minimal(
        json!([
            {"id": "lines", "type": "forEach", "items": "input.items[", "body": "line",
             "result": "bodyResult", "maxItems": 17, "maxConcurrency": 3, "next": "done"},
            {"id": "done", "type": "end"}
        ]),
        "lines",
    );
    document["bodies"] = json!({"line": {"entrypoint": "return", "nodes": [
        {"id": "return", "type": "return", "result": "input.item"}
    ]}});

    assert!(matches!(
        JsonIrCodec.parse_structured(document.to_string().as_bytes()),
        Err(IrError::Expression { field: "items", .. })
    ));

    document["nodes"][0]["items"] = json!("input.items");
    document["bodies"]["line"]["nodes"][0]["result"] = json!("input.item[");
    assert!(matches!(
        JsonIrCodec.parse_structured(document.to_string().as_bytes()),
        Err(IrError::Expression {
            node,
            field: "result",
            ..
        }) if node == "return"
    ));
}

#[test]
fn structured_validation_enforces_closed_bodies_and_capability_boundaries() {
    let mut document = minimal(
        json!([
            {"id": "lines", "type": "forEach", "items": "input.items", "body": "line",
             "result": "bodyResult", "maxItems": 17, "maxConcurrency": 3, "next": "done"},
            {"id": "done", "type": "end"}
        ]),
        "lines",
    );
    document["bodies"] = json!({"line": {"entrypoint": "work", "nodes": [
        {"id": "work", "type": "activity", "activity": "line.process", "next": "done"},
        {"id": "return", "type": "return", "result": "steps.work.output"}
    ]}});

    assert!(matches!(
        JsonIrCodec.parse_structured(document.to_string().as_bytes()),
        Err(IrError::EscapingBodyEdge { body, from, to })
            if body.as_str() == "line" && from.as_str() == "work" && to.as_str() == "done"
    ));

    document["bodies"]["line"]["nodes"] = json!([
        {"id": "nested", "type": "forEach", "items": "input.items", "body": "line",
         "result": "bodyResult", "maxItems": 2, "maxConcurrency": 1, "next": "return"},
        {"id": "return", "type": "return", "result": "steps.nested.output"}
    ]);
    document["bodies"]["line"]["entrypoint"] = json!("nested");
    assert!(matches!(
        JsonIrCodec.parse_structured(document.to_string().as_bytes()),
        Err(IrError::Invalid(WorkflowError::RecursiveBody(body))) if body.as_str() == "line"
    ));

    document["nodes"][0] = json!({
        "id": "lines", "type": "while", "condition": "true", "body": "line",
        "initial": "0", "carry": "carry + 1", "maxIterations": 2,
        "timeoutMs": 1_000, "next": "done"
    });
    assert!(matches!(
        JsonIrCodec.parse_structured(document.to_string().as_bytes()),
        Err(IrError::Invalid(WorkflowError::RecursiveBody(body))) if body.as_str() == "line"
    ));
}

#[test]
fn structured_loops_match_the_reviewed_sdk_golden_and_publish() {
    let golden: Value = serde_json::from_str(LOOP_BUILDERS_GOLDEN).expect("loop golden is JSON");
    let document = &golden["workflows"]["pre-and-post-test"];
    let expected = golden["canonical"]["pre-and-post-test"]
        .as_str()
        .expect("canonical loop document");

    let parsed = JsonIrCodec
        .parse_structured(document.to_string().as_bytes())
        .expect("structured loops validate");
    assert_eq!(parsed.canonical, expected.as_bytes());
    let pre = parsed
        .definition
        .node(&NodeId::parse("pre").expect("node id"))
        .expect("while node");
    assert!(matches!(
        pre.kind(),
        NodeKind::While {
            condition,
            body,
            initial,
            carry,
            max_iterations: 3,
            delay: Some(delay),
        } if condition.as_str() == "carry < 3"
            && body.as_str() == "iteration"
            && initial.as_str() == "input.start"
            && carry.as_str() == "bodyResult"
            && *delay == Duration::from_millis(250)
    ));
    assert_eq!(pre.timeout(), Some(Duration::from_millis(60_000)));

    let post = parsed
        .definition
        .node(&NodeId::parse("post").expect("node id"))
        .expect("repeatUntil node");
    assert!(matches!(
        post.kind(),
        NodeKind::RepeatUntil {
            max_iterations: 10_000,
            delay: None,
            ..
        }
    ));
    assert_eq!(post.timeout(), Some(Duration::from_millis(31_622_400_000)));

    let pre_path = InstancePath::root()
        .child(NodeId::parse("pre").expect("scope id"), 2)
        .expect("bounded loop path");
    let iteration = ExecutionInstanceId::in_path(
        pre_path.clone(),
        NodeId::parse("increment").expect("body node id"),
    );
    assert!(matches!(
        parsed
            .definition
            .node_for_instance(&iteration)
            .map(|node| node.kind()),
        Some(NodeKind::Activity { .. })
    ));
    let outside_budget = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("pre").expect("scope id"), 3)
            .expect("bounded loop path"),
        NodeId::parse("increment").expect("body node id"),
    );
    assert!(
        parsed
            .definition
            .node_for_instance(&outside_budget)
            .is_none()
    );

    let reparsed = JsonIrCodec
        .parse_structured(&parsed.canonical)
        .expect("canonical loops reparse");
    assert_eq!(reparsed.canonical, parsed.canonical);
    assert_eq!(reparsed.digest, parsed.digest);
    let published = JsonIrCodec
        .parse(document.to_string().as_bytes())
        .expect("restricted loop golden is publishable");
    assert_eq!(published.digest, parsed.digest);
}

#[test]
fn structured_loops_compile_all_expressions_and_validate_their_body() {
    let mut document = minimal(
        json!([
            {"id": "poll", "type": "while", "condition": "carry < 3", "body": "iteration",
             "initial": "input.start", "carry": "bodyResult", "maxIterations": 3,
             "timeoutMs": 60_000, "next": "done"},
            {"id": "done", "type": "end"}
        ]),
        "poll",
    );
    document["bodies"] = json!({"iteration": {"entrypoint": "returned", "nodes": [
        {"id": "returned", "type": "return", "result": "carry"}
    ]}});

    for (field, invalid) in [
        ("condition", "carry <"),
        ("initial", "input["),
        ("carry", "bodyResult["),
    ] {
        let mut invalid_document = document.clone();
        invalid_document["nodes"][0][field] = json!(invalid);
        assert!(matches!(
            JsonIrCodec.parse_structured(invalid_document.to_string().as_bytes()),
            Err(IrError::Expression {
                node,
                field: actual,
                ..
            }) if node == "poll" && actual == field
        ));
    }

    document["nodes"][0]["body"] = json!("missing");
    assert!(matches!(
        JsonIrCodec.parse_structured(document.to_string().as_bytes()),
        Err(IrError::Invalid(WorkflowError::UnknownBody { node, body }))
            if node.as_str() == "poll" && body.as_str() == "missing"
    ));

    document["nodes"][0]["body"] = json!("iteration");
    document["bodies"]["iteration"]["nodes"] = json!([
        {"id": "again", "type": "repeatUntil", "condition": "carry >= 3",
         "body": "iteration", "initial": "carry", "carry": "bodyResult",
         "maxIterations": 3, "timeoutMs": 60_000, "next": "returned"},
        {"id": "returned", "type": "return", "result": "carry"}
    ]);
    document["bodies"]["iteration"]["entrypoint"] = json!("again");
    assert!(matches!(
        JsonIrCodec.parse_structured(document.to_string().as_bytes()),
        Err(IrError::Invalid(WorkflowError::RecursiveBody(body)))
            if body.as_str() == "iteration"
    ));
}

#[test]
fn structured_loop_limits_and_body_boundaries_fail_before_runtime() {
    let base = || {
        let mut document = minimal(
            json!([
                {"id": "poll", "type": "repeatUntil", "condition": "carry >= 3",
                 "body": "iteration", "initial": "0", "carry": "bodyResult",
                 "maxIterations": 3, "timeoutMs": 60_000, "next": "done"},
                {"id": "done", "type": "end"}
            ]),
            "poll",
        );
        document["bodies"] = json!({"iteration": {"entrypoint": "returned", "nodes": [
            {"id": "returned", "type": "return", "result": "carry"}
        ]}});
        document
    };

    for (field, value) in [
        ("maxIterations", json!(0)),
        ("maxIterations", json!(10_001)),
        ("timeoutMs", json!(0)),
        ("timeoutMs", json!(31_622_400_001_u64)),
        ("delayMs", json!(0)),
        ("delayMs", json!(31_622_400_001_u64)),
    ] {
        let mut document = base();
        document["nodes"][0][field] = value;
        let issues = match JsonIrCodec.parse_structured(document.to_string().as_bytes()) {
            Err(IrError::Schema(issues)) => issues,
            other => panic!("expected {field} schema issue, got {other:?}"),
        };
        assert!(
            issues.iter().any(|issue| issue.path.ends_with(field)),
            "{issues:?}"
        );
    }

    let mut terminal = base();
    terminal["nodes"][0]
        .as_object_mut()
        .expect("loop node")
        .remove("next");
    assert!(matches!(
        JsonIrCodec.parse_structured(terminal.to_string().as_bytes()),
        Err(IrError::Schema(_))
    ));

    let mut body_end = base();
    body_end["bodies"]["iteration"]["nodes"] = json!([
        {"id": "finished", "type": "end"},
        {"id": "returned", "type": "return", "result": "carry"}
    ]);
    body_end["bodies"]["iteration"]["entrypoint"] = json!("finished");
    assert!(matches!(
        JsonIrCodec.parse_structured(body_end.to_string().as_bytes()),
        Err(IrError::Schema(_))
    ));

    let mut body_continuation = base();
    body_continuation["bodies"]["iteration"]["nodes"] = json!([
        {"id": "continued", "type": "continueAsNew", "input": "carry"},
        {"id": "returned", "type": "return", "result": "carry"}
    ]);
    body_continuation["bodies"]["iteration"]["entrypoint"] = json!("continued");
    assert!(matches!(
        JsonIrCodec.parse_structured(body_continuation.to_string().as_bytes()),
        Err(IrError::Schema(_))
    ));

    let mut oversized_expression = base();
    oversized_expression["nodes"][0]["condition"] = json!("é".repeat(513));
    assert!(matches!(
        JsonIrCodec.parse_structured(oversized_expression.to_string().as_bytes()),
        Err(IrError::Invalid(WorkflowError::ExpressionByteLimit { node, limit: 1024 }))
            if node.as_str() == "poll"
    ));
}

#[test]
fn structured_schema_errors_identify_the_offending_fields() {
    let mut document = minimal(
        json!([
            {"id": "lines", "type": "forEach", "items": "input.items", "body": "line",
             "result": "bodyResult", "maxItems": 17, "next": "done"},
            {"id": "done", "type": "end"}
        ]),
        "lines",
    );
    document["bodies"] = json!({"line": {"entrypoint": "work", "nodes": [
        {"id": "work", "type": "activity", "next": "return"},
        {"id": "return", "type": "return", "result": "steps.work.output"}
    ]}});

    let issues = match JsonIrCodec.parse_structured(document.to_string().as_bytes()) {
        Err(IrError::Schema(issues)) => issues,
        other => panic!("expected schema errors, got {other:?}"),
    };
    assert!(
        issues
            .iter()
            .any(|issue| { issue.path == "/nodes/0" && issue.message.contains("maxConcurrency") }),
        "{issues:?}"
    );
    assert!(
        issues.iter().any(|issue| {
            issue.path == "/bodies/line/nodes/0" && issue.message.contains("activity")
        }),
        "{issues:?}"
    );
}

#[test]
fn switch_cases_keep_declaration_order_and_reject_an_undefined_target() {
    let mut document = minimal(
        json!([
            {"id": "choose", "type": "switch", "cases": [
                {"id": "first", "when": "input.amount > 0", "target": "accept"},
                {"id": "second", "when": "input.amount >= 0", "target": "review"}
            ], "default": "reject"},
            {"id": "accept", "type": "end"},
            {"id": "review", "type": "end"},
            {"id": "reject", "type": "end"}
        ]),
        "choose",
    );
    let parsed = parse(&document).expect("switch parses");
    let node = parsed
        .definition
        .node(&NodeId::parse("choose").unwrap())
        .unwrap();
    let NodeKind::Switch { cases, default } = node.kind() else {
        panic!("switch kind");
    };
    assert_eq!(
        cases
            .iter()
            .map(|case| case.id().as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(default.as_str(), "reject");

    document["nodes"][0]["cases"][1]["target"] = json!("missing");
    assert!(matches!(
        parse(&document),
        Err(IrError::Invalid(WorkflowError::UnknownTarget { .. }))
    ));
    document["nodes"][0]["cases"][1]["target"] = json!("review");
    document["nodes"][0]["cases"][1]["when"] = json!("input.amount >");
    assert!(matches!(
        parse(&document),
        Err(IrError::Expression { field: "when", .. })
    ));
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
    assert_eq!(
        def.len(),
        7,
        "validate, the saga, its two steps, the wait and two more"
    );
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
fn the_example_saga_compensates_its_steps_and_has_nothing_to_warn_about() {
    let parsed = JsonIrCodec.parse(EXAMPLE.as_bytes()).expect("parses");
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let def = &parsed.definition;
    let settlement = def
        .node(&NodeId::parse("settlement").expect("id"))
        .expect("the saga");
    assert!(matches!(
        settlement.kind(),
        NodeKind::Saga {
            on_compensation_failure: OnCompensationFailure::ManualReview,
            cancel_policy: SagaCancelPolicy::CompensateCompleted,
            ..
        }
    ));
    // Each compensable step has its compensation and the lost-response one its reconciliation.
    for derived in ["reserve~c", "submit~c", "submit~r"] {
        assert!(
            def.node(&NodeId::parse_stored(derived).expect("id"))
                .is_some(),
            "{derived}"
        );
    }
    let submit = def
        .node(&NodeId::parse("submit").expect("id"))
        .expect("submit");
    assert!(submit.effect().high_risk);
    let reserve = def
        .node(&NodeId::parse("reserve").expect("id"))
        .expect("reserve");
    assert_eq!(
        reserve
            .effect()
            .compensation_retry
            .map(|r| r.max_attempts()),
        Some(5)
    );
}

#[test]
fn the_example_digest_is_pinned() {
    // If this changes, every published version's identity changes: that must be a decision.
    // The value was cross-checked against an independent implementation (Python:
    // `json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)` then SHA-256).
    let parsed = JsonIrCodec.parse(EXAMPLE.as_bytes()).expect("parses");
    assert_eq!(
        parsed.digest.to_hex(),
        "e757aa04ac3fa293f29bb2aef04063f449974fef5dc3c710d0d99717213e0c35"
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
            "foreach",
            json!({"id": "c", "type": "foreach", "collectionExpr": "input.items", "itemKeyExpr": "item.id", "body": "x", "maxItems": 10}),
            "legacy collection placeholder",
        ),
        (
            "humanApproval",
            json!({"id": "c", "type": "humanApproval", "approverPolicy": "p", "timeoutMs": 1000, "formSchema": {}}),
            "legacy approval placeholder",
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

// ------------------------------------------------------------------ script nodes

const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn a_script_node_runs_as_the_activity_named_after_its_digest() {
    let doc = minimal(
        json!([{
            "id": "c", "type": "script", "artifactDigest": DIGEST, "language": "typescript",
            "secrets": ["payments-api-key"], "compensateWith": "undo.charge",
            "retry": {"maxAttempts": 2}, "timeoutMs": 5000, "next": "after"
        }, activity("after")]),
        "c",
    );
    let parsed = parse(&doc).expect("a script node is executable now");
    let node = parsed
        .definition
        .node(&NodeId::parse("c").expect("id"))
        .expect("node");
    let NodeKind::Activity {
        activity,
        retry,
        compensate_with,
        secrets,
        ..
    } = node.kind()
    else {
        panic!("a script node is an activity");
    };
    assert_eq!(
        activity.as_str(),
        format!("script.sha256-{}", &DIGEST["sha256:".len()..])
    );
    assert_eq!(retry.max_attempts(), 2);
    assert_eq!(
        compensate_with.as_ref().map(|a| a.as_str()),
        Some("undo.charge")
    );
    assert_eq!(
        secrets.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec!["payments-api-key"]
    );
    // What the definition pins, for the check a publication makes.
    let pinned: Vec<_> = parsed.definition.script_digests().into_iter().collect();
    assert_eq!(pinned.len(), 1);
    assert_eq!(pinned[0].as_str(), DIGEST);
}

#[test]
fn a_script_node_pins_a_digest_and_nothing_else() {
    for bad in [
        json!({"id": "c", "type": "script"}),
        json!({"id": "c", "type": "script", "artifactDigest": "sha256:0123456789abcdef"}),
        json!({"id": "c", "type": "script", "artifactDigest": DIGEST.to_uppercase()}),
        json!({"id": "c", "type": "script", "artifactDigest": "latest"}),
        json!({"id": "c", "type": "script", "artifactDigest": DIGEST, "language": "cobol"}),
        json!({"id": "c", "type": "script", "artifactDigest": DIGEST, "secrets": ["Not A Name"]}),
    ] {
        assert!(
            matches!(
                parse(&minimal(json!([bad.clone()]), "c")),
                Err(IrError::Schema(_))
            ),
            "{bad}"
        );
    }
    // An ordinary activity that happens to be called like one is not a pinned script.
    let doc = minimal(
        json!([{"id": "c", "type": "activity", "activity": "script.run"}]),
        "c",
    );
    let parsed = parse(&doc).expect("an activity");
    assert!(parsed.definition.script_digests().is_empty());
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

// ------------------------------------------------------------------ declared secrets (FR-009-002)

fn with_secrets(secrets: Value) -> Value {
    let mut a = activity("a");
    a["secrets"] = secrets;
    minimal(json!([a]), "a")
}

#[test]
fn an_activity_declares_secrets_by_name_and_the_definition_lists_them() {
    let parsed = parse(&with_secrets(json!(["stripe-key", "smtp.password"]))).expect("parses");
    let node = parsed
        .definition
        .node(&NodeId::parse("a").expect("id"))
        .expect("node");
    let names: Vec<_> = node.secrets().iter().map(|s| s.as_str()).collect();
    assert_eq!(names, ["stripe-key", "smtp.password"]);
    let all: Vec<_> = parsed
        .definition
        .declared_secrets()
        .into_iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    assert_eq!(
        all,
        ["smtp.password", "stripe-key"],
        "once each, in name order"
    );

    // No declaration is no secret, and the digest of a document without the field is what it was.
    let plain = parse(&minimal(json!([activity("a")]), "a")).expect("parses");
    assert!(plain.definition.declared_secrets().is_empty());
    assert_ne!(
        plain.digest, parsed.digest,
        "the declaration is part of the pinned version"
    );
}

#[test]
fn a_secret_declaration_that_is_not_a_list_of_names_is_refused_with_its_path() {
    for (what, value) in [
        ("not an array", json!("stripe-key")),
        ("a number", json!([5])),
        ("a value written where a name belongs", json!(["has space"])),
        ("upper case", json!(["Stripe"])),
        ("starting with a digit", json!(["1stripe"])),
        ("an object", json!([{"name": "stripe"}])),
        ("a repeat", json!(["stripe", "stripe"])),
        (
            "too many",
            json!((0..17).map(|i| format!("s{i}")).collect::<Vec<_>>()),
        ),
        ("too long", json!(["a".repeat(65)])),
    ] {
        let doc = with_secrets(value);
        let result = parse(&doc);
        assert!(
            matches!(result, Err(IrError::Schema(_))),
            "{what}: {result:?}"
        );
    }
    let issues = issues_of(&with_secrets(json!(["ok", "NOT OK"])));
    assert!(!issues.is_empty(), "the refusal says where to look");
}

#[test]
fn only_an_activity_can_declare_secrets() {
    // A timer with a `secrets` field is just an unknown field of that node, ignored like any other
    // extra field; nothing is declared by it.
    let doc = minimal(
        json!([{"id": "t", "type": "timer", "durationMs": 1000, "secrets": ["stripe-key"]}]),
        "t",
    );
    let parsed = parse(&doc).expect("parses");
    assert!(parsed.definition.declared_secrets().is_empty());
}

// ------------------------------------------------------------------ sagas (feature 003)

fn saga_document(saga: Value, steps: Vec<Value>) -> Value {
    let mut nodes = vec![saga];
    nodes.extend(steps);
    minimal(Value::Array(nodes), "s")
}

#[test]
fn a_saga_node_defaults_to_waiting_for_an_operator_and_compensating_on_cancel() {
    let doc = saga_document(
        json!({"id": "s", "type": "saga", "begin": "a"}),
        vec![activity("a")],
    );
    let parsed = parse(&doc).expect("parses");
    let saga = parsed
        .definition
        .node(&NodeId::parse("s").expect("id"))
        .expect("saga");
    assert!(matches!(
        saga.kind(),
        NodeKind::Saga {
            on_compensation_failure: OnCompensationFailure::ManualReview,
            cancel_policy: SagaCancelPolicy::CompensateCompleted,
            ..
        }
    ));
}

#[test]
fn a_saga_node_reads_its_policies() {
    let doc = saga_document(
        json!({
            "id": "s", "type": "saga", "begin": "a",
            "onCompensationFailure": "fail", "cancelPolicy": "wait_in_flight_then_compensate"
        }),
        vec![activity("a")],
    );
    let parsed = parse(&doc).expect("parses");
    let saga = parsed
        .definition
        .node(&NodeId::parse("s").expect("id"))
        .expect("saga");
    assert!(matches!(
        saga.kind(),
        NodeKind::Saga {
            on_compensation_failure: OnCompensationFailure::Fail,
            cancel_policy: SagaCancelPolicy::WaitInFlightThenCompensate,
            ..
        }
    ));
    for (field, value) in [
        ("onCompensationFailure", "ignore"),
        ("cancelPolicy", "propagate"),
    ] {
        let mut bad = doc.clone();
        bad["nodes"][0][field] = json!(value);
        assert!(parse(&bad).is_err(), "{field}={value}");
    }
}

#[test]
fn the_effect_of_an_activity_is_read_with_its_defaults() {
    let mut step = activity("a");
    step["compensateWith"] = json!("undo.a");
    step["compensationRetry"] = json!({"maxAttempts": 7, "initialDelayMs": 500});
    step["compensationTimeoutMs"] = json!(45_000);
    step["highRisk"] = json!(true);
    let doc = saga_document(json!({"id": "s", "type": "saga", "begin": "a"}), vec![step]);
    let parsed = parse(&doc).expect("parses");
    let a = parsed
        .definition
        .node(&NodeId::parse("a").expect("id"))
        .expect("a");
    let effect = a.effect();
    assert!(effect.high_risk && !effect.irreversible);
    assert_eq!(
        effect.compensation_timeout,
        Some(Duration::from_millis(45_000))
    );
    let retry = effect.compensation_retry.expect("retry");
    assert_eq!(retry.max_attempts(), 7);
    assert_eq!(retry.initial_delay(), Duration::from_millis(500));
    // What the author left out comes from the compensation's defaults, not the forward ones.
    assert_eq!(retry.max_delay(), RetryPolicy::COMPENSATION.max_delay());
    // The derived node carries the budget and the timeout.
    let undo = parsed
        .definition
        .node(&NodeId::parse_stored("a~c").expect("id"))
        .expect("compensation");
    assert_eq!(undo.retry_policy().max_attempts(), 7);
    assert_eq!(undo.timeout(), Some(Duration::from_millis(45_000)));
}

#[test]
fn nonsense_in_a_saga_definition_is_refused_where_it_is() {
    let mut irreversible = activity("a");
    irreversible["irreversible"] = json!(true);
    irreversible["compensateWith"] = json!("undo.a");
    assert!(matches!(
        parse(&saga_document(
            json!({"id": "s", "type": "saga", "begin": "a"}),
            vec![irreversible]
        )),
        Err(IrError::Invalid(
            WorkflowError::IrreversibleWithCompensation(_)
        ))
    ));

    let mut on_timer = json!({"id": "t", "type": "timer", "durationMs": 1000});
    on_timer["highRisk"] = json!(true);
    assert!(matches!(
        parse(&minimal(json!([on_timer]), "t")),
        Err(IrError::Invalid(WorkflowError::EffectOnNonActivity(_)))
    ));

    let mut not_a_flag = activity("a");
    not_a_flag["highRisk"] = json!("yes");
    assert!(
        parse(&saga_document(
            json!({"id": "s", "type": "saga", "begin": "a"}),
            vec![not_a_flag]
        ))
        .is_err()
    );

    let with_end = saga_document(
        json!({"id": "s", "type": "saga", "begin": "a"}),
        vec![
            json!({"id": "a", "type": "activity", "activity": "act.a", "next": "z"}),
            json!({"id": "z", "type": "end"}),
        ],
    );
    assert!(matches!(
        parse(&with_end),
        Err(IrError::Invalid(WorkflowError::EndInsideSaga { .. }))
    ));

    let leaking = minimal(
        json!([
            json!({"id": "pre", "type": "activity", "activity": "act.pre", "next": "a"}),
            json!({"id": "s", "type": "saga", "begin": "a"}),
            activity("a"),
        ]),
        "pre",
    );
    assert!(
        parse(&leaking).is_err(),
        "a saga body is entered only from its begin"
    );
}

#[test]
fn a_compensation_outside_a_saga_is_a_warning() {
    let mut step = activity("a");
    step["compensateWith"] = json!("undo.a");
    let parsed = parse(&minimal(json!([step]), "a")).expect("parses");
    assert_eq!(parsed.warnings.len(), 1);
    assert!(parsed.warnings[0].to_string().contains("saga"));
}

// ------------------------------------------------------------------ problems an author can act on

#[test]
fn a_node_that_fits_no_type_is_reported_against_the_type_it_says_it_is() {
    // The generic answer of a `oneOf` ("is not valid under any of the schemas") tells an author
    // nothing: the problem is reported against the node's own type, at the field that is wrong.
    let doc = minimal(
        json!([
            {"id": "a", "type": "activity", "next": "s"},
            {"id": "s", "type": "script", "artifactDigest": "not-a-digest"},
            {"id": "t", "type": "timer"},
            {"id": "e", "type": "end"}
        ]),
        "a",
    );
    let issues = issues_of(&doc);
    let at = |path: &str| {
        issues
            .iter()
            .find(|(p, _)| p == path)
            .unwrap_or_else(|| panic!("no issue at {path} in {issues:#?}"))
            .1
            .clone()
    };
    assert!(at("/nodes/0").contains("activity"), "{issues:#?}");
    assert!(
        at("/nodes/1/artifactDigest").contains("sha256"),
        "{issues:#?}"
    );
    assert!(at("/nodes/2").contains("durationMs"), "{issues:#?}");
    assert!(
        issues.iter().all(|(_, m)| !m.contains("oneOf")),
        "no generic message is left: {issues:#?}"
    );
}

#[test]
fn a_node_with_an_unknown_or_missing_type_says_so_and_lists_the_types() {
    let doc = minimal(
        json!([
            {"id": "a", "type": "teleport", "next": "b"},
            {"id": "b", "next": "c"},
            {"id": "c", "type": "end"}
        ]),
        "a",
    );
    let issues = issues_of(&doc);
    let teleport = issues
        .iter()
        .find(|(p, _)| p == "/nodes/0/type")
        .expect("the unknown type is located");
    assert!(
        teleport.1.contains("teleport") && teleport.1.contains("activity"),
        "{teleport:?}"
    );
    assert!(
        issues.iter().any(|(p, _)| p.starts_with("/nodes/1")),
        "a node with no type is reported: {issues:#?}"
    );
}

#[test]
fn a_hostile_node_cannot_make_the_message_long() {
    let long = "x".repeat(10_000);
    let doc = minimal(json!([{"id": "a", "type": long}]), "a");
    for (_, message) in issues_of(&doc) {
        assert!(
            message.chars().count() <= 400,
            "{} chars",
            message.chars().count()
        );
    }
}

#[test]
fn an_approval_in_a_collection_or_loop_body_waits_for_scoped_compensation() {
    let gated = json!([
        {"id": "gate", "type": "approval", "policy": {"roles": ["operator"]}, "timeoutMs": 60_000,
         "onApprove": "work", "onReject": "work", "onExpire": "work"},
        {"id": "work", "type": "activity", "activity": "gate.work", "next": "out"},
        {"id": "out", "type": "return", "result": "steps.work.output"}
    ]);
    for owner in [
        json!({"id": "owner", "type": "forEach", "items": "input.items", "body": "iteration",
               "result": "bodyResult", "maxItems": 4, "maxConcurrency": 1, "next": "done"}),
        json!({"id": "owner", "type": "while", "condition": "carry < 1", "body": "iteration",
               "initial": "0", "carry": "bodyResult", "maxIterations": 3, "timeoutMs": 60_000,
               "next": "done"}),
    ] {
        let mut document = minimal(json!([owner, {"id": "done", "type": "end"}]), "owner");
        document["bodies"] = json!({"iteration": {"entrypoint": "gate", "nodes": gated}});
        match parse(&document) {
            Err(IrError::Pending {
                node,
                kind,
                dependency,
                reason,
            }) => {
                assert_eq!(
                    (node.as_str(), kind.as_str(), dependency),
                    ("gate", "approval", "SCOPED_COMPENSATION")
                );
                assert!(reason.contains("scoped compensation"), "{reason}");
            }
            other => panic!("expected the pending dependency, got {other:?}"),
        }
    }
}

//! The publishable top-level `limits` block (D297): an author may lower the platform ceilings of
//! 100000 instances and depth 8, never raise them, in every shape the runtime can run.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::{IrCodec, IrError};
use sagat_domain::workflow::WorkflowLimits;
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn plain() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "limited-flow"},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "flow.work", "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn with_limits(mut document: Value, limits: Value) -> Value {
    document["limits"] = limits;
    document
}

fn parse(document: &Value) -> Result<sagat_application::definitions::ParsedDefinition, IrError> {
    JsonIrCodec.parse(document.to_string().as_bytes())
}

fn declared_state() -> Value {
    let mut document = plain();
    document["state"] = json!({"schema": {"type": "integer"}, "initial": "0"});
    document
}

fn approval() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "limited-approval"},
        "entrypoint": "review",
        "nodes": [
            {
                "id": "review",
                "type": "approval",
                "timeoutMs": 60000,
                "policy": {"roles": ["operator"]},
                "onApprove": "approved",
                "onReject": "rejected",
                "onExpire": "rejected"
            },
            {"id": "approved", "type": "end"},
            {"id": "rejected", "type": "end"}
        ]
    })
}

fn collection() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "limited-collection"},
        "entrypoint": "items",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "line.process", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "items", "type": "forEach", "items": "input.items", "body": "line",
             "result": "bodyResult", "maxItems": 4, "maxConcurrency": 2, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn looping() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "limited-loop"},
        "entrypoint": "again",
        "bodies": {"iteration": {"entrypoint": "step", "nodes": [
            {"id": "step", "type": "activity", "activity": "loop.step", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.step.output"}
        ]}},
        "nodes": [
            {"id": "again", "type": "while", "condition": "carry < 3", "body": "iteration",
             "initial": "0", "carry": "bodyResult", "maxIterations": 5, "timeoutMs": 60000,
             "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn continuation() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "limited-continuation"},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    })
}

#[test]
fn a_lowered_limit_is_published_with_the_definition() {
    let parsed = parse(&with_limits(
        plain(),
        json!({"maxInstances": 7, "maxDepth": 2}),
    ))
    .expect("lowered limits publish");
    assert_eq!(
        parsed.definition.limits(),
        WorkflowLimits::new(7, 2).expect("limits")
    );
    let canonical = String::from_utf8(parsed.canonical).expect("canonical JSON");
    assert!(
        canonical.contains("\"maxInstances\":7"),
        "the pinned document must carry the author's limit: {canonical}"
    );
}

#[test]
fn an_omitted_field_keeps_its_platform_ceiling() {
    let instances = parse(&with_limits(plain(), json!({"maxInstances": 50}))).expect("instances");
    assert_eq!(
        instances.definition.limits(),
        WorkflowLimits::new(50, 8).expect("limits")
    );
    let depth = parse(&with_limits(plain(), json!({"maxDepth": 3}))).expect("depth");
    assert_eq!(
        depth.definition.limits(),
        WorkflowLimits::new(100_000, 3).expect("limits")
    );
    let empty = parse(&with_limits(plain(), json!({}))).expect("empty block");
    assert_eq!(empty.definition.limits(), WorkflowLimits::default());
}

#[test]
fn a_document_without_limits_is_unchanged() {
    let parsed = parse(&plain()).expect("plain workflow");
    assert_eq!(parsed.definition.limits(), WorkflowLimits::default());
    assert!(
        !String::from_utf8(parsed.canonical.clone())
            .expect("canonical JSON")
            .contains("limits"),
        "no limit may be added to a document that did not declare one"
    );
    let limited = parse(&with_limits(plain(), json!({"maxInstances": 7}))).expect("limited");
    assert_ne!(
        parsed.digest, limited.digest,
        "a different limit is a different pinned definition"
    );
}

#[test]
fn a_limit_can_never_be_raised_above_the_platform_ceiling() {
    for limits in [
        json!({"maxInstances": 0}),
        json!({"maxInstances": 100_001}),
        json!({"maxInstances": -1}),
        json!({"maxInstances": 1.5}),
        json!({"maxInstances": "7"}),
        json!({"maxDepth": 0}),
        json!({"maxDepth": 9}),
        json!({"maxDepth": 300}),
        json!({"maxInstances": 7, "maxNodes": 3}),
        json!([7]),
        json!(7),
    ] {
        assert!(
            parse(&with_limits(plain(), limits.clone())).is_err(),
            "{limits} must be refused at publication"
        );
    }
    parse(&with_limits(
        plain(),
        json!({"maxInstances": 100_000, "maxDepth": 8}),
    ))
    .expect("the ceiling itself is allowed");
    parse(&with_limits(
        plain(),
        json!({"maxInstances": 1, "maxDepth": 1}),
    ))
    .expect("the floor itself is allowed");
}

#[test]
fn every_publishable_shape_accepts_and_carries_the_block() {
    let limits = json!({"maxInstances": 40, "maxDepth": 2});
    for (shape, document) in [
        ("plain", plain()),
        ("declared state", declared_state()),
        ("approval", approval()),
        ("collection", collection()),
        ("loop", looping()),
        ("continuation", continuation()),
    ] {
        let parsed = parse(&with_limits(document, limits.clone()))
            .unwrap_or_else(|error| panic!("{shape} must accept limits: {error:?}"));
        assert_eq!(
            parsed.definition.limits(),
            WorkflowLimits::new(40, 2).expect("limits"),
            "{shape}"
        );
    }
}

#[test]
fn the_other_declarations_stay_closed_when_limits_are_present() {
    let mut triggered = with_limits(continuation(), json!({"maxInstances": 40}));
    triggered["triggers"] = json!([{"kind": "manual"}]);
    assert!(matches!(
        parse(&triggered),
        Err(IrError::UnsupportedDeclaration {
            field: "triggers",
            ..
        })
    ));
    let mut mixed = with_limits(collection(), json!({"maxInstances": 40}));
    mixed["state"] = json!({"schema": {"type": "integer"}, "initial": "0"});
    assert!(parse(&mixed).is_err(), "state with bodies stays closed");
}

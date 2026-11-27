//! The code document of the IR schema (design 044, Technical Plan 3.1 and 5.1): a published version is a graph or
//! a code version, never both; the envelope and the code profiles refuse what run-time code must not choose.

use std::sync::OnceLock;

use jsonschema::Validator;
use serde_json::{Value, json};

use crate::read_json;

fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| read_json("specs/001-platform-foundation/contracts/workflow.schema.json"))
}

fn document() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| jsonschema::validator_for(schema()).expect("schema compiles"))
}

/// A validator of one definition of the schema, with every definition in reach.
fn definition(name: &str) -> Validator {
    let wrapper = json!({
        "$schema": schema()["$schema"],
        "$ref": format!("#/$defs/{name}"),
        "$defs": schema()["$defs"],
    });
    jsonschema::validator_for(&wrapper).expect("definition compiles")
}

fn code_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "order"},
        "code": {
            "workflowType": "order",
            "language": "typescript",
            "digest": format!("sha256:{}", "a".repeat(64)),
            "envelope": {
                "activities": [
                    {"name": "inventory.*"},
                    {"name": "payments.charge", "secrets": ["payments-api"], "highRisk": true,
                     "compensateWith": "payments.refund", "reconcileWith": "payments.lookup",
                     "requiresApproval": "large-payments"},
                    {"name": "payments.refund", "secrets": ["payments-api"]},
                    {"name": "payments.lookup", "secrets": ["payments-api"]}
                ],
                "children": [{"workflowId": "shipping", "version": "active"}],
                "events": [{"eventType": "payment.confirmed", "maxLookbackMs": 600_000}],
                "approvalPolicies": {
                    "large-payments": {"policy": {"roles": ["operator"], "excludeInitiator": true}, "timeoutMs": 86_400_000}
                }
            }
        },
        "inputSchema": {"type": "object"},
        "state": {
            "schema": {"type": "object", "properties": {"total": {"type": "integer"}}, "additionalProperties": false},
            "initial": "{'total': 0}",
            "updates": {
                "addLine": {
                    "argsSchema": {"type": "object", "properties": {}, "additionalProperties": false},
                    "disclosure": "declared-business-result-v1",
                    "resultSchema": {"type": "object", "properties": {}, "additionalProperties": false}
                }
            }
        }
    })
}

fn graph_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "graph"},
        "entrypoint": "a",
        "nodes": [{"id": "a", "type": "end"}]
    })
}

fn with(mut document: Value, pointer: &str, value: Value) -> Value {
    let (parent, key) = pointer.rsplit_once('/').expect("a pointer");
    match document.pointer_mut(parent).expect("the parent exists") {
        Value::Object(object) => {
            object.insert(key.to_owned(), value);
        }
        Value::Array(items) => {
            let index: usize = key.parse().expect("an array index");
            items[index] = value;
        }
        other => panic!("{parent} is neither an object nor an array: {other}"),
    }
    document
}

#[test]
fn a_document_with_code_and_nodes_is_refused() {
    assert!(
        document().is_valid(&graph_document()),
        "a graph document stays valid"
    );
    assert!(
        document().is_valid(&code_document()),
        "a code document is valid"
    );

    // Both models at once, in every combination.
    for (key, value) in [
        ("entrypoint", json!("a")),
        ("nodes", json!([{"id": "a", "type": "end"}])),
        ("bodies", json!({})),
    ] {
        let both = with(code_document(), &format!("/{key}"), value);
        assert!(!document().is_valid(&both), "code with {key} is refused");
    }
    let both = with(graph_document(), "/code", code_document()["code"].clone());
    assert!(!document().is_valid(&both), "a graph with code is refused");

    // Neither model.
    let mut neither = graph_document();
    neither.as_object_mut().expect("object").remove("nodes");
    neither
        .as_object_mut()
        .expect("object")
        .remove("entrypoint");
    assert!(
        !document().is_valid(&neither),
        "a document with neither model is refused"
    );
}

#[test]
fn the_envelope_refuses_what_run_time_code_must_not_choose() {
    let refused = [
        // Only an exact entry may list secrets (constitution 5.0.0, principle IV).
        (
            "/code/envelope/activities/0",
            json!({"name": "inventory.*", "secrets": ["s"]}),
        ),
        // The reserved prefixes.
        (
            "/code/envelope/activities/0",
            json!({"name": "sagat.workflow"}),
        ),
        ("/code/envelope/activities/0", json!({"name": "script.run"})),
        // A malformed digest and workflow type.
        ("/code/digest", json!("sha256:ABC")),
        ("/code/workflowType", json!("9order")),
        // Design 045's timeout level waits for design 045 (Gap 044-020).
        ("/code/envelope/timeoutLevel", json!("activity")),
        // A code update has no expression form.
        ("/state/updates/addLine/validate", json!("true")),
    ];
    for (pointer, value) in refused {
        let changed = with(code_document(), pointer, value.clone());
        assert!(
            !document().is_valid(&changed),
            "{pointer} = {value} is refused"
        );
    }
    let timeouts = with(
        code_document(),
        "/code/workflowTaskTimeoutMs",
        json!(60_001),
    );
    assert!(
        !document().is_valid(&timeouts),
        "a workflow task timeout above 60 s is refused"
    );
}

#[test]
fn every_code_profile_refuses_the_fields_outside_it() {
    let accepted = [
        (
            "codeProfileActivity",
            json!({"type": "activity", "activity": "payments.charge", "retry": {"maxAttempts": 3}}),
        ),
        (
            "codeProfileScript",
            json!({"type": "script", "artifactDigest": format!("sha256:{}", "b".repeat(64))}),
        ),
        (
            "codeProfileTimer",
            json!({"type": "timer", "durationMs": 1000}),
        ),
        (
            "codeProfileWaitEvent",
            json!({"type": "waitEvent", "eventType": "payment.confirmed", "correlationKey": "o-1", "timeoutMs": 1000}),
        ),
        (
            "codeProfileChild",
            json!({"type": "child", "workflowId": "shipping", "cancelPolicy": "propagate"}),
        ),
        (
            "codeProfileApproval",
            json!({"type": "approval", "policyName": "large-payments"}),
        ),
    ];
    for (profile, fragment) in &accepted {
        assert!(
            definition(profile).is_valid(fragment),
            "{profile} accepts {fragment}"
        );
    }
    let refused = [
        (
            "codeProfileActivity",
            "idempotencyTemplate",
            json!("{{input.id}}"),
        ),
        ("codeProfileActivity", "destinations", json!([])),
        ("codeProfileActivity", "onError", json!("n")),
        ("codeProfileActivity", "outputSchema", json!({})),
        ("codeProfileActivity", "input", json!({})),
        ("codeProfileActivity", "next", json!("n")),
        ("codeProfileWaitEvent", "correlationExpr", json!("input.id")),
        ("codeProfileWaitEvent", "onTimeout", json!("n")),
        ("codeProfileChild", "version", json!(3)),
        ("codeProfileChild", "input", json!("input.order")),
        (
            "codeProfileApproval",
            "policy",
            json!({"roles": ["operator"]}),
        ),
        ("codeProfileApproval", "onApprove", json!("n")),
    ];
    for (profile, key, value) in refused {
        let fragment = accepted
            .iter()
            .find(|(name, _)| *name == profile)
            .map(|(_, fragment)| fragment.clone())
            .expect("an accepted fragment of the profile");
        let changed = with(fragment, &format!("/{key}"), value);
        assert!(
            !definition(profile).is_valid(&changed),
            "{profile} refuses {key}"
        );
    }
    let reserved = json!({"type": "activity", "activity": "sagat.workflow:order"});
    assert!(
        !definition("codeProfileActivity").is_valid(&reserved),
        "a reserved activity name is refused"
    );
}

/// A script command takes the graph script node's `timeoutMs` and a wait always has a bound, each within the code
/// profile's 366 days (Decision 044-247).
#[test]
fn a_script_takes_a_bounded_timeout_and_a_wait_needs_one() {
    let script = json!({
        "type": "script",
        "artifactDigest": format!("sha256:{}", "b".repeat(64)),
        "timeoutMs": 31_622_400_000_u64
    });
    assert!(
        definition("codeProfileScript").is_valid(&script),
        "a script command takes timeoutMs up to 366 days, as a graph script node does"
    );
    let longer_script = with(script, "/timeoutMs", json!(31_622_400_001_u64));
    assert!(
        !definition("codeProfileScript").is_valid(&longer_script),
        "and no longer"
    );
    let unbounded =
        json!({"type": "waitEvent", "eventType": "payment.confirmed", "correlationKey": "o-1"});
    assert!(
        !definition("codeProfileWaitEvent").is_valid(&unbounded),
        "a wait for an event needs timeoutMs"
    );
    let longest = with(unbounded, "/timeoutMs", json!(31_622_400_000_u64));
    assert!(
        definition("codeProfileWaitEvent").is_valid(&longest),
        "a wait may last 366 days"
    );
    let longer = with(longest, "/timeoutMs", json!(31_622_400_001_u64));
    assert!(
        !definition("codeProfileWaitEvent").is_valid(&longer),
        "and no longer"
    );
}

//! Internal additive-v1 parsing for declared workflow state.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::{IrCodec, IrError};
use sagat_domain::workflow::ActivityName;
use sagat_infra_ir::{JsonIrCodec, MAX_DECLARED_SCHEMA_BYTES, MAX_DECLARED_SCHEMA_DEPTH};
use serde_json::{Value, json};

fn document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "account-state"},
        "entrypoint": "done",
        "nodes": [{"id": "done", "type": "end"}],
        "state": {
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"balance": {"type": "integer"}},
                "required": ["balance"]
            },
            "initial": "input.account",
            "queries": {
                "balance.read": {
                    "argsSchema": {"type": "object"},
                    "resultSchema": {"type": "integer"},
                    "expression": "state.balance",
                    "disclosure": "declared-business-result-v1"
                }
            },
            "updates": {
                "balance.credit": {
                    "argsSchema": {"type": "integer"},
                    "validate": "args > 0",
                    "state": "state",
                    "result": "newState.balance",
                    "resultSchema": {"type": "integer"},
                    "disclosure": "declared-business-result-v1"
                }
            }
        }
    })
}

#[test]
fn preserves_typed_state_and_operation_metadata_for_publication() {
    let source = document();
    let bytes = serde_json::to_vec(&source).expect("document");
    let parsed = JsonIrCodec
        .parse_declared_state(&bytes)
        .expect("declared state validates");

    assert_eq!(parsed.state().initial().as_str(), "input.account");
    assert!(!parsed.state().schema().canonical().is_empty());
    let query_name = ActivityName::parse("balance.read").expect("query name");
    let query = parsed.state().query(&query_name).expect("query");
    assert_eq!(query.expression().as_str(), "state.balance");
    assert_eq!(query.name(), &query_name);
    let update_name = ActivityName::parse("balance.credit").expect("update name");
    let update = parsed.state().update(&update_name).expect("update");
    assert_eq!(update.validator().expect("validator").as_str(), "args > 0");
    assert_eq!(update.state().as_str(), "state");
    assert_eq!(update.result().as_str(), "newState.balance");
    assert_eq!(parsed.state().queries().count(), 1);
    assert_eq!(parsed.state().updates().count(), 1);

    let canonical: Value =
        serde_json::from_slice(&parsed.parsed().canonical).expect("canonical document");
    assert_eq!(canonical["state"], source["state"]);
    let reparsed = JsonIrCodec
        .parse_declared_state(&parsed.parsed().canonical)
        .expect("canonical document reparses");
    assert_eq!(reparsed.state(), parsed.state());
    assert_eq!(reparsed.parsed().digest, parsed.parsed().digest);

    let publication = JsonIrCodec
        .parse(&bytes)
        .expect("basic workflow with declared state is publishable");
    assert_eq!(publication.canonical, parsed.parsed().canonical);
    assert_eq!(publication.digest, parsed.parsed().digest);
    assert!(matches!(
        JsonIrCodec.parse_structured(&bytes),
        Err(IrError::UnsupportedDeclaration { field: "state", .. })
    ));
}

#[test]
fn publication_rejects_unreleased_capabilities_combined_with_declared_state() {
    let mut with_chain = document();
    with_chain["chainPolicy"] = json!({"maxSegments": 2, "timeoutMs": 1_000});
    assert!(
        JsonIrCodec
            .parse(with_chain.to_string().as_bytes())
            .is_err()
    );

    let mut with_limits = document();
    with_limits["limits"] = json!({"maxInstances": 100, "maxDepth": 2});
    let parsed = JsonIrCodec
        .parse(with_limits.to_string().as_bytes())
        .expect("declared state accepts lowered limits (D297)");
    assert_eq!(parsed.definition.limits().max_instances(), 100);
    assert_eq!(parsed.definition.limits().max_depth(), 2);

    let mut with_structured_node = document();
    with_structured_node["entrypoint"] = json!("review");
    with_structured_node["nodes"] = json!([
        {
            "id": "review",
            "type": "approval",
            "policy": {"roles": ["admin"]},
            "timeoutMs": 1_000,
            "onApprove": "approved",
            "onReject": "rejected",
            "onExpire": "expired"
        },
        {"id": "approved", "type": "end"},
        {"id": "rejected", "type": "end"},
        {"id": "expired", "type": "end"}
    ]);
    assert!(matches!(
        JsonIrCodec.parse(with_structured_node.to_string().as_bytes()),
        Err(IrError::Unsupported { kind, .. }) if kind == "approval"
    ));
}

#[test]
fn compiles_expressions_in_their_exact_data_environments() {
    for (pointer, expression, field) in [
        ("/state/initial", "state", "initial"),
        (
            "/state/queries/balance.read/expression",
            "newState",
            "expression",
        ),
        (
            "/state/updates/balance.credit/validate",
            "newState.balance > 0",
            "validate",
        ),
        ("/state/updates/balance.credit/state", "newState", "state"),
    ] {
        let mut invalid = document();
        *invalid.pointer_mut(pointer).expect("expression pointer") = json!(expression);
        let bytes = invalid.to_string();
        assert!(matches!(
            JsonIrCodec.parse_declared_state(bytes.as_bytes()),
            Err(IrError::Expression { field: actual, .. }) if actual == field
        ));
        assert!(matches!(
            JsonIrCodec.parse(bytes.as_bytes()),
            Err(IrError::Expression { field: actual, .. }) if actual == field
        ));
    }

    let mut invalid = document();
    invalid["state"]["updates"]["balance.credit"]["result"] = json!("network.call(args)");
    let bytes = invalid.to_string();
    assert!(matches!(
        JsonIrCodec.parse_declared_state(bytes.as_bytes()),
        Err(IrError::Expression {
            field: "result",
            ..
        })
    ));
    assert!(matches!(
        JsonIrCodec.parse(bytes.as_bytes()),
        Err(IrError::Expression {
            field: "result",
            ..
        })
    ));
}

#[test]
fn rejects_uncompiled_oversized_deep_referenced_or_incoherent_schemas() {
    assert_eq!(MAX_DECLARED_SCHEMA_BYTES, 65_536);
    assert_eq!(MAX_DECLARED_SCHEMA_DEPTH, 16);

    let mut oversized = document();
    oversized["state"]["queries"]["balance.read"]["argsSchema"] = json!({
        "type": "object",
        "x-padding": "x".repeat(MAX_DECLARED_SCHEMA_BYTES)
    });
    assert!(matches!(
        JsonIrCodec.parse_declared_state(oversized.to_string().as_bytes()),
        Err(IrError::Schema(issues)) if issues[0].path.ends_with("/argsSchema")
    ));

    let mut nested = json!({"type": "integer"});
    for _ in 0..MAX_DECLARED_SCHEMA_DEPTH {
        nested = json!({"type": "array", "items": nested, "maxItems": 1});
    }
    let mut deep = document();
    deep["state"]["schema"] = nested;
    assert!(matches!(
        JsonIrCodec.parse_declared_state(deep.to_string().as_bytes()),
        Err(IrError::Schema(issues)) if issues[0].path == "/state/schema"
    ));

    let mut referenced = document();
    referenced["state"]["queries"]["balance.read"]["argsSchema"] =
        json!({"$ref": "https://example.invalid/schema.json"});
    assert!(matches!(
        JsonIrCodec.parse_declared_state(referenced.to_string().as_bytes()),
        Err(IrError::Schema(issues)) if issues[0].path.ends_with("/argsSchema")
    ));

    let mut incoherent = document();
    incoherent["state"]["schema"] = json!({"type": "string", "minLength": 10, "maxLength": 2});
    assert!(matches!(
        JsonIrCodec.parse_declared_state(incoherent.to_string().as_bytes()),
        Err(IrError::Schema(issues)) if issues[0].path == "/state/schema"
    ));
}

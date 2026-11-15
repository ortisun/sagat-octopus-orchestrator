//! `reapplyOnReset` once a deployment has enabled the capability (`T018-078`, ADR-0032 decision 8): a new
//! publication keeps the field in the canonical document, and switching the capability off again refuses a
//! new publication while a stored version still restores. The switch belongs to the process, so this is a
//! binary of its own; `reapply_on_reset.rs` holds the refusals of the closed default.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::{IrCodec, IrError};
use sagat_infra_ir::{JsonIrCodec, reset_reapply_enabled, set_reset_reapply};
use serde_json::{Value, json};

fn document(reapply: bool) -> Vec<u8> {
    serde_json::to_vec(&json!({
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
            "updates": {"balance.credit": {
                "argsSchema": {"type": "integer"},
                "state": "state",
                "result": "newState.balance",
                "resultSchema": {"type": "integer"},
                "disclosure": "declared-business-result-v1",
                "reapplyOnReset": reapply
            }}
        }
    }))
    .expect("document")
}

#[test]
fn the_capability_is_enabled_and_disabled_by_the_deployment_one_switch_at_a_time() {
    assert!(
        !reset_reapply_enabled(),
        "off until a deployment turns it on"
    );
    let bytes = document(true);
    assert!(matches!(
        JsonIrCodec.parse(&bytes),
        Err(IrError::UnsupportedDeclaration {
            field: "reapplyOnReset",
            ..
        })
    ));

    set_reset_reapply(true);
    assert!(reset_reapply_enabled());
    let published = JsonIrCodec.parse(&bytes).expect("publishes once enabled");
    let canonical: Value = serde_json::from_slice(&published.canonical).expect("canonical");
    assert_eq!(
        canonical["state"]["updates"]["balance.credit"]["reapplyOnReset"], true,
        "the field is part of the canonical document, so of its digest"
    );
    let publication = JsonIrCodec
        .declared_state_publication(&bytes)
        .expect("the publication of the operations")
        .expect("a state is declared");
    assert!(
        publication.operations().next().is_some(),
        "the Update is one of the published operations"
    );

    // closing it again refuses a new publication and leaves the stored version readable
    set_reset_reapply(false);
    assert!(JsonIrCodec.parse(&bytes).is_err());
    let stored = JsonIrCodec
        .parse_stored(&published.canonical)
        .expect("a version published while it was on restores");
    assert_eq!(stored.digest, published.digest);
}

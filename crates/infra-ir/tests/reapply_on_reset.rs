//! `reapplyOnReset` on an Update declaration (`T018-078`, `FR-018-039`, `FR-018-046`, ADR-0032): an optional,
//! additive boolean that is read by every parser, left out of a document that does not write it, and
//! refused by a new publication while the capability that uses it is not enabled. Nothing here turns the
//! capability on; `reapply_on_reset_enabled.rs` is the binary that does, because the switch is the
//! process's.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::{IrCodec, IrError};
use sagat_domain::workflow::ActivityName;
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

/// A workflow with one declared Update; `reapply` is written into the declaration as given.
fn document(reapply: Option<Value>) -> Vec<u8> {
    let mut update = json!({
        "argsSchema": {"type": "integer"},
        "validate": "args > 0",
        "state": "state",
        "result": "newState.balance",
        "resultSchema": {"type": "integer"},
        "disclosure": "declared-business-result-v1"
    });
    if let Some(value) = reapply {
        update["reapplyOnReset"] = value;
    }
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
            "updates": {"balance.credit": update}
        }
    }))
    .expect("document")
}

fn reapplies(document: &[u8]) -> bool {
    let parsed = JsonIrCodec
        .parse_declared_state(document)
        .expect("declared state validates");
    let name = ActivityName::parse("balance.credit").expect("name");
    parsed
        .state()
        .update(&name)
        .expect("update")
        .reapply_on_reset()
}

#[test]
fn a_declaration_says_whether_its_arguments_are_kept_for_a_reset_and_the_default_is_no() {
    assert!(reapplies(&document(Some(json!(true)))));
    assert!(!reapplies(&document(Some(json!(false)))));
    assert!(
        !reapplies(&document(None)),
        "a declaration that does not write the field keeps nothing"
    );
}

#[test]
fn the_field_is_a_boolean_and_nothing_else() {
    for wrong in [
        json!("yes"),
        json!(1),
        json!(null),
        json!({}),
        json!([true]),
    ] {
        let bytes = document(Some(wrong.clone()));
        assert!(
            matches!(
                JsonIrCodec.parse_declared_state(&bytes),
                Err(IrError::Schema(_))
            ),
            "{wrong} is not a boolean"
        );
        assert!(
            matches!(JsonIrCodec.parse(&bytes), Err(IrError::Schema(_))),
            "{wrong} is not published"
        );
    }
}

#[test]
fn a_new_publication_refuses_it_until_the_capability_is_enabled_and_names_the_field() {
    let bytes = document(Some(json!(true)));
    match JsonIrCodec.parse(&bytes) {
        Err(IrError::UnsupportedDeclaration { field, reason }) => {
            assert_eq!(field, "reapplyOnReset");
            assert_eq!(
                reason,
                "this workflow capability is not enabled by the runtime"
            );
        }
        other => panic!("expected the capability to be refused, got {other:?}"),
    }
    // the publication path a version is stored through stands on the same parse
    assert!(JsonIrCodec.declared_state_publication(&bytes).is_err());

    // a declaration that opts out, or says nothing, is an ordinary document
    for ordinary in [document(Some(json!(false))), document(None)] {
        JsonIrCodec.parse(&ordinary).expect("publishable as before");
    }
}

#[test]
fn a_version_already_published_with_the_field_is_still_read() {
    // ADR-0032 decision 8: closing a capability never erases what was published under it
    let bytes = document(Some(json!(true)));
    let stored = JsonIrCodec
        .parse_stored(&bytes)
        .expect("a stored version restores");
    assert_eq!(
        stored.canonical,
        JsonIrCodec
            .parse_stored(&stored.canonical)
            .expect("again")
            .canonical
    );
    assert!(reapplies(&bytes));
    let restored = JsonIrCodec
        .parse_stored_declared_state(&bytes)
        .expect("the operations of a stored version restore");
    let name = ActivityName::parse("balance.credit").expect("name");
    assert!(
        restored
            .state()
            .update(&name)
            .expect("update")
            .reapply_on_reset()
    );
}

#[test]
fn only_a_declaration_that_writes_the_field_changes_its_digest() {
    let digest = |bytes: &[u8]| {
        let parsed = JsonIrCodec.parse_declared_state(bytes).expect("parses");
        let name = ActivityName::parse("balance.credit").expect("name");
        parsed.state().update(&name).expect("update").digest()
    };
    // the digest this declaration had before the field existed, taken from the code of that day: a
    // document that does not write the field must keep it, or every published operation would change
    const BEFORE_THE_FIELD: &str =
        "e6de3b4dc9d4b42819a58bf35ee96da2ca69a8653e7985ad846bc29ad3e97354";
    assert_eq!(digest(&document(None)).to_hex(), BEFORE_THE_FIELD);
    // writing the default is the same declaration as not writing it
    assert_eq!(
        digest(&document(Some(json!(false)))).to_hex(),
        BEFORE_THE_FIELD
    );
    // opting in is a different declaration
    assert_ne!(
        digest(&document(Some(json!(true)))).to_hex(),
        BEFORE_THE_FIELD
    );
}

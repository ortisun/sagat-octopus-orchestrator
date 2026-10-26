//! Declarative evaluation of schema-pinned Query and Update operations.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::operation_evaluation::{
    MAX_OPERATION_CONTEXT_BYTES, OperationDataField, OperationEvaluationContext,
    OperationEvaluationError, RetainedOperationValue,
};
use sagat_domain::execution::{
    DeclaredOperationsState, OperationValue, RunId, UpdateEvaluation, UpdateRejection,
};
use sagat_domain::workflow::{ActivityName, Digest};
use sagat_infra_ir::{JsonDeclaredOperationEvaluator, JsonIrCodec};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

fn document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "evaluated-account"},
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
                    "argsSchema": {"type": "integer"},
                    "resultSchema": {"type": "integer"},
                    "expression": "input.offset + completedResults.seed + state.balance + args",
                    "disclosure": "declared-business-result-v1"
                }
            },
            "updates": {
                "balance.replace": {
                    "argsSchema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "allowed": {"type": "boolean"},
                            "nextState": {"type": "object"}
                        },
                        "required": ["allowed", "nextState"]
                    },
                    "validate": "args.allowed",
                    "state": "args.nextState",
                    "result": "newState.balance",
                    "resultSchema": {"type": "integer"},
                    "disclosure": "declared-business-result-v1"
                }
            }
        }
    })
}

fn parsed(document: &Value) -> sagat_infra_ir::ParsedDeclaredState {
    JsonIrCodec
        .parse_declared_state(document.to_string().as_bytes())
        .expect("declared operation document")
}

fn operation_value(content: &[u8], schema_digest: Digest) -> OperationValue {
    let digest: [u8; 32] = Sha256::digest(content).into();
    OperationValue::new(
        Digest::new(digest),
        schema_digest,
        u32::try_from(content.len()).expect("bounded test value"),
    )
    .expect("operation value")
}

fn context<'a>(
    state_reference: &'a OperationValue,
    state: &'a [u8],
    arguments: &'a [u8],
) -> OperationEvaluationContext<'a> {
    let state = RetainedOperationValue::new(state_reference, state).expect("retained state");
    OperationEvaluationContext::new(8, br#"{"offset":1}"#, state, br#"{"seed":2}"#, arguments)
        .expect("bounded context")
}

#[test]
fn initial_state_is_schema_checked_canonical_and_ready_for_transactional_retention() {
    let declared = parsed(&document());
    let evaluated = JsonDeclaredOperationEvaluator
        .initial_state(declared.state(), br#"{"account":{"balance":4}}"#)
        .expect("initial state");

    assert_eq!(evaluated.content(), br#"{"balance":4}"#);
    assert_eq!(
        evaluated.value().schema_digest(),
        declared.state().schema().digest()
    );
    assert_eq!(
        declared
            .state()
            .publication()
            .expect("publication metadata")
            .operations()
            .count(),
        2
    );
}

#[test]
fn initial_state_rejects_a_value_that_does_not_match_the_published_schema() {
    let declared = parsed(&document());

    assert_eq!(
        JsonDeclaredOperationEvaluator.initial_state(
            declared.state(),
            br#"{"account":{"balance":"canary-secret"}}"#,
        ),
        Err(OperationEvaluationError::SchemaMismatch {
            field: OperationDataField::State,
        })
    );
}

#[test]
fn query_validates_inputs_and_result_then_binds_it_to_the_observed_snapshot() {
    let declared = parsed(&document());
    let state_content = br#"{"balance":4}"#;
    let state_reference = operation_value(state_content, declared.state().schema().digest());
    let context = context(&state_reference, state_content, b"3");
    let evaluated = JsonDeclaredOperationEvaluator
        .query(
            declared.state(),
            &ActivityName::parse("balance.read").expect("operation"),
            &context,
        )
        .expect("query result");

    assert_eq!(evaluated.result().content(), b"10");
    assert_eq!(
        evaluated.result().value().schema_digest(),
        declared
            .state()
            .query(&ActivityName::parse("balance.read").expect("operation"))
            .expect("query")
            .result_schema()
            .digest()
    );
    let run = "0198c5a0-0000-7000-8000-000000000001"
        .parse::<RunId>()
        .expect("run id");
    let stale = DeclaredOperationsState::new(state_reference.clone(), 5).query_snapshot(run, 9);
    assert_eq!(
        evaluated.response(stale),
        Err(OperationEvaluationError::StateChanged)
    );
    let snapshot = DeclaredOperationsState::new(state_reference, 5).query_snapshot(run, 8);
    let response = evaluated.response(snapshot).expect("same snapshot");
    assert_eq!(response.run_id(), run);
    assert_eq!(response.run_version(), 8);
    assert_eq!(response.state_revision(), 0);
    assert_eq!(response.result(), evaluated.result().value());
}

#[test]
fn update_uses_one_pre_state_and_validates_new_state_before_its_result() {
    let declared = parsed(&document());
    let state_content = br#"{"balance":4}"#;
    let state_reference = operation_value(state_content, declared.state().schema().digest());
    let arguments = br#"{"allowed":true,"nextState":{"balance":8}}"#;
    let context = context(&state_reference, state_content, arguments);
    let evaluated = JsonDeclaredOperationEvaluator
        .update(
            declared.state(),
            &ActivityName::parse("balance.replace").expect("operation"),
            &context,
        )
        .expect("update result");

    assert_eq!(
        evaluated.new_state().expect("new state").content(),
        br#"{"balance":8}"#
    );
    assert_eq!(evaluated.result().expect("result").content(), b"8");
    assert!(matches!(
        evaluated
            .domain_evaluation(&state_reference, 8)
            .expect("same pre-state"),
        UpdateEvaluation::Applied { new_state, result }
            if new_state == *evaluated.new_state().expect("new state").value()
                && result == *evaluated.result().expect("result").value()
    ));

    let other_state = operation_value(br#"{"balance":5}"#, declared.state().schema().digest());
    assert_eq!(
        evaluated.domain_evaluation(&other_state, 8),
        Err(OperationEvaluationError::StateChanged)
    );
    assert_eq!(
        evaluated.domain_evaluation(&state_reference, 9),
        Err(OperationEvaluationError::StateChanged)
    );
}

#[test]
fn validator_rejection_does_not_evaluate_the_state_or_result_expressions() {
    let mut source = document();
    source["state"]["updates"]["balance.replace"]["state"] = json!("args.missing");
    source["state"]["updates"]["balance.replace"]["result"] = json!("newState.missing");
    let declared = parsed(&source);
    let state_content = br#"{"balance":4}"#;
    let state_reference = operation_value(state_content, declared.state().schema().digest());
    let arguments = br#"{"allowed":false,"nextState":{"balance":8}}"#;
    let context = context(&state_reference, state_content, arguments);

    let evaluated = JsonDeclaredOperationEvaluator
        .update(
            declared.state(),
            &ActivityName::parse("balance.replace").expect("operation"),
            &context,
        )
        .expect("durable business rejection");
    assert!(evaluated.new_state().is_none());
    assert!(evaluated.result().is_none());
    assert_eq!(
        evaluated
            .domain_evaluation(&state_reference, 8)
            .expect("same pre-state"),
        UpdateEvaluation::Rejected(UpdateRejection::ValidatorRejected)
    );
}

#[test]
fn schema_failures_are_typed_and_never_return_unchecked_values() {
    let declared = parsed(&document());
    let state_content = br#"{"balance":4}"#;
    let state_reference = operation_value(state_content, declared.state().schema().digest());
    let invalid_arguments = context(&state_reference, state_content, br#"{"wrong":true}"#);
    assert_eq!(
        JsonDeclaredOperationEvaluator.query(
            declared.state(),
            &ActivityName::parse("balance.read").expect("operation"),
            &invalid_arguments,
        ),
        Err(OperationEvaluationError::SchemaMismatch {
            field: OperationDataField::Arguments,
        })
    );

    let invalid_state_content = br#"{"balance":"canary-secret"}"#;
    let invalid_state_reference =
        operation_value(invalid_state_content, declared.state().schema().digest());
    let invalid_state = context(&invalid_state_reference, invalid_state_content, b"3");
    assert_eq!(
        JsonDeclaredOperationEvaluator.query(
            declared.state(),
            &ActivityName::parse("balance.read").expect("operation"),
            &invalid_state,
        ),
        Err(OperationEvaluationError::SchemaMismatch {
            field: OperationDataField::State,
        })
    );

    let invalid_new_state = context(
        &state_reference,
        state_content,
        br#"{"allowed":true,"nextState":{"balance":"canary-secret"}}"#,
    );
    assert_eq!(
        JsonDeclaredOperationEvaluator.update(
            declared.state(),
            &ActivityName::parse("balance.replace").expect("operation"),
            &invalid_new_state,
        ),
        Err(OperationEvaluationError::SchemaMismatch {
            field: OperationDataField::NewState,
        })
    );

    let mut invalid_query_result = document();
    invalid_query_result["state"]["queries"]["balance.read"]["resultSchema"] =
        json!({"type": "string", "maxLength": 16});
    let declared_query = parsed(&invalid_query_result);
    let query_state_reference =
        operation_value(state_content, declared_query.state().schema().digest());
    assert_eq!(
        JsonDeclaredOperationEvaluator.query(
            declared_query.state(),
            &ActivityName::parse("balance.read").expect("operation"),
            &context(&query_state_reference, state_content, b"3"),
        ),
        Err(OperationEvaluationError::SchemaMismatch {
            field: OperationDataField::QueryResult,
        })
    );

    let mut invalid_update_result = document();
    invalid_update_result["state"]["updates"]["balance.replace"]["resultSchema"] =
        json!({"type": "string", "maxLength": 16});
    let declared_update = parsed(&invalid_update_result);
    let update_state_reference =
        operation_value(state_content, declared_update.state().schema().digest());
    assert_eq!(
        JsonDeclaredOperationEvaluator.update(
            declared_update.state(),
            &ActivityName::parse("balance.replace").expect("operation"),
            &context(
                &update_state_reference,
                state_content,
                br#"{"allowed":true,"nextState":{"balance":8}}"#,
            ),
        ),
        Err(OperationEvaluationError::SchemaMismatch {
            field: OperationDataField::UpdateResult,
        })
    );
}

#[test]
fn malformed_oversized_or_mismatched_contexts_fail_without_disclosing_values() {
    let declared = parsed(&document());
    let state_content = br#"{"balance":4}"#;
    let state_reference = operation_value(state_content, declared.state().schema().digest());

    assert_eq!(
        RetainedOperationValue::new(&state_reference, br#"{"balance":5}"#),
        Err(OperationEvaluationError::RetainedValueMismatch)
    );
    assert_eq!(
        OperationEvaluationContext::new(
            8,
            br#"{"canary-secret":"unterminated"#,
            RetainedOperationValue::new(&state_reference, state_content).expect("state"),
            b"{}",
            b"3",
        )
        .and_then(|context| {
            JsonDeclaredOperationEvaluator.query(
                declared.state(),
                &ActivityName::parse("balance.read").expect("operation"),
                &context,
            )
        }),
        Err(OperationEvaluationError::MalformedJson {
            field: OperationDataField::WorkflowInput,
        })
    );

    let oversized = vec![b'x'; 256 * 1024 + 1];
    assert_eq!(
        OperationEvaluationContext::new(
            8,
            b"{}",
            RetainedOperationValue::new(&state_reference, state_content).expect("state"),
            b"{}",
            &oversized,
        ),
        Err(OperationEvaluationError::ArgumentsTooLarge)
    );
    let oversized_context = vec![b' '; MAX_OPERATION_CONTEXT_BYTES];
    assert_eq!(
        OperationEvaluationContext::new(
            8,
            &oversized_context,
            RetainedOperationValue::new(&state_reference, state_content).expect("state"),
            b"{}",
            b"3",
        ),
        Err(OperationEvaluationError::ContextTooLarge)
    );
    let rendered = format!(
        "{:?}",
        OperationEvaluationContext::new(
            8,
            br#"{"canary-secret":1}"#,
            RetainedOperationValue::new(&state_reference, state_content).expect("state"),
            b"{}",
            b"3",
        )
        .expect("context")
    );
    assert!(!rendered.contains("canary-secret"));
}

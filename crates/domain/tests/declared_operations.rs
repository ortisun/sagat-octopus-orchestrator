//! Pure state and receipt semantics for declared Query and Update operations.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{
    DeclaredOperationsState, OperationAddress, OperationError, OperationScope, OperationValue,
    QueryResponse, ResultDisclosure, RunId, RunStatus, UpdateEvaluation, UpdateKey,
    UpdateReceiptStatus, UpdateRejection, UpdateRequest,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::{ActivityName, Digest};

fn value(marker: u8, size: u32) -> OperationValue {
    OperationValue::new(Digest::new([marker; 32]), Digest::new([240; 32]), size)
        .expect("bounded value")
}

fn value_with_schema(marker: u8, schema_marker: u8, size: u32) -> OperationValue {
    OperationValue::new(
        Digest::new([marker; 32]),
        Digest::new([schema_marker; 32]),
        size,
    )
    .expect("bounded value")
}

fn name(value: &str) -> ActivityName {
    ActivityName::parse(value).expect("valid operation name")
}

fn key(value: &str) -> UpdateKey {
    UpdateKey::parse(value).expect("valid update key")
}

fn actor() -> Subject {
    Subject::parse("operator@example.test").expect("valid actor")
}

fn scope(run: RunId) -> OperationScope {
    OperationScope::new(
        TenantId::generate(),
        NamespaceId::generate(),
        OperationAddress::Run(run),
    )
}

fn request(
    scope: OperationScope,
    run: RunId,
    operation: &str,
    update_key: &str,
    request_marker: u8,
    expected_revision: Option<u64>,
) -> UpdateRequest {
    UpdateRequest::new(
        scope,
        run,
        name(operation),
        key(update_key),
        Digest::new([request_marker; 32]),
        expected_revision,
        actor(),
    )
}

#[test]
fn disclosure_operation_names_keys_and_values_are_closed_and_bounded() {
    assert_eq!(
        ResultDisclosure::parse("declared-business-result-v1"),
        Ok(ResultDisclosure::DeclaredBusinessResult)
    );
    assert_eq!(
        ResultDisclosure::DeclaredBusinessResult.as_str(),
        "declared-business-result-v1"
    );
    assert!(ResultDisclosure::parse("implicit").is_err());

    assert!(ActivityName::parse("inventory.reserve:v1").is_ok());
    assert!(ActivityName::parse("").is_err());
    assert!(ActivityName::parse("-starts-wrong").is_err());
    assert!(ActivityName::parse(&"a".repeat(129)).is_err());

    assert!(UpdateKey::parse("request:01/attempt").is_ok());
    for invalid in ["", "contains space", "line\nbreak", "não-ascii"] {
        assert!(UpdateKey::parse(invalid).is_err(), "{invalid:?}");
    }
    assert!(UpdateKey::parse(&"k".repeat(129)).is_err());

    let bounded = OperationValue::new(Digest::new([1; 32]), Digest::new([2; 32]), 256 * 1024)
        .expect("bounded value");
    assert_eq!(bounded.digest(), Digest::new([1; 32]));
    assert_eq!(bounded.schema_digest(), Digest::new([2; 32]));
    assert_eq!(
        OperationValue::new(Digest::new([1; 32]), Digest::new([2; 32]), 256 * 1024 + 1,),
        Err(OperationError::ValueTooLarge)
    );
}

#[test]
fn query_reads_one_snapshot_without_mutating_state_or_revision() {
    let run = RunId::generate();
    let state = DeclaredOperationsState::new(value(1, 8), 4);
    let before = state.clone();

    let snapshot = state.query_snapshot(run, 9);
    let response = QueryResponse::new(snapshot, value(2, 16));

    assert_eq!(response.run_id(), run);
    assert_eq!(response.run_version(), 9);
    assert_eq!(response.state_revision(), 0);
    assert_eq!(response.result(), &value(2, 16));
    assert_eq!(response.snapshot().state(), &value(1, 8));
    assert_eq!(state, before);
    assert_eq!(state.last_changed_run_version(), 4);
}

#[test]
fn applied_update_changes_state_once_and_lost_response_replays_the_receipt() {
    let run = RunId::generate();
    let scope = scope(run);
    let request = request(scope, run, "increment", "update-once", 7, Some(0));
    let mut state = DeclaredOperationsState::new(value(1, 8), 4);

    let applied = state
        .resolve_update(RunStatus::Running, &request, 10, |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: value(2, 8),
                result: value(3, 8),
            })
        })
        .expect("update applies");

    assert!(!applied.replayed());
    assert_eq!(applied.receipt().status(), UpdateReceiptStatus::Applied);
    assert_eq!(applied.receipt().before_state_revision(), 0);
    assert_eq!(applied.receipt().after_state_revision(), 1);
    assert_eq!(applied.receipt().run_version(), 10);
    assert_eq!(applied.receipt().result(), Some(&value(3, 8)));
    assert_eq!(state.state_revision(), 1);
    assert_eq!(state.state(), &value(2, 8));
    assert_eq!(state.last_changed_run_version(), 10);

    let original = applied.receipt().clone();
    let replayed = state
        .resolve_update(RunStatus::Failed, &request, 99, |_| {
            panic!("an authorized replay must not re-evaluate")
        })
        .expect("committed receipt replays before mutability checks");
    assert!(replayed.replayed());
    assert_eq!(replayed.receipt(), &original);
    assert_eq!(state.state_revision(), 1);
}

#[test]
fn a_chain_key_replays_its_original_receipt_after_head_resolution_changes() {
    let chain = RunId::generate();
    let original_head = RunId::generate();
    let later_head = RunId::generate();
    let scope = OperationScope::new(
        TenantId::generate(),
        NamespaceId::generate(),
        OperationAddress::Chain(chain),
    );
    let original = request(
        scope.clone(),
        original_head,
        "increment",
        "chain-update",
        9,
        None,
    );
    let retry = request(scope, later_head, "increment", "chain-update", 9, None);
    let mut state = DeclaredOperationsState::new(value(1, 8), 1);

    let applied = state
        .resolve_update(RunStatus::Running, &original, 2, |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: value(2, 8),
                result: value(2, 8),
            })
        })
        .expect("chain update applies");
    let replayed = state
        .resolve_update(RunStatus::Running, &retry, 3, |_| {
            panic!("a chain replay must not apply at the newer head")
        })
        .expect("chain receipt replays across rollover");

    assert!(replayed.replayed());
    assert_eq!(replayed.receipt(), applied.receipt());
    assert_eq!(replayed.receipt().resolved_run_id(), original_head);
    assert_ne!(replayed.receipt().resolved_run_id(), later_head);
    assert_eq!(state.state_revision(), 1);
}

#[test]
fn same_key_with_changed_content_or_operation_conflicts_without_mutation() {
    let run = RunId::generate();
    let scope = scope(run);
    let first = request(scope.clone(), run, "increment", "same-key", 1, None);
    let mut state = DeclaredOperationsState::new(value(1, 8), 1);
    state
        .resolve_update(RunStatus::Waiting, &first, 2, |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: value(2, 8),
                result: value(3, 8),
            })
        })
        .expect("first request applies");
    let before = state.clone();

    for changed in [
        request(scope.clone(), run, "increment", "same-key", 2, None),
        request(scope, run, "decrement", "same-key", 1, None),
    ] {
        assert_eq!(
            state.resolve_update(RunStatus::Running, &changed, 3, |_| {
                panic!("a conflicting key must not evaluate")
            }),
            Err(OperationError::UpdateKeyReused)
        );
        assert_eq!(state, before);
    }
}

#[test]
fn revision_conflict_is_a_stable_rejected_receipt_and_does_not_evaluate() {
    let run = RunId::generate();
    let scope = scope(run);
    let rejected_request = request(scope.clone(), run, "increment", "stale", 4, Some(2));
    let mut state = DeclaredOperationsState::new(value(1, 8), 1);

    let rejected = state
        .resolve_update(RunStatus::Running, &rejected_request, 5, |_| {
            panic!("revision is checked before evaluation")
        })
        .expect("business conflict is retained");
    assert_eq!(rejected.receipt().status(), UpdateReceiptStatus::Rejected);
    assert_eq!(
        rejected.receipt().rejection(),
        Some(&UpdateRejection::RevisionConflict {
            expected: 2,
            actual: 0,
        })
    );
    assert_eq!(rejected.receipt().before_state_revision(), 0);
    assert_eq!(rejected.receipt().after_state_revision(), 0);
    assert_eq!(state.state_revision(), 0);

    let advancing = request(scope, run, "increment", "fresh", 5, Some(0));
    state
        .resolve_update(RunStatus::Running, &advancing, 6, |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: value(2, 8),
                result: value(2, 8),
            })
        })
        .expect("independent update applies");
    let replayed = state
        .resolve_update(RunStatus::Failed, &rejected_request, 7, |_| {
            panic!("rejection replay must not evaluate")
        })
        .expect("original rejection replays");
    assert!(replayed.replayed());
    assert_eq!(replayed.receipt(), rejected.receipt());
    assert_eq!(state.state_revision(), 1);
}

#[test]
fn equal_expected_revisions_allow_one_update_and_reject_the_later_one() {
    let run = RunId::generate();
    let scope = scope(run);
    let first = request(scope.clone(), run, "increment", "first", 1, Some(0));
    let second = request(scope, run, "increment", "second", 2, Some(0));
    let mut state = DeclaredOperationsState::new(value(1, 8), 1);

    state
        .resolve_update(RunStatus::Running, &first, 2, |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: value(2, 8),
                result: value(2, 8),
            })
        })
        .expect("first serialized request applies");
    let conflict = state
        .resolve_update(RunStatus::Running, &second, 3, |_| {
            panic!("the stale revision must be rejected before evaluation")
        })
        .expect("revision conflict is retained");

    assert_eq!(state.state_revision(), 1);
    assert_eq!(state.state(), &value(2, 8));
    assert_eq!(conflict.receipt().status(), UpdateReceiptStatus::Rejected);
    assert_eq!(
        conflict.receipt().rejection(),
        Some(&UpdateRejection::RevisionConflict {
            expected: 0,
            actual: 1,
        })
    );
    assert_eq!(state.receipts().count(), 2);
}

#[test]
fn validator_rejection_is_durable_without_changing_business_state() {
    let run = RunId::generate();
    let request = request(scope(run), run, "withdraw", "denied", 8, None);
    let mut state = DeclaredOperationsState::new(value(1, 8), 3);
    let before_value = state.state().clone();

    let resolution = state
        .resolve_update(RunStatus::Waiting, &request, 4, |_| {
            Ok(UpdateEvaluation::Rejected(
                UpdateRejection::ValidatorRejected,
            ))
        })
        .expect("validator decision is retained");

    assert_eq!(resolution.receipt().status(), UpdateReceiptStatus::Rejected);
    assert_eq!(
        resolution.receipt().rejection(),
        Some(&UpdateRejection::ValidatorRejected)
    );
    assert_eq!(state.state(), &before_value);
    assert_eq!(state.state_revision(), 0);
    assert_eq!(state.last_changed_run_version(), 3);
}

#[test]
fn new_updates_are_refused_outside_running_or_waiting_without_a_receipt() {
    let run = RunId::generate();
    let request = request(scope(run), run, "increment", "blocked", 1, None);
    for status in [
        RunStatus::Paused,
        RunStatus::Cancelling,
        RunStatus::Compensating,
        RunStatus::ManualReview,
        RunStatus::Succeeded,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::TimedOut,
        RunStatus::Compensated,
        RunStatus::Partial,
        RunStatus::CompensationFailed,
    ] {
        let mut state = DeclaredOperationsState::new(value(1, 8), 1);
        assert_eq!(
            state.resolve_update(status, &request, 2, |_| {
                panic!("ineligible run must not evaluate")
            }),
            Err(OperationError::RunNotMutable(status))
        );
        assert_eq!(state.receipts().count(), 0);
        assert_eq!(state.state_revision(), 0);
    }
}

#[test]
fn evaluator_failure_leaves_no_state_or_receipt_to_fake_a_commit() {
    let run = RunId::generate();
    let request = request(scope(run), run, "increment", "transient", 1, None);
    let mut state = DeclaredOperationsState::new(value(1, 8), 1);
    let before = state.clone();

    assert_eq!(
        state.resolve_update(RunStatus::Running, &request, 2, |_| {
            Err(OperationError::EvaluationFailed)
        }),
        Err(OperationError::EvaluationFailed)
    );
    assert_eq!(state, before);
}

#[test]
fn an_update_cannot_replace_the_pinned_state_schema() {
    let run = RunId::generate();
    let request = request(scope(run), run, "increment", "wrong-schema", 1, None);
    let mut state = DeclaredOperationsState::new(value(1, 8), 1);
    let before = state.clone();

    assert_eq!(
        state.resolve_update(RunStatus::Running, &request, 2, |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: value_with_schema(2, 241, 8),
                result: value(2, 8),
            })
        }),
        Err(OperationError::StateSchemaChanged)
    );
    assert_eq!(state, before);
}

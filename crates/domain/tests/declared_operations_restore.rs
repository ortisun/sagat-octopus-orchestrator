//! Restoration checks for declared operation state persisted by adapters.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{
    DeclaredOperationsState, OperationAddress, OperationError, OperationScope, OperationValue,
    RunId, RunStatus, UpdateEvaluation, UpdateKey, UpdateReceipt, UpdateReceiptStatus,
    UpdateRequest,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::{ActivityName, Digest};

fn value(marker: u8) -> OperationValue {
    OperationValue::new(Digest::new([marker; 32]), Digest::new([250; 32]), 8)
        .expect("bounded value")
}

fn receipt(
    scope: OperationScope,
    run: RunId,
    status: UpdateReceiptStatus,
) -> Result<UpdateReceipt, OperationError> {
    UpdateReceipt::restore(
        scope,
        run,
        ActivityName::parse("balance.credit").expect("operation"),
        UpdateKey::parse("restore-key").expect("key"),
        Digest::new([9; 32]),
        status,
        None,
        Some(value(2)),
        0,
        1,
        7,
        Subject::parse("operator@example.test").expect("actor"),
    )
}

#[test]
fn restored_receipt_replays_without_evaluation() {
    let run = RunId::generate();
    let scope = OperationScope::new(
        TenantId::generate(),
        NamespaceId::generate(),
        OperationAddress::Run(run),
    );
    let stored = receipt(scope.clone(), run, UpdateReceiptStatus::Applied).expect("receipt");
    let mut state =
        DeclaredOperationsState::restore(value(2), 1, 7, [stored.clone()]).expect("stored state");
    let request = UpdateRequest::new(
        scope,
        run,
        ActivityName::parse("balance.credit").expect("operation"),
        UpdateKey::parse("restore-key").expect("key"),
        Digest::new([9; 32]),
        None,
        Subject::parse("operator@example.test").expect("actor"),
    );

    let replay = state
        .resolve_update(RunStatus::Failed, &request, 99, |_| {
            panic!("a restored receipt must replay without evaluation")
        })
        .expect("replay");

    assert!(replay.replayed());
    assert_eq!(replay.receipt(), &stored);
    assert_eq!(state.state_revision(), 1);
}

#[test]
fn restoration_rejects_inconsistent_receipts() {
    let run = RunId::generate();
    let scope = OperationScope::new(
        TenantId::generate(),
        NamespaceId::generate(),
        OperationAddress::Run(run),
    );

    assert_eq!(
        receipt(scope, run, UpdateReceiptStatus::Rejected),
        Err(OperationError::InvalidStoredOperation)
    );
}

#[test]
fn durable_rejection_keeps_the_current_run_version() {
    let run = RunId::generate();
    let scope = OperationScope::new(
        TenantId::generate(),
        NamespaceId::generate(),
        OperationAddress::Run(run),
    );
    let request = UpdateRequest::new(
        scope,
        run,
        ActivityName::parse("balance.credit").expect("operation"),
        UpdateKey::parse("rejected-key").expect("key"),
        Digest::new([7; 32]),
        Some(3),
        Subject::parse("operator@example.test").expect("actor"),
    );
    let mut state = DeclaredOperationsState::new(value(1), 11);

    let rejected = state
        .resolve_update_at(RunStatus::Running, &request, 11, 12, |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: value(2),
                result: value(3),
            })
        })
        .expect("revision rejection");

    assert_eq!(rejected.receipt().status(), UpdateReceiptStatus::Rejected);
    assert_eq!(rejected.receipt().run_version(), 11);
    assert_eq!(state.last_changed_run_version(), 11);
}

#[test]
fn exhausted_run_version_still_allows_rejection_and_replay_but_not_apply() {
    let run = RunId::generate();
    let scope = OperationScope::new(
        TenantId::generate(),
        NamespaceId::generate(),
        OperationAddress::Run(run),
    );
    let rejected_request = UpdateRequest::new(
        scope.clone(),
        run,
        ActivityName::parse("balance.credit").expect("operation"),
        UpdateKey::parse("max-rejected").expect("key"),
        Digest::new([17; 32]),
        Some(1),
        Subject::parse("operator@example.test").expect("actor"),
    );
    let mut state = DeclaredOperationsState::new(value(1), u64::MAX);

    let rejected = state
        .resolve_update_advancing(RunStatus::Running, &rejected_request, u64::MAX, |_| {
            panic!("revision conflict does not evaluate")
        })
        .expect("durable rejection");
    assert_eq!(rejected.receipt().status(), UpdateReceiptStatus::Rejected);
    assert_eq!(rejected.receipt().run_version(), u64::MAX);
    let replay = state
        .resolve_update_advancing(RunStatus::Failed, &rejected_request, u64::MAX, |_| {
            panic!("receipt replay does not evaluate")
        })
        .expect("receipt replay");
    assert!(replay.replayed());

    let applied_request = UpdateRequest::new(
        scope,
        run,
        ActivityName::parse("balance.credit").expect("operation"),
        UpdateKey::parse("max-applied").expect("key"),
        Digest::new([18; 32]),
        None,
        Subject::parse("operator@example.test").expect("actor"),
    );
    assert_eq!(
        state.resolve_update_advancing(RunStatus::Running, &applied_request, u64::MAX, |_| Ok(
            UpdateEvaluation::Applied {
                new_state: value(2),
                result: value(3),
            }
        ),),
        Err(OperationError::RevisionExhausted)
    );
    assert_eq!(state.state_revision(), 0);
    assert_eq!(state.receipts().count(), 1);
}

//! Pure continuation-chain invariants.
#![allow(clippy::expect_used)]

use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ChainError, ChainFinalOutcome, ChainPolicy, ChainScope, ChildChainLink, ChildObservation,
    ContinuationProposal, ContinuationTransfer, ExecutionChain, ExecutionInstanceId,
    MAX_CHAIN_SEGMENTS, Quiescence, QuiescenceBlocker, RunId, SegmentCompletionKind, SegmentIndex,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::{Digest, NodeId};
use time::OffsetDateTime;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("valid timestamp")
}

fn subject(value: &str) -> Subject {
    Subject::parse(value).expect("valid subject")
}

fn scope() -> ChainScope {
    ChainScope::new(TenantId::generate(), NamespaceId::generate())
}

fn transfer(marker: u8) -> ContinuationTransfer {
    ContinuationTransfer::new(
        Digest::new([marker; 32]),
        Some(Digest::new([marker + 1; 32])),
    )
}

fn chain_with_limit(max_segments: u16) -> ExecutionChain {
    ExecutionChain::start(
        scope(),
        RunId::generate(),
        WorkflowVersionId::generate(),
        subject("chain-initiator"),
        at(10_000),
        ChainPolicy::new(max_segments).expect("valid policy"),
    )
}

fn proposal(chain: &ExecutionChain, successor: RunId, marker: u8) -> ContinuationProposal {
    ContinuationProposal::new(
        chain.scope(),
        chain.current_run_id(),
        chain.revision(),
        successor,
        chain.definition_version(),
        chain.original_deadline(),
        transfer(marker),
        Quiescence::ready(),
    )
}

#[test]
fn policy_and_segment_indices_are_bounded() {
    assert_eq!(ChainPolicy::new(0), Err(ChainError::InvalidSegmentLimit));
    assert_eq!(
        ChainPolicy::new(MAX_CHAIN_SEGMENTS + 1),
        Err(ChainError::InvalidSegmentLimit)
    );
    assert_eq!(
        ChainPolicy::new(MAX_CHAIN_SEGMENTS)
            .expect("global ceiling is valid")
            .max_segments(),
        10_000
    );
    assert_eq!(SegmentIndex::new(9_999).expect("last index").get(), 9_999);
    assert_eq!(
        SegmentIndex::new(10_000),
        Err(ChainError::InvalidSegmentIndex)
    );
}

#[test]
fn the_global_segment_ceiling_stops_before_index_ten_thousand() {
    let mut chain = chain_with_limit(MAX_CHAIN_SEGMENTS);

    for marker in 1..MAX_CHAIN_SEGMENTS {
        let next = RunId::generate();
        let decision = chain
            .continue_as_new(proposal(&chain, next, (marker % 250) as u8))
            .expect("segment remains inside configured limit");
        assert_eq!(decision.successor_index().get(), marker);
    }

    let before = chain.clone();
    let rejected = chain.continue_as_new(proposal(&chain, RunId::generate(), 7));
    assert_eq!(rejected, Err(ChainError::SegmentLimitExceeded));
    assert_eq!(chain, before);
}

#[test]
fn handoff_advances_one_head_and_retains_lineage_and_transfer() {
    let mut chain = chain_with_limit(3);
    let initial = chain.current_run_id();
    let successor = RunId::generate();
    let selected = transfer(4);
    let decision = chain
        .continue_as_new(ContinuationProposal::new(
            chain.scope(),
            initial,
            chain.revision(),
            successor,
            chain.definition_version(),
            chain.original_deadline(),
            selected.clone(),
            Quiescence::ready(),
        ))
        .expect("quiescent handoff");

    assert!(!decision.replayed());
    assert_eq!(decision.predecessor(), initial);
    assert_eq!(decision.successor(), successor);
    assert_eq!(decision.successor_index().get(), 1);
    assert_eq!(chain.current_run_id(), successor);
    assert_eq!(chain.segment_count(), 2);
    assert_eq!(chain.revision(), 1);
    assert_eq!(chain.segments()[0].successor(), Some(successor));
    assert_eq!(
        chain.segments()[0].completion_kind(),
        Some(SegmentCompletionKind::Continued)
    );
    assert_eq!(chain.segments()[0].transfer(), Some(&selected));
    assert_eq!(chain.segments()[1].predecessor(), Some(initial));
}

#[test]
fn every_unresolved_obligation_blocks_handoff_without_mutation() {
    let blockers = [
        QuiescenceBlocker::SiblingWork,
        QuiescenceBlocker::Child,
        QuiescenceBlocker::Lease,
        QuiescenceBlocker::Timer,
        QuiescenceBlocker::Subscription,
        QuiescenceBlocker::Approval,
        QuiescenceBlocker::UnknownEffect,
        QuiescenceBlocker::Compensation,
    ];

    for blocker in blockers {
        let mut chain = chain_with_limit(2);
        let before = chain.clone();
        let mut handoff = proposal(&chain, RunId::generate(), 1);
        handoff.set_quiescence(Quiescence::blocked(blocker));

        assert_eq!(
            chain.continue_as_new(handoff),
            Err(ChainError::NotQuiescent(blocker))
        );
        assert_eq!(chain, before);
    }
}

#[test]
fn current_head_and_revision_are_compare_and_swap_inputs() {
    let mut chain = chain_with_limit(3);
    let before = chain.clone();
    let mut stale_head = proposal(&chain, RunId::generate(), 1);
    stale_head.set_expected_head(RunId::generate());
    assert!(matches!(
        chain.continue_as_new(stale_head),
        Err(ChainError::HeadChanged { .. })
    ));
    assert_eq!(chain, before);

    let mut stale_revision = proposal(&chain, RunId::generate(), 1);
    stale_revision.set_expected_revision(chain.revision() + 1);
    assert_eq!(
        chain.continue_as_new(stale_revision),
        Err(ChainError::RevisionChanged {
            expected: 1,
            actual: 0,
        })
    );
    assert_eq!(chain, before);
}

#[test]
fn duplicate_handoff_replays_the_first_successor_and_conflicts_on_changed_content() {
    let mut chain = chain_with_limit(4);
    let old_head = chain.current_run_id();
    let old_revision = chain.revision();
    let successor = RunId::generate();
    let first = proposal(&chain, successor, 9);
    chain
        .continue_as_new(first.clone())
        .expect("first handoff commits");

    let replay = chain
        .continue_as_new(first)
        .expect("lost response replays the committed handoff");
    assert!(replay.replayed());
    assert_eq!(replay.predecessor(), old_head);
    assert_eq!(replay.successor(), successor);
    assert_eq!(chain.segment_count(), 2);
    assert_eq!(chain.revision(), old_revision + 1);

    let changed = ContinuationProposal::new(
        chain.scope(),
        old_head,
        old_revision,
        successor,
        chain.definition_version(),
        chain.original_deadline(),
        transfer(10),
        Quiescence::ready(),
    );
    assert_eq!(
        chain.continue_as_new(changed),
        Err(ChainError::ContinuationConflict)
    );

    let other_successor = ContinuationProposal::new(
        chain.scope(),
        old_head,
        old_revision,
        RunId::generate(),
        chain.definition_version(),
        chain.original_deadline(),
        transfer(9),
        Quiescence::ready(),
    );
    assert!(matches!(
        chain.continue_as_new(other_successor),
        Err(ChainError::HeadChanged { current }) if current == successor
    ));
}

#[test]
fn cancellation_committed_before_handoff_prevents_a_successor() {
    let mut chain = chain_with_limit(3);
    let expected_head = chain.current_run_id();
    let expected_revision = chain.revision();
    let cancel = chain
        .request_cancel(expected_head, expected_revision)
        .expect("cancel latch commits");
    assert!(cancel.changed());
    assert_eq!(cancel.target_run_id(), expected_head);

    let mut stale_handoff = proposal(&chain, RunId::generate(), 2);
    stale_handoff.set_expected_revision(expected_revision);
    assert_eq!(
        chain.continue_as_new(stale_handoff),
        Err(ChainError::CancellationRequested)
    );
    assert_eq!(chain.segment_count(), 1);

    let replay = chain
        .request_cancel(expected_head, expected_revision)
        .expect("cancel is idempotent");
    assert!(!replay.changed());
}

#[test]
fn cancellation_after_handoff_targets_the_new_head_and_latches_future_handoffs() {
    let mut chain = chain_with_limit(3);
    let predecessor = chain.current_run_id();
    let predecessor_revision = chain.revision();
    let successor = RunId::generate();
    let committed = proposal(&chain, successor, 3);
    chain
        .continue_as_new(committed.clone())
        .expect("handoff wins first");

    let cancel = chain
        .request_cancel(successor, chain.revision())
        .expect("cancel targets current head");
    assert_eq!(cancel.target_run_id(), successor);
    let replay = chain
        .continue_as_new(committed)
        .expect("retry still observes the earlier handoff");
    assert!(replay.replayed());
    assert_eq!(replay.predecessor(), predecessor);
    assert_eq!(chain.revision(), predecessor_revision + 2);
    assert_eq!(
        chain.continue_as_new(proposal(&chain, RunId::generate(), 4)),
        Err(ChainError::CancellationRequested)
    );
}

#[test]
fn an_existing_run_cannot_be_inserted_as_another_successor() {
    let mut chain = chain_with_limit(3);
    let initial = chain.current_run_id();
    chain
        .continue_as_new(proposal(&chain, RunId::generate(), 1))
        .expect("first successor is unique");
    let before = chain.clone();

    assert_eq!(
        chain.continue_as_new(proposal(&chain, initial, 2)),
        Err(ChainError::DuplicateRun)
    );
    assert_eq!(chain, before);
}

#[test]
fn scope_version_and_original_deadline_cannot_change_at_handoff() {
    let mut chain = chain_with_limit(2);
    let before = chain.clone();

    let mut wrong_scope = proposal(&chain, RunId::generate(), 1);
    wrong_scope.set_scope(scope());
    assert_eq!(
        chain.continue_as_new(wrong_scope),
        Err(ChainError::ScopeMismatch)
    );

    let mut wrong_version = proposal(&chain, RunId::generate(), 1);
    wrong_version.set_definition_version(WorkflowVersionId::generate());
    assert_eq!(
        chain.continue_as_new(wrong_version),
        Err(ChainError::DefinitionVersionChanged)
    );

    let mut renewed_deadline = proposal(&chain, RunId::generate(), 1);
    renewed_deadline.set_deadline(at(20_000));
    assert_eq!(
        chain.continue_as_new(renewed_deadline),
        Err(ChainError::DeadlineChanged)
    );
    assert_eq!(chain, before);
}

#[test]
fn finalization_is_first_commit_and_a_terminal_chain_never_reopens() {
    let mut chain = chain_with_limit(2);
    let head = chain.current_run_id();
    let revision = chain.revision();
    let first = chain
        .finish(head, revision, ChainFinalOutcome::Succeeded)
        .expect("final outcome commits");
    assert!(!first.replayed());
    assert_eq!(chain.final_outcome(), Some(ChainFinalOutcome::Succeeded));
    assert_eq!(
        chain.segments()[0].completion_kind(),
        Some(SegmentCompletionKind::Final)
    );

    let replay = chain
        .finish(head, revision, ChainFinalOutcome::Succeeded)
        .expect("identical finalization replays");
    assert!(replay.replayed());
    assert_eq!(
        chain.finish(head, revision, ChainFinalOutcome::Failed),
        Err(ChainError::FinalOutcomeConflict)
    );
    assert_eq!(
        chain.continue_as_new(proposal(&chain, RunId::generate(), 1)),
        Err(ChainError::ChainFinal)
    );
}

#[test]
fn child_link_rejects_cross_tenant_and_cross_namespace_chains() {
    let parent_scope = scope();
    let parent_run = RunId::generate();
    let parent_instance =
        ExecutionInstanceId::root(NodeId::parse("wait-child").expect("valid definition node"));
    let child = chain_with_limit(3);

    assert_eq!(
        ChildChainLink::new(parent_scope, parent_run, parent_instance.clone(), &child),
        Err(ChainError::ScopeMismatch)
    );

    let child_scope = child.scope();
    let same_tenant_other_namespace =
        ChainScope::new(child_scope.tenant_id(), NamespaceId::generate());
    assert_eq!(
        ChildChainLink::new(
            same_tenant_other_namespace,
            parent_run,
            parent_instance,
            &child,
        ),
        Err(ChainError::ScopeMismatch)
    );
}

#[test]
fn child_link_waits_across_continued_segments_and_settles_final_once() {
    let mut child = chain_with_limit(3);
    let mut link = ChildChainLink::new(
        child.scope(),
        RunId::generate(),
        ExecutionInstanceId::root(NodeId::parse("child").expect("valid node")),
        &child,
    )
    .expect("same-scope child link");

    let successor = RunId::generate();
    child
        .continue_as_new(proposal(&child, successor, 3))
        .expect("child continues");
    assert_eq!(
        link.observe(&child).expect("linked chain"),
        ChildObservation::Waiting { head: successor }
    );
    assert!(!link.is_settled());

    child
        .finish(successor, child.revision(), ChainFinalOutcome::Succeeded)
        .expect("child reaches final outcome");
    assert_eq!(
        link.observe(&child)
            .expect("final outcome becomes available"),
        ChildObservation::Final(ChainFinalOutcome::Succeeded)
    );
    assert!(
        link.settle_final(ChainFinalOutcome::Succeeded)
            .expect("parent applies final outcome")
    );
    assert!(
        !link
            .settle_final(ChainFinalOutcome::Succeeded)
            .expect("duplicate settlement is a no-op")
    );
    assert_eq!(
        link.settle_final(ChainFinalOutcome::Failed),
        Err(ChainError::FinalOutcomeConflict)
    );
}

#[test]
fn child_cancellation_latch_blocks_handoff_even_before_chain_cancel_is_applied() {
    let child = chain_with_limit(3);
    let mut link = ChildChainLink::new(
        child.scope(),
        RunId::generate(),
        ExecutionInstanceId::root(NodeId::parse("child").expect("valid node")),
        &child,
    )
    .expect("same-scope child link");
    assert!(link.request_cancel());
    assert!(!link.request_cancel());
    assert_eq!(
        link.permit_handoff(&child),
        Err(ChainError::ChildCancellationRequested)
    );
}

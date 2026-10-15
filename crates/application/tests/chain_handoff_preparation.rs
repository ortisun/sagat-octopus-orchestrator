use sagat_application::chain_progress::{
    ChainHandoffCommand, ContinuationPreparationError, EvaluatedChainHandoffCommand,
};
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{ChainScope, ContinuationTransfer, RunId};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::{Digest, NodeId};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

fn digest(value: &[u8]) -> Digest {
    Digest::new(Sha256::digest(value).into())
}

fn handoff(input: Digest, state: Option<Digest>) -> ChainHandoffCommand {
    let tenant = TenantId::new(Uuid::now_v7());
    let namespace = NamespaceId::new(Uuid::now_v7());
    let head = RunId::generate();
    let at = OffsetDateTime::UNIX_EPOCH;
    ChainHandoffCommand::new(
        ChainScope::new(tenant, namespace),
        head,
        head,
        3,
        9,
        RunId::generate(),
        ContinuationTransfer::new(input, state),
        SchedulerAuthority::for_run("scheduler", head, 2),
        Subject::engine(),
        None,
        at,
        AuditRecord::engine(
            tenant,
            AuditAction::RunContinued,
            ResourceType::Run,
            head.to_string(),
            "continuation selected",
            None,
            at,
        ),
        Uuid::now_v7(),
        Uuid::now_v7(),
    )
}

#[test]
fn evaluated_handoff_binds_bytes_digests_and_state_revision() {
    let input = br#"{"segment":2}"#.to_vec();
    let state = b"7".to_vec();
    let prepared = EvaluatedChainHandoffCommand::new(
        handoff(digest(&input), Some(digest(&state))),
        NodeId::parse("rollover").expect("node"),
        input.clone(),
        Some(state.clone()),
        Some(4),
    )
    .expect("prepared handoff");

    assert_eq!(prepared.input(), input);
    assert_eq!(prepared.state(), Some(state.as_slice()));
    assert_eq!(prepared.expected_state_revision(), Some(4));
    assert_eq!(prepared.boundary().as_str(), "rollover");
}

#[test]
fn evaluated_handoff_rejects_changed_content_or_unpinned_state() {
    let input = br#"{"segment":2}"#.to_vec();
    assert_eq!(
        EvaluatedChainHandoffCommand::new(
            handoff(Digest::new([1; 32]), None),
            NodeId::parse("rollover").expect("node"),
            input.clone(),
            None,
            None,
        ),
        Err(ContinuationPreparationError::DigestMismatch)
    );
    assert_eq!(
        EvaluatedChainHandoffCommand::new(
            handoff(digest(&input), Some(digest(b"7"))),
            NodeId::parse("rollover").expect("node"),
            input,
            Some(b"7".to_vec()),
            None,
        ),
        Err(ContinuationPreparationError::StateRevisionMismatch)
    );
}

//! Pure eligibility, settlement and replay semantics for human approvals.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{
    ApprovalDecision, ApprovalDecisionCommand, ApprovalError, ApprovalPolicy, ApprovalReasonCode,
    ApprovalReceiptId, ApprovalRequest, ApprovalRequestId, ApprovalStatus, DecisionKey,
    ExecutionInstanceId, RunId,
};
use sagat_domain::identity::{NamespaceId, Principal, Role, Subject, TenantId};
use sagat_domain::workflow::{Digest, NodeId};
use time::OffsetDateTime;

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("valid timestamp")
}

fn subject(value: &str) -> Subject {
    Subject::parse(value).expect("valid subject")
}

fn policy(subjects: &[&str], roles: &[Role], exclude_initiator: bool) -> ApprovalPolicy {
    ApprovalPolicy::with_initiator_exclusion(
        subjects.iter().map(|value| subject(value)).collect(),
        roles.to_vec(),
        exclude_initiator,
    )
    .expect("valid policy")
}

fn principal(tenant_id: TenantId, actor: &str, role: Role) -> Principal {
    Principal::new(tenant_id, subject(actor), role)
}

fn command(key: &str, marker: u8, decision: ApprovalDecision) -> ApprovalDecisionCommand {
    ApprovalDecisionCommand::new(
        DecisionKey::parse(key).expect("valid key"),
        Digest::new([marker; 32]),
        decision,
        Some(ApprovalReasonCode::parse("POLICY_CONFIRMED").expect("valid reason code")),
    )
}

fn request(tenant_id: TenantId, initiator: Subject, policy: ApprovalPolicy) -> ApprovalRequest {
    ApprovalRequest::open(
        ApprovalRequestId::generate(),
        tenant_id,
        NamespaceId::generate(),
        RunId::generate(),
        ExecutionInstanceId::root(NodeId::parse("manager-approval").expect("valid node")),
        policy,
        initiator,
        7,
        at(100),
    )
}

#[test]
fn policy_is_non_empty_unique_bounded_and_limited_to_human_decision_roles() {
    assert_eq!(
        ApprovalPolicy::new(Vec::new(), Vec::new()),
        Err(ApprovalError::EmptyPolicy)
    );
    assert_eq!(
        ApprovalPolicy::new(vec![subject("same"), subject("same")], Vec::new()),
        Err(ApprovalError::DuplicateEligibleSubject)
    );
    assert_eq!(
        ApprovalPolicy::new(Vec::new(), vec![Role::Admin, Role::Admin]),
        Err(ApprovalError::DuplicateEligibleRole)
    );
    for role in [Role::Viewer, Role::Worker, Role::Runner] {
        assert_eq!(
            ApprovalPolicy::new(Vec::new(), vec![role]),
            Err(ApprovalError::IneligiblePolicyRole(role))
        );
    }
    assert_eq!(
        ApprovalPolicy::new(
            (0..33)
                .map(|index| subject(&format!("approver-{index}")))
                .collect(),
            Vec::new(),
        ),
        Err(ApprovalError::TooManyPolicyEntries)
    );

    let default_policy =
        ApprovalPolicy::new(vec![subject("default-approver")], Vec::new()).expect("valid policy");
    assert!(default_policy.excludes_initiator());

    let policy = policy(&["zeta", "alpha"], &[Role::Operator, Role::Admin], true);
    assert_eq!(
        policy.subjects().map(Subject::as_str).collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
    assert_eq!(
        policy.roles().collect::<Vec<_>>(),
        vec![Role::Admin, Role::Operator]
    );
    assert!(policy.excludes_initiator());
}

#[test]
fn decision_keys_and_reason_codes_are_machine_values_with_finite_bounds() {
    assert!(DecisionKey::parse("approval:request/01").is_ok());
    assert!(DecisionKey::parse("").is_err());
    assert!(DecisionKey::parse("contains space").is_err());
    assert!(DecisionKey::parse(&"k".repeat(129)).is_err());

    assert!(ApprovalReasonCode::parse("RISK_ACCEPTED-01").is_ok());
    for invalid in ["", "free text", "line\nbreak", "não-ascii"] {
        assert!(ApprovalReasonCode::parse(invalid).is_err(), "{invalid:?}");
    }
    assert!(ApprovalReasonCode::parse(&"R".repeat(65)).is_err());

    assert_eq!(ApprovalError::Forbidden.reason(), "FORBIDDEN");
    assert_eq!(
        ApprovalError::InitiatorCannotDecide.reason(),
        "INITIATOR_CANNOT_DECIDE"
    );
    assert_eq!(
        ApprovalError::IdempotencyKeyReused.reason(),
        "IDEMPOTENCY_KEY_REUSED"
    );
    assert_eq!(ApprovalError::AlreadyDecided.reason(), "ALREADY_DECIDED");
}

#[test]
fn an_eligible_subject_commits_one_approval_receipt() {
    let tenant = TenantId::generate();
    let actor = principal(tenant, "eligible-person", Role::Operator);
    let mut approval = request(
        tenant,
        subject("initiator"),
        policy(&["eligible-person"], &[], true),
    );
    let receipt_id = ApprovalReceiptId::generate();
    let resolution = approval
        .decide(
            &actor,
            receipt_id,
            &command("decision-1", 1, ApprovalDecision::Approve),
            at(90),
            8,
        )
        .expect("eligible decision applies");

    assert!(!resolution.replayed());
    assert_eq!(approval.status(), ApprovalStatus::Approved);
    assert_eq!(approval.outcome_sequence(), Some(8));
    assert_eq!(approval.settled_at(), Some(at(90)));
    assert_eq!(approval.winning_receipt_id(), Some(receipt_id));
    assert_eq!(resolution.receipt().actor(), actor.subject());
    assert_eq!(resolution.receipt().decision(), ApprovalDecision::Approve);
    assert_eq!(resolution.receipt().decided_at(), at(90));
    assert_eq!(resolution.receipt().outcome_sequence(), 8);
}

#[test]
fn current_permission_tenant_eligibility_and_initiator_exclusion_are_checked_before_mutation() {
    let tenant = TenantId::generate();
    let mut approval = request(
        tenant,
        subject("starter"),
        policy(&["starter", "listed-but-revoked"], &[Role::Developer], true),
    );
    let before = approval.clone();
    let command = command("decision-1", 1, ApprovalDecision::Approve);

    for (caller, expected) in [
        (
            principal(tenant, "starter", Role::Admin),
            ApprovalError::InitiatorCannotDecide,
        ),
        (
            principal(tenant, "listed-but-revoked", Role::Viewer),
            ApprovalError::Forbidden,
        ),
        (
            principal(tenant, "not-listed", Role::Operator),
            ApprovalError::Forbidden,
        ),
        (
            principal(TenantId::generate(), "foreign", Role::Admin),
            ApprovalError::Forbidden,
        ),
    ] {
        assert_eq!(
            approval.decide(&caller, ApprovalReceiptId::generate(), &command, at(90), 8,),
            Err(expected)
        );
        assert_eq!(approval, before);
    }
}

#[test]
fn identical_retry_replays_the_receipt_but_changed_content_or_key_conflicts() {
    let tenant = TenantId::generate();
    let actor = principal(tenant, "approver", Role::Admin);
    let mut approval = request(
        tenant,
        subject("starter"),
        policy(&["approver"], &[Role::Admin], true),
    );
    let original_command = command("same-key", 1, ApprovalDecision::Reject);
    let original = approval
        .decide(
            &actor,
            ApprovalReceiptId::generate(),
            &original_command,
            at(90),
            8,
        )
        .expect("first response wins");

    let replaying_actor = principal(tenant, "another-admin", Role::Admin);
    let replayed = approval
        .decide(
            &replaying_actor,
            ApprovalReceiptId::generate(),
            &original_command,
            at(99),
            20,
        )
        .expect("identical response replays");
    assert!(replayed.replayed());
    assert_eq!(replayed.receipt(), original.receipt());
    assert_eq!(replayed.receipt().actor(), actor.subject());
    assert_eq!(approval.status(), ApprovalStatus::Rejected);

    for changed in [
        command("same-key", 2, ApprovalDecision::Reject),
        command("same-key", 1, ApprovalDecision::Approve),
    ] {
        assert_eq!(
            approval.decide(&actor, ApprovalReceiptId::generate(), &changed, at(99), 20,),
            Err(ApprovalError::IdempotencyKeyReused)
        );
    }
    assert_eq!(
        approval.decide(
            &actor,
            ApprovalReceiptId::generate(),
            &command("other-key", 3, ApprovalDecision::Reject),
            at(99),
            20,
        ),
        Err(ApprovalError::AlreadyDecided)
    );
}

#[test]
fn an_explicit_policy_may_allow_the_initiator_to_decide() {
    let tenant = TenantId::generate();
    let initiator = subject("starter");
    let actor = Principal::new(tenant, initiator.clone(), Role::Admin);
    let mut approval = request(tenant, initiator, policy(&["starter"], &[], false));

    approval
        .decide(
            &actor,
            ApprovalReceiptId::generate(),
            &command("self-approved", 1, ApprovalDecision::Approve),
            at(90),
            8,
        )
        .expect("explicit policy allows the initiator");
    assert_eq!(approval.status(), ApprovalStatus::Approved);
}

#[test]
fn replay_rechecks_current_authorization_and_policy_membership() {
    let tenant = TenantId::generate();
    let actor = principal(tenant, "approver", Role::Admin);
    let mut approval = request(tenant, subject("starter"), policy(&["approver"], &[], true));
    let command = command("decision", 1, ApprovalDecision::Approve);
    approval
        .decide(&actor, ApprovalReceiptId::generate(), &command, at(90), 8)
        .expect("first decision applies");

    let revoked = principal(tenant, "approver", Role::Viewer);
    assert_eq!(
        approval.decide(&revoked, ApprovalReceiptId::generate(), &command, at(91), 9,),
        Err(ApprovalError::Forbidden)
    );
}

#[test]
fn a_response_after_the_deadline_wins_when_expiry_has_not_committed() {
    let tenant = TenantId::generate();
    let actor = principal(tenant, "approver", Role::Developer);
    let mut approval = request(
        tenant,
        subject("starter"),
        policy(&[], &[Role::Developer], true),
    );

    approval
        .decide(
            &actor,
            ApprovalReceiptId::generate(),
            &command("late-response", 1, ApprovalDecision::Approve),
            at(101),
            8,
        )
        .expect("first commit wins despite wall clock");
    let expiry = approval.expire(at(101), 9).expect("expiry is due");

    assert_eq!(approval.status(), ApprovalStatus::Approved);
    assert!(!expiry.changed());
    assert_eq!(expiry.status(), ApprovalStatus::Approved);
    assert_eq!(approval.outcome_sequence(), Some(8));
    assert_eq!(approval.settled_at(), Some(at(101)));
}

#[test]
fn committed_expiry_wins_over_a_later_response_and_early_expiry_is_refused() {
    let tenant = TenantId::generate();
    let actor = principal(tenant, "approver", Role::Operator);
    let mut approval = request(
        tenant,
        subject("starter"),
        policy(&[], &[Role::Operator], true),
    );

    assert_eq!(approval.expire(at(99), 8), Err(ApprovalError::ExpiryNotDue));
    let expiry = approval.expire(at(100), 8).expect("deadline is inclusive");
    assert!(expiry.changed());
    assert_eq!(expiry.status(), ApprovalStatus::Expired);
    assert_eq!(approval.outcome_sequence(), Some(8));
    assert_eq!(approval.settled_at(), Some(at(100)));
    assert_eq!(
        approval.decide(
            &actor,
            ApprovalReceiptId::generate(),
            &command("late", 1, ApprovalDecision::Approve),
            at(100),
            9,
        ),
        Err(ApprovalError::AlreadyDecided)
    );
}

#[test]
fn cancellation_settles_pending_once_and_losing_internal_transitions_are_no_ops() {
    let tenant = TenantId::generate();
    let mut approval = request(tenant, subject("starter"), policy(&["approver"], &[], true));

    let cancelled = approval.cancel(at(80), 8);
    assert!(cancelled.changed());
    assert_eq!(cancelled.status(), ApprovalStatus::Cancelled);
    assert_eq!(approval.outcome_sequence(), Some(8));
    assert_eq!(approval.settled_at(), Some(at(80)));

    let duplicate_cancel = approval.cancel(at(90), 9);
    assert!(!duplicate_cancel.changed());
    assert_eq!(duplicate_cancel.status(), ApprovalStatus::Cancelled);
    let losing_expiry = approval.expire(at(100), 10).expect("expiry is due");
    assert!(!losing_expiry.changed());
    assert_eq!(approval.outcome_sequence(), Some(8));
    assert_eq!(approval.settled_at(), Some(at(80)));
}

#[test]
fn restored_state_replays_the_winning_receipt_without_reevaluating_the_race() {
    let tenant = TenantId::generate();
    let actor = principal(tenant, "approver", Role::Operator);
    let mut approval = request(tenant, subject("starter"), policy(&["approver"], &[], true));
    let command = command("restart-safe", 1, ApprovalDecision::Approve);
    let original = approval
        .decide(&actor, ApprovalReceiptId::generate(), &command, at(90), 8)
        .expect("decision applies");
    let mut restored = approval.clone();

    let replayed = restored
        .decide(&actor, ApprovalReceiptId::generate(), &command, at(110), 20)
        .expect("restored receipt replays");

    assert!(replayed.replayed());
    assert_eq!(replayed.receipt(), original.receipt());
    assert_eq!(restored, approval);
}

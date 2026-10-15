//! Pure state transitions for one authorized human approval.
//!
//! This module snapshots eligibility and records the first committed settlement. Current namespace
//! membership, audit persistence, timers, journal entries and route scheduling remain transactional
//! boundary responsibilities and must succeed before an adapter exposes a decision receipt.

use time::OffsetDateTime;

use crate::identity::{NamespaceId, Permission, Principal, Role, Subject, TenantId};
use crate::workflow::Digest;

use super::{ExecutionInstanceId, RunId};

pub const MAX_APPROVAL_POLICY_ENTRIES: usize = 32;
pub const MAX_DECISION_KEY_BYTES: usize = 128;
pub const MAX_APPROVAL_REASON_CODE_BYTES: usize = 64;

uuid_id!(
    /// Identifier of an approval request.
    ApprovalRequestId
);
uuid_id!(
    /// Identifier of the durable receipt for the winning human decision.
    ApprovalReceiptId
);

/// Caller-chosen idempotency key for an approval decision.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecisionKey(String);

impl DecisionKey {
    /// # Errors
    /// Returns [`ApprovalError::InvalidDecisionKey`] unless the key is bounded printable ASCII
    /// without spaces.
    pub fn parse(value: &str) -> Result<Self, ApprovalError> {
        if value.is_empty()
            || value.len() > MAX_DECISION_KEY_BYTES
            || !value.chars().all(|character| character.is_ascii_graphic())
        {
            Err(ApprovalError::InvalidDecisionKey)
        } else {
            Ok(Self(value.to_owned()))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Bounded machine-readable explanation attached to a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalReasonCode(String);

impl ApprovalReasonCode {
    /// # Errors
    /// Returns [`ApprovalError::InvalidReasonCode`] for free text or an unbounded code.
    pub fn parse(value: &str) -> Result<Self, ApprovalError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_APPROVAL_REASON_CODE_BYTES
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_.:-".contains(character));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(ApprovalError::InvalidReasonCode)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Eligibility captured when an approval request opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPolicy {
    subjects: Vec<Subject>,
    roles: Vec<Role>,
    exclude_initiator: bool,
}

impl ApprovalPolicy {
    /// Builds a canonical policy snapshot with initiator exclusion enabled.
    ///
    /// # Errors
    /// Returns a typed error when the policy is empty, oversized, duplicated or includes a role
    /// that cannot make human approval decisions.
    pub fn new(subjects: Vec<Subject>, roles: Vec<Role>) -> Result<Self, ApprovalError> {
        Self::with_initiator_exclusion(subjects, roles, true)
    }

    /// Builds a canonical policy snapshot with an explicit initiator-exclusion setting.
    ///
    /// # Errors
    /// Returns a typed error when the policy is empty, oversized, duplicated or includes a role
    /// that cannot make human approval decisions.
    pub fn with_initiator_exclusion(
        mut subjects: Vec<Subject>,
        mut roles: Vec<Role>,
        exclude_initiator: bool,
    ) -> Result<Self, ApprovalError> {
        let entry_count = subjects
            .len()
            .checked_add(roles.len())
            .ok_or(ApprovalError::TooManyPolicyEntries)?;
        if entry_count == 0 {
            return Err(ApprovalError::EmptyPolicy);
        }
        if entry_count > MAX_APPROVAL_POLICY_ENTRIES {
            return Err(ApprovalError::TooManyPolicyEntries);
        }
        if let Some(role) = roles
            .iter()
            .copied()
            .find(|role| !matches!(role, Role::Admin | Role::Developer | Role::Operator))
        {
            return Err(ApprovalError::IneligiblePolicyRole(role));
        }

        subjects.sort();
        if subjects.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ApprovalError::DuplicateEligibleSubject);
        }
        roles.sort_by_key(|role| role_order(*role));
        if roles.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ApprovalError::DuplicateEligibleRole);
        }

        Ok(Self {
            subjects,
            roles,
            exclude_initiator,
        })
    }

    pub fn subjects(&self) -> impl ExactSizeIterator<Item = &Subject> {
        self.subjects.iter()
    }

    pub fn roles(&self) -> impl ExactSizeIterator<Item = Role> + '_ {
        self.roles.iter().copied()
    }

    #[must_use]
    pub const fn excludes_initiator(&self) -> bool {
        self.exclude_initiator
    }

    fn includes(&self, principal: &Principal) -> bool {
        self.subjects.contains(principal.subject()) || self.roles.contains(&principal.role())
    }
}

const fn role_order(role: Role) -> u8 {
    match role {
        Role::Admin => 0,
        Role::Developer => 1,
        Role::Operator => 2,
        Role::Viewer => 3,
        Role::Worker => 4,
        Role::Runner => 5,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Reject,
}

impl ApprovalDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "APPROVE",
            Self::Reject => "REJECT",
        }
    }

    const fn status(self) -> ApprovalStatus {
        match self {
            Self::Approve => ApprovalStatus::Approved,
            Self::Reject => ApprovalStatus::Rejected,
        }
    }
}

/// Authenticated decision body after transport validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDecisionCommand {
    decision_key: DecisionKey,
    request_hash: Digest,
    decision: ApprovalDecision,
    reason_code: Option<ApprovalReasonCode>,
}

impl ApprovalDecisionCommand {
    #[must_use]
    pub const fn new(
        decision_key: DecisionKey,
        request_hash: Digest,
        decision: ApprovalDecision,
        reason_code: Option<ApprovalReasonCode>,
    ) -> Self {
        Self {
            decision_key,
            request_hash,
            decision,
            reason_code,
        }
    }

    #[must_use]
    pub const fn decision_key(&self) -> &DecisionKey {
        &self.decision_key
    }

    #[must_use]
    pub const fn request_hash(&self) -> Digest {
        self.request_hash
    }

    #[must_use]
    pub const fn decision(&self) -> ApprovalDecision {
        self.decision
    }

    #[must_use]
    pub const fn reason_code(&self) -> Option<&ApprovalReasonCode> {
        self.reason_code.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
    Cancelled,
}

impl ApprovalStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Approved => "APPROVED",
            Self::Rejected => "REJECTED",
            Self::Expired => "EXPIRED",
            Self::Cancelled => "CANCELLED",
        }
    }

    #[must_use]
    pub const fn is_settled(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// Durable result of the first eligible human decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDecisionReceipt {
    id: ApprovalReceiptId,
    request_id: ApprovalRequestId,
    decision_key: DecisionKey,
    request_hash: Digest,
    actor: Subject,
    decision: ApprovalDecision,
    reason_code: Option<ApprovalReasonCode>,
    decided_at: OffsetDateTime,
    outcome_sequence: u64,
}

impl ApprovalDecisionReceipt {
    #[must_use]
    pub const fn id(&self) -> ApprovalReceiptId {
        self.id
    }

    #[must_use]
    pub const fn request_id(&self) -> ApprovalRequestId {
        self.request_id
    }

    #[must_use]
    pub const fn decision_key(&self) -> &DecisionKey {
        &self.decision_key
    }

    #[must_use]
    pub const fn request_hash(&self) -> Digest {
        self.request_hash
    }

    #[must_use]
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }

    #[must_use]
    pub const fn decision(&self) -> ApprovalDecision {
        self.decision
    }

    #[must_use]
    pub const fn reason_code(&self) -> Option<&ApprovalReasonCode> {
        self.reason_code.as_ref()
    }

    #[must_use]
    pub const fn decided_at(&self) -> OffsetDateTime {
        self.decided_at
    }

    #[must_use]
    pub const fn outcome_sequence(&self) -> u64 {
        self.outcome_sequence
    }

    fn matches(&self, command: &ApprovalDecisionCommand) -> bool {
        self.request_hash == command.request_hash
            && self.decision == command.decision
            && self.reason_code == command.reason_code
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDecisionResolution {
    receipt: ApprovalDecisionReceipt,
    replayed: bool,
}

impl ApprovalDecisionResolution {
    #[must_use]
    pub const fn receipt(&self) -> &ApprovalDecisionReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }
}

/// Outcome of an internal expiry or cancellation transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalTransition {
    status: ApprovalStatus,
    changed: bool,
}

impl ApprovalTransition {
    #[must_use]
    pub const fn status(self) -> ApprovalStatus {
        self.status
    }

    #[must_use]
    pub const fn changed(self) -> bool {
        self.changed
    }
}

/// Snapshot and first-commit state for one approval owner instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    id: ApprovalRequestId,
    tenant_id: TenantId,
    namespace_id: NamespaceId,
    run_id: RunId,
    owner: ExecutionInstanceId,
    policy: ApprovalPolicy,
    initiator: Subject,
    opened_sequence: u64,
    deadline: OffsetDateTime,
    status: ApprovalStatus,
    winning_receipt: Option<ApprovalDecisionReceipt>,
    outcome_sequence: Option<u64>,
    settled_at: Option<OffsetDateTime>,
}

impl ApprovalRequest {
    /// Opens a pending request from trusted workflow activation data.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn open(
        id: ApprovalRequestId,
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        run_id: RunId,
        owner: ExecutionInstanceId,
        policy: ApprovalPolicy,
        initiator: Subject,
        opened_sequence: u64,
        deadline: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            tenant_id,
            namespace_id,
            run_id,
            owner,
            policy,
            initiator,
            opened_sequence,
            deadline,
            status: ApprovalStatus::Pending,
            winning_receipt: None,
            outcome_sequence: None,
            settled_at: None,
        }
    }

    #[must_use]
    pub const fn id(&self) -> ApprovalRequestId {
        self.id
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace_id
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn owner(&self) -> &ExecutionInstanceId {
        &self.owner
    }

    #[must_use]
    pub const fn policy(&self) -> &ApprovalPolicy {
        &self.policy
    }

    #[must_use]
    pub const fn initiator(&self) -> &Subject {
        &self.initiator
    }

    #[must_use]
    pub const fn opened_sequence(&self) -> u64 {
        self.opened_sequence
    }

    #[must_use]
    pub const fn deadline(&self) -> OffsetDateTime {
        self.deadline
    }

    #[must_use]
    pub const fn status(&self) -> ApprovalStatus {
        self.status
    }

    #[must_use]
    pub const fn winning_receipt(&self) -> Option<&ApprovalDecisionReceipt> {
        self.winning_receipt.as_ref()
    }

    #[must_use]
    pub const fn winning_receipt_id(&self) -> Option<ApprovalReceiptId> {
        match &self.winning_receipt {
            Some(receipt) => Some(receipt.id),
            None => None,
        }
    }

    #[must_use]
    pub const fn outcome_sequence(&self) -> Option<u64> {
        self.outcome_sequence
    }

    #[must_use]
    pub const fn settled_at(&self) -> Option<OffsetDateTime> {
        self.settled_at
    }

    /// Applies or replays an authorized response. Authorization and eligibility are checked before
    /// a stored receipt may be returned. The nominal deadline does not override first-commit order.
    ///
    /// Persistence must commit the receipt, audit, timer cancellation, journal, chosen route and
    /// outbox in one transaction before exposing this resolution.
    ///
    /// # Errors
    /// Returns a typed denial or conflict without mutating the request.
    pub fn decide(
        &mut self,
        principal: &Principal,
        receipt_id: ApprovalReceiptId,
        command: &ApprovalDecisionCommand,
        decided_at: OffsetDateTime,
        outcome_sequence: u64,
    ) -> Result<ApprovalDecisionResolution, ApprovalError> {
        self.authorize_decision(principal)?;

        if let Some(receipt) = &self.winning_receipt {
            if receipt.decision_key == command.decision_key {
                if receipt.matches(command) {
                    return Ok(ApprovalDecisionResolution {
                        receipt: receipt.clone(),
                        replayed: true,
                    });
                }
                return Err(ApprovalError::IdempotencyKeyReused);
            }
            return Err(ApprovalError::AlreadyDecided);
        }
        if self.status != ApprovalStatus::Pending {
            return Err(ApprovalError::AlreadyDecided);
        }

        let receipt = ApprovalDecisionReceipt {
            id: receipt_id,
            request_id: self.id,
            decision_key: command.decision_key.clone(),
            request_hash: command.request_hash,
            actor: principal.subject().clone(),
            decision: command.decision,
            reason_code: command.reason_code.clone(),
            decided_at,
            outcome_sequence,
        };
        self.status = command.decision.status();
        self.outcome_sequence = Some(outcome_sequence);
        self.settled_at = Some(decided_at);
        self.winning_receipt = Some(receipt.clone());

        Ok(ApprovalDecisionResolution {
            receipt,
            replayed: false,
        })
    }

    /// Commits expiry only when its timer is due and the request is still pending. A losing expiry
    /// is a deterministic no-op that retains the first committed outcome.
    ///
    /// # Errors
    /// Returns [`ApprovalError::ExpiryNotDue`] for an early expiry of a pending request.
    pub fn expire(
        &mut self,
        now: OffsetDateTime,
        outcome_sequence: u64,
    ) -> Result<ApprovalTransition, ApprovalError> {
        if self.status != ApprovalStatus::Pending {
            return Ok(ApprovalTransition {
                status: self.status,
                changed: false,
            });
        }
        if now < self.deadline {
            return Err(ApprovalError::ExpiryNotDue);
        }
        self.status = ApprovalStatus::Expired;
        self.outcome_sequence = Some(outcome_sequence);
        self.settled_at = Some(now);
        Ok(ApprovalTransition {
            status: self.status,
            changed: true,
        })
    }

    /// Cancels a pending request as part of the enclosing run transition. A losing cancellation is
    /// a deterministic no-op that retains the first committed outcome.
    pub fn cancel(
        &mut self,
        cancelled_at: OffsetDateTime,
        outcome_sequence: u64,
    ) -> ApprovalTransition {
        if self.status != ApprovalStatus::Pending {
            return ApprovalTransition {
                status: self.status,
                changed: false,
            };
        }
        self.status = ApprovalStatus::Cancelled;
        self.outcome_sequence = Some(outcome_sequence);
        self.settled_at = Some(cancelled_at);
        ApprovalTransition {
            status: self.status,
            changed: true,
        }
    }

    fn authorize_decision(&self, principal: &Principal) -> Result<(), ApprovalError> {
        if principal.tenant_id() != self.tenant_id
            || !principal.role().grants(Permission::ApprovalDecide)
            || !self.policy.includes(principal)
        {
            return Err(ApprovalError::Forbidden);
        }
        if self.policy.exclude_initiator && principal.subject() == &self.initiator {
            return Err(ApprovalError::InitiatorCannotDecide);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalError {
    #[error("an approval policy needs at least one eligible subject or role")]
    EmptyPolicy,
    #[error("an approval policy may contain at most 32 subjects and roles")]
    TooManyPolicyEntries,
    #[error("an approval policy contains the same subject more than once")]
    DuplicateEligibleSubject,
    #[error("an approval policy contains the same role more than once")]
    DuplicateEligibleRole,
    #[error("role '{0}' cannot be used as an approval policy role")]
    IneligiblePolicyRole(Role),
    #[error("decision key must be 1..128 printable ASCII characters without spaces")]
    InvalidDecisionKey,
    #[error("approval reason code must be 1..64 machine-code ASCII characters")]
    InvalidReasonCode,
    #[error("the current caller is not eligible to decide this approval")]
    Forbidden,
    #[error("the initiating identity cannot decide this approval")]
    InitiatorCannotDecide,
    #[error("the decision key was already used with different request content")]
    IdempotencyKeyReused,
    #[error("the approval already has a committed outcome")]
    AlreadyDecided,
    #[error("the approval deadline has not been reached")]
    ExpiryNotDue,
}

impl ApprovalError {
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::EmptyPolicy
            | Self::TooManyPolicyEntries
            | Self::DuplicateEligibleSubject
            | Self::DuplicateEligibleRole
            | Self::IneligiblePolicyRole(_) => "INVALID_APPROVAL_POLICY",
            Self::InvalidDecisionKey => "INVALID_DECISION_KEY",
            Self::InvalidReasonCode => "INVALID_REASON_CODE",
            Self::Forbidden => "FORBIDDEN",
            Self::InitiatorCannotDecide => "INITIATOR_CANNOT_DECIDE",
            Self::IdempotencyKeyReused => "IDEMPOTENCY_KEY_REUSED",
            Self::AlreadyDecided => "ALREADY_DECIDED",
            Self::ExpiryNotDue => "EXPIRY_NOT_DUE",
        }
    }
}

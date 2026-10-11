//! Versions, approvals and the active pointer (FR-011-001..006).
//!
//! A published version is immutable (`definition.rs`); what moves is the **pointer** to the version
//! new runs use, and the permission to publish into a namespace. This module owns the rules of both:
//! how many people other than the author must approve a publication into a namespace, the life of a
//! request to publish, and the kinds of change the pointer's history records. It knows no storage:
//! the adapter loads a request, applies one of the transitions below, and writes it back under the
//! lock of the request's row.

use std::fmt;
use std::str::FromStr;

use time::OffsetDateTime;

use crate::definition::{WorkflowId, WorkflowVersionId};
use crate::identity::{Environment, NamespaceId, Subject, TenantId};
use crate::workflow::Digest;

uuid_id!(
    /// Identifier of a [`PublicationRequest`].
    RequestId
);

/// The most approvals a namespace can ask for: a policy is a few people, not a committee.
pub const MAX_REQUIRED_APPROVALS: u8 = 3;

/// How many people other than the one who asks must approve a publication into a namespace.
///
/// The author can never approve their own request, whatever the number: a policy of 0 means the
/// namespace does not ask, not that the author may sign off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApprovalPolicy(u8);

impl ApprovalPolicy {
    /// No approval is asked.
    pub const NONE: Self = Self(0);

    /// # Errors
    /// [`VersioningError::PolicyOutOfRange`] above [`MAX_REQUIRED_APPROVALS`].
    pub const fn new(required: u8) -> Result<Self, VersioningError> {
        if required > MAX_REQUIRED_APPROVALS {
            return Err(VersioningError::PolicyOutOfRange);
        }
        Ok(Self(required))
    }

    /// What a namespace asks for until someone sets it: one approval in production, none elsewhere.
    #[must_use]
    pub const fn default_for(environment: Environment) -> Self {
        match environment {
            Environment::Prod => Self(1),
            Environment::Dev | Environment::Staging => Self(0),
        }
    }

    #[must_use]
    pub const fn required(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn asks(self) -> bool {
        self.0 > 0
    }
}

/// Why the pointer to the active version of a workflow changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivationKind {
    /// A draft was published (directly, or once its request was approved).
    Publish,
    /// A version made in another namespace was published here with the same document.
    Promote,
    /// The pointer went back to a version older than the one it left.
    Rollback,
    /// The pointer went forward to a version that was published earlier.
    Activate,
}

impl ActivationKind {
    pub const ALL: [Self; 4] = [Self::Publish, Self::Promote, Self::Rollback, Self::Activate];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Publish => "PUBLISH",
            Self::Promote => "PROMOTE",
            Self::Rollback => "ROLLBACK",
            Self::Activate => "ACTIVATE",
        }
    }

    /// The kind of a change made by hand: backwards is a rollback, forwards an activation.
    #[must_use]
    pub const fn of_switch(from: u32, to: u32) -> Self {
        if to < from {
            Self::Rollback
        } else {
            Self::Activate
        }
    }
}

impl FromStr for ActivationKind {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        Self::ALL.into_iter().find(|k| k.as_str() == text).ok_or(())
    }
}

impl fmt::Display for ActivationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a request to publish is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    /// The draft of the workflow, as it is at the revision the request names.
    Publish,
    /// The document of a version made in another namespace.
    Promote,
}

impl RequestKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Publish => "PUBLISH",
            Self::Promote => "PROMOTE",
        }
    }

    /// `None` for a name that is not one of the two.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "PUBLISH" => Some(Self::Publish),
            "PROMOTE" => Some(Self::Promote),
            _ => None,
        }
    }
}

/// Where a request is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestState {
    /// Waiting for approvals.
    Pending,
    /// Has all the approvals the policy asks for; waiting to be published.
    Approved,
    /// Someone other than the author said no. Final.
    Rejected,
    /// The author took it back. Final.
    Withdrawn,
    /// The draft moved on after the request was made: what was approved is not what would be
    /// published. Final; a new request is made for the new draft.
    Superseded,
    /// Published as a version. Final.
    Published,
}

impl RequestState {
    pub const ALL: [Self; 6] = [
        Self::Pending,
        Self::Approved,
        Self::Rejected,
        Self::Withdrawn,
        Self::Superseded,
        Self::Published,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Approved => "APPROVED",
            Self::Rejected => "REJECTED",
            Self::Withdrawn => "WITHDRAWN",
            Self::Superseded => "SUPERSEDED",
            Self::Published => "PUBLISHED",
        }
    }

    /// `None` for a name that is not one of the six.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text)
    }

    /// Whether the request can still be decided, withdrawn or published.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Pending | Self::Approved)
    }
}

impl fmt::Display for RequestState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What one person decided about a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Approve,
    Reject,
}

impl Verdict {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "APPROVED",
            Self::Reject => "REJECTED",
        }
    }
}

/// A decision, kept with who made it, why and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub by: Subject,
    pub verdict: Verdict,
    pub reason: String,
    pub at: OffsetDateTime,
}

/// Why a transition of a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VersioningError {
    #[error("a policy asks for at most {MAX_REQUIRED_APPROVALS} approvals")]
    PolicyOutOfRange,
    /// The request is final or not in the state the transition needs.
    #[error("the request is {0}")]
    NotInState(RequestState),
    /// The author may not decide their own request.
    #[error("the author of a request cannot decide it")]
    AuthorCannotDecide,
    /// A person decides a request once.
    #[error("this person has already decided the request")]
    AlreadyDecided,
    /// Only the author takes a request back.
    #[error("only the author can withdraw a request")]
    NotTheAuthor,
    /// The request does not have the approvals the policy asks for.
    #[error("{have} of {need} approvals")]
    NotEnoughApprovals { have: u8, need: u8 },
}

/// A request to publish into a namespace that asks for approvals (FR-011-003, FR-011-005).
///
/// It is bound to the draft revision and the digest it names: the people who approve it approve
/// that document. If the draft moves, the request is superseded and a new one is made; nobody
/// approves one thing and publishes another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationRequest {
    id: RequestId,
    tenant_id: TenantId,
    namespace_id: NamespaceId,
    workflow_id: WorkflowId,
    kind: RequestKind,
    draft_revision: u64,
    digest: Digest,
    release_notes: Option<String>,
    promoted_from: Option<WorkflowVersionId>,
    requested_by: Subject,
    requested_at: OffsetDateTime,
    required: u8,
    decisions: Vec<Decision>,
    state: RequestState,
    published: Option<(WorkflowVersionId, Subject, OffsetDateTime)>,
}

impl PublicationRequest {
    /// Opens a request. With a policy that asks for nothing it is born approved.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // the facts of a request, named at the one place it is built
    pub fn open(
        id: RequestId,
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        workflow_id: WorkflowId,
        kind: RequestKind,
        draft_revision: u64,
        digest: Digest,
        release_notes: Option<String>,
        promoted_from: Option<WorkflowVersionId>,
        requested_by: Subject,
        policy: ApprovalPolicy,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            tenant_id,
            namespace_id,
            workflow_id,
            kind,
            draft_revision,
            digest,
            release_notes,
            promoted_from,
            requested_by,
            requested_at: now,
            required: policy.required(),
            decisions: Vec::new(),
            state: if policy.asks() {
                RequestState::Pending
            } else {
                RequestState::Approved
            },
            published: None,
        }
    }

    /// Rebuilds a request from storage; nothing is checked, because storage only holds what the
    /// transitions wrote.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: RequestId,
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        workflow_id: WorkflowId,
        kind: RequestKind,
        draft_revision: u64,
        digest: Digest,
        release_notes: Option<String>,
        promoted_from: Option<WorkflowVersionId>,
        requested_by: Subject,
        requested_at: OffsetDateTime,
        required: u8,
        decisions: Vec<Decision>,
        state: RequestState,
        published: Option<(WorkflowVersionId, Subject, OffsetDateTime)>,
    ) -> Self {
        Self {
            id,
            tenant_id,
            namespace_id,
            workflow_id,
            kind,
            draft_revision,
            digest,
            release_notes,
            promoted_from,
            requested_by,
            requested_at,
            required,
            decisions,
            state,
            published,
        }
    }

    #[must_use]
    pub const fn id(&self) -> RequestId {
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
    pub const fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }
    #[must_use]
    pub const fn kind(&self) -> RequestKind {
        self.kind
    }
    #[must_use]
    pub const fn draft_revision(&self) -> u64 {
        self.draft_revision
    }
    #[must_use]
    pub const fn digest(&self) -> &Digest {
        &self.digest
    }
    #[must_use]
    pub fn release_notes(&self) -> Option<&str> {
        self.release_notes.as_deref()
    }
    #[must_use]
    pub const fn promoted_from(&self) -> Option<WorkflowVersionId> {
        self.promoted_from
    }
    #[must_use]
    pub const fn requested_by(&self) -> &Subject {
        &self.requested_by
    }
    #[must_use]
    pub const fn requested_at(&self) -> OffsetDateTime {
        self.requested_at
    }
    /// The approvals that were asked for when the request was opened.
    #[must_use]
    pub const fn required(&self) -> u8 {
        self.required
    }
    #[must_use]
    pub fn decisions(&self) -> &[Decision] {
        &self.decisions
    }
    #[must_use]
    pub const fn state(&self) -> RequestState {
        self.state
    }
    #[must_use]
    pub fn published(&self) -> Option<&(WorkflowVersionId, Subject, OffsetDateTime)> {
        self.published.as_ref()
    }

    /// How many people have approved.
    #[must_use]
    pub fn approvals(&self) -> u8 {
        u8::try_from(
            self.decisions
                .iter()
                .filter(|d| d.verdict == Verdict::Approve)
                .count(),
        )
        .unwrap_or(u8::MAX)
    }

    fn decide(
        &mut self,
        by: &Subject,
        verdict: Verdict,
        reason: &str,
        now: OffsetDateTime,
    ) -> Result<(), VersioningError> {
        if !self.state.is_open() {
            return Err(VersioningError::NotInState(self.state));
        }
        if *by == self.requested_by {
            return Err(VersioningError::AuthorCannotDecide);
        }
        if self.decisions.iter().any(|d| d.by == *by) {
            return Err(VersioningError::AlreadyDecided);
        }
        self.decisions.push(Decision {
            by: by.clone(),
            verdict,
            reason: reason.to_owned(),
            at: now,
        });
        Ok(())
    }

    /// Adds an approval; the request is `Approved` once it has as many as were asked for.
    ///
    /// # Errors
    /// The request is final, the person is its author, or has decided already.
    pub fn approve(
        &mut self,
        by: &Subject,
        reason: &str,
        now: OffsetDateTime,
    ) -> Result<RequestState, VersioningError> {
        self.decide(by, Verdict::Approve, reason, now)?;
        if self.approvals() >= self.required {
            self.state = RequestState::Approved;
        }
        Ok(self.state)
    }

    /// One person saying no ends the request, whatever the others said.
    ///
    /// # Errors
    /// As [`Self::approve`].
    pub fn reject(
        &mut self,
        by: &Subject,
        reason: &str,
        now: OffsetDateTime,
    ) -> Result<(), VersioningError> {
        self.decide(by, Verdict::Reject, reason, now)?;
        self.state = RequestState::Rejected;
        Ok(())
    }

    /// The author takes the request back.
    ///
    /// # Errors
    /// The request is final, or the person is not its author.
    pub fn withdraw(&mut self, by: &Subject) -> Result<(), VersioningError> {
        if !self.state.is_open() {
            return Err(VersioningError::NotInState(self.state));
        }
        if *by != self.requested_by {
            return Err(VersioningError::NotTheAuthor);
        }
        self.state = RequestState::Withdrawn;
        Ok(())
    }

    /// The draft moved on: what was approved is no longer what would be published.
    ///
    /// # Errors
    /// The request is final.
    pub fn supersede(&mut self) -> Result<(), VersioningError> {
        if !self.state.is_open() {
            return Err(VersioningError::NotInState(self.state));
        }
        self.state = RequestState::Superseded;
        Ok(())
    }

    /// Whether it has what `policy` asks for now. A policy raised after the approvals were given
    /// asks for more: the request waits again.
    #[must_use]
    pub fn satisfies(&self, policy: ApprovalPolicy) -> bool {
        self.approvals() >= policy.required()
    }

    /// Records that the request was published as `version` by `by`.
    ///
    /// # Errors
    /// The request is not `Approved`, or the policy now asks for more than it has.
    pub fn publish(
        &mut self,
        by: &Subject,
        version: WorkflowVersionId,
        policy: ApprovalPolicy,
        now: OffsetDateTime,
    ) -> Result<(), VersioningError> {
        if self.state != RequestState::Approved {
            return Err(VersioningError::NotInState(self.state));
        }
        if !self.satisfies(policy) {
            return Err(VersioningError::NotEnoughApprovals {
                have: self.approvals(),
                need: policy.required(),
            });
        }
        self.state = RequestState::Published;
        self.published = Some((version, by.clone(), now));
        Ok(())
    }
}

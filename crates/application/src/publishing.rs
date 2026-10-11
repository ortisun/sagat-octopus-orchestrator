//! Publishing into an environment, with approvals, and moving the active version (FR-011-002..005).
//!
//! A published version never changes. Four things move around it, and this module owns them:
//!
//! - **the policy** of a namespace: how many people other than the author must approve a
//!   publication into it (one in production until an administrator says otherwise);
//! - **the request** to publish, which names the draft revision and the digest the approvers see,
//!   is decided by people other than the author, and ends published, rejected, withdrawn or
//!   superseded by a newer draft;
//! - **the pointer** to the version new runs use, whose every change (a publication, a promotion,
//!   a rollback) is a row of a history that is never rewritten;
//! - **the promotion** of a version made in one namespace into another, as the same document, so
//!   that what was tried in staging is exactly what reaches production.
//!
//! Nothing here touches a run: a run is bound to the version it started with.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::identity::{
    Environment, Namespace, NamespaceId, Permission, Principal, Subject, TenantId,
};
use sagat_domain::versioning::{
    ActivationId, ActivationKind, ApprovalPolicy, PublicationRequest, RequestId, RequestKind,
    RequestState, Verdict, VersioningError,
};
use sagat_domain::workflow::Digest;
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::definitions::{
    DefinitionService, DefinitionStore, Idempotency, IrCodec, PublishContext, Published,
    VersionRecord, publication_idempotency,
};
use crate::dispatch::{TriggerRegistration, triggers_of};
use crate::error::{AppError, Refusal, StoreError};
use crate::identity::{Page, PageRequest, RequestContext};
use crate::operations::justification;

/// How many findings of a comparison an error that refuses a change names.
const CODES_SHOWN: usize = 5;

/// What an administrator set for a namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPolicy {
    pub required: ApprovalPolicy,
    pub set_by: String,
    pub set_at: OffsetDateTime,
}

/// Where the number a namespace asks for comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicySource {
    /// Nobody set it: the default of the environment.
    Default(Environment),
    Set {
        by: String,
        at: OffsetDateTime,
    },
}

/// The policy of a namespace as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyView {
    pub namespace_id: NamespaceId,
    pub environment: Environment,
    pub policy: ApprovalPolicy,
    pub source: PolicySource,
}

/// The version, in another namespace, that a promotion took its document from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotedFrom {
    pub namespace_id: NamespaceId,
    pub workflow_id: WorkflowId,
    pub version: u32,
}

/// One change of the pointer to the active version of a workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationRecord {
    pub id: ActivationId,
    pub workflow_id: WorkflowId,
    pub version_id: WorkflowVersionId,
    pub version: u32,
    pub previous: Option<u32>,
    pub kind: ActivationKind,
    pub reason: Option<String>,
    pub actor: String,
    pub request: Option<RequestId>,
    pub promoted_from: Option<PromotedFrom>,
    pub at: OffsetDateTime,
}

/// A change of the pointer that is about to be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewActivation {
    pub id: ActivationId,
    pub kind: ActivationKind,
    pub reason: String,
    pub actor: String,
    pub at: OffsetDateTime,
}

/// What opening a request found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The request was stored, for the document that `edited_by` saved (when the platform knows).
    Done { edited_by: Option<Subject> },
    /// A request is open for the workflow already.
    AlreadyOpen(RequestId),
    /// The draft was saved again after the request was prepared: it is at revision `current`, and
    /// nobody is asked to approve a document that is gone.
    Stale { current: u64 },
}

/// What deciding, or withdrawing, a request came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decided {
    Done(Box<PublicationRequest>),
    Refused(VersioningError),
    NotFound,
}

/// What moving the pointer came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Switched {
    Done(Box<ActivationRecord>),
    /// The version asked for is the active one already.
    Unchanged,
    /// The active version is not the one the caller looked at.
    Stale {
        current: Option<u32>,
    },
    NoVersion,
    NoWorkflow,
}

/// What publishing a document or opening a request for it turned out to be.
#[derive(Debug, Clone)]
pub enum Promotion {
    /// The namespace asked for nothing and the document is the active version there.
    Published(Box<Published>),
    /// The namespace asks for approvals: a request is waiting for them.
    Requested(Box<PublicationRequest>),
    /// The target already runs exactly this document.
    AlreadyActive { version: u32 },
}

/// What moving the pointer did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    pub record: ActivationRecord,
    /// False when the version was the active one already.
    pub changed: bool,
}

/// The storage port of publishing. Every method takes the tenant and applies it to every statement.
pub trait PublishingStore: DefinitionStore {
    /// Reads the committed result of a publication with the same key and fingerprint. Never
    /// validates the current draft, moves the active pointer or writes another audit record.
    fn replay_publication(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        idempotency: &Idempotency,
    ) -> impl Future<Output = Result<Option<VersionRecord>, StoreError>> + Send;

    fn namespace_by_id(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> impl Future<Output = Result<Option<Namespace>, StoreError>> + Send;

    /// Whether the namespace exists in a tenant other than `tenant` (decides whether a miss is
    /// recorded as a cross-tenant attempt; never shown).
    fn namespace_outside(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Whether the request exists in a tenant other than `tenant` (decides whether a miss is
    /// recorded as a cross-tenant attempt; never shown).
    fn request_outside(
        &self,
        tenant: TenantId,
        id: RequestId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The digests of the script artifacts a version pins, sorted.
    fn version_scripts(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;

    fn stored_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Option<StoredPolicy>, StoreError>> + Send;

    /// Replaces the policy and writes `audit`, in one transaction.
    fn set_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        policy: ApprovalPolicy,
        by: &str,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Stores a request and writes `audit`, unless one is open for the workflow.
    fn open_request(
        &self,
        request: &PublicationRequest,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Opened, StoreError>> + Send;

    fn request(
        &self,
        tenant: TenantId,
        id: RequestId,
    ) -> impl Future<Output = Result<Option<PublicationRequest>, StoreError>> + Send;

    /// The requests of a workflow, newest first, `before` a request, at most `limit`.
    fn requests(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        state: Option<RequestState>,
        before: Option<RequestId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<PublicationRequest>, StoreError>> + Send;

    /// Records a decision under the lock of the request and writes `audit`; the rules are the
    /// domain's ([`PublicationRequest::approve`] and [`PublicationRequest::reject`]).
    #[allow(clippy::too_many_arguments)]
    fn decide(
        &self,
        tenant: TenantId,
        id: RequestId,
        by: &Subject,
        verdict: Verdict,
        reason: &str,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Decided, StoreError>> + Send;

    /// The author takes the request back.
    fn withdraw(
        &self,
        tenant: TenantId,
        id: RequestId,
        by: &Subject,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Decided, StoreError>> + Send;

    /// The changes of the pointer of a workflow, newest first, `before` a change, at most `limit`.
    fn activations(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        before: Option<ActivationId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ActivationRecord>, StoreError>> + Send;

    /// Points the workflow at version `to` if `expected` is still the active one, re-registering
    /// the triggers of that version, recording the change and `audit`, in one transaction.
    #[allow(clippy::too_many_arguments)]
    fn switch(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        to: u32,
        expected: Option<u32>,
        change: &NewActivation,
        triggers: &[TriggerRegistration],
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Switched, StoreError>> + Send;
}

/// Publishing use cases.
#[derive(Debug, Clone)]
pub struct PublishingService<S, C, X> {
    store: S,
    clock: C,
    codec: X,
    definitions: DefinitionService<S, C, X>,
}

impl<S, C, X> PublishingService<S, C, X>
where
    S: PublishingStore + Clone,
    C: Clock + Clone,
    X: IrCodec + Clone,
{
    pub fn new(store: S, clock: C, codec: X) -> Self {
        Self {
            definitions: DefinitionService::new(store.clone(), clock.clone(), codec.clone()),
            store,
            clock,
            codec,
        }
    }

    // ------------------------------------------------------------------ dependencies

    /// The script artifacts the version pins (FR-011-001): what a promotion has to find in the target
    /// and what the database will not let be removed while the version exists.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`].
    pub async fn dependencies(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        version: u32,
        ctx: &RequestContext,
    ) -> Result<Vec<String>, AppError> {
        let record = self
            .definitions
            .get_version(principal, workflow, version, ctx)
            .await?;
        Ok(self
            .store
            .version_scripts(principal.tenant_id(), record.version.id())
            .await?)
    }

    // ------------------------------------------------------------------ policy

    async fn namespace(
        &self,
        principal: &Principal,
        id: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Namespace, AppError> {
        if let Some(namespace) = self
            .store
            .namespace_by_id(principal.tenant_id(), id)
            .await?
        {
            return Ok(namespace);
        }
        if let Ok(true) = self
            .store
            .namespace_outside(principal.tenant_id(), id)
            .await
        {
            let record = AuditRecord::by(
                principal,
                AuditAction::CrossTenantAccessDenied,
                ResourceType::Namespace,
                id.to_string(),
                ctx.trace_id.clone(),
                self.clock.now(),
            )
            .with_reason("resource not accessible from this tenant");
            if let Err(error) = self.store.record_denial(&record).await {
                tracing::error!(%error, action = %record.action(), "failed to record audit entry");
            }
        }
        Err(AppError::NotFound)
    }

    async fn effective_policy(
        &self,
        principal: &Principal,
        namespace: &Namespace,
    ) -> Result<PolicyView, AppError> {
        let stored = self
            .store
            .stored_policy(principal.tenant_id(), namespace.id())
            .await?;
        Ok(match stored {
            Some(stored) => PolicyView {
                namespace_id: namespace.id(),
                environment: namespace.environment(),
                policy: stored.required,
                source: PolicySource::Set {
                    by: stored.set_by,
                    at: stored.set_at,
                },
            },
            None => PolicyView {
                namespace_id: namespace.id(),
                environment: namespace.environment(),
                policy: ApprovalPolicy::default_for(namespace.environment()),
                source: PolicySource::Default(namespace.environment()),
            },
        })
    }

    /// How many approvals a publication into the namespace needs.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`].
    pub async fn policy(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<PolicyView, AppError> {
        principal.authorize(Permission::DefinitionRead)?;
        let namespace = self.namespace(principal, namespace, ctx).await?;
        self.effective_policy(principal, &namespace).await
    }

    /// Sets how many approvals a publication into the namespace needs.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `publication:manage`; [`AppError::NotAllowed`] for a
    /// number above the most a policy asks for, or no justification; [`AppError::NotFound`].
    pub async fn set_policy(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        required: u8,
        reason: &str,
        ctx: &RequestContext,
    ) -> Result<PolicyView, AppError> {
        principal.authorize(Permission::PublicationManage)?;
        let reason = justification(reason)?;
        let policy = ApprovalPolicy::new(required)
            .map_err(|_| AppError::NotAllowed("a policy asks for at most 3 approvals"))?;
        let namespace = self.namespace(principal, namespace, ctx).await?;
        let before = self.effective_policy(principal, &namespace).await?.policy;
        let now = self.clock.now();
        let audit = AuditRecord::by(
            principal,
            AuditAction::PublicationPolicyChanged,
            ResourceType::Namespace,
            namespace.id().to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "from {} to {} approval(s): {reason}",
            before.required(),
            policy.required()
        ));
        self.store
            .set_policy(
                principal.tenant_id(),
                namespace.id(),
                policy,
                principal.subject().as_str(),
                now,
                &audit,
            )
            .await?;
        self.effective_policy(principal, &namespace).await
    }

    // ------------------------------------------------------------------ requests

    /// Asks for the draft, at `draft_revision`, to be approved and published.
    ///
    /// # Errors
    /// As publishing (the draft must be publishable), plus [`Refusal::RequestOpen`] when a request
    /// is open for the workflow.
    pub async fn open_request(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        draft_revision: u64,
        release_notes: Option<&str>,
        ctx: &RequestContext,
    ) -> Result<PublicationRequest, AppError> {
        principal.authorize(Permission::DefinitionPublish)?;
        crate::definitions::check_release_notes(release_notes)?;
        let (record, parsed) = self
            .definitions
            .prepare(principal, workflow, draft_revision, ctx)
            .await?;
        let namespace = self
            .namespace(principal, record.workflow.namespace_id(), ctx)
            .await?;
        let policy = self.effective_policy(principal, &namespace).await?.policy;
        self.open(
            principal,
            &record.workflow.name().to_string(),
            workflow,
            namespace.id(),
            RequestKind::Publish,
            draft_revision,
            parsed.digest,
            release_notes,
            None,
            policy,
            ctx,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn open(
        &self,
        principal: &Principal,
        name: &str,
        workflow: WorkflowId,
        namespace: NamespaceId,
        kind: RequestKind,
        draft_revision: u64,
        digest: Digest,
        release_notes: Option<&str>,
        promoted_from: Option<WorkflowVersionId>,
        policy: ApprovalPolicy,
        ctx: &RequestContext,
    ) -> Result<PublicationRequest, AppError> {
        let now = self.clock.now();
        let request = PublicationRequest::open(
            RequestId::generate(),
            principal.tenant_id(),
            namespace,
            workflow,
            kind,
            draft_revision,
            digest,
            release_notes.map(str::to_owned),
            promoted_from,
            principal.subject().clone(),
            policy,
            now,
        );
        let audit = AuditRecord::by(
            principal,
            AuditAction::PublicationRequested,
            ResourceType::PublicationRequest,
            request.id().to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "{} of workflow {name}, draft revision {draft_revision}, digest {}",
            kind.as_str(),
            request.digest().to_hex()
        ));
        match self.store.open_request(&request, &audit).await? {
            Opened::Done { edited_by } => Ok(request.with_edited_by(edited_by)),
            Opened::Stale { current } => Err(AppError::RevisionConflict { current }),
            Opened::AlreadyOpen(open) => {
                Err(AppError::Versioning(Refusal::RequestOpen { request: open }))
            }
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`].
    pub async fn get_request(
        &self,
        principal: &Principal,
        id: RequestId,
        ctx: &RequestContext,
    ) -> Result<PublicationRequest, AppError> {
        principal.authorize(Permission::DefinitionRead)?;
        match self.store.request(principal.tenant_id(), id).await? {
            Some(request) => Ok(request),
            None => Err(self.missing_request(principal, id, ctx).await),
        }
    }

    async fn missing_request(
        &self,
        principal: &Principal,
        id: RequestId,
        ctx: &RequestContext,
    ) -> AppError {
        if let Ok(true) = self.store.request_outside(principal.tenant_id(), id).await {
            let record = AuditRecord::by(
                principal,
                AuditAction::CrossTenantAccessDenied,
                ResourceType::PublicationRequest,
                id.to_string(),
                ctx.trace_id.clone(),
                self.clock.now(),
            )
            .with_reason("resource not accessible from this tenant");
            if let Err(error) = self.store.record_denial(&record).await {
                tracing::error!(%error, action = %record.action(), "failed to record audit entry");
            }
        }
        AppError::NotFound
    }

    /// The requests of a workflow, newest first.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`];
    /// [`AppError::Invalid`] for a bad cursor.
    pub async fn requests(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        state: Option<RequestState>,
        page: &PageRequest,
        ctx: &RequestContext,
    ) -> Result<Page<PublicationRequest>, AppError> {
        principal.authorize(Permission::DefinitionRead)?;
        self.definitions
            .get_workflow(principal, workflow, ctx)
            .await?;
        let before = page
            .cursor
            .as_deref()
            .map(str::parse::<RequestId>)
            .transpose()?;
        let limit = page.effective_limit();
        let rows = self
            .store
            .requests(principal.tenant_id(), workflow, state, before, limit + 1)
            .await?;
        Ok(paginate(rows, limit, |r| r.id().to_string()))
    }

    /// One person's decision about a request.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `publication:approve`; [`AppError::NotAllowed`] without a
    /// justification; [`AppError::NotFound`]; [`Refusal::Request`] when the author decides, a
    /// person decides twice, or the request is final.
    pub async fn decide(
        &self,
        principal: &Principal,
        id: RequestId,
        verdict: Verdict,
        reason: &str,
        ctx: &RequestContext,
    ) -> Result<PublicationRequest, AppError> {
        principal.authorize(Permission::PublicationApprove)?;
        let reason = justification(reason)?;
        let request = self.get_request(principal, id, ctx).await?;
        let now = self.clock.now();
        let action = match verdict {
            Verdict::Approve => AuditAction::PublicationApproved,
            Verdict::Reject => AuditAction::PublicationRejected,
        };
        let audit = AuditRecord::by(
            principal,
            action,
            ResourceType::PublicationRequest,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!("digest {}: {reason}", request.digest().to_hex()));
        match self
            .store
            .decide(
                principal.tenant_id(),
                id,
                principal.subject(),
                verdict,
                reason,
                now,
                &audit,
            )
            .await?
        {
            Decided::Done(request) => Ok(*request),
            Decided::Refused(error) => {
                self.record_refusal(principal, id, error, ctx).await;
                Err(AppError::Versioning(Refusal::Request(error)))
            }
            Decided::NotFound => Err(self.missing_request(principal, id, ctx).await),
        }
    }

    /// Writes down that somebody tried to decide their own request, or to withdraw somebody
    /// else's: the two refusals that are worth a person looking at. A failure to write it is
    /// logged and does not change the answer.
    async fn record_refusal(
        &self,
        principal: &Principal,
        id: RequestId,
        error: VersioningError,
        ctx: &RequestContext,
    ) {
        let code = match error {
            VersioningError::AuthorCannotDecide => "AUTHOR_CANNOT_DECIDE",
            VersioningError::NotTheAuthor => "NOT_THE_AUTHOR",
            _ => return,
        };
        let record = AuditRecord::by(
            principal,
            AuditAction::PublicationDenied,
            ResourceType::PublicationRequest,
            id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(code);
        if let Err(error) = self.store.record_denial(&record).await {
            tracing::error!(%error, action = %record.action(), "failed to record audit entry");
        }
    }

    /// The author takes the request back.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:publish`; [`AppError::NotFound`];
    /// [`Refusal::Request`] for anyone but the author, or a final request.
    pub async fn withdraw(
        &self,
        principal: &Principal,
        id: RequestId,
        ctx: &RequestContext,
    ) -> Result<PublicationRequest, AppError> {
        principal.authorize(Permission::DefinitionPublish)?;
        let now = self.clock.now();
        let audit = AuditRecord::by(
            principal,
            AuditAction::PublicationWithdrawn,
            ResourceType::PublicationRequest,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        );
        match self
            .store
            .withdraw(principal.tenant_id(), id, principal.subject(), now, &audit)
            .await?
        {
            Decided::Done(request) => Ok(*request),
            Decided::Refused(error) => {
                self.record_refusal(principal, id, error, ctx).await;
                Err(AppError::Versioning(Refusal::Request(error)))
            }
            Decided::NotFound => Err(self.missing_request(principal, id, ctx).await),
        }
    }

    /// Publishes an approved request: the draft at the revision it names, as the version its
    /// approvers saw. If the draft moved on, the request was superseded and nothing is published.
    ///
    /// # Errors
    /// As publishing, plus [`Refusal::Request`] when the request is not approved or the policy
    /// now asks for more approvals than it has.
    pub async fn publish_request(
        &self,
        principal: &Principal,
        id: RequestId,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<Published, AppError> {
        principal.authorize(Permission::DefinitionPublish)?;
        let request = self.get_request(principal, id, ctx).await?;
        // A request that was published is asked again with the key it was published with when the
        // answer was lost: that is answered as it was by the store, which knows the key; any other
        // key finds the request published and is refused.
        if request.state() == RequestState::Published {
            return self
                .replay_published_request(principal, &request, key)
                .await;
        }
        if request.state() != RequestState::Approved {
            return Err(AppError::Versioning(Refusal::Request(
                VersioningError::NotInState(request.state()),
            )));
        }
        let namespace = self
            .namespace(principal, request.namespace_id(), ctx)
            .await?;
        let policy = self.effective_policy(principal, &namespace).await?.policy;
        if !request.satisfies(policy) {
            return Err(AppError::Versioning(Refusal::Request(
                VersioningError::NotEnoughApprovals {
                    have: request.approvals(),
                    need: policy.required(),
                },
            )));
        }
        let context = PublishContext {
            request: Some(id),
            promoted_from: request.promoted_from(),
            reason: request.release_notes().map(str::to_owned),
            scripts: Vec::new(),
        };
        match self
            .definitions
            .publish_through(
                principal,
                request.workflow_id(),
                request.draft_revision(),
                request.release_notes(),
                key,
                ctx,
                context,
            )
            .await
        {
            // The draft moved on after the request was made: the store superseded it with the edit.
            Err(AppError::RevisionConflict { .. }) => {
                let current = self.get_request(principal, id, ctx).await?;
                if current.state() == RequestState::Published {
                    self.replay_published_request(principal, &current, key)
                        .await
                } else {
                    Err(AppError::Versioning(Refusal::Request(
                        VersioningError::NotInState(current.state()),
                    )))
                }
            }
            other => other,
        }
    }

    async fn replay_published_request(
        &self,
        principal: &Principal,
        request: &PublicationRequest,
        key: &str,
    ) -> Result<Published, AppError> {
        let identity = publication_idempotency(
            request.workflow_id(),
            request.draft_revision(),
            *request.digest(),
            request.release_notes(),
            key,
        );
        let stored = self
            .store
            .replay_publication(principal.tenant_id(), request.workflow_id(), &identity)
            .await?
            .ok_or(AppError::Versioning(Refusal::Request(
                VersioningError::NotInState(RequestState::Published),
            )))?;
        if request.published().map(|(id, _, _)| *id) != Some(stored.version.id()) {
            return Err(AppError::Internal);
        }
        let parsed = self
            .codec
            .parse(&stored.document)
            .map_err(|_| AppError::Internal)?;
        Ok(Published {
            version: stored.version,
            created: false,
            warnings: parsed.warnings,
        })
    }

    // ------------------------------------------------------------------ the pointer

    /// The changes of the pointer to the active version, newest first.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:read`; [`AppError::NotFound`];
    /// [`AppError::Invalid`] for a bad cursor.
    pub async fn activations(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        page: &PageRequest,
        ctx: &RequestContext,
    ) -> Result<Page<ActivationRecord>, AppError> {
        principal.authorize(Permission::DefinitionRead)?;
        self.definitions
            .get_workflow(principal, workflow, ctx)
            .await?;
        let before = page
            .cursor
            .as_deref()
            .map(str::parse::<ActivationId>)
            .transpose()?;
        let limit = page.effective_limit();
        let rows = self
            .store
            .activations(principal.tenant_id(), workflow, before, limit + 1)
            .await?;
        Ok(paginate(rows, limit, |r| r.id.to_string()))
    }

    /// What would break if the pointer moved from the version `from` to the document `to`: the
    /// findings of the comparison that concern callers and emitters, or `None` when none do.
    fn breaking(&self, from: &[u8], to: &[u8]) -> Result<Option<Refusal>, AppError> {
        let diff = self
            .codec
            .diff(from, to)
            .map_err(|e| AppError::InvalidDefinition(Box::new(e)))?;
        if diff.breaking.is_empty() {
            return Ok(None);
        }
        Ok(Some(Refusal::BreakingChange {
            findings: diff.breaking.len(),
            codes: diff
                .breaking
                .iter()
                .take(CODES_SHOWN)
                .map(|f| f.code)
                .collect(),
        }))
    }

    /// Points new runs of the workflow at a version that was published already: a rollback, or the
    /// way forward again. Nothing is rewritten: the versions are as they were, the runs in flight
    /// keep the version they started with, and the change is a row of the history.
    ///
    /// `expected` is the version the caller saw active, and the change is refused if it is not any
    /// more. A change that breaks what callers and emitters rely on (a trigger gone, an input
    /// property gone, retyped or newly required) is refused unless `accept_breaking`: the caller
    /// is shown what they would be accepting.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `version:activate`; [`AppError::NotAllowed`] without a
    /// justification; [`AppError::NotFound`]; [`Refusal::ActiveChanged`], [`Refusal::BreakingChange`].
    #[allow(clippy::too_many_arguments)]
    pub async fn activate(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        version: u32,
        reason: &str,
        expected: Option<u32>,
        accept_breaking: bool,
        ctx: &RequestContext,
    ) -> Result<Activation, AppError> {
        principal.authorize(Permission::VersionActivate)?;
        let reason = justification(reason)?;
        let record = self
            .definitions
            .get_workflow(principal, workflow, ctx)
            .await?;
        let current = record
            .active
            .as_ref()
            .map(sagat_domain::definition::WorkflowVersion::number);
        if expected.is_some() && expected != current {
            return Err(AppError::Versioning(Refusal::ActiveChanged { current }));
        }
        let target = self
            .definitions
            .get_version(principal, workflow, version, ctx)
            .await?;
        if current == Some(version) {
            let latest = self
                .store
                .activations(principal.tenant_id(), workflow, None, 1)
                .await?
                .into_iter()
                .next()
                .ok_or(AppError::Internal)?;
            return Ok(Activation {
                record: latest,
                changed: false,
            });
        }
        if let (Some(from), false) = (current, accept_breaking) {
            let from = self
                .definitions
                .get_version(principal, workflow, from, ctx)
                .await?;
            if let Some(refusal) = self.breaking(&from.document, &target.document)? {
                return Err(AppError::Versioning(refusal));
            }
        }
        let parsed = self
            .codec
            .parse(&target.document)
            .map_err(|e| AppError::InvalidDefinition(Box::new(e)))?;
        let kind = current.map_or(ActivationKind::Activate, |from| {
            ActivationKind::of_switch(from, version)
        });
        let now = self.clock.now();
        let change = NewActivation {
            id: ActivationId::generate(),
            kind,
            reason: reason.to_owned(),
            actor: principal.subject().as_str().to_owned(),
            at: now,
        };
        let audit = AuditRecord::by(
            principal,
            AuditAction::WorkflowActivated,
            ResourceType::Workflow,
            workflow.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "{kind}: {} -> {version}: {reason}",
            current.map_or_else(|| "none".to_owned(), |c| c.to_string())
        ));
        match self
            .store
            .switch(
                principal.tenant_id(),
                workflow,
                version,
                current,
                &change,
                &triggers_of(workflow, &parsed.definition),
                &audit,
            )
            .await?
        {
            Switched::Done(record) => Ok(Activation {
                record: *record,
                changed: true,
            }),
            Switched::Unchanged => {
                let latest = self
                    .store
                    .activations(principal.tenant_id(), workflow, None, 1)
                    .await?
                    .into_iter()
                    .next()
                    .ok_or(AppError::Internal)?;
                Ok(Activation {
                    record: latest,
                    changed: false,
                })
            }
            Switched::Stale { current } => {
                Err(AppError::Versioning(Refusal::ActiveChanged { current }))
            }
            Switched::NoVersion | Switched::NoWorkflow => {
                Err(self.definitions_missing(principal, workflow, ctx).await)
            }
        }
    }

    async fn definitions_missing(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        ctx: &RequestContext,
    ) -> AppError {
        match self
            .definitions
            .get_workflow(principal, workflow, ctx)
            .await
        {
            Err(error) => error,
            Ok(_) => AppError::NotFound,
        }
    }

    // ------------------------------------------------------------------ promotion

    /// Takes version `version` of a workflow and publishes the same document, byte for byte, into
    /// `target` as the workflow of the same name, so that what was tried in one environment is
    /// exactly what reaches the next. A namespace that asks for approvals gets a request instead,
    /// bound to that document.
    ///
    /// The target's draft is replaced by the document; a draft with changes nobody published is
    /// not overwritten. A target that already runs the document is left alone. A change that breaks
    /// what callers and emitters of the target rely on is refused unless `accept_breaking`.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `definition:publish`; [`AppError::NotAllowed`] without a
    /// justification, or to a namespace of an earlier environment, or to the same namespace;
    /// [`AppError::NotFound`]; [`Refusal::DraftHasChanges`], [`Refusal::BreakingChange`]; and
    /// whatever publishing the document into the target refuses (a secret the target does not
    /// have, for one).
    #[allow(clippy::too_many_arguments)]
    pub async fn promote(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        version: u32,
        target: NamespaceId,
        reason: &str,
        accept_breaking: bool,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<Promotion, AppError> {
        principal.authorize(Permission::DefinitionPublish)?;
        principal.authorize(Permission::DefinitionWrite)?;
        let reason = justification(reason)?;
        let source = self
            .definitions
            .get_workflow(principal, workflow, ctx)
            .await?;
        let from = self
            .definitions
            .get_version(principal, workflow, version, ctx)
            .await?;
        let from_namespace = self
            .namespace(principal, source.workflow.namespace_id(), ctx)
            .await?;
        let target = self.namespace(principal, target, ctx).await?;
        if target.id() == from_namespace.id() {
            return Err(AppError::NotAllowed(
                "a version is promoted to another namespace",
            ));
        }
        if rank(target.environment()) < rank(from_namespace.environment()) {
            return Err(AppError::NotAllowed(
                "a version is promoted to the same environment or a later one",
            ));
        }
        let name = source.workflow.name().clone();
        let existing = self
            .store
            .find_workflow_by_name(principal.tenant_id(), target.id(), &name)
            .await?;
        let policy = self.effective_policy(principal, &target).await?.policy;

        // What the target runs now, and whether its draft holds work nobody published.
        let mut active_document = None;
        let (target_workflow, draft_revision) = if let Some(existing) = existing {
            if let Some(active) = &existing.active {
                if active.digest() == from.version.digest() {
                    return Ok(Promotion::AlreadyActive {
                        version: active.number(),
                    });
                }
                active_document = Some(
                    self.definitions
                        .get_version(principal, existing.workflow.id(), active.number(), ctx)
                        .await?
                        .document,
                );
            }
            let draft_digest = self.codec.parse(&existing.draft).ok().map(|p| p.digest);
            let clean = draft_digest.is_some_and(|d| {
                &d == from.version.digest()
                    || existing.active.as_ref().is_some_and(|a| &d == a.digest())
            });
            if !clean {
                return Err(AppError::Versioning(Refusal::DraftHasChanges));
            }
            if let (Some(active_document), false) = (&active_document, accept_breaking)
                && let Some(refusal) = self.breaking(active_document, &from.document)?
            {
                return Err(AppError::Versioning(refusal));
            }
            let updated = if draft_digest.as_ref() == Some(from.version.digest()) {
                existing.workflow.draft_revision()
            } else {
                self.definitions
                    .update_draft(
                        principal,
                        existing.workflow.id(),
                        existing.workflow.draft_revision(),
                        &from.document,
                        ctx,
                    )
                    .await?
                    .workflow
                    .draft_revision()
            };
            (existing.workflow.id(), updated)
        } else {
            let created = self
                .definitions
                .create_workflow(
                    principal,
                    target.slug().as_str(),
                    name.as_str(),
                    &from.document,
                    &derived_key(key, "create"),
                    ctx,
                )
                .await?;
            (created.workflow.id(), created.workflow.draft_revision())
        };

        if policy.asks() {
            let request = self
                .open(
                    principal,
                    name.as_str(),
                    target_workflow,
                    target.id(),
                    RequestKind::Promote,
                    draft_revision,
                    *from.version.digest(),
                    Some(reason),
                    Some(from.version.id()),
                    policy,
                    ctx,
                )
                .await?;
            return Ok(Promotion::Requested(Box::new(request)));
        }
        let context = PublishContext {
            request: None,
            promoted_from: Some(from.version.id()),
            reason: Some(reason.to_owned()),
            scripts: Vec::new(),
        };
        let published = self
            .definitions
            .publish_through(
                principal,
                target_workflow,
                draft_revision,
                Some(reason),
                key,
                ctx,
                context,
            )
            .await?;
        Ok(Promotion::Published(Box::new(published)))
    }
}

/// A key for a step of an operation that has a key of its own: the same for the same key, a
/// different one for each step, and short whatever the caller's is (the store takes 8 to 160
/// characters, and the caller's may be 160 already).
fn derived_key(key: &str, step: &str) -> String {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"sagat/derived-key/v1");
    hash.update((key.len() as u64).to_be_bytes());
    hash.update(key.as_bytes());
    hash.update(step.as_bytes());
    let digest = hash.finalize();
    let hex: String = digest.iter().take(20).map(|b| format!("{b:02x}")).collect();
    format!("{step}-{hex}")
}

/// How late in a pipeline an environment is.
const fn rank(environment: Environment) -> u8 {
    match environment {
        Environment::Dev => 0,
        Environment::Staging => 1,
        Environment::Prod => 2,
    }
}

fn paginate<T>(mut rows: Vec<T>, limit: u32, id_of: impl Fn(&T) -> String) -> Page<T> {
    let limit = limit as usize;
    if rows.len() > limit {
        rows.truncate(limit);
        let next_cursor = rows.last().map(id_of);
        Page {
            items: rows,
            next_cursor,
        }
    } else {
        Page {
            items: rows,
            next_cursor: None,
        }
    }
}

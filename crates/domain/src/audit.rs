//! Audit trail records (FR-001-007).
//!
//! A record is always created from the acting [`Principal`]: the tenant and the actor are taken
//! from it, never from caller input, so an audit entry can only ever land in the actor's tenant.

use std::fmt;

use time::OffsetDateTime;

use crate::identity::{Principal, Subject, TenantId};

uuid_id!(
    /// Identifier of an [`AuditRecord`].
    AuditId
);

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditAction {
    /// A tenant was created by an operator or the development seed.
    TenantProvisioned,
    WorkspaceCreated,
    NamespaceCreated,
    MembershipGranted,
    WorkflowCreated,
    WorkflowDraftUpdated,
    /// An immutable version was published (or an identical one was found to exist).
    WorkflowPublished,
    RunStarted,
    RunCancelRequested,
    RunPaused,
    RunResumed,
    /// An operator inspected a payload's uniformly redacted structure.
    PayloadInspected,
    /// An authenticated caller without the dedicated permission tried to inspect a payload.
    PayloadInspectionDenied,
    /// A signal was delivered to a waiting run.
    RunSignalled,
    /// The caller asked for a resource that exists only in another tenant. The record carries the
    /// identifier the caller supplied and nothing about the other tenant.
    CrossTenantAccessDenied,
    /// A secret was created, or a new version of it was stored (rotation). Never carries a value.
    SecretStored,
    SecretProvisioningStarted,
    DatabaseConnectionChanged,
    DatabaseDenied,
    DatabaseResultRead,
    SecretDeleted,
    /// Someone was allowed to reference a secret from workflows, or no longer is.
    SecretAccessGranted,
    SecretAccessRevoked,
    /// A worker received the value of a secret for one leased attempt.
    SecretResolved,
    /// A workflow referenced a secret its publisher may not use, or a lease asked for one it was
    /// not given.
    SecretDenied,
    QuotaChanged,
    /// The egress policy of a namespace was replaced.
    EgressPolicyChanged,
    /// An outbound request was refused by the egress policy.
    EgressDenied,
    /// A worker identity was bound to a namespace and activities, or unbound.
    WorkerBound,
    WorkerUnbound,
    /// A worker asked to serve an activity its identity is not bound to.
    WorkerDenied,
    /// An operator asked for another attempt of a step that waited for a decision.
    SagaStepRetried,
    /// An operator gave up on a compensation, with a justification.
    SagaStepSkipped,
    /// An operator held a step that had not started for a person to decide.
    RunMarkedForReview,
    /// An operator started a new run from an ended one.
    RunRerun,
    /// An administrator read the audit trail. Reading it again with the same filters adds nothing.
    AuditRead,
    /// An operator stopped a worker identity from taking new work, or let it take work again.
    WorkerDrained,
    WorkerUndrained,
    /// An operator settled a step with an unknown outcome (or a compensation done elsewhere).
    SagaStepResolved,
    /// An administrator set how many approvals a namespace asks for before something is published.
    PublicationPolicyChanged,
    /// Someone asked for a publication to be approved, and what each person decided about it.
    PublicationRequested,
    PublicationApproved,
    PublicationRejected,
    PublicationWithdrawn,
    /// A request was ended because the draft it was about was saved again.
    PublicationSuperseded,
    /// Somebody tried to decide their own request, or to withdraw somebody else's. Written once
    /// per person, request and reason in a window: whoever can reach the endpoint can repeat it.
    PublicationDenied,
    /// The pointer to the active version of a workflow moved without a publication: a rollback, or
    /// the way forward again.
    WorkflowActivated,
    /// A webhook endpoint was created, changed or deleted.
    EventEndpointCreated,
    EventEndpointChanged,
    EventEndpointDeleted,
    /// A principal published an event through the API. Who did it is here and nowhere else: the
    /// runs the event starts are the engine's, and the inbox row is purged with its window.
    EventPublished,
    /// A request to a webhook endpoint was refused (a signature that does not verify, a stale
    /// timestamp, an event id reused for another payload). Written once per endpoint and reason in
    /// a window: whoever can reach the endpoint can repeat it as often as they like.
    EventRejected,
    /// An outbound webhook subscription was created, changed or deleted.
    WebhookSubscriptionCreated,
    WebhookSubscriptionChanged,
    WebhookSubscriptionDeleted,
    /// The platform switched a subscription off after too many deliveries were set aside.
    WebhookSubscriptionDisabled,
    /// An operator put dead deliveries back in the queue.
    WebhookDeliveryRedriven,
    /// An outbound broker subscription was created, changed, deleted or disabled.
    BrokerSubscriptionCreated,
    BrokerSubscriptionChanged,
    BrokerSubscriptionDeleted,
    BrokerSubscriptionDisabled,
    /// An operator put dead broker deliveries back in the queue.
    BrokerDeliveryRedriven,
    /// A connector to a broker was created, changed or deleted.
    ConnectorCreated,
    ConnectorChanged,
    ConnectorDeleted,
    /// An operator discarded or tried again a message a connector had set aside.
    ConnectorMessageDiscarded,
    ConnectorMessageRedriven,
    /// A script was created, its draft changed, or it was deleted.
    ScriptCreated,
    ScriptChanged,
    ScriptDeleted,
    /// A version of a script was published (its build queued).
    ScriptPublished,
    /// The build of a version finished: the runner that made it is the actor.
    ScriptBuilt,
    /// A draft was tried in the sandbox.
    ScriptTested,
    /// A script asked for something it may not have (a request the egress policy refuses, a secret
    /// it did not declare). Written once per script, kind and reason, and at most a hundred
    /// different ones per script: a script in a loop can ask as often as it likes.
    ScriptViolation,
    BlobUploaded,
    BlobRead,
    BlobDeleted,
}

impl AuditAction {
    /// Actions that are kept for whoever runs the platform and never read back by the tenant they
    /// are written on. A denial is recorded only when the resource exists in another tenant, so a
    /// trail that listed it would tell anyone who can read it whether an identifier they tried
    /// exists elsewhere, which the `404` is careful not to say.
    pub const WITHHELD_FROM_TENANT: &'static [Self] = &[Self::CrossTenantAccessDenied];

    /// Whether doing the same thing again, to the same resource and for the same stated reason, adds
    /// nothing to the trail. These events are driven by a caller who can repeat them as often as
    /// they like (a worker asking again for the secrets of the attempt it holds), so the trail keeps
    /// the first and not every repeat: its growth is then bounded by what actually happened (one
    /// per run, node and attempt), not by how many times somebody asked.
    #[must_use]
    pub const fn collapses_repeats(self) -> bool {
        matches!(
            self,
            Self::BlobRead
                | Self::SecretResolved
                | Self::SecretDenied
                | Self::DatabaseDenied
                | Self::EventRejected
                | Self::ScriptViolation
                | Self::AuditRead
                | Self::PublicationDenied
                | Self::PayloadInspected
                | Self::PayloadInspectionDenied
        )
    }

    /// How long an identical event is taken as the same one, for the actions that collapse repeats:
    /// `None` for ever (what a worker asks again is the same ask), or a period after which the same
    /// thing done again is a new event. Reading the trail is the second kind: an administrator who
    /// exports it every day leaves a record every day, not one record for good.
    #[must_use]
    pub const fn repeat_window(self) -> Option<std::time::Duration> {
        match self {
            Self::AuditRead
            | Self::PublicationDenied
            | Self::DatabaseDenied
            | Self::PayloadInspected
            | Self::PayloadInspectionDenied => Some(std::time::Duration::from_secs(300)),
            _ => None,
        }
    }

    /// How many different events of this action one actor may have on one resource before the
    /// rest are not recorded: `None` for no limit. The text of an event can be chosen by whoever
    /// causes it, so "once per distinct text" alone does not bound the trail.
    #[must_use]
    pub const fn distinct_cap(self) -> Option<u32> {
        match self {
            Self::ScriptViolation => Some(100),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TenantProvisioned => "tenant.provisioned",
            Self::WorkspaceCreated => "workspace.created",
            Self::NamespaceCreated => "namespace.created",
            Self::MembershipGranted => "membership.granted",
            Self::WorkflowCreated => "workflow.created",
            Self::WorkflowDraftUpdated => "workflow.draft_updated",
            Self::WorkflowPublished => "workflow.published",
            Self::RunStarted => "run.started",
            Self::RunCancelRequested => "run.cancel_requested",
            Self::RunPaused => "run.paused",
            Self::RunResumed => "run.resumed",
            Self::PayloadInspected => "payload.inspected",
            Self::PayloadInspectionDenied => "payload.inspection_denied",
            Self::RunSignalled => "run.signalled",
            Self::CrossTenantAccessDenied => "access.denied.cross_tenant",
            Self::SecretStored => "secret.stored",
            Self::SecretProvisioningStarted => "secret.provisioning_started",
            Self::DatabaseConnectionChanged => "database.connection_changed",
            Self::DatabaseResultRead => "database.result_read",
            Self::DatabaseDenied => "database.denied",
            Self::SecretDeleted => "secret.deleted",
            Self::SecretAccessGranted => "secret.access_granted",
            Self::SecretAccessRevoked => "secret.access_revoked",
            Self::SecretResolved => "secret.resolved",
            Self::SecretDenied => "secret.denied",
            Self::QuotaChanged => "quota.changed",
            Self::EgressPolicyChanged => "egress.policy_changed",
            Self::EgressDenied => "egress.denied",
            Self::RunMarkedForReview => "run.marked_for_review",
            Self::RunRerun => "run.rerun",
            Self::AuditRead => "audit.read",
            Self::WorkerDrained => "worker.drained",
            Self::WorkerUndrained => "worker.undrained",
            Self::WorkerBound => "worker.bound",
            Self::WorkerUnbound => "worker.unbound",
            Self::WorkerDenied => "worker.denied",
            Self::SagaStepRetried => "saga.step_retried",
            Self::SagaStepSkipped => "saga.step_skipped",
            Self::SagaStepResolved => "saga.step_resolved",
            Self::PublicationPolicyChanged => "publication.policy_changed",
            Self::PublicationRequested => "publication.requested",
            Self::PublicationApproved => "publication.approved",
            Self::PublicationRejected => "publication.rejected",
            Self::PublicationWithdrawn => "publication.withdrawn",
            Self::PublicationSuperseded => "publication.superseded",
            Self::PublicationDenied => "publication.denied",
            Self::WorkflowActivated => "workflow.activated",
            Self::EventEndpointCreated => "event.endpoint_created",
            Self::EventEndpointChanged => "event.endpoint_changed",
            Self::EventEndpointDeleted => "event.endpoint_deleted",
            Self::EventPublished => "event.published",
            Self::EventRejected => "event.rejected",
            Self::WebhookSubscriptionCreated => "webhook.subscription_created",
            Self::WebhookSubscriptionChanged => "webhook.subscription_changed",
            Self::WebhookSubscriptionDeleted => "webhook.subscription_deleted",
            Self::WebhookSubscriptionDisabled => "webhook.subscription_disabled",
            Self::WebhookDeliveryRedriven => "webhook.delivery_redriven",
            Self::BrokerSubscriptionCreated => "broker.subscription_created",
            Self::BrokerSubscriptionChanged => "broker.subscription_changed",
            Self::BrokerSubscriptionDeleted => "broker.subscription_deleted",
            Self::BrokerSubscriptionDisabled => "broker.subscription_disabled",
            Self::BrokerDeliveryRedriven => "broker.delivery_redriven",
            Self::ConnectorCreated => "connector.created",
            Self::ConnectorChanged => "connector.changed",
            Self::ConnectorDeleted => "connector.deleted",
            Self::ConnectorMessageDiscarded => "connector.message_discarded",
            Self::ConnectorMessageRedriven => "connector.message_redriven",
            Self::ScriptCreated => "script.created",
            Self::ScriptChanged => "script.changed",
            Self::ScriptDeleted => "script.deleted",
            Self::ScriptPublished => "script.published",
            Self::ScriptBuilt => "script.built",
            Self::ScriptTested => "script.tested",
            Self::ScriptViolation => "script.violation",
            Self::BlobUploaded => "blob.uploaded",
            Self::BlobRead => "blob.read",
            Self::BlobDeleted => "blob.deleted",
        }
    }
}

impl fmt::Display for AuditAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Kind of resource an audit record refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    Blob,
    Tenant,
    Workspace,
    Namespace,
    Membership,
    Workflow,
    WorkflowVersion,
    Run,
    Secret,
    DatabaseConnection,
    Quota,
    EgressPolicy,
    WorkerBinding,
    /// A worker identity, as `namespace-id/subject`.
    Worker,
    EventEndpoint,
    /// An event admitted to the inbox, as `source/id`.
    Event,
    WebhookSubscription,
    BrokerSubscription,
    Connector,
    Script,
    /// A version of a script, as `script-id/number`.
    ScriptVersion,
    /// A request to publish a workflow into a namespace that asks for approvals.
    PublicationRequest,
}

impl ResourceType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tenant => "tenant",
            Self::Workspace => "workspace",
            Self::Namespace => "namespace",
            Self::Membership => "membership",
            Self::Workflow => "workflow",
            Self::WorkflowVersion => "workflow_version",
            Self::Run => "run",
            Self::Secret => "secret",
            Self::DatabaseConnection => "database_connection",
            Self::Quota => "quota",
            Self::EgressPolicy => "egress_policy",
            Self::WorkerBinding => "worker_binding",
            Self::Worker => "worker",
            Self::EventEndpoint => "event_endpoint",
            Self::Event => "event",
            Self::WebhookSubscription => "webhook_subscription",
            Self::BrokerSubscription => "broker_subscription",
            Self::Connector => "connector",
            Self::Script => "script",
            Self::ScriptVersion => "script_version",
            Self::PublicationRequest => "publication_request",
        }
    }
}

impl fmt::Display for ResourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Immutable audit entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    id: AuditId,
    tenant_id: TenantId,
    actor: Subject,
    action: AuditAction,
    resource_type: ResourceType,
    resource_id: String,
    reason: Option<String>,
    trace_id: Option<String>,
    created_at: OffsetDateTime,
}

impl AuditRecord {
    /// Records `action` performed by `principal` on `resource_id`.
    #[must_use]
    pub fn by(
        principal: &Principal,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: impl Into<String>,
        trace_id: Option<String>,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            id: AuditId::generate(),
            tenant_id: principal.tenant_id(),
            actor: principal.subject().clone(),
            action,
            resource_type,
            resource_id: resource_id.into(),
            reason: None,
            trace_id,
            created_at: now,
        }
    }

    /// A second record of the same transaction, about another resource, by the same actor at the
    /// same time and in the same trace: the requests that saving a draft ended, one for each.
    #[must_use]
    pub fn follow_up(
        &self,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            id: AuditId::generate(),
            tenant_id: self.tenant_id,
            actor: self.actor.clone(),
            action,
            resource_type,
            resource_id: resource_id.into(),
            reason: Some(reason.into()),
            trace_id: self.trace_id.clone(),
            created_at: self.created_at,
        }
    }

    /// Records a platform-level operation (no tenant principal) performed by an operator or the
    /// development seed. The actor is [`Subject::provisioning`].
    #[must_use]
    pub fn provisioning(
        tenant_id: TenantId,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: impl Into<String>,
        reason: impl Into<String>,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            id: AuditId::generate(),
            tenant_id,
            actor: Subject::provisioning(),
            action,
            resource_type,
            resource_id: resource_id.into(),
            reason: Some(reason.into()),
            trace_id: None,
            created_at: now,
        }
    }

    /// Records something the execution engine did on its own initiative. The actor is
    /// [`Subject::engine`].
    #[must_use]
    pub fn engine(
        tenant_id: TenantId,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: impl Into<String>,
        reason: impl Into<String>,
        trace_id: Option<String>,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            id: AuditId::generate(),
            tenant_id,
            actor: Subject::engine(),
            action,
            resource_type,
            resource_id: resource_id.into(),
            reason: Some(reason.into()),
            trace_id,
            created_at: now,
        }
    }

    /// Records something an unauthenticated caller attempted at a public endpoint. The actor is
    /// [`Subject::gateway`]; the tenant is the one that owns the endpoint.
    #[must_use]
    pub fn gateway(
        tenant_id: TenantId,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: impl Into<String>,
        reason: impl Into<String>,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            id: AuditId::generate(),
            tenant_id,
            actor: Subject::gateway(),
            action,
            resource_type,
            resource_id: resource_id.into(),
            reason: Some(reason.into()),
            trace_id: None,
            created_at: now,
        }
    }

    /// Records something a platform operator did from the command line. The actor is
    /// [`Subject::operator`]; the operator authenticated to the host, not to the API.
    #[must_use]
    pub fn operator(
        tenant_id: TenantId,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: impl Into<String>,
        reason: impl Into<String>,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            id: AuditId::generate(),
            tenant_id,
            actor: Subject::operator(),
            action,
            resource_type,
            resource_id: resource_id.into(),
            reason: Some(reason.into()),
            trace_id: None,
            created_at: now,
        }
    }

    /// Adds a human-readable reason (must not contain secrets or personal data).
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Rebuilds a record from trusted storage.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn restore(
        id: AuditId,
        tenant_id: TenantId,
        actor: Subject,
        action: AuditAction,
        resource_type: ResourceType,
        resource_id: String,
        reason: Option<String>,
        trace_id: Option<String>,
        created_at: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            tenant_id,
            actor,
            action,
            resource_type,
            resource_id,
            reason,
            trace_id,
            created_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> AuditId {
        self.id
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }

    #[must_use]
    pub const fn action(&self) -> AuditAction {
        self.action
    }

    #[must_use]
    pub const fn resource_type(&self) -> ResourceType {
        self.resource_type
    }

    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    #[must_use]
    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
}

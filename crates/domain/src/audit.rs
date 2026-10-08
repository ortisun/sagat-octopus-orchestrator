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
    /// A signal was delivered to a waiting run.
    RunSignalled,
    /// The caller asked for a resource that exists only in another tenant. The record carries the
    /// identifier the caller supplied and nothing about the other tenant.
    CrossTenantAccessDenied,
}

impl AuditAction {
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
            Self::RunSignalled => "run.signalled",
            Self::CrossTenantAccessDenied => "access.denied.cross_tenant",
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
    Tenant,
    Workspace,
    Namespace,
    Membership,
    Workflow,
    WorkflowVersion,
    Run,
}

impl ResourceType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tenant => "tenant",
            Self::Workspace => "workspace",
            Self::Namespace => "namespace",
            Self::Membership => "membership",
            Self::Workflow => "workflow",
            Self::WorkflowVersion => "workflow_version",
            Self::Run => "run",
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

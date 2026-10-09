//! The lifecycle of a workflow definition: a mutable draft and immutable published versions.
//!
//! Workflow IRs and versions are immutable once published (`spec.md` assumptions); a run is bound to
//! one version for its whole life. The draft is an opaque document here — validating it is the job
//! of the IR codec at publish time — so this module only owns identity, ownership and numbering.
//! Atomic compare-and-swap on the draft revision is a persistence concern (the adapter performs it
//! in one statement); the domain gives the rules a name and keeps tenant isolation by construction.

use time::OffsetDateTime;

use crate::identity::{
    IdentityError, Namespace, NamespaceId, Permission, Principal, Slug, Subject, TenantId,
};
use crate::workflow::Digest;

uuid_id!(
    /// Identifier of a [`Workflow`].
    WorkflowId
);
uuid_id!(
    /// Identifier of a [`WorkflowVersion`].
    WorkflowVersionId
);

/// A named workflow inside a namespace, with its current draft revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workflow {
    id: WorkflowId,
    tenant_id: TenantId,
    namespace_id: NamespaceId,
    name: Slug,
    draft_revision: u64,
    active_version: Option<WorkflowVersionId>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl Workflow {
    /// Creates a workflow in the principal's own tenant. The draft starts at revision 1.
    ///
    /// # Errors
    /// - [`IdentityError::PermissionDenied`] unless the principal holds `definition:write`.
    /// - [`IdentityError::TenantMismatch`] when `namespace` belongs to another tenant.
    pub fn create(
        principal: &Principal,
        namespace: &Namespace,
        id: WorkflowId,
        name: Slug,
        now: OffsetDateTime,
    ) -> Result<Self, IdentityError> {
        principal.authorize(Permission::DefinitionWrite)?;
        if namespace.tenant_id() != principal.tenant_id() {
            return Err(IdentityError::TenantMismatch);
        }
        Ok(Self {
            id,
            tenant_id: principal.tenant_id(),
            namespace_id: namespace.id(),
            name,
            draft_revision: 1,
            active_version: None,
            created_at: now,
            updated_at: now,
        })
    }

    /// Rebuilds a workflow from trusted storage. Performs no authorization.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn restore(
        id: WorkflowId,
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        name: Slug,
        draft_revision: u64,
        active_version: Option<WorkflowVersionId>,
        created_at: OffsetDateTime,
        updated_at: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            tenant_id,
            namespace_id,
            name,
            draft_revision,
            active_version,
            created_at,
            updated_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> WorkflowId {
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
    pub const fn name(&self) -> &Slug {
        &self.name
    }

    #[must_use]
    pub const fn draft_revision(&self) -> u64 {
        self.draft_revision
    }

    /// The version new runs use when they do not pin one.
    #[must_use]
    pub const fn active_version(&self) -> Option<WorkflowVersionId> {
        self.active_version
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    #[must_use]
    pub const fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }
}

/// An immutable, numbered publication of a workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowVersion {
    id: WorkflowVersionId,
    tenant_id: TenantId,
    workflow_id: WorkflowId,
    number: u32,
    digest: Digest,
    published_by: Subject,
    published_at: OffsetDateTime,
}

impl WorkflowVersion {
    /// Publishes the next version of `workflow`. `number` is assigned by the store inside the same
    /// transaction (it is `max + 1` of that workflow), never by a client.
    ///
    /// # Errors
    /// - [`IdentityError::PermissionDenied`] unless the principal holds `definition:publish`.
    /// - [`IdentityError::TenantMismatch`] when `workflow` belongs to another tenant.
    pub fn publish(
        principal: &Principal,
        workflow: &Workflow,
        id: WorkflowVersionId,
        number: u32,
        digest: Digest,
        now: OffsetDateTime,
    ) -> Result<Self, IdentityError> {
        principal.authorize(Permission::DefinitionPublish)?;
        if workflow.tenant_id() != principal.tenant_id() {
            return Err(IdentityError::TenantMismatch);
        }
        Ok(Self {
            id,
            tenant_id: principal.tenant_id(),
            workflow_id: workflow.id(),
            number,
            digest,
            published_by: principal.subject().clone(),
            published_at: now,
        })
    }

    /// Rebuilds a version from trusted storage. Performs no authorization.
    #[must_use]
    pub const fn restore(
        id: WorkflowVersionId,
        tenant_id: TenantId,
        workflow_id: WorkflowId,
        number: u32,
        digest: Digest,
        published_by: Subject,
        published_at: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            tenant_id,
            workflow_id,
            number,
            digest,
            published_by,
            published_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> WorkflowVersionId {
        self.id
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }

    #[must_use]
    pub const fn number(&self) -> u32 {
        self.number
    }

    #[must_use]
    pub const fn digest(&self) -> &Digest {
        &self.digest
    }

    #[must_use]
    pub const fn published_by(&self) -> &Subject {
        &self.published_by
    }

    #[must_use]
    pub const fn published_at(&self) -> OffsetDateTime {
        self.published_at
    }
}

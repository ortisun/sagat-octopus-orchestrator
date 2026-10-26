//! Transaction boundary for durable workflow approval progress.
//!
//! Public transports stay outside this module. The store must re-read current membership before
//! each decision, serialize on the run and request, and commit the canonical run journal,
//! projection, timer, audit, route and outbox as one unit.

use std::future::Future;

use sagat_domain::audit::AuditRecord;
use sagat_domain::execution::{
    ApprovalDecisionCommand, ApprovalDecisionResolution, ApprovalError, ApprovalReceiptId,
    ApprovalRequest, ApprovalRequestId, ApprovalStatus, ApprovalTransition, ExecutionInstanceId,
    RunId,
};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::NodeId;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;
use crate::primitives::SchedulerAuthority;

/// Immutable outcome routes captured from the published definition when a request opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRoutes {
    approved: NodeId,
    rejected: NodeId,
    expired: NodeId,
}

impl ApprovalRoutes {
    #[must_use]
    pub const fn new(approved: NodeId, rejected: NodeId, expired: NodeId) -> Self {
        Self {
            approved,
            rejected,
            expired,
        }
    }

    #[must_use]
    pub const fn approved(&self) -> &NodeId {
        &self.approved
    }

    #[must_use]
    pub const fn rejected(&self) -> &NodeId {
        &self.rejected
    }

    #[must_use]
    pub const fn expired(&self) -> &NodeId {
        &self.expired
    }
}

/// A request activation and every non-sensitive record that must commit with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalOpening {
    request: ApprovalRequest,
    routes: ApprovalRoutes,
    scheduler: SchedulerAuthority,
    audit: AuditRecord,
    outbox_id: Uuid,
    occurred_at: OffsetDateTime,
}

impl ApprovalOpening {
    #[must_use]
    pub const fn new(
        request: ApprovalRequest,
        routes: ApprovalRoutes,
        scheduler: SchedulerAuthority,
        audit: AuditRecord,
        outbox_id: Uuid,
        occurred_at: OffsetDateTime,
    ) -> Self {
        Self {
            request,
            routes,
            scheduler,
            audit,
            outbox_id,
            occurred_at,
        }
    }

    #[must_use]
    pub const fn request(&self) -> &ApprovalRequest {
        &self.request
    }

    #[must_use]
    pub const fn routes(&self) -> &ApprovalRoutes {
        &self.routes
    }

    #[must_use]
    pub const fn scheduler(&self) -> &SchedulerAuthority {
        &self.scheduler
    }

    #[must_use]
    pub const fn audit(&self) -> &AuditRecord {
        &self.audit
    }

    #[must_use]
    pub const fn outbox_id(&self) -> Uuid {
        self.outbox_id
    }

    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at
    }
}

/// Tenant-scoped address used before a request is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalAddress {
    tenant_id: TenantId,
    namespace_id: NamespaceId,
    request_id: ApprovalRequestId,
}

/// Non-sensitive approval data visible to callers that can read the owning run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalListItem {
    request_id: ApprovalRequestId,
    run_id: RunId,
    owner: ExecutionInstanceId,
    opened_sequence: u64,
    deadline: OffsetDateTime,
    status: ApprovalStatus,
    settled_at: Option<OffsetDateTime>,
}

/// A page bound enforced before the persistence adapter is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalPageSize(u32);

impl ApprovalPageSize {
    pub const DEFAULT: Self = Self(50);
    pub const MAX: u32 = 201;

    #[must_use]
    pub const fn new(requested: u32) -> Self {
        if requested == 0 {
            Self(1)
        } else if requested > Self::MAX {
            Self(Self::MAX)
        } else {
            Self(requested)
        }
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl ApprovalListItem {
    #[must_use]
    pub fn from_request(request: &ApprovalRequest) -> Self {
        Self {
            request_id: request.id(),
            run_id: request.run_id(),
            owner: request.owner().clone(),
            opened_sequence: request.opened_sequence(),
            deadline: request.deadline(),
            status: request.status(),
            settled_at: request.settled_at(),
        }
    }

    #[must_use]
    pub const fn request_id(&self) -> ApprovalRequestId {
        self.request_id
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
    pub const fn settled_at(&self) -> Option<OffsetDateTime> {
        self.settled_at
    }
}

impl ApprovalAddress {
    #[must_use]
    pub const fn new(
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        request_id: ApprovalRequestId,
    ) -> Self {
        Self {
            tenant_id,
            namespace_id,
            request_id,
        }
    }

    #[must_use]
    pub const fn tenant_id(self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn namespace_id(self) -> NamespaceId {
        self.namespace_id
    }

    #[must_use]
    pub const fn request_id(self) -> ApprovalRequestId {
        self.request_id
    }
}

/// One authenticated decision attempt. The adapter resolves the actor's current role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDecisionAttempt {
    address: ApprovalAddress,
    actor: Subject,
    receipt_id: ApprovalReceiptId,
    command: ApprovalDecisionCommand,
    audit: AuditRecord,
    outbox_id: Uuid,
    occurred_at: OffsetDateTime,
}

impl ApprovalDecisionAttempt {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        address: ApprovalAddress,
        actor: Subject,
        receipt_id: ApprovalReceiptId,
        command: ApprovalDecisionCommand,
        audit: AuditRecord,
        outbox_id: Uuid,
        occurred_at: OffsetDateTime,
    ) -> Self {
        Self {
            address,
            actor,
            receipt_id,
            command,
            audit,
            outbox_id,
            occurred_at,
        }
    }

    #[must_use]
    pub const fn address(&self) -> ApprovalAddress {
        self.address
    }

    #[must_use]
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }

    #[must_use]
    pub const fn receipt_id(&self) -> ApprovalReceiptId {
        self.receipt_id
    }

    #[must_use]
    pub const fn command(&self) -> &ApprovalDecisionCommand {
        &self.command
    }

    #[must_use]
    pub const fn audit(&self) -> &AuditRecord {
        &self.audit
    }

    #[must_use]
    pub const fn outbox_id(&self) -> Uuid {
        self.outbox_id
    }

    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at
    }
}

/// Internal timer delivery. Duplicate or losing expiry deliveries are committed no-ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalExpiryAttempt {
    address: ApprovalAddress,
    scheduler: SchedulerAuthority,
    audit: AuditRecord,
    outbox_id: Uuid,
    occurred_at: OffsetDateTime,
}

impl ApprovalExpiryAttempt {
    #[must_use]
    pub const fn new(
        address: ApprovalAddress,
        scheduler: SchedulerAuthority,
        audit: AuditRecord,
        outbox_id: Uuid,
        occurred_at: OffsetDateTime,
    ) -> Self {
        Self {
            address,
            scheduler,
            audit,
            outbox_id,
            occurred_at,
        }
    }

    #[must_use]
    pub const fn address(&self) -> ApprovalAddress {
        self.address
    }

    #[must_use]
    pub const fn scheduler(&self) -> &SchedulerAuthority {
        &self.scheduler
    }

    #[must_use]
    pub const fn audit(&self) -> &AuditRecord {
        &self.audit
    }

    #[must_use]
    pub const fn outbox_id(&self) -> Uuid {
        self.outbox_id
    }

    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalProgressError {
    #[error("approval request was not found")]
    NotFound,
    #[error("approval audit evidence does not match the transition")]
    AuditMismatch,
    #[error("approval projection does not match its canonical run journal")]
    CorruptProjection,
    #[error("approval runtime support is not available")]
    RuntimeUnavailable,
    #[error("scheduler authority no longer owns the run shard")]
    SchedulerFenceLost,
    #[error("approval transition rejected: {0}")]
    Approval(#[from] ApprovalError),
    #[error("approval storage failed: {0}")]
    Storage(#[from] StoreError),
}

/// Durable boundary for opening, deciding and expiring a workflow approval.
pub trait ApprovalProgressStore: Send + Sync {
    fn open(
        &self,
        opening: &ApprovalOpening,
    ) -> impl Future<Output = Result<ApprovalRequest, ApprovalProgressError>> + Send;

    fn decide(
        &self,
        attempt: &ApprovalDecisionAttempt,
    ) -> impl Future<Output = Result<ApprovalDecisionResolution, ApprovalProgressError>> + Send;

    fn expire(
        &self,
        attempt: &ApprovalExpiryAttempt,
    ) -> impl Future<Output = Result<ApprovalTransition, ApprovalProgressError>> + Send;

    fn load(
        &self,
        address: ApprovalAddress,
    ) -> impl Future<Output = Result<ApprovalRequest, ApprovalProgressError>> + Send;

    fn list_for_run(
        &self,
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        run_id: RunId,
        after_opened_sequence: Option<u64>,
        limit: ApprovalPageSize,
    ) -> impl Future<Output = Result<Vec<ApprovalListItem>, ApprovalProgressError>> + Send;
}

//! Scheduler-facing orchestration for durable human approvals.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{
    ApprovalPolicy, ApprovalRequest, ApprovalRequestId, ExecutionInstanceId, RunId, StepStatus,
};
use sagat_domain::identity::{Subject, TenantId};
use sagat_domain::workflow::NodeKind;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::approval_progress::{
    ApprovalAddress, ApprovalExpiryAttempt, ApprovalOpening, ApprovalProgressError,
    ApprovalProgressStore, ApprovalRoutes,
};
use crate::error::StoreError;
use crate::execution::RunTx;
use crate::maintenance::{StepRef, Window};
use crate::primitives::SchedulerAuthority;
use crate::runs::{Lock, RunStore};

/// A pending approval timer discovered without taking the run lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueApproval {
    address: ApprovalAddress,
    run_id: RunId,
    trace_id: Option<String>,
    as_of: OffsetDateTime,
}

impl DueApproval {
    #[must_use]
    pub const fn new(
        address: ApprovalAddress,
        run_id: RunId,
        trace_id: Option<String>,
        as_of: OffsetDateTime,
    ) -> Self {
        Self {
            address,
            run_id,
            trace_id,
            as_of,
        }
    }

    #[must_use]
    pub const fn address(&self) -> ApprovalAddress {
        self.address
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    #[must_use]
    pub const fn as_of(&self) -> OffsetDateTime {
        self.as_of
    }
}

/// Candidate discovery plus the transactional approval boundary used by the scheduler.
pub trait ApprovalSchedulerStore: RunStore + ApprovalProgressStore {
    fn approval_candidates(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    fn due_approvals(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<DueApproval>, StoreError>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAdvance {
    Opened { request_id: ApprovalRequestId },
    Expired,
    Skipped,
}

#[derive(Debug, thiserror::Error)]
pub enum ApprovalAdvanceError {
    #[error("the run does not exist")]
    MissingRun,
    #[error("the pending instance is not an approval")]
    NotApproval,
    #[error("approval definition or run projection is invalid")]
    InvalidProgress,
    #[error("approval storage failed: {0}")]
    Progress(#[from] ApprovalProgressError),
    #[error("run storage failed: {0}")]
    Store(#[from] StoreError),
}

impl ApprovalAdvanceError {
    #[must_use]
    pub const fn scheduler_fence_lost(&self) -> bool {
        matches!(
            self,
            Self::Progress(ApprovalProgressError::SchedulerFenceLost)
        )
    }
}

/// Builds approval transitions only from the immutable stored definition and locked run snapshot.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalScheduler<'a, S> {
    store: &'a S,
}

impl<'a, S> ApprovalScheduler<'a, S>
where
    S: ApprovalSchedulerStore,
{
    #[must_use]
    pub const fn new(store: &'a S) -> Self {
        Self { store }
    }

    /// Opens a request for a pending approval instance. The progress adapter re-locks the run and
    /// compares its version, so a concurrent scheduler can only commit one request.
    pub async fn open(
        &self,
        tenant_id: TenantId,
        run_id: RunId,
        owner: &ExecutionInstanceId,
        scheduler: SchedulerAuthority,
    ) -> Result<ApprovalAdvance, ApprovalAdvanceError> {
        let Some(mut tx) = self.store.begin(tenant_id, run_id, Lock::Wait).await? else {
            return Err(ApprovalAdvanceError::MissingRun);
        };
        let info = tx.info().clone();
        let occurred_at = tx.now();
        let (definition, state) = tx.parts();
        if state.instance(owner).map(|step| step.status()) != Some(StepStatus::Pending) {
            return Ok(ApprovalAdvance::Skipped);
        }
        let node = definition
            .node_for_instance(owner)
            .ok_or(ApprovalAdvanceError::InvalidProgress)?;
        let NodeKind::Approval {
            policy,
            on_approve,
            on_reject,
            on_expire,
        } = node.kind()
        else {
            return Err(ApprovalAdvanceError::NotApproval);
        };
        let timeout = node
            .timeout()
            .ok_or(ApprovalAdvanceError::InvalidProgress)?;
        let timeout =
            time::Duration::try_from(timeout).map_err(|_| ApprovalAdvanceError::InvalidProgress)?;
        let deadline = occurred_at
            .checked_add(timeout)
            .ok_or(ApprovalAdvanceError::InvalidProgress)?;
        let opened_sequence = state
            .version()
            .checked_add(1)
            .ok_or(ApprovalAdvanceError::InvalidProgress)?;
        let policy: ApprovalPolicy = policy.clone();
        let routes = ApprovalRoutes::new(on_approve.clone(), on_reject.clone(), on_expire.clone());
        drop(tx);

        let request = ApprovalRequest::open(
            ApprovalRequestId::generate(),
            tenant_id,
            info.namespace_id,
            run_id,
            owner.clone(),
            policy,
            Subject::parse(&info.started_by).map_err(|_| ApprovalAdvanceError::InvalidProgress)?,
            opened_sequence,
            deadline,
        );
        let audit = AuditRecord::engine(
            tenant_id,
            AuditAction::ApprovalOpened,
            ResourceType::ApprovalRequest,
            request.id().to_string(),
            "approval opened",
            info.trace_id,
            occurred_at,
        );
        let opening = ApprovalOpening::new(
            request,
            routes,
            scheduler,
            audit,
            Uuid::now_v7(),
            occurred_at,
        );
        match self.store.open(&opening).await {
            Ok(request) => Ok(ApprovalAdvance::Opened {
                request_id: request.id(),
            }),
            Err(ApprovalProgressError::RuntimeUnavailable) => Ok(ApprovalAdvance::Skipped),
            Err(error) => Err(error.into()),
        }
    }

    /// Fires a due request using the database timestamp captured by candidate discovery. The
    /// progress adapter rechecks both the shard fence and first-commit outcome under locks.
    pub async fn expire(
        &self,
        candidate: &DueApproval,
        scheduler: SchedulerAuthority,
    ) -> Result<ApprovalAdvance, ApprovalAdvanceError> {
        let audit = AuditRecord::engine(
            candidate.address().tenant_id(),
            AuditAction::ApprovalExpired,
            ResourceType::ApprovalRequest,
            candidate.address().request_id().to_string(),
            "approval expired",
            candidate.trace_id.clone(),
            candidate.as_of(),
        );
        match self
            .store
            .expire(&ApprovalExpiryAttempt::new(
                candidate.address(),
                scheduler,
                audit,
                Uuid::now_v7(),
                candidate.as_of(),
            ))
            .await
        {
            Ok(transition) if transition.changed() => Ok(ApprovalAdvance::Expired),
            Ok(_) | Err(ApprovalProgressError::RuntimeUnavailable) => Ok(ApprovalAdvance::Skipped),
            Err(error) => Err(error.into()),
        }
    }
}

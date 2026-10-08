//! Ports of the background work that moves runs forward without any caller: timers, wait
//! deadlines, retry backoff, expired leases, run deadlines, lost wake-up hints and the outbox
//! (`FR-002-004`, `FR-002-006`, `FR-002-007`).
//!
//! Every query here is a cheap index scan that only *finds candidates*, without locking. The
//! authority is the run's row lock: the scheduler re-checks the condition inside the locked
//! transaction through the domain commands, so a candidate that someone else already handled is
//! a harmless no-op.

use std::future::Future;
use std::time::Duration;

use sagat_domain::execution::{Fence, RunId, RunStatus};
use sagat_domain::identity::TenantId;
use sagat_domain::workflow::NodeId;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;

/// A step found by a maintenance query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRef {
    pub tenant_id: TenantId,
    pub run_id: RunId,
    pub node: NodeId,
}

/// A run found by a maintenance query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRef {
    pub tenant_id: TenantId,
    pub run_id: RunId,
}

/// A child run that was reserved by a `child` step and does not exist yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingChild {
    pub step: StepRef,
    pub child: RunId,
    /// The input the child starts with (evaluated when the step was reserved).
    pub input: Vec<u8>,
    /// The database clock when the candidate was read.
    pub as_of: OffsetDateTime,
}

/// A child run that reached a terminal status while its parent step still waits for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedChild {
    pub tenant_id: TenantId,
    pub child: RunId,
    pub parent: RunId,
    pub node: NodeId,
    pub status: RunStatus,
}

/// Why a leased attempt is considered over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseLoss {
    /// The worker stopped heartbeating.
    Expired,
    /// The attempt exceeded the node's `timeout`.
    TimedOut,
}

/// A leased attempt that is over, with the fence it held when it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostLease {
    pub step: StepRef,
    pub fence: Fence,
    pub cause: LeaseLoss,
}

/// Candidate queries and housekeeping. Implemented by `sagat-infra-postgres`; every time
/// comparison uses the database clock, so instances with skewed clocks agree.
pub trait MaintenanceStore: Send + Sync {
    /// Timers whose due time has passed, oldest first.
    fn due_timers(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    /// Event waits whose deadline has passed.
    fn due_wait_deadlines(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    /// Steps in retry backoff whose delay has elapsed.
    fn due_retries(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    /// Leased attempts whose lease expired or whose attempt deadline passed.
    fn lost_leases(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<LostLease>, StoreError>> + Send;

    /// Unfinished runs past their deadline.
    fn overdue_runs(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<RunRef>, StoreError>> + Send;

    /// `READY` steps that nobody was notified about for `older_than` (the notification may have
    /// been lost with the broker; claiming does not depend on it, but latency does).
    fn stale_hints(
        &self,
        older_than: Duration,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    /// Child runs that `child` steps reserved and nobody has created yet, oldest first.
    fn pending_children(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<PendingChild>, StoreError>> + Send;

    /// Child runs that ended while the parent step that waits for them has not been told.
    fn ended_children(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<EndedChild>, StoreError>> + Send;

    /// Running child runs whose parent was stopped under a policy that cancels them and that have
    /// not been asked to stop yet.
    fn children_to_cancel(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<RunRef>, StoreError>> + Send;

    /// Deletes idempotency records older than `older_than`; returns how many.
    fn purge_idempotency(
        &self,
        older_than: Duration,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Deletes expired worker sessions; returns how many.
    fn purge_sessions(&self) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Deletes up to `limit` outbox messages delivered more than `older_than` ago; returns how
    /// many. Undelivered messages are never deleted.
    fn purge_outbox(
        &self,
        older_than: Duration,
        limit: u32,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;
}

/// Why a message could not be published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PublishError {
    /// The broker cannot be reached or did not acknowledge in time; try again later.
    #[error("the broker is unavailable")]
    Unavailable,
    /// The broker or the adapter will never accept this message as it is.
    #[error("the message was refused: {0}")]
    Rejected(&'static str),
}

impl PublishError {
    /// A short stable code for metrics and the outbox's `last_error_code`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable => "BROKER_UNAVAILABLE",
            Self::Rejected(_) => "REJECTED",
        }
    }
}

/// The broker a dispatcher publishes to (NATS JetStream in production).
///
/// Publishing is at-least-once: `message_id` is stable across attempts so the broker can drop the
/// duplicates a crash between publishing and confirming produces.
pub trait EventPublisher: Send + Sync {
    /// # Errors
    /// [`PublishError::Unavailable`] when the broker did not confirm; [`PublishError::Rejected`]
    /// for a message it can never accept.
    fn publish(
        &self,
        subject: &str,
        message_id: &str,
        payload: &[u8],
    ) -> impl Future<Output = Result<(), PublishError>> + Send;
}

/// A message waiting to be published to the broker.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxMessage {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub aggregate_id: Uuid,
    pub event_type: String,
    pub payload: Vec<u8>,
    pub attempt_count: u32,
    pub created_at: OffsetDateTime,
}

/// The transactional outbox, read by a dispatcher. Delivery is at-least-once: a message claimed but
/// never confirmed becomes visible again after `visibility`, so a dispatcher that crashes after
/// publishing and before confirming causes a duplicate, never a loss.
pub trait OutboxStore: Send + Sync {
    /// Claims up to `limit` due messages by pushing their next attempt `visibility` into the future.
    fn claim_outbox(
        &self,
        limit: u32,
        visibility: Duration,
    ) -> impl Future<Output = Result<Vec<OutboxMessage>, StoreError>> + Send;

    fn mark_delivered(&self, ids: &[Uuid]) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Records a failed attempt and schedules the next one after `retry_in`.
    fn mark_failed(
        &self,
        id: Uuid,
        code: &str,
        retry_in: Duration,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Messages not yet delivered (an operations gauge).
    fn outbox_backlog(&self) -> impl Future<Output = Result<u64, StoreError>> + Send;
}

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

use sagat_domain::execution::{Fence, RunId};
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

    /// Deletes idempotency records older than `older_than`; returns how many.
    fn purge_idempotency(
        &self,
        older_than: Duration,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Deletes expired worker sessions; returns how many.
    fn purge_sessions(&self) -> impl Future<Output = Result<u64, StoreError>> + Send;
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

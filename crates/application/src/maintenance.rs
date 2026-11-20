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

use sagat_domain::execution::{ExecutionInstanceId, Fence, InstancePath, RunId, RunStatus};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::sharding::ShardSet;
use sagat_domain::workflow::NodeId;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;

/// What a candidate query is asked: which shards to look at (those the replica owns), which tenants
/// to leave out (those that already have their share of this pass: `FR-010-002`), and how many.
#[derive(Debug, Clone)]
pub struct Window {
    pub shards: ShardSet,
    pub exclude: Vec<TenantId>,
    pub limit: u32,
}

impl Window {
    /// Every shard, no tenant left out: what a single engine, and the tests that do not care about
    /// ownership, ask.
    #[must_use]
    pub const fn everything(limit: u32) -> Self {
        Self {
            shards: ShardSet::all(),
            exclude: Vec::new(),
            limit,
        }
    }
}

/// A step found by a maintenance query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRef {
    pub tenant_id: TenantId,
    pub run_id: RunId,
    pub node: NodeId,
    pub instance_path: InstancePath,
}

impl StepRef {
    #[must_use]
    pub fn instance(&self) -> ExecutionInstanceId {
        ExecutionInstanceId::in_path(self.instance_path.clone(), self.node.clone())
    }
}

/// A pending top-level continuation boundary tied to its serialized chain head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingContinuation {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub chain_id: RunId,
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

/// What one call of the purge of batch jobs removed: the items, then the jobs whose items were all gone
/// (`FR-018-033`). The two are counted apart because a job is one row and its items may be many, and an
/// operator reads them apart (`batch_purged_total{kind}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchPurged {
    pub jobs: u64,
    pub items: u64,
}

impl BatchPurged {
    /// Rows removed, the unit the bound of a call is counted in.
    #[must_use]
    pub const fn rows(self) -> u64 {
        self.jobs + self.items
    }
}

/// Candidate queries and housekeeping. Implemented by `sagat-infra-postgres`; every time
/// comparison uses the database clock, so instances with skewed clocks agree.
pub trait MaintenanceStore: Send + Sync {
    /// Feature 022 (T26): the step keys with waiters whose next-eligible hint has come, on the home shards of `window`,
    /// oldest hint first. Only finds candidates; each grant is decided again under the waiter's run and the key's lock.
    fn due_gate_keys(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<crate::flow_control::GateKey>, StoreError>> + Send;

    /// The waiters of `key`, oldest first by the commit that queued them (ties by run, then node), at most `limit`.
    fn gate_key_waiters(
        &self,
        key: &crate::flow_control::GateKey,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<crate::flow_control::GateWaiter>, StoreError>> + Send;

    /// Clears the next-eligible hint of `key` when it is still the one the pass read and `holder` (when given) still holds
    /// the key's home shard: a pass that could grant none of its waiters leaves the key to the next release, queue, pause
    /// or resume, which sets the hint again (early, never late); a window rests until its oldest counted grant ages out.
    fn rest_gate_key(
        &self,
        key: &crate::flow_control::GateKey,
        holder: Option<(&str, crate::shards::Lease)>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Feature 022 (transaction T23, T022-074): copies the lease instant of the attempt a slot was granted for onto the granted,
    /// unleased step grants of the run shards of `window` (at most `window.limit` a shard): the instant of that attempt's
    /// `step.leased`, journaled after the grant (D022-029), or the grant's own instant when later. Locks the grant rows
    /// only (no run, no window: the key's hint never waits for it, D022-021). Returns how many it recorded.
    fn record_rate_leases(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Feature 022 (transaction T25): deletes leased grants that no acquirer can count any more, those leased longer ago than their
    /// window row's longest period plus one minute (FR-022-056): at most `keys` keys a call and `limit` grants a key
    /// (so at most `keys × limit` rows), each key in a transaction of its own that takes its window row first and skips a
    /// key another transaction holds. Returns how many grant rows it deleted.
    fn purge_rate_grants(
        &self,
        keys: u32,
        limit: u32,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Feature 022 (T27): queued steps whose queue deadline passed, on the run shards of `window`, earliest first.
    fn due_gate_waits(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<crate::flow_control::GateWaiter>, StoreError>> + Send;

    /// Pending continuation boundaries whose run is still the current chain head.
    fn continuation_candidates(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<PendingContinuation>, StoreError>> + Send;

    /// Collection owners that can enter, admit, checkpoint or finish without waiting for worker
    /// progress. The query only discovers candidates; the fenced transition revalidates them.
    fn collection_candidates(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    /// Timers whose due time has passed, oldest first.
    fn due_timers(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    /// Event waits whose deadline has passed.
    fn due_wait_deadlines(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    /// Steps in retry backoff whose delay has elapsed.
    fn due_retries(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    /// Leased attempts whose lease expired or whose attempt deadline passed.
    fn lost_leases(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<LostLease>, StoreError>> + Send;

    /// Runs whose start delay has ended and whose entry is still held (`WAITING`), oldest first
    /// (feature 018, `FR-018-023`). The query only discovers candidates; the fenced release
    /// revalidates each one under the run's lock.
    fn due_start_releases(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<RunRef>, StoreError>> + Send;

    /// Holds a delayed start whose release a flow-control ceiling refused (feature 022, D022-052, D022-054) back from
    /// [`Self::due_start_releases`] until a retry instant that backs off (5 s doubling to 5 min), in a transaction of its
    /// own, so that refused releases neither take every pass's batch nor hide the starts behind them. `reason` is the
    /// refusal's closed `details.reason`. Returns how many times the run has now been deferred.
    fn defer_start_release(
        &self,
        run: RunRef,
        reason: &'static str,
    ) -> impl Future<Output = Result<u32, StoreError>> + Send;

    /// Unfinished runs past their deadline.
    fn overdue_runs(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<RunRef>, StoreError>> + Send;

    /// Runs that were told to stop by a durable transaction of their own (a loop that reached its
    /// iteration limit) and that no attempt holds any more: the planner still has to drain and end
    /// them, and no later event would.
    fn stopped_runs(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<RunRef>, StoreError>> + Send;

    /// `READY` steps that nobody was notified about for `older_than` (the notification may have
    /// been lost with the broker; claiming does not depend on it, but latency does).
    fn stale_hints(
        &self,
        older_than: Duration,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<StepRef>, StoreError>> + Send;

    /// Child runs that `child` steps reserved and nobody has created yet, oldest first.
    fn pending_children(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<PendingChild>, StoreError>> + Send;

    /// Child runs that ended while the parent step that waits for them has not been told.
    fn ended_children(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<EndedChild>, StoreError>> + Send;

    /// The result of a run that ended (G21): the output of the `end` step that finished it, or
    /// `None` when that `end` declares no `outputExpr`.
    fn run_result(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, StoreError>> + Send;

    /// Running child runs whose parent was stopped under a policy that cancels them and that have
    /// not been asked to stop yet.
    fn children_to_cancel(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<RunRef>, StoreError>> + Send;

    /// Deletes idempotency records older than `older_than`; returns how many.
    fn purge_idempotency(
        &self,
        older_than: Duration,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Deletes expired worker sessions; returns how many.
    fn purge_sessions(&self) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Deletes up to `limit` of the records scripts keep for a while — executions whose retention
    /// ended, test runs finished longer ago than their script's retention — returns how many.
    fn purge_script_records(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Deletes up to `rows` actions of schedules that are past their retention, looking at up to
    /// `schedules` schedules, least recently visited first; returns how many. An action is kept
    /// while it is among the last 1,000 of its schedule or younger than 30 days, and a waiting
    /// (`BUFFERED`) action is never deleted (`FR-018-013`).
    fn purge_schedule_actions(
        &self,
        schedules: u32,
        rows: u32,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Deletes up to `rows` bindings of business identifiers that are free again: the holder
    /// closed and its window passed, or the holder is gone; returns how many. A binding whose
    /// holder is open, or closed inside its window, is never deleted (`FR-018-018`).
    fn purge_business_ids(&self, rows: u32)
    -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Deletes the items and then the rows of batch jobs that are past their retention, oldest
    /// window first, at most `jobs` jobs and `rows` rows (items and jobs together) in one call; a job
    /// that the row bound cut short keeps its terminal state and the items it has left, and the next
    /// call goes on with it. A job that has not finished, or that ended or is inside its window, is
    /// never deleted, and neither are its items (`FR-018-033`). The error carries no text of the
    /// database: a job holds customer text (a selector, a reason), and an error can quote it.
    fn purge_batch_jobs(
        &self,
        jobs: u32,
        rows: u32,
    ) -> impl Future<Output = Result<BatchPurged, StoreError>> + Send;

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

/// The configured internal notification transport (NATS JetStream or Redis Streams).
///
/// Publishing is at-least-once: `message_id` is a stable application id across attempts.
/// A transport may deduplicate within a window; consumers must tolerate physical duplicates.
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
    /// Narrows the subject the message is published on (the activity of a `task.ready` hint).
    pub routing_key: Option<String>,
    pub payload: Vec<u8>,
    pub attempt_count: u32,
    pub created_at: OffsetDateTime,
}

/// A message that is due and has not been claimed: what a dispatcher looks at before choosing which
/// to take (`FR-010-002`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboxDue {
    pub id: Uuid,
    pub tenant_id: TenantId,
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

    /// The oldest due messages of the shards in `window`, without claiming them: what a dispatcher
    /// that owns some shards chooses from. Messages of one aggregate (a run) are in one shard.
    fn due_outbox(
        &self,
        window: &Window,
    ) -> impl Future<Output = Result<Vec<OutboxDue>, StoreError>> + Send;

    /// Claims those of `ids` that are still due and that no other dispatcher has claimed, as
    /// [`Self::claim_outbox`] does for the oldest.
    fn claim_outbox_ids(
        &self,
        ids: &[Uuid],
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

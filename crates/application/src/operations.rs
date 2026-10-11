//! Operations: the alerts for conditions that need a person, and what an operator is told about the
//! platform around their runs (FR-008-003, FR-008-005).
//!
//! The monitor is a loop in the engine ([`MonitorStore`] is what it needs); the service here is what
//! the API reads through. A condition is found by an aggregate query that returns counts and ages
//! per namespace, reconciled against the alerts already on record: new ones begin to fire, ones no
//! longer seen are resolved, and each change is written with the obligation to tell the namespace's
//! subscribers.

use std::future::Future;
use std::time::Duration;

use sagat_domain::identity::{NamespaceId, Permission, Principal, TenantId};
use sagat_domain::operations::{AlertKind, AlertState, MAX_SUBJECT_CHARS, Severity};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::{MAX_PAGE_SIZE, RequestContext};

/// How long a condition must last before it is an alert. Each is a count of seconds the platform
/// should never keep a thing waiting for; the defaults are several times what a healthy deployment
/// needs, so an alert is a condition and not a blip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    /// A message of the outbox undelivered for this long.
    pub outbox_age: Duration,
    /// An attempt whose lease ran out this long ago and was not taken back.
    pub lease_grace: Duration,
    /// A step ready for this long that no worker has taken.
    pub unclaimed: Duration,
    /// A run waiting for a person for this long.
    pub review: Duration,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            outbox_age: Duration::from_secs(120),
            lease_grace: Duration::from_secs(60),
            unclaimed: Duration::from_secs(300),
            review: Duration::from_secs(900),
        }
    }
}

impl Thresholds {
    /// The threshold of `kind`; dead items have none: any of them is a condition.
    #[must_use]
    pub const fn of(&self, kind: AlertKind) -> Duration {
        match kind {
            AlertKind::OutboxStuck => self.outbox_age,
            AlertKind::LeaseNotReaped => self.lease_grace,
            AlertKind::StepUnclaimed => self.unclaimed,
            AlertKind::ReviewPending => self.review,
            AlertKind::DeadItems => Duration::ZERO,
        }
    }
}

/// Most alerts of one kind kept for one namespace in one look: a namespace with a thousand
/// activities nobody serves has a few alerts and not a thousand.
pub const MAX_PER_NAMESPACE: u32 = 25;
/// Most groups one detection query returns.
pub const MAX_CANDIDATES: u32 = 1000;

/// A condition seen in a namespace.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub kind: AlertKind,
    /// An activity name, the kind of dead item; empty when the kind says it all.
    pub subject: String,
    /// How many things are in the condition.
    pub items: u64,
    /// The age of the oldest of them, in seconds.
    pub oldest_seconds: f64,
}

impl Candidate {
    /// The subject cut to what an alert keeps.
    #[must_use]
    pub fn bounded_subject(subject: &str) -> String {
        subject.chars().take(MAX_SUBJECT_CHARS).collect()
    }
}

/// An alert as the API shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertRecord {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub kind: AlertKind,
    pub subject: String,
    pub state: AlertState,
    pub severity: Severity,
    pub first_seen_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    pub resolved_at: Option<OffsetDateTime>,
    pub fired_count: u32,
    pub items: u64,
    pub oldest_seconds: f64,
}

impl AlertRecord {
    /// Where the runbook for this alert is: `base` is a URL or a path to `alerts.md`.
    #[must_use]
    pub fn runbook(&self, base: &str) -> String {
        format!(
            "{}#{}",
            base.trim_end_matches('#'),
            self.kind.runbook_anchor()
        )
    }
}

/// What reconciling the alerts of one kind changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reconciled {
    pub fired: Vec<AlertRecord>,
    pub resolved: Vec<AlertRecord>,
}

/// A look at the platform's queues, for gauges: the totals the dashboards draw (FR-008-003).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    /// Undelivered outbox messages and the age of the oldest, in seconds.
    pub outbox_backlog: u64,
    pub outbox_oldest_seconds: f64,
    /// Steps ready for a worker and the age of the oldest, in seconds.
    pub ready_steps: u64,
    pub ready_oldest_seconds: f64,
    /// Attempts in flight, and how many of them are past their lease.
    pub leases_active: u64,
    pub leases_expired: u64,
    /// Events waiting for the dispatcher and the age of the oldest.
    pub inbox_pending: u64,
    pub inbox_oldest_seconds: f64,
    /// Things that ran out of attempts, by kind (`events`, `webhooks`, `connectors`).
    pub dead_events: u64,
    pub dead_deliveries: u64,
    pub dead_letters: u64,
    /// Runs in each live status.
    pub runs_by_status: Vec<(String, u64)>,
    /// Runs in the middle of compensating.
    pub compensating_runs: u64,
    /// Worker sessions that have not expired.
    pub workers: u64,
}

/// What the monitor needs from the store.
pub trait MonitorStore: Send + Sync {
    /// Takes, or renews, the right to monitor: one process does at a time.
    fn acquire_monitor(&self) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The namespaces in the condition `kind`, with counts and ages. At most
    /// [`MAX_PER_NAMESPACE`] groups of a namespace and [`MAX_CANDIDATES`] in all.
    fn detect(
        &self,
        kind: AlertKind,
        thresholds: &Thresholds,
    ) -> impl Future<Output = Result<Vec<Candidate>, StoreError>> + Send;

    /// Makes the alerts of `kind` match what was seen: a candidate with no alert (or a resolved one)
    /// begins to fire; a firing alert with no candidate is resolved. Every change is written with
    /// the obligation to tell the namespace's subscribers, in the same transaction.
    fn reconcile(
        &self,
        kind: AlertKind,
        seen: &[Candidate],
        runbook_base: &str,
    ) -> impl Future<Output = Result<Reconciled, StoreError>> + Send;

    /// Totals of the platform's queues.
    fn sample(&self) -> impl Future<Output = Result<Sample, StoreError>> + Send;

    /// Removes the alerts that were resolved more than `retention` ago.
    fn purge_alerts(
        &self,
        retention: Duration,
        limit: u32,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;
}

/// What the API reads alerts through.
pub trait AlertReader: NamespaceDirectory {
    /// The alerts of a namespace, newest first.
    fn list_alerts(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        state: Option<AlertState>,
        before: Option<Uuid>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<AlertRecord>, StoreError>> + Send;
}

/// Reads alerts for the API.
pub struct OperationsService<S, C> {
    store: S,
    clock: C,
    runbook_base: String,
}

impl<S, C> std::fmt::Debug for OperationsService<S, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationsService").finish_non_exhaustive()
    }
}

impl<S: AlertReader, C: Clock> OperationsService<S, C> {
    /// `runbook_base` is the URL or path of `alerts.md`, to which each alert's anchor is appended.
    pub fn new(store: S, clock: C, runbook_base: impl Into<String>) -> Self {
        Self {
            store,
            clock,
            runbook_base: runbook_base.into(),
        }
    }

    /// Where the runbook for `alert` is.
    #[must_use]
    pub fn runbook(&self, alert: &AlertRecord) -> String {
        alert.runbook(&self.runbook_base)
    }

    /// The alerts of a namespace, newest first, optionally only those firing or only those resolved.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `ops:read`; [`AppError::NotFound`] for a namespace that is
    /// not the caller's tenant's.
    pub async fn alerts(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        state: Option<AlertState>,
        before: Option<Uuid>,
        limit: Option<u32>,
        ctx: &RequestContext,
    ) -> Result<Vec<AlertRecord>, AppError> {
        principal.authorize(Permission::OpsRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let limit = limit.unwrap_or(50).clamp(1, MAX_PAGE_SIZE);
        Ok(self
            .store
            .list_alerts(principal.tenant_id(), namespace, state, before, limit)
            .await?)
    }
}

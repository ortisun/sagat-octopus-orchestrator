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
/// Most alerts of one kind kept for one tenant in one look: one tenant, however many namespaces it
/// has, cannot crowd the others out of the look.
pub const MAX_PER_TENANT: u32 = 100;
/// Most groups one detection query returns.
pub const MAX_CANDIDATES: u32 = 1000;

/// Most worker identities a namespace has drained at once. The list of them shows this many.
pub const MAX_DRAINS_PER_NAMESPACE: usize = 500;

/// How many alerts of a kind one look may produce, in a namespace, for a tenant, and in all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub per_namespace: u32,
    pub per_tenant: u32,
    pub total: u32,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            per_namespace: MAX_PER_NAMESPACE,
            per_tenant: MAX_PER_TENANT,
            total: MAX_CANDIDATES,
        }
    }
}

/// A part of the platform where a look stopped short of everything it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Short {
    /// More groups than a namespace is given alerts for.
    Namespace(TenantId, NamespaceId),
    /// More groups than a tenant is given alerts for.
    Tenant(TenantId),
    /// More groups than a look is given in all.
    Everything,
}

/// What looking for one kind of condition found, and where it did not see everything.
///
/// An alert that is firing and not in `seen` is resolved, unless the look stopped short where it
/// is: not having seen it is then no sign that it is gone.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Detection {
    pub seen: Vec<Candidate>,
    pub short: Vec<Short>,
}

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
    /// How many alerts of the kind are firing now, in all.
    pub firing: u64,
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
        bounds: &Bounds,
    ) -> impl Future<Output = Result<Detection, StoreError>> + Send;

    /// Makes the alerts of `kind` match what was seen: a candidate with no alert (or a resolved one)
    /// begins to fire; a firing alert with no candidate is resolved, except where the look stopped
    /// short. Every change is written with the obligation to tell the namespace's subscribers, in
    /// the same transaction.
    fn reconcile(
        &self,
        kind: AlertKind,
        detection: &Detection,
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

/// A worker identity serving a namespace, as an operator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerView {
    pub id: Uuid,
    pub subject: String,
    pub name: String,
    pub activities: Vec<String>,
    pub languages: Vec<String>,
    pub pools: Vec<String>,
    pub max_inflight: u32,
    /// Attempts it holds a lease on right now.
    pub in_flight: u32,
    pub registered_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    /// Its identity is being drained: it takes no new work, and what it holds goes on.
    pub draining: bool,
}

/// Who stopped a worker identity from taking work, why and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainRecord {
    pub subject: String,
    pub reason: String,
    pub drained_by: String,
    pub drained_at: OffsetDateTime,
}

/// What the API reads and changes about the workers of a namespace.
pub trait WorkerAdmin: NamespaceDirectory {
    /// The live worker sessions of a namespace, newest first, with what they hold.
    fn list_workers(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<WorkerView>, StoreError>> + Send;

    /// The identities being drained in a namespace.
    fn list_drains(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Vec<DrainRecord>, StoreError>> + Send;

    /// Marks the identity as drained (replacing the reason of a drain already there) and writes
    /// `audit` in the same transaction.
    fn drain_worker(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        record: &DrainRecord,
        audit: &sagat_domain::audit::AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Lets the identity take work again, writing `audit` in the same transaction. Whether it was
    /// being drained.
    fn undrain_worker(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
        audit: &sagat_domain::audit::AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

/// What narrows a reading of the audit trail.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditFilter {
    /// An action by its dotted name (`run.rerun`).
    pub action: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub actor: Option<String>,
    pub from: Option<OffsetDateTime>,
    pub to: Option<OffsetDateTime>,
}

impl AuditFilter {
    /// The filter in words, as the record of a reading says what was looked for.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(a) = &self.action {
            parts.push(format!("action={a}"));
        }
        if let Some(t) = &self.resource_type {
            parts.push(format!("resourceType={t}"));
        }
        if let Some(r) = &self.resource_id {
            parts.push(format!("resourceId={r}"));
        }
        if let Some(a) = &self.actor {
            parts.push(format!("actor={a}"));
        }
        if let Some(f) = self.from {
            parts.push(format!("from={f}"));
        }
        if let Some(t) = self.to {
            parts.push(format!("to={t}"));
        }
        if parts.is_empty() {
            "the whole trail".to_owned()
        } else {
            parts.join(", ")
        }
    }
}

/// One record of the audit trail as it is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: Uuid,
    pub actor: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub reason: Option<String>,
    pub trace_id: Option<String>,
    pub created_at: OffsetDateTime,
}

/// Reading the audit trail of a tenant.
pub trait AuditStore: Send + Sync {
    /// The records of `tenant` matching `filter`, newest first, after the one with id `before`.
    /// `reading` is written in the same transaction: a trail that can be read is a trail whose
    /// readings are on it.
    fn read_audit(
        &self,
        tenant: TenantId,
        filter: &AuditFilter,
        before: Option<Uuid>,
        limit: u32,
        reading: &sagat_domain::audit::AuditRecord,
    ) -> impl Future<Output = Result<Vec<AuditEntry>, StoreError>> + Send;
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

impl<S: AlertReader + WorkerAdmin + AuditStore, C: Clock> OperationsService<S, C> {
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

    /// The audit trail of the tenant, newest first. Only an administrator reads it, and the reading
    /// is itself recorded (once for the same filters within five minutes).
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `audit:read`.
    pub async fn audit(
        &self,
        principal: &Principal,
        filter: &AuditFilter,
        before: Option<Uuid>,
        limit: Option<u32>,
        ctx: &RequestContext,
    ) -> Result<Vec<AuditEntry>, AppError> {
        principal.authorize(Permission::AuditRead)?;
        // A caller that asks for one more than its page, to know whether another follows, is
        // answered with that one more.
        let limit = limit.unwrap_or(50).clamp(1, MAX_PAGE_SIZE + 1);
        let reading = sagat_domain::audit::AuditRecord::by(
            principal,
            sagat_domain::audit::AuditAction::AuditRead,
            sagat_domain::audit::ResourceType::Tenant,
            principal.tenant_id().to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(filter.describe());
        Ok(self
            .store
            .read_audit(principal.tenant_id(), filter, before, limit, &reading)
            .await?)
    }

    /// The worker identities serving a namespace, with what each holds and whether it is being
    /// drained.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `ops:read`; [`AppError::NotFound`] for a namespace that is not
    /// the caller's tenant's.
    pub async fn workers(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Vec<WorkerView>, AppError> {
        principal.authorize(Permission::OpsRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .list_workers(principal.tenant_id(), namespace, 200)
            .await?)
    }

    /// The identities being drained in a namespace.
    ///
    /// # Errors
    /// As [`Self::workers`].
    pub async fn drains(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Vec<DrainRecord>, AppError> {
        principal.authorize(Permission::OpsRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .list_drains(principal.tenant_id(), namespace)
            .await?)
    }

    /// Stops a worker identity from taking new work in a namespace (a deploy, a bad version, a
    /// maintenance window). What it holds goes on until it ends; it can register and heartbeat as
    /// before. Audited, with the justification.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `worker:manage`; [`AppError::NotAllowed`] without a
    /// justification or for a subject that cannot be one; [`AppError::NotFound`].
    pub async fn drain(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        subject: &str,
        reason: &str,
        ctx: &RequestContext,
    ) -> Result<DrainRecord, AppError> {
        principal.authorize(Permission::WorkerManage)?;
        let reason = justification(reason)?;
        let subject = worker_subject(subject)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        // A drain is a row that stays until someone lifts it: a namespace holds a bounded number.
        let held = self
            .store
            .list_drains(principal.tenant_id(), namespace)
            .await?;
        if held.len() >= MAX_DRAINS_PER_NAMESPACE && !held.iter().any(|d| d.subject == subject) {
            return Err(AppError::NotAllowed(
                "too many worker identities are drained in this namespace; let some take work again",
            ));
        }
        let now = self.clock.now();
        let record = DrainRecord {
            subject: subject.to_owned(),
            reason: reason.to_owned(),
            drained_by: principal.subject().as_str().to_owned(),
            drained_at: now,
        };
        let audit = sagat_domain::audit::AuditRecord::by(
            principal,
            sagat_domain::audit::AuditAction::WorkerDrained,
            sagat_domain::audit::ResourceType::Worker,
            format!("{namespace}/{subject}"),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(reason.to_owned());
        self.store
            .drain_worker(principal.tenant_id(), namespace, &record, &audit)
            .await?;
        Ok(record)
    }

    /// Lets the identity take work again. `false` if it was not being drained.
    ///
    /// # Errors
    /// As [`Self::drain`].
    pub async fn undrain(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        subject: &str,
        reason: &str,
        ctx: &RequestContext,
    ) -> Result<bool, AppError> {
        principal.authorize(Permission::WorkerManage)?;
        let reason = justification(reason)?;
        let subject = worker_subject(subject)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let audit = sagat_domain::audit::AuditRecord::by(
            principal,
            sagat_domain::audit::AuditAction::WorkerUndrained,
            sagat_domain::audit::ResourceType::Worker,
            format!("{namespace}/{subject}"),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(reason.to_owned());
        Ok(self
            .store
            .undrain_worker(principal.tenant_id(), namespace, subject, &audit)
            .await?)
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
        // A caller that asks for one more than its page, to know whether another follows, is
        // answered with that one more.
        let limit = limit.unwrap_or(50).clamp(1, MAX_PAGE_SIZE + 1);
        Ok(self
            .store
            .list_alerts(principal.tenant_id(), namespace, state, before, limit)
            .await?)
    }
}

/// An operator's reason, trimmed, 8 to 2000 characters of plain text.
pub(crate) fn justification(reason: &str) -> Result<&str, AppError> {
    let reason = reason.trim();
    let length = reason.chars().count();
    if !(crate::runs::MIN_JUSTIFICATION..=crate::runs::MAX_JUSTIFICATION).contains(&length)
        || reason.chars().any(char::is_control)
    {
        return Err(AppError::NotAllowed(
            "a justification of 8 to 2000 characters, in plain text, is required",
        ));
    }
    Ok(reason)
}

/// The identity of a worker: what the `sub` of its token can be.
fn worker_subject(subject: &str) -> Result<&str, AppError> {
    if subject.is_empty() || subject.chars().count() > 255 || subject.chars().any(char::is_control)
    {
        return Err(AppError::NotAllowed("the worker identity is not valid"));
    }
    Ok(subject)
}

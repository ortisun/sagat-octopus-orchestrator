//! The monitor: finds the conditions that need a person, keeps the alerts in step with them, and
//! sets the gauges the dashboards draw (FR-008-003, FR-008-005).
//!
//! It looks at the platform from the outside of the paths it watches: aggregate queries over the
//! indexes the scheduler itself uses, never a lock on a run. One monitor works at a time (a lease in
//! the database), so one condition is one alert and not one per replica. A condition it cannot look
//! into this round leaves its alerts as they are: not seeing is not the same as being fine.

use std::time::Duration;

use sagat_application::operations::{
    Bounds, MonitorStore, PoolLag, Reconciled, Sample, Thresholds, WorkerPool,
};
use sagat_domain::operations::AlertKind;

/// Tunable limits of the monitor.
#[derive(Debug, Clone)]
pub struct MonitorSettings {
    pub thresholds: Thresholds,
    /// How many alerts of a kind a look may produce.
    pub bounds: Bounds,
    /// The URL or path of `alerts.md`, to which each alert's anchor is appended.
    pub runbook_base: String,
    /// Resolved alerts are kept this long.
    pub retention: Duration,
    /// The pools of workers whose queues are measured (`FR-010-004`); none by default.
    pub pools: Vec<WorkerPool>,
}

impl Default for MonitorSettings {
    fn default() -> Self {
        Self {
            thresholds: Thresholds::default(),
            bounds: Bounds::default(),
            runbook_base: "docs/runbooks/alerts.md".to_owned(),
            retention: Duration::from_secs(30 * 24 * 3600),
            pools: Vec::new(),
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MonitorRound {
    /// Another process holds the lease: nothing was done.
    pub passive: bool,
    /// Alerts that began to fire.
    pub fired: u32,
    /// Alerts that stopped.
    pub resolved: u32,
    /// Kinds that could not be looked into this round.
    pub unseen: u32,
}

/// The live statuses of a run, so a gauge that was set once is set to zero when it empties.
const LIVE_STATUSES: [&str; 6] = [
    "RUNNING",
    "WAITING",
    "PAUSED",
    "CANCELLING",
    "COMPENSATING",
    "MANUAL_REVIEW",
];

/// The monitor.
pub struct Monitor<S> {
    store: S,
    settings: MonitorSettings,
    /// Whether this process set the gauges on its last round: when another process takes the lease
    /// they are zeroed here, because a gauge keeps its last value for as long as the process lives
    /// and two replicas would then export two answers to the same question.
    exporting: std::sync::atomic::AtomicBool,
}

impl<S> std::fmt::Debug for Monitor<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Monitor").finish_non_exhaustive()
    }
}

impl<S: MonitorStore> Monitor<S> {
    pub const fn new(store: S, settings: MonitorSettings) -> Self {
        Self {
            store,
            settings,
            exporting: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// One round: samples the queues, looks for every kind of condition and reconciles its alerts.
    /// Never fails: what cannot be done now is counted and tried again by the next round.
    pub async fn run_once(&self) -> MonitorRound {
        let mut round = MonitorRound::default();
        match self.store.acquire_monitor().await {
            Ok(true) => {}
            Ok(false) => {
                round.passive = true;
                if self
                    .exporting
                    .swap(false, std::sync::atomic::Ordering::Relaxed)
                {
                    gauges(&Sample::default());
                    pool_gauges(
                        &self
                            .settings
                            .pools
                            .iter()
                            .map(|p| PoolLag {
                                pool: p.name.clone(),
                                ready: 0,
                                oldest_seconds: 0.0,
                            })
                            .collect::<Vec<_>>(),
                    );
                }
                return round;
            }
            Err(error) => {
                tracing::warn!(%error, "could not take the monitor's lease");
                round.unseen = u32::try_from(AlertKind::ALL.len()).unwrap_or(u32::MAX);
                return round;
            }
        }
        self.exporting
            .store(true, std::sync::atomic::Ordering::Relaxed);
        match self.store.sample().await {
            Ok(sample) => gauges(&sample),
            Err(error) => tracing::warn!(%error, "could not sample the queues"),
        }
        if !self.settings.pools.is_empty() {
            match self.store.pool_lag(&self.settings.pools).await {
                Ok(lags) => pool_gauges(&lags),
                Err(error) => {
                    tracing::warn!(%error, "could not sample the queues of the worker pools")
                }
            }
        }
        for kind in AlertKind::ALL {
            let detection = match self
                .store
                .detect(kind, &self.settings.thresholds, &self.settings.bounds)
                .await
            {
                Ok(detection) => detection,
                Err(error) => {
                    tracing::warn!(%error, kind = kind.as_str(), "could not look for an alert condition");
                    metrics::counter!("alert_checks_failed_total", "kind" => kind.as_str())
                        .increment(1);
                    round.unseen += 1;
                    continue;
                }
            };
            match self
                .store
                .reconcile(kind, &detection, &self.settings.runbook_base)
                .await
            {
                Ok(changed) => {
                    #[allow(clippy::cast_precision_loss)] // a gauge
                    metrics::gauge!("alerts_firing", "kind" => kind.as_str())
                        .set(changed.firing as f64);
                    round.fired += u32::try_from(changed.fired.len()).unwrap_or(u32::MAX);
                    round.resolved += u32::try_from(changed.resolved.len()).unwrap_or(u32::MAX);
                    announce(&changed, &self.settings.runbook_base);
                }
                Err(error) => {
                    tracing::warn!(%error, kind = kind.as_str(), "could not record the alerts of a kind");
                    metrics::counter!("alert_checks_failed_total", "kind" => kind.as_str())
                        .increment(1);
                    round.unseen += 1;
                }
            }
        }
        if let Err(error) = self.store.purge_alerts(self.settings.retention, 1000).await {
            tracing::warn!(%error, "could not purge old alerts");
        }
        round
    }
}

/// Tells the log and the counters what changed. Identifiers and counts only.
fn announce(changed: &Reconciled, runbook_base: &str) {
    for alert in &changed.fired {
        metrics::counter!("alerts_fired_total", "kind" => alert.kind.as_str()).increment(1);
        tracing::warn!(
            alert = alert.kind.as_str(),
            severity = alert.severity.as_str(),
            tenant_id = %alert.tenant_id,
            namespace_id = %alert.namespace_id,
            subject = alert.subject.as_str(),
            items = alert.items,
            oldest_seconds = alert.oldest_seconds,
            runbook = alert.runbook(runbook_base).as_str(),
            "alert fired: {}",
            alert.kind.summary()
        );
    }
    for alert in &changed.resolved {
        metrics::counter!("alerts_resolved_total", "kind" => alert.kind.as_str()).increment(1);
        tracing::info!(
            alert = alert.kind.as_str(),
            tenant_id = %alert.tenant_id,
            namespace_id = %alert.namespace_id,
            subject = alert.subject.as_str(),
            "alert resolved"
        );
    }
}

/// The gauges of the worker pools: the only label is the name the operator gave the pool.
#[allow(clippy::cast_precision_loss)] // gauges
fn pool_gauges(lags: &[PoolLag]) {
    for lag in lags {
        metrics::gauge!("pool_ready_steps", "pool" => lag.pool.clone()).set(lag.ready as f64);
        metrics::gauge!("pool_queue_lag_seconds", "pool" => lag.pool.clone())
            .set(lag.oldest_seconds);
    }
}

/// The gauges of a sample. No label is a tenant, a namespace, a run or an activity.
#[allow(clippy::cast_precision_loss)] // gauges
fn gauges(sample: &Sample) {
    metrics::gauge!("outbox_oldest_age_seconds").set(sample.outbox_oldest_seconds);
    metrics::gauge!("ready_steps").set(sample.ready_steps as f64);
    metrics::gauge!("queue_lag_seconds").set(sample.ready_oldest_seconds);
    metrics::gauge!("leases_active").set(sample.leases_active as f64);
    metrics::gauge!("leases_expired").set(sample.leases_expired as f64);
    metrics::gauge!("inbox_oldest_age_seconds").set(sample.inbox_oldest_seconds);
    metrics::gauge!("dead_items", "kind" => "events").set(sample.dead_events as f64);
    metrics::gauge!("dead_items", "kind" => "webhooks").set(sample.dead_deliveries as f64);
    metrics::gauge!("dead_items", "kind" => "connectors").set(sample.dead_letters as f64);
    metrics::gauge!("workers_registered").set(sample.workers as f64);
    metrics::gauge!("compensating_runs").set(sample.compensating_runs as f64);
    // How late each pass of the scheduler is: what the engine's replicas scale on, and what an
    // alert watches (a pass that is minutes behind with every shard owned needs more replicas; one
    // with a shard unowned needs a replica back).
    let late = &sample.overdue;
    metrics::gauge!("scheduler_lag_seconds", "pass" => "timers").set(late.timers);
    metrics::gauge!("scheduler_lag_seconds", "pass" => "wait_deadlines").set(late.wait_deadlines);
    metrics::gauge!("scheduler_lag_seconds", "pass" => "retries").set(late.retries);
    metrics::gauge!("scheduler_lag_seconds", "pass" => "leases").set(late.leases);
    metrics::gauge!("scheduler_lag_seconds", "pass" => "deadlines").set(late.deadlines);
    for status in LIVE_STATUSES {
        let n = sample
            .runs_by_status
            .iter()
            .find(|(s, _)| s == status)
            .map_or(0, |(_, n)| *n);
        metrics::gauge!("runs_active", "status" => status).set(n as f64);
    }
}

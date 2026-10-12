//! How late the scheduler is, as the monitor samples it (FR-010-004): the signal the engine's
//! replicas scale on.

use sagat_application::operations::MonitorStore;
use serde_json::json;

use crate::passes::{scheduler, sleepers};
use crate::support::engine::{chain, world};
use sagat_scheduler::SchedulerSettings;

#[tokio::test]
async fn the_lag_of_a_pass_is_how_long_its_oldest_item_has_been_overdue() {
    let w = world().await;
    // Nothing is due: the scheduler is not late.
    let quiet = w.store.sample().await.expect("sample").overdue;
    assert_eq!(
        (
            quiet.timers,
            quiet.retries,
            quiet.leases,
            quiet.deadlines,
            quiet.wait_deadlines
        ),
        (0.0, 0.0, 0.0, 0.0, 0.0)
    );

    // Three timers, the oldest due ten seconds ago.
    let runs = sleepers(&w, "lagging", 3).await;
    sqlx::query(
        "UPDATE steps SET due_at = clock_timestamp() - interval '10 seconds' WHERE run_id = $1",
    )
    .bind(runs[0].as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("age");
    let late = w.store.sample().await.expect("sample").overdue;
    assert!(
        (9.0..60.0).contains(&late.timers),
        "timers: {}",
        late.timers
    );
    assert_eq!(late.retries, 0.0, "the other passes are not late");

    // A lease nobody took back and a run past its deadline are late in their own passes.
    let (wf, _) = w.publish(chain("lagging-lease")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let _task = w.one_task(&session).await;
    w.age_lease(run, "a").await;
    w.age_run_deadline(run).await;
    let late = w.store.sample().await.expect("sample").overdue;
    assert!(late.leases > 0.0, "leases: {}", late.leases);
    assert!(late.deadlines > 0.0, "deadlines: {}", late.deadlines);

    // The scheduler catches up and the lag is gone.
    let alone = scheduler(&w, SchedulerSettings::default(), None);
    while alone.tick().await.did_work() {}
    let caught_up = w.store.sample().await.expect("sample").overdue;
    assert_eq!(
        (caught_up.timers, caught_up.leases, caught_up.deadlines),
        (0.0, 0.0, 0.0)
    );
}

#[tokio::test]
async fn the_queue_of_a_pool_is_what_is_ready_for_the_activities_it_serves_across_tenants() {
    use sagat_application::operations::WorkerPool;

    let w = world().await;
    let (wf, _) = w.publish(chain("pool-lag")).await;
    for _ in 0..3 {
        w.start(wf, &json!({})).await;
    }
    // The other tenant's step of the same activity counts for the same pool: a pool is a set of
    // workers, the scaler's question is how many of them there should be.
    let other = w.publish_as_other_tenant(chain("pool-lag-other")).await;
    w.start_as_other_tenant(other, &json!({}))
        .await
        .expect("start");
    sqlx::query("UPDATE steps SET ready_at = clock_timestamp() - interval '40 seconds' WHERE status = 'READY'")
        .execute(&w.db.pool)
        .await
        .expect("age");

    // A paused run's ready step waits for nobody: it is not in the queue of the pool.
    let paused = w.start(wf, &json!({})).await;
    w.operate_run(paused, sagat_application::runs::RunOperation::Pause)
        .await;
    sqlx::query("UPDATE steps SET ready_at = clock_timestamp() - interval '40 seconds' WHERE status = 'READY'")
        .execute(&w.db.pool)
        .await
        .expect("age");

    let pools = [
        WorkerPool {
            name: "front".into(),
            // Named twice: counted once.
            activities: vec!["act.a".into(), "act.a".into()],
        },
        WorkerPool {
            name: "idle".into(),
            activities: vec!["act.nobody".into(), "act.b".into()],
        },
        WorkerPool {
            name: "both".into(),
            activities: vec!["act.a".into(), "act.b".into()],
        },
    ];
    let lags = w.store.pool_lag(&pools).await.expect("lags");
    assert_eq!(
        lags.iter().map(|l| l.pool.as_str()).collect::<Vec<_>>(),
        ["front", "idle", "both"],
        "in the operator's order"
    );
    assert_eq!(lags[0].ready, 4);
    assert!(
        (39.0..120.0).contains(&lags[0].oldest_seconds),
        "{}",
        lags[0].oldest_seconds
    );
    assert_eq!(
        (lags[1].ready, lags[1].oldest_seconds),
        (0, 0.0),
        "act.b is not ready yet"
    );
    assert_eq!(lags[2].ready, 4);
    assert!(w.store.pool_lag(&[]).await.expect("none").is_empty());
}

/// A monitor store whose sample can be made to fail, over the real one.
struct Flaky<S> {
    inner: S,
    fail_samples: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl<S: sagat_application::operations::MonitorStore> sagat_application::operations::MonitorStore
    for Flaky<S>
{
    async fn acquire_monitor(&self) -> Result<bool, sagat_application::error::StoreError> {
        self.inner.acquire_monitor().await
    }
    async fn detect(
        &self,
        kind: sagat_domain::operations::AlertKind,
        thresholds: &sagat_application::operations::Thresholds,
        bounds: &sagat_application::operations::Bounds,
    ) -> Result<sagat_application::operations::Detection, sagat_application::error::StoreError>
    {
        self.inner.detect(kind, thresholds, bounds).await
    }
    async fn reconcile(
        &self,
        kind: sagat_domain::operations::AlertKind,
        detection: &sagat_application::operations::Detection,
        runbook_base: &str,
    ) -> Result<sagat_application::operations::Reconciled, sagat_application::error::StoreError>
    {
        self.inner.reconcile(kind, detection, runbook_base).await
    }
    async fn sample(
        &self,
    ) -> Result<sagat_application::operations::Sample, sagat_application::error::StoreError> {
        if self.fail_samples.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(sagat_application::error::StoreError::Unavailable {
                detail: "the sample timed out".to_owned(),
            });
        }
        self.inner.sample().await
    }
    async fn pool_lag(
        &self,
        pools: &[sagat_application::operations::WorkerPool],
    ) -> Result<Vec<sagat_application::operations::PoolLag>, sagat_application::error::StoreError>
    {
        self.inner.pool_lag(pools).await
    }
    async fn purge_alerts(
        &self,
        retention: std::time::Duration,
        limit: u32,
    ) -> Result<u64, sagat_application::error::StoreError> {
        self.inner.purge_alerts(retention, limit).await
    }
}

#[tokio::test]
async fn a_sample_that_fails_zeroes_the_gauges_the_autoscalers_read_and_the_backlog_is_the_monitors()
 {
    use sagat_application::maintenance::OutboxStore;
    use sagat_scheduler::{Monitor, MonitorSettings};

    // The recorder is installed before anything records, as the processes do at start. No other test
    // of this binary runs a monitor, so the series below are this test's.
    sagat_telemetry::Metrics::global();
    let w = world().await;
    let runs = sleepers(&w, "gauge-lag", 2).await;
    sqlx::query(
        "UPDATE steps SET due_at = clock_timestamp() - interval '20 seconds' WHERE run_id = $1",
    )
    .bind(runs[0].as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("age");
    // A run with an activity ready leaves a hint in the outbox for the dispatcher (a timer does not).
    let (wf, _) = w.publish(chain("gauge-backlog")).await;
    w.start(wf, &json!({})).await;
    let owed = w.store.outbox_backlog().await.expect("backlog");
    assert!(
        owed > 0,
        "a ready activity leaves a message for the dispatcher"
    );

    let fail_samples = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let monitor = Monitor::new(
        Flaky {
            inner: w.store.clone(),
            fail_samples: std::sync::Arc::clone(&fail_samples),
        },
        MonitorSettings::default(),
    );
    let value = |series: &str| -> Option<f64> {
        sagat_telemetry::Metrics::global()
            .render()
            .lines()
            .find_map(|line| line.strip_prefix(series))
            .and_then(|v| v.trim().parse::<f64>().ok())
    };

    monitor.run_once().await;
    assert!(
        value("scheduler_lag_seconds{pass=\"timers\"} ").is_some_and(|lag| lag >= 19.0),
        "the lag the autoscaler reads"
    );
    assert_eq!(
        value("outbox_backlog ").map(|n| n as u64),
        Some(owed),
        "the backlog is the monitor's, counted once"
    );

    // The database cannot answer: the gauges are not left at the last value, which would go on scaling
    // the engine for as long as the failure lasts.
    fail_samples.store(true, std::sync::atomic::Ordering::SeqCst);
    monitor.run_once().await;
    assert_eq!(value("scheduler_lag_seconds{pass=\"timers\"} "), Some(0.0));
    assert_eq!(value("outbox_backlog "), Some(0.0));
    assert_eq!(value("ready_steps "), Some(0.0));

    // And they come back with the database.
    fail_samples.store(false, std::sync::atomic::Ordering::SeqCst);
    monitor.run_once().await;
    assert!(value("scheduler_lag_seconds{pass=\"timers\"} ").is_some_and(|lag| lag >= 19.0));
}

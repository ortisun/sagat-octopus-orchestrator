//! What goes wrong around the alerts: a look that fails must not resolve what it did not see, two
//! monitors at once must not raise an alert twice, a namespace with a thousand unserved activities
//! has a few alerts and not a thousand, and old alerts go away (FR-008-005, NFR-008-DUR).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::Method;
use sagat_application::error::StoreError;
use sagat_application::operations::{
    Bounds, Detection, MAX_PER_NAMESPACE, MonitorStore, Reconciled, Sample, Thresholds,
};
use sagat_domain::operations::AlertKind;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Monitor, MonitorSettings};
use serde_json::json;

use crate::support::worker::{Platform, platform, single_ir};

/// The real store, except that looking for one kind of condition can be made to fail.
#[derive(Clone)]
struct Blinkable {
    inner: PgRunStore,
    blind_to: Option<AlertKind>,
    broken: Arc<AtomicBool>,
}

impl MonitorStore for Blinkable {
    async fn acquire_monitor(&self) -> Result<bool, StoreError> {
        self.inner.acquire_monitor().await
    }

    async fn detect(
        &self,
        kind: AlertKind,
        thresholds: &Thresholds,
        bounds: &Bounds,
    ) -> Result<Detection, StoreError> {
        if self.broken.load(Ordering::SeqCst) && self.blind_to == Some(kind) {
            return Err(StoreError::Unavailable {
                detail: "the query timed out".to_owned(),
            });
        }
        self.inner.detect(kind, thresholds, bounds).await
    }

    async fn reconcile(
        &self,
        kind: AlertKind,
        detection: &Detection,
        runbook: &str,
    ) -> Result<Reconciled, StoreError> {
        self.inner.reconcile(kind, detection, runbook).await
    }

    async fn sample(&self) -> Result<Sample, StoreError> {
        self.inner.sample().await
    }

    async fn pool_lag(
        &self,
        pools: &[sagat_application::operations::WorkerPool],
    ) -> Result<Vec<sagat_application::operations::PoolLag>, StoreError> {
        self.inner.pool_lag(pools).await
    }

    async fn purge_alerts(&self, retention: Duration, limit: u32) -> Result<u64, StoreError> {
        self.inner.purge_alerts(retention, limit).await
    }
}

async fn firing(p: &Platform, kind: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM operational_alerts WHERE kind = $1 AND state = 'FIRING'",
    )
    .bind(kind)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count")
}

async fn unserved_run(p: &Platform, name: &str, activity: &str) {
    let mut ir = single_ir(name, None);
    ir["nodes"][0]["activity"] = json!(activity);
    let workflow = p.publish(name, &ir).await;
    p.start(&workflow, &json!({})).await;
}

async fn age_ready_steps(p: &Platform) {
    sqlx::query(
        "UPDATE steps SET ready_at = clock_timestamp() - interval '1 hour' WHERE status = 'READY'",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("time passes");
}

#[tokio::test]
async fn a_look_that_fails_leaves_the_alerts_of_that_kind_as_they_are() {
    let p = platform().await;
    unserved_run(&p, "unserved", "act.a").await;
    age_ready_steps(&p).await;
    let store = Blinkable {
        inner: p.api.executions.clone(),
        blind_to: Some(AlertKind::StepUnclaimed),
        broken: Arc::new(AtomicBool::new(false)),
    };
    let monitor = Monitor::new(store.clone(), MonitorSettings::default());
    assert_eq!(monitor.run_once().await.fired, 1);
    assert_eq!(firing(&p, "STEP_UNCLAIMED").await, 1);

    // The query cannot run: not being able to look is not the same as the condition being gone.
    store.broken.store(true, Ordering::SeqCst);
    let round = monitor.run_once().await;
    assert_eq!((round.unseen, round.resolved), (1, 0), "{round:?}");
    assert_eq!(
        firing(&p, "STEP_UNCLAIMED").await,
        1,
        "the alert was not resolved by a look that failed"
    );

    // Once it can look again and the condition is gone, it resolves.
    store.broken.store(false, Ordering::SeqCst);
    sqlx::query("UPDATE steps SET status = 'CANCELLED', ready_at = NULL WHERE status = 'READY'")
        .execute(&p.api.db.pool)
        .await
        .ok();
    sqlx::query("UPDATE steps SET ready_at = clock_timestamp() WHERE status = 'READY'")
        .execute(&p.api.db.pool)
        .await
        .expect("the step is taken");
    assert_eq!(monitor.run_once().await.resolved, 1);
    assert_eq!(firing(&p, "STEP_UNCLAIMED").await, 0);
}

/// Both monitors are in one process, so they share the holder of the lease and both pass it: this
/// holds the alerts themselves to one row and one notification (`a_dead_monitor_is_replaced_when_its_lease_runs_out`
/// holds the lease).
#[tokio::test]
async fn two_monitors_that_both_think_they_hold_the_lease_still_raise_one_alert_and_tell_the_subscribers_once()
 {
    let p = platform().await;
    let ns = p.namespace_id().await;
    for (method, path, body) in [
        (
            Method::PUT,
            format!("/namespaces/{ns}/secrets/alert-signing"),
            json!({"value": "whsec-0123456789abcdef0123456789abcdef"}),
        ),
        (
            Method::PUT,
            format!("/namespaces/{ns}/egress-policy"),
            json!({"rules": ["hooks.example.com"]}),
        ),
    ] {
        let (status, answer) = p.rest(method, &path, Some(&body)).await;
        assert_eq!(status, 200, "{answer}");
    }
    let (status, subscription) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/webhook-subscriptions"),
            Some(&json!({"name": "ops", "url": "https://hooks.example.com/in", "secretName": "alert-signing",
                         "events": ["ops.alert.fired"]})),
        )
        .await;
    assert_eq!(status, 201, "{subscription}");
    unserved_run(&p, "unserved", "act.a").await;
    age_ready_steps(&p).await;

    let first = Monitor::new(p.api.executions.clone(), MonitorSettings::default());
    let second = Monitor::new(p.api.executions.clone(), MonitorSettings::default());
    let (a, b) = tokio::join!(first.run_once(), second.run_once());
    assert!(a.fired + b.fired >= 1, "{a:?} {b:?}");
    assert_eq!(
        firing(&p, "STEP_UNCLAIMED").await,
        1,
        "one condition is one alert"
    );
    let told: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_deliveries WHERE kind = 'ops.alert.fired'",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count");
    assert_eq!(
        told, 1,
        "and one notification, however many monitors saw it"
    );
    assert_eq!(
        (a.unseen, b.unseen),
        (0, 0),
        "neither failed on the other: {a:?} {b:?}"
    );
}

#[tokio::test]
async fn a_dead_monitor_is_replaced_when_its_lease_runs_out() {
    let p = platform().await;
    sqlx::query("INSERT INTO engine_leases (name, holder, expires_at) VALUES ('operations-monitor', 'someone-else', clock_timestamp() + interval '1 minute') ON CONFLICT (name) DO UPDATE SET holder = 'someone-else', expires_at = EXCLUDED.expires_at")
        .execute(&p.api.db.pool)
        .await
        .expect("a lease");
    let monitor = Monitor::new(p.api.executions.clone(), MonitorSettings::default());
    assert!(monitor.run_once().await.passive);
    sqlx::query("UPDATE engine_leases SET expires_at = clock_timestamp() - interval '1 second' WHERE name = 'operations-monitor'")
        .execute(&p.api.db.pool)
        .await
        .expect("it dies");
    assert!(!monitor.run_once().await.passive);
}

#[tokio::test]
async fn a_namespace_with_many_unserved_activities_has_a_few_alerts_and_not_one_for_each() {
    let p = platform().await;
    let many = usize::try_from(MAX_PER_NAMESPACE).expect("small") + 10;
    for n in 0..many {
        unserved_run(&p, &format!("flow-{n}"), &format!("act.n{n}")).await;
    }
    age_ready_steps(&p).await;
    let round = Monitor::new(p.api.executions.clone(), MonitorSettings::default())
        .run_once()
        .await;
    assert_eq!(round.unseen, 0, "{round:?}");
    assert_eq!(
        firing(&p, "STEP_UNCLAIMED").await,
        i64::from(MAX_PER_NAMESPACE),
        "the oldest {MAX_PER_NAMESPACE}: the rest are in the gauges and the queue"
    );
}

#[tokio::test]
async fn resolved_alerts_are_purged_after_their_retention_and_firing_ones_never() {
    let p = platform().await;
    unserved_run(&p, "unserved", "act.a").await;
    age_ready_steps(&p).await;
    let monitor = Monitor::new(p.api.executions.clone(), MonitorSettings::default());
    assert_eq!(monitor.run_once().await.fired, 1);
    // Old and resolved: goes. Old and firing: stays.
    sqlx::query("INSERT INTO operational_alerts (id, tenant_id, namespace_id, kind, subject, state, severity, first_seen_at, last_seen_at, resolved_at, fired_count, item_count, oldest_seconds) \
                 SELECT gen_random_uuid(), tenant_id, namespace_id, 'DEAD_ITEMS', 'events', 'RESOLVED', 'warning', \
                        clock_timestamp() - interval '90 days', clock_timestamp() - interval '90 days', clock_timestamp() - interval '60 days', 1, 3, 10 \
                 FROM operational_alerts LIMIT 1")
        .execute(&p.api.db.pool)
        .await
        .expect("an old resolved alert");
    sqlx::query("UPDATE operational_alerts SET first_seen_at = clock_timestamp() - interval '90 days' WHERE state = 'FIRING'")
        .execute(&p.api.db.pool)
        .await
        .expect("an old firing alert");
    let removed =
        MonitorStore::purge_alerts(&p.api.executions, Duration::from_secs(30 * 24 * 3600), 100)
            .await
            .expect("purge");
    assert_eq!(removed, 1);
    assert_eq!(firing(&p, "STEP_UNCLAIMED").await, 1);
}

#[tokio::test]
async fn the_gauges_follow_the_queues_and_never_carry_a_tenant_a_namespace_or_an_activity() {
    // The recorder is installed before anything records, as the processes do at start.
    sagat_telemetry::Metrics::global();
    let p = platform().await;
    unserved_run(&p, "unserved", "act.secret-activity-name").await;
    age_ready_steps(&p).await;
    let monitor = Monitor::new(p.api.executions.clone(), MonitorSettings::default());
    // The recorder is the process's, and the other tests of this binary run monitors of their own
    // that set the same gauges (to the lag of their own, younger, platform): look until the page
    // shows this platform's figure, which is the one that is being tested.
    let mut page = String::new();
    for _ in 0..40 {
        monitor.run_once().await;
        page = sagat_telemetry::Metrics::global().render();
        let lag = page
            .lines()
            .find_map(|l| l.strip_prefix("queue_lag_seconds "))
            .and_then(|v| v.trim().parse::<f64>().ok());
        if lag.is_some_and(|lag| lag >= 3500.0) {
            break;
        }
    }
    for series in [
        "ready_steps",
        "queue_lag_seconds",
        "outbox_oldest_age_seconds",
        "leases_expired",
        "workers_registered",
        "alerts_firing{kind=\"STEP_UNCLAIMED\"}",
    ] {
        assert!(
            page.contains(series),
            "{series} is not on the page:\n{page}"
        );
    }
    assert!(
        !page.contains("secret-activity-name"),
        "an activity name is on the page:\n{page}"
    );
    let lag = page
        .lines()
        .find_map(|l| l.strip_prefix("queue_lag_seconds "))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .expect("queue lag");
    assert!(lag >= 3500.0, "the oldest ready step is an hour old: {lag}");
}

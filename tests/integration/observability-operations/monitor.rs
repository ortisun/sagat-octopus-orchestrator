//! What the monitor does at the edges of what it is allowed to say: the bounds on a look, and the
//! steps it must not call unserved (FR-008-005).

use std::time::Duration;

use sagat_application::operations::{AlertReader, Bounds, Thresholds};
use sagat_domain::operations::AlertKind;
use sagat_scheduler::{Monitor, MonitorSettings};
use serde_json::json;

use crate::support::engine::{World, activity, document, world};

fn immediately(bounds: Bounds) -> MonitorSettings {
    MonitorSettings {
        thresholds: Thresholds {
            unclaimed: Duration::ZERO,
            ..Thresholds::default()
        },
        bounds,
        ..MonitorSettings::default()
    }
}

/// A run whose only step waits for `activity`, ready since `seconds_ago`.
async fn unserved(w: &World, activity_name: &str, seconds_ago: u32) {
    let (wf, _) = w
        .publish(document(
            &format!("flow-{}", activity_name.replace('.', "-")),
            "a",
            vec![serde_json::json!({"id": "a", "type": "activity", "activity": activity_name})],
        ))
        .await;
    let run = w.start(wf, &json!({})).await;
    aged(w, run, seconds_ago).await;
}

async fn aged(w: &World, run: sagat_domain::execution::RunId, seconds_ago: u32) {
    sqlx::query(
        "UPDATE steps SET ready_at = clock_timestamp() - make_interval(secs => $2) \
         WHERE run_id = $1 AND status = 'READY'",
    )
    .bind(run.as_uuid())
    .bind(f64::from(seconds_ago))
    .execute(&w.db.pool)
    .await
    .expect("time passes");
}

async fn firing(w: &World, tenant: sagat_domain::identity::TenantId) -> Vec<String> {
    let mut subjects: Vec<String> = sqlx::query_scalar(
        "SELECT subject FROM operational_alerts \
         WHERE tenant_id = $1 AND kind = 'STEP_UNCLAIMED' AND state = 'FIRING' ORDER BY subject",
    )
    .bind(tenant.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("alerts");
    subjects.sort();
    subjects
}

#[tokio::test]
async fn an_alert_pushed_past_the_bound_by_an_older_one_stays_while_its_condition_does() {
    let w = world().await;
    let monitor = Monitor::new(
        w.store.clone(),
        immediately(Bounds {
            per_namespace: 3,
            ..Bounds::default()
        }),
    );
    for (name, age) in [
        ("act.a", 900),
        ("act.b", 800),
        ("act.c", 700),
        ("act.d", 600),
    ] {
        unserved(&w, name, age).await;
    }
    let round = monitor.run_once().await;
    assert_eq!(round.fired, 3, "the three oldest: {round:?}");
    assert_eq!(
        firing(&w, w.admin.tenant_id()).await,
        ["act.a", "act.b", "act.c"]
    );

    // An older one appears: the look now has `act.e`, `act.a` and `act.b` in its three, and
    // `act.c` is pushed out of them while its step is exactly as unserved as before.
    unserved(&w, "act.e", 1000).await;
    let round = monitor.run_once().await;
    assert_eq!(round.fired, 1, "{round:?}");
    assert_eq!(
        round.resolved, 0,
        "`act.c` was not looked at, which is not the same as being served: {round:?}"
    );
    assert_eq!(
        firing(&w, w.admin.tenant_id()).await,
        ["act.a", "act.b", "act.c", "act.e"]
    );
}

#[tokio::test]
async fn an_alert_resolves_when_its_condition_ends_even_with_others_beyond_the_bound() {
    let w = world().await;
    let monitor = Monitor::new(
        w.store.clone(),
        immediately(Bounds {
            per_namespace: 2,
            ..Bounds::default()
        }),
    );
    unserved(&w, "act.a", 900).await;
    unserved(&w, "act.b", 800).await;
    assert_eq!(monitor.run_once().await.fired, 2);
    // A third is beyond the bound; `act.a` is taken by a worker.
    unserved(&w, "act.c", 700).await;
    sqlx::query("UPDATE steps SET status = 'CANCELLED', ready_at = NULL WHERE activity = 'act.a'")
        .execute(&w.db.pool)
        .await
        .expect("a worker takes it");
    let round = monitor.run_once().await;
    // `act.b` and `act.c` are the two oldest now, so the namespace is not short for what mattered.
    assert_eq!(
        firing(&w, w.admin.tenant_id()).await,
        ["act.b", "act.c"],
        "{round:?}"
    );
}

#[tokio::test]
async fn one_tenant_with_many_conditions_cannot_hide_the_condition_of_another() {
    let w = world().await;
    let monitor = Monitor::new(
        w.store.clone(),
        immediately(Bounds {
            per_namespace: 25,
            per_tenant: 2,
            total: 3,
        }),
    );
    // The first tenant has four groups, all older than the other's single one.
    for (name, age) in [
        ("act.a", 900),
        ("act.b", 800),
        ("act.c", 700),
        ("act.d", 600),
    ] {
        unserved(&w, name, age).await;
    }
    let theirs = w
        .publish_as_other_tenant(document("theirs", "a", vec![activity("a")]))
        .await;
    let run = w
        .start_as_other_tenant(theirs, &json!({}))
        .await
        .expect("start");
    sqlx::query(
        "UPDATE steps SET ready_at = clock_timestamp() - interval '100 seconds' WHERE run_id = $1",
    )
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("time passes");

    monitor.run_once().await;
    assert_eq!(
        firing(&w, w.other_admin.tenant_id()).await,
        ["act.a"],
        "the other tenant's alert is raised however much the first has stuck"
    );
    assert_eq!(
        firing(&w, w.admin.tenant_id()).await.len(),
        2,
        "and the first has at most its share"
    );
}

#[tokio::test]
async fn a_step_held_back_on_purpose_is_not_called_unserved() {
    let w = world().await;
    let monitor = Monitor::new(w.store.clone(), immediately(Bounds::default()));
    let (wf, _) = w.publish(document("held", "a", vec![activity("a")])).await;
    let run = w.start(wf, &json!({})).await;
    aged(&w, run, 900).await;
    w.operate_run(run, sagat_application::runs::RunOperation::Pause)
        .await;
    monitor.run_once().await;
    assert!(
        firing(&w, w.admin.tenant_id()).await.is_empty(),
        "a paused run waits for a person, not for a worker"
    );

    w.operate_run(run, sagat_application::runs::RunOperation::Resume)
        .await;
    aged(&w, run, 900).await;
    monitor.run_once().await;
    assert_eq!(firing(&w, w.admin.tenant_id()).await, ["act.a"]);
}

#[tokio::test]
async fn the_gauge_of_firing_alerts_counts_those_kept_and_not_only_those_seen() {
    let w = world().await;
    let monitor = Monitor::new(
        w.store.clone(),
        immediately(Bounds {
            per_namespace: 1,
            ..Bounds::default()
        }),
    );
    unserved(&w, "act.a", 900).await;
    unserved(&w, "act.b", 800).await;
    monitor.run_once().await;
    unserved(&w, "act.c", 1000).await;
    monitor.run_once().await;
    // `act.a` and `act.c` are firing and `act.b` was never raised; `act.a` is kept though unseen.
    let listed = w
        .store
        .list_alerts(w.admin.tenant_id(), w.namespace.id(), None, None, 50)
        .await
        .expect("alerts");
    assert_eq!(
        listed
            .iter()
            .filter(|a| a.kind == AlertKind::StepUnclaimed
                && a.state == sagat_domain::operations::AlertState::Firing)
            .count(),
        2
    );
}

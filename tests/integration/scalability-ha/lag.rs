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

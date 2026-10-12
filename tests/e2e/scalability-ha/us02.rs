//! US02 — Sustain wait volume. Given thousands of runs await timers, when the workers are idle,
//! then processes are not dedicated per run (US02-AC01). The benchmark takes the same story to a
//! million (`tests/performance/reports/scalability-ha.md`); this test holds the structural claim.

use std::time::Duration;

use serde_json::json;

use crate::support::worker::{RunningEngine, engine_env, platform};

async fn connections(p: &crate::support::worker::Platform) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database()")
        .fetch_one(&p.api.db.pool)
        .await
        .expect("connections")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thousands_of_suspended_runs_cost_rows_and_not_tasks_connections_or_leases() {
    let p = platform().await;
    let engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let ir = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "sleeper"}, "entrypoint": "pause",
        "nodes": [
            {"id": "pause", "type": "timer", "durationMs": 3_600_000, "next": "a"},
            {"id": "a", "type": "activity", "activity": "act.a"}
        ]
    });
    let workflow = p.publish("sleeper", &ir).await;
    // The pools of the API and of the engine open connections as load asks for them, up to their size, and a
    // slow machine opens them late. The claim is that a suspended run costs no connection of its own, so the
    // first runs warm the pools and the count is taken after them: it must not grow with the next fifteen hundred.
    const WARM_UP: usize = 200;
    for i in 0..WARM_UP {
        p.start(&workflow, &json!({ "warm": i })).await;
    }
    // Let the engine's loops and pools settle before the first reading.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let tasks_before = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let connections_before = connections(&p).await;

    const RUNS: usize = 1_500;
    for i in 0..RUNS {
        p.start(&workflow, &json!({ "i": i })).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let tasks_after = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let connections_after = connections(&p).await;

    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE status = 'WAITING' AND waiting_kind = 'TIMER'",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count");
    assert_eq!(waiting, i64::try_from(WARM_UP + RUNS).expect("fits"));

    // A suspended run is a row: not a task, not a connection, not a lease, not a worker.
    assert!(
        tasks_after < tasks_before + 20,
        "{RUNS} suspended runs added {} tasks to the runtime",
        tasks_after.saturating_sub(tasks_before)
    );
    assert!(
        connections_after <= connections_before + 3,
        "connections went from {connections_before} to {connections_after}"
    );
    for (what, sql) in [
        (
            "leases",
            "SELECT count(*) FROM steps WHERE status IN ('LEASED', 'RUNNING')",
        ),
        ("worker sessions", "SELECT count(*) FROM worker_sessions"),
        (
            "ready steps",
            "SELECT count(*) FROM steps WHERE status = 'READY'",
        ),
    ] {
        let n: i64 = sqlx::query_scalar(sql)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("count");
        assert_eq!(n, 0, "{what} for {RUNS} suspended runs");
    }
    // The engine stays alive and ready, and its scheduler is not busy with them.
    assert_eq!(engine.status("/readyz").await, Some(200));
    let tick = std::time::Instant::now();
    let report = p.tick().await;
    assert!(!report.did_work(), "{report:?}");
    assert!(
        tick.elapsed() < Duration::from_secs(1),
        "a pass over {RUNS} waits with nothing due took {:?}",
        tick.elapsed()
    );
}

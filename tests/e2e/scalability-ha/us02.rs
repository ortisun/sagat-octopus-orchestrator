//! US02 — Sustain wait volume. Given thousands of runs await timers, when the workers are idle,
//! then processes are not dedicated per run (US02-AC01). The benchmark takes the same story to a
//! million (`tests/performance/reports/scalability-ha.md`); this test holds the structural claim.

use std::time::Duration;

use sagat_control_plane::config::Config;
use serde_json::json;

use crate::support::worker::{RunningEngine, engine_env, platform};

async fn connections(p: &crate::support::worker::Platform) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND backend_type = 'client backend'",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("connections")
}

async fn assert_connection_budget(
    p: &crate::support::worker::Platform,
    budget: i64,
    checkpoint: &str,
) {
    let observed = connections(p).await;
    assert!(
        observed <= budget,
        "{checkpoint}: {observed} client backends exceed the configured pool budget {budget}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thousands_of_suspended_runs_cost_rows_and_not_tasks_connections_or_leases() {
    let p = platform().await;
    let engine_environment = engine_env(&p.api.db, &p.api.idp, &[]);
    let lookup = |name: &str| engine_environment.get(name).cloned();
    let engine_config = Config::from_lookup(&lookup).expect("engine configuration");
    // All three pools open lazily under load, but never beyond these configured budgets. A
    // suspended run must not allocate a connection outside that fixed, run-count-independent cap.
    let connection_budget = i64::from(p.api.db.pool.options().get_max_connections())
        + i64::from(p.api.api_pool.options().get_max_connections())
        + i64::from(engine_config.database.max_connections);
    let engine = RunningEngine::start(engine_environment).await;
    let ir = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "sleeper"}, "entrypoint": "pause",
        "nodes": [
            {"id": "pause", "type": "timer", "durationMs": 3_600_000, "next": "a"},
            {"id": "a", "type": "activity", "activity": "act.a"}
        ]
    });
    let workflow = p.publish("sleeper", &ir).await;
    assert_connection_budget(&p, connection_budget, "before suspended runs").await;
    let tasks_before = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();

    const RUNS: usize = 1_500;
    for i in 0..RUNS {
        p.start(&workflow, &json!({ "i": i })).await;
        if (i + 1) % 250 == 0 {
            assert_connection_budget(
                &p,
                connection_budget,
                &format!("after {} suspended runs", i + 1),
            )
            .await;
        }
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let tasks_after = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();

    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE status = 'WAITING' AND waiting_kind = 'TIMER'",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count");
    assert_eq!(waiting, i64::try_from(RUNS).expect("fits"));
    assert_connection_budget(&p, connection_budget, "after suspended runs settled").await;

    // A suspended run is a row: not a task, not a connection, not a lease, not a worker.
    assert!(
        tasks_after < tasks_before + 20,
        "{RUNS} suspended runs added {} tasks to the runtime",
        tasks_after.saturating_sub(tasks_before)
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

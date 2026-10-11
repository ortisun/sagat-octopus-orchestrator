//! US03 — Survive an engine node failure. Given a scheduler shard owner goes down, when another
//! takes over the lease, then timers and work are resumed without inconsistency (US03-AC01).

use std::time::{Duration, Instant};

use serde_json::json;

use crate::isolated::IsolatedEngine;
use crate::support::worker::{engine_env, platform};

const LEASE_SECS: u64 = 3;

fn env(
    p: &crate::support::worker::Platform,
    replica: &str,
) -> std::collections::HashMap<String, String> {
    engine_env(
        &p.api.db,
        &p.api.idp,
        &[
            ("SAGAT_SHARD_LEASE_SECS", &LEASE_SECS.to_string()),
            ("SAGAT_REPLICA_ID", replica),
        ],
    )
}

async fn holders(p: &crate::support::worker::Platform) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT split_part(owner, '-', 1), count(*) FROM scheduler_shards \
         WHERE expires_at > clock_timestamp() GROUP BY 1 ORDER BY 1",
    )
    .fetch_all(&p.api.db.pool)
    .await
    .expect("holders")
}

async fn until<F, Fut>(what: &str, within: Duration, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + within;
    while !check().await {
        assert!(Instant::now() < deadline, "not within {within:?}: {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn when_an_engine_is_killed_the_other_takes_its_shards_and_the_timers_fire_once() {
    let p = platform().await;
    let mut alpha = IsolatedEngine::start(env(&p, "alpha")).await;
    let beta = IsolatedEngine::start(env(&p, "beta")).await;
    until(
        "the two engines share the 64 shards",
        Duration::from_secs(30),
        || async {
            let held = holders(&p).await;
            held.len() == 2 && held.iter().map(|(_, n)| n).sum::<i64>() == 64
        },
    )
    .await;

    // Forty runs whose timer falls due in two seconds: after the kill and before the lease runs out.
    let ir = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "failover"}, "entrypoint": "pause",
        "nodes": [
            {"id": "pause", "type": "timer", "durationMs": 2000, "next": "a"},
            {"id": "a", "type": "activity", "activity": "act.a"}
        ]
    });
    let workflow = p.publish("failover", &ir).await;
    let mut runs = Vec::new();
    for i in 0..40 {
        runs.push(p.start(&workflow, &json!({ "i": i })).await);
    }
    alpha.crash().await;
    let killed = Instant::now();

    // beta takes alpha's shards when their lease runs out, and fires what was due meanwhile.
    until(
        "beta holds every shard",
        Duration::from_secs(LEASE_SECS * 4),
        || async {
            holders(&p).await.iter().map(|(_, n)| n).sum::<i64>() == 64
                && holders(&p).await.len() == 1
        },
    )
    .await;
    let took_over = killed.elapsed();
    assert!(
        took_over <= Duration::from_secs(LEASE_SECS + 3),
        "the shards were taken over after {took_over:?}, with a lease of {LEASE_SECS} s"
    );
    until("every timer fired", Duration::from_secs(20), || async {
        let fired: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT run_id) FROM run_events WHERE event_type = 'timer.fired'",
        )
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count");
        fired == 40
    })
    .await;

    // Without inconsistency: each timer fired once, and the work after it goes on.
    let twice: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT run_id FROM run_events WHERE event_type = 'timer.fired' \
         GROUP BY run_id HAVING count(*) > 1) t",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count");
    assert_eq!(twice, 0, "a timer fired twice");
    let mut worker = p.worker_at(&beta.grpc_url, "wendy").await;
    worker.register(&["act.a"]).await;
    let mut done = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    while done < 40 {
        for task in worker.poll(16).await {
            worker.complete(&task, &json!({})).await.expect("complete");
            done += 1;
        }
        assert!(Instant::now() < deadline, "only {done} of 40 tasks");
    }
    for run in &runs {
        assert_eq!(p.status(run).await, "SUCCEEDED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_engine_that_is_stopped_gives_its_shards_back_for_the_other_to_take_at_once() {
    let p = platform().await;
    let mut alpha = IsolatedEngine::start(env(&p, "alpha")).await;
    let beta = IsolatedEngine::start(env(&p, "beta")).await;
    until(
        "the two engines share the shards",
        Duration::from_secs(30),
        || async { holders(&p).await.len() == 2 },
    )
    .await;

    alpha.stop().await;
    let stopped = Instant::now();
    until(
        "beta holds every shard",
        Duration::from_secs(LEASE_SECS * 2),
        || async {
            let held = holders(&p).await;
            held.len() == 1 && held[0].1 == 64
        },
    )
    .await;
    // A clean stop does not wait for the lease: it takes a keeper round, a third of it.
    assert!(
        stopped.elapsed() < Duration::from_secs(LEASE_SECS),
        "took {:?} after a clean stop",
        stopped.elapsed()
    );
    drop(beta);
}

//! US01 — Scale workers horizontally. Given the queue grows and the quota allows, when worker pools
//! scale, then throughput increases and no invalid claim exists (US01-AC01).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::support::worker::{RunningEngine, engine_env, platform, single_ir};

/// `tasks` runs of a one-activity workflow, drained by `workers` workers that each take `work` to
/// process a task. Returns how long it took and the runs each worker completed.
async fn drain(
    p: &crate::support::worker::Platform,
    engine: &RunningEngine,
    workflow: &str,
    tasks: usize,
    workers: usize,
    work: Duration,
    round: usize,
) -> (Duration, Vec<Vec<String>>) {
    for i in 0..tasks {
        p.start(workflow, &json!({ "i": i, "round": round })).await;
    }
    let mut clients = Vec::new();
    for n in 0..workers {
        let mut worker = p
            .worker_at(&engine.grpc_url, &format!("w{round}-{n}"))
            .await;
        worker.register(&["act.a"]).await;
        clients.push(worker);
    }
    let started = Instant::now();
    let done = futures_util::future::join_all(clients.into_iter().map(|mut worker| async move {
        let mut mine = Vec::new();
        let mut empty = 0;
        while empty < 3 {
            let mut leased = worker.poll(1).await;
            let Some(task) = leased.pop() else {
                empty += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            empty = 0;
            tokio::time::sleep(work).await;
            worker.complete(&task, &json!({})).await.expect("complete");
            mine.push(task.run_id.clone());
        }
        mine
    }))
    .await;
    (started.elapsed(), done)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn more_workers_finish_the_same_queue_sooner_and_no_task_goes_to_two_of_them() {
    let p = platform().await;
    let engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let workflow = p.publish("scale-out", &single_ir("scale-out", None)).await;
    let (tasks, work) = (80, Duration::from_millis(50));

    let (slow, few) = drain(&p, &engine, &workflow, tasks, 2, work, 1).await;
    let (fast, many) = drain(&p, &engine, &workflow, tasks, 8, work, 2).await;

    // No invalid claim: every task was completed by exactly one worker, none twice, none lost.
    for (label, done) in [("2 workers", &few), ("8 workers", &many)] {
        let all: Vec<&String> = done.iter().flatten().collect();
        let unique: BTreeSet<&&String> = all.iter().collect();
        assert_eq!(all.len(), tasks, "{label}: every task was completed");
        assert_eq!(unique.len(), tasks, "{label}: a task went to two workers");
    }
    assert!(
        many.iter().all(|mine| !mine.is_empty()),
        "every one of the eight took part"
    );
    // Throughput increased: four times the workers, a good part of four times the throughput.
    assert!(
        fast.as_secs_f64() < slow.as_secs_f64() * 0.6,
        "8 workers took {fast:?}, 2 workers took {slow:?}"
    );
    // And nothing is left behind.
    let unfinished: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE finished_at IS NULL")
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count");
    assert_eq!(unfinished, 0);
}

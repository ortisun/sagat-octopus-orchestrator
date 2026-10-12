//! The workloads and the checks of the disaster-recovery drill (`infra/dr/drill.sh`, T010-031,
//! FR-010-007). They are `#[ignore]`d: the script starts the PostgreSQL clusters, takes the base
//! backup, kills the primary and restores, and runs these in between. They talk to the script
//! through files in `SAGAT_DR_DIR`.
//!
//! - `dr_workload` makes runs in three phases, waiting for the script between them: before the base
//!   backup, while it is taken, and after the point the restore will stop at.
//! - `dr_acked_workload` starts runs as fast as it can and writes the id of every run the platform
//!   *acknowledged*; the script kills the primary under it.
//! - `dr_verify` replays the journal of every run of a database against its projections
//!   (`verify_run`), and, given the file of acknowledged runs, says how many of them are gone.

use std::io::Write as _;
use std::path::PathBuf;

use sagat_application::execution::DefinitionCache;
use sagat_domain::execution::RunId;
use sagat_domain::identity::TenantId;
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{PoolConfig, connect};

use super::*;

fn dir() -> PathBuf {
    PathBuf::from(std::env::var("SAGAT_DR_DIR").expect("SAGAT_DR_DIR names the drill's directory"))
}

fn signal(name: &str) {
    std::fs::write(dir().join(name), b"").expect("signal the script");
}

async fn wait_for(name: &str) {
    let path = dir().join(name);
    let deadline = Instant::now() + Duration::from_secs(1_800);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "the script never signalled {name}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Makes `k` timers due and carries their runs through the activity that follows: some of the
/// population is finished, some in flight, most suspended.
async fn progress(w: &World, k: usize) {
    make_due(w, w.admin.tenant_id(), k).await;
    let scheduler = replica_scheduler(w, None, SchedulerSettings::default()).await;
    while scheduler.tick().await.did_work() {}
    let session = w.session(&["act.work"]).await;
    loop {
        let tasks = w.poll(&session, 16).await;
        if tasks.is_empty() {
            break;
        }
        for task in &tasks {
            w.complete(&session, task, &json!({}))
                .await
                .expect("complete");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "run by infra/dr/drill.sh"]
async fn dr_workload() {
    let n = env_usize("SAGAT_DR_RUNS", 3_000);
    let w = world().await;
    std::fs::write(dir().join("db"), &w.db.name).expect("name the database");
    let wf = sleeper(&w, "dr-flow").await;

    start_runs(&w, wf, "dr-a", 0, n).await;
    progress(&w, n / 3).await;
    signal("a.done");

    // The base backup is taken while this phase runs: the backup is of a database in use.
    wait_for("backup.started").await;
    start_runs(&w, wf, "dr-b", 0, n).await;
    progress(&w, n / 3).await;
    signal("b.done");

    // What happens after the point the restore stops at, and is discarded by it.
    wait_for("target.set").await;
    start_runs(&w, wf, "dr-c", 0, n / 2).await;
    signal("c.done");
    wait_for("finish").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "run by infra/dr/drill.sh"]
async fn dr_acked_workload() {
    let w = world().await;
    std::fs::write(dir().join("db"), &w.db.name).expect("name the database");
    let wf = sleeper(&w, "dr-acked").await;
    let mut acked = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir().join("acked"))
        .expect("the file of acknowledged runs");
    signal("workload.ready");
    let counter = std::sync::atomic::AtomicUsize::new(0);
    let failures = std::sync::atomic::AtomicUsize::new(0);
    let (acked_tx, mut acked_rx) = tokio::sync::mpsc::unbounded_channel::<RunId>();
    let callers = futures_util::future::join_all((0..24).map(|_| {
        let (w, counter, failures, tx) = (&w, &counter, &failures, acked_tx.clone());
        async move {
            while failures.load(std::sync::atomic::Ordering::SeqCst) < 12
                && !dir().join("stop").exists()
            {
                let i = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match w
                    .start_with(wf, &json!({ "i": i }), &format!("dr-acked-{i:09}"), None)
                    .await
                {
                    // The platform said yes: from now on the run must exist, whatever happens.
                    Ok(run) => {
                        let _ = tx.send(run);
                    }
                    Err(_) => {
                        failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }
        }
    }));
    drop(acked_tx);
    let writer = async {
        while let Some(run) = acked_rx.recv().await {
            writeln!(acked, "{run}").expect("record an acknowledged run");
        }
    };
    tokio::join!(callers, writer);
    signal("workload.done");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "run by infra/dr/drill.sh"]
async fn dr_verify() {
    let url = std::env::var("SAGAT_DR_URL").expect("SAGAT_DR_URL: the database to check");
    let pool = connect(&PoolConfig::new(url)).await.expect("connect");
    let store = sagat_infra_postgres::PgRunStore::new(
        pool.clone(),
        DefinitionCache::new(std::sync::Arc::new(JsonIrCodec), 16),
    );
    let runs: Vec<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as("SELECT tenant_id, id FROM runs")
        .fetch_all(&pool)
        .await
        .expect("the runs");
    let results: Vec<Result<(), String>> = stream::iter(runs.iter().copied())
        .map(|(tenant, run)| {
            let store = store.clone();
            async move {
                store
                    .verify_run(TenantId::new(tenant), RunId::new(run))
                    .await
                    .map_err(|why| format!("run {run}: {why}"))
            }
        })
        .buffer_unordered(16)
        .collect()
        .await;
    let failed: Vec<&String> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    println!("verified={} failed={}", results.len(), failed.len());
    for why in failed.iter().take(10) {
        println!("  {why}");
    }
    assert!(
        failed.is_empty(),
        "{} runs do not replay to their projections",
        failed.len()
    );

    if let Ok(path) = std::env::var("SAGAT_DR_ACKED") {
        let acked: Vec<uuid::Uuid> = std::fs::read_to_string(path)
            .expect("the file of acknowledged runs")
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect();
        let present: std::collections::HashSet<uuid::Uuid> =
            runs.iter().map(|(_, run)| *run).collect();
        let lost = acked.iter().filter(|run| !present.contains(run)).count();
        println!(
            "acknowledged={} present={} lost={lost}",
            acked.len(),
            acked.len() - lost
        );
        if std::env::var("SAGAT_DR_EXPECT_ZERO_LOSS").is_ok() {
            assert_eq!(lost, 0, "{lost} acknowledged runs are gone");
        }
    }
}

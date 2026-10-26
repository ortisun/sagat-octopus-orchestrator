//! A real process dies after target commit but before the engine accepts completion.
use super::*;
use std::path::Path;

pub async fn run(
    p: &Platform,
    proxy: &proxy::Running,
    target: &sqlx::PgPool,
    config: &mut Value,
    binary: &Path,
    path: &Path,
) {
    config["namespaces"][0]["allowCidrs"] = json!(["127.0.0.0/8"]);
    std::fs::write(path, config.to_string()).unwrap();
    let spawn = || {
        Process(
            Command::new(binary)
                .env_clear()
                .env("SAGAT_DATABASE_RUNNER_CONFIG", path)
                .env("SAGAT_DATABASE_RUNNER_METRICS_ADDR", "127.0.0.1:0")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    proxy.faults.block_complete.store(true, Ordering::SeqCst);
    let mut process = spawn();
    let run = start(
        p,
        "crash-after-commit",
        "execute",
        "INSERT INTO effects VALUES (700, 'crash-effect') RETURNING id",
        json!([]),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while proxy.faults.blocked_completions.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("SQL committed and completion reached the fault boundary");
    let effect: i64 = sqlx::query_scalar("SELECT count(*) FROM effects WHERE id=700")
        .fetch_one(target)
        .await
        .unwrap();
    assert_eq!(effect, 1);
    assert_eq!(p.status(&run).await, "RUNNING");
    process.0.kill().unwrap(); // SIGKILL: no cancellation/report drain runs.
    process.0.wait().unwrap();
    tokio::time::sleep(Duration::from_secs(9)).await; // real eight-second lease TTL
    p.tick().await;
    assert_eq!(p.status(&run).await, "MANUAL_REVIEW");
    proxy.faults.block_complete.store(false, Ordering::SeqCst);
    let mut restarted = spawn();
    // Fresh work succeeds after the same worker identity restarts. No broker hint is required.
    let healthy = start(
        p,
        "crash-restart-query",
        "query",
        "SELECT 1::bigint",
        json!([]),
    )
    .await;
    finished(p, &healthy, "SUCCEEDED").await;
    p.tick().await;
    let attempt: i32 =
        sqlx::query_scalar("SELECT attempt FROM steps WHERE run_id=$1 AND node='database'")
            .bind(run.parse::<Uuid>().unwrap())
            .fetch_one(&p.api.db.pool)
            .await
            .unwrap();
    assert_eq!(attempt, 1, "unknown write never receives a second attempt");
    assert_eq!(p.status(&run).await, "MANUAL_REVIEW");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM effects WHERE id=700")
            .fetch_one(target)
            .await
            .unwrap(),
        1
    );
    restarted.stop().await;
}

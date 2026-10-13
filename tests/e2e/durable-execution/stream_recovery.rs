//! A streamed lease remains a database lease across receive cancellation and engine restart.
use std::time::Duration;

use sagat_api_grpc::pb::{TaskLease, worker_service_client::WorkerServiceClient};
use serde_json::json;

use crate::support::worker::{
    RunningEngine, Worker, engine_env, machine_code, platform, single_ir,
};

async fn streamed(worker: &mut Worker<'_>) -> TaskLease {
    let request = worker.req(worker.poll_message(1));
    let mut body = worker
        .client
        .stream_tasks(request)
        .await
        .expect("open stream")
        .into_inner();
    let frame = tokio::time::timeout(Duration::from_secs(6), body.message())
        .await
        .expect("bounded receive")
        .expect("receive")
        .expect("task frame");
    assert_eq!(frame.tasks.len(), 1);
    // Drop the response before reading its EOF. Neither this nor reconnecting acknowledges a task.
    frame.tasks.into_iter().next().expect("lease")
}

#[tokio::test]
async fn a_streamed_lease_can_complete_after_the_engine_restarts_without_registering_again() {
    let p = platform().await;
    let env = engine_env(&p.api.db, &p.api.idp, &[("SAGAT_LEASE_TTL_SECS", "30")]);
    let mut engine = RunningEngine::start(env.clone()).await;
    let wf = p
        .publish("stream-restart", &single_ir("stream-restart", None))
        .await;
    let run = p.start(&wf, &json!({})).await;
    let mut worker = p.worker_at(&engine.grpc_url, "stream-restart-worker").await;
    worker.register_version(&["act.a"], "1.4").await;
    let lease = streamed(&mut worker).await;
    engine.finish().await.expect("stop engine");
    let mut replacement = RunningEngine::start(env).await;
    worker.client = WorkerServiceClient::connect(replacement.grpc_url.clone())
        .await
        .expect("reconnect");
    worker
        .complete(&lease, &json!({"recovered": true}))
        .await
        .expect("durable lease accepted");
    worker
        .complete(&lease, &json!({"recovered": true}))
        .await
        .expect("idempotent repeat");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    let completed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id::text=$1 AND event_type='step.completed'",
    )
    .bind(&run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("journal");
    assert_eq!(completed, 1, "no duplicate committed transition");
    replacement.finish().await.expect("stop replacement");
}

#[tokio::test]
async fn an_abandoned_streamed_lease_expires_after_restart_and_fences_its_late_result() {
    let p = platform().await;
    let env = engine_env(&p.api.db, &p.api.idp, &[("SAGAT_LEASE_TTL_SECS", "1")]);
    let mut engine = RunningEngine::start(env.clone()).await;
    let wf = p
        .publish("stream-expiry", &single_ir("stream-expiry", None))
        .await;
    let run = p.start(&wf, &json!({})).await;
    let mut abandoned = p.worker_at(&engine.grpc_url, "stream-abandoned").await;
    abandoned.register_version(&["act.a"], "1.4").await;
    let stale = streamed(&mut abandoned).await;
    engine.finish().await.expect("stop engine");
    let mut replacement = RunningEngine::start(env).await;
    abandoned.client = WorkerServiceClient::connect(replacement.grpc_url.clone())
        .await
        .expect("reconnect");
    let mut rescuer = p.worker_at(&replacement.grpc_url, "stream-rescuer").await;
    rescuer.register_version(&["act.a"], "1.4").await;
    let fresh = streamed(&mut rescuer).await;
    assert_eq!(fresh.run_id, run);
    assert_eq!(fresh.attempt, stale.attempt + 1);
    assert!(fresh.generation > stale.generation);
    let refused = abandoned
        .complete(&stale, &json!({}))
        .await
        .expect_err("stale report");
    assert_eq!(machine_code(&refused), "STALE_ATTEMPT");
    rescuer.complete(&fresh, &json!({})).await.expect("rescued");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    let completed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id::text=$1 AND event_type='step.completed'",
    )
    .bind(&run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("journal");
    assert_eq!(completed, 1);
    replacement.finish().await.expect("stop replacement");
}

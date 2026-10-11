//! The `execution-engine` process itself: its loops do the work with nobody calling them, its
//! probes tell the truth, it refuses unsafe roles and it stops cleanly (T002-024, T002-031).

use std::time::{Duration, Instant};

use sagat_control_plane::RunError;
use sagat_control_plane::config::{Config, Environment};
use sagat_execution_engine::EngineConfig;
use serde_json::json;

use crate::support::nats;
use crate::support::worker::{RunningEngine, engine_env, machine_code, platform};
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, eventually, fresh_db};

async fn eventually_task(
    worker: &mut crate::support::worker::Worker<'_>,
    within: Duration,
) -> sagat_api_grpc::pb::TaskLease {
    let deadline = Instant::now() + within;
    loop {
        let mut tasks = worker.poll(8).await;
        if let Some(task) = tasks.pop() {
            return task;
        }
        assert!(Instant::now() < deadline, "no task within {within:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn the_engine_fires_timers_announces_work_and_serves_workers_with_nobody_driving_it() {
    let p = platform().await;
    let broker = nats::start().await;
    let engine = RunningEngine::start(engine_env(
        &p.api.db,
        &p.api.idp,
        &[("SAGAT_NATS_URL", &broker.url())],
    ))
    .await;

    let ir = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "reminder"}, "entrypoint": "pause",
        "nodes": [
            {"id": "pause", "type": "timer", "durationMs": 1000, "next": "a"},
            {"id": "a", "type": "activity", "activity": "act.a"}
        ]
    });
    let wf = p.publish("reminder", &ir).await;
    // The timer runs from the start of the run, so the clock starts there too: setup that follows
    // (registering, polling) must not eat into what the assertion below measures.
    let started = Instant::now();
    let run = p.start(&wf, &json!({})).await;
    assert_eq!(p.status(&run).await, "WAITING");

    let mut worker = p.worker_at(&engine.grpc_url, "wendy").await;
    worker.register(&["act.a"]).await;
    assert!(worker.poll(8).await.is_empty(), "the timer is not due");

    // Nobody calls the scheduler: the engine's own loop fires the timer and the task appears.
    let task = eventually_task(&mut worker, Duration::from_secs(10)).await;
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "not before the timer"
    );
    assert_eq!(task.activity_name, "act.a");
    worker.complete(&task, &json!({})).await.expect("complete");
    assert_eq!(p.status(&run).await, "SUCCEEDED");

    // The dispatcher announced the step (and the finish) on NATS without being asked either.
    let tenant = p.tenant;
    let subject = format!("sagat.v1.task.ready.{tenant}.act.a");
    let publisher =
        sagat_infra_nats::NatsPublisher::connect(sagat_infra_nats::NatsConfig::new(broker.url()))
            .await
            .expect("connect");
    let hints = eventually("the task.ready hint on NATS", || async {
        let seen = nats::drain(&publisher, &subject).await;
        (!seen.is_empty()).then_some(seen)
    })
    .await;
    let hint: serde_json::Value = serde_json::from_slice(&hints[0].1).expect("json");
    assert_eq!(hint["runId"], run);
    assert_eq!(hint["activity"], "act.a");
    let finished = eventually("the run.finished event on NATS", || async {
        let seen = nats::drain(&publisher, &format!("sagat.v1.run.finished.{tenant}")).await;
        (!seen.is_empty()).then_some(seen)
    })
    .await;
    let finished: serde_json::Value = serde_json::from_slice(&finished[0].1).expect("json");
    assert_eq!(finished["status"], "SUCCEEDED");
    eventually("an empty outbox", || async {
        (p.outbox_backlog().await == 0).then_some(())
    })
    .await;
}

#[tokio::test]
async fn the_engine_takes_a_dead_workers_step_back_by_itself() {
    let p = platform().await;
    let engine = RunningEngine::start(engine_env(
        &p.api.db,
        &p.api.idp,
        &[("SAGAT_LEASE_TTL_SECS", "1")],
    ))
    .await;
    let wf = p
        .publish("single", &crate::support::worker::single_ir("single", None))
        .await;
    let run = p.start(&wf, &json!({})).await;

    let mut doomed = p.worker_at(&engine.grpc_url, "wendy").await;
    doomed.register(&["act.a"]).await;
    let stale = eventually_task(&mut doomed, Duration::from_secs(5)).await;

    // The worker never heartbeats. The engine expires the lease, backs off and releases the retry;
    // another worker picks it up with a newer generation.
    let mut rescuer = p.worker_at(&engine.grpc_url, "rhea").await;
    rescuer.register(&["act.a"]).await;
    let fresh = eventually_task(&mut rescuer, Duration::from_secs(20)).await;
    assert_eq!(fresh.run_id, run);
    assert!(fresh.generation > stale.generation);
    assert_eq!(fresh.attempt, stale.attempt + 1);

    let late = doomed
        .complete(&stale, &json!({}))
        .await
        .expect_err("stale");
    assert_eq!(machine_code(&late), "STALE_ATTEMPT");
    rescuer.complete(&fresh, &json!({})).await.expect("rescued");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}

#[tokio::test]
async fn probes_tell_the_truth_and_metrics_are_served_on_the_admin_listener_only() {
    let p = platform().await;
    let engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    assert_eq!(engine.status("/livez").await, Some(200));
    assert_eq!(engine.status("/readyz").await, Some(200));
    assert_eq!(engine.status("/nope").await, Some(404));
    // The grace is one second in these tests: still alive after twice that means the loop is
    // really ticking, not merely recently started.
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(
        engine.status("/livez").await,
        Some(200),
        "the scheduler keeps ticking"
    );

    // Work through the gRPC API so there is something to count.
    let mut worker = p.worker_at(&engine.grpc_url, "wendy").await;
    worker.register(&["act.a"]).await;
    assert!(worker.poll(1).await.is_empty());
    let metrics = engine.body("/metrics").await;
    assert!(metrics.contains("grpc_requests_total"), "{metrics}");
    assert!(
        metrics.contains("scheduler_actions_total") || metrics.contains("grpc_request_duration"),
        "{metrics}"
    );

    // The worker API does not serve the admin surface, nor the REST API.
    let wrong = reqwest::get(format!("{}/metrics", engine.grpc_url)).await;
    assert!(
        wrong.map_or(true, |r| r.status().as_u16() != 200),
        "metrics must not be on the worker port"
    );
}

#[tokio::test]
async fn an_engine_pointed_at_an_unmigrated_database_is_alive_but_not_ready_until_the_migration_runs()
 {
    let db = fresh_db().await; // no schema at all
    let idp = crate::support::oidc::TestIdp::start().await;
    let engine = RunningEngine::start(engine_env(&db, &idp, &[])).await;
    assert_eq!(
        engine.status("/livez").await,
        Some(200),
        "the process is fine"
    );
    assert_eq!(
        engine.status("/readyz").await,
        Some(503),
        "but it cannot do its work yet"
    );
    sagat_infra_postgres::migrate(&db.pool)
        .await
        .expect("migrate");
    eventually("readiness after the migration", || async {
        (engine.status("/readyz").await == Some(200)).then_some(())
    })
    .await;
}

#[tokio::test]
async fn outside_development_the_engine_refuses_a_role_that_could_rewrite_the_audit_trail() {
    let p = platform().await;
    // Production settings (the TLS rule of the environment parser aside: the test database has
    // none), a key URL that is never fetched before the privilege check, and two database roles.
    let production = |url: String| {
        let env = engine_env(
            &p.api.db,
            &p.api.idp,
            &[
                ("DATABASE_URL", &url),
                ("SAGAT_ALLOW_INSECURE_JWKS_HTTP", "false"),
                ("SAGAT_OIDC_JWKS_URI", "https://idp.test.invalid/jwks.json"),
            ],
        );
        let lookup = move |name: &str| env.get(name).cloned();
        let mut config = Config::from_lookup(&lookup).expect("config");
        config.environment = Environment::Prod;
        let engine = EngineConfig::from_lookup(&lookup, Environment::Prod).expect("engine");
        (config, engine)
    };

    // The schema owner can rewrite the audit trail: refused, and the exit code says it is the
    // operator's configuration to fix.
    let (config, engine) = production(p.api.db.url.clone());
    let refused = RunningEngine::try_start_with(config, engine).await;
    match refused {
        Err(error @ RunError::LeastPrivilege(_)) => assert_eq!(error.exit_code(), 2),
        other => panic!("the owner role must be refused, got {:?}", other.err()),
    }

    // The runtime role is what it is meant to run as: same settings, and it starts.
    let runtime = p.api.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD);
    let (config, engine) = production(runtime);
    let mut engine = RunningEngine::try_start_with(config, engine)
        .await
        .expect("the least-privilege role is accepted");
    assert_eq!(engine.status("/readyz").await, Some(200));
    engine.finish().await.expect("clean stop");
}

#[tokio::test]
async fn the_insecure_key_url_is_refused_outside_development() {
    let p = platform().await;
    let url = format!("{}?sslmode=require", p.api.db.url);
    let env = engine_env(
        &p.api.db,
        &p.api.idp,
        &[("SAGAT_ENV", "prod"), ("DATABASE_URL", &url)],
    );
    match RunningEngine::try_start(env).await {
        Err(RunError::Refused(reason)) => assert!(reason.contains("INSECURE_JWKS"), "{reason}"),
        other => panic!("expected a refusal, got {:?}", other.err()),
    }
}

#[tokio::test]
async fn stopping_the_engine_drains_the_server_and_ends_every_loop() {
    let p = platform().await;
    let mut engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let mut worker = p.worker_at(&engine.grpc_url, "wendy").await;
    worker.register(&["act.a"]).await;
    assert!(worker.poll(1).await.is_empty());
    let grpc = engine.grpc_url.clone();

    let started = Instant::now();
    engine.finish().await.expect("a clean stop");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "stops promptly"
    );
    assert!(
        sagat_api_grpc::pb::worker_service_client::WorkerServiceClient::connect(grpc)
            .await
            .is_err(),
        "the worker port is closed"
    );
}

#[tokio::test]
async fn without_nats_workers_still_poll_and_the_outbox_waits_for_a_dispatcher() {
    let p = platform().await;
    let engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let wf = p
        .publish("single", &crate::support::worker::single_ir("single", None))
        .await;
    let run = p.start(&wf, &json!({})).await;
    let mut worker = p.worker_at(&engine.grpc_url, "wendy").await;
    worker.register(&["act.a"]).await;
    let task = eventually_task(&mut worker, Duration::from_secs(5)).await;
    worker.complete(&task, &json!({})).await.expect("complete");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        p.outbox_backlog().await >= 2,
        "nothing publishes the hints, and nothing loses them either"
    );
}

#[tokio::test]
async fn a_parent_run_waits_for_its_child_across_the_whole_stack_and_the_api_shows_the_link() {
    let p = platform().await;
    let engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let kid = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "kid-flow"}, "entrypoint": "kid",
        "nodes": [{"id": "kid", "type": "activity", "activity": "act.kid"}]
    });
    p.publish("kid-flow", &kid).await;
    let mother = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "mother-flow"}, "entrypoint": "c",
        "nodes": [
            {"id": "c", "type": "child", "workflowId": "kid-flow", "input": "input.order", "next": "after"},
            {"id": "after", "type": "activity", "activity": "act.after"}
        ]
    });
    let wf = p.publish("mother-flow", &mother).await;
    let run = p.start(&wf, &json!({"order": {"id": 5}})).await;
    assert_eq!(p.status(&run).await, "WAITING");

    // The engine's own loop creates the child; the worker finds its task.
    let mut worker = p.worker_at(&engine.grpc_url, "wendy").await;
    worker.register(&["act.kid", "act.after"]).await;
    let task = eventually_task(&mut worker, Duration::from_secs(10)).await;
    assert_eq!(task.activity_name, "act.kid");
    assert_ne!(task.run_id, run, "the task belongs to the child run");
    assert!(task.input_json.contains("\"id\":5"), "{}", task.input_json);

    // The REST API shows the link in both directions.
    let child = p.run(&task.run_id).await;
    assert_eq!(child["parent"]["runId"], run);
    assert_eq!(child["parent"]["node"], "c");
    assert_eq!(child["parent"]["cancelPolicy"], "PROPAGATE");
    assert_eq!(child["depth"], 1);
    let (status, graph) = p
        .rest(reqwest::Method::GET, &format!("/runs/{run}/graph"), None)
        .await;
    assert_eq!(status, 200);
    let node = graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["id"] == "c")
        .expect("c");
    assert_eq!(node["status"], "WAITING");
    assert_eq!(node["waitingOn"], "child");
    assert_eq!(node["childRunId"], task.run_id);
    assert_eq!(p.run(&run).await["parent"], serde_json::Value::Null);

    // Finishing the child releases the parent's next step, again without anyone calling the engine.
    worker
        .complete(&task, &json!({}))
        .await
        .expect("child step");
    let after = eventually_task(&mut worker, Duration::from_secs(10)).await;
    assert_eq!(after.activity_name, "act.after");
    assert_eq!(after.run_id, run);
    worker.complete(&after, &json!({})).await.expect("after");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    assert_eq!(p.status(&task.run_id).await, "SUCCEEDED");
}

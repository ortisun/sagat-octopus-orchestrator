//! Publishable top-level `limits` (D297, WP-12): captured when a run is created and enforced when
//! the next instance would be admitted, with durable typed failures, against real PostgreSQL.

use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::quotas::QuotaService;
use sagat_application::worker::{Capabilities, WorkerService, WorkerSettings};
use sagat_domain::quota::QuotaResource;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::support::engine::{World, activity, ctx, document, task_ref, with, world};

fn limited(name: &str, nodes: Vec<Value>, limits: Option<Value>) -> Value {
    let entry = nodes[0]["id"].as_str().expect("entry id").to_owned();
    let mut document = document(name, &entry, nodes);
    if let Some(limits) = limits {
        document["limits"] = limits;
    }
    document
}

/// `a -> b -> done`: three instances if every node runs, the `end` node included.
fn chain_of_two(name: &str, limits: Option<Value>) -> Value {
    limited(
        name,
        vec![
            with(activity("a"), "next", json!("b")),
            with(activity("b"), "next", json!("done")),
            json!({"id": "done", "type": "end"}),
        ],
        limits,
    )
}

async fn usage(world: &World, run: sagat_domain::execution::RunId) -> (i32, i16, i32, i32) {
    let row = sqlx::query(
        "SELECT max_instances,max_depth,admitted_instances,reserved_instances \
         FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("run instance usage");
    (
        row.get("max_instances"),
        row.get("max_depth"),
        row.get("admitted_instances"),
        row.get("reserved_instances"),
    )
}

#[tokio::test]
async fn run_creation_captures_the_lowered_limits_and_old_documents_keep_the_ceilings() {
    let world = world().await;
    let (limited_flow, _) = world
        .publish(chain_of_two(
            "limits-captured",
            Some(json!({"maxInstances": 7, "maxDepth": 2})),
        ))
        .await;
    let run = world.start(limited_flow, &json!({})).await;
    let (max_instances, max_depth, _, _) = usage(&world, run).await;
    assert_eq!((max_instances, max_depth), (7, 2));

    let (old_flow, _) = world.publish(chain_of_two("limits-absent", None)).await;
    let run = world.start(old_flow, &json!({})).await;
    let (max_instances, max_depth, _, _) = usage(&world, run).await;
    assert_eq!(
        (max_instances, max_depth),
        (100_000, 8),
        "a document without the block keeps the platform ceilings"
    );
}

#[tokio::test]
async fn the_last_instance_inside_the_limit_still_runs() {
    let world = world().await;
    let (workflow, _) = world
        .publish(chain_of_two(
            "limits-boundary",
            Some(json!({"maxInstances": 3})),
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let session = world.session(&["act.a", "act.b"]).await;
    let task = world.one_task_of(&session, "act.a").await;
    world
        .complete(&session, &task, &json!({}))
        .await
        .expect("a");
    let task = world.one_task_of(&session, "act.b").await;
    world
        .complete(&session, &task, &json!({}))
        .await
        .expect("b");
    assert_eq!(world.run_status(run).await.as_str(), "SUCCEEDED");
    assert_eq!(usage(&world, run).await.2, 3);
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn the_instance_beyond_the_limit_fails_the_run_with_instance_limit() {
    let world = world().await;
    let (workflow, _) = world
        .publish(chain_of_two(
            "limits-excess",
            Some(json!({"maxInstances": 2})),
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let session = world.session(&["act.a", "act.b"]).await;
    let task = world.one_task_of(&session, "act.a").await;
    world
        .complete(&session, &task, &json!({}))
        .await
        .expect("a");
    let task = world.one_task_of(&session, "act.b").await;
    // Completing `b` would activate the `end` node as a third instance.
    world
        .complete(&session, &task, &json!({}))
        .await
        .expect("the completion itself is accepted; the next instance is what is refused");

    assert_eq!(world.run_status(run).await.as_str(), "FAILED");
    let code: Option<String> =
        sqlx::query_scalar("SELECT stop_code FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(world.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("stop code");
    assert_eq!(code.as_deref(), Some("INSTANCE_LIMIT"));
    let journal = world.journal(run).await;
    assert!(
        journal.iter().any(|event| event == "run.stop_requested"),
        "the refusal is a journaled stop, not a silent drop: {journal:?}"
    );
    assert_eq!(
        world.step_status(run, "done").await.map(|s| s.as_str()),
        Some("CANCELLED"),
        "the refused instance never ran"
    );
    assert_eq!(
        usage(&world, run).await.2,
        2,
        "nothing beyond the limit was charged"
    );
    world.assert_consistent(run).await;
}

// ---------------------------------------------------------------------------------------------
// Scoped admission: collections and loops
// ---------------------------------------------------------------------------------------------

async fn scheduler(
    world: &World,
    name: &str,
) -> Scheduler<PgRunStore, JsonEvaluator, SystemEntropy> {
    let placement = Placement::new(format!("limits-scheduler-{name}-{}", Uuid::now_v7()));
    Keeper::new(
        world.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    Scheduler::new(
        world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_children(world.cache.clone())
    .with_placement(placement)
}

async fn allow_iterations(world: &World, limit: u64) {
    QuotaService::set(
        &world.quotas,
        &world.admin,
        None,
        QuotaResource::IterationConcurrency,
        limit,
        &ctx(),
    )
    .await
    .expect("configure iteration capacity");
}

/// `items` collection of one activity and a return: an iteration charges 2 instances.
fn collection(name: &str, max_items: u32, limits: Option<Value>) -> Value {
    let mut document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "items",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "limits.line", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "items", "type": "forEach", "items": "input.items", "body": "line",
             "result": "bodyResult", "maxItems": max_items, "maxConcurrency": 2, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    });
    if let Some(limits) = limits {
        document["limits"] = limits;
    }
    document
}

/// Ticks until the run is finished (a stopped run needs a pass or two to drain and finish).
async fn settle(
    scheduler: &Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>,
    world: &World,
    run: sagat_domain::execution::RunId,
) {
    for _ in 0..8 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        if world.run_status(run).await.is_terminal() {
            return;
        }
    }
}

/// A run stopped by a loop checkpoint is `CANCELLING` with its durable failure until the engine
/// consumes the committed hint; one stopped by the planner is already `FAILED`.
async fn assert_failed_with(world: &World, run: sagat_domain::execution::RunId, code: &str) {
    let status = world.run_status(run).await;
    assert!(
        matches!(status.as_str(), "CANCELLING" | "FAILED"),
        "the run is stopping or stopped, not {}",
        status.as_str()
    );
    let reason: Option<String> =
        sqlx::query_scalar("SELECT stop_reason FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(world.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("stop reason");
    assert_eq!(reason.as_deref(), Some("FAILED"));
    assert_eq!(stop_code(world, run).await.as_deref(), Some(code));
}

async fn stop_code(world: &World, run: sagat_domain::execution::RunId) -> Option<String> {
    sqlx::query_scalar("SELECT stop_code FROM runs WHERE tenant_id=$1 AND id=$2")
        .bind(world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&world.db.pool)
        .await
        .expect("stop code")
}

async fn scopes(world: &World, run: sagat_domain::execution::RunId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2")
        .bind(world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&world.db.pool)
        .await
        .expect("scope count")
}

#[tokio::test]
async fn more_items_than_the_collection_limit_fail_the_run_with_iteration_limit() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world.publish(collection("limits-items", 2, None)).await;
    let run = world.start(workflow, &json!({"items": [1, 2, 3]})).await;
    let scheduler = scheduler(&world, "items").await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(world.run_status(run).await.as_str(), "FAILED");
    assert_eq!(
        stop_code(&world, run).await.as_deref(),
        Some("ITERATION_LIMIT")
    );
    assert_eq!(scopes(&world, run).await, 0, "nothing was admitted");
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_collection_that_cannot_be_admitted_within_the_instance_limit_fails_before_it_starts() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let scheduler = scheduler(&world, "instances").await;
    // The owner is one instance and each of the three iterations charges two: seven in all.
    let (tight, _) = world
        .publish(collection(
            "limits-instances-tight",
            4,
            Some(json!({"maxInstances": 6})),
        ))
        .await;
    let run = world.start(tight, &json!({"items": [1, 2, 3]})).await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(world.run_status(run).await.as_str(), "FAILED");
    assert_eq!(
        stop_code(&world, run).await.as_deref(),
        Some("INSTANCE_LIMIT")
    );
    assert_eq!(scopes(&world, run).await, 0);
    world.assert_consistent(run).await;

    let (exact, _) = world
        .publish(collection(
            "limits-instances-exact",
            4,
            Some(json!({"maxInstances": 7})),
        ))
        .await;
    let run = world.start(exact, &json!({"items": [1, 2, 3]})).await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(report.collections_advanced, 1, "{report:?}");
    assert_eq!(scopes(&world, run).await, 1, "an exact fit enters");
    assert_eq!(world.run_status(run).await.as_str(), "WAITING");
}

#[tokio::test]
async fn a_tenant_ceiling_below_the_authors_limit_is_never_raised() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world
        .publish(collection(
            "limits-never-raised",
            4,
            Some(json!({"maxInstances": 100})),
        ))
        .await;
    let run = world.start(workflow, &json!({"items": [1, 2]})).await;
    sqlx::query("UPDATE run_instance_usage SET max_instances=3 WHERE tenant_id=$1 AND run_id=$2")
        .bind(world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .execute(&world.db.pool)
        .await
        .expect("tenant policy lowers the run");
    let report = scheduler(&world, "lowered").await.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(world.run_status(run).await.as_str(), "FAILED");
    assert_eq!(
        stop_code(&world, run).await.as_deref(),
        Some("INSTANCE_LIMIT")
    );
    let (max_instances, ..) = usage(&world, run).await;
    assert_eq!(
        max_instances, 3,
        "the author's 100 did not raise the lowered ceiling"
    );
}

#[tokio::test]
async fn a_policy_reduction_midway_pauses_admission_without_failing_or_killing_accepted_work() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world.publish(collection("limits-reduced", 4, None)).await;
    let run = world.start(workflow, &json!({"items": [1, 2, 3]})).await;
    let scheduler = scheduler(&world, "reduced").await;
    let opened = scheduler.tick().await;
    assert_eq!(opened.collections_advanced, 1, "{opened:?}");
    let admitted = scheduler.tick().await;
    assert_eq!(admitted.collections_advanced, 1, "{admitted:?}");
    let (_, _, consumed, reserved) = usage(&world, run).await;
    sqlx::query("UPDATE run_instance_usage SET max_instances=$3 WHERE tenant_id=$1 AND run_id=$2")
        .bind(world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .bind(consumed + reserved)
        .execute(&world.db.pool)
        .await
        .expect("reduce to what is already accepted");
    let paused = scheduler.tick().await;
    assert_eq!(
        paused.failed, 0,
        "a reduction is a pause, not an error: {paused:?}"
    );
    assert_eq!(world.run_status(run).await.as_str(), "RUNNING");
    assert_eq!(
        usage(&world, run).await.2,
        consumed,
        "accepted work keeps its charge"
    );
    sqlx::query(
        "UPDATE run_instance_usage SET max_instances=100000 WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .execute(&world.db.pool)
    .await
    .expect("lift the reduction");
    let resumed = scheduler.tick().await;
    assert_eq!(resumed.collections_advanced, 1, "{resumed:?}");
}

// ---------------------------------------------------------------------------------------------
// Loops
// ---------------------------------------------------------------------------------------------

/// `again` runs its body while `carry < 3`; the body returns the activity's output. One iteration
/// charges two instances (the activity and its return), the owner one, the `end` node one.
fn looping(name: &str, max_iterations: u32, delay_ms: Option<u64>, limits: Option<Value>) -> Value {
    let mut owner = json!({
        "id": "again", "type": "while", "condition": "carry < 3", "body": "iteration",
        "initial": "0", "carry": "bodyResult", "maxIterations": max_iterations,
        "timeoutMs": 600_000, "next": "done"
    });
    if let Some(delay) = delay_ms {
        owner["delayMs"] = json!(delay);
    }
    let mut document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "again",
        "bodies": {"iteration": {"entrypoint": "step", "nodes": [
            {"id": "step", "type": "activity", "activity": "limits.step", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.step.output"}
        ]}},
        "nodes": [owner, {"id": "done", "type": "end", "outputExpr": "steps.again.output"}]
    });
    if let Some(limits) = limits {
        document["limits"] = limits;
    }
    document
}

fn scoped_workers(
    world: &World,
) -> WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy> {
    WorkerService::new(
        PgRunStore::new(world.db.pool.clone(), world.cache.clone()).with_scoped_worker_protocol(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_vault(world.keys.cipher())
}

/// Completes the one task the loop's current iteration offers, with `output`.
async fn finish_iteration(
    world: &World,
    workers: &WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>,
    session: &(uuid::Uuid, String),
    output: i64,
) {
    let tasks = workers
        .poll(
            &world.worker,
            session.0,
            &session.1,
            1,
            Some("0af7651916cd43dd8448eb211c80319c"),
        )
        .await
        .expect("poll the iteration");
    assert_eq!(tasks.len(), 1, "one body activity is offered at a time");
    workers
        .complete(
            &world.worker,
            &task_ref(&tasks[0]),
            output.to_string().as_bytes(),
            &format!("finish-{}", Uuid::now_v7()),
        )
        .await
        .expect("complete the iteration");
}

async fn register_scoped(
    world: &World,
    workers: &WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>,
) -> (uuid::Uuid, String) {
    world.bind_everything(&world.worker).await;
    let registered = workers
        .register(
            &world.worker,
            "payments",
            &format!("limits-loop-worker-{}", Uuid::now_v7()),
            &Capabilities {
                activities: vec!["limits.step".into()],
                languages: vec!["rust".into()],
                pools: Vec::new(),
                max_inflight: 1,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("register a scoped worker");
    (
        registered.worker_id,
        registered.session_token.expose().to_owned(),
    )
}

async fn loop_scope(
    world: &World,
    run: sagat_domain::execution::RunId,
) -> Option<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT s.lifecycle, c.continuation FROM loop_scopes s \
         LEFT JOIN loop_checkpoints c ON c.tenant_id=s.tenant_id AND c.run_id=s.run_id \
          AND c.owner_instance_id=s.owner_instance_id AND c.iteration_index=0 \
         WHERE s.tenant_id=$1 AND s.run_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_optional(&world.db.pool)
    .await
    .expect("loop scope")
}

#[tokio::test]
async fn a_loop_that_cannot_afford_its_next_iteration_fails_with_instance_limit() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Owner (1) + first iteration (2) = 3; a second iteration would need five.
    let (workflow, _) = world
        .publish(looping(
            "limits-loop-excess",
            5,
            None,
            Some(json!({"maxInstances": 4})),
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "loop-excess").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;

    let entered = scheduler.tick().await;
    assert_eq!(entered.loops_advanced, 1, "{entered:?}");
    finish_iteration(&world, &workers, &session, 1).await;
    let checkpointed = scheduler.tick().await;
    assert_eq!(checkpointed.failed, 0, "{checkpointed:?}");
    assert_eq!(checkpointed.loops_advanced, 1, "{checkpointed:?}");

    assert_failed_with(&world, run, "INSTANCE_LIMIT").await;
    assert_eq!(
        loop_scope(&world, run).await,
        Some(("FAILED".to_owned(), Some("INSTANCE_LIMIT".to_owned())))
    );
    assert_eq!(
        usage(&world, run).await.2,
        3,
        "the refused iteration was never charged"
    );
    let journal = world.journal(run).await;
    assert!(
        journal
            .iter()
            .any(|event| event == "loop.iteration_checkpointed"),
        "the completed iteration is kept: {journal:?}"
    );
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_loop_whose_next_iteration_exactly_fits_continues() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world
        .publish(looping(
            "limits-loop-fits",
            5,
            None,
            Some(json!({"maxInstances": 5})),
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "loop-fits").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    finish_iteration(&world, &workers, &session, 1).await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(world.run_status(run).await.as_str(), "RUNNING");
    let (_, _, admitted, reserved) = usage(&world, run).await;
    assert_eq!(
        (admitted, reserved),
        (4, 1),
        "the second iteration's activity is admitted and its return reserved"
    );
}

#[tokio::test]
async fn a_loop_that_cannot_afford_its_first_iteration_fails_before_it_starts() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world
        .publish(looping(
            "limits-loop-first",
            5,
            None,
            Some(json!({"maxInstances": 2})),
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "loop-first").await;
    settle(&scheduler, &world, run).await;
    assert_eq!(world.run_status(run).await.as_str(), "FAILED");
    assert_eq!(
        stop_code(&world, run).await.as_deref(),
        Some("INSTANCE_LIMIT")
    );
    assert_eq!(loop_scope(&world, run).await, None, "no scope was entered");
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_delayed_loop_is_refused_before_it_waits_for_an_iteration_it_cannot_afford() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world
        .publish(looping(
            "limits-loop-delay",
            5,
            Some(60_000),
            Some(json!({"maxInstances": 4})),
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "loop-delay").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    finish_iteration(&world, &workers, &session, 1).await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "INSTANCE_LIMIT").await;
    let timers: i64 =
        sqlx::query_scalar("SELECT count(*) FROM loop_timers WHERE tenant_id=$1 AND run_id=$2")
            .bind(world.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("timers");
    assert_eq!(
        timers, 0,
        "no delay was scheduled for work that can never be admitted"
    );
}

#[tokio::test]
async fn a_loop_that_needs_more_than_its_iteration_limit_fails_with_iteration_limit() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world
        .publish(looping("limits-loop-iterations", 1, None, None))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "loop-iterations").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    finish_iteration(&world, &workers, &session, 1).await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "ITERATION_LIMIT").await;
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_finished_loop_whose_successor_cannot_be_afforded_stops_with_instance_limit() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Owner (1) + one iteration (2) = 3; the `end` node after the loop would be the fourth.
    let (workflow, _) = world
        .publish(looping(
            "limits-loop-successor",
            5,
            None,
            Some(json!({"maxInstances": 3})),
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "loop-successor").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    // The carry reaches 3, so the loop is done.
    finish_iteration(&world, &workers, &session, 3).await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "INSTANCE_LIMIT").await;
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_tenant_reduction_below_the_next_iteration_stops_the_loop_with_instance_limit() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world
        .publish(looping("limits-loop-reduced", 5, None, None))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "loop-reduced").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    // The author declared nothing; tenant policy lowers the run to what is already accepted.
    sqlx::query("UPDATE run_instance_usage SET max_instances=3 WHERE tenant_id=$1 AND run_id=$2")
        .bind(world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .execute(&world.db.pool)
        .await
        .expect("tenant policy lowers the run");
    finish_iteration(&world, &workers, &session, 1).await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "INSTANCE_LIMIT").await;
    assert_eq!(
        usage(&world, run).await.2,
        3,
        "accepted work keeps its charge"
    );
    world.assert_consistent(run).await;
}

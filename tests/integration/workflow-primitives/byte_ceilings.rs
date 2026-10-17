//! The existing byte ceilings (256 KiB per value, 4 MiB retained per run, 8 MiB per task context)
//! hold for collection and loop values too, against real PostgreSQL. A value over a ceiling is a
//! durable `RUN_DATA_LIMIT` failure, never a retry loop and never a silently stored oversize value.

use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{MAX_PAYLOAD_BYTES, MAX_RUN_OUTPUT_BYTES};
use sagat_application::worker::{MAX_TASK_INPUT_BYTES, WorkerService};
use sagat_domain::execution::RunId;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use serde_json::{Value, json};

use crate::limits::{
    allow_iterations, assert_failed_with, register_scoped, scheduler, scoped_workers, scopes,
};
use crate::support::engine::{World, task_ref, world};

type Workers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;

fn text_of(bytes: usize) -> String {
    // A JSON string of exactly `bytes` bytes: the quotes count.
    format!("\"{}\"", "x".repeat(bytes - 2))
}

pub(crate) fn collection(
    name: &str,
    items: &str,
    max_items: u32,
    max_concurrency: u16,
    result: &str,
) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "items",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "limits.step", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "items", "type": "forEach", "items": items, "body": "line",
             "result": result, "maxItems": max_items, "maxConcurrency": max_concurrency,
             "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn looping(name: &str, initial: &str, carry: &str) -> Value {
    looping_until(name, "while", initial, carry, "size(carry) >= 0", 3)
}

fn looping_until(
    name: &str,
    kind: &str,
    initial: &str,
    carry: &str,
    condition: &str,
    max_iterations: u32,
) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "again",
        "bodies": {"iteration": {"entrypoint": "step", "nodes": [
            {"id": "step", "type": "activity", "activity": "limits.step", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.step.output"}
        ]}},
        "nodes": [
            {"id": "again", "type": kind, "condition": condition, "body": "iteration",
             "initial": initial, "carry": carry, "maxIterations": max_iterations,
             "timeoutMs": 600_000, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

/// Polls what is offered and completes each task with `output` (raw JSON text).
pub(crate) async fn answer(
    world: &World,
    workers: &Workers,
    session: &(uuid::Uuid, String),
    output: &str,
) -> usize {
    let tasks = workers
        .poll(
            &world.worker,
            session.0,
            &session.1,
            8,
            Some("0af7651916cd43dd8448eb211c80319c"),
        )
        .await
        .expect("poll the scope");
    for task in &tasks {
        workers
            .complete(
                &world.worker,
                &task_ref(task),
                output.as_bytes(),
                &format!("answer-{}", uuid::Uuid::now_v7()),
            )
            .await
            .expect("complete the member");
    }
    tasks.len()
}

/// Whether the run is stopping or finished: it is the end of what a test drives.
async fn is_over(world: &World, run: RunId) -> bool {
    matches!(
        world.run_status(run).await.as_str(),
        "CANCELLING" | "FAILED" | "SUCCEEDED" | "CANCELLED" | "TIMED_OUT"
    )
}

async fn start_collection(world: &World, document: Value, input: &Value) -> RunId {
    let (workflow, _) = world.publish(document).await;
    world.start(workflow, input).await
}

#[tokio::test]
async fn a_collection_snapshot_over_the_value_ceiling_fails_the_run_before_it_starts() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Two 80 KiB items fit the 256 KiB input; the doubled list is 320 KiB.
    let big = "y".repeat(80 * 1024);
    let run = start_collection(
        &world,
        collection(
            "bytes-snapshot",
            "input.items + input.items",
            4,
            2,
            "bodyResult",
        ),
        &json!({"items": [big.clone(), big]}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-snapshot").await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    assert_eq!(
        scopes(&world, run).await,
        0,
        "no oversize value was retained"
    );
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_member_output_over_the_value_ceiling_is_refused_and_the_lease_survives() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let run = start_collection(
        &world,
        collection("bytes-member", "input.items", 2, 1, "bodyResult"),
        &json!({"items": [1]}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-member").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    let tasks = workers
        .poll(&world.worker, session.0, &session.1, 1, None)
        .await
        .expect("poll");
    assert_eq!(tasks.len(), 1);
    let oversize = text_of(MAX_PAYLOAD_BYTES + 1);
    assert!(
        workers
            .complete(
                &world.worker,
                &task_ref(&tasks[0]),
                oversize.as_bytes(),
                "oversize"
            )
            .await
            .is_err(),
        "a scoped member output is held to the same 256 KiB as a root one"
    );
    let fits = text_of(MAX_PAYLOAD_BYTES);
    workers
        .complete(
            &world.worker,
            &task_ref(&tasks[0]),
            fits.as_bytes(),
            "at-the-ceiling",
        )
        .await
        .expect("exactly 256 KiB is allowed");
    assert_eq!(world.run_status(run).await.as_str(), "RUNNING");
}

#[tokio::test]
async fn member_outputs_that_pass_four_mib_in_all_fail_the_run_with_run_data_limit() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // 250,000-byte outputs: sixteen retain 4,000,000 bytes, the seventeenth passes 4 MiB.
    let items: Vec<u32> = (0..17).collect();
    let run = start_collection(
        &world,
        collection("bytes-run", "input.items", 17, 1, "1"),
        &json!({"items": items}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-run").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    let output = text_of(250_000);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    for _ in 0..40 {
        scheduler.tick().await;
        answer(&world, &workers, &session, &output).await;
        scheduler.tick().await;
        if is_over(&world, run).await {
            break;
        }
    }
    const { assert!(16 * 250_000 < MAX_RUN_OUTPUT_BYTES && 17 * 250_000 > MAX_RUN_OUTPUT_BYTES) };
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
}

#[tokio::test]
async fn an_aggregate_over_the_value_ceiling_fails_the_scope_instead_of_storing_it() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Three 100 KiB outputs are each fine and together 300 KiB, over the 256 KiB value ceiling.
    let run = start_collection(
        &world,
        collection("bytes-aggregate", "input.items", 3, 1, "bodyResult"),
        &json!({"items": [1, 2, 3]}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-aggregate").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    let output = text_of(100 * 1024);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    for _ in 0..12 {
        scheduler.tick().await;
        answer(&world, &workers, &session, &output).await;
        scheduler.tick().await;
        if is_over(&world, run).await {
            break;
        }
    }
    let last = scheduler.tick().await;
    assert_eq!(last.failed, 0, "{last:?}");
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payloads WHERE tenant_id=$1 AND octet_length(content) > $2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(i64::try_from(MAX_PAYLOAD_BYTES).expect("ceiling"))
    .fetch_one(&world.db.pool)
    .await
    .expect("oversize payloads");
    assert_eq!(stored, 0, "nothing over 256 KiB was retained");
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_loop_whose_initial_carry_is_over_the_value_ceiling_fails_before_it_starts() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let big = "z".repeat(200 * 1024);
    let (workflow, _) = world
        .publish(looping(
            "bytes-loop-initial",
            "[input.big, input.big]",
            "bodyResult",
        ))
        .await;
    let run = world.start(workflow, &json!({"big": big})).await;
    let report = scheduler(&world, "bytes-loop-initial").await.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    assert_eq!(scopes_of_loops(&world, run).await, 0);
}

async fn scopes_of_loops(world: &World, run: RunId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM loop_scopes WHERE tenant_id=$1 AND run_id=$2")
        .bind(world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&world.db.pool)
        .await
        .expect("loop scope count")
}

#[tokio::test]
async fn a_loop_carry_over_the_value_ceiling_fails_the_loop_and_keeps_the_ceiling() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world
        .publish(looping(
            "bytes-loop-carry",
            "[0]",
            "[bodyResult, bodyResult]",
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "bytes-loop-carry").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    // 200 KiB is a legal output; the carry that repeats it twice is 400 KiB.
    assert_eq!(
        answer(&world, &workers, &session, &text_of(200 * 1024)).await,
        1
    );
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payloads WHERE tenant_id=$1 AND octet_length(content) > $2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(i64::try_from(MAX_PAYLOAD_BYTES).expect("ceiling"))
    .fetch_one(&world.db.pool)
    .await
    .expect("oversize payloads");
    assert_eq!(stored, 0, "the oversize carry was never retained");
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_scoped_task_context_built_from_the_largest_values_stays_under_eight_mib() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // The run input and the loop's carry are each about 250 KiB; the iteration task is built from
    // both. Only the iteration's own outputs join the context, so it stays far below the limit.
    let big = "q".repeat(250 * 1024);
    let (workflow, _) = world
        .publish(looping("bytes-context", "input.big", "bodyResult"))
        .await;
    world.start(workflow, &json!({"big": big})).await;
    let scheduler = scheduler(&world, "bytes-context").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    let tasks = workers
        .poll(&world.worker, session.0, &session.1, 1, None)
        .await
        .expect("the iteration is offered with its context");
    assert_eq!(tasks.len(), 1);
    assert_eq!(MAX_TASK_INPUT_BYTES, 8 * 1024 * 1024);
    assert!(
        tasks[0].input.len() > 2 * 250 * 1024,
        "the context carries the input and the carry: {} bytes",
        tasks[0].input.len()
    );
    assert!(
        tasks[0].input.len() < MAX_TASK_INPUT_BYTES,
        "{} bytes",
        tasks[0].input.len()
    );
}

#[tokio::test]
async fn the_carry_that_ends_a_loop_must_fit_beside_the_outputs_the_run_retains() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Sixteen 255,000-byte outputs retain 4,080,000 bytes, which fits 4 MiB. The final carry is
    // the last output again: another 255,000 bytes would make the loop's own output unretainable.
    let (workflow, _) = world
        .publish(looping_until(
            "bytes-loop-final",
            "repeatUntil",
            "\"start\"",
            "bodyResult",
            "iteration >= 15",
            20,
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "bytes-loop-final").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    let output = text_of(255_000);
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    for round in 0..40 {
        let answered = answer(&world, &workers, &session, &output).await;
        assert_eq!(
            answered,
            1,
            "round {round}: {:?} {:?}",
            world.run_status(run).await,
            world.journal(run).await.last()
        );
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        if is_over(&world, run).await {
            break;
        }
    }
    const { assert!(16 * 255_000 < MAX_RUN_OUTPUT_BYTES && 17 * 255_000 > MAX_RUN_OUTPUT_BYTES) };
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn an_aggregate_that_fits_alone_but_not_beside_the_retained_outputs_fails_the_scope() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Sixteen 262,000-byte outputs retain 4,192,000 bytes, 2,304 short of 4 MiB. The aggregate is
    // sixteen small lists (about 3 KB, far under the 256 KiB value ceiling) and does not fit in
    // what is left.
    let members = (1..=64)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let items: Vec<u32> = (0..16).collect();
    let run = start_collection(
        &world,
        collection(
            "bytes-aggregate-retained",
            "input.items",
            16,
            1,
            &format!("[{members}]"),
        ),
        &json!({"items": items}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-aggregate-retained").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    let output = text_of(262_000);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    for _ in 0..60 {
        scheduler.tick().await;
        answer(&world, &workers, &session, &output).await;
        scheduler.tick().await;
        if is_over(&world, run).await {
            break;
        }
    }
    const { assert!(16 * 262_000 < MAX_RUN_OUTPUT_BYTES && MAX_RUN_OUTPUT_BYTES - 16 * 262_000 < 2_500) };
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn an_aggregate_that_cannot_be_computed_fails_the_scope_with_expression_error() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Dividing by zero fails on every member, so the aggregate can never be built.
    let run = start_collection(
        &world,
        collection(
            "bytes-aggregate-error",
            "input.items",
            2,
            1,
            "bodyResult / 0",
        ),
        &json!({"items": [1, 2]}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-aggregate-error").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    for _ in 0..8 {
        scheduler.tick().await;
        answer(&world, &workers, &session, "1").await;
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        if is_over(&world, run).await {
            break;
        }
    }
    assert_failed_with(&world, run, "EXPRESSION_ERROR").await;
    world.assert_consistent(run).await;
}

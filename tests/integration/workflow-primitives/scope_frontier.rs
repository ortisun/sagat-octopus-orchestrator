//! G461 (D520-D524): the scheduler decides a collection or loop transition from a bounded frontier
//! instead of the whole restored run. These tests prove the frontier path is the same decision.
//!
//! * the frontier equals what a whole-run restore says about the scope, at every step of a run;
//! * driving a scope through the frontier path and through the whole-run path (the previous
//!   implementation, still the way scopes are entered and finished) writes the same journal, for the
//!   happy path, a failing member, a cancellation and a loop delay;
//! * the projections equal a replay of the journal afterwards (`verify_run`).

use std::time::Duration;

use regex::Regex;
use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{Engine, RunTx};
use sagat_application::runs::{Lock, RunOperation, RunStore};
use sagat_application::scope_frontier::ScopeFrontierStore;
use sagat_application::worker::{Capabilities, ClaimedTask, WorkerService, WorkerSettings};
use sagat_domain::execution::{ExecutionInstanceId, RunId};
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{
    CollectionAdvance, Keeper, KeeperSettings, LoopAdvance, Placement, Scheduler, SchedulerSettings,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{Session, World, task_ref, world};

const ACTIVITY: &str = "frontier.item";

type Workers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;
type Sched = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;

struct Rig {
    world: World,
    store: PgRunStore,
    workers: Workers,
    scheduler: Sched,
    session: Session,
}

async fn rig() -> Rig {
    let world = world().await;
    let store =
        PgRunStore::new(world.db.pool.clone(), world.cache.clone()).with_scoped_worker_protocol();
    let workers = WorkerService::new(
        store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_vault(world.keys.cipher());
    let placement = Placement::new(format!("frontier-{}", Uuid::now_v7()));
    Keeper::new(
        store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(3_600),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    let scheduler = Scheduler::new(
        store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_children(world.cache.clone())
    .with_placement(placement);
    let registered = workers
        .register(
            &world.worker,
            "payments",
            &format!("frontier-worker-{}", Uuid::now_v7()),
            &Capabilities {
                activities: vec![ACTIVITY.into()],
                languages: vec!["rust".into()],
                pools: Vec::new(),
                max_inflight: 64,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("register the protocol 1.5 worker");
    let session = Session {
        principal: world.worker.clone(),
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    };
    Rig {
        world,
        store,
        workers,
        scheduler,
        session,
    }
}

impl Rig {
    async fn capacity(&self, slots: i32) {
        sqlx::query(
            "INSERT INTO tenant_collection_policies \
             (tenant_id, max_active_iterations, updated_by) VALUES ($1, $2, 'frontier') \
             ON CONFLICT (tenant_id) DO UPDATE SET max_active_iterations = EXCLUDED.max_active_iterations",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(slots)
        .execute(&self.world.db.pool)
        .await
        .expect("tenant slots");
    }

    async fn tasks(&self) -> Vec<ClaimedTask> {
        let mut tasks = self
            .workers
            .poll(
                &self.session.principal,
                self.session.worker_id,
                &self.session.token,
                64,
                Some("0af7651916cd43dd8448eb211c80319c"),
            )
            .await
            .expect("poll");
        // The worker answers in a fixed order that is not admission order, so results arrive
        // out of order.
        tasks.sort_by_key(|task| std::cmp::Reverse(index_of(task)));
        tasks
    }

    async fn answer(&self, task: &ClaimedTask, fail_at: Option<u32>) {
        let index = index_of(task);
        if fail_at == Some(index) {
            self.workers
                .fail(
                    &self.session.principal,
                    &task_ref(task),
                    "PAYMENT_DECLINED",
                    false,
                    &format!("fail-{}", task.task_id()),
                )
                .await
                .expect("fail the member");
        } else {
            self.workers
                .complete(
                    &self.session.principal,
                    &task_ref(task),
                    &serde_json::to_vec(&json!({ "v": index })).expect("output"),
                    &format!("done-{}", task.task_id()),
                )
                .await
                .expect("complete the member");
        }
    }

    /// The journal of a run with what differs between two runs of the same workflow removed: the
    /// identifiers and the instants.
    async fn journal(&self, run: RunId) -> Vec<String> {
        let uuid = Regex::new("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
            .expect("uuid pattern");
        let stamp = Regex::new(r"\d{4}-\d\d-\d\dT[0-9:.]+Z?|\b1[67]\d{14}\b").expect("stamp");
        let rows: Vec<(i64, String, Option<String>, String, String, String)> = sqlx::query_as(
            "SELECT seq, event_type, node, coalesce(instance_path_nodes::text,''), \
                    coalesce(instance_path_indices::text,''), detail::text \
             FROM run_events WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .fetch_all(&self.world.db.pool)
        .await
        .expect("journal");
        rows.into_iter()
            .map(|(seq, kind, node, nodes, indices, detail)| {
                let detail = uuid.replace_all(&detail, "<id>");
                let detail = stamp.replace_all(&detail, "<time>");
                format!(
                    "{seq} {kind} {} {nodes}{indices} {detail}",
                    node.unwrap_or_default()
                )
            })
            .collect()
    }
}

fn index_of(task: &ClaimedTask) -> u32 {
    task.instance.path().segments()[0].iteration_index()
}

fn collection_document(name: &str, items: usize, concurrency: usize) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "items",
        "bodies": {
            "line": {
                "entrypoint": "work",
                "nodes": [
                    { "id": "work", "type": "activity", "activity": ACTIVITY, "next": "returned" },
                    { "id": "returned", "type": "return", "result": "steps.work.output" }
                ]
            }
        },
        "nodes": [
            {
                "id": "items", "type": "forEach", "items": "input.items", "body": "line",
                "result": "bodyResult", "maxItems": items, "maxConcurrency": concurrency,
                "next": "done"
            },
            { "id": "done", "type": "end" }
        ]
    })
}

fn loop_document(name: &str, iterations: usize, delay_ms: Option<u64>) -> Value {
    let mut node = json!({
        "id": "poll", "type": "while", "condition": format!("carry < {iterations}"),
        "body": "iteration", "initial": "0", "carry": "bodyResult",
        "maxIterations": iterations, "timeoutMs": 86_400_000, "next": "done"
    });
    if let Some(delay) = delay_ms {
        node["delayMs"] = json!(delay);
    }
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "poll",
        "bodies": {
            "iteration": {
                "entrypoint": "step",
                "nodes": [
                    { "id": "step", "type": "activity", "activity": ACTIVITY, "next": "returned" },
                    { "id": "returned", "type": "return", "result": "steps.step.output" }
                ]
            }
        },
        "nodes": [node, { "id": "done", "type": "end" }]
    })
}

#[derive(Clone, Copy, Debug)]
enum Path {
    /// The production transition: the scope's frontier.
    Frontier,
    /// The previous implementation: the whole restored run.
    WholeRun,
}

#[derive(Clone, Copy, Debug, Default)]
struct Script {
    /// The member whose activity fails (not retryable).
    fail_at: Option<u32>,
    /// Cancel the run after this many admissions or checkpoints.
    cancel_after: Option<usize>,
}

fn owner(node: &str) -> ExecutionInstanceId {
    ExecutionInstanceId::root(NodeId::parse(node).expect("owner node"))
}

/// Drives a collection to its end and returns every transition it took.
async fn drive_collection(rig: &Rig, path: Path, run: RunId, script: Script) -> Vec<String> {
    let tenant = rig.world.admin.tenant_id();
    let owner = owner("items");
    let mut taken = Vec::new();
    let mut cancelled = false;
    let mut idle = 0;
    loop {
        let outcome = match path {
            Path::Frontier => rig.scheduler.advance_collection(tenant, run, &owner).await,
            Path::WholeRun => {
                rig.scheduler
                    .advance_collection_from_run(tenant, run, &owner)
                    .await
            }
        }
        .expect("transition");
        taken.push(format!("{outcome:?}"));
        match outcome {
            CollectionAdvance::Finished | CollectionAdvance::AlreadyFinished => break,
            CollectionAdvance::Waiting => {
                let tasks = rig.tasks().await;
                idle = if tasks.is_empty() { idle + 1 } else { 0 };
                assert!(idle < 100, "stalled after {taken:?}");
                for task in &tasks {
                    rig.answer(task, script.fail_at).await;
                }
                if tasks.is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            _ => {}
        }
        if script.cancel_after == Some(taken.len()) && !cancelled {
            cancelled = true;
            rig.world.operate_run(run, RunOperation::Cancel).await;
        }
        assert!(taken.len() < 400, "no end after {taken:?}");
    }
    taken
}

async fn waiting_timer(rig: &Rig, run: RunId) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT timer_id FROM loop_timers WHERE tenant_id = $1 AND run_id = $2 \
           AND lifecycle = 'WAITING'",
    )
    .bind(rig.world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_optional(&rig.world.db.pool)
    .await
    .expect("waiting loop timer")
}

/// Drives a loop to its end and returns every transition it took.
async fn drive_loop(rig: &Rig, path: Path, run: RunId, script: Script) -> Vec<String> {
    let tenant = rig.world.admin.tenant_id();
    let owner = owner("poll");
    let mut taken = Vec::new();
    let mut cancelled = false;
    let mut idle = 0;
    loop {
        let timer = waiting_timer(rig, run).await;
        let outcome = match path {
            Path::Frontier => rig.scheduler.advance_loop(tenant, run, &owner, timer).await,
            Path::WholeRun => {
                rig.scheduler
                    .advance_loop_from_run(tenant, run, &owner, timer)
                    .await
            }
        }
        .expect("transition");
        if matches!(outcome, LoopAdvance::Waiting) && timer.is_some() {
            // The delay has not elapsed; wait for it without recording the polls.
            tokio::time::sleep(Duration::from_millis(10)).await;
            continue;
        }
        taken.push(format!("{outcome:?}"));
        match outcome {
            LoopAdvance::AlreadyFinished => break,
            LoopAdvance::Waiting => {
                let tasks = rig.tasks().await;
                idle = if tasks.is_empty() { idle + 1 } else { 0 };
                assert!(idle < 100, "stalled after {taken:?}");
                for task in &tasks {
                    rig.answer(task, script.fail_at).await;
                }
                if tasks.is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            _ => {}
        }
        if script.cancel_after == Some(taken.len()) && !cancelled {
            cancelled = true;
            rig.world.operate_run(run, RunOperation::Cancel).await;
        }
        assert!(taken.len() < 400, "no end after {taken:?}");
        if rig.world.run_status(run).await.is_terminal() {
            break;
        }
    }
    taken
}

async fn collection_run(rig: &Rig, name: &str, items: usize, concurrency: usize) -> RunId {
    rig.capacity(16).await;
    let (workflow, _) = rig
        .world
        .publish(collection_document(name, items, concurrency))
        .await;
    rig.world
        .start(
            workflow,
            &json!({ "items": (0..items).collect::<Vec<_>>() }),
        )
        .await
}

async fn loop_run(rig: &Rig, name: &str, iterations: usize, delay_ms: Option<u64>) -> RunId {
    rig.capacity(16).await;
    let (workflow, _) = rig
        .world
        .publish(loop_document(name, iterations, delay_ms))
        .await;
    rig.world.start(workflow, &json!({})).await
}

/// The same collection through both paths: the same transitions, the same journal.
async fn compare_collection(label: &str, items: usize, concurrency: usize, script: Script) {
    let rig = rig().await;
    // One run at a time: they share the worker, whose poll would hand out the other run's tasks.
    let frontier = collection_run(&rig, &format!("{label}-frontier"), items, concurrency).await;
    let frontier_taken = drive_collection(&rig, Path::Frontier, frontier, script).await;
    let whole = collection_run(&rig, &format!("{label}-whole"), items, concurrency).await;
    let whole_taken = drive_collection(&rig, Path::WholeRun, whole, script).await;
    assert_eq!(frontier_taken, whole_taken, "{label}: transitions differ");
    assert_eq!(
        rig.journal(frontier).await,
        rig.journal(whole).await,
        "{label}: journals differ"
    );
    assert_eq!(
        rig.world.run_status(frontier).await,
        rig.world.run_status(whole).await
    );
    rig.world.assert_consistent(frontier).await;
    rig.world.assert_consistent(whole).await;
}

async fn compare_loop(label: &str, iterations: usize, delay_ms: Option<u64>, script: Script) {
    let rig = rig().await;
    let frontier = loop_run(&rig, &format!("{label}-frontier"), iterations, delay_ms).await;
    let frontier_taken = drive_loop(&rig, Path::Frontier, frontier, script).await;
    let whole = loop_run(&rig, &format!("{label}-whole"), iterations, delay_ms).await;
    let whole_taken = drive_loop(&rig, Path::WholeRun, whole, script).await;
    assert_eq!(frontier_taken, whole_taken, "{label}: transitions differ");
    assert_eq!(
        rig.journal(frontier).await,
        rig.journal(whole).await,
        "{label}: journals differ"
    );
    assert_eq!(
        rig.world.run_status(frontier).await,
        rig.world.run_status(whole).await
    );
    rig.world.assert_consistent(frontier).await;
    rig.world.assert_consistent(whole).await;
}

#[tokio::test]
async fn a_collection_writes_the_same_journal_through_the_frontier_and_the_whole_run() {
    compare_collection("coll-ok", 8, 3, Script::default()).await;
}

#[tokio::test]
async fn a_failing_member_drains_the_same_way_through_both_paths() {
    compare_collection(
        "coll-fail",
        6,
        3,
        Script {
            fail_at: Some(2),
            ..Script::default()
        },
    )
    .await;
}

#[tokio::test]
async fn a_cancelled_collection_drains_the_same_way_through_both_paths() {
    compare_collection(
        "coll-cancel",
        6,
        3,
        Script {
            cancel_after: Some(3),
            ..Script::default()
        },
    )
    .await;
}

#[tokio::test]
async fn a_loop_writes_the_same_journal_through_the_frontier_and_the_whole_run() {
    compare_loop("loop-ok", 4, None, Script::default()).await;
}

#[tokio::test]
async fn a_loop_with_a_delay_writes_the_same_journal_through_both_paths() {
    compare_loop("loop-delay", 3, Some(40), Script::default()).await;
}

#[tokio::test]
async fn a_failing_loop_iteration_drains_the_same_way_through_both_paths() {
    compare_loop(
        "loop-fail",
        4,
        None,
        Script {
            fail_at: Some(1),
            ..Script::default()
        },
    )
    .await;
}

#[tokio::test]
async fn a_cancelled_loop_drains_the_same_way_through_both_paths() {
    compare_loop(
        "loop-cancel",
        4,
        None,
        Script {
            cancel_after: Some(2),
            ..Script::default()
        },
    )
    .await;
}

/// The frontier says what a whole-run restore says about the scope, at every step of a run, and its
/// instance charge is the one `RunState` counts (D522).
#[tokio::test]
async fn the_frontier_equals_the_restored_scope_at_every_step() {
    let rig = rig().await;
    let tenant = rig.world.admin.tenant_id();
    let run = collection_run(&rig, "frontier-equals", 6, 3).await;
    let owner = owner("items");
    let mut checked = 0;
    loop {
        let outcome = rig
            .scheduler
            .advance_collection(tenant, run, &owner)
            .await
            .expect("transition");
        if let Some(frontier) = rig
            .store
            .collection_frontier(tenant, run, &owner)
            .await
            .expect("frontier")
        {
            let mut tx = rig
                .store
                .begin(tenant, run, Lock::Wait)
                .await
                .expect("restore")
                .expect("run");
            let (_, state) = tx.parts();
            let collection = state.collection(&owner).expect("restored collection");
            assert_eq!(frontier.run.version, state.version());
            assert_eq!(frontier.run.stop_reason, state.stop_reason());
            assert_eq!(frontier.phase, collection.phase());
            assert_eq!(frontier.item_count, collection.item_count());
            assert_eq!(frontier.next_admission, collection.next_admission());
            assert_eq!(
                usize::try_from(frontier.completed).expect("count"),
                collection.ordered_results().count()
            );
            assert_eq!(frontier.max_concurrency, collection.max_concurrency());
            assert_eq!(
                frontier.run.charged_instances,
                state.charged_instances(),
                "the store's counter is the number RunState counts"
            );
            let restored: Vec<u32> = collection.active_iterations().collect();
            let read: Vec<u32> = frontier.active.iter().map(|member| member.index).collect();
            assert_eq!(read, restored);
            for member in &frontier.active {
                assert_eq!(collection.entry_instance(member.index), Some(&member.entry));
                let expected: Vec<_> = state
                    .instances()
                    .filter(|(instance, _)| instance.path() == member.entry.path())
                    .map(|(instance, step)| (instance.clone(), step.status(), step.attempt()))
                    .collect();
                let found: Vec<_> = member
                    .steps
                    .iter()
                    .map(|(instance, step)| (instance.clone(), step.status(), step.attempt()))
                    .collect();
                assert_eq!(found, expected, "steps of member {}", member.index);
            }
            checked += 1;
        }
        match outcome {
            CollectionAdvance::Finished | CollectionAdvance::AlreadyFinished => break,
            CollectionAdvance::Waiting => {
                for task in rig.tasks().await {
                    rig.answer(&task, None).await;
                }
            }
            _ => {}
        }
    }
    assert!(checked >= 12, "only {checked} states were compared");
}

/// The same for a loop, across its delay and its end.
#[tokio::test]
async fn the_loop_frontier_equals_the_restored_scope_at_every_step() {
    let rig = rig().await;
    let tenant = rig.world.admin.tenant_id();
    let run = loop_run(&rig, "loop-frontier-equals", 4, Some(30)).await;
    let owner = owner("poll");
    let mut checked = 0;
    loop {
        let timer = waiting_timer(&rig, run).await;
        let outcome = rig
            .scheduler
            .advance_loop(tenant, run, &owner, timer)
            .await
            .expect("transition");
        if let Some(frontier) = rig
            .store
            .loop_frontier(tenant, run, &owner)
            .await
            .expect("frontier")
        {
            let mut tx = rig
                .store
                .begin(tenant, run, Lock::Wait)
                .await
                .expect("restore")
                .expect("run");
            let (_, state) = tx.parts();
            let restored = state.loop_state(&owner).expect("restored loop");
            assert_eq!(frontier.run.version, state.version());
            assert_eq!(frontier.phase, restored.phase());
            assert_eq!(frontier.deadline, restored.deadline());
            assert_eq!(frontier.next_iteration, restored.next_iteration());
            assert_eq!(frontier.delay_until, restored.delay_until());
            assert_eq!(
                frontier.member.as_ref().map(|member| member.index),
                restored.active_iteration()
            );
            assert_eq!(
                frontier.run.charged_instances,
                state.charged_instances(),
                "the store's counter is the number RunState counts"
            );
            if let Some(member) = &frontier.member {
                let expected: Vec<_> = state
                    .instances()
                    .filter(|(instance, _)| instance.path() == member.entry.path())
                    .map(|(instance, step)| (instance.clone(), step.status(), step.attempt()))
                    .collect();
                let found: Vec<_> = member
                    .steps
                    .iter()
                    .map(|(instance, step)| (instance.clone(), step.status(), step.attempt()))
                    .collect();
                assert_eq!(found, expected);
            }
            checked += 1;
        }
        match outcome {
            LoopAdvance::AlreadyFinished => break,
            LoopAdvance::Waiting if timer.is_none() => {
                for task in rig.tasks().await {
                    rig.answer(&task, None).await;
                }
            }
            LoopAdvance::Waiting => tokio::time::sleep(Duration::from_millis(10)).await,
            _ => {}
        }
        if rig.world.run_status(run).await.is_terminal() {
            break;
        }
    }
    assert!(checked >= 4, "only {checked} states were compared");
}

//! Collection operators applied at scope entry (design 042, slice S1; ADR-0087), against real PostgreSQL, the real
//! codec and evaluator, the scheduler, and a protocol 1.5 worker.
//!
//! The chain is applied once by one evaluator call, its output is the membership the entry records, every ceiling is
//! checked before `enter_collection`, a failing chain fails the run with its typed code, and once the entry has
//! committed nothing evaluates the chain again (T042-8, T042-45, T042-46, T042-48, T042-49, T042-51).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Method;
use sagat_application::blobs::{BlobAccess, BlobKey};
use sagat_application::collection_chain::{ChainAnswer, ChainCeilings, ChainFailure, ChainInput};
use sagat_application::collection_source::CollectionItemSource;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{
    DefinitionCache, Engine, ExpressionEvaluator, ExpressionFailure, ReconcileAnswer, RunData,
};
use sagat_application::loop_progress::{LoopCheckpointEvaluation, LoopEntryEvaluation};
use sagat_domain::blobs::BlobId;
use sagat_domain::execution::{CorrelationKey, ExecutionInstanceId, RunId};
use sagat_domain::operators::OperatorChain;
use sagat_domain::workflow::{Expression, NodeId};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use sagat_telemetry::Metrics;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::support::blob_collections::{
    MIB, blob_source, namespace_of, platform_with_operators, scoped_worker, scopes, start_inline,
    start_with_blob, status_and_code, upload,
};
use crate::support::worker::{HUMAN, NAMESPACE, Platform, TENANT, Worker};

// ---------------------------------------------------------------------------------------------------------------------
// Fixtures shared with the race and old-reader tests
// ---------------------------------------------------------------------------------------------------------------------

/// A `forEach` over `items` shaped by `operators`; its body returns what the worker answers, its result is
/// `bodyResult`, and the run's output is the collection's.
pub(crate) fn chained_ir(name: &str, items: &str, operators: &Value, max_items: u32) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "items",
        "bodies": {"line": {"entrypoint": "line", "nodes": [
            {"id": "line", "type": "activity", "activity": "src.line", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.line.output"}
        ]}},
        "nodes": [
            {"id": "items", "type": "forEach", "items": items, "operators": operators, "body": "line",
             "result": "bodyResult", "maxItems": max_items, "maxConcurrency": 4, "next": "done"},
            {"id": "done", "type": "end", "outputExpr": "steps.items.output"}
        ]
    })
}

pub(crate) fn owner() -> ExecutionInstanceId {
    ExecutionInstanceId::root(NodeId::parse("items").expect("node"))
}

/// How an evaluator under test treats the chain call; every other call is the production evaluator's.
#[derive(Clone)]
pub(crate) enum Mode {
    /// Applies the chain.
    Apply,
    /// Panics: after entry nothing may call it (FR-042-009).
    Panic,
    /// Says it entered, then waits to be released before applying the chain (a barrier inside the evaluator).
    Hold(Arc<Gate>),
}

/// The barrier of [`Mode::Hold`]: `entered` is told from the blocking task, `release` is waited for there.
pub(crate) struct Gate {
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

/// The test side of a [`Gate`].
pub(crate) struct Barrier {
    entered: tokio::sync::mpsc::UnboundedReceiver<()>,
    release: std::sync::mpsc::Sender<()>,
}

impl Barrier {
    /// Waits until the chain is held inside the evaluator.
    /// Bounded, so that a chain that never reaches the evaluator fails the test instead of hanging it; the bound
    /// orders nothing.
    pub(crate) async fn held(&mut self) {
        tokio::time::timeout(Duration::from_secs(60), self.entered.recv())
            .await
            .expect("the evaluator never entered the chain")
            .expect("the evaluator entered the chain");
    }

    pub(crate) fn release(&self) {
        self.release.send(()).expect("release the chain");
    }
}

pub(crate) fn gate() -> (Arc<Gate>, Barrier) {
    let (entered_tx, entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    (
        Arc::new(Gate {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        }),
        Barrier {
            entered: entered_rx,
            release: release_tx,
        },
    )
}

/// The production evaluator, with its chain calls counted and shaped by a [`Mode`].
#[derive(Clone)]
pub(crate) struct Watched {
    pub(crate) calls: Arc<AtomicUsize>,
    mode: Mode,
}

impl Watched {
    pub(crate) fn new(mode: Mode) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            mode,
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

macro_rules! delegate {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $result:ty;)+) => {
        $(fn $name(&self, $($arg: $ty),*) -> $result {
            JsonEvaluator.$name($($arg),*)
        })+
    };
}

impl ExpressionEvaluator for Watched {
    delegate! {
        fn condition(expression: &Expression, data: &RunData) -> Result<bool, ExpressionFailure>;
        fn condition_instance(expression: &Expression, data: &RunData, instance: &ExecutionInstanceId) -> Result<bool, ExpressionFailure>;
        fn correlation_key(expression: &Expression, data: &RunData) -> Result<CorrelationKey, ExpressionFailure>;
        fn correlation_key_instance(expression: &Expression, data: &RunData, instance: &ExecutionInstanceId) -> Result<CorrelationKey, ExpressionFailure>;
        fn idempotency_key(template: &Expression, data: &RunData, node: &NodeId) -> Result<String, ExpressionFailure>;
        fn idempotency_key_instance(template: &Expression, data: &RunData, instance: &ExecutionInstanceId) -> Result<String, ExpressionFailure>;
        fn child_input(expression: Option<&Expression>, data: &RunData) -> Result<Vec<u8>, ExpressionFailure>;
        fn run_result(expression: &Expression, data: &RunData) -> Result<Vec<u8>, ExpressionFailure>;
        fn continuation_value(expression: &Expression, data: &RunData) -> Result<Vec<u8>, ExpressionFailure>;
        fn database_parameter(parameter: &sagat_domain::database::Parameter, data: &RunData) -> Result<sagat_domain::database::TypedValue, ExpressionFailure>;
        fn task_input(data: &RunData, node: &NodeId) -> Vec<u8>;
        fn task_input_instance(data: &RunData, instance: &ExecutionInstanceId) -> Result<Vec<u8>, ExpressionFailure>;
        fn collection_items(expression: &Expression, data: &RunData) -> Result<Vec<Vec<u8>>, ExpressionFailure>;
        fn collection_source(expression: &Expression, data: &RunData) -> Result<sagat_application::collection_source::CollectionSource, ExpressionFailure>;
        fn collection_return(expression: &Expression, data: &RunData, instance: &ExecutionInstanceId) -> Result<Vec<u8>, ExpressionFailure>;
        fn collection_member_result(expression: &Expression, data: &RunData, instance: &ExecutionInstanceId, body_result: &[u8]) -> Result<Vec<u8>, ExpressionFailure>;
        fn loop_entry(initial: &Expression, condition: Option<&Expression>, data: &RunData) -> Result<LoopEntryEvaluation, ExpressionFailure>;
        fn loop_checkpoint(result: &Expression, carry: &Expression, condition: &Expression, data: &RunData, return_instance: &ExecutionInstanceId) -> Result<LoopCheckpointEvaluation, ExpressionFailure>;
        fn reconcile_answer(output: &[u8]) -> Result<ReconcileAnswer, ExpressionFailure>;
        fn event_document(event: &sagat_application::dispatch::EventView<'_>) -> Vec<u8>;
        fn event_filter(filter: &Expression, document: &[u8]) -> Result<bool, ExpressionFailure>;
        fn is_json(bytes: &[u8]) -> bool;
        fn is_json_object(bytes: &[u8]) -> bool;
    }

    fn collection_chain(
        &self,
        input: ChainInput<'_>,
        chain: &OperatorChain,
        data: &RunData,
        ceilings: &ChainCeilings,
    ) -> Result<ChainAnswer, ChainFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.mode {
            Mode::Apply => {}
            Mode::Panic => panic!("the chain was evaluated again after its entry committed"),
            Mode::Hold(gate) => {
                gate.entered
                    .send(())
                    .expect("the test waits for the barrier");
                gate.release
                    .lock()
                    .expect("barrier")
                    .recv()
                    .expect("the test releases the barrier");
            }
        }
        JsonEvaluator.collection_chain(input, chain, data, ceilings)
    }
}

pub(crate) type WatchedScheduler = Scheduler<PgRunStore, Watched, SystemEntropy>;

/// A scheduler replica over `evaluator`, with the keeper that holds its shard leases. A second replica takes over
/// only once the first one's keeper has left.
pub(crate) async fn replica(
    p: &Platform,
    evaluator: Watched,
    source: Option<Arc<dyn CollectionItemSource>>,
) -> (WatchedScheduler, Keeper<PgRunStore>) {
    let placement = Placement::new(format!("operators-{}", Uuid::now_v7()));
    let keeper = Keeper::new(
        p.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(600),
        },
    );
    keeper.round().await.expect("acquire shards");
    let scheduler = Scheduler::new(
        p.api.executions.clone(),
        Engine::new(evaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_children(DefinitionCache::new(Arc::new(JsonIrCodec), 64))
    .with_placement(placement);
    let scheduler = match source {
        Some(source) => scheduler.with_item_source(source),
        None => scheduler,
    };
    (scheduler, keeper)
}

/// Ticks `scheduler` and completes every iteration with its own item until the run ends. Returns what each index saw.
pub(crate) async fn drive(
    p: &Platform,
    scheduler: &WatchedScheduler,
    worker: &mut Worker<'_>,
    run: Uuid,
) -> Vec<(u32, Value)> {
    let mut seen = Vec::new();
    for _ in 0..200 {
        scheduler.tick().await;
        for task in worker.poll(8).await {
            let input: Value = serde_json::from_str(&task.input_json).expect("task input");
            let index = task
                .instance
                .as_ref()
                .and_then(|instance| instance.instance_path.first())
                .map(|segment| segment.iteration_index)
                .expect("a scoped task");
            seen.push((index, input["item"].clone()));
            worker
                .complete(&task, &input["item"])
                .await
                .expect("complete the iteration");
        }
        let (status, _) = status_and_code(p, run).await;
        if matches!(status.as_str(), "SUCCEEDED" | "FAILED" | "CANCELLED") {
            break;
        }
    }
    seen.sort_by_key(|(index, _)| *index);
    seen
}

/// Ticks until the run leaves the running states, at most `limit` times.
pub(crate) async fn settle(p: &Platform, scheduler: &WatchedScheduler, run: Uuid, limit: usize) {
    for _ in 0..limit {
        scheduler.tick().await;
        if matches!(
            status_and_code(p, run).await.0.as_str(),
            "SUCCEEDED" | "FAILED" | "CANCELLED"
        ) {
            return;
        }
    }
}

/// The scope's item count and its snapshot payload, as stored.
pub(crate) async fn scope_of(p: &Platform, run: Uuid) -> (i32, Vec<u8>) {
    let row = sqlx::query(
        "SELECT scope.item_count, payload.content FROM iteration_scopes scope JOIN payloads payload \
           ON payload.tenant_id = scope.tenant_id AND payload.digest = scope.snapshot_digest \
         WHERE scope.run_id = $1 AND scope.owner_node = 'items'",
    )
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the scope");
    (row.get("item_count"), row.get("content"))
}

/// The items the scope stores, in order.
pub(crate) async fn stored_items(p: &Platform, run: Uuid) -> Vec<Value> {
    let items: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT payload.content FROM collection_scope_items item JOIN payloads payload \
           ON payload.tenant_id = item.tenant_id AND payload.digest = item.item_digest \
         JOIN iteration_scopes scope ON scope.tenant_id = item.tenant_id \
          AND scope.run_id = item.run_id AND scope.scope_id = item.scope_id \
         WHERE scope.run_id = $1 ORDER BY item.iteration_index",
    )
    .bind(run)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("stored items");
    items
        .iter()
        .map(|item| serde_json::from_slice(item).expect("json"))
        .collect()
}

pub(crate) async fn run_output(p: &Platform, run: Uuid) -> Value {
    crate::support::blob_collections::aggregate(p, run).await
}

pub(crate) async fn verified(p: &Platform, run: Uuid) {
    p.api
        .executions
        .verify_run(p.tenant, RunId::new(run))
        .await
        .expect("journal and projection agree");
}

pub(crate) async fn cancel(p: &Platform, run: Uuid) {
    let response = p
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/operations"),
            HUMAN,
            TENANT,
        )
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", format!("cancel-{}", Uuid::now_v7()))
        .json(&json!({"action": "CANCEL", "reason": "operator chain race"}))
        .send()
        .await
        .expect("cancel");
    let status = response.status();
    let body = crate::support::api::json_of(response).await;
    assert!(status.is_success(), "{status} {body}");
}

fn chain_reentries(page: &str, reason: &str) -> f64 {
    metric(
        page,
        "collection_chain_reentries_total",
        &[("reason", reason)],
    )
}

/// The sum of every series of `name` whose labels include all of `labels`.
pub(crate) fn metric(page: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    page.lines()
        .filter(|line| {
            line.starts_with(name)
                && matches!(line[name.len()..].chars().next(), Some('{' | ' '))
                && labels
                    .iter()
                    .all(|(key, value)| line.contains(&format!("{key}=\"{value}\"")))
        })
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

// ---------------------------------------------------------------------------------------------------------------------
// T042-8: the chain shapes the membership, and fails the run with its code
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_chain_shapes_the_membership_and_the_body_index_has_no_gap() {
    let p = platform_with_operators().await;
    let workflow = p
        .publish(
            "operators-shape",
            &chained_ir(
                "operators-shape",
                "input.lines",
                &json!([
                    {"filter": {"where": "element > 0"}},
                    {"skip": {"count": 1}},
                    {"map": {"select": "[element, position, input.tag]"}}
                ]),
                10,
            ),
        )
        .await;
    let run = start_inline(
        &p,
        &workflow,
        &json!({"lines": [5, -1, 7, -2, 9, 11], "tag": "t"}),
    )
    .await;
    let evaluator = Watched::new(Mode::Apply);
    let (scheduler, _keeper) = replica(&p, evaluator.clone(), None).await;
    let mut worker = scoped_worker(&p, "operators-shape-worker", &["src.line"]).await;

    let seen = drive(&p, &scheduler, &mut worker, run).await;

    assert_eq!(
        seen,
        vec![
            (0, json!([7, 0, "t"])),
            (1, json!([9, 1, "t"])),
            (2, json!([11, 2, "t"]))
        ],
        "the body's index is the position in the chain's output: 0 to n-1 after a filter and a skip"
    );
    assert_eq!(
        status_and_code(&p, run).await,
        ("SUCCEEDED".to_owned(), None)
    );
    assert_eq!(
        run_output(&p, run).await,
        json!([[7, 0, "t"], [9, 1, "t"], [11, 2, "t"]])
    );
    let (count, snapshot) = scope_of(&p, run).await;
    assert_eq!(count, 3);
    assert_eq!(
        snapshot,
        br#"[[7,0,"t"],[9,1,"t"],[11,2,"t"]]"#.to_vec(),
        "an inline source's output is today's snapshot array"
    );
    assert_eq!(
        evaluator.calls(),
        1,
        "one evaluator call for the whole chain"
    );
    verified(&p, run).await;
}

#[tokio::test]
async fn an_empty_output_succeeds_with_an_empty_result() {
    let p = platform_with_operators().await;
    let workflow = p
        .publish(
            "operators-empty",
            &chained_ir(
                "operators-empty",
                "input.lines",
                &json!([{"filter": {"where": "element > 100"}}]),
                10,
            ),
        )
        .await;
    let run = start_inline(&p, &workflow, &json!({"lines": [1, 2, 3]})).await;
    let (scheduler, _keeper) = replica(&p, Watched::new(Mode::Apply), None).await;
    settle(&p, &scheduler, run, 20).await;
    assert_eq!(
        status_and_code(&p, run).await,
        ("SUCCEEDED".to_owned(), None)
    );
    assert_eq!(run_output(&p, run).await, json!([]));
    assert_eq!(scope_of(&p, run).await, (0, b"[]".to_vec()));
    verified(&p, run).await;
}

/// Starts a run of a chained collection over `input`, ticks it until it settles and returns its status and code.
async fn outcome_of(
    p: &Platform,
    name: &str,
    operators: &Value,
    max_items: u32,
    input: &Value,
    ceilings: Option<ChainCeilings>,
) -> (Uuid, (String, Option<String>)) {
    let workflow = p
        .publish(name, &chained_ir(name, "input.lines", operators, max_items))
        .await;
    let run = start_inline(p, &workflow, input).await;
    let (scheduler, keeper) = replica(p, Watched::new(Mode::Apply), None).await;
    let scheduler = match ceilings {
        Some(ceilings) => scheduler.with_chain_ceilings(ceilings),
        None => scheduler,
    };
    settle(p, &scheduler, run, 20).await;
    // The next replica of this platform takes the shards over.
    keeper.leave().await;
    (run, status_and_code(p, run).await)
}

fn failed(code: &str) -> (String, Option<String>) {
    ("FAILED".to_owned(), Some(code.to_owned()))
}

#[tokio::test]
async fn the_chain_input_holds_1000_elements_and_fails_the_run_at_1001() {
    let p = platform_with_operators().await;
    let operators = json!([{"map": {"select": "element"}}, {"take": {"count": 2}}]);
    let at: Vec<u32> = (0..1000).collect();
    let (run, outcome) = outcome_of(
        &p,
        "operators-input-at",
        &operators,
        10,
        &json!({"lines": at}),
        None,
    )
    .await;
    assert_eq!(
        (scopes(&p, run).await, scope_of(&p, run).await.0),
        (1, 2),
        "at the ceiling the entry reaches the store and is recorded ({outcome:?})"
    );
    let over: Vec<u32> = (0..1001).collect();
    let (run, outcome) = outcome_of(
        &p,
        "operators-input-over",
        &operators,
        10,
        &json!({"lines": over}),
        None,
    )
    .await;
    assert_eq!(outcome, failed("COLLECTION_OPERATOR_BUDGET"));
    assert_eq!(scopes(&p, run).await, 0);
    verified(&p, run).await;
}

#[tokio::test]
async fn a_stage_holds_1000_elements_and_fails_the_run_at_1001() {
    let p = platform_with_operators().await;
    let operators = json!([{"flatMap": {"select": "[element, element]"}}, {"take": {"count": 2}}]);
    let at: Vec<u32> = (0..500).collect();
    let (run, _) = outcome_of(
        &p,
        "operators-stage-at",
        &operators,
        10,
        &json!({"lines": at}),
        None,
    )
    .await;
    assert_eq!(scope_of(&p, run).await.0, 2);
    let over: Vec<u32> = (0..501).collect();
    let (run, outcome) = outcome_of(
        &p,
        "operators-stage-over",
        &operators,
        10,
        &json!({"lines": over}),
        None,
    )
    .await;
    assert_eq!(outcome, failed("COLLECTION_OPERATOR_BUDGET"));
    assert_eq!(scopes(&p, run).await, 0);
}

#[tokio::test]
async fn the_wall_time_fails_the_run_with_the_budget_code() {
    let p = platform_with_operators().await;
    let (run, outcome) = outcome_of(
        &p,
        "operators-wall-time",
        &json!([{"map": {"select": "element"}}]),
        10,
        &json!({"lines": [1, 2]}),
        Some(ChainCeilings {
            wall_time: Duration::ZERO,
            ..ChainCeilings::COLLECTION
        }),
    )
    .await;
    assert_eq!(outcome, failed("COLLECTION_OPERATOR_BUDGET"));
    assert_eq!(scopes(&p, run).await, 0);
}

#[tokio::test]
async fn an_expression_error_fails_the_run_and_on_error_is_not_taken() {
    let p = platform_with_operators().await;
    let mut ir = chained_ir(
        "operators-expression",
        "input.lines",
        &json!([{"map": {"select": "element"}}, {"filter": {"where": "element.missing"}}]),
        10,
    );
    ir["nodes"][0]["onError"] = json!("recover");
    ir["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(json!({"id": "recover", "type": "end"}));
    let workflow = p.publish("operators-expression", &ir).await;
    let run = start_inline(&p, &workflow, &json!({"lines": [{"a": 1}]})).await;
    let (scheduler, keeper) = replica(&p, Watched::new(Mode::Apply), None).await;
    let (guard, captured) = crate::support::logs::capture();
    settle(&p, &scheduler, run, 20).await;
    drop(guard);
    keeper.leave().await;
    assert_eq!(status_and_code(&p, run).await, failed("EXPRESSION_ERROR"));
    // The run's failure detail says which stage, which operator and which position, never a value (review of S1).
    let detail: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE tenant_id = $1 AND resource_id = $2 \
           AND action = 'run.collection_chain_failed'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.to_string())
    .fetch_all(&p.api.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        detail,
        ["operators[1] (filter) failed at position 0 (expression)"],
        "one value-free record of where the chain failed"
    );
    let log = captured.text();
    let warning = log
        .lines()
        .find(|line| {
            line.contains("a collection source was refused") && line.contains(&run.to_string())
        })
        .unwrap_or_else(|| panic!("the warning of the refusal: {log}"));
    for field in [
        "stage=1",
        "operator=\"filter\"",
        "position=0",
        "code=\"EXPRESSION_ERROR\"",
    ] {
        assert!(warning.contains(field), "{field} in {warning}");
    }
    let recovered: i64 =
        sqlx::query_scalar("SELECT count(*) FROM steps WHERE run_id = $1 AND node = 'recover'")
            .bind(run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("steps");
    assert_eq!(
        recovered, 0,
        "a chain that fails at entry does not take onError (G042-008)"
    );
    for (name, operators, input) in [
        (
            "operators-not-boolean",
            json!([{"filter": {"where": "element"}}]),
            json!({"lines": [1]}),
        ),
        (
            "operators-not-a-list",
            json!([{"flatMap": {"select": "element"}}]),
            json!({"lines": [1]}),
        ),
    ] {
        let (_, outcome) = outcome_of(&p, name, &operators, 10, &input, None).await;
        assert_eq!(outcome, failed("EXPRESSION_ERROR"), "{name}");
    }
}

/// T042-46: what today's entry would refuse is refused before `enter_collection`, with today's code.
#[tokio::test]
async fn the_output_ceilings_of_today_are_checked_before_the_store() {
    let p = platform_with_operators().await;
    let (run, outcome) = outcome_of(
        &p,
        "operators-max-items",
        &json!([{"flatMap": {"select": "[element, element]"}}]),
        3,
        &json!({"lines": [1, 2]}),
        None,
    )
    .await;
    assert_eq!(
        outcome,
        failed("ITERATION_LIMIT"),
        "four elements over maxItems 3"
    );
    assert_eq!(scopes(&p, run).await, 0);
    let (_, outcome) = outcome_of(
        &p,
        "operators-max-items-at",
        &json!([{"flatMap": {"select": "[element, element]"}}, {"take": {"count": 3}}]),
        3,
        &json!({"lines": [1, 2]}),
        None,
    )
    .await;
    assert_ne!(
        outcome,
        failed("ITERATION_LIMIT"),
        "exactly maxItems enters"
    );

    // Three copies of a 100 KiB string: each element and the stage are within their ceilings, the inline snapshot
    // (256 KiB) is not.
    let text = "x".repeat(100 * 1024);
    let (run, outcome) = outcome_of(
        &p,
        "operators-snapshot",
        &json!([{"flatMap": {"select": "[element, element, element]"}}]),
        10,
        &json!({"lines": [text]}),
        None,
    )
    .await;
    assert_eq!(outcome, failed("RUN_DATA_LIMIT"));
    assert_eq!(scopes(&p, run).await, 0);
}

// ---------------------------------------------------------------------------------------------------------------------
// T042-45: a blob source with a chain
// ---------------------------------------------------------------------------------------------------------------------

async fn remove_bytes(p: &Platform, blob: &Value) {
    p.api
        .blob_backend
        .clone()
        .expect("a backend")
        .delete(&BlobKey::of(
            BlobAccess {
                tenant_id: p.tenant,
                namespace_id: namespace_of(p).await,
            },
            blob["id"]
                .as_str()
                .expect("id")
                .parse::<BlobId>()
                .expect("blob id"),
        ))
        .await
        .expect("remove the bytes");
}

#[tokio::test]
async fn a_blob_source_with_a_chain_records_a_chained_manifest_and_its_output() {
    let p = platform_with_operators().await;
    let workflow = p
        .publish(
            "operators-blob",
            &chained_ir(
                "operators-blob",
                "input",
                &json!([{"filter": {"where": "element.v > 0"}}, {"map": {"select": "element.v"}}]),
                10,
            ),
        )
        .await;
    let source = br#"[{"v": 4}, {"v": -1}, {"v": 6}, {"v": 0}, {"v": 8}]"#;
    let blob = upload(&p, source).await;
    let run = start_with_blob(&p, &workflow, blob["id"].as_str().expect("id")).await;
    let evaluator = Watched::new(Mode::Apply);
    let (scheduler, keeper) = replica(&p, evaluator.clone(), Some(blob_source(&p))).await;
    scheduler.tick().await;
    assert_eq!(scopes(&p, run).await, 1, "entered on the first pass");
    assert_eq!(
        evaluator.calls(),
        2,
        "the items call that found the blob, and the chain over its elements"
    );

    let (count, snapshot) = scope_of(&p, run).await;
    assert_eq!(count, 3);
    let manifest: Value = serde_json::from_slice(&snapshot).expect("json");
    let body = &manifest["$collectionChainSource"];
    assert_eq!(body["blob"], blob, "the blob that was read");
    assert_eq!(body["sourceCount"], 5);
    assert_eq!(body["count"], 3);
    let expected_items: Vec<Vec<u8>> = vec![b"4".to_vec(), b"6".to_vec(), b"8".to_vec()];
    assert_eq!(
        body["itemsSha256"],
        sagat_application::collection_source::items_digest(&expected_items).to_hex()
    );
    let source_items =
        sagat_application::collection_source::canonical_items(source, 1000).expect("items");
    assert_eq!(
        body["sourceSha256"],
        sagat_application::collection_source::items_digest(&source_items).to_hex()
    );
    let definition = sagat_application::definitions::IrCodec::parse(
        &JsonIrCodec,
        &serde_json::to_vec(&chained_ir(
            "operators-blob",
            "input",
            &json!([{"filter": {"where": "element.v > 0"}}, {"map": {"select": "element.v"}}]),
            10,
        ))
        .expect("json"),
    )
    .expect("parses");
    let sagat_domain::workflow::NodeKind::ForEach {
        operators: Some(chain),
        ..
    } = definition
        .definition
        .node(&NodeId::parse("items").expect("node"))
        .expect("node")
        .kind()
    else {
        panic!("a chain");
    };
    assert_eq!(
        body["chainSha256"],
        sagat_infra_ir::chain_digest(chain).to_hex()
    );
    assert_eq!(body.as_object().expect("manifest").len(), 6);
    assert!(
        manifest.get("$collectionSource").is_none(),
        "a key of its own, not today's manifest"
    );
    assert_eq!(stored_items(&p, run).await, [json!(4), json!(6), json!(8)]);

    // Restart on another replica with the blob's bytes gone: the restore reads the chained manifest and its items.
    keeper.leave().await;
    drop(scheduler);
    remove_bytes(&p, &blob).await;
    let after = Watched::new(Mode::Panic);
    let (scheduler, _keeper) = replica(&p, after.clone(), Some(blob_source(&p))).await;
    let mut worker = scoped_worker(&p, "operators-blob-worker", &["src.line"]).await;
    let seen = drive(&p, &scheduler, &mut worker, run).await;
    assert_eq!(seen, vec![(0, json!(4)), (1, json!(6)), (2, json!(8))]);
    assert_eq!(
        status_and_code(&p, run).await,
        ("SUCCEEDED".to_owned(), None)
    );
    assert_eq!(run_output(&p, run).await, json!([4, 6, 8]));
    assert_eq!(after.calls(), 0, "the restore never evaluates the chain");
    verified(&p, run).await;
}

#[tokio::test]
async fn a_blob_source_with_a_chain_is_materialised_within_the_chain_input_ceiling() {
    let p = platform_with_operators().await;
    // `maxItems` 2,000 would let today's entry read 1,001 elements; with a chain the input is bounded by 1,000.
    let workflow = p
        .publish(
            "operators-blob-input",
            &chained_ir(
                "operators-blob-input",
                "input",
                &json!([{"take": {"count": 2}}]),
                2000,
            ),
        )
        .await;
    for (elements, entered) in [(1000_u32, true), (1001, false)] {
        let array: Vec<u32> = (0..elements).collect();
        let blob = upload(&p, &serde_json::to_vec(&array).expect("json")).await;
        let run = start_with_blob(&p, &workflow, blob["id"].as_str().expect("id")).await;
        let (scheduler, keeper) =
            replica(&p, Watched::new(Mode::Apply), Some(blob_source(&p))).await;
        settle(&p, &scheduler, run, 3).await;
        keeper.leave().await;
        if entered {
            assert_eq!(scope_of(&p, run).await.0, 2, "{elements} elements");
        } else {
            assert_eq!(
                status_and_code(&p, run).await,
                failed("COLLECTION_OPERATOR_BUDGET"),
                "{elements} elements"
            );
            assert_eq!(scopes(&p, run).await, 0);
        }
    }
}

/// T042-46 with a blob source, where elements and stages may reach their own byte ceilings: at each the entry reaches
/// the store and is recorded; one byte over fails the run with the budget code and records nothing.
#[tokio::test]
async fn a_chain_at_each_byte_ceiling_reaches_the_store_with_a_valid_entry() {
    let p = platform_with_operators().await;
    let element = 256 * 1024;
    let string = |bytes: usize| Value::String("x".repeat(bytes - 2));
    let cases: [(&str, Value, Vec<Value>, Option<i32>); 4] = [
        // `[element]` adds two bytes to each element.
        (
            "operators-element-at",
            json!([{"map": {"select": "[element]"}}]),
            vec![string(element - 2)],
            Some(1),
        ),
        (
            "operators-element-over",
            json!([{"map": {"select": "[element]"}}]),
            vec![string(element - 1)],
            None,
        ),
        // Eight elements of 256 KiB, each twice: 4 MiB exactly; one more byte crosses.
        (
            "operators-stage-bytes-at",
            json!([{"flatMap": {"select": "[element, element]"}}]),
            vec![string(element); 8],
            Some(16),
        ),
        (
            "operators-stage-bytes-over",
            json!([{"flatMap": {"select": "[element, element]"}}]),
            {
                let mut values = vec![string(element); 8];
                values.push(json!(1));
                values
            },
            None,
        ),
    ];
    for (name, operators, array, entered) in cases {
        let workflow = p
            .publish(name, &chained_ir(name, "input", &operators, 20))
            .await;
        let bytes = serde_json::to_vec(&array).expect("json");
        assert!(
            bytes.len() <= 4 * MIB + 64,
            "{name}: a source the reader accepts"
        );
        let blob = upload(&p, &bytes).await;
        let run = start_with_blob(&p, &workflow, blob["id"].as_str().expect("id")).await;
        let (scheduler, keeper) =
            replica(&p, Watched::new(Mode::Apply), Some(blob_source(&p))).await;
        let advance = scheduler
            .advance_collection_from_run(p.tenant, RunId::new(run), &owner())
            .await;
        keeper.leave().await;
        match entered {
            Some(count) => {
                assert!(
                    matches!(advance, Ok(sagat_scheduler::CollectionAdvance::Entered)),
                    "{name}: {advance:?}"
                );
                assert_eq!(scope_of(&p, run).await.0, count, "{name}");
            }
            None => {
                assert!(
                    matches!(
                        advance,
                        Ok(sagat_scheduler::CollectionAdvance::LimitStopped)
                    ),
                    "{name}: {advance:?}"
                );
                assert_eq!(
                    status_and_code(&p, run).await,
                    failed("COLLECTION_OPERATOR_BUDGET"),
                    "{name}"
                );
                assert_eq!(scopes(&p, run).await, 0, "{name}");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// T042-48: once the entry has committed, nothing evaluates the chain again
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn after_entry_an_inline_chain_is_never_evaluated_again_across_a_restart() {
    let p = platform_with_operators().await;
    let workflow = p
        .publish(
            "operators-once",
            &chained_ir(
                "operators-once",
                "input.lines",
                &json!([{"distinct": {}}, {"window": {"count": 2}}]),
                10,
            ),
        )
        .await;
    let run = start_inline(&p, &workflow, &json!({"lines": [1, 1.0, 2, 3, 2, 4, 5]})).await;
    let before = Watched::new(Mode::Apply);
    let (scheduler, keeper) = replica(&p, before.clone(), None).await;
    assert!(matches!(
        scheduler
            .advance_collection_from_run(p.tenant, RunId::new(run), &owner())
            .await,
        Ok(sagat_scheduler::CollectionAdvance::Entered)
    ));
    assert_eq!(before.calls(), 1);
    keeper.leave().await;
    drop(scheduler);

    let after = Watched::new(Mode::Panic);
    let (scheduler, _keeper) = replica(&p, after.clone(), None).await;
    let mut worker = scoped_worker(&p, "operators-once-worker", &["src.line"]).await;
    let seen = drive(&p, &scheduler, &mut worker, run).await;
    assert_eq!(
        seen,
        vec![(0, json!([1, 2])), (1, json!([3, 4])), (2, json!([5]))]
    );
    assert_eq!(
        status_and_code(&p, run).await,
        ("SUCCEEDED".to_owned(), None)
    );
    assert_eq!(
        after.calls(),
        0,
        "zero chain evaluations after the entry committed"
    );
    verified(&p, run).await;
}

// ---------------------------------------------------------------------------------------------------------------------
// T042-49: telemetry, and no element value in a log line, a span or a label
// ---------------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn elements_are_counted_per_operator_and_no_value_reaches_a_log_a_span_or_a_label() {
    let p = platform_with_operators().await;
    let mark = format!("MARKED-{}", Uuid::now_v7().simple());
    let passing = p
        .publish(
            "operators-telemetry-pass",
            &chained_ir(
                "operators-telemetry-pass",
                "input.lines",
                &json!([
                    {"filter": {"where": "element.size() > 3"}},
                    {"map": {"select": "element + \"!\""}},
                    {"distinct": {}}
                ]),
                10,
            ),
        )
        .await;
    let failing = p
        .publish(
            "operators-telemetry-fail",
            &chained_ir(
                "operators-telemetry-fail",
                "input.lines",
                &json!([{"map": {"select": "element.missing"}}]),
                10,
            ),
        )
        .await;
    let lines = json!([mark.clone(), "ab", mark.clone(), format!("{mark}-2")]);
    let passed = start_inline(&p, &passing, &json!({ "lines": lines })).await;
    let refused = start_inline(&p, &failing, &json!({ "lines": [mark.clone()] })).await;
    let (scheduler, _keeper) = replica(&p, Watched::new(Mode::Apply), None).await;
    let metrics = Metrics::global();
    let before = metrics.render();

    let (guard, captured) = crate::support::logs::capture_with_spans(tracing::Level::TRACE);
    for _ in 0..3 {
        scheduler.tick().await;
    }
    drop(guard);

    assert_eq!(scope_of(&p, passed).await.0, 2);
    assert_eq!(
        status_and_code(&p, refused).await,
        failed("EXPRESSION_ERROR")
    );
    let after = metrics.render();
    let delta = |labels: &[(&str, &str)]| {
        metric(&after, "operator_elements_total", labels)
            - metric(&before, "operator_elements_total", labels)
    };
    let collection = |operator: &'static str, outcome: &'static str| {
        delta(&[
            ("surface", "collection"),
            ("operator", operator),
            ("outcome", outcome),
        ])
    };
    // Other tests of this process count too: the deltas are at least this run's.
    assert!(collection("filter", "kept") >= 3.0);
    assert!(collection("filter", "dropped") >= 1.0);
    assert!(collection("map", "produced") >= 3.0);
    assert!(collection("distinct", "kept") >= 2.0);
    assert!(collection("distinct", "dropped") >= 1.0);
    let rejections = metric(
        &after,
        "workflow_primitive_rejections_total",
        &[("kind", "collection"), ("code", "EXPRESSION_ERROR")],
    ) - metric(
        &before,
        "workflow_primitive_rejections_total",
        &[("kind", "collection"), ("code", "EXPRESSION_ERROR")],
    );
    assert!(rejections >= 1.0);

    let text = captured.text();
    assert!(
        text.contains("a collection source was refused"),
        "the capture saw the failing chain's warning: {text}"
    );
    assert!(
        !text.contains("MARKED"),
        "a value reached a log line or a span:\n{text}"
    );
    for line in after.lines().filter(|line| {
        line.starts_with("operator_elements_total")
            || line.starts_with("collection_chain_reentries_total")
            || line.starts_with("scheduler_collection_source_failures_total")
            || line.starts_with("workflow_primitive_rejections_total")
    }) {
        assert!(!line.contains("MARKED"), "a value reached a label: {line}");
    }
}

/// T042-52: a chained entry that does not commit is counted as a re-entry by its reason when it is tried again.
#[tokio::test]
async fn a_chained_entry_that_lost_its_fence_is_counted_as_a_re_entry() {
    let p = platform_with_operators().await;
    let workflow = p
        .publish(
            "operators-reentry",
            &chained_ir(
                "operators-reentry",
                "input.lines",
                &json!([{"take": {"count": 1}}]),
                10,
            ),
        )
        .await;
    let run = start_inline(&p, &workflow, &json!({"lines": [1, 2]})).await;
    let (hold, mut barrier) = gate();
    let evaluator = Watched::new(Mode::Hold(hold));
    let (scheduler, keeper) = replica(&p, evaluator.clone(), None).await;
    // A recorder of this test's own (review of S1): every metric the scheduler records on this thread lands here, and
    // nothing another test records does, so the counts below are equalities. The blocking task records nothing.
    let (metrics, recorder) =
        sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(600));
    let local = metrics::set_default_local_recorder(&recorder);

    let steal = async {
        barrier.held().await;
        crate::collection_operators_races::steal_shard(&p, run).await;
        barrier.release();
    };
    let joined_owner = owner();
    let (first, ()) = tokio::join!(
        scheduler.advance_collection_from_run(p.tenant, RunId::new(run), &joined_owner),
        steal
    );
    assert!(first.is_err(), "the stale entry is refused: {first:?}");
    assert_eq!(scopes(&p, run).await, 0);
    crate::collection_operators_races::give_back_shard(&p, run, &keeper).await;
    let again = async {
        barrier.held().await;
        barrier.release();
    };
    let joined_owner = owner();
    let (second, ()) = tokio::join!(
        scheduler.advance_collection_from_run(p.tenant, RunId::new(run), &joined_owner),
        again
    );
    assert!(
        matches!(second, Ok(sagat_scheduler::CollectionAdvance::Entered)),
        "{second:?}"
    );
    // A third try finds the scope entered: nothing more is counted.
    let joined_owner = owner();
    let third = scheduler
        .advance_collection_from_run(p.tenant, RunId::new(run), &joined_owner)
        .await;
    drop(local);
    assert!(third.is_ok(), "{third:?}");
    let page = metrics.render();
    assert_eq!(
        chain_reentries(&page, "fence"),
        1.0,
        "one re-entry, counted once:\n{page}"
    );
    assert_eq!(
        metric(
            &page,
            "operator_elements_total",
            &[
                ("surface", "collection"),
                ("operator", "take"),
                ("outcome", "kept")
            ]
        ),
        1.0,
        "the discarded evaluation is not counted; the committed one is, once"
    );
    assert_eq!(
        metric(
            &page,
            "operator_elements_total",
            &[
                ("surface", "collection"),
                ("operator", "take"),
                ("outcome", "dropped")
            ]
        ),
        1.0
    );
    assert_eq!(evaluator.calls(), 2);
    assert_eq!(scopes(&p, run).await, 1);
}

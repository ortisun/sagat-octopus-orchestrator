//! Run-creator conformance of feature 022 (T022-064, SC-022-015; plan, "Run creators"; the creators are the rows of
//! design 043's table, the complete list of what creates a run on this tree): every creator admits a run of a definition
//! that gates its start through the one admission port (`Engine::start`, D022-049), so its first commit journals
//! `concurrency.acquired` with a holder row, or `concurrency.queued` with a waiter row, and `verify_run` agrees. A
//! creator that started the run around the port would journal neither (the domain refuses such a start, so it fails
//! instead): each case fails then.
//!
//! The child node's case is `run_admission::a_child_run_of_a_gated_definition_is_admitted_through_its_key`; the API's
//! plain start is every other case of `run_admission`; a continuation successor cannot carry a gate
//! (`admission::a_gated_definition_that_reaches_a_continuation_successor_is_refused` and the publication case below).

use std::sync::Arc;

use sagat_application::batch::{BatchService, BatchWrite, DueJob};
use sagat_application::batch_executor::{
    BatchExecutor, ExecutorSettings, ItemActions, ItemEffect, ItemRequest, Stepped,
};
use sagat_application::clock::Clock as _;
use sagat_application::error::AppError;
use sagat_application::events::PublishRequest;
use sagat_application::flow_control::FlowControlRefusal;
use sagat_application::identity::RequestContext;
use sagat_application::operation_evaluation::DeclaredOperationArguments;
use sagat_application::runs::{
    BusinessStart, SignalAtStart, StartOptions, StartRun, UpdateAtStart,
};
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::{BackfillRequest, DueBackfill};
use sagat_domain::batch::{Action, Filter, Justification, Selector};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_domain::definition::WorkflowId;
use sagat_domain::events::EventId;
use sagat_domain::execution::{RunId, RunStatus, UpdateKey};
use sagat_domain::quota::QuotaResource;
use sagat_domain::schedules::Ceilings;
use sagat_domain::workflow::{ActivityName, EventType};
use sagat_infra_postgres::{PgBatchStore, PgIdentityStore};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::run_admission::{serial_start, set, start_rows};
use crate::support::engine::{
    DECLARED_OPERATION, World, activity, ctx, declared_waiter, document, wait_node, with,
};
use crate::support::flow_control::{gated_world, publish, try_publish_as};
use crate::support::schedules::{Fixture, TestClock, fixture_over, hourly};

/// How a creator admitted a run of a gated definition: the decision its first commit journals, checked against the rows
/// and against `verify_run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admitted {
    Acquired,
    Queued,
}

async fn admitted(w: &World, run: RunId) -> Admitted {
    let journal = w.journal(run).await;
    assert_eq!(
        journal.first().map(String::as_str),
        Some("run.started"),
        "{journal:?}"
    );
    let decision = match journal.get(1).map(String::as_str) {
        Some("concurrency.acquired") => Admitted::Acquired,
        Some("concurrency.queued") => Admitted::Queued,
        other => panic!("the run was not admitted through its key: {other:?} in {journal:?}"),
    };
    let rows = start_rows(w, run).await;
    match decision {
        Admitted::Acquired => assert_eq!(rows, (1, 0), "a holder row"),
        Admitted::Queued => assert_eq!(rows, (0, 1), "a waiter row"),
    }
    w.assert_consistent(run).await;
    decision
}

pub(crate) async fn runs_of(w: &World, workflow: WorkflowId) -> Vec<RunId> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM runs WHERE workflow_id = $1 ORDER BY created_at, id")
            .bind(workflow.as_uuid())
            .fetch_all(&w.db.pool)
            .await
            .expect("runs");
    ids.into_iter().map(RunId::new).collect()
}

pub(crate) fn idempotency() -> String {
    format!("creators-{}", Uuid::now_v7())
}

pub(crate) fn request<'a>(workflow: WorkflowId, input: &'a [u8], key: &'a str) -> StartRun<'a> {
    StartRun {
        namespace: "payments",
        workflow,
        version: None,
        input,
        correlation_id: None,
        deadline: None,
        key,
    }
}

pub(crate) fn business(id: &str) -> BusinessStart {
    BusinessStart {
        id: BusinessId::new(id).expect("an identifier"),
        conflict: ConflictPolicy::UseExisting,
        reuse: ReusePolicy::AllowDuplicate,
        window: ReuseWindow::default(),
    }
}

/// Completes every task of `activity` that is offered now; returns how many.
pub(crate) async fn complete_all(w: &World, activity: &str) -> usize {
    let session = w.session(&[activity]).await;
    let tasks = w.poll(&session, 16).await;
    for task in &tasks {
        w.complete(&session, task, &json!({}))
            .await
            .expect("completes");
    }
    tasks.len()
}

/// Row 1 of the creators (`POST /runs` with the options of feature 018): a start under a business identifier holds a
/// permit when the key has room and queues behind the holder when it has not.
#[tokio::test]
async fn the_start_api_with_a_business_identifier_admits_through_the_key() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("creators-api", 1)).await;
    let input = br#"{"accountId": "acct-1"}"#;
    let mut runs = Vec::new();
    for id in ["order-1", "order-2"] {
        let key = idempotency();
        let started = w
            .runs
            .start_with_options(
                &w.admin,
                &request(workflow, input, &key),
                StartOptions {
                    business: Some(business(id)),
                    start_delay: None,
                },
                &ctx(),
            )
            .await
            .expect("started");
        assert!(!started.existing);
        runs.push(started.run.info.run_id);
    }
    assert_eq!(admitted(&w, runs[0]).await, Admitted::Acquired);
    assert_eq!(admitted(&w, runs[1]).await, Admitted::Queued);
}

/// Row 4 (`RERUN_AS_NEW`): the rerun of an ended run is a new run admitted through its key: it queues behind a
/// holder, and is granted when the holder ends.
#[tokio::test]
async fn a_rerun_as_new_admits_through_the_key() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("creators-rerun", 1)).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(complete_all(&w, "act.g").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let rerun = w
        .runs
        .rerun(
            &w.admin,
            source,
            "the price list was wrong",
            &idempotency(),
            &ctx(),
        )
        .await
        .expect("rerun")
        .info
        .run_id;
    assert_eq!(admitted(&w, holder).await, Admitted::Acquired);
    assert_eq!(admitted(&w, rerun).await, Admitted::Queued);
    assert_eq!(complete_all(&w, "act.g").await, 1, "only the holder runs");
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    assert_eq!(start_rows(&w, rerun).await, (1, 0), "granted");
    w.assert_consistent(rerun).await;
}

/// The entry of `doc` waits for the signal `payment.confirmed`; its start takes a permit of `input.accountId`.
pub(crate) fn gated_waiter(name: &str) -> Value {
    let mut doc = document(
        name,
        "w",
        vec![
            with(
                wait_node("w", "payment.confirmed", "input.k", 300_000, None),
                "next",
                json!("notify"),
            ),
            activity("notify"),
        ],
    );
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    doc
}

pub(crate) async fn signal_with_start(
    w: &World,
    workflow: WorkflowId,
    id: &str,
) -> Result<sagat_application::runs::SignalStarted, AppError> {
    let key = idempotency();
    w.runs
        .signal_with_start(
            &w.admin,
            &request(workflow, br#"{"k": "s", "accountId": "acct-1"}"#, &key),
            business(id),
            SignalAtStart {
                name: "payment.confirmed",
                payload: br#"{"paid": true}"#,
            },
            &ctx(),
        )
        .await
}

/// Row 3 (Signal-with-Start): a run created by a Signal-with-Start whose key has room holds its permit and receives the
/// signal in its creation transaction.
#[tokio::test]
async fn a_signal_with_start_with_room_holds_a_permit_and_is_signalled_at_creation() {
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-signal")).await;
    let done = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("started and signalled");
    assert!(done.started);
    let run = done.run.info.run_id;
    assert_eq!(admitted(&w, run).await, Admitted::Acquired);
    let delivered: Vec<&str> = done.delivered_to.iter().map(|n| n.as_str()).collect();
    assert_eq!(delivered, ["w"]);
}

pub(crate) fn declared(name: &str) -> Value {
    let mut doc = declared_waiter(name);
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    doc
}

pub(crate) async fn update_with_start(
    w: &World,
    workflow: WorkflowId,
    id: &str,
    balance: i64,
) -> Result<sagat_application::runs::UpdateStarted, AppError> {
    let key = idempotency();
    let operation = ActivityName::parse(DECLARED_OPERATION).expect("an operation name");
    let update_key = UpdateKey::parse(&format!("update-{id}-0001")).expect("an update key");
    let arguments = DeclaredOperationArguments::parse(
        json!({"allowed": true, "nextState": {"balance": balance}})
            .to_string()
            .as_bytes(),
    )
    .expect("arguments");
    w.runs
        .update_with_start(
            &w.admin,
            &request(
                workflow,
                br#"{"account": {"balance": 10}, "k": "s", "accountId": "acct-1"}"#,
                &key,
            ),
            StartOptions {
                business: Some(business(id)),
                start_delay: None,
            },
            UpdateAtStart {
                operation: &operation,
                update_key: &update_key,
                arguments: &arguments,
                expected_state_revision: None,
                batch_job: None,
            },
            &ctx(),
        )
        .await
}

/// Row 3 (Update-with-Start): the run is admitted through its key whether it holds a permit or queues, and the Update
/// is applied to its declared state in its creation transaction either way (an Update changes the state, not a step).
#[tokio::test]
async fn an_update_with_start_admits_through_the_key_and_applies_its_update() {
    let w = gated_world().await;
    let workflow = publish(&w, declared("creators-update")).await;
    let first = update_with_start(&w, workflow, "acct-a", 20)
        .await
        .expect("started");
    assert!(first.started);
    assert_eq!(
        admitted(&w, first.run.info.run_id).await,
        Admitted::Acquired
    );
    let second = update_with_start(&w, workflow, "acct-b", 30)
        .await
        .expect("started");
    assert!(second.started);
    let run = second.run.info.run_id;
    assert_eq!(admitted(&w, run).await, Admitted::Queued);
    assert_eq!(
        w.journal(run).await,
        ["run.started", "concurrency.queued", "state.updated"],
        "the Update is applied while the run waits"
    );
}

/// A fixture of feature 018's schedules over a world whose store runs the gate; its schedules start a workflow whose
/// start takes a permit of `input.region` with limit 1.
async fn gated_schedules() -> Fixture {
    let w = gated_world().await;
    let mut doc = document("creators-scheduled", "g", vec![activity("g")]);
    doc["concurrency"] = json!({"group": "regions", "key": "input.region", "limit": 1});
    let workflow = publish(&w, doc).await;
    fixture_over(w, workflow, OffsetDateTime::now_utc(), Ceilings::default())
}

async fn schedule_runs(f: &Fixture, schedule: Uuid) -> Vec<RunId> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM runs WHERE schedule_id = $1 ORDER BY created_at, id")
            .bind(schedule)
            .fetch_all(&f.w.db.pool)
            .await
            .expect("runs");
    ids.into_iter().map(RunId::new).collect()
}

/// Rows 6, 7 and 8 (a schedule's action, a backfill, trigger-now): each run of the schedule is admitted through its
/// key; behind a holder of the key started by the API, each queues.
#[tokio::test]
async fn a_schedule_action_a_backfill_and_a_trigger_now_admit_through_the_key() {
    let f = gated_schedules().await;
    let holder = f.w.start(f.workflow, &json!({"region": "eu"})).await;
    assert_eq!(admitted(&f.w, holder).await, Admitted::Acquired);
    // the overlap policy lets every time start a run, so the key, not the schedule, decides
    let schedule = f
        .create(
            "creators-hourly",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "the action");
    let backfill = f
        .service
        .create_backfill(
            &f.w.admin,
            schedule.id,
            &BackfillRequest {
                start: f.clock.now() - Duration::hours(3),
                end: f.clock.now() - Duration::hours(2),
                overlap: Some(sagat_domain::schedules::Overlap::AllowAll),
            },
            &RequestContext::default(),
        )
        .await
        .expect("a backfill");
    let due = DueBackfill {
        tenant_id: backfill.tenant_id,
        schedule: backfill.schedule,
        id: backfill.id,
    };
    let mut acted = 0;
    for _ in 0..20 {
        match f
            .firing
            .backfill_step(&due, None)
            .await
            .expect("a backfill step")
        {
            Fired::Acted => acted += 1,
            Fired::Idle | Fired::Refused(_) => break,
        }
    }
    assert!(acted >= 1, "the backfill acted");
    f.firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: "creators-trigger-now-0001",
            },
            &RequestContext::default(),
        )
        .await
        .expect("trigger-now");
    let runs = schedule_runs(&f, schedule.id.as_uuid()).await;
    let actions: Vec<(String, String)> = sqlx::query_as(
        "SELECT kind, outcome FROM schedule_actions WHERE schedule_id = $1 ORDER BY nominal_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("actions");
    assert_eq!(
        runs.len(),
        3,
        "an action, a backfill time and a trigger: {actions:?}"
    );
    for run in runs {
        assert_eq!(admitted(&f.w, run).await, Admitted::Queued);
    }
}

/// Row 6 when the key refuses the start (D022-050; the schedule's case of the reviews of 2026-11-20): a full key queue
/// is a refusal for now, so the time stays owed and nothing of a run exists; once the key has room the same time starts.
#[tokio::test]
async fn a_schedule_whose_start_the_key_refuses_keeps_its_time_owed() {
    let f = gated_schedules().await;
    set(&f.w, QuotaResource::FlowControlKeyWaiters, 1).await;
    let _holder = f.w.start(f.workflow, &json!({"region": "eu"})).await;
    let _waiter = f.w.start(f.workflow, &json!({"region": "eu"})).await;
    let schedule = f
        .create(
            "creators-owed",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert!(
        matches!(f.fire(&schedule).await, Fired::Refused(_)),
        "the admission was refused"
    );
    assert!(schedule_runs(&f, schedule.id.as_uuid()).await.is_empty());
    assert!(
        f.outcomes(&schedule).await.is_empty(),
        "no action is recorded"
    );
    set(&f.w, QuotaResource::FlowControlKeyWaiters, 10).await;
    // the time was deferred: the next attempt is after the retry delay
    f.clock.advance(Duration::minutes(10));
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "the owed time");
    let runs = schedule_runs(&f, schedule.id.as_uuid()).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(admitted(&f.w, runs[0]).await, Admitted::Queued);
}

fn triggered(name: &str, triggers: Value, key: Option<&str>) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["triggers"] = triggers;
    doc["concurrency"] = match key {
        Some(key) => json!({"group": "accounts", "key": key, "limit": 1}),
        None => json!({"group": "accounts", "limit": 1}),
    };
    doc
}

async fn publish_event(w: &World, id: &str, data: &Value) {
    w.events
        .publish(
            &w.admin,
            PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(EventId::parse(id).expect("id")),
                event_type: EventType::parse("order.created").expect("type"),
                subject: None,
                correlation_key: None,
                occurred_at: None,
                data: serde_json::to_vec(data).expect("json"),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("published");
}

/// Row 9 (an event trigger): the run an event starts is admitted through the key its envelope names. An event whose
/// run the key refuses (its queue is full) starts nothing and stays in the inbox, and its redelivery starts the run once
/// the key has room (the plan's "pending retry" of an asynchronous creator).
#[tokio::test]
async fn an_event_trigger_admits_through_the_key_and_retries_an_event_the_key_refuses() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        triggered(
            "creators-event",
            json!([{"kind": "event", "eventType": "order.created"}]),
            Some("input.data.accountId"),
        ),
    )
    .await;
    set(&w, QuotaResource::FlowControlKeyWaiters, 1).await;
    for (id, account) in [("creators-e1", "acct-1"), ("creators-e2", "acct-1")] {
        publish_event(&w, id, &json!({"accountId": account})).await;
        w.dispatcher.tick().await;
    }
    let runs = runs_of(&w, workflow).await;
    assert_eq!(runs.len(), 2);
    assert_eq!(admitted(&w, runs[0]).await, Admitted::Acquired);
    assert_eq!(admitted(&w, runs[1]).await, Admitted::Queued);

    publish_event(&w, "creators-e3", &json!({"accountId": "acct-1"})).await;
    w.dispatcher.tick().await;
    assert_eq!(runs_of(&w, workflow).await.len(), 2, "the key refused it");
    let pending: String =
        sqlx::query_scalar("SELECT status FROM inbox_events WHERE event_id = 'creators-e3'")
            .fetch_one(&w.db.pool)
            .await
            .expect("the event");
    assert_eq!(pending, "PENDING", "the event is tried again");

    set(&w, QuotaResource::FlowControlKeyWaiters, 10).await;
    sqlx::query(
        "UPDATE inbox_events SET next_attempt_at = clock_timestamp() WHERE event_id = 'creators-e3'",
    )
    .execute(&w.db.pool)
    .await
    .expect("due again");
    w.dispatcher.tick().await;
    let runs = runs_of(&w, workflow).await;
    assert_eq!(runs.len(), 3, "the retry starts it");
    assert_eq!(admitted(&w, runs[2]).await, Admitted::Queued);
}

/// Row 10 (a cron trigger, the legacy cron pass): the run of a tick is admitted through its key; a definition whose
/// gate names no key has one key for the group, so the second tick queues behind the first.
#[tokio::test]
async fn a_cron_trigger_admits_through_the_key() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        triggered(
            "creators-cron",
            json!([{"kind": "cron", "cron": "*/5 * * * *", "timezone": "UTC"}]),
            None,
        ),
    )
    .await;
    w.dispatcher.tick().await; // plans the first time
    // two ticks of distinct minutes (a tick's key is its scheduled time)
    for minutes_ago in [10_i32, 5] {
        sqlx::query(
            "UPDATE workflow_triggers \
                SET next_fire_at = date_trunc('minute', clock_timestamp()) - make_interval(mins => $1)",
        )
        .bind(minutes_ago)
        .execute(&w.db.pool)
        .await
        .expect("due");
        w.dispatcher.tick().await;
    }
    let runs = runs_of(&w, workflow).await;
    assert_eq!(runs.len(), 2, "two ticks");
    assert_eq!(admitted(&w, runs[0]).await, Admitted::Acquired);
    assert_eq!(admitted(&w, runs[1]).await, Admitted::Queued);
}

/// The run service as the batch executor's single-run use cases.
#[derive(Clone)]
struct Runs(Arc<World>);

impl ItemActions for Runs {
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        ItemActions::apply(&self.0.runs, request).await
    }
}

/// The batch jobs of the namespace and their items, as `(job state, failure, [(item state, reason, error)])`: what a
/// failed assertion on a batch rerun shows.
pub(crate) async fn batch_state(w: &World) -> String {
    let jobs: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT state, failure_reason FROM batch_jobs WHERE namespace_id = $1 ORDER BY created_at",
    )
    .bind(w.namespace.id().as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("jobs");
    let items: Vec<(String, Option<String>, Option<String>, i16)> = sqlx::query_as(
        "SELECT i.state, i.reason, i.error_code, i.attempts FROM batch_items i \
           JOIN batch_jobs j ON j.tenant_id = i.tenant_id AND j.id = i.job_id \
          WHERE j.namespace_id = $1 ORDER BY i.ordinal",
    )
    .bind(w.namespace.id().as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("items");
    format!("jobs {jobs:?}, items {items:?}")
}

/// Creates a batch job that reruns the namespace's succeeded runs and steps it until `workflow` has `runs_after` runs (or
/// two minutes passed).
pub(crate) async fn rerun_the_succeeded_by_a_batch(
    w: &Arc<World>,
    workflow: WorkflowId,
    runs_after: usize,
) {
    let clock = TestClock::at(OffsetDateTime::now_utc() + Duration::seconds(2));
    let store = PgBatchStore::new(w.db.pool.clone());
    let job = BatchService::new(store.clone(), clock.clone())
        .create(
            &w.admin,
            w.namespace.id(),
            BatchWrite {
                selector: Selector::filter(Filter {
                    statuses: vec![RunStatus::Succeeded],
                    ..Filter::default()
                })
                .expect("selector"),
                action: Action::Rerun,
                step_id: None,
                instance_path: Vec::new(),
                signal: None,
                update: None,
                drop_business_id: false,
                repin: false,
                reason: Justification::new("rerun the succeeded run").expect("reason"),
                max_items: None,
                rate: None,
            },
            &idempotency(),
            &RequestContext::default(),
        )
        .await
        .expect("a job");
    let executor = BatchExecutor::new(
        store,
        Runs(Arc::clone(w)),
        PgIdentityStore::new(w.db.pool.clone()),
        clock,
        ExecutorSettings::default(),
    );
    let due = DueJob {
        tenant_id: job.tenant_id,
        id: job.id,
    };
    let give_up = std::time::Instant::now() + std::time::Duration::from_secs(120);
    // bounded by `give_up` only: under a loaded machine the wait for older transactions outlasts any count of steps
    loop {
        if std::time::Instant::now() > give_up {
            break;
        }
        match executor.step(&due, None).await.expect("a step") {
            Stepped::Acted => {}
            Stepped::Idle | Stepped::Refused(_) | Stepped::Backoff => {
                if runs_of(w, workflow).await.len() == runs_after
                    || std::time::Instant::now() > give_up
                {
                    break;
                }
                // the selection waits for transactions open at the job's creation (another test's)
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
    }
}

/// Row 4 (a batch `RERUN`): each rerun the job makes is a new run admitted through its key.
#[tokio::test]
async fn a_batch_rerun_admits_through_the_key() {
    let w = Arc::new(gated_world().await);
    let workflow = publish(&w, serial_start("creators-batch", 1)).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(complete_all(&w, "act.g").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    rerun_the_succeeded_by_a_batch(&w, workflow, 3).await;
    let runs = runs_of(&w, workflow).await;
    assert_eq!(
        runs.len(),
        3,
        "the source, the holder and the rerun: {}",
        batch_state(&w).await
    );
    assert_eq!(admitted(&w, holder).await, Admitted::Acquired);
    assert_eq!(admitted(&w, runs[2]).await, Admitted::Queued);
}

/// Publication: a definition that continues as new cannot gate its start (`FLOW_CONTROL_COMBINATION`), so a
/// continuation successor, the one creator that does not pass the port, never has a gate to pass.
#[tokio::test]
async fn a_definition_that_continues_as_new_cannot_gate_its_start() {
    let w = gated_world().await;
    let doc = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "creators-continue"},
        "entrypoint": "work",
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 60000},
        "concurrency": {"key": "input.accountId", "limit": 1},
        "nodes": [
            {"id": "work", "type": "activity", "activity": "act.work", "next": "again"},
            {"id": "again", "type": "continueAsNew", "input": "input"}
        ]
    });
    let refused = try_publish_as(&w, &w.admin, doc)
        .await
        .expect_err("refused");
    assert!(
        format!("{refused:?}").contains("FlowControlCombination"),
        "{refused:?}"
    );
}

/// A start refused by the key leaves nothing (the synchronous creators' answer), whichever creator asked.
#[tokio::test]
async fn a_with_start_the_key_refuses_creates_nothing() {
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-signal-full")).await;
    set(&w, QuotaResource::FlowControlKeyWaiters, 0).await;
    let first = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("the first holds the permit");
    assert_eq!(
        admitted(&w, first.run.info.run_id).await,
        Admitted::Acquired
    );
    let refused = signal_with_start(&w, workflow, "cart-2")
        .await
        .expect_err("refused");
    assert_eq!(
        refused,
        AppError::FlowControl(FlowControlRefusal::ConcurrencyQueueFull)
    );
    assert_eq!(runs_of(&w, workflow).await.len(), 1);
}

/// The connection `creators-db` of the namespace, whose credentials are a secret held by OpenBao (nothing reads or
/// connects in these tests).
pub(crate) async fn ad_hoc_connection(w: &World) {
    use sagat_application::database::{ConnectionService, ConnectionWrite};
    use sagat_domain::database::{ConnectionConfig, ConnectionStatus, DatabaseLimits, TlsPolicy};
    use sagat_domain::identity::Slug;
    use sagat_domain::secrets::SecretName;

    // the connection names a secret of the namespace held by OpenBao; nothing reads or connects in this test, so the
    // fixture records the authority without a provider (a fixture value)
    w.put_secret("db-credentials", "creators-fixture-value")
        .await;
    sqlx::query("UPDATE secrets SET authority = 'openbao' WHERE name = 'db-credentials'")
        .execute(&w.db.pool)
        .await
        .expect("the secret's authority");
    ConnectionService::new(w.store.clone(), sagat_application::clock::SystemClock)
        .put(
            &w.admin,
            w.namespace.id(),
            ConnectionWrite {
                name: Slug::parse("creators-db").expect("a name"),
                expected_revision: 0,
                request_id: Uuid::new_v4(),
                status: ConnectionStatus::Active,
                config: ConnectionConfig::postgres(
                    "db.example.com",
                    5432,
                    "orders",
                    TlsPolicy::VerifyFull { ca_pem: None },
                    SecretName::parse("db-credentials").expect("a secret name"),
                    DatabaseLimits::default(),
                )
                .expect("a configuration"),
            },
            &ctx(),
        )
        .await
        .expect("the connection");
}

/// Starts the caller's ad hoc workflow `document` under the idempotency key `key`.
pub(crate) async fn start_ad_hoc(w: &World, document: &[u8], key: &str) -> RunId {
    use sagat_application::runs::{AdHocMode, StartDatabaseExecution};
    w.runs
        .start_database(
            &w.admin,
            &StartDatabaseExecution {
                namespace: w.namespace.id(),
                mode: AdHocMode::Workflow,
                document,
                input: b"{}",
                timeout_ms: None,
                key,
            },
            &ctx(),
        )
        .await
        .expect("an ad hoc start")
        .info
        .run_id
}

/// Row 13 (an ad hoc database workflow, and the private rerun of one): a caller's ad hoc document that gates its start
/// is admitted through its key like a published one, and so is its private rerun. (A connection probe's document is the
/// platform's own and declares no gate.)
#[tokio::test]
async fn an_ad_hoc_workflow_and_its_private_rerun_admit_through_the_key() {
    use sagat_application::runs::RunOperation;

    let w = gated_world().await;
    ad_hoc_connection(&w).await;
    let mut doc = document(
        "creators-ad-hoc",
        "query",
        vec![json!({
            "id": "query", "type": "database", "connectionRef": {"name": "creators-db"},
            "operation": "query", "statement": "SELECT 1::bigint"
        })],
    );
    doc["concurrency"] = json!({"group": "reports", "limit": 1});
    let bytes = serde_json::to_vec(&doc).expect("json");
    let mut runs = Vec::new();
    for key in ["creators-ad-hoc-0001", "creators-ad-hoc-0002"] {
        runs.push(start_ad_hoc(&w, &bytes, key).await);
    }
    assert_eq!(admitted(&w, runs[0]).await, Admitted::Acquired);
    assert_eq!(admitted(&w, runs[1]).await, Admitted::Queued);
    // the holder ends; the waiter is granted; the holder's private rerun queues behind it
    w.operate_run(runs[0], RunOperation::Cancel).await;
    assert_eq!(w.run_status(runs[0]).await, RunStatus::Cancelled);
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    assert_eq!(start_rows(&w, runs[1]).await, (1, 0), "granted");
    let rerun = w
        .runs
        .rerun(
            &w.admin,
            runs[0],
            "run the report again",
            &idempotency(),
            &ctx(),
        )
        .await
        .expect("a private rerun");
    assert_eq!(
        rerun.info.origin.map(|origin| origin.as_str()),
        Some("AD_HOC_WORKFLOW")
    );
    assert_eq!(admitted(&w, rerun.info.run_id).await, Admitted::Queued);
}

pub(crate) async fn start_delayed(w: &World, workflow: WorkflowId, account: Option<&str>) -> RunId {
    let input = match account {
        Some(account) => json!({"accountId": account}),
        None => json!({}),
    };
    let bytes = serde_json::to_vec(&input).expect("json");
    let key = idempotency();
    let started = w
        .runs
        .start_with_options(
            &w.admin,
            &request(workflow, &bytes, &key),
            StartOptions {
                business: None,
                start_delay: Some(std::time::Duration::from_millis(1)),
            },
            &ctx(),
        )
        .await
        .expect("a delayed start");
    // The delay's end is this process's clock plus a millisecond, and the release pass compares it with the database's
    // `now()`: a database clock behind this host's (the test VM's drifted 85 ms on 2026-11-20) would leave it not due on
    // the next tick. The tests mean "the delay has ended": wait until the database's clock is past it (a bounded
    // condition on the clock the pass decides with; the row's delay is fixed by a trigger).
    let until: i64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM start_delayed_until) * 1000000)::bigint FROM runs WHERE id = $1",
    )
    .bind(started.run.info.run_id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the delay's end");
    crate::support::flow_control::until_past(&w.db.pool, until).await;
    started.run.info.run_id
}

/// Row 1 with feature 018's start delay (FR-022-018: "a run with an 018 start delay acquires or enqueues when the delay
/// elapses"; D022-052): while its delay runs the run holds no permit and waits in no queue; the release decides its key
/// in the release's transaction: a permit when the key has room, a wait behind the holder when it has not, and a key
/// that does not evaluate ends the run `FAILED`/`EXPRESSION_ERROR` there.
#[tokio::test]
async fn a_delayed_start_is_admitted_through_the_key_at_its_release() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("creators-delay", 1)).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let free = start_delayed(&w, workflow, Some("acct-2")).await;
    let behind = start_delayed(&w, workflow, Some("acct-1")).await;
    let no_key = start_delayed(&w, workflow, None).await;
    for run in [free, behind, no_key] {
        assert_eq!(w.journal(run).await, ["run.started", "run.start_delayed"]);
        assert_eq!(start_rows(&w, run).await, (0, 0), "nothing while delayed");
        assert_eq!(w.run_status(run).await, RunStatus::Waiting);
        w.assert_consistent(run).await;
    }
    let report = w.scheduler.tick().await;
    assert_eq!(report.starts_released, 3, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(
        w.journal(free).await,
        [
            "run.started",
            "run.start_delayed",
            "run.start_released",
            "concurrency.acquired",
            "step.released"
        ]
    );
    assert_eq!(start_rows(&w, free).await, (1, 0));
    assert_eq!(
        w.journal(behind).await,
        [
            "run.started",
            "run.start_delayed",
            "run.start_released",
            "concurrency.queued"
        ]
    );
    assert_eq!(start_rows(&w, behind).await, (0, 1));
    assert_eq!(w.run_status(behind).await, RunStatus::Waiting);
    assert_eq!(w.run_status(no_key).await, RunStatus::Failed);
    let (code,): (Option<String>,) = sqlx::query_as("SELECT stop_code FROM runs WHERE id = $1")
        .bind(no_key.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("run");
    assert_eq!(code.as_deref(), Some("EXPRESSION_ERROR"));
    assert_eq!(start_rows(&w, no_key).await, (0, 0));
    for run in [holder, free, behind, no_key] {
        w.assert_consistent(run).await;
    }
    // the queued one is granted when the holder ends, like any queued start
    let session = w.session(&["act.g"]).await;
    let tasks = w.poll(&session, 8).await;
    let task = tasks
        .iter()
        .find(|t| t.run_id == holder)
        .expect("the holder's task");
    w.complete(&session, task, &json!({})).await.expect("done");
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    assert_eq!(start_rows(&w, behind).await, (1, 0), "granted");
    w.assert_consistent(behind).await;
}

/// D022-052, D022-054: a key that refuses a delayed run at its release (its waiter queue is full) keeps it delayed, with
/// nothing of the run written; the pass holds it back until its retry instant, then tries again and starts it once the key
/// has room. A refusal is not a failure of the pass.
#[tokio::test]
async fn a_delayed_start_the_key_refuses_at_its_release_is_tried_again_by_the_next_pass() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("creators-delay-full", 1)).await;
    set(&w, QuotaResource::FlowControlKeyWaiters, 1).await;
    let _holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let _waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let delayed = start_delayed(&w, workflow, Some("acct-1")).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.starts_released, 0, "{report:?}");
    assert_eq!(report.failed, 0, "a refusal is not a failure: {report:?}");
    assert_eq!(report.gate_starts_deferred, 1, "{report:?}");
    assert_eq!(
        w.journal(delayed).await,
        ["run.started", "run.start_delayed"]
    );
    assert_eq!(start_rows(&w, delayed).await, (0, 0));
    w.assert_consistent(delayed).await;
    set(&w, QuotaResource::FlowControlKeyWaiters, 10).await;
    let report = w.scheduler.tick().await;
    assert_eq!(
        (report.starts_released, report.gate_starts_deferred),
        (0, 0),
        "held back until its retry instant: {report:?}"
    );
    sqlx::query("UPDATE start_release_deferrals SET retry_at = now() WHERE run_id = $1")
        .bind(delayed.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("due again");
    let report = w.scheduler.tick().await;
    assert_eq!(report.starts_released, 1, "{report:?}");
    assert_eq!(start_rows(&w, delayed).await, (0, 1), "queued now");
    w.assert_consistent(delayed).await;
}

/// `a` then `b`; the start takes a permit of `input.accountId` with limit 1.
fn serial_pair(name: &str) -> Value {
    let mut doc = document(
        name,
        "a",
        vec![with(activity("a"), "next", json!("b")), activity("b")],
    );
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    doc
}

fn reset_request(sequence: u64) -> sagat_application::runs::ResetRequest<'static> {
    sagat_application::runs::ResetRequest {
        sequence,
        reason: "the second step used a wrong price list",
        acknowledge_reexecution: true,
        business: sagat_application::runs::RerunBusinessId::Keep,
        reapply: sagat_domain::execution::ResetReapply::None,
        source_policy: sagat_application::runs::ResetSourcePolicy::RequireTerminal,
    }
}

/// `(event type, restored)` of each row of a run's journal.
async fn rows(w: &World, run: RunId) -> Vec<(String, bool)> {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT event_type, detail->>'restored' FROM run_events WHERE run_id = $1 ORDER BY seq",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("the journal");
    rows.into_iter()
        .map(|(event, restored)| (event, restored.as_deref() == Some("true")))
        .collect()
}

/// Row 5 (a reset, `REQUIRE_TERMINAL`; plan, "Run creators"; FR-022-018, FR-022-038; `contracts/journal-events.md`, reset
/// rules): the completion of `a` of an ended gated run is a reset point, since its prefix's `concurrency.*` events are
/// history (`REMATERIALIZE`). The reset run takes a fresh permit in its admission transaction, after `run.reset`, or the
/// reset is refused `CONCURRENCY_PERMIT_UNAVAILABLE` and nothing is created; it never queues. The source's acquisition,
/// copied `restored`, makes no row, and the consistency check reads the journal with it.
#[tokio::test]
async fn a_reset_takes_a_fresh_permit_or_is_refused_and_never_queues() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_pair("creators-reset")).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    let points = w
        .runs
        .reset_points(&w.admin, source, &ctx())
        .await
        .expect("points");
    let after_a = points
        .iter()
        .find(|point| point.node.as_deref() == Some("a"))
        .expect("the completion of `a` is a point")
        .sequence;

    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let runs_before = runs_of(&w, workflow).await.len();
    let refused = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(after_a),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect_err("no free permit");
    assert_eq!(
        refused,
        AppError::FlowControl(FlowControlRefusal::PermitUnavailable)
    );
    assert_eq!(
        runs_of(&w, workflow).await.len(),
        runs_before,
        "nothing is created, nothing queues"
    );
    assert_eq!(start_rows(&w, holder).await, (1, 0));

    // the holder ends: the reset takes the permit it gave back
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(holder).await, RunStatus::Succeeded);
    let made = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(after_a),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect("a reset");
    let reset = made.run.expect("the reset run").info.run_id;
    let journal = rows(&w, reset).await;
    let restored: Vec<&str> = journal
        .iter()
        .filter(|(_, restored)| *restored)
        .map(|(event, _)| event.as_str())
        .collect();
    assert!(
        restored.starts_with(&["run.started", "concurrency.acquired"]),
        "the source's acquisition is copied as history: {journal:?}"
    );
    let fresh: Vec<&str> = journal
        .iter()
        .filter(|(_, restored)| !restored)
        .map(|(event, _)| event.as_str())
        .collect();
    assert_eq!(
        fresh[..2],
        ["run.reset", "concurrency.acquired"],
        "the fresh permit right after the reset, before the run goes on: {journal:?}"
    );
    assert!(fresh.contains(&"step.released"), "{journal:?}");
    assert_eq!(
        start_rows(&w, reset).await,
        (1, 0),
        "one holder row: the fresh permit"
    );
    w.assert_consistent(reset).await;
    assert_eq!(complete_all(&w, "act.b").await, 1, "`b` runs again");
    assert_eq!(w.run_status(reset).await, RunStatus::Succeeded);
    assert_eq!(start_rows(&w, reset).await, (0, 0), "given back at its end");
    w.assert_consistent(reset).await;
}

/// `contracts/journal-events.md`, reset rules: a point after a run's `concurrency.queued` and before its grant is no
/// point (the run never started work), and every point after a `concurrency.wait_timed_out` or `concurrency.dequeued` is
/// refused (`REFUSE`); the source's start, the first event, stays a point.
#[test]
fn the_reset_rules_of_the_concurrency_events_are_the_contracts() {
    use sagat_domain::execution::{ResetRule, rule_by_name};
    for event in [
        "concurrency.queued",
        "concurrency.acquired",
        "concurrency.released",
    ] {
        assert_eq!(
            rule_by_name(event),
            Some(ResetRule::Rematerialize),
            "{event}"
        );
    }
    for event in ["concurrency.wait_timed_out", "concurrency.dequeued"] {
        assert_eq!(
            rule_by_name(event),
            Some(ResetRule::Refuse { until: None }),
            "{event}"
        );
    }
}

/// The reset rules of a step's permit (`contracts/journal-events.md`): a reset at the completion of a step that held a
/// permit restores its acquisition as history; the reset run gives nothing back for it and holds no row of it, and a
/// step after the point asks for its own permit as in any run.
#[tokio::test]
async fn a_reset_after_a_gated_step_restores_its_permit_as_history() {
    let w = gated_world().await;
    let mut doc = document(
        "creators-reset-step",
        "a",
        vec![with(activity("a"), "next", json!("b")), activity("b")],
    );
    for node in 0..2 {
        doc["nodes"][node]["concurrency"] =
            json!({"group": "steps", "key": "input.accountId", "limit": 1});
    }
    let workflow = publish(&w, doc).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    let after_a = w
        .runs
        .reset_points(&w.admin, source, &ctx())
        .await
        .expect("points")
        .iter()
        .find(|point| point.node.as_deref() == Some("a"))
        .expect("the completion of `a` is a point")
        .sequence;
    let made = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(after_a),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect("a reset");
    let reset = made.run.expect("the reset run").info.run_id;
    let journal = rows(&w, reset).await;
    assert!(
        journal
            .iter()
            .any(|(event, restored)| event == "concurrency.acquired" && *restored),
        "{journal:?}"
    );
    assert!(
        !journal
            .iter()
            .any(|(event, restored)| event == "concurrency.released" && !*restored),
        "nothing is given back for the restored permit: {journal:?}"
    );
    w.assert_consistent(reset).await;
    assert_eq!(
        complete_all(&w, "act.b").await,
        1,
        "`b` takes its own permit and runs"
    );
    assert_eq!(w.run_status(reset).await, RunStatus::Succeeded);
    w.assert_consistent(reset).await;
}

/// The run service as the completer's single-reset use case.
struct Completions(Arc<World>);

impl sagat_application::reset_completion::ResetCompletions for Completions {
    async fn complete(
        &self,
        principal: &sagat_domain::identity::Principal,
        entry: &sagat_application::runs::PendingReset,
        fence: Option<sagat_application::schedules::ShardFence>,
        ctx: &RequestContext,
    ) -> Result<sagat_application::reset_completion::Completion, AppError> {
        self.0
            .runs
            .complete_reset(principal, entry, fence, ctx)
            .await
    }
}

/// `(state, failure reason, new run)` of an entry of the reset register.
async fn reset_entry(
    w: &World,
    id: sagat_domain::execution::ResetId,
) -> (String, Option<String>, Option<Uuid>) {
    sqlx::query_as("SELECT state, failure_reason, new_run_id FROM run_resets WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the entry")
}

/// Row 5's pending reset (`CANCEL_SOURCE`; 018's plan, row 18): the completion of a reset of a source that held a permit
/// takes a fresh one once the source has ended, or fails the entry `CONCURRENCY_PERMIT_UNAVAILABLE` when the key has no
/// room (a run queued behind the source is not passed); it never queues.
#[tokio::test]
async fn a_pending_reset_takes_a_fresh_permit_or_fails_concurrency_permit_unavailable() {
    use sagat_application::reset_completion::{Completed, ResetCompleter};
    use sagat_application::runs::ResetSourcePolicy;

    let w = Arc::new(gated_world().await);
    let workflow = publish(&w, serial_pair("creators-pending-reset")).await;
    let completer = ResetCompleter::new(
        w.store.clone(),
        Completions(Arc::clone(&w)),
        PgIdentityStore::new(w.db.pool.clone()),
        sagat_application::clock::SystemClock,
    );
    let session = w.session(&["act.a", "act.b"]).await;
    // two open sources of two keys: `a` done, `b` in flight
    let mut open = Vec::new();
    for account in ["acct-1", "acct-2"] {
        let run = w.start(workflow, &json!({"accountId": account})).await;
        let a = w.one_task_of(&session, "act.a").await;
        w.complete(&session, &a, &json!({})).await.expect("a");
        let b = w.one_task_of(&session, "act.b").await;
        let after_a: i64 = sqlx::query_scalar(
            "SELECT seq FROM run_events WHERE run_id = $1 AND event_type = 'step.completed' ORDER BY seq LIMIT 1",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the completion of `a`");
        open.push((run, b, u64::try_from(after_a).expect("small")));
    }
    // a run of the first key queues behind its source
    let behind = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(admitted(&w, behind).await, Admitted::Queued);
    let mut entries = Vec::new();
    for (run, _, after_a) in &open {
        let mut request = reset_request(*after_a);
        request.source_policy = ResetSourcePolicy::CancelSource;
        let made = w
            .runs
            .reset(&w.admin, *run, request, &idempotency(), &ctx())
            .await
            .expect("a pending reset");
        assert!(made.run.is_none(), "it waits for its source");
        entries.push(made.reset.id);
    }
    // the sources end: their permits are given back, and the run behind the first is granted
    for (run, b, _) in &open {
        w.complete(&session, b, &json!({})).await.expect("b");
        assert!(w.run_status(*run).await.is_terminal());
    }
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    assert_eq!(start_rows(&w, behind).await, (1, 0));

    let mut completions = Vec::new();
    for id in &entries {
        let due = sagat_application::reset_completion::DueReset {
            tenant_id: w.admin.tenant_id(),
            id: *id,
        };
        completions.push(completer.step(&due, None).await.expect("a completion"));
    }
    assert_eq!(
        completions,
        [
            Completed::Failed("CONCURRENCY_PERMIT_UNAVAILABLE"),
            Completed::Created
        ]
    );
    let (state, reason, new_run) = reset_entry(&w, entries[0]).await;
    assert_eq!(
        (state.as_str(), reason.as_deref(), new_run),
        ("FAILED", Some("CONCURRENCY_PERMIT_UNAVAILABLE"), None),
        "the first key's permit is the queued run's"
    );
    let (state, reason, new_run) = reset_entry(&w, entries[1]).await;
    assert_eq!((state.as_str(), reason.as_deref()), ("CREATED", None));
    let reset = RunId::new(new_run.expect("the reset run"));
    assert_eq!(start_rows(&w, reset).await, (1, 0), "a fresh permit");
    w.assert_consistent(reset).await;
}

/// D022-051 and D022-052: the release of a start delay queues a run outside its creation, so it takes the namespace's
/// admission lock before it counts the queued runs. With the lock held by the test, a release and a start at the
/// `queued_runs` ceiling both wait on it (`pg_stat_activity`, no sleep); once it is released exactly one queues and the
/// other is refused (the start `FLOW_CONTROL_LIMIT`) or deferred (the release, tried again by the next pass).
#[tokio::test]
async fn a_release_and_a_start_at_the_queued_runs_ceiling_are_decided_one_after_the_other() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("creators-release-race", 1)).await;
    for account in ["a", "b", "c"] {
        w.start(workflow, &json!({"accountId": account})).await;
    }
    // one run queued already; a key's waiters stay one below the namespace's ceiling
    let first = w.start(workflow, &json!({"accountId": "c"})).await;
    assert_eq!(start_rows(&w, first).await, (0, 1));
    set(&w, QuotaResource::QueuedRuns, 2).await;
    let delayed = start_delayed(&w, workflow, Some("a")).await;
    let lock_key = format!(
        "quota:{}:{}:{}",
        QuotaResource::ActiveRuns,
        w.admin.tenant_id(),
        w.namespace.id()
    );
    let mut held = w.db.pool.begin().await.expect("a transaction");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *held)
        .await
        .expect("the namespace's admission lock held");
    let input = json!({"accountId": "b"});
    let start = w.start_with(workflow, &input, "release-race-start", None);
    let tick = w.scheduler.tick();
    let release = async {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                     AND wait_event_type = 'Lock' AND wait_event = 'advisory' AND pid <> pg_backend_pid()",
                )
                .fetch_one(&w.db.pool)
                .await
                .expect("activity");
                if waiting >= 2 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the release and the start wait on the namespace's admission lock");
        held.rollback().await.expect("the lock released");
    };
    let (started, report, ()) = tokio::join!(start, tick, release);
    let released = start_rows(&w, delayed).await == (0, 1);
    match &started {
        Ok(run) => {
            assert!(!released, "both queued: {report:?}");
            assert_eq!(start_rows(&w, *run).await, (0, 1));
            assert_eq!(report.gate_starts_deferred, 1, "{report:?}");
        }
        Err(refused) => {
            assert!(released, "neither queued: {refused:?} {report:?}");
            assert_eq!(
                *refused,
                AppError::FlowControl(FlowControlRefusal::Limit {
                    limit: "queued_runs"
                })
            );
        }
    }
    let queued: i64 =
        sqlx::query_scalar("SELECT count(*) FROM concurrency_waiters WHERE permit_scope = 'RUN'")
            .fetch_one(&w.db.pool)
            .await
            .expect("waiters");
    assert_eq!(
        queued, 2,
        "the namespace holds exactly its ceiling of queued runs"
    );
    w.assert_consistent(delayed).await;
}

/// A Signal-with-Start of `workflow` for business identifier `id` under the idempotency key `key`, signal `name`.
async fn signal_with_start_as(
    w: &World,
    workflow: WorkflowId,
    id: &str,
    key: &str,
    name: &str,
) -> Result<sagat_application::runs::SignalStarted, AppError> {
    w.runs
        .signal_with_start(
            &w.admin,
            &request(workflow, br#"{"k": "s", "accountId": "acct-1"}"#, key),
            business(id),
            SignalAtStart {
                name,
                payload: br#"{"paid": true}"#,
            },
            &ctx(),
        )
        .await
}

/// The held signal rows of `run`, and the audit records of `action` about it.
async fn held_and_audited(w: &World, run: RunId, action: &str) -> (i64, Vec<Option<String>>) {
    let held: i64 =
        sqlx::query_scalar("SELECT count(*) FROM flow_control_held_signals WHERE run_id = $1")
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("held rows");
    let reasons: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE resource_id = $1 AND action = $2 ORDER BY created_at",
    )
    .bind(run.to_string())
    .bind(action)
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    (held, reasons)
}

/// Row 3 when the start would queue (task T022-109, decision D022-087; FR-022-018, FR-018-024 as amended): the run is
/// created and queued, the signal held with it, and the answer says so (`held`, nothing delivered); a retry of the request
/// answers the same. When the holder ends, the grant of the queued start delivers the signal in its own transaction,
/// once, audited `run.signalled` as the caller's; a later pass delivers nothing more, and a retry still answers `held`.
#[tokio::test]
async fn a_signal_with_start_whose_start_queues_holds_the_signal_and_the_grant_delivers_it() {
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-signal-queued")).await;
    let first = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("the first holds the permit");
    assert_eq!(
        admitted(&w, first.run.info.run_id).await,
        Admitted::Acquired
    );
    let key = idempotency();
    let queued = signal_with_start_as(&w, workflow, "cart-2", &key, "payment.confirmed")
        .await
        .expect("created and held");
    let run = queued.run.info.run_id;
    assert!(queued.started && queued.held, "{queued:?}");
    assert!(queued.delivered_to.is_empty(), "{queued:?}");
    assert_eq!(admitted(&w, run).await, Admitted::Queued);
    assert_eq!(
        held_and_audited(&w, run, "run.signalled").await,
        (1, vec![])
    );
    let retried = signal_with_start_as(&w, workflow, "cart-2", &key, "payment.confirmed")
        .await
        .expect("a retry answers the same");
    assert_eq!(
        (
            retried.run.info.run_id,
            retried.started,
            retried.held,
            retried.delivered_to.len()
        ),
        (run, true, true, 0)
    );
    // the holder ends: the next pass grants the queued start and delivers the held signal with it
    w.operate_run(
        first.run.info.run_id,
        sagat_application::runs::RunOperation::Cancel,
    )
    .await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(start_rows(&w, run).await, (1, 0), "granted");
    assert_eq!(
        w.step_status(run, "w").await,
        Some(sagat_domain::execution::StepStatus::Succeeded),
        "{:?}",
        w.journal(run).await
    );
    let (held, delivered) = held_and_audited(&w, run, "run.signalled").await;
    assert_eq!(
        (held, delivered.len()),
        (0, 1),
        "delivered once: {delivered:?}"
    );
    let signalled_by: String = sqlx::query_scalar(
        "SELECT actor FROM audit_records WHERE resource_id = $1 AND action = 'run.signalled'",
    )
    .bind(run.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("the actor");
    assert_eq!(
        signalled_by,
        w.admin.subject().as_str(),
        "audited as the caller's"
    );
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 0);
    assert_eq!(held_and_audited(&w, run, "run.signalled").await.1.len(), 1);
    let again = signal_with_start_as(&w, workflow, "cart-2", &key, "payment.confirmed")
        .await
        .expect("a retry after the delivery answers the same");
    assert!(again.held && again.delivered_to.is_empty(), "{again:?}");
    w.assert_consistent(run).await;
}

/// Decision D022-087 item 4: a queued run whose queue timeout passes, or which is cancelled, drops the signal held with
/// it in the commit that ends it, audited `run.signal_dropped` (reason `RUN_ENDED`), and nothing is delivered.
#[tokio::test]
async fn a_held_signal_is_dropped_with_an_audit_when_its_queued_run_times_out_or_is_cancelled() {
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-signal-dropped")).await;
    let _first = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("the first holds the permit");
    let timed_out = signal_with_start(&w, workflow, "cart-2")
        .await
        .expect("held");
    let cancelled = signal_with_start(&w, workflow, "cart-3")
        .await
        .expect("held");
    for started in [&timed_out, &cancelled] {
        assert!(started.held, "{started:?}");
    }
    let (timed_out, cancelled) = (timed_out.run.info.run_id, cancelled.run.info.run_id);
    crate::run_admission::age_start_deadline(&w, timed_out).await;
    let report = w.scheduler.time_out_gates().await;
    assert_eq!(report.gate_waits_timed_out, 1, "{report:?}");
    w.operate_run(cancelled, sagat_application::runs::RunOperation::Cancel)
        .await;
    for run in [timed_out, cancelled] {
        assert!(w.run_status(run).await.is_terminal());
        let (held, dropped) = held_and_audited(&w, run, "run.signal_dropped").await;
        assert_eq!(held, 0, "the row went with the run's end");
        assert_eq!(dropped.len(), 1, "{dropped:?}");
        assert!(
            dropped[0]
                .as_deref()
                .is_some_and(|reason| reason.starts_with("RUN_ENDED")),
            "{dropped:?}"
        );
        assert!(
            held_and_audited(&w, run, "run.signalled")
                .await
                .1
                .is_empty(),
            "nothing was delivered"
        );
        w.assert_consistent(run).await;
    }
}

/// A Signal-with-Start of `workflow` for `id`, signal `payment.confirmed` with `payload`, as `principal`.
async fn signal_with_start_by(
    w: &World,
    principal: &sagat_domain::identity::Principal,
    workflow: WorkflowId,
    id: &str,
    payload: &[u8],
) -> Result<sagat_application::runs::SignalStarted, AppError> {
    let key = idempotency();
    w.runs
        .signal_with_start(
            principal,
            &request(workflow, br#"{"k": "s", "accountId": "acct-1"}"#, &key),
            business(id),
            SignalAtStart {
                name: "payment.confirmed",
                payload,
            },
            &ctx(),
        )
        .await
}

/// Decision D022-087 item 3 as narrowed by D022-126: a held signal whose wait is on the entry path but on a branch the
/// grant does not take (the entry is a condition, true for this input, whose true branch is an activity) finds no step
/// waiting at the grant; it is dropped with an audit record (`NOT_WAITING_AT_GRANT`), as `signalRun` would refuse it,
/// and the run goes on. With room on the key, the creation takes the same branch and refuses the request.
#[tokio::test]
async fn a_held_signal_the_branch_taken_does_not_reach_is_dropped_with_an_audit() {
    let w = gated_world().await;
    let mut doc = document(
        "creators-signal-branch",
        "check",
        vec![
            crate::support::engine::condition_node("check", "input.k == 's'", "g", "w"),
            activity("g"),
            wait_node("w", "payment.confirmed", "input.k", 300_000, None),
        ],
    );
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    let workflow = publish(&w, doc).await;
    let first = signal_with_start(&w, workflow, "cart-1").await;
    assert!(
        matches!(first, Err(AppError::SignalRefused { .. })),
        "with room, the branch taken does not wait: {first:?}"
    );
    let holder = w
        .start(workflow, &json!({"k": "s", "accountId": "acct-1"}))
        .await;
    let held = signal_with_start(&w, workflow, "cart-2")
        .await
        .expect("held: a branch of the entry path waits for it");
    assert!(held.held, "{held:?}");
    let run = held.run.info.run_id;
    w.operate_run(holder, sagat_application::runs::RunOperation::Cancel)
        .await;
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    let (rows, dropped) = held_and_audited(&w, run, "run.signal_dropped").await;
    assert_eq!(rows, 0);
    assert!(
        dropped.len() == 1
            && dropped[0]
                .as_deref()
                .is_some_and(|reason| reason.starts_with("NOT_WAITING_AT_GRANT")),
        "{dropped:?}"
    );
    assert_eq!(
        w.step_status(run, "g").await,
        Some(sagat_domain::execution::StepStatus::Ready),
        "the run goes on"
    );
    w.assert_consistent(run).await;
}

/// Batch 5 correctness minor 1 and security suspicion 4 (decision D022-126): a Signal-with-Start whose wait is not on
/// the start's entry path (an activity runs first) is refused when the start would queue, as it is with room on the
/// key (`SIGNAL_NOT_ACCEPTED_AT_START`), and nothing is created: no run, no waiter row, no held signal. Before, it was
/// held, charged to the tenant, and always dropped at the grant.
#[tokio::test]
async fn a_signal_with_start_whose_wait_is_not_on_the_entry_path_is_refused_when_the_start_would_queue()
 {
    let w = gated_world().await;
    let mut doc = document(
        "creators-signal-later",
        "g",
        vec![
            with(activity("g"), "next", json!("w")),
            wait_node("w", "payment.confirmed", "input.k", 300_000, None),
        ],
    );
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    let workflow = publish(&w, doc).await;
    let with_room = signal_with_start(&w, workflow, "cart-1").await;
    assert_eq!(
        with_room.expect_err("refused with room"),
        AppError::SignalRefused {
            reason: "SIGNAL_NOT_ACCEPTED_AT_START"
        }
    );
    let _holder = w
        .start(workflow, &json!({"k": "s", "accountId": "acct-1"}))
        .await;
    let queued = signal_with_start(&w, workflow, "cart-2").await;
    assert_eq!(
        queued.expect_err("refused when the start would queue"),
        AppError::SignalRefused {
            reason: "SIGNAL_NOT_ACCEPTED_AT_START"
        }
    );
    assert_eq!(
        runs_of(&w, workflow).await.len(),
        1,
        "only the holder exists"
    );
    let (waiters, held): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM concurrency_waiters WHERE permit_scope = 'RUN'), \
                (SELECT count(*) FROM flow_control_held_signals)",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("rows");
    assert_eq!((waiters, held), (0, 0), "no waiter row and no held signal");
}

/// Batch 5 correctness minor 1 (decision D022-126 item 1): a Signal-with-Start whose payload, copied to every wait of
/// its name on the entry path (seventeen parallel waits, 250 KiB each), would pass the run's 4 MiB retained-output bound
/// is refused `QUOTA_EXCEEDED` when the start would queue, as it is with room on the key, and nothing is created.
#[tokio::test]
async fn a_signal_with_start_over_the_output_bound_is_refused_when_the_start_would_queue() {
    let w = gated_world().await;
    let branches: Vec<String> = (0..17).map(|i| format!("w{i:02}")).collect();
    let names: Vec<&str> = branches.iter().map(String::as_str).collect();
    let mut nodes = vec![crate::support::engine::parallel_node("p", &names, "all")];
    nodes.extend(
        names
            .iter()
            .map(|id| wait_node(id, "payment.confirmed", "input.k", 300_000, None)),
    );
    let mut doc = document("creators-signal-bound", "p", nodes);
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    let workflow = publish(&w, doc).await;
    let payload = format!("\"{}\"", "x".repeat(250 * 1024 - 2));
    let refused = |result: Result<sagat_application::runs::SignalStarted, AppError>| {
        assert_eq!(
            result.expect_err("over the bound"),
            AppError::QuotaExceeded("output retained by one run")
        );
    };
    refused(signal_with_start_by(&w, &w.admin, workflow, "cart-1", payload.as_bytes()).await);
    let _holder = w
        .start(workflow, &json!({"k": "s", "accountId": "acct-1"}))
        .await;
    refused(signal_with_start_by(&w, &w.admin, workflow, "cart-2", payload.as_bytes()).await);
    assert_eq!(
        runs_of(&w, workflow).await.len(),
        1,
        "only the holder exists"
    );
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM flow_control_held_signals")
        .fetch_one(&w.db.pool)
        .await
        .expect("rows");
    assert_eq!(held, 0, "no held signal");
}

/// The value of the series `flow_control_held_signals_total{outcome="<outcome>"}` on the process's page, 0 when absent.
fn held_signals_counted(page: &str, outcome: &str) -> f64 {
    let series = format!("flow_control_held_signals_total{{outcome=\"{outcome}\"}} ");
    page.lines()
        .find_map(|line| line.strip_prefix(&series))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0.0)
}

/// Batch 5 security nit (decision D022-127): the held, delivered and dropped signals are counted by outcome once their
/// transaction commits, in the closed vocabulary (no tenant, run or signal name). Other tests of the process count too,
/// so each outcome is asserted to have grown by at least the commits of this test.
#[tokio::test]
async fn the_held_signal_counter_counts_each_outcome_after_its_commit() {
    let telemetry = sagat_telemetry::Metrics::global();
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-signal-counted")).await;
    let before = telemetry.render();
    let first = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("the first holds the permit");
    let delivered = signal_with_start(&w, workflow, "cart-2")
        .await
        .expect("held");
    let ended = signal_with_start(&w, workflow, "cart-3")
        .await
        .expect("held");
    assert!(delivered.held && ended.held);
    w.operate_run(
        ended.run.info.run_id,
        sagat_application::runs::RunOperation::Cancel,
    )
    .await;
    w.operate_run(
        first.run.info.run_id,
        sagat_application::runs::RunOperation::Cancel,
    )
    .await;
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    let after = telemetry.render();
    for (outcome, at_least) in [
        ("held", 2.0),
        ("delivered", 1.0),
        ("dropped_run_ended", 1.0),
    ] {
        assert!(
            held_signals_counted(&after, outcome) - held_signals_counted(&before, outcome)
                >= at_least,
            "{outcome}: {}",
            after
                .lines()
                .filter(|line| line.starts_with("flow_control_held_signals_total"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    for line in after
        .lines()
        .filter(|line| line.starts_with("flow_control_held_signals_total{"))
    {
        let labels = line
            .split_once('{')
            .and_then(|(_, rest)| rest.split_once('}'));
        assert!(
            labels
                .is_some_and(|(labels, _)| labels.starts_with("outcome=") && !labels.contains(',')),
            "one label, the outcome: {line}"
        );
    }
}

/// Re-read security nit 3 (decision D022-129): the audit record of a dropped signal names its sender whole (an actor is
/// bounded at 255 characters), not cut to 128.
#[tokio::test]
async fn a_dropped_signal_keeps_the_whole_name_of_its_sender() {
    use sagat_domain::identity::Role;
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-signal-sender")).await;
    let _first = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("the first holds the permit");
    let name = format!("carol-{}", "x".repeat(200));
    let carol = w.member(&name, Role::Operator).await;
    w.bind_everything(&carol).await;
    let held = signal_with_start_by(&w, &carol, workflow, "cart-2", br#"{"paid": true}"#)
        .await
        .expect("held");
    assert!(held.held, "{held:?}");
    let run = held.run.info.run_id;
    w.operate_run(run, sagat_application::runs::RunOperation::Cancel)
        .await;
    let (_, dropped) = held_and_audited(&w, run, "run.signal_dropped").await;
    let reason = dropped.first().cloned().flatten().unwrap_or_default();
    assert!(
        reason.starts_with("RUN_ENDED") && reason.contains(&format!("sent by '{name}'")),
        "the whole sender: {reason}"
    );
}

/// FR-018-024 as amended by D022-087 item 2: a Signal-with-Start whose start would queue, of a signal no node of the
/// definition waits for, keeps 018's typed refusal (`SIGNAL_NOT_ACCEPTED_AT_START`) and nothing of the run exists.
#[tokio::test]
async fn a_signal_with_start_of_a_name_no_node_waits_for_is_refused_when_the_start_would_queue() {
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-signal-unknown")).await;
    let _first = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("the first holds the permit");
    let refused = signal_with_start_as(&w, workflow, "cart-2", &idempotency(), "order.shipped")
        .await
        .expect_err("refused");
    assert_eq!(
        refused,
        AppError::SignalRefused {
            reason: "SIGNAL_NOT_ACCEPTED_AT_START"
        }
    );
    assert_eq!(runs_of(&w, workflow).await.len(), 1, "nothing was created");
    let (waiters, held): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM concurrency_waiters WHERE permit_scope = 'RUN'), \
                (SELECT count(*) FROM flow_control_held_signals)",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("rows");
    assert_eq!((waiters, held), (0, 0), "no waiter row and no held signal");
}

/// Reviews of 2026-11-20 (architecture B1, security Major 1, correctness M1): a release a ceiling refused is held back
/// from the pass with a backoff (`start_release_deferrals`), so refused releases do not take every pass's batch and a
/// later delayed start of another key is released by the next pass. The deferral backs off: 5 s, then 10 s.
#[tokio::test]
async fn deferred_releases_back_off_and_do_not_hold_back_the_starts_behind_them() {
    use sagat_application::entropy::SystemEntropy;
    use sagat_application::execution::Engine;
    use sagat_scheduler::{Scheduler, SchedulerSettings};

    let w = gated_world().await;
    let workflow = publish(&w, serial_start("creators-deferral", 1)).await;
    set(&w, QuotaResource::FlowControlKeyWaiters, 1).await;
    let _holder = w.start(workflow, &json!({"accountId": "full"})).await;
    let _waiter = w.start(workflow, &json!({"accountId": "full"})).await;
    let refused = [
        start_delayed(&w, workflow, Some("full")).await,
        start_delayed(&w, workflow, Some("full")).await,
    ];
    let later = start_delayed(&w, workflow, Some("free")).await;
    // a pass that takes two candidates at a time: the two refused ones are the oldest
    let narrow = Scheduler::new(
        w.store.clone(),
        Engine::new(sagat_infra_ir::JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            batch: 2,
            ..SchedulerSettings::default()
        },
    );
    let first = narrow.tick().await;
    assert_eq!(first.gate_starts_deferred, 2, "{first:?}");
    assert_eq!(first.starts_released, 0, "{first:?}");
    let second = narrow.tick().await;
    assert_eq!(
        second.starts_released, 1,
        "the later start is not held back: {second:?}"
    );
    assert_eq!(
        second.gate_starts_deferred, 0,
        "the refused ones back off: {second:?}"
    );
    assert_eq!(start_rows(&w, later).await, (1, 0));
    let backoff: Vec<(i32, String, f64)> = sqlx::query_as(
        "SELECT deferrals, reason, extract(epoch FROM retry_at - first_deferred_at)::float8 \
           FROM start_release_deferrals WHERE run_id = ANY($1) ORDER BY run_id",
    )
    .bind(refused.iter().map(|r| r.as_uuid()).collect::<Vec<_>>())
    .fetch_all(&w.db.pool)
    .await
    .expect("the deferrals");
    assert_eq!(backoff.len(), 2);
    for (deferrals, reason, wait) in &backoff {
        assert_eq!((*deferrals, reason.as_str()), (1, "CONCURRENCY_QUEUE_FULL"));
        assert!(
            (5.0..6.0).contains(wait),
            "the first backoff is 5 s: {wait}"
        );
    }
    // past its retry instant a refused release is tried again, and backs off twice as long
    sqlx::query(
        "UPDATE start_release_deferrals SET retry_at = now() - interval '1 second', \
                first_deferred_at = now() - interval '6 seconds' WHERE run_id = $1",
    )
    .bind(refused[0].as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("due again");
    let third = narrow.tick().await;
    assert_eq!(third.gate_starts_deferred, 1, "{third:?}");
    let (deferrals, ahead): (i32, f64) = sqlx::query_as(
        "SELECT deferrals, extract(epoch FROM retry_at - now())::float8 FROM start_release_deferrals WHERE run_id = $1",
    )
    .bind(refused[0].as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the deferral");
    assert_eq!(deferrals, 2);
    assert!(
        (9.0..=10.0).contains(&ahead),
        "the second backoff is 10 s: {ahead}"
    );
    for run in refused {
        assert_eq!(start_rows(&w, run).await, (0, 0), "nothing was written");
        w.assert_consistent(run).await;
    }
}

/// Reviews of 2026-11-20 (correctness M3): a prefix that holds a step's `concurrency.released` (the reset is made after
/// the next step's completion) is history too: its restore gives nothing back in the new run, which holds no row of it.
#[tokio::test]
async fn a_reset_whose_prefix_gave_a_step_permit_back_restores_it_as_history() {
    let w = gated_world().await;
    let mut doc = document(
        "creators-reset-released",
        "a",
        vec![
            with(activity("a"), "next", json!("b")),
            with(activity("b"), "next", json!("c")),
            activity("c"),
        ],
    );
    doc["nodes"][0]["concurrency"] =
        json!({"group": "steps", "key": "input.accountId", "limit": 1});
    let workflow = publish(&w, doc).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    for activity in ["act.a", "act.b", "act.c"] {
        assert_eq!(complete_all(&w, activity).await, 1, "{activity}");
    }
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    let after_b = w
        .runs
        .reset_points(&w.admin, source, &ctx())
        .await
        .expect("points")
        .iter()
        .find(|point| point.node.as_deref() == Some("b"))
        .expect("the completion of `b` is a point")
        .sequence;
    let made = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(after_b),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect("a reset");
    let reset = made.run.expect("the reset run").info.run_id;
    let journal = rows(&w, reset).await;
    assert!(
        journal
            .iter()
            .any(|(event, restored)| event == "concurrency.released" && *restored),
        "the prefix gave the step's permit back: {journal:?}"
    );
    w.assert_consistent(reset).await;
    assert_eq!(complete_all(&w, "act.c").await, 1);
    assert_eq!(w.run_status(reset).await, RunStatus::Succeeded);
}

/// Reviews of 2026-11-20 (correctness m1): the reservation rule holds for a reset. With the source's permit free but a run
/// still queued for it, a reset is refused `CONCURRENCY_PERMIT_UNAVAILABLE`: it never takes a permit a waiter waits for.
#[tokio::test]
async fn a_reset_never_takes_a_free_permit_a_queued_run_waits_for() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_pair("creators-reset-reservation")).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let behind = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    // the source gave its permit back; no pass has granted the waiter yet
    assert_eq!(start_rows(&w, behind).await, (0, 1));
    let after_a = w
        .runs
        .reset_points(&w.admin, source, &ctx())
        .await
        .expect("points")
        .iter()
        .find(|point| point.node.as_deref() == Some("a"))
        .expect("a point")
        .sequence;
    let refused = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(after_a),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect_err("the free permit is the waiter's");
    assert_eq!(
        refused,
        AppError::FlowControl(FlowControlRefusal::PermitUnavailable)
    );
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    assert_eq!(
        start_rows(&w, behind).await,
        (1, 0),
        "the waiter is granted"
    );
}

/// The two guards of a gate a process does not serve (keys reviews of 2026-11-20, the surviving mutation), each proved
/// alone. A reset's run is created through the admission point: on a control plane whose store holds no digest keys it
/// is refused `BINARY_CAPABILITY_MISSING` there, nothing reset (the start gate's guard answers an admission or a reset
/// with an internal error, so dropping the admission point's check turns this red, as it does
/// `admission::a_process_with_the_switch_the_activation_and_the_keys_admits_a_concurrency_gated_run`). A release has no
/// admission point: on such a scheduler the start gate's own guard decides nothing, the pass fails it (a configuration
/// to look at, not a key's ceiling to defer, D022-054), and nothing of the run is written.
#[tokio::test]
async fn a_process_without_the_digest_keys_refuses_a_reset_and_decides_no_release() {
    let w = gated_world().await;
    let ringless = || {
        sagat_infra_postgres::PgRunStore::new(w.execution_pool.clone(), w.cache.clone())
            .with_flow_control(crate::support::flow_control::switches(true))
    };
    let workflow = publish(&w, serial_pair("creators-reset-ringless")).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    let after_a = w
        .runs
        .reset_points(&w.admin, source, &ctx())
        .await
        .expect("points")
        .iter()
        .find(|point| point.node.as_deref() == Some("a"))
        .expect("the completion of `a` is a point")
        .sequence;
    let runs_before = runs_of(&w, workflow).await;
    let refused = sagat_application::runs::RunService::new(
        ringless(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .reset(
        &w.admin,
        source,
        reset_request(after_a),
        &idempotency(),
        &ctx(),
    )
    .await
    .expect_err("no digest keys");
    assert_eq!(
        refused,
        AppError::FlowControl(FlowControlRefusal::BinaryCapabilityMissing)
    );
    assert_eq!(
        runs_of(&w, workflow).await,
        runs_before,
        "no reset run was created"
    );

    let delayed_workflow = publish(&w, serial_start("creators-delay-ringless", 1)).await;
    let delayed = start_delayed(&w, delayed_workflow, Some("acct-9")).await;
    let scheduler = sagat_scheduler::Scheduler::new(
        ringless(),
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_scheduler::SchedulerSettings::default(),
    )
    .with_children(w.cache.clone());
    let report = scheduler.tick().await;
    assert_eq!(
        (
            report.starts_released,
            report.gate_starts_deferred,
            report.failed
        ),
        (0, 0, 1),
        "{report:?}"
    );
    assert_eq!(
        w.journal(delayed).await,
        ["run.started", "run.start_delayed"]
    );
    assert_eq!(start_rows(&w, delayed).await, (0, 0));
    w.assert_consistent(delayed).await;
}

/// `a` then `b`; the start takes a slot of `input.accountId`, `limit` per `per`.
fn rated_pair(name: &str, limit: u32, per: &str) -> Value {
    let mut doc = document(
        name,
        "a",
        vec![with(activity("a"), "next", json!("b")), activity("b")],
    );
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": limit, "per": per});
    doc
}

/// The run's own run-level grant and waiter rows: (grants, waiters).
async fn slot_rows(w: &World, run: RunId) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN'), \
                (SELECT count(*) FROM rate_limit_waiters WHERE run_id = $1 AND limit_scope = 'RUN')",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows")
}

/// The sequence of the completion of `node` among the source's reset points.
async fn point_after(w: &World, source: RunId, node: &str) -> u64 {
    w.runs
        .reset_points(&w.admin, source, &ctx())
        .await
        .expect("points")
        .iter()
        .find(|point| point.node.as_deref() == Some(node))
        .unwrap_or_else(|| panic!("the completion of `{node}` is a point"))
        .sequence
}

/// `contracts/journal-events.md`, reset rules (task T022-086, D022-069 item 1): a run's `rate_limit.queued`, `granted`
/// and `returned` are history (`REMATERIALIZE`), and every point after a `rate_limit.wait_timed_out` or
/// `rate_limit.dequeued` is refused (`REFUSE`, to the end of the journal).
#[test]
fn the_reset_rules_of_the_rate_limit_events_are_the_contracts() {
    use sagat_domain::execution::{ResetRule, rule_by_name};
    for event in [
        "rate_limit.queued",
        "rate_limit.granted",
        "rate_limit.returned",
    ] {
        assert_eq!(
            rule_by_name(event),
            Some(ResetRule::Rematerialize),
            "{event}"
        );
    }
    for event in ["rate_limit.wait_timed_out", "rate_limit.dequeued"] {
        assert_eq!(
            rule_by_name(event),
            Some(ResetRule::Refuse { until: None }),
            "{event}"
        );
    }
}

/// Task T022-086 (FR-022-041, FR-022-038; D022-069 items 2 and 3): the completion of `a` of an ended rate-limited run is a
/// reset point, since its prefix's `rate_limit.granted` is history. The reset run takes a fresh slot in its admission
/// transaction right after `run.reset`, counted from its grant; the source's grant, copied `restored`, makes no row, and
/// the key's window then counts the source's slot and the reset's (2 of `limit: 2`).
#[tokio::test]
async fn a_reset_takes_a_fresh_slot_right_after_run_reset() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_pair("creators-rate-reset", 2, "hour")).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    let after_a = point_after(&w, source, "a").await;
    let made = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(after_a),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect("a reset");
    let reset = made.run.expect("the reset run").info.run_id;
    let journal = rows(&w, reset).await;
    let restored: Vec<&str> = journal
        .iter()
        .filter(|(_, restored)| *restored)
        .map(|(event, _)| event.as_str())
        .collect();
    assert!(
        restored.starts_with(&["run.started", "rate_limit.granted"]),
        "the source's grant is copied as history: {journal:?}"
    );
    let fresh: Vec<&str> = journal
        .iter()
        .filter(|(_, restored)| !restored)
        .map(|(event, _)| event.as_str())
        .collect();
    assert_eq!(
        fresh[..2],
        ["run.reset", "rate_limit.granted"],
        "the fresh slot right after the reset, before the run goes on: {journal:?}"
    );
    assert!(fresh.contains(&"step.released"), "{journal:?}");
    assert_eq!(
        slot_rows(&w, reset).await,
        (1, 0),
        "one grant row: the fresh slot"
    );
    let (counted, from_grant): (i64, bool) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM rate_limit_grants g JOIN runs r ON r.id = g.run_id \
                  WHERE r.workflow_id = $1 AND g.limit_scope = 'RUN'), \
                (SELECT leased_at = granted_at FROM rate_limit_grants WHERE run_id = $2 AND limit_scope = 'RUN')",
    )
    .bind(workflow.as_uuid())
    .bind(reset.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the key's grants");
    assert_eq!(counted, 2, "the source's slot and the reset's");
    assert!(from_grant, "a run's slot counts from its grant");
    w.assert_consistent(reset).await;
    assert_eq!(complete_all(&w, "act.b").await, 1, "`b` runs again");
    assert_eq!(w.run_status(reset).await, RunStatus::Succeeded);
    w.assert_consistent(reset).await;
}

/// Task T022-086 (D022-069 items 3 and 4): a reset never waits and never takes a slot a buffered run waits for. With
/// `limit: 1, per: hour` the source's own slot still counts, so its reset is refused `RATE_LIMIT_SLOT_UNAVAILABLE` and
/// nothing is created or buffered. With `limit: 1, per: second` and a run buffered behind the source, the slot that has
/// aged out is the buffered run's (the reservation rule): the reset is refused, and the next pass grants the waiter. A
/// start that declares both gates is refused rate first when its slot is not free, and `CONCURRENCY_PERMIT_UNAVAILABLE`
/// when only its permit is not.
#[tokio::test]
async fn a_reset_without_a_free_slot_is_refused_rate_first_and_never_waits() {
    let w = gated_world().await;
    let refused_reset = |source: RunId, sequence: u64| {
        let w = &w;
        async move {
            w.runs
                .reset(
                    &w.admin,
                    source,
                    reset_request(sequence),
                    &idempotency(),
                    &ctx(),
                )
                .await
                .expect_err("refused")
        }
    };
    // the source's own slot counts for its window
    let hourly = publish(&w, rated_pair("creators-rate-reset-full", 1, "hour")).await;
    let source = w.start(hourly, &json!({"accountId": "acct-1"})).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    let runs_before = runs_of(&w, hourly).await.len();
    assert_eq!(
        refused_reset(source, point_after(&w, source, "a").await).await,
        AppError::FlowControl(FlowControlRefusal::SlotUnavailable)
    );
    assert_eq!(
        runs_of(&w, hourly).await.len(),
        runs_before,
        "nothing is created, nothing is buffered"
    );
    let waiters: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_limit_waiters")
        .fetch_one(&w.db.pool)
        .await
        .expect("waiters");
    assert_eq!(waiters, 0);

    // a slot that aged out is the buffered run's
    let per_second = publish(&w, rated_pair("creators-rate-reset-reserved", 1, "second")).await;
    let source = w.start(per_second, &json!({"accountId": "acct-2"})).await;
    let behind = w.start(per_second, &json!({"accountId": "acct-2"})).await;
    assert_eq!(slot_rows(&w, behind).await, (0, 1), "buffered");
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    let granted_us: i64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM granted_at) * 1000000)::bigint FROM rate_limit_grants \
          WHERE run_id = $1 AND limit_scope = 'RUN'",
    )
    .bind(source.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the source's grant");
    crate::support::flow_control::until_past(&w.db.pool, granted_us + 1_000_000).await;
    assert_eq!(
        refused_reset(source, point_after(&w, source, "a").await).await,
        AppError::FlowControl(FlowControlRefusal::SlotUnavailable),
        "the free slot is the buffered run's"
    );
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    assert_eq!(slot_rows(&w, behind).await, (1, 0), "the waiter is granted");
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(behind).await, RunStatus::Succeeded);

    // both gates: rate first, then the permit
    let mut both = rated_pair("creators-rate-reset-both", 1, "hour");
    both["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    let both = publish(&w, both).await;
    let source = w.start(both, &json!({"accountId": "acct-3"})).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    assert_eq!(
        refused_reset(source, point_after(&w, source, "a").await).await,
        AppError::FlowControl(FlowControlRefusal::SlotUnavailable),
        "its permit is free, its slot is not: rate first"
    );
    let mut roomy = rated_pair("creators-rate-reset-permit", 10, "hour");
    roomy["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    let roomy = publish(&w, roomy).await;
    let source = w.start(roomy, &json!({"accountId": "acct-4"})).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    let holder = w.start(roomy, &json!({"accountId": "acct-4"})).await;
    assert_eq!(start_rows(&w, holder).await, (1, 0), "it holds the permit");
    assert_eq!(
        refused_reset(source, point_after(&w, source, "a").await).await,
        AppError::FlowControl(FlowControlRefusal::PermitUnavailable),
        "its slot is free, its permit is not"
    );
    for run in [source, holder, behind] {
        w.assert_consistent(run).await;
    }
}

/// Decision D022-076 item 3 (re-review Major 3; FR-022-041, SC-022-022): a reset is refused only without a free slot, by
/// the reservation rule: it takes a fresh slot when its key's counted grants plus the eligible waiters are below its
/// limit, and a waiter ahead keeps its slot. With `limit: 3, per: hour`, the source's slot counted and one eligible
/// waiter (a start that also waits for a permit another run holds, so it keeps its slot reserved), the reset finds
/// `1 + 1 < 3` and takes a fresh slot; under strict order it was refused. With the reset's slot and a second such waiter
/// (`2 + 2`), the next reset is refused `RATE_LIMIT_SLOT_UNAVAILABLE`, and the waiters keep their room.
#[tokio::test]
async fn a_reset_takes_a_fresh_slot_beside_the_eligible_waiters_and_is_refused_only_without_one() {
    let w = gated_world().await;
    let rated = publish(&w, rated_pair("creators-rate-reset-reservation", 3, "hour")).await;
    let mut dual = rated_pair("creators-rate-reset-reservation-dual", 3, "hour");
    dual["concurrency"] = json!({"group": "serial", "key": "input.accountId", "limit": 1});
    let dual = publish(&w, dual).await;
    let mut permit = document(
        "creators-rate-reset-reservation-permit",
        "p",
        vec![activity("p")],
    );
    permit["concurrency"] = json!({"group": "serial", "key": "input.accountId", "limit": 1});
    let permit = publish(&w, permit).await;
    let input = json!({"accountId": "acct-1"});
    let source = w.start(rated, &input).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    let holder = w.start(permit, &input).await;
    assert_eq!(start_rows(&w, holder).await, (1, 0), "it holds the permit");
    let first = w.start(dual, &input).await;
    assert_eq!(
        slot_rows(&w, first).await,
        (0, 1),
        "its slot has room, its permit has not: it waits for both"
    );
    let made = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(point_after(&w, source, "a").await),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect("one slot counted and one eligible waiter leave room for the reset");
    let reset = made.run.expect("the reset run").info.run_id;
    assert_eq!(slot_rows(&w, reset).await, (1, 0), "a fresh slot");
    let second = w.start(dual, &input).await;
    assert_eq!(slot_rows(&w, second).await, (0, 1));
    let refused = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(point_after(&w, source, "a").await),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect_err("two slots counted and two eligible waiters fill the limit");
    assert_eq!(
        refused,
        AppError::FlowControl(FlowControlRefusal::SlotUnavailable)
    );
    for run in [first, second] {
        assert_eq!(
            slot_rows(&w, run).await,
            (0, 1),
            "the waiters keep their room"
        );
    }
    for run in [source, holder, first, reset, second] {
        w.assert_consistent(run).await;
    }
}

/// Task T022-086 (`CANCEL_SOURCE`; 018's plan, row 18; D022-069 item 4): the completion of a pending reset of a
/// rate-limited source takes a fresh slot once the source has ended, or fails its entry `RATE_LIMIT_SLOT_UNAVAILABLE`
/// when the key's window has no free slot; it never waits. With `limit: 2, per: hour`, the first key holds its source's
/// slot and another run's (no room); the second key holds its source's only (room for one).
#[tokio::test]
async fn a_pending_reset_takes_a_fresh_slot_or_fails_rate_limit_slot_unavailable() {
    use sagat_application::reset_completion::{Completed, ResetCompleter};
    use sagat_application::runs::ResetSourcePolicy;

    let w = Arc::new(gated_world().await);
    let workflow = publish(&w, rated_pair("creators-rate-pending-reset", 2, "hour")).await;
    let completer = ResetCompleter::new(
        w.store.clone(),
        Completions(Arc::clone(&w)),
        PgIdentityStore::new(w.db.pool.clone()),
        sagat_application::clock::SystemClock,
    );
    let session = w.session(&["act.a", "act.b"]).await;
    // two open sources of two keys: `a` done, `b` in flight
    let mut open = Vec::new();
    for account in ["acct-1", "acct-2"] {
        let run = w.start(workflow, &json!({"accountId": account})).await;
        let a = w.one_task_of(&session, "act.a").await;
        w.complete(&session, &a, &json!({})).await.expect("a");
        let b = w.one_task_of(&session, "act.b").await;
        open.push((run, b, point_after(&w, run, "a").await));
    }
    let mut entries = Vec::new();
    for (run, _, after_a) in &open {
        let mut request = reset_request(*after_a);
        request.source_policy = ResetSourcePolicy::CancelSource;
        let made = w
            .runs
            .reset(&w.admin, *run, request, &idempotency(), &ctx())
            .await
            .expect("a pending reset");
        assert!(made.run.is_none(), "it waits for its source");
        entries.push(made.reset.id);
    }
    // a second slot of the first key after its request was accepted (a full window at the request is refused there,
    // decision D022-079): the window is full when the source ends
    let other = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(slot_rows(&w, other).await, (1, 0));
    for (run, b, _) in &open {
        w.complete(&session, b, &json!({})).await.expect("b");
        assert!(w.run_status(*run).await.is_terminal());
    }
    let mut completions = Vec::new();
    for id in &entries {
        let due = sagat_application::reset_completion::DueReset {
            tenant_id: w.admin.tenant_id(),
            id: *id,
        };
        completions.push(completer.step(&due, None).await.expect("a completion"));
    }
    assert_eq!(
        completions,
        [
            Completed::Failed("RATE_LIMIT_SLOT_UNAVAILABLE"),
            Completed::Created
        ]
    );
    let (state, reason, new_run) = reset_entry(&w, entries[0]).await;
    assert_eq!(
        (state.as_str(), reason.as_deref(), new_run),
        ("FAILED", Some("RATE_LIMIT_SLOT_UNAVAILABLE"), None)
    );
    let (state, reason, new_run) = reset_entry(&w, entries[1]).await;
    assert_eq!((state.as_str(), reason.as_deref()), ("CREATED", None));
    let reset = RunId::new(new_run.expect("the reset run"));
    assert_eq!(slot_rows(&w, reset).await, (1, 0), "a fresh slot");
    w.assert_consistent(reset).await;
}

/// Task T022-086 (correctness Major 2 of the reviews of 2026-11-21; D022-069 item 2): a reset at the completion of a step
/// that held a slot restores its `rate_limit.granted` as history. The reset run gives nothing back for it
/// (`rate_limit.returned` is never fresh) and holds no row of it: its only step grant is the one `b` takes after the reset,
/// and the consistency check reads the journal with the restored copy before and after `b` runs.
#[tokio::test]
async fn a_reset_after_a_rate_limited_step_restores_its_slot_as_history() {
    let w = gated_world().await;
    let mut doc = document(
        "creators-rate-reset-step",
        "a",
        vec![with(activity("a"), "next", json!("b")), activity("b")],
    );
    for node in 0..2 {
        doc["nodes"][node]["rateLimit"] =
            json!({"group": "steps", "key": "input.accountId", "limit": 3, "per": "hour"});
    }
    let workflow = publish(&w, doc).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    let after_a = point_after(&w, source, "a").await;
    let made = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(after_a),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect("a reset");
    let reset = made.run.expect("the reset run").info.run_id;
    let step_grants = || async {
        let nodes: Vec<String> = sqlx::query_scalar(
            "SELECT node FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'STEP' ORDER BY node",
        )
        .bind(reset.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("grants");
        nodes
    };
    let journal = rows(&w, reset).await;
    assert!(
        journal
            .iter()
            .any(|(event, restored)| event == "rate_limit.granted" && *restored),
        "{journal:?}"
    );
    assert!(
        !journal
            .iter()
            .any(|(event, restored)| event == "rate_limit.returned" && !*restored),
        "nothing is given back for the restored slot: {journal:?}"
    );
    assert_eq!(step_grants().await, ["b"], "only `b`'s fresh slot");
    w.assert_consistent(reset).await;
    assert_eq!(
        complete_all(&w, "act.b").await,
        1,
        "`b` runs with its own slot"
    );
    assert_eq!(w.run_status(reset).await, RunStatus::Succeeded);
    assert_eq!(step_grants().await, ["b"]);
    w.assert_consistent(reset).await;
}

/// Rows of the step-scope slots of `run`, by node.
async fn step_slot_nodes(w: &World, run: RunId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT node FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'STEP' ORDER BY node",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("grants")
}

/// Task T022-086 (correctness re-review of 2026-11-21, minor 2; D022-069 item 2): a reset whose prefix returned a
/// step's slot (a pause while `a` was granted and not leased journals `rate_limit.returned`; the resume asks again and
/// the empty window grants it) restores the `granted`, `returned`, `granted` history of `a` and holds no row of it.
/// With `limit: 3` the window has no slack: the source counts two grants (`a`'s second and `b`'s), so a restored grant
/// counted as a third would leave the reset's `b` queued.
#[tokio::test]
async fn a_reset_whose_prefix_returned_a_step_slot_restores_it_as_history() {
    let w = gated_world().await;
    let mut doc = document(
        "creators-rate-reset-returned",
        "a",
        vec![with(activity("a"), "next", json!("b")), activity("b")],
    );
    for node in 0..2 {
        doc["nodes"][node]["rateLimit"] =
            json!({"group": "steps", "key": "input.accountId", "limit": 3, "per": "hour"});
    }
    let workflow = publish(&w, doc).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(
        step_slot_nodes(&w, source).await,
        ["a"],
        "`a` is granted at the start"
    );
    w.operate_run(source, sagat_application::runs::RunOperation::Pause)
        .await;
    assert_eq!(
        step_slot_nodes(&w, source).await,
        Vec::<String>::new(),
        "the pause returned the unleased slot"
    );
    w.operate_run(source, sagat_application::runs::RunOperation::Resume)
        .await;
    assert_eq!(
        step_slot_nodes(&w, source).await,
        ["a"],
        "the resume asks again and the empty window grants `a` a second time"
    );
    assert_eq!(complete_all(&w, "act.a").await, 1);
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(source).await, RunStatus::Succeeded);
    assert_eq!(step_slot_nodes(&w, source).await, ["a", "b"]);
    let after_a = point_after(&w, source, "a").await;
    let made = w
        .runs
        .reset(
            &w.admin,
            source,
            reset_request(after_a),
            &idempotency(),
            &ctx(),
        )
        .await
        .expect("a reset");
    let reset = made.run.expect("the reset run").info.run_id;
    let journal = rows(&w, reset).await;
    let restored = |name: &str| {
        journal
            .iter()
            .filter(|(event, restored)| event == name && *restored)
            .count()
    };
    assert_eq!(
        (
            restored("rate_limit.granted"),
            restored("rate_limit.returned")
        ),
        (2, 1),
        "`a`'s granted, returned, granted are history: {journal:?}"
    );
    assert!(
        !journal
            .iter()
            .any(|(event, restored)| event == "rate_limit.returned" && !*restored),
        "nothing is given back for the restored slot: {journal:?}"
    );
    assert_eq!(
        step_slot_nodes(&w, reset).await,
        ["b"],
        "only `b`'s fresh slot"
    );
    w.assert_consistent(reset).await;
    assert_eq!(complete_all(&w, "act.b").await, 1);
    assert_eq!(w.run_status(reset).await, RunStatus::Succeeded);
    assert_eq!(step_slot_nodes(&w, reset).await, ["b"]);
    w.assert_consistent(reset).await;
}

/// Task T022-114 (decision D022-079, gap G022-019): a `CANCEL_SOURCE` reset of an open source whose start took the only
/// slot of `limit: 1, per: hour` finds the key's window full as it arrives (the source's own slot counts for its window
/// and cancelling frees none): it is refused `RATE_LIMIT_SLOT_UNAVAILABLE` at once, and nothing happens: the source is
/// not cancelled, no entry and no run exist. A source of a key whose window has room is accepted as before.
#[tokio::test]
async fn a_cancel_source_reset_on_a_full_window_is_refused_at_its_request_and_cancels_nothing() {
    use sagat_application::runs::ResetSourcePolicy;

    let w = gated_world().await;
    let workflow = publish(&w, rated_pair("creators-rate-full-cancel", 1, "hour")).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let a = w.one_task_of(&session, "act.a").await;
    w.complete(&session, &a, &json!({})).await.expect("a");
    let after_a = point_after(&w, source, "a").await;
    let mut request = reset_request(after_a);
    request.source_policy = ResetSourcePolicy::CancelSource;
    let refused = w
        .runs
        .reset(&w.admin, source, request, &idempotency(), &ctx())
        .await
        .expect_err("refused at its request");
    assert_eq!(
        refused,
        AppError::FlowControl(FlowControlRefusal::SlotUnavailable)
    );
    assert_eq!(
        w.run_status(source).await,
        RunStatus::Running,
        "not cancelled"
    );
    // the refusal tells only a caller who may reset the run: another tenant's principal does not see the run and a
    // viewer may not reset it, whatever its key's window (review of 2026-11-25, security minor 1)
    {
        use sagat_domain::identity::{Principal, Role};
        let mut request = reset_request(after_a);
        request.source_policy = ResetSourcePolicy::CancelSource;
        let foreign = w
            .runs
            .reset(&w.other_admin, source, request, &idempotency(), &ctx())
            .await
            .expect_err("another tenant's principal");
        assert_eq!(foreign, AppError::NotFound);
        let viewer = w.member("vera", Role::Viewer).await;
        let viewer = Principal::new(w.admin.tenant_id(), viewer.subject().clone(), Role::Viewer);
        let mut request = reset_request(after_a);
        request.source_policy = ResetSourcePolicy::CancelSource;
        let forbidden = w
            .runs
            .reset(&viewer, source, request, &idempotency(), &ctx())
            .await
            .expect_err("a viewer");
        assert!(
            matches!(forbidden, AppError::Forbidden { .. }),
            "{forbidden:?}"
        );
    }
    let stopping: bool =
        sqlx::query_scalar("SELECT stop_reason IS NOT NULL FROM runs WHERE id = $1")
            .bind(source.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("the source");
    assert!(!stopping, "no cancellation was requested");
    let entries: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_resets WHERE source_run_id = $1")
            .bind(source.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("entries");
    assert_eq!(entries, 0, "no entry");
    assert_eq!(runs_of(&w, workflow).await, [source], "no run");
    w.assert_consistent(source).await;

    // a source of another key, whose window has room once the source's own slot is counted (`limit: 2`)
    let roomy = publish(&w, rated_pair("creators-rate-roomy-cancel", 2, "hour")).await;
    let open = w.start(roomy, &json!({"accountId": "acct-2"})).await;
    let a = w.one_task_of(&session, "act.a").await;
    assert_eq!(a.run_id, open);
    w.complete(&session, &a, &json!({})).await.expect("a");
    let mut request = reset_request(point_after(&w, open, "a").await);
    request.source_policy = ResetSourcePolicy::CancelSource;
    let made = w
        .runs
        .reset(&w.admin, open, request, &idempotency(), &ctx())
        .await
        .expect("accepted");
    assert!(made.run.is_none(), "it waits for its source");
}

/// Review of 2026-11-25 (correctness minor 2, security minor 2; decision D022-079): a `CANCEL_SOURCE` reset's advisory
/// window check is asked under the source's row, after the receipt, so a retry that arrives while the accepted request
/// is still in flight, the window filled meanwhile, answers that request's entry instead of being refused.
#[tokio::test]
async fn a_cancel_source_reset_retried_while_the_first_is_in_flight_answers_its_entry() {
    use sagat_application::runs::ResetSourcePolicy;

    let w = gated_world().await;
    let workflow = publish(&w, rated_pair("creators-rate-replay-cancel", 2, "hour")).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let source = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let a = w.one_task_of(&session, "act.a").await;
    w.complete(&session, &a, &json!({})).await.expect("a");
    let after_a = point_after(&w, source, "a").await;
    let key = idempotency();
    let request = || {
        let mut request = reset_request(after_a);
        request.source_policy = ResetSourcePolicy::CancelSource;
        request
    };
    // the first request holds the source's row, has passed its advisory check (one slot of two is free) and waits to
    // write its entry, which the test holds back; the window then fills, and a retry with the same key arrives before the
    // first commits: it must wait for the source's row and answer the first request's entry
    let mut held = w.db.pool.begin().await.expect("tx");
    sqlx::query("LOCK TABLE run_resets IN SHARE MODE")
        .execute(&mut *held)
        .await
        .expect("the entries held back");
    let waiting = |pool: sqlx::PgPool, n: i64| async move {
        for _ in 0..3_000 {
            let blocked: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                   AND wait_event_type = 'Lock' AND query NOT LIKE '%pg_stat_activity%'",
            )
            .fetch_one(&pool)
            .await
            .expect("activity");
            if blocked >= n {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    };
    let context = ctx();
    let first = w.runs.reset(&w.admin, source, request(), &key, &context);
    let controller = async {
        assert!(
            waiting(w.db.pool.clone(), 1).await,
            "the first request waits to write its entry"
        );
        let _other = w.start(workflow, &json!({"accountId": "acct-1"})).await;
        let retry = w.runs.reset(&w.admin, source, request(), &key, &context);
        let release = async {
            // the retry waits for the source's row (its receipt and the window are read under it), or it answered at once
            let _ = waiting(w.db.pool.clone(), 2).await;
            held.commit().await.expect("released");
        };
        tokio::join!(retry, release).0
    };
    let (first, again) = tokio::join!(first, controller);
    let first = first.expect("accepted: one slot of two was free");
    let again = again.expect("the retry answers the first request's entry, not the full window");
    assert_eq!(again.reset.id, first.reset.id);
}
/// Batch 5 correctness Major 1 and security suspicion 2 (decision D022-121): a held signal whose delivery fails (here a
/// trigger refuses its `run.signalled` audit record, as any failure of the delivery would) no longer fails the grant of
/// its start, which would end the key's pass on every pass: the grant is tried again without it, the signal dropped with
/// its audit record (`DELIVERY_FAILED`, naming the caller who sent it), and the start granted.
#[tokio::test]
async fn a_held_signal_whose_delivery_fails_is_dropped_and_the_start_is_granted() {
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-signal-undeliverable")).await;
    let first = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("the first holds the permit");
    let queued = signal_with_start(&w, workflow, "cart-2")
        .await
        .expect("created and held");
    let run = queued.run.info.run_id;
    assert!(queued.held, "{queued:?}");
    for statement in [
        format!(
            "CREATE FUNCTION test_refuse_signalled() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
               IF NEW.resource_id = '{run}' AND NEW.action = 'run.signalled' THEN \
                 RAISE EXCEPTION 'the test refuses this delivery'; END IF; RETURN NEW; END $$"
        ),
        "CREATE TRIGGER test_refuse_signalled BEFORE INSERT ON audit_records FOR EACH ROW \
           EXECUTE FUNCTION test_refuse_signalled()"
            .to_owned(),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&w.db.pool)
            .await
            .expect("the refusing trigger");
    }
    w.operate_run(
        first.run.info.run_id,
        sagat_application::runs::RunOperation::Cancel,
    )
    .await;
    let report = w.scheduler.grant_gates().await;
    sqlx::query("DROP TRIGGER test_refuse_signalled ON audit_records")
        .execute(&w.db.pool)
        .await
        .expect("the trigger dropped");
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(start_rows(&w, run).await, (1, 0), "granted");
    let (held, dropped) = held_and_audited(&w, run, "run.signal_dropped").await;
    assert_eq!(held, 0, "the held row is gone");
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    let reason = dropped[0].as_deref().unwrap_or_default();
    assert!(
        reason.starts_with("DELIVERY_FAILED") && reason.contains(w.admin.subject().as_str()),
        "dropped for its delivery, naming its sender: {reason}"
    );
    assert_eq!(held_and_audited(&w, run, "run.signalled").await.1.len(), 0);
    assert_eq!(
        w.step_status(run, "w").await,
        Some(sagat_domain::execution::StepStatus::Waiting),
        "nothing was delivered"
    );
    w.assert_consistent(run).await;
}

/// Batch 5 correctness minor 3 (decision D022-087): `verify_run` holds that a signal is held only while the run's start
/// waits in its queue. A held row kept for a run whose start was granted (here written by hand beside a run that took its
/// permit at creation) is a breach.
#[tokio::test]
async fn verify_run_refuses_a_held_signal_kept_for_a_run_whose_start_does_not_wait() {
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-held-stray")).await;
    let done = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("started and signalled");
    let run = done.run.info.run_id;
    assert_eq!(admitted(&w, run).await, Admitted::Acquired);
    w.assert_consistent(run).await;
    sqlx::query(
        "INSERT INTO flow_control_held_signals (tenant_id, namespace_id, run_id, name, payload_digest, actor, held_at) \
         SELECT r.tenant_id, r.namespace_id, r.id, 'payment.confirmed', p.digest, 'admin', now() \
           FROM runs r JOIN payloads p ON p.tenant_id = r.tenant_id WHERE r.id = $1 LIMIT 1",
    )
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("a stray held row");
    let verdict = w.store.verify_run(w.admin.tenant_id(), run).await;
    assert!(
        verdict
            .as_ref()
            .is_err_and(|breach| breach.contains("held signal")),
        "{verdict:?}"
    );
}

/// Batch 5 security minor 1 (FR-022-060 as amended, task T022-106 for signals): the signal a Signal-with-Start held with
/// its queued run is delivered by the grant under the name of the caller who sent it, without checking `run:signal`
/// again: a caller demoted to a role without it while the start waits still has its signal delivered, audited as its own.
#[tokio::test]
async fn a_held_signal_is_delivered_without_reauthorization_and_keeps_its_sender() {
    use sagat_domain::identity::Role;
    let w = gated_world().await;
    let workflow = publish(&w, gated_waiter("creators-held-reauthorize")).await;
    let first = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("the first holds the permit");
    // an operator holds `run:start` and `run:signal`; a viewer holds neither
    let carol = w.member("carol", Role::Operator).await;
    w.bind_everything(&carol).await;
    let key = idempotency();
    let queued = w
        .runs
        .signal_with_start(
            &carol,
            &request(workflow, br#"{"k": "s", "accountId": "acct-1"}"#, &key),
            business("cart-2"),
            SignalAtStart {
                name: "payment.confirmed",
                payload: br#"{"paid": true}"#,
            },
            &ctx(),
        )
        .await
        .expect("created and held");
    let run = queued.run.info.run_id;
    assert!(queued.held, "{queued:?}");
    let demoted = sqlx::query(
        "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = 'carol'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("demoted")
    .rows_affected();
    // the demotion took effect (re-read correctness finding 6, decision D022-130 item 4): one membership row changed,
    // the identity store reads carol back as a viewer, and a signal carol sends herself now is refused
    assert_eq!(demoted, 1, "carol's one membership row");
    let membership = sagat_application::identity::IdentityStore::find_membership(
        &PgIdentityStore::new(w.db.pool.clone()),
        w.admin.tenant_id(),
        carol.subject(),
    )
    .await
    .expect("read")
    .expect("carol is a member");
    assert_eq!(membership.role(), Role::Viewer, "{membership:?}");
    let as_viewer = sagat_domain::identity::Principal::new(
        w.admin.tenant_id(),
        carol.subject().clone(),
        membership.role(),
    );
    let direct = w
        .runs
        .signal(
            &as_viewer,
            run,
            "payment.confirmed",
            br#"{"paid": true}"#,
            &idempotency(),
            &ctx(),
        )
        .await;
    assert!(
        matches!(direct, Err(AppError::Forbidden { .. })),
        "a viewer may not signal: {direct:?}"
    );
    w.operate_run(
        first.run.info.run_id,
        sagat_application::runs::RunOperation::Cancel,
    )
    .await;
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    assert_eq!(
        w.step_status(run, "w").await,
        Some(sagat_domain::execution::StepStatus::Succeeded),
        "delivered: {:?}",
        w.journal(run).await
    );
    let signalled_by: String = sqlx::query_scalar(
        "SELECT actor FROM audit_records WHERE resource_id = $1 AND action = 'run.signalled'",
    )
    .bind(run.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("the actor");
    assert_eq!(signalled_by, "carol", "audited as its sender's");
    w.assert_consistent(run).await;
}

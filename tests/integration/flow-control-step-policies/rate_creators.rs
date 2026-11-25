//! The run creators of a definition whose start declares `rateLimit` (task T022-072; FR-022-041, FR-022-042,
//! FR-022-044, FR-022-047; plan "Run creators (rate limits)"): an event trigger, a cron trigger, a schedule's action, a
//! child node and a delayed start each admit the run through the one admission point, so its first decision journals
//! `rate_limit.granted` with a grant row or `rate_limit.queued` with a waiter row; a start that declares both gates is
//! granted its permit and its slot together from the queue, or times out on both. Every instant that must pass is waited
//! for on the database clock with a bound (`until_past`), never by a sleep.

use sagat_application::events::PublishRequest;
use sagat_application::schedule_firing::Fired;
use sagat_domain::definition::WorkflowId;
use sagat_domain::events::EventId;
use sagat_domain::execution::{RunId, RunStatus};
use sagat_domain::quota::QuotaResource;
use sagat_domain::schedules::Ceilings;
use sagat_domain::workflow::EventType;
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::run_admission::{age_start_deadline, set, start_rows};
use crate::support::engine::{World, activity, child_node, ctx, document};
use crate::support::flow_control::{gated_world, publish, until_past};
use crate::support::schedules::{fixture_over, hourly};

const SECOND_US: i64 = 1_000_000;

/// How a creator admitted a run of a rate-limited definition: the decision its first commit journals, checked against
/// the run's own grant and waiter rows and against the consistency check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admitted {
    Granted,
    Buffered,
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

async fn admitted(w: &World, run: RunId) -> Admitted {
    let journal = w.journal(run).await;
    let decision = journal
        .iter()
        .find_map(|event| match event.as_str() {
            "rate_limit.granted" => Some(Admitted::Granted),
            "rate_limit.queued" => Some(Admitted::Buffered),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the run was not admitted through its key: {journal:?}"));
    let rows = slot_rows(w, run).await;
    match decision {
        Admitted::Granted => assert_eq!(rows, (1, 0), "a grant row: {journal:?}"),
        Admitted::Buffered => assert_eq!(rows, (0, 1), "a waiter row: {journal:?}"),
    }
    w.assert_consistent(run).await;
    decision
}

async fn runs_of(w: &World, workflow: WorkflowId) -> Vec<RunId> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM runs WHERE workflow_id = $1 ORDER BY created_at, id")
            .bind(workflow.as_uuid())
            .fetch_all(&w.db.pool)
            .await
            .expect("runs");
    ids.into_iter().map(RunId::new).collect()
}

/// A one-step workflow `name` whose start takes a slot of `key` (the group's single key when `None`), `limit` per `per`.
fn rated(name: &str, key: Option<&str>, limit: u32, per: &str) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["rateLimit"] = match key {
        Some(key) => json!({"group": "accounts", "key": key, "limit": limit, "per": per}),
        None => json!({"group": "accounts", "limit": limit, "per": per}),
    };
    doc
}

/// The grant instant of the run's slot, in microseconds.
async fn granted_us(w: &World, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT (extract(epoch FROM granted_at) * 1000000)::bigint FROM rate_limit_grants \
          WHERE run_id = $1 AND limit_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the run's grant")
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

/// An event trigger (FR-022-041): the run an event starts is admitted through the key its envelope names; an event whose
/// run the key refuses (its waiters are at the key's bound: `RATE_LIMIT_QUEUE_FULL`) starts nothing and stays in the
/// inbox, and its redelivery starts the run once the key has room.
#[tokio::test]
async fn an_event_trigger_admits_a_rate_limited_start_and_retries_an_event_the_key_refuses() {
    let w = gated_world().await;
    let mut doc = rated(
        "rate-creators-event",
        Some("input.data.accountId"),
        1,
        "hour",
    );
    doc["triggers"] = json!([{"kind": "event", "eventType": "order.created"}]);
    let workflow = publish(&w, doc).await;
    set(&w, QuotaResource::FlowControlKeyWaiters, 1).await;
    for id in ["rate-creators-e1", "rate-creators-e2"] {
        publish_event(&w, id, &json!({"accountId": "acct-1"})).await;
        w.dispatcher.tick().await;
    }
    let runs = runs_of(&w, workflow).await;
    assert_eq!(runs.len(), 2);
    assert_eq!(admitted(&w, runs[0]).await, Admitted::Granted);
    assert_eq!(admitted(&w, runs[1]).await, Admitted::Buffered);

    publish_event(&w, "rate-creators-e3", &json!({"accountId": "acct-1"})).await;
    w.dispatcher.tick().await;
    assert_eq!(runs_of(&w, workflow).await.len(), 2, "the key refused it");
    let pending: String =
        sqlx::query_scalar("SELECT status FROM inbox_events WHERE event_id = 'rate-creators-e3'")
            .fetch_one(&w.db.pool)
            .await
            .expect("the event");
    assert_eq!(pending, "PENDING", "the event is tried again");

    set(&w, QuotaResource::FlowControlKeyWaiters, 10).await;
    sqlx::query(
        "UPDATE inbox_events SET next_attempt_at = clock_timestamp() WHERE event_id = 'rate-creators-e3'",
    )
    .execute(&w.db.pool)
    .await
    .expect("due again");
    w.dispatcher.tick().await;
    let runs = runs_of(&w, workflow).await;
    assert_eq!(runs.len(), 3, "the retry starts it");
    assert_eq!(admitted(&w, runs[2]).await, Admitted::Buffered);
}

/// A cron trigger (the legacy cron pass): the run of a tick is admitted through its key; a declaration that names no key
/// has one key for the group, so the second tick is buffered behind the first.
#[tokio::test]
async fn a_cron_trigger_admits_a_rate_limited_start() {
    let w = gated_world().await;
    let mut doc = rated("rate-creators-cron", None, 1, "hour");
    doc["triggers"] = json!([{"kind": "cron", "cron": "*/5 * * * *", "timezone": "UTC"}]);
    let workflow = publish(&w, doc).await;
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
    assert_eq!(admitted(&w, runs[0]).await, Admitted::Granted);
    assert_eq!(admitted(&w, runs[1]).await, Admitted::Buffered);
}

/// A schedule's action (feature 018): the run of a nominal time is admitted through its key; behind a holder of the key
/// started by the API, it is buffered.
#[tokio::test]
async fn a_schedule_action_admits_a_rate_limited_start() {
    let w = gated_world().await;
    let mut doc = document("rate-creators-scheduled", "g", vec![activity("g")]);
    doc["rateLimit"] =
        json!({"group": "regions", "key": "input.region", "limit": 1, "per": "hour"});
    let workflow = publish(&w, doc).await;
    let f = fixture_over(w, workflow, OffsetDateTime::now_utc(), Ceilings::default());
    let holder = f.w.start(f.workflow, &json!({"region": "eu"})).await;
    assert_eq!(admitted(&f.w, holder).await, Admitted::Granted);
    let schedule = f
        .create(
            "rate-creators-hourly",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "the action");
    let runs: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM runs WHERE schedule_id = $1")
        .bind(schedule.id.as_uuid())
        .fetch_all(&f.w.db.pool)
        .await
        .expect("runs");
    assert_eq!(runs.len(), 1, "one action");
    assert_eq!(
        admitted(&f.w, RunId::new(runs[0])).await,
        Admitted::Buffered
    );
}

/// A child node (D022-031): the child's run is admitted through its key like a start of the API and is buffered in the
/// same queue; once the slot ahead of it has aged out (`limit: 1, per: second`), the next pass grants it.
#[tokio::test]
async fn a_child_run_of_a_rate_limited_definition_is_buffered_and_granted_from_the_queue() {
    let w = gated_world().await;
    let target = publish(
        &w,
        rated("rate-child-target", Some("input.accountId"), 1, "second"),
    )
    .await;
    let holder = w.start(target, &json!({"accountId": "acct-1"})).await;
    // the child's input is its parent's (the child node carries no input of its own)
    let parent = publish(
        &w,
        document(
            "rate-child-parent",
            "c",
            vec![child_node("c", "rate-child-target", None)],
        ),
    )
    .await;
    let parent_run = w.start(parent, &json!({"accountId": "acct-1"})).await;
    w.scheduler.tick().await;
    let child: Uuid = sqlx::query_scalar("SELECT id FROM runs WHERE parent_run_id = $1")
        .bind(parent_run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the child run exists");
    let child = RunId::new(child);
    assert_eq!(w.journal(child).await, ["run.started", "rate_limit.queued"]);
    assert_eq!(admitted(&w, child).await, Admitted::Buffered);
    until_past(&w.db.pool, granted_us(&w, holder).await + SECOND_US).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(slot_rows(&w, child).await, (1, 0), "granted from the queue");
    for run in [holder, child, parent_run] {
        w.assert_consistent(run).await;
    }
}

/// A start delayed by feature 018 (FR-022-018, D022-052): while its delay runs the run holds no slot and waits in no
/// queue; the release decides its key in the release's transaction: a slot when the key has room, a wait behind the
/// holder when it has not, and a key that does not evaluate ends the run `FAILED`/`EXPRESSION_ERROR` there.
#[tokio::test]
async fn a_delayed_rate_limited_start_is_decided_at_its_release() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        rated("rate-creators-delay", Some("input.accountId"), 1, "hour"),
    )
    .await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let free = crate::run_creators::start_delayed(&w, workflow, Some("acct-2")).await;
    let behind = crate::run_creators::start_delayed(&w, workflow, Some("acct-1")).await;
    let no_key = crate::run_creators::start_delayed(&w, workflow, None).await;
    for run in [free, behind, no_key] {
        assert_eq!(w.journal(run).await, ["run.started", "run.start_delayed"]);
        assert_eq!(slot_rows(&w, run).await, (0, 0), "nothing while delayed");
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
            "rate_limit.granted",
            "step.released"
        ]
    );
    assert_eq!(slot_rows(&w, free).await, (1, 0));
    assert_eq!(
        w.journal(behind).await,
        [
            "run.started",
            "run.start_delayed",
            "run.start_released",
            "rate_limit.queued"
        ]
    );
    assert_eq!(slot_rows(&w, behind).await, (0, 1));
    assert_eq!(w.run_status(behind).await, RunStatus::Waiting);
    assert_eq!(w.run_status(no_key).await, RunStatus::Failed);
    let code: Option<String> = sqlx::query_scalar("SELECT stop_code FROM runs WHERE id = $1")
        .bind(no_key.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("run");
    assert_eq!(code.as_deref(), Some("EXPRESSION_ERROR"));
    assert_eq!(slot_rows(&w, no_key).await, (0, 0));
    for run in [holder, free, behind, no_key] {
        w.assert_consistent(run).await;
    }
}

/// D022-006, FR-022-044 (correctness review of task T022-072, minor 6): a start that waits for both its permit and its
/// slot is granted both from the queue in one commit: after its holder ends (its permit free) and its slot has aged out
/// (`limit: 1, per: second`), the next pass grants the waiter its permit and its slot, each journaled once. (That no
/// pass grants one gate without the other is `queue::a_dual_gated_head_is_not_starved_by_a_younger_waiter` and
/// `rate_gate::a_dual_gated_start_takes_its_permit_and_slot_together_or_waits_for_both`.) A waiter of `limit: 1, per:
/// hour` behind a run that holds both, past its one deadline, times out once, rate first: its slot's wait ends `rate_limit.wait_timed_out` and its permit's
/// `concurrency.dequeued`, and the run fails `RATE_LIMIT_WAIT_TIMEOUT` with no row left.
#[tokio::test]
async fn a_dual_gated_start_is_granted_both_from_the_queue_or_times_out_rate_first() {
    let w = gated_world().await;
    let mut doc = rated("rate-dual-queue", Some("input.accountId"), 1, "second");
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    let workflow = publish(&w, doc).await;
    let session = w.session(&["act.g"]).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(
        w.journal(waiter).await,
        ["run.started", "rate_limit.queued", "concurrency.queued"]
    );
    // the holder ends at once: its permit is free, its slot counts for a second from its grant
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder);
    w.complete(&session, &task, &json!({})).await.expect("done");
    assert_eq!(w.run_status(holder).await, RunStatus::Succeeded);
    let granted = granted_us(&w, holder).await;
    until_past(&w.db.pool, granted + SECOND_US).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(start_rows(&w, waiter).await, (1, 0), "the permit");
    assert_eq!(
        slot_rows(&w, waiter).await,
        (1, 0),
        "and the slot, in one commit"
    );
    let journal = w.journal(waiter).await;
    for event in ["rate_limit.granted", "concurrency.acquired"] {
        assert_eq!(
            journal.iter().filter(|e| e.as_str() == event).count(),
            1,
            "{event} once: {journal:?}"
        );
    }

    // a waiter behind a run that holds both gates for an hour, past its one deadline: both refuse it, rate first
    let mut hourly_doc = rated("rate-dual-timeout", Some("input.accountId"), 1, "hour");
    hourly_doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    let hourly_workflow = publish(&w, hourly_doc).await;
    let holding = w
        .start(hourly_workflow, &json!({"accountId": "acct-2"}))
        .await;
    let late = w
        .start(hourly_workflow, &json!({"accountId": "acct-2"}))
        .await;
    assert_eq!(
        (start_rows(&w, late).await, slot_rows(&w, late).await),
        ((0, 1), (0, 1))
    );
    age_start_deadline(&w, late).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.gate_waits_timed_out, 1, "{report:?}");
    assert_eq!(w.run_status(late).await, RunStatus::Failed);
    let code: Option<String> = sqlx::query_scalar("SELECT stop_code FROM runs WHERE id = $1")
        .bind(late.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("run");
    assert_eq!(code.as_deref(), Some("RATE_LIMIT_WAIT_TIMEOUT"));
    let journal = w.journal(late).await;
    for event in ["rate_limit.wait_timed_out", "concurrency.dequeued"] {
        assert_eq!(
            journal.iter().filter(|e| e.as_str() == event).count(),
            1,
            "{event} once: {journal:?}"
        );
    }
    assert_eq!(
        (start_rows(&w, late).await, slot_rows(&w, late).await),
        ((0, 0), (0, 0))
    );
    for run in [holder, waiter, holding, late] {
        w.assert_consistent(run).await;
    }
}

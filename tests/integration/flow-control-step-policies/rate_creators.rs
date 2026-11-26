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

// ------------------------------------------------------------------------------------------- the rate twins (T022-113)
// The creators of task T022-064 that `run_creators` proves for a permit, each proved here for a slot (FR-022-041;
// decision D022-069 item 7): the run is admitted through its key, `rate_limit.granted` with a grant row or
// `rate_limit.queued` with a waiter row, and the consistency check agrees (`admitted`).

/// `doc` with its start's `concurrency` replaced by a `rateLimit` of the same key, `limit` per hour.
fn hourly_instead(mut doc: Value, limit: u32) -> Value {
    doc.as_object_mut()
        .expect("a document")
        .remove("concurrency");
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": limit, "per": "hour"});
    doc
}

/// Row 1 (`POST /runs` with feature 018's options): a start under a business identifier takes the slot while the key's
/// window has room and is buffered behind it when it has not.
#[tokio::test]
async fn the_start_api_with_a_business_identifier_admits_a_rate_limited_start() {
    use crate::run_creators::{business, idempotency, request};
    use sagat_application::runs::StartOptions;

    let w = gated_world().await;
    let workflow = publish(
        &w,
        rated("rate-twin-api", Some("input.accountId"), 1, "hour"),
    )
    .await;
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
    assert_eq!(admitted(&w, runs[0]).await, Admitted::Granted);
    assert_eq!(admitted(&w, runs[1]).await, Admitted::Buffered);
}

/// Row 4 (`RERUN_AS_NEW`): the rerun of an ended run is a new run that takes a slot of its own: with `limit: 2, per:
/// hour`, the source's slot and a holder's fill the window, and the rerun is buffered.
#[tokio::test]
async fn a_rerun_as_new_of_a_rate_limited_run_admits_through_the_key() {
    use crate::run_creators::{complete_all, idempotency};

    let w = gated_world().await;
    let workflow = publish(
        &w,
        rated("rate-twin-rerun", Some("input.accountId"), 2, "hour"),
    )
    .await;
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
    assert_eq!(admitted(&w, holder).await, Admitted::Granted);
    assert_eq!(admitted(&w, rerun).await, Admitted::Buffered);
}

/// Row 3 (Signal-with-Start): a run whose key's window has room takes its slot and receives the signal in its creation
/// transaction; a second one on the full window is buffered and its signal held with it (task T022-109, decision
/// D022-087), and the grant of its slot, once the first slot leaves the window, delivers it.
#[tokio::test]
async fn a_signal_with_start_of_a_rate_limited_definition_takes_its_slot_or_holds_its_signal() {
    use crate::run_creators::{gated_waiter, signal_with_start};

    let w = gated_world().await;
    let workflow = publish(&w, hourly_instead(gated_waiter("rate-twin-signal"), 1)).await;
    let done = signal_with_start(&w, workflow, "cart-1")
        .await
        .expect("started and signalled");
    assert!(done.started && !done.held);
    assert_eq!(admitted(&w, done.run.info.run_id).await, Admitted::Granted);
    let delivered: Vec<&str> = done.delivered_to.iter().map(|n| n.as_str()).collect();
    assert_eq!(delivered, ["w"]);
    let held = signal_with_start(&w, workflow, "cart-2")
        .await
        .expect("buffered and held");
    assert!(
        held.started && held.held && held.delivered_to.is_empty(),
        "{held:?}"
    );
    let run = held.run.info.run_id;
    assert_eq!(admitted(&w, run).await, Admitted::Buffered);
    // the first slot two hours old, out of its window (the guard of a grant row off in this one transaction)
    let mut tx = w.db.pool.begin().await.expect("tx");
    for statement in [
        "ALTER TABLE rate_limit_grants DISABLE TRIGGER USER",
        "UPDATE rate_limit_grants SET granted_at = granted_at - interval '2 hours', \
                leased_at = leased_at - interval '2 hours' WHERE limit_scope = 'RUN'",
        "ALTER TABLE rate_limit_grants ENABLE TRIGGER USER",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect(statement);
    }
    tx.commit().await.expect("aged");
    assert_eq!(w.scheduler.grant_gates().await.gates_granted, 1);
    assert_eq!(
        w.step_status(run, "w").await,
        Some(sagat_domain::execution::StepStatus::Succeeded),
        "{:?}",
        w.journal(run).await
    );
    let (rows, signalled): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM flow_control_held_signals WHERE run_id = $1), \
                (SELECT count(*) FROM audit_records WHERE resource_id = $1::text AND action = 'run.signalled')",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows");
    assert_eq!((rows, signalled), (0, 1), "delivered once by the grant");
    // not `assert_consistent`: the first grant was aged in its rows only, so its journal still places it in the window
}

/// Row 3 (Update-with-Start): the run takes its slot or is buffered, and the Update is applied to its declared state in
/// its creation transaction either way.
#[tokio::test]
async fn an_update_with_start_of_a_rate_limited_definition_admits_through_the_key() {
    use crate::run_creators::{declared, update_with_start};

    let w = gated_world().await;
    let workflow = publish(&w, hourly_instead(declared("rate-twin-update"), 1)).await;
    let first = update_with_start(&w, workflow, "acct-a", 20)
        .await
        .expect("started");
    assert!(first.started);
    assert_eq!(admitted(&w, first.run.info.run_id).await, Admitted::Granted);
    let second = update_with_start(&w, workflow, "acct-b", 30)
        .await
        .expect("started");
    assert!(second.started);
    let run = second.run.info.run_id;
    assert_eq!(admitted(&w, run).await, Admitted::Buffered);
    assert_eq!(
        w.journal(run).await,
        ["run.started", "rate_limit.queued", "state.updated"],
        "the Update is applied while the run waits"
    );
}

/// Row 4 (a batch `RERUN`): the rerun a job makes is a new run admitted through its key: buffered behind the source's
/// and a holder's slots of `limit: 2, per: hour`.
#[tokio::test]
async fn a_batch_rerun_of_a_rate_limited_run_admits_through_the_key() {
    use crate::run_creators::{complete_all, rerun_the_succeeded_by_a_batch};

    let w = std::sync::Arc::new(gated_world().await);
    let workflow = publish(
        &w,
        rated("rate-twin-batch", Some("input.accountId"), 2, "hour"),
    )
    .await;
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
        crate::run_creators::batch_state(&w).await
    );
    assert_eq!(admitted(&w, holder).await, Admitted::Granted);
    assert_eq!(admitted(&w, runs[2]).await, Admitted::Buffered);
}

/// Row 13 (an ad hoc database workflow, and its private rerun): a caller's ad hoc document whose start declares
/// `rateLimit` takes a slot like a published one; the second is buffered, and so is the first's private rerun (a
/// cancelled run's slot still counts for its window).
#[tokio::test]
async fn an_ad_hoc_rate_limited_workflow_and_its_private_rerun_admit_through_the_key() {
    use crate::run_creators::{ad_hoc_connection, idempotency, start_ad_hoc};
    use sagat_application::runs::RunOperation;

    let w = gated_world().await;
    ad_hoc_connection(&w).await;
    let mut doc = document(
        "rate-twin-ad-hoc",
        "query",
        vec![json!({
            "id": "query", "type": "database", "connectionRef": {"name": "creators-db"},
            "operation": "query", "statement": "SELECT 1::bigint"
        })],
    );
    doc["rateLimit"] = json!({"group": "reports", "limit": 1, "per": "hour"});
    let bytes = serde_json::to_vec(&doc).expect("json");
    let first = start_ad_hoc(&w, &bytes, "rate-twin-ad-hoc-0001").await;
    let second = start_ad_hoc(&w, &bytes, "rate-twin-ad-hoc-0002").await;
    assert_eq!(admitted(&w, first).await, Admitted::Granted);
    assert_eq!(admitted(&w, second).await, Admitted::Buffered);
    w.operate_run(first, RunOperation::Cancel).await;
    assert_eq!(w.run_status(first).await, RunStatus::Cancelled);
    let rerun = w
        .runs
        .rerun(
            &w.admin,
            first,
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
    assert_eq!(admitted(&w, rerun.info.run_id).await, Admitted::Buffered);
}

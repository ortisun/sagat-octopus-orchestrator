//! US02, US03 and the delivery promises: an event starts the runs of the triggers it matches and
//! wakes the steps that wait for it, each exactly once, however the dispatcher is interrupted,
//! repeated or raced (FR-004-001, FR-004-003).

use std::time::Duration;

use sagat_application::dispatch::EventDispatchStore;
use sagat_application::events::PublishRequest;
use sagat_application::quotas::QuotaLookup as _;
use sagat_domain::events::EventId;
use sagat_domain::execution::{CorrelationKey, RunId, RunStatus, StepStatus};
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::EventType;
use serde_json::{Value, json};

use crate::support::engine::{World, activity, ctx, document, wait_node, with, world};

/// A workflow with one activity, started by `triggers`.
fn triggered(name: &str, triggers: Vec<Value>) -> Value {
    let mut doc = document(name, "work", vec![activity("work")]);
    doc["triggers"] = Value::Array(triggers);
    doc
}

/// A workflow that waits for `payment.confirmed` for the order in its input.
fn waiting(name: &str, lookback_ms: u64) -> Value {
    let mut wait = wait_node("pay", "payment.confirmed", "input.orderId", 300_000, None);
    if lookback_ms > 0 {
        wait = with(wait, "lookbackMs", json!(lookback_ms));
    }
    document(
        name,
        "pay",
        vec![with(wait, "next", json!("work")), activity("work")],
    )
}

async fn publish_event(
    w: &World,
    id: &str,
    kind: &str,
    correlation: Option<&str>,
    data: &Value,
) -> sagat_application::events::Received {
    w.events
        .publish(
            &w.admin,
            PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(EventId::parse(id).expect("id")),
                event_type: EventType::parse(kind).expect("type"),
                subject: Some(format!("subject/{id}")),
                correlation_key: correlation.map(|c| CorrelationKey::parse(c).expect("key")),
                occurred_at: None,
                data: serde_json::to_vec(data).expect("json"),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("published")
}

async fn runs_of_workflow(w: &World, name: &str) -> Vec<RunId> {
    sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT r.id FROM runs r JOIN workflows f ON f.id = r.workflow_id \
         WHERE f.name = $1 ORDER BY r.id",
    )
    .bind(name)
    .fetch_all(&w.db.pool)
    .await
    .expect("runs")
    .into_iter()
    .map(RunId::new)
    .collect()
}

async fn event_status(w: &World, id: &str) -> (String, i32, Option<String>) {
    sqlx::query_as("SELECT status, attempts, last_error FROM inbox_events WHERE event_id = $1")
        .bind(id)
        .fetch_one(&w.db.pool)
        .await
        .expect("status")
}

async fn deliveries(w: &World, id: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT consumer FROM event_deliveries WHERE event_id = $1 ORDER BY consumer",
    )
    .bind(id)
    .fetch_all(&w.db.pool)
    .await
    .expect("deliveries")
}

async fn input_of(w: &World, run: RunId) -> Value {
    let content: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM runs r JOIN payloads p \
           ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest WHERE r.id = $1",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("input");
    serde_json::from_slice(&content).expect("json")
}

#[tokio::test]
async fn an_event_starts_the_run_of_a_matching_trigger_once_and_gives_it_the_event() {
    let w = world().await;
    w.publish(triggered(
        "on-order",
        vec![json!({"kind": "event", "eventType": "order.created"})],
    ))
    .await;

    publish_event(
        &w,
        "o-1",
        "order.created",
        Some("order-7"),
        &json!({"amount": 150}),
    )
    .await;
    let round = w.dispatcher.tick().await;
    assert_eq!((round.dispatched, round.runs_started), (1, 1));

    let runs = runs_of_workflow(&w, "on-order").await;
    assert_eq!(runs.len(), 1);
    assert_eq!(w.run_status(runs[0]).await, RunStatus::Running);
    let input = input_of(&w, runs[0]).await;
    assert_eq!(input["data"], json!({"amount": 150}));
    assert_eq!(input["event"]["id"], "o-1");
    assert_eq!(input["event"]["source"], "api");
    assert_eq!(input["event"]["type"], "order.created");
    assert_eq!(input["event"]["subject"], "subject/o-1");
    assert_eq!(input["event"]["correlationKey"], "order-7");
    assert_eq!(event_status(&w, "o-1").await.0, "DISPATCHED");
    assert_eq!(deliveries(&w, "o-1").await.len(), 1);
    w.assert_consistent(runs[0]).await;

    // Nothing is left to do, and the same event published again is a duplicate: one run.
    assert!(!w.dispatcher.tick().await.did_work());
    assert!(
        publish_event(
            &w,
            "o-1",
            "order.created",
            Some("order-7"),
            &json!({"amount": 150})
        )
        .await
        .duplicate
    );
    w.dispatcher.tick().await;
    assert_eq!(runs_of_workflow(&w, "on-order").await.len(), 1);
}

#[tokio::test]
async fn dispatching_an_event_again_after_a_crash_starts_nothing_new() {
    let w = world().await;
    w.publish(triggered(
        "on-order",
        vec![json!({"kind": "event", "eventType": "order.created"})],
    ))
    .await;
    publish_event(&w, "o-1", "order.created", None, &json!({"amount": 1})).await;
    w.dispatcher.tick().await;

    // The dispatcher died after starting the run and before marking the event: it is claimed again.
    sqlx::query("UPDATE inbox_events SET status = 'PENDING', dispatched_at = NULL, next_attempt_at = clock_timestamp()")
        .execute(&w.db.pool)
        .await
        .expect("rewind");
    let round = w.dispatcher.tick().await;
    assert_eq!(round.dispatched, 1, "the event is finished this time");
    assert_eq!(
        round.runs_started, 0,
        "and nothing was started for it again"
    );
    assert_eq!(runs_of_workflow(&w, "on-order").await.len(), 1);
    assert_eq!(deliveries(&w, "o-1").await.len(), 1);
}

#[tokio::test]
async fn a_filter_decides_which_events_start_a_run() {
    let w = world().await;
    w.publish(triggered(
        "big-orders",
        vec![json!({
            "kind": "event", "eventType": "order.created",
            "filter": "data.amount > 100 && event.subject == 'subject/big'"
        })],
    ))
    .await;

    publish_event(&w, "small", "order.created", None, &json!({"amount": 5})).await;
    publish_event(&w, "big", "order.created", None, &json!({"amount": 500})).await;
    publish_event(&w, "no-amount", "order.created", None, &json!({"other": 1})).await;
    let round = w.dispatcher.tick().await;
    assert_eq!(
        round.dispatched, 3,
        "an event no trigger wants is dispatched all the same"
    );
    assert_eq!(
        round.runs_started, 1,
        "only the one that passes both conditions"
    );
    assert_eq!(runs_of_workflow(&w, "big-orders").await.len(), 1);
    assert_eq!(deliveries(&w, "big").await.len(), 1);
    assert!(deliveries(&w, "small").await.is_empty());
    assert_eq!(
        event_status(&w, "no-amount").await.0,
        "DISPATCHED",
        "a filter that cannot be evaluated does not match, and does not poison the event"
    );
}

#[tokio::test]
async fn a_webhook_trigger_is_about_its_endpoint_and_an_event_trigger_about_a_type() {
    let (w, endpoint) = crate::with_endpoint().await;
    w.publish(triggered(
        "from-billing",
        vec![json!({"kind": "webhook", "endpoint": "billing"})],
    ))
    .await;
    w.publish(triggered(
        "any-paid",
        vec![json!({"kind": "event", "eventType": "invoice.paid"})],
    ))
    .await;
    w.publish(triggered(
        "api-only",
        vec![json!({"kind": "event", "eventType": "invoice.paid", "source": "api"})],
    ))
    .await;

    let signed = crate::Signed::typed(
        crate::SECRET,
        crate::now_seconds(),
        "w-1",
        "invoice.paid",
        r#"{"n":1}"#,
    );
    w.events
        .receive(endpoint.id, &signed.request(Some("invoice.paid")))
        .await
        .expect("webhook");
    publish_event(&w, "a-1", "invoice.paid", None, &json!({"n": 2})).await;
    publish_event(&w, "a-2", "invoice.voided", None, &json!({"n": 3})).await;
    w.dispatcher.tick().await;

    // The webhook event: `from-billing` (its endpoint) and `any-paid` (any source), not `api-only`.
    // The API event: `any-paid` and `api-only`. The voided one: nobody.
    assert_eq!(runs_of_workflow(&w, "from-billing").await.len(), 1);
    assert_eq!(runs_of_workflow(&w, "any-paid").await.len(), 2);
    assert_eq!(runs_of_workflow(&w, "api-only").await.len(), 1);
    assert_eq!(deliveries(&w, "a-2").await.len(), 0);
}

#[tokio::test]
async fn the_triggers_in_force_are_those_of_the_active_version() {
    let w = world().await;
    let (workflow, _) = w
        .publish(triggered(
            "shifting",
            vec![json!({"kind": "event", "eventType": "order.created"})],
        ))
        .await;
    let trigger_ids = |w: &World| {
        let pool = w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM workflow_triggers ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("triggers")
        }
    };
    let first = trigger_ids(&w).await;
    assert_eq!(first.len(), 1);

    // A version without triggers: the workflow is no longer started by the event.
    w.publish_new_version(workflow, triggered("shifting", vec![]))
        .await;
    assert!(trigger_ids(&w).await.is_empty());
    publish_event(&w, "o-1", "order.created", None, &json!({})).await;
    assert_eq!(w.dispatcher.tick().await.runs_started, 0);

    // A later version brings it back with the same identity (so an event handled before cannot
    // start a second run under a new id), and a filter edit does not change the identity either.
    w.publish_new_version(
        workflow,
        triggered(
            "shifting",
            vec![json!({"kind": "event", "eventType": "order.created", "filter": "data.vip == true"})],
        ),
    )
    .await;
    assert_eq!(
        trigger_ids(&w).await,
        first,
        "the id depends on what the trigger is about"
    );
    let filter: Option<String> = sqlx::query_scalar("SELECT filter FROM workflow_triggers")
        .fetch_one(&w.db.pool)
        .await
        .expect("filter");
    assert_eq!(filter.as_deref(), Some("data.vip == true"));
}

#[tokio::test]
async fn an_event_wakes_the_step_that_waits_for_it_and_gives_it_the_event() {
    let w = world().await;
    let (workflow, _) = w.publish(waiting("payment-flow", 0)).await;
    let run = w.start(workflow, &json!({"orderId": "ord-1"})).await;
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Waiting));

    // An event for another order, and one of another type, wake nothing.
    publish_event(
        &w,
        "e-other",
        "payment.confirmed",
        Some("ord-2"),
        &json!({"n": 0}),
    )
    .await;
    publish_event(
        &w,
        "e-type",
        "payment.refunded",
        Some("ord-1"),
        &json!({"n": 0}),
    )
    .await;
    w.dispatcher.tick().await;
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Waiting));

    publish_event(
        &w,
        "e-pay",
        "payment.confirmed",
        Some("ord-1"),
        &json!({"paid": 99}),
    )
    .await;
    let round = w.dispatcher.tick().await;
    assert_eq!(round.steps_woken, 1);
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Succeeded));
    assert_eq!(
        deliveries(&w, "e-pay").await,
        vec![format!("wait:{run}/pay")]
    );

    let output: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM steps s JOIN payloads p \
           ON p.tenant_id = s.tenant_id AND p.digest = s.output_digest \
         WHERE s.run_id = $1 AND s.node = 'pay'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("output");
    let output: Value = serde_json::from_slice(&output).expect("json");
    assert_eq!(output["data"], json!({"paid": 99}));
    assert_eq!(output["event"]["id"], "e-pay");
    w.assert_consistent(run).await;

    // The run goes on from the wait, and the audit trail says what woke it.
    assert!(w.journal(run).await.contains(&"wait.satisfied".to_owned()));
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.signalled' AND resource_id = $1",
    )
    .bind(run.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audited, 1);
}

#[tokio::test]
async fn an_event_that_came_before_the_wait_is_delivered_only_within_its_lookback() {
    let w = world().await;
    let (without, _) = w.publish(waiting("no-lookback", 0)).await;
    let (with, _) = w.publish(waiting("with-lookback", 60_000)).await;

    // The event is admitted and dispatched while nobody waits for it.
    publish_event(
        &w,
        "early",
        "payment.confirmed",
        Some("ord-1"),
        &json!({"n": 1}),
    )
    .await;
    w.dispatcher.tick().await;
    assert_eq!(event_status(&w, "early").await.0, "DISPATCHED");

    let plain = w.start(without, &json!({"orderId": "ord-1"})).await;
    let looking_back = w.start(with, &json!({"orderId": "ord-1"})).await;
    let round = w.dispatcher.tick().await;
    assert_eq!(
        round.steps_woken, 1,
        "only the wait that asked to look back"
    );
    assert_eq!(w.step_status(plain, "pay").await, Some(StepStatus::Waiting));
    assert_eq!(
        w.step_status(looking_back, "pay").await,
        Some(StepStatus::Succeeded)
    );
    assert_eq!(
        deliveries(&w, "early").await,
        vec![format!("wait:{looking_back}/pay")]
    );

    // Looking again finds nothing more: the event is delivered to a wait once.
    assert!(!w.dispatcher.tick().await.did_work());
}

#[tokio::test]
async fn an_event_that_arrives_while_the_matcher_is_looking_is_not_lost() {
    // The matcher found no wait for the event (the wait had not committed yet) and finished it. The
    // wait, then, must find the event: this is the race the second direction of the search closes.
    let w = world().await;
    let (workflow, _) = w.publish(waiting("payment-flow", 0)).await;
    let run = w.start(workflow, &json!({"orderId": "ord-1"})).await;
    publish_event(
        &w,
        "raced",
        "payment.confirmed",
        Some("ord-1"),
        &json!({"n": 1}),
    )
    .await;
    sqlx::query("UPDATE inbox_events SET status = 'DISPATCHED', dispatched_at = clock_timestamp()")
        .execute(&w.db.pool)
        .await
        .expect("the matcher finished it without seeing the wait");
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Waiting));

    let round = w.dispatcher.tick().await;
    assert_eq!(round.steps_woken, 1);
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Succeeded));
}

#[tokio::test]
async fn an_event_goes_to_every_wait_with_its_key_once_each() {
    let w = world().await;
    let (workflow, _) = w.publish(waiting("payment-flow", 0)).await;
    let mut runs = Vec::new();
    for _ in 0..3 {
        runs.push(w.start(workflow, &json!({"orderId": "shared"})).await);
    }
    let elsewhere = w.start(workflow, &json!({"orderId": "other"})).await;

    publish_event(
        &w,
        "fan",
        "payment.confirmed",
        Some("shared"),
        &json!({"n": 1}),
    )
    .await;
    assert_eq!(w.dispatcher.tick().await.steps_woken, 3);
    for run in &runs {
        assert_eq!(
            w.step_status(*run, "pay").await,
            Some(StepStatus::Succeeded)
        );
    }
    assert_eq!(
        w.step_status(elsewhere, "pay").await,
        Some(StepStatus::Waiting)
    );
    assert_eq!(deliveries(&w, "fan").await.len(), 3);

    // Dispatched again (a lost claim): no step is woken twice, and the deliveries do not grow.
    sqlx::query("UPDATE inbox_events SET status = 'PENDING', dispatched_at = NULL, next_attempt_at = clock_timestamp()")
        .execute(&w.db.pool)
        .await
        .expect("rewind");
    let round = w.dispatcher.tick().await;
    assert_eq!((round.dispatched, round.steps_woken), (1, 0));
    assert_eq!(deliveries(&w, "fan").await.len(), 3);
}

#[tokio::test]
async fn an_event_with_more_waits_than_a_page_reaches_all_of_them_in_one_dispatch() {
    let w = world().await;
    let (workflow, _) = w.publish(waiting("payment-flow", 0)).await;
    let mut runs = Vec::new();
    for _ in 0..7 {
        runs.push(w.start(workflow, &json!({"orderId": "shared"})).await);
    }
    let small_pages = w.dispatcher_with(sagat_scheduler::EventDispatchSettings {
        fan_out: 3,
        ..sagat_scheduler::EventDispatchSettings::default()
    });
    publish_event(&w, "fan", "payment.confirmed", Some("shared"), &json!({})).await;
    let round = small_pages.tick().await;
    assert_eq!(
        (round.dispatched, round.steps_woken, round.retried),
        (1, 7, 0)
    );
    for run in runs {
        assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Succeeded));
    }
    assert_eq!(deliveries(&w, "fan").await.len(), 7);
}

#[tokio::test]
async fn a_wait_that_timed_out_is_not_woken_by_a_late_event_and_the_event_is_not_recorded_as_delivered()
 {
    let w = world().await;
    let mut wait = wait_node(
        "pay",
        "payment.confirmed",
        "input.orderId",
        300_000,
        Some("late"),
    );
    wait["next"] = json!("work");
    let (workflow, _) = w
        .publish(document(
            "timeout-flow",
            "pay",
            vec![wait, activity("work"), activity("late")],
        ))
        .await;
    let run = w.start(workflow, &json!({"orderId": "ord-1"})).await;

    w.age_due(run, "pay").await;
    w.scheduler.tick().await;
    assert!(w.journal(run).await.contains(&"wait.timed_out".to_owned()));

    publish_event(
        &w,
        "late-event",
        "payment.confirmed",
        Some("ord-1"),
        &json!({"n": 1}),
    )
    .await;
    let round = w.dispatcher.tick().await;
    assert_eq!(round.steps_woken, 0);
    assert!(!w.journal(run).await.contains(&"wait.satisfied".to_owned()));
    assert!(
        deliveries(&w, "late-event").await.is_empty(),
        "no delivery is claimed for a wake-up that did not happen"
    );
    assert_eq!(event_status(&w, "late-event").await.0, "DISPATCHED");
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn the_event_and_the_timeout_racing_resolve_to_exactly_one_of_them() {
    for round in 0..8 {
        let w = world().await;
        let mut wait = wait_node(
            "pay",
            "payment.confirmed",
            "input.orderId",
            300_000,
            Some("late"),
        );
        wait["next"] = json!("work");
        let (workflow, _) = w
            .publish(document(
                "race-flow",
                "pay",
                vec![wait, activity("work"), activity("late")],
            ))
            .await;
        let run = w.start(workflow, &json!({"orderId": "ord-1"})).await;
        publish_event(
            &w,
            "e",
            "payment.confirmed",
            Some("ord-1"),
            &json!({"n": round}),
        )
        .await;
        w.age_due(run, "pay").await;

        let (_, _) = tokio::join!(w.scheduler.tick(), w.dispatcher.tick());
        // Whatever ran second may need another look to settle what the first left.
        w.scheduler.tick().await;
        w.dispatcher.tick().await;

        let journal = w.journal(run).await;
        let woken = journal.contains(&"wait.satisfied".to_owned());
        let timed_out = journal.contains(&"wait.timed_out".to_owned());
        let delivered = deliveries(&w, "e").await.len();
        assert!(
            woken ^ timed_out,
            "round {round}: exactly one of the two won: {journal:?}"
        );
        assert_eq!(
            delivered,
            usize::from(woken),
            "round {round}: delivered if and only if it woke"
        );
        w.assert_consistent(run).await;
    }
}

#[tokio::test]
async fn events_never_cross_a_tenant_boundary() {
    let w = world().await;
    w.publish(triggered(
        "on-order",
        vec![json!({"kind": "event", "eventType": "order.created"})],
    ))
    .await;
    let (waiter, _) = w.publish(waiting("payment-flow", 0)).await;
    let run = w.start(waiter, &json!({"orderId": "ord-1"})).await;

    // The other tenant has a namespace of the same name; its events are its own.
    for (id, kind, key) in [
        ("x-1", "order.created", None),
        ("x-2", "payment.confirmed", Some("ord-1")),
    ] {
        w.events
            .publish(
                &w.other_admin,
                PublishRequest {
                    namespace: "payments".to_owned(),
                    event_id: Some(EventId::parse(id).expect("id")),
                    event_type: EventType::parse(kind).expect("type"),
                    subject: None,
                    correlation_key: key.map(|k| CorrelationKey::parse(k).expect("key")),
                    occurred_at: None,
                    data: b"{}".to_vec(),
                    dedupe_ttl: None,
                },
                &ctx(),
            )
            .await
            .expect("published");
    }
    let round = w.dispatcher.tick().await;
    assert_eq!(round.dispatched, 2);
    assert_eq!((round.runs_started, round.steps_woken), (0, 0));
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Waiting));
    assert!(runs_of_workflow(&w, "on-order").await.is_empty());
}

#[tokio::test]
async fn a_failing_dispatch_is_retried_with_backoff_and_then_set_aside_where_it_can_be_seen() {
    let w = world().await;
    w.publish(triggered(
        "on-order",
        vec![json!({"kind": "event", "eventType": "order.created"})],
    ))
    .await;
    // No room for another active run in the namespace: starting the run fails, and keeps failing.
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            0,
            &ctx(),
        )
        .await
        .expect("limit");
    assert_eq!(
        w.store
            .effective_limit(
                w.admin.tenant_id(),
                Some(w.namespace.id()),
                QuotaResource::ActiveRuns
            )
            .await,
        Ok(0)
    );
    publish_event(&w, "stuck", "order.created", None, &json!({})).await;

    let round = w.dispatcher.tick().await;
    assert_eq!((round.retried, round.dispatched), (1, 0));
    let (status, attempts, error) = event_status(&w, "stuck").await;
    assert_eq!(
        (status.as_str(), attempts),
        ("PENDING", 0),
        "waiting for room is not a failure to count"
    );
    let error = error.expect("the reason is kept");
    assert!(error.contains("quota"), "{error}");
    assert!(
        !error.contains("order.created"),
        "the reason names no payload or id: {error}"
    );
    let wait: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM next_attempt_at - clock_timestamp())::float8 FROM inbox_events",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("wait");
    assert!(wait > 0.0, "it is not retried at once: {wait}");
    assert!(
        !w.dispatcher.tick().await.did_work(),
        "and not claimed before its time"
    );

    // Room appears: the next attempt, when its time comes, succeeds.
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            100,
            &ctx(),
        )
        .await
        .expect("limit");
    sqlx::query("UPDATE inbox_events SET next_attempt_at = clock_timestamp()")
        .execute(&w.db.pool)
        .await
        .expect("time passes");
    let round = w.dispatcher.tick().await;
    assert_eq!((round.dispatched, round.runs_started), (1, 1));
}

#[tokio::test]
async fn an_event_waiting_for_room_is_patient_for_a_day_and_then_set_aside_with_its_payload() {
    let w = world().await;
    w.publish(triggered(
        "on-order",
        vec![json!({"kind": "event", "eventType": "order.created"})],
    ))
    .await;
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            0,
            &ctx(),
        )
        .await
        .expect("limit");
    publish_event(&w, "waiting", "order.created", None, &json!({"keep": "me"})).await;

    // However many times it is tried, a lack of room does not use the event up.
    for _ in 0..15 {
        sqlx::query("UPDATE inbox_events SET next_attempt_at = clock_timestamp()")
            .execute(&w.db.pool)
            .await
            .expect("time passes");
        let round = w.dispatcher.tick().await;
        assert_eq!((round.retried, round.dead), (1, 0));
    }
    let (status, attempts, _) = event_status(&w, "waiting").await;
    assert_eq!((status.as_str(), attempts), ("PENDING", 0));

    // A day without room is long enough: it is set aside, whole, where a person can see it.
    sqlx::query(
        "UPDATE inbox_events SET received_at = clock_timestamp() - interval '25 hours', \
                next_attempt_at = clock_timestamp()",
    )
    .execute(&w.db.pool)
    .await
    .expect("a day passes");
    let round = w.dispatcher.tick().await;
    assert_eq!(round.dead, 1);
    let (status, _, error) = event_status(&w, "waiting").await;
    assert_eq!(status, "DEAD");
    assert!(error.expect("a reason").contains("quota"));
    let payload: Vec<u8> =
        sqlx::query_scalar("SELECT payload FROM inbox_events WHERE event_id = 'waiting'")
            .fetch_one(&w.db.pool)
            .await
            .expect("payload");
    assert_eq!(payload, br#"{"keep":"me"}"#, "kept for inspection");
    assert!(
        !w.dispatcher.tick().await.did_work(),
        "and never claimed again"
    );
}

#[tokio::test]
async fn an_event_that_fails_for_another_reason_is_set_aside_after_its_attempts() {
    let w = world().await;
    w.publish(triggered(
        "on-order",
        vec![json!({"kind": "event", "eventType": "order.created"})],
    ))
    .await;
    publish_event(&w, "doomed", "order.created", None, &json!({"keep": "me"})).await;
    // Something the platform cannot do for this event: the table that remembers starts is gone.
    sqlx::query("ALTER TABLE command_dedupe RENAME TO command_dedupe_away")
        .execute(&w.db.pool)
        .await
        .expect("break the store");
    sqlx::query("UPDATE inbox_events SET attempts = 11 WHERE event_id = 'doomed'")
        .execute(&w.db.pool)
        .await
        .expect("attempts");
    let round = w.dispatcher.tick().await;
    assert_eq!(round.dead, 1);
    let (status, attempts, error) = event_status(&w, "doomed").await;
    assert_eq!((status.as_str(), attempts), ("DEAD", 12));
    assert!(error.is_some());
}

#[tokio::test]
async fn one_consumer_that_cannot_be_served_does_not_keep_the_others_waiting() {
    let w = world().await;
    w.publish(triggered(
        "on-payment",
        vec![json!({"kind": "event", "eventType": "payment.confirmed"})],
    ))
    .await;
    let (waiter, _) = w.publish(waiting("payment-flow", 0)).await;
    let run = w.start(waiter, &json!({"orderId": "ord-1"})).await;
    // The dispatcher has looked for what the wait missed, so only the event itself can wake it.
    w.dispatcher.tick().await;
    // No room for a run started by the trigger; the wait is not about that.
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            1,
            &ctx(),
        )
        .await
        .expect("limit");
    publish_event(
        &w,
        "paid",
        "payment.confirmed",
        Some("ord-1"),
        &json!({"n": 1}),
    )
    .await;

    let round = w.dispatcher.tick().await;
    assert_eq!(
        (round.steps_woken, round.runs_started, round.retried),
        (1, 0, 1),
        "the wait was woken though the trigger could not start its run"
    );
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Succeeded));
    assert_eq!(
        deliveries(&w, "paid").await,
        vec![format!("wait:{run}/pay")]
    );

    // Room appears; the retry starts the run that is missing and wakes nothing twice.
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            100,
            &ctx(),
        )
        .await
        .expect("limit");
    sqlx::query("UPDATE inbox_events SET next_attempt_at = clock_timestamp()")
        .execute(&w.db.pool)
        .await
        .expect("time passes");
    let round = w.dispatcher.tick().await;
    assert_eq!(
        (round.dispatched, round.runs_started, round.steps_woken),
        (1, 1, 0)
    );
    assert_eq!(deliveries(&w, "paid").await.len(), 2);
}

#[tokio::test]
async fn a_claimed_event_is_not_claimed_by_another_dispatcher_until_its_lease_ends() {
    let w = world().await;
    publish_event(&w, "claimed", "order.created", None, &json!({})).await;
    let first = w
        .store
        .claim_events(10, Duration::from_secs(60))
        .await
        .expect("claim");
    assert_eq!(first.len(), 1);
    assert!(
        w.store
            .claim_events(10, Duration::from_secs(60))
            .await
            .expect("claim")
            .is_empty(),
        "a second dispatcher does not see it"
    );
    sqlx::query(
        "UPDATE inbox_events SET next_attempt_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("the lease runs out");
    assert_eq!(
        w.store
            .claim_events(10, Duration::from_secs(60))
            .await
            .expect("claim")
            .len(),
        1,
        "the dispatcher that died holding it is replaced"
    );
}

#[tokio::test]
async fn expired_events_are_purged_but_one_still_waiting_to_be_dispatched_is_kept() {
    let w = world().await;
    publish_event(&w, "done", "order.created", None, &json!({})).await;
    publish_event(&w, "stuck", "order.created", None, &json!({})).await;
    sqlx::query("UPDATE inbox_events SET status = 'DISPATCHED', dispatched_at = clock_timestamp() WHERE event_id = 'done'")
        .execute(&w.db.pool)
        .await
        .expect("dispatch");
    sqlx::query("UPDATE inbox_events SET expires_at = clock_timestamp() - interval '1 second'")
        .execute(&w.db.pool)
        .await
        .expect("expire");
    w.dispatcher.housekeeping().await;
    let left: Vec<String> = sqlx::query_scalar("SELECT event_id FROM inbox_events")
        .fetch_all(&w.db.pool)
        .await
        .expect("left");
    assert_eq!(left, vec!["stuck".to_owned()]);

    // An event id that is still pending outlives its window, and is not a new event meanwhile.
    let again = publish_event(&w, "stuck", "order.created", None, &json!({})).await;
    assert!(again.duplicate);
}

#[tokio::test]
async fn an_id_admitted_again_after_its_window_is_a_new_event_and_starts_its_run_again() {
    let w = world().await;
    w.publish(triggered(
        "on-order",
        vec![json!({"kind": "event", "eventType": "order.created"})],
    ))
    .await;
    publish_event(&w, "nightly", "order.created", None, &json!({"n": 1})).await;
    w.dispatcher.tick().await;
    assert_eq!(runs_of_workflow(&w, "on-order").await.len(), 1);

    // The window ends; the provider uses the same id for the next night's event.
    sqlx::query(
        "UPDATE inbox_events SET expires_at = clock_timestamp() - interval '1 second' \
         WHERE event_id = 'nightly'",
    )
    .execute(&w.db.pool)
    .await
    .expect("the window ends");
    let again = publish_event(&w, "nightly", "order.created", None, &json!({"n": 1})).await;
    assert!(!again.duplicate, "after its window an id is a new event");
    let round = w.dispatcher.tick().await;
    assert_eq!(
        (round.dispatched, round.runs_started),
        (1, 1),
        "and the new event starts its own run"
    );
    assert_eq!(runs_of_workflow(&w, "on-order").await.len(), 2);

    // Dispatching the same admission again (a crash before it was marked done) still starts nothing
    // new: the second run is the second admission's, once.
    sqlx::query("UPDATE inbox_events SET status = 'PENDING', dispatched_at = NULL, next_attempt_at = clock_timestamp() WHERE event_id = 'nightly'")
        .execute(&w.db.pool)
        .await
        .expect("as if the mark was lost");
    sqlx::query("DELETE FROM event_deliveries WHERE event_id = 'nightly'")
        .execute(&w.db.pool)
        .await
        .expect("and the record with it");
    w.dispatcher.tick().await;
    assert_eq!(runs_of_workflow(&w, "on-order").await.len(), 2);
}

#[tokio::test]
async fn a_wait_that_began_before_the_events_migration_does_not_stop_the_search_for_the_others() {
    let w = world().await;
    let (old, _) = w.publish(waiting("old-wait", 0)).await;
    let (with, _) = w.publish(waiting("with-lookback", 60_000)).await;
    let before = w.start(old, &json!({"orderId": "ord-0"})).await;
    // As it is in a database that was waiting before this migration: no instant to start from, and
    // never searched for.
    sqlx::query("UPDATE steps SET event_since = NULL, event_scanned_at = NULL WHERE run_id = $1")
        .bind(before.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("a wait as feature 002 left it");

    publish_event(
        &w,
        "early",
        "payment.confirmed",
        Some("ord-1"),
        &json!({"n": 1}),
    )
    .await;
    w.dispatcher.tick().await;
    let looking_back = w.start(with, &json!({"orderId": "ord-1"})).await;
    let round = w.dispatcher.tick().await;
    assert_eq!(
        round.steps_woken, 1,
        "the wait that asks to look back finds its event whatever the old one looks like"
    );
    assert_eq!(
        w.step_status(looking_back, "pay").await,
        Some(StepStatus::Succeeded)
    );
    assert_eq!(
        w.step_status(before, "pay").await,
        Some(StepStatus::Waiting)
    );
}

#[tokio::test]
async fn the_migration_gives_the_waits_it_finds_a_start_and_marks_them_searched() {
    let w = world().await;
    let (old, _) = w.publish(waiting("old-wait", 0)).await;
    let before = w.start(old, &json!({"orderId": "ord-0"})).await;
    sqlx::query("UPDATE steps SET event_since = NULL, event_scanned_at = NULL WHERE run_id = $1")
        .bind(before.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("a wait as feature 002 left it");

    // The statement of the migration itself, run on what it was written for.
    let migration =
        include_str!("../../../crates/infra-postgres/migrations/20261012000000_events.up.sql");
    let backfill = migration
        .split(';')
        .map(str::trim)
        .find(|statement| statement.contains("UPDATE steps SET event_since"))
        .expect("the migration backfills the waits that exist");
    sqlx::query(backfill)
        .execute(&w.db.pool)
        .await
        .expect("the backfill runs");

    let (since, scanned): (Option<time::OffsetDateTime>, Option<time::OffsetDateTime>) =
        sqlx::query_as(
            "SELECT event_since, event_scanned_at FROM steps WHERE run_id = $1 AND node = 'pay'",
        )
        .bind(before.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("row");
    assert!(since.is_some(), "it has an instant to receive events from");
    assert!(
        scanned.is_some(),
        "and there is no inbox from before the migration to search"
    );
    // And an event that arrives now still wakes it.
    publish_event(
        &w,
        "payment-0",
        "payment.confirmed",
        Some("ord-0"),
        &json!({"n": 1}),
    )
    .await;
    assert_eq!(w.dispatcher.tick().await.steps_woken, 1);
}

#[tokio::test]
async fn two_sources_may_use_the_same_event_id_at_the_same_instant_and_each_starts_its_run() {
    let w = world().await;
    w.publish(triggered(
        "on-order",
        vec![json!({"kind": "event", "eventType": "order.created"})],
    ))
    .await;
    publish_event(&w, "same-id", "order.created", None, &json!({"n": 1})).await;
    // The same id from another source, admitted at the very same microsecond.
    sqlx::query(
        "INSERT INTO inbox_events (tenant_id, source, event_id, namespace_id, event_type, \
            correlation_key, subject, occurred_at, payload, payload_digest, received_at, \
            expires_at, status, attempts, next_attempt_at) \
         SELECT tenant_id, 'billing', event_id, namespace_id, event_type, correlation_key, \
            subject, occurred_at, payload, payload_digest, received_at, expires_at, status, \
            attempts, next_attempt_at FROM inbox_events WHERE source = 'api'",
    )
    .execute(&w.db.pool)
    .await
    .expect("a second source");
    let round = w.dispatcher.tick().await;
    assert_eq!((round.dispatched, round.runs_started), (2, 2));
    assert_eq!(runs_of_workflow(&w, "on-order").await.len(), 2);
}

#[tokio::test]
async fn an_event_that_was_admitted_before_a_wait_began_does_not_wake_it_from_the_events_side() {
    let w = world().await;
    let (without, _) = w.publish(waiting("no-lookback", 0)).await;
    // Admitted, and not yet dispatched, when the wait begins.
    publish_event(
        &w,
        "early",
        "payment.confirmed",
        Some("ord-1"),
        &json!({"n": 1}),
    )
    .await;
    let run = w.start(without, &json!({"orderId": "ord-1"})).await;
    let round = w.dispatcher.tick().await;
    assert_eq!(round.dispatched, 1);
    assert_eq!(
        round.steps_woken, 0,
        "it predates the wait and the wait did not ask to look back"
    );
    assert_eq!(w.step_status(run, "pay").await, Some(StepStatus::Waiting));
}

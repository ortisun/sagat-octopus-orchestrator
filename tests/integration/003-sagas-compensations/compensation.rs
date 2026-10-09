//! US02: a saga undoes what it did, in reverse order, with ordinary steps (FR-003-001, -002, -004).

use sagat_application::execution::compensation_key;
use sagat_application::runs::RunStore;
use sagat_application::worker::TaskKind;
use sagat_domain::execution::{RunStatus, StepStatus};
use sagat_domain::workflow::NodeId;
use serde_json::{Value, json};

use crate::support::engine::{
    activity, document, effect_activities, effect_node, parallel_node, saga_node, with, world,
};
use crate::{quick, settlement, worker};

fn node(id: &str) -> NodeId {
    NodeId::parse_stored(id).expect("a node id")
}

#[tokio::test]
async fn a_permanent_failure_after_a_reservation_compensates_it_and_the_run_ends_compensated() {
    let w = world().await;
    let (workflow, version) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({ "order": "o-1" })).await;

    let reserve = w.one_task(&s).await;
    assert_eq!(
        (reserve.activity.as_str(), reserve.kind),
        ("act.reserve", TaskKind::Forward)
    );
    w.complete(&s, &reserve, &json!({ "reservation": "r-1" }))
        .await
        .expect("reserve");
    let submit = w.one_task(&s).await;
    assert_eq!(submit.activity.as_str(), "act.submit");
    w.fail(&s, &submit, false)
        .await
        .expect("submit fails for good");

    // The failure took the run into the compensation, in the same transaction.
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
    assert_eq!(
        w.step_status(run, "settlement").await,
        Some(StepStatus::Waiting)
    );
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        Some(StepStatus::Ready)
    );
    assert_eq!(
        w.step_status(run, "submit~c").await,
        None,
        "submit never completed, there is nothing to undo"
    );

    // The compensation is an ordinary task of the same worker, with the data it needs.
    let undo = w.one_task(&s).await;
    assert_eq!(
        (undo.activity.as_str(), undo.kind),
        ("undo.reserve", TaskKind::Compensation)
    );
    assert_eq!(undo.node.as_str(), "reserve~c");
    let input: Value = serde_json::from_slice(&undo.input).expect("json");
    assert_eq!(input["steps"]["reserve"]["output"]["reservation"], "r-1");
    // It is about the forward step, and carries the key of that step's own attempts.
    let target = undo
        .target
        .clone()
        .expect("a compensation names its target");
    assert_eq!(target.node.as_str(), "reserve");
    assert_eq!(target.idempotency_key, reserve.idempotency_key);
    // Its own key is stable per run, step and action, and is not the forward step's.
    assert_eq!(
        undo.idempotency_key,
        compensation_key(version, run, &node("reserve"))
    );
    assert_ne!(undo.idempotency_key, reserve.idempotency_key);

    w.complete(&s, &undo, &json!({})).await.expect("undo");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    assert_eq!(
        w.step_status(run, "settlement").await,
        Some(StepStatus::Failed)
    );
    assert_eq!(
        w.step_status(run, "notify").await,
        None,
        "nothing after the saga ran"
    );

    // The journal says what happened, in order, and the projections equal its replay.
    let names = w.journal(run).await;
    let at = |name: &str| {
        names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("{name} in {names:?}"))
    };
    assert!(
        at("step.failed") < at("saga.aborting")
            && at("saga.aborting") < at("compensation.planned")
            && at("compensation.planned") < at("saga.compensated")
            && at("saga.compensated") < at("run.finished"),
        "{names:?}"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn compensations_run_in_reverse_order_and_each_waits_for_the_one_after_it() {
    let w = world().await;
    let (workflow, _) = w
        .publish(document(
            "booking",
            "trip",
            vec![
                saga_node("trip", "flight", None),
                effect_node("flight", Some("hotel"), false),
                effect_node("hotel", Some("car"), false),
                effect_node("car", None, false),
            ],
        ))
        .await;
    let names = effect_activities(&["flight", "hotel", "car"]);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = w.session(&names).await;
    let run = w.start(workflow, &json!({})).await;

    for expected in ["act.flight", "act.hotel"] {
        let task = w.one_task(&s).await;
        assert_eq!(task.activity.as_str(), expected);
        w.complete(&s, &task, &json!({})).await.expect("forward");
    }
    let car = w.one_task(&s).await;
    w.fail(&s, &car, false)
        .await
        .expect("the car fails for good");

    // Last done, first undone: the hotel, then the flight, and only one at a time.
    let first = w.one_task(&s).await;
    assert_eq!(first.activity.as_str(), "undo.hotel");
    assert_eq!(
        w.step_status(run, "flight~c").await,
        Some(StepStatus::Pending)
    );
    assert!(
        w.poll(&s, 8).await.is_empty(),
        "the flight waits for the hotel"
    );
    w.complete(&s, &first, &json!({}))
        .await
        .expect("undo hotel");
    let second = w.one_task(&s).await;
    assert_eq!(second.activity.as_str(), "undo.flight");
    w.complete(&s, &second, &json!({}))
        .await
        .expect("undo flight");

    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn independent_branches_are_compensated_at_the_same_time() {
    let w = world().await;
    let (workflow, _) = w
        .publish(document(
            "fanout",
            "shop",
            vec![
                saga_node("shop", "split", None),
                with(
                    parallel_node("split", &["card", "stock"], "all"),
                    "next",
                    json!("ship"),
                ),
                effect_node("card", None, false),
                effect_node("stock", None, false),
                activity("ship"),
            ],
        ))
        .await;
    let names = {
        let mut n = effect_activities(&["card", "stock"]);
        n.push("act.ship".to_owned());
        n
    };
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = w.session(&names).await;
    let run = w.start(workflow, &json!({})).await;

    for task in w.poll(&s, 8).await {
        w.complete(&s, &task, &json!({})).await.expect("branch");
    }
    let ship = w.one_task(&s).await;
    assert_eq!(ship.activity.as_str(), "act.ship");
    w.fail(&s, &ship, false).await.expect("fails for good");

    // Neither branch depends on the other, so both undo tasks are ready now.
    let mut undone: Vec<String> = w
        .poll(&s, 8)
        .await
        .iter()
        .map(|t| t.activity.as_str().to_owned())
        .collect();
    undone.sort();
    assert_eq!(undone, ["undo.card", "undo.stock"]);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_failure_inside_the_saga_that_something_handles_does_not_compensate() {
    let w = world().await;
    // `reserve` fails and the body routes the failure to `recover`: still inside the saga.
    let (workflow, _) = w
        .publish(document(
            "handled",
            "tx",
            vec![
                saga_node("tx", "reserve", Some("notify")),
                with(
                    effect_node("reserve", None, false),
                    "onError",
                    json!("recover"),
                ),
                activity("recover"),
                activity("notify"),
            ],
        ))
        .await;
    let names = effect_activities(&["reserve"]);
    let mut names: Vec<&str> = names.iter().map(String::as_str).collect();
    names.extend(["act.recover", "act.notify"]);
    let s = w.session(&names).await;
    let run = w.start(workflow, &json!({})).await;

    let reserve = w.one_task(&s).await;
    w.fail(&s, &reserve, false).await.expect("fails");
    let journal = w.journal(run).await;
    assert!(
        !journal.iter().any(|n| n == "saga.aborting"),
        "the body handled it: {journal:?}"
    );
    for expected in ["act.recover", "act.notify"] {
        let task = w.one_task(&s).await;
        assert_eq!(task.activity.as_str(), expected);
        w.complete(&s, &task, &json!({})).await.expect("goes on");
    }
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_saga_that_completes_just_continues() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    for expected in ["act.reserve", "act.submit", "act.notify"] {
        let task = w.one_task(&s).await;
        assert_eq!(task.activity.as_str(), expected);
        w.complete(&s, &task, &json!({})).await.expect("forward");
    }
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    assert!(
        w.journal(run).await.iter().any(|n| n == "saga.completed"),
        "the saga recorded that it completed"
    );
    // Nothing was planned, and no compensation step exists.
    let compensations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM steps WHERE run_id = $1 AND kind <> 'FORWARD'")
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(compensations, 0);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_compensation_that_fails_is_retried_with_its_own_budget() {
    let w = world().await;
    let (workflow, _) = w.publish(quick(settlement())).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("fails");

    let undo = w.one_task(&s).await;
    w.fail(&s, &undo, true).await.expect("the provider is down");
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        Some(StepStatus::RetryWait)
    );
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);

    w.age_retry(run, "reserve~c").await;
    w.scheduler.tick().await;
    let again = w.one_task(&s).await;
    assert_eq!(again.activity.as_str(), "undo.reserve");
    // Another attempt of the same compensation: same key, a fence of its own.
    assert_eq!(again.idempotency_key, undo.idempotency_key);
    assert_eq!(again.fence.attempt, undo.fence.attempt + 1);
    w.complete(&s, &again, &json!({})).await.expect("works now");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn the_derived_steps_are_stored_with_their_kind_and_the_database_agrees() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("fails");

    let kinds: Vec<(String, String)> =
        sqlx::query_as("SELECT node, kind FROM steps WHERE run_id = $1 ORDER BY node")
            .bind(run.as_uuid())
            .fetch_all(&w.db.pool)
            .await
            .expect("steps");
    let kind_of = |n: &str| {
        kinds
            .iter()
            .find(|(node, _)| node == n)
            .map(|(_, k)| k.as_str())
    };
    assert_eq!(kind_of("reserve"), Some("FORWARD"));
    assert_eq!(kind_of("reserve~c"), Some("COMPENSATION"));
    // The database refuses a step whose kind does not match its id.
    let wrong =
        sqlx::query("UPDATE steps SET kind = 'FORWARD' WHERE run_id = $1 AND node = 'reserve~c'")
            .bind(run.as_uuid())
            .execute(&w.db.pool)
            .await;
    assert!(wrong.is_err(), "kind and node id must agree");
}

#[tokio::test]
async fn another_tenant_cannot_see_the_compensation_of_a_run() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("fails");

    let seen = w
        .store
        .get(w.other_admin.tenant_id(), run)
        .await
        .expect("get");
    assert!(seen.is_none(), "the run does not exist for another tenant");
    let journal = w
        .store
        .journal(w.other_admin.tenant_id(), run, None, 100)
        .await
        .expect("journal");
    assert!(journal.is_none());
}

#[tokio::test]
async fn the_end_of_a_compensated_run_is_announced_with_its_status() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("fails");
    assert_eq!(
        w.outbox("run.finished").await,
        0,
        "not announced while it is being undone"
    );
    let undo = w.one_task(&s).await;
    w.complete(&s, &undo, &json!({})).await.expect("undo");

    let payloads: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM event_outbox WHERE event_type = 'run.finished' AND aggregate_id = $1",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("outbox");
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["status"], "COMPENSATED");
}

#[tokio::test]
async fn a_compensation_that_answers_with_a_large_output_does_not_fail_because_the_run_is_full() {
    use sagat_application::execution::MAX_RUN_OUTPUT_BYTES;
    let w = world().await;
    let big = json!({ "blob": "x".repeat(240_000) });
    let len = serde_json::to_vec(&big).expect("json").len();
    let fits = MAX_RUN_OUTPUT_BYTES / len;
    // The forward steps fill the budget the run may retain, then the last one fails.
    let mut nodes = vec![saga_node("tx", "s0", None)];
    for i in 0..=fits {
        let next = (i < fits).then(|| format!("s{}", i + 1));
        nodes.push(effect_node(&format!("s{i}"), next.as_deref(), false));
    }
    let (workflow, _) = w.publish(document("full", "tx", nodes)).await;
    let ids: Vec<String> = (0..=fits).map(|i| format!("s{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let names = effect_activities(&refs);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = w.session(&names).await;
    let run = w.start(workflow, &json!({})).await;
    for _ in 0..fits {
        let task = w.one_task(&s).await;
        w.complete(&s, &task, &big)
            .await
            .expect("within the budget");
    }
    let last = w.one_task(&s).await;
    w.fail(&s, &last, false).await.expect("fails for good");

    // Every undo answers as much as the forward steps did: none of it is kept, so none of it can
    // push the run over its budget and turn a compensation that worked into a failure.
    let mut undone = 0;
    while w.run_status(run).await == RunStatus::Compensating {
        for task in w.poll(&s, 8).await {
            assert_eq!(task.kind, sagat_application::worker::TaskKind::Compensation);
            w.complete(&s, &task, &big).await.expect("undo");
            undone += 1;
        }
    }
    assert_eq!(undone, fits);
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_poll_for_one_task_returns_one_even_when_a_run_has_several_ready() {
    let w = world().await;
    let (workflow, _) = w
        .publish(document(
            "wide",
            "tx",
            vec![
                saga_node("tx", "split", None),
                parallel_node("split", &["a", "b", "c"], "all"),
                effect_node("a", None, false),
                effect_node("b", None, false),
                effect_node("c", None, false),
            ],
        ))
        .await;
    let names = effect_activities(&["a", "b", "c"]);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = w.session(&names).await;
    let run = w.start(workflow, &json!({})).await;
    // Three steps of one run are ready at once; each poll for one task takes exactly one.
    for left in (0..3).rev() {
        let tasks = w.poll(&s, 1).await;
        assert_eq!(tasks.len(), 1, "asked for one, got {}", tasks.len());
        assert_eq!(tasks[0].run_id, run);
        let ready: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM steps WHERE run_id = $1 AND status = 'READY' AND kind = 'FORWARD' \
             AND node IN ('a', 'b', 'c')",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
        assert_eq!(ready, left, "the others were not leased");
    }
}

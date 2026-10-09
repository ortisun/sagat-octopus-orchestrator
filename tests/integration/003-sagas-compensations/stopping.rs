//! Cancelling a run and its deadline, with a saga around the work in flight (FR-003-004).

use sagat_application::runs::RunOperation;
use sagat_application::worker::TaskKind;
use sagat_domain::execution::{RunStatus, StepStatus};
use serde_json::json;

use crate::support::engine::{World, document, effect_node, saga_node, task_ref, with, world};
use crate::{settlement, worker};

/// `settlement` with a cancel policy on the saga.
fn with_policy(policy: &str) -> serde_json::Value {
    document(
        "settlement",
        "settlement",
        vec![
            with(
                saga_node("settlement", "reserve", Some("notify")),
                "cancelPolicy",
                json!(policy),
            ),
            effect_node("reserve", Some("submit"), false),
            effect_node("submit", None, true),
            crate::support::engine::activity("notify"),
        ],
    )
}

/// `reserve` done, `submit` running (it has heartbeated once).
async fn mid_flight(
    w: &World,
    doc: serde_json::Value,
) -> (
    crate::support::engine::Session,
    sagat_domain::execution::RunId,
    sagat_application::worker::ClaimedTask,
) {
    let (workflow, _) = w.publish(doc).await;
    let s = worker(w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.workers
        .heartbeat(&s.principal, &task_ref(&submit))
        .await
        .expect("started");
    (s, run, submit)
}

#[tokio::test]
async fn cancelling_stops_what_is_in_flight_and_compensates_what_completed() {
    let w = world().await;
    let (s, run, submit) = mid_flight(&w, settlement()).await;
    w.operate_run(run, RunOperation::Cancel).await;

    // The attempt in flight is told to stop; the run is not cancelled until the undo is done.
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
    let ack = w
        .workers
        .heartbeat(&s.principal, &task_ref(&submit))
        .await
        .expect("heartbeat");
    assert!(ack.cancel_requested);
    w.fail_with(&s, &submit, "CANCELLED", true)
        .await
        .expect("the worker stops");
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::Cancelled)
    );

    // A compensation runs although the run is being stopped.
    let undo = w.one_task(&s).await;
    assert_eq!(
        (undo.activity.as_str(), undo.kind),
        ("undo.reserve", TaskKind::Compensation)
    );
    assert_eq!(
        w.step_status(run, "submit~c").await,
        None,
        "the worker answered, so submit did not happen"
    );
    w.complete(&s, &undo, &json!({})).await.expect("undo");
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn waiting_for_what_is_in_flight_then_compensating_everything_that_completed() {
    let w = world().await;
    let (s, run, submit) = mid_flight(&w, with_policy("wait_in_flight_then_compensate")).await;
    w.operate_run(run, RunOperation::Cancel).await;
    let ack = w
        .workers
        .heartbeat(&s.principal, &task_ref(&submit))
        .await
        .expect("heartbeat");
    assert!(
        !ack.cancel_requested,
        "the attempt in flight is left to finish"
    );

    w.complete(&s, &submit, &json!({}))
        .await
        .expect("it finishes");
    // Its effect happened, so it is undone first and the reservation waits for it.
    let first = w.one_task(&s).await;
    assert_eq!(first.activity.as_str(), "undo.submit");
    w.complete(&s, &first, &json!({}))
        .await
        .expect("undo submit");
    let second = w.one_task(&s).await;
    assert_eq!(second.activity.as_str(), "undo.reserve");
    w.complete(&s, &second, &json!({}))
        .await
        .expect("undo reserve");
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn aborting_without_compensation_just_stops() {
    let w = world().await;
    let (s, run, submit) = mid_flight(&w, with_policy("abort_no_compensation")).await;
    w.operate_run(run, RunOperation::Cancel).await;
    w.fail_with(&s, &submit, "CANCELLED", true)
        .await
        .expect("stops");
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        None,
        "nothing was undone, as the policy says"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_deadline_compensates_and_the_run_times_out_only_when_that_is_done() {
    let w = world().await;
    let (s, run, submit) = mid_flight(&w, settlement()).await;
    w.age_run_deadline(run).await;
    w.scheduler.tick().await;
    w.fail_with(&s, &submit, "CANCELLED", true)
        .await
        .expect("stops");
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Compensating,
        "not TIMED_OUT yet: a compensation is active"
    );
    let undo = w.one_task(&s).await;
    w.complete(&s, &undo, &json!({})).await.expect("undo");
    assert_eq!(w.run_status(run).await, RunStatus::TimedOut);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn cancelling_a_run_that_waits_for_an_operator_gives_up_on_the_review() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let _submit = w.one_task(&s).await;
    w.age_lease(run, "submit").await;
    w.scheduler.tick().await;
    let lookup = w.one_task(&s).await;
    w.answer_reconciliation(&s, &lookup, "unknown", None)
        .await
        .expect("unknown");
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);

    w.operate_run(run, RunOperation::Cancel).await;
    // Nobody is left to decide: the step is given up on and, because its outcome was never
    // established, it is undone like what completed.
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::Cancelled)
    );
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
    for expected in ["undo.submit", "undo.reserve"] {
        let task = w.one_task(&s).await;
        assert_eq!(task.activity.as_str(), expected);
        w.complete(&s, &task, &json!({})).await.expect("undo");
    }
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn cancelling_a_paused_run_still_undoes_what_it_did() {
    let w = world().await;
    let (run, s, submit) = {
        let (s, run, submit) = mid_flight(&w, settlement()).await;
        (run, s, submit)
    };
    w.operate_run(run, RunOperation::Pause).await;
    assert_eq!(w.run_status(run).await, RunStatus::Paused);
    w.operate_run(run, RunOperation::Cancel).await;

    // Stopping supersedes the pause: the run is no longer paused, so the undo can be leased.
    let paused: bool = sqlx::query_scalar("SELECT paused FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("paused");
    assert!(!paused);
    w.fail_with(&s, &submit, "CANCELLED", true)
        .await
        .expect("stops");
    let undo = w.one_task(&s).await;
    assert_eq!(undo.activity.as_str(), "undo.reserve");
    w.complete(&s, &undo, &json!({})).await.expect("undone");
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    w.assert_consistent(run).await;
}

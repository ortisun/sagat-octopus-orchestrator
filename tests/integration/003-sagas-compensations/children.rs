//! FR-003-006: a cancellation or a deadline reaches the sub-workflows under the policy the `child`
//! node declares, and a saga in a child undoes what it did on the way.

use sagat_application::execution::child_run_id;
use sagat_application::runs::RunOperation;
use sagat_domain::execution::{RunId, RunStatus, StepStatus};
use serde_json::{Value, json};

use crate::support::engine::{
    Session, World, activity, child_node, document, effect_activities, effect_node, saga_node,
    with, world,
};

/// The child: a saga over `reserve` then `submit`.
fn kid() -> Value {
    document(
        "kid-saga",
        "tx",
        vec![
            saga_node("tx", "reserve", None),
            effect_node("reserve", Some("submit"), false),
            effect_node("submit", None, false),
        ],
    )
}

/// The parent: the child, then `after`.
fn mother(policy: &str) -> Value {
    document(
        "mother",
        "c",
        vec![
            with(
                with(
                    child_node("c", "kid-saga", None),
                    "cancelPolicy",
                    json!(policy),
                ),
                "next",
                json!("after"),
            ),
            activity("after"),
        ],
    )
}

fn child_of(parent: RunId) -> RunId {
    child_run_id(
        parent,
        &sagat_domain::workflow::NodeId::parse("c").expect("node"),
    )
}

/// The parent started, its child created, `reserve` done and `submit` leased by the worker.
async fn mid_flight(
    w: &World,
    policy: &str,
) -> (
    RunId,
    RunId,
    Session,
    sagat_application::worker::ClaimedTask,
) {
    w.publish(kid()).await;
    let (wf, _) = w.publish(mother(policy)).await;
    let run = w.start(wf, &json!({})).await;
    assert_eq!(w.scheduler.tick().await.children_started, 1);
    let child = child_of(run);
    let names = effect_activities(&["reserve", "submit"]);
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = w.session(&refs).await;
    let reserve = w.one_task(&s).await;
    assert_eq!(reserve.run_id, child);
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    (run, child, s, submit)
}

#[tokio::test]
async fn cancelling_the_parent_makes_the_child_saga_undo_its_work_before_the_parent_is_done() {
    let w = world().await;
    let (run, child, s, submit) = mid_flight(&w, "request_cancel").await;

    w.operate(run, RunOperation::Cancel).await.expect("cancel");
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Cancelling,
        "the parent waits for its child"
    );
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_cancelled, 1);

    // The child is being undone, not simply stopped: its saga compensates what completed.
    assert_eq!(w.run_status(child).await, RunStatus::Compensating);
    w.fail_with(&s, &submit, "CANCELLED", true)
        .await
        .expect("stops");
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Cancelling,
        "still waiting: the child is not done"
    );
    let undo = w.one_task(&s).await;
    assert_eq!(undo.activity.as_str(), "undo.reserve");
    w.complete(&s, &undo, &json!({})).await.expect("undone");
    assert_eq!(w.run_status(child).await, RunStatus::Cancelled);

    // Only now does the parent learn that the child settled.
    w.scheduler.tick().await;
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    assert_eq!(w.step_status(run, "after").await, None);
    w.assert_consistent(run).await;
    w.assert_consistent(child).await;
}

#[tokio::test]
async fn a_propagating_parent_finishes_at_once_and_the_child_still_undoes_its_work() {
    let w = world().await;
    let (run, child, s, submit) = mid_flight(&w, "propagate").await;
    w.operate(run, RunOperation::Cancel).await.expect("cancel");
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Cancelled,
        "the parent does not wait"
    );

    assert_eq!(w.scheduler.tick().await.children_cancelled, 1);
    w.fail_with(&s, &submit, "CANCELLED", true)
        .await
        .expect("stops");
    let undo = w.one_task(&s).await;
    assert_eq!(undo.activity.as_str(), "undo.reserve");
    assert_eq!(undo.run_id, child);
    w.complete(&s, &undo, &json!({})).await.expect("undone");
    assert_eq!(w.run_status(child).await, RunStatus::Cancelled);
    w.assert_consistent(child).await;
}

#[tokio::test]
async fn a_detached_child_saga_is_left_alone() {
    let w = world().await;
    let (run, child, s, submit) = mid_flight(&w, "detach").await;
    w.operate(run, RunOperation::Cancel).await.expect("cancel");
    assert_eq!(w.scheduler.tick().await.children_cancelled, 0);
    assert_eq!(w.run_status(child).await, RunStatus::Running);
    w.complete(&s, &submit, &json!({}))
        .await
        .expect("the child goes on");
    assert_eq!(w.run_status(child).await, RunStatus::Succeeded);
}

#[tokio::test]
async fn a_deadline_of_the_parent_reaches_the_child_saga_which_undoes_its_work() {
    let w = world().await;
    let (run, child, s, submit) = mid_flight(&w, "request_cancel").await;
    w.age_run_deadline(run).await;
    w.scheduler.tick().await;
    w.scheduler.tick().await;
    assert_eq!(
        w.run_status(child).await,
        RunStatus::Compensating,
        "the child is undoing its work"
    );
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Cancelling,
        "and the parent waits for it"
    );

    w.fail_with(&s, &submit, "CANCELLED", true)
        .await
        .expect("stops");
    let undo = w.one_task(&s).await;
    w.complete(&s, &undo, &json!({})).await.expect("undone");
    w.scheduler.tick().await;
    assert_eq!(w.run_status(run).await, RunStatus::TimedOut);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_child_that_fails_inside_a_parent_saga_makes_the_parent_undo_what_it_did_before() {
    let w = world().await;
    w.publish(document("kid-plain", "kid", vec![activity("kid")]))
        .await;
    let (wf, _) = w
        .publish(document(
            "mother-saga",
            "tx",
            vec![
                saga_node("tx", "reserve", None),
                effect_node("reserve", Some("c"), false),
                child_node("c", "kid-plain", None),
            ],
        ))
        .await;
    let run = w.start(wf, &json!({})).await;
    let s = w
        .session(&["act.reserve", "undo.reserve", "lookup.reserve", "act.kid"])
        .await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    assert_eq!(w.scheduler.tick().await.children_started, 1);
    let kid_task = w.one_task(&s).await;
    w.fail(&s, &kid_task, false)
        .await
        .expect("the child fails for good");
    w.scheduler.tick().await;

    // The child's failure is the saga's failure: the reservation is undone.
    assert_eq!(w.step_status(run, "c").await, Some(StepStatus::Failed));
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
    let undo = w.one_task(&s).await;
    assert_eq!(undo.activity.as_str(), "undo.reserve");
    w.complete(&s, &undo, &json!({})).await.expect("undone");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    w.assert_consistent(run).await;
}

/// Publishes a parent whose child is a saga, optionally with an error route on the child step.
async fn publish_mother(w: &World, on_error: bool) -> sagat_domain::definition::WorkflowId {
    w.publish(kid()).await;
    let mut node = with(child_node("c", "kid-saga", None), "next", json!("after"));
    let mut nodes = vec![activity("after")];
    if on_error {
        node = with(node, "onError", json!("recover"));
        nodes.push(activity("recover"));
    }
    nodes.insert(0, node);
    let (wf, _) = w.publish(document("mother-compensated", "c", nodes)).await;
    wf
}

/// Starts the parent and has its child fail inside its own saga and be undone.
async fn child_that_compensates(
    w: &World,
    wf: sagat_domain::definition::WorkflowId,
) -> (RunId, RunId) {
    let run = w.start(wf, &json!({})).await;
    assert_eq!(w.scheduler.tick().await.children_started, 1);
    let child = child_of(run);
    let names = effect_activities(&["reserve", "submit"]);
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = w.session(&refs).await;
    let reserve = w.one_task(&s).await;
    assert_eq!(reserve.run_id, child);
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("declined");
    let undo = w.one_task(&s).await;
    w.complete(&s, &undo, &json!({})).await.expect("undone");
    assert_eq!(w.run_status(child).await, RunStatus::Compensated);
    (run, child)
}

#[tokio::test]
async fn a_child_that_ended_compensated_fails_its_parents_step_instead_of_keeping_it_waiting() {
    let w = world().await;
    let wf = publish_mother(&w, false).await;
    let (run, _child) = child_that_compensates(&w, wf).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_finished, 1, "{report:?}");
    assert_eq!(w.step_status(run, "c").await, Some(StepStatus::Failed));
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    let code: Option<String> = sqlx::query_scalar("SELECT stop_code FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("code");
    assert_eq!(code.as_deref(), Some("CHILD_COMPENSATED"));
    // Nothing is left to report: the next tick has nothing to do about it.
    assert_eq!(w.scheduler.tick().await.children_finished, 0);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn the_parent_can_route_on_a_child_that_was_compensated() {
    let w = world().await;
    let wf = publish_mother(&w, true).await;
    let (run, _child) = child_that_compensates(&w, wf).await;
    w.scheduler.tick().await;
    let s = w.session(&["act.recover"]).await;
    let recover = w.one_task(&s).await;
    assert_eq!(recover.run_id, run);
    w.complete(&s, &recover, &json!({})).await.expect("recover");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
}

#[tokio::test]
async fn children_that_ended_compensated_do_not_hold_up_the_report_of_the_others() {
    // The scheduler reports the oldest ended children first and a batch at a time: a kind of end it
    // skipped would sit at the head of the list for ever and starve everyone behind it.
    let w = world().await;
    let wf = publish_mother(&w, false).await;
    let mut parents = Vec::new();
    for _ in 0..3 {
        parents.push(child_that_compensates(&w, wf).await.0);
    }
    for _ in 0..3 {
        w.scheduler.tick().await;
    }
    for run in &parents {
        assert_eq!(
            w.run_status(*run).await,
            RunStatus::Failed,
            "every parent was told"
        );
        w.assert_consistent(*run).await;
    }
}

//! Signals, conditions and parallel branches through the real engine (FR-002-001, FR-002-007).

use sagat_application::error::AppError;
use sagat_application::runs::RunOperation;
use sagat_domain::execution::{RunStatus, StepStatus};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{
    activity, condition_node, ctx, document, end_node, flaky, parallel_node, task_ref, wait_node,
    with, world,
};

fn payment_flow(name: &str) -> Value {
    document(
        name,
        "w",
        vec![
            with(
                wait_node(
                    "w",
                    "payment.confirmed",
                    "input.paymentId",
                    300_000,
                    Some("review"),
                ),
                "next",
                json!("notify"),
            ),
            activity("notify"),
            activity("review"),
        ],
    )
}

// ------------------------------------------------------------------ signals

#[tokio::test]
async fn a_signal_wakes_a_waiting_run_and_its_payload_reaches_the_next_step() {
    let w = world().await;
    let (wf, _) = w.publish(payment_flow("signalled")).await;
    let run = w.start(wf, &json!({"paymentId": "p-9"})).await;
    assert_eq!(w.run_status(run).await, RunStatus::Waiting);

    let woken = w
        .runs
        .signal(
            &w.admin,
            run,
            "payment.confirmed",
            br#"{"amount": 150, "ok": true}"#,
            "signal-key-1",
            &ctx(),
        )
        .await
        .expect("signal");
    assert_eq!(woken.iter().map(|n| n.as_str()).collect::<Vec<_>>(), ["w"]);
    assert_eq!(w.run_status(run).await, RunStatus::Running);

    let session = w.session(&["act.notify", "act.review"]).await;
    let task = w.one_task(&session).await;
    assert_eq!(
        task.instance.node().as_str(),
        "notify",
        "the normal route, not the timeout route"
    );
    let input: Value = serde_json::from_slice(&task.input).expect("json");
    assert_eq!(
        input["steps"]["w"]["output"],
        json!({"amount": 150, "ok": true})
    );
    w.complete(&session, &task, &json!({}))
        .await
        .expect("notify");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_signal_nobody_is_waiting_for_is_refused_never_buffered_or_dropped_silently() {
    let w = world().await;
    let (wf, _) = w.publish(payment_flow("unexpected")).await;
    let run = w.start(wf, &json!({"paymentId": "p-9"})).await;
    let wrong_name = w
        .runs
        .signal(
            &w.admin,
            run,
            "something.else",
            b"{}",
            "signal-key-1",
            &ctx(),
        )
        .await;
    assert_eq!(
        wrong_name.map(|_| ()),
        Err(AppError::StateConflict(
            "the run is not waiting for that signal"
        ))
    );

    w.runs
        .signal(
            &w.admin,
            run,
            "payment.confirmed",
            b"{}",
            "signal-key-2",
            &ctx(),
        )
        .await
        .expect("first");
    let twice = w
        .runs
        .signal(
            &w.admin,
            run,
            "payment.confirmed",
            b"{}",
            "signal-key-3",
            &ctx(),
        )
        .await;
    assert!(
        matches!(twice, Err(AppError::StateConflict(_))),
        "the wait is already satisfied: {twice:?}"
    );
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "wait.satisfied")
            .count(),
        1
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_retried_signal_with_the_same_key_is_answered_not_applied_twice() {
    let w = world().await;
    let (wf, _) = w.publish(payment_flow("retried-signal")).await;
    let run = w.start(wf, &json!({"paymentId": "p-9"})).await;
    let first = w
        .runs
        .signal(
            &w.admin,
            run,
            "payment.confirmed",
            br#"{"n":1}"#,
            "signal-key-1",
            &ctx(),
        )
        .await
        .expect("first");
    let retry = w
        .runs
        .signal(
            &w.admin,
            run,
            "payment.confirmed",
            br#"{"n":1}"#,
            "signal-key-1",
            &ctx(),
        )
        .await
        .expect("retry");
    assert_eq!(first, retry);
    let reused = w
        .runs
        .signal(
            &w.admin,
            run,
            "payment.confirmed",
            br#"{"n":2}"#,
            "signal-key-1",
            &ctx(),
        )
        .await;
    assert_eq!(reused.map(|_| ()), Err(AppError::IdempotencyKeyReused));
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "wait.satisfied")
            .count(),
        1
    );
}

#[tokio::test]
async fn the_first_commit_wins_the_race_between_a_signal_and_the_timeout() {
    let w = world().await;
    let (wf, _) = w.publish(payment_flow("race")).await;
    let session = w.session(&["act.notify", "act.review"]).await;
    let context = ctx();
    let mut outcomes = std::collections::BTreeSet::new();
    for round in 0..10 {
        let run = w.start(wf, &json!({"paymentId": "p"})).await;
        w.age_due(run, "w").await;

        let key = format!("signal-key-{round:04}");
        let (signal, tick) = tokio::join!(
            w.runs
                .signal(&w.admin, run, "payment.confirmed", b"{}", &key, &context),
            w.scheduler.tick()
        );
        let signalled = signal.is_ok();
        let timed_out = tick.waits_timed_out == 1;
        assert!(
            signalled ^ timed_out,
            "round {round}: signal={signal:?} tick={tick:?}"
        );
        let expected = if signalled { "notify" } else { "review" };
        outcomes.insert(expected);
        assert_eq!(
            w.one_task(&session).await.instance.node().as_str(),
            expected,
            "round {round}"
        );
        w.assert_consistent(run).await;
    }
    // Either side may win a round; what must hold is that exactly one did, every time.
    assert!(!outcomes.is_empty());
}

// ------------------------------------------------------------------ conditions

fn routing(name: &str) -> Value {
    document(
        name,
        "check",
        vec![
            condition_node("check", "input.amount > 100", "big", "small"),
            activity("big"),
            activity("small"),
        ],
    )
}

#[tokio::test]
async fn a_condition_routes_by_the_run_input() {
    let w = world().await;
    let (wf, _) = w.publish(routing("by-amount")).await;
    let session = w.session(&["act.big", "act.small"]).await;
    for (amount, expected) in [(500, "big"), (5, "small"), (100, "small"), (101, "big")] {
        let run = w.start(wf, &json!({ "amount": amount })).await;
        let task = w.one_task(&session).await;
        assert_eq!(task.instance.node().as_str(), expected, "amount {amount}");
        assert_eq!(
            w.step_status(run, "check").await,
            Some(StepStatus::Succeeded)
        );
        w.complete(&session, &task, &json!({}))
            .await
            .expect("complete");
        assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
        w.assert_consistent(run).await;
    }
}

#[tokio::test]
async fn a_condition_over_a_previous_steps_output_waits_for_that_step() {
    let w = world().await;
    let doc = document(
        "by-output",
        "score",
        vec![
            with(activity("score"), "next", json!("check")),
            condition_node("check", "steps.score.output.risk > 0.5", "manual", "auto"),
            activity("manual"),
            activity("auto"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.score", "act.manual", "act.auto"]).await;
    let score = w.one_task(&session).await;
    w.complete(&session, &score, &json!({"risk": 0.9}))
        .await
        .expect("score");
    assert_eq!(
        w.one_task(&session).await.instance.node().as_str(),
        "manual"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_expression_that_cannot_be_evaluated_fails_the_run_without_retrying() {
    let w = world().await;
    let (wf, _) = w.publish(routing("missing-field")).await;
    let run = w.start(wf, &json!({"amount_typo": 1})).await;
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    let code: Option<String> = sqlx::query_scalar("SELECT stop_code FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("code");
    assert_eq!(code.as_deref(), Some("EXPRESSION_ERROR"));
    assert!(w.journal(run).await.iter().any(|e| e == "step.failed"));
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_correlation_expression_that_fails_routes_through_on_error() {
    let w = world().await;
    let doc = document(
        "bad-key",
        "w",
        vec![
            with(
                wait_node("w", "e", "input.missing", 1000, None),
                "onError",
                json!("fallback"),
            ),
            activity("fallback"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.fallback"]).await;
    assert_eq!(
        w.one_task(&session).await.instance.node().as_str(),
        "fallback"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_end_node_closes_the_run() {
    let w = world().await;
    let doc = document(
        "ends",
        "a",
        vec![with(activity("a"), "next", json!("done")), end_node("done")],
    );
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({})).await.expect("a");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
}

// ------------------------------------------------------------------ parallel

fn fork(name: &str, join: &str) -> Value {
    document(
        name,
        "fork",
        vec![
            with(
                parallel_node("fork", &["left", "right"], join),
                "next",
                json!("after"),
            ),
            activity("left"),
            activity("right"),
            activity("after"),
        ],
    )
}

#[tokio::test]
async fn an_all_join_waits_for_both_branches_which_run_on_different_workers() {
    let w = world().await;
    let (wf, _) = w.publish(fork("all-join", "all")).await;
    let run = w.start(wf, &json!({})).await;
    let one = w.session(&["act.left", "act.after"]).await;
    let two = w.session(&["act.right"]).await;
    let left = w.one_task(&one).await;
    let right = w.one_task(&two).await;
    assert_eq!(
        (
            left.instance.node().as_str(),
            right.instance.node().as_str()
        ),
        ("left", "right")
    );

    w.complete(&one, &left, &json!({"l": 1}))
        .await
        .expect("left");
    assert_eq!(
        w.step_status(run, "after").await,
        None,
        "the join is not satisfied by one branch"
    );
    assert_eq!(w.run_status(run).await, RunStatus::Running);
    w.complete(&two, &right, &json!({"r": 2}))
        .await
        .expect("right");
    let after = w.one_task(&one).await;
    assert_eq!(after.instance.node().as_str(), "after");
    let input: Value = serde_json::from_slice(&after.input).expect("json");
    assert_eq!(input["steps"]["left"]["output"], json!({"l": 1}));
    assert_eq!(input["steps"]["right"]["output"], json!({"r": 2}));
    w.complete(&one, &after, &json!({})).await.expect("after");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_all_join_fails_fast_and_tells_the_running_sibling_to_stop() {
    let w = world().await;
    let (wf, _) = w.publish(fork("fail-fast", "all")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.left", "act.right", "act.after"]).await;
    let tasks = w.poll(&session, 8).await;
    assert_eq!(tasks.len(), 2);
    let (left, right) = if tasks[0].instance.node().as_str() == "left" {
        (&tasks[0], &tasks[1])
    } else {
        (&tasks[1], &tasks[0])
    };
    w.workers
        .heartbeat(&session.principal, &task_ref(right))
        .await
        .expect("right is running");

    w.fail(&session, left, false)
        .await
        .expect("left fails for good");
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Cancelling,
        "the run waits for the sibling to acknowledge"
    );
    let beat = w
        .workers
        .heartbeat(&session.principal, &task_ref(right))
        .await
        .expect("heartbeat");
    assert!(beat.cancel_requested);
    w.fail(&session, right, true)
        .await
        .expect("the sibling acknowledges");
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    assert_eq!(
        w.step_status(run, "right").await,
        Some(StepStatus::Cancelled)
    );
    assert_eq!(w.step_status(run, "after").await, None);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_any_join_continues_with_the_first_branch_and_cancels_the_other() {
    let w = world().await;
    let (wf, _) = w.publish(fork("any-join", "any")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.left", "act.right", "act.after"]).await;
    let tasks = w.poll(&session, 8).await;
    let left = tasks
        .iter()
        .find(|t| t.instance.node().as_str() == "left")
        .expect("left");
    let right = tasks
        .iter()
        .find(|t| t.instance.node().as_str() == "right")
        .expect("right");

    w.complete(&session, left, &json!({}))
        .await
        .expect("left wins");
    assert_eq!(
        w.step_status(run, "after").await,
        Some(StepStatus::Ready),
        "the run continues at once"
    );
    // The loser is told to stop; its late result is recorded but starts nothing.
    let beat = w
        .workers
        .heartbeat(&session.principal, &task_ref(right))
        .await
        .expect("heartbeat");
    assert!(beat.cancel_requested);
    w.complete(&session, right, &json!({"late": true}))
        .await
        .expect("late result is recorded");
    let after = w.one_task(&session).await;
    w.complete(&session, &after, &json!({}))
        .await
        .expect("after");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

// ------------------------------------------------------------------ cancellation and pause

#[tokio::test]
async fn cancelling_a_run_with_work_ready_ends_it_at_once() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("cancel-ready", "a", vec![flaky("a", 3)]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let record = w.operate(run, RunOperation::Cancel).await.expect("cancel");
    assert_eq!(record.status, RunStatus::Cancelled);
    let session = w.session(&["act.a"]).await;
    assert!(
        w.poll(&session, 4).await.is_empty(),
        "nothing is claimable after cancellation"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn cancelling_a_running_attempt_waits_for_its_acknowledgement() {
    let w = world().await;
    let (wf, _) = w
        .publish(document(
            "cancel-running",
            "a",
            vec![with(activity("a"), "next", json!("b")), activity("b")],
        ))
        .await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;
    assert_eq!(
        w.operate(run, RunOperation::Cancel)
            .await
            .expect("cancel")
            .status,
        RunStatus::Cancelling
    );

    let beat = w
        .workers
        .heartbeat(&session.principal, &task_ref(&task))
        .await
        .expect("heartbeat");
    assert!(beat.cancel_requested);
    w.complete(&session, &task, &json!({}))
        .await
        .expect("the late result is still recorded");
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    assert_eq!(
        w.step_status(run, "b").await,
        None,
        "but nothing new started"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn pausing_holds_back_new_work_and_resuming_releases_it() {
    let w = world().await;
    let (wf, _) = w
        .publish(document(
            "pausable",
            "a",
            vec![with(activity("a"), "next", json!("b")), activity("b")],
        ))
        .await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let a = w.one_task(&session).await;

    assert_eq!(
        w.operate(run, RunOperation::Pause)
            .await
            .expect("pause")
            .status,
        RunStatus::Paused
    );
    w.complete(&session, &a, &json!({}))
        .await
        .expect("an in-flight attempt may finish");
    assert_eq!(
        w.step_status(run, "b").await,
        None,
        "its successor is held back"
    );
    assert!(w.poll(&session, 4).await.is_empty());

    assert_eq!(
        w.operate(run, RunOperation::Resume)
            .await
            .expect("resume")
            .status,
        RunStatus::Running
    );
    assert_eq!(w.one_task(&session).await.instance.node().as_str(), "b");
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn operator_actions_are_idempotent_audited_and_refused_when_the_state_forbids_them() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("operated", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;

    let key = format!("op-{}", Uuid::now_v7());
    let first = w
        .runs
        .operate(
            &w.admin,
            run,
            RunOperation::Pause,
            "pausing for maintenance",
            None,
            &key,
            &ctx(),
        )
        .await
        .expect("pause");
    let again = w
        .runs
        .operate(
            &w.admin,
            run,
            RunOperation::Pause,
            "pausing for maintenance",
            None,
            &key,
            &ctx(),
        )
        .await
        .expect("retry");
    assert_eq!(first.status, again.status);
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "run.paused")
            .count(),
        1
    );
    let reused = w
        .runs
        .operate(
            &w.admin,
            run,
            RunOperation::Resume,
            "resuming",
            None,
            &key,
            &ctx(),
        )
        .await;
    assert_eq!(reused.map(|_| ()), Err(AppError::IdempotencyKeyReused));

    // Not paused twice, not resumed when running, not changed from a stale version.
    let second_pause = w
        .runs
        .operate(
            &w.admin,
            run,
            RunOperation::Pause,
            "again",
            None,
            "op-key-2222",
            &ctx(),
        )
        .await;
    assert!(matches!(second_pause, Err(AppError::StateConflict(_))));
    let stale = w
        .runs
        .operate(
            &w.admin,
            run,
            RunOperation::Resume,
            "resume",
            Some(0),
            "op-key-3333",
            &ctx(),
        )
        .await;
    assert!(
        matches!(stale, Err(AppError::RevisionConflict { .. })),
        "{stale:?}"
    );

    w.runs
        .operate(
            &w.admin,
            run,
            RunOperation::Cancel,
            "no longer needed",
            None,
            "op-key-4444",
            &ctx(),
        )
        .await
        .expect("cancel");
    let finished = w
        .runs
        .operate(
            &w.admin,
            run,
            RunOperation::Cancel,
            "again",
            None,
            "op-key-5555",
            &ctx(),
        )
        .await;
    assert!(matches!(finished, Err(AppError::StateConflict(_))));

    let audited: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_records WHERE resource_type = 'run' AND action <> 'run.started' ORDER BY created_at")
        .fetch_all(&w.db.pool)
        .await
        .expect("audit");
    assert_eq!(
        audited,
        ["run.paused", "run.cancel_requested"],
        "one record per effective action, none for refused ones"
    );
    w.assert_consistent(run).await;
}

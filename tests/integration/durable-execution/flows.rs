//! A workflow runs end to end against the real store (T002-008, T002-013; US01..US04).

use sagat_application::error::AppError;
use sagat_application::worker::StepAck;
use sagat_domain::execution::{RunStatus, StepStatus};
use serde_json::json;

use crate::support::engine::{chain, world};

#[tokio::test]
async fn a_two_step_workflow_runs_to_success_and_the_journal_tells_the_story() {
    let w = world().await;
    let (wf, _) = w.publish(chain("two-steps")).await;
    let run = w.start(wf, &json!({"orderId": "o-1"})).await;

    // Started: the first step exists and waits for a worker.
    assert_eq!(w.run_status(run).await, RunStatus::Running);
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Ready));
    assert_eq!(
        w.step_status(run, "b").await,
        None,
        "b does not exist before a is done"
    );
    w.assert_consistent(run).await;

    let session = w.session(&["act.a", "act.b"]).await;
    let a = w.one_task(&session).await;
    assert_eq!(a.node.as_str(), "a");
    assert_eq!((a.fence.attempt, a.fence.generation), (1, 1));
    assert_eq!(a.activity.as_str(), "act.a");
    assert_eq!(a.idempotency_key, format!("{run}:a"));
    assert!(a.task_id().starts_with(&run.to_string()));
    let input: serde_json::Value = serde_json::from_slice(&a.input).expect("json input");
    assert_eq!(input["input"]["orderId"], "o-1");
    assert_eq!(input["node"], "a");
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Leased));
    w.assert_consistent(run).await;

    let ack = w
        .complete(&session, &a, &json!({"approved": true}))
        .await
        .expect("complete a");
    assert_eq!(
        (ack.step_status, ack.run_status, ack.already_applied),
        (StepStatus::Succeeded, RunStatus::Running, false)
    );
    assert_eq!(w.step_status(run, "b").await, Some(StepStatus::Ready));
    w.assert_consistent(run).await;

    // The output of a is part of what b receives.
    let b = w.one_task(&session).await;
    assert_eq!(b.node.as_str(), "b");
    let input: serde_json::Value = serde_json::from_slice(&b.input).expect("json input");
    assert_eq!(input["steps"]["a"]["output"], json!({"approved": true}));

    let ack = w
        .complete(&session, &b, &json!({}))
        .await
        .expect("complete b");
    assert_eq!(ack.run_status, RunStatus::Succeeded);
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;

    assert_eq!(
        w.journal(run).await,
        [
            "run.started",
            "step.released",
            "step.leased",
            "step.completed",
            "step.routed",
            "step.released",
            "step.leased",
            "step.completed",
            "step.routed",
            "run.finished",
        ]
    );
}

#[tokio::test]
async fn a_completion_with_the_wrong_token_or_generation_changes_nothing() {
    let w = world().await;
    let (wf, _) = w.publish(chain("fenced")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;
    let journal_before = w.journal(run).await;

    // A forged token.
    let mut forged = task.clone();
    forged.lease_token = sagat_application::entropy::Token::from_bytes([7u8; 32]).expect("token");
    assert_eq!(
        w.complete(&session, &forged, &json!({})).await.map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    // A wrong generation or attempt with the right token.
    for (attempt, generation) in [(1, 2), (2, 1), (0, 0)] {
        let mut wrong = task.clone();
        wrong.fence.attempt = attempt;
        wrong.fence.generation = generation;
        assert_eq!(
            w.complete(&session, &wrong, &json!({})).await.map(|_| ()),
            Err(AppError::StaleAttempt),
            "{attempt}/{generation}"
        );
    }
    assert_eq!(
        w.journal(run).await,
        journal_before,
        "a refused report journals nothing"
    );
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Leased));
    w.assert_consistent(run).await;

    w.complete(&session, &task, &json!({}))
        .await
        .expect("the real holder still can");
}

#[tokio::test]
async fn reporting_the_same_completion_twice_returns_the_original_outcome() {
    let w = world().await;
    let (wf, _) = w.publish(chain("duplicate")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;

    let first = w
        .complete(&session, &task, &json!({"x": 1}))
        .await
        .expect("first");
    // The step has moved on (b exists), yet the repeat is answered from the idempotency record.
    let again = w
        .complete(&session, &task, &json!({"x": 1}))
        .await
        .expect("repeat");
    assert!(!first.already_applied && again.already_applied);
    assert_eq!(
        StepAck {
            already_applied: false,
            ..again.clone()
        },
        first
    );
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "step.completed")
            .count(),
        1
    );

    // The same key with a different result is refused, not silently accepted.
    let other = w
        .workers
        .complete(
            &session.principal,
            &crate::support::engine::task_ref(&task),
            br#"{"x":2}"#,
            &format!("done-{}", task.task_id()),
        )
        .await;
    assert_eq!(other.map(|_| ()), Err(AppError::IdempotencyKeyReused));
    w.assert_consistent(run).await;
}

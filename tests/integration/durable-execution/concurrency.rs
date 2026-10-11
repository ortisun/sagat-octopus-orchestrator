//! Races: who gets a task, who wins a completion, what happens when a lease expires mid-report
//! (T002-023, T002-027; US04; FR-002-004, NFR-002-DUR).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use sagat_application::error::AppError;
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::execution::{RunStatus, StepStatus};
use serde_json::json;

use crate::support::engine::{activity, chain, document, task_ref, with, world};

#[tokio::test]
async fn sixteen_workers_racing_for_one_task_produce_exactly_one_lease() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("one-task", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;

    let mut sessions = Vec::new();
    for _ in 0..16 {
        sessions.push(w.session(&["act.a"]).await);
    }
    let polls = futures_util::future::join_all(sessions.iter().map(|s| w.poll(s, 4))).await;
    let granted: Vec<_> = polls.iter().flatten().collect();
    assert_eq!(granted.len(), 1, "one step, one lease: {}", granted.len());
    assert_eq!(
        (granted[0].fence.attempt, granted[0].fence.generation),
        (1, 1)
    );
    let leased: i64 = sqlx::query_scalar("SELECT count(*) FROM steps WHERE status = 'LEASED'")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(leased, 1);
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "step.leased")
            .count(),
        1,
        "the lease is journaled once"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn many_workers_and_many_tasks_never_assign_a_step_twice() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("fanout", "a", vec![activity("a")]))
        .await;
    let mut runs = Vec::new();
    for i in 0..30 {
        runs.push(w.start(wf, &json!({"i": i})).await);
    }
    let mut sessions = Vec::new();
    for _ in 0..8 {
        sessions.push(w.session(&["act.a"]).await);
    }
    // Everyone polls repeatedly until the queue is empty.
    let claimed: Vec<Vec<_>> = futures_util::future::join_all(sessions.iter().map(|s| async {
        let mut mine = Vec::new();
        for _ in 0..12 {
            let got = w.poll(s, 4).await;
            if got.is_empty() {
                break;
            }
            mine.extend(got);
        }
        mine
    }))
    .await;
    let all: Vec<_> = claimed.iter().flatten().collect();
    let unique: BTreeSet<_> = all
        .iter()
        .map(|t| (t.run_id, t.node.clone(), t.fence.generation))
        .collect();
    assert_eq!(all.len(), unique.len(), "a step was handed out twice");
    assert_eq!(
        unique.len(),
        30,
        "every ready step was handed out exactly once"
    );
    for run in runs {
        w.assert_consistent(run).await;
    }
}

#[tokio::test]
async fn a_poll_never_waits_for_a_run_that_another_transaction_holds() {
    let w = world().await;
    let (wf, _) = w.publish(chain("locked")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;

    // Something else (an operator command, a timer) holds the run's row lock.
    let held = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("exists");
    let started = Instant::now();
    let tasks = w.poll(&session, 4).await;
    assert!(tasks.is_empty(), "the locked run is skipped");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "polling blocked for {:?}",
        started.elapsed()
    );
    drop(held); // rolls back

    assert_eq!(
        w.poll(&session, 4).await.len(),
        1,
        "and picked up as soon as it is free"
    );
}

#[tokio::test]
async fn eight_simultaneous_reports_of_one_completion_apply_it_once() {
    let w = world().await;
    let (wf, _) = w.publish(chain("storm")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;

    let output = json!({"ok": true});
    let acks =
        futures_util::future::join_all((0..8).map(|_| w.complete(&session, &task, &output))).await;
    let applied = acks
        .iter()
        .filter(|a| matches!(a, Ok(ack) if !ack.already_applied))
        .count();
    let replays = acks
        .iter()
        .filter(|a| matches!(a, Ok(ack) if ack.already_applied))
        .count();
    assert_eq!((applied, replays), (1, 7), "{acks:?}");
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "step.completed")
            .count(),
        1
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_completion_racing_a_lease_expiry_has_exactly_one_winner() {
    // Whichever commits first decides; the loser is told so, and the run stays consistent.
    let w = world().await;
    let (wf, _) = w
        .publish(document(
            "photo-finish",
            "a",
            vec![with(activity("a"), "next", json!("b")), activity("b")],
        ))
        .await;
    let session = w.session(&["act.a", "act.b"]).await;
    let mut completions_won = 0;
    for round in 0..8 {
        let run = w.start(wf, &json!({})).await;
        let task = w.one_task(&session).await;
        w.age_lease(run, "a").await;

        let output = json!({});
        let (completion, tick) =
            tokio::join!(w.complete(&session, &task, &output), w.scheduler.tick());
        let completed = completion.is_ok();
        let expired = tick.leases_expired == 1;
        assert!(
            completed ^ expired,
            "round {round}: completed={completed} expired={expired} ({completion:?}, {tick:?})"
        );
        if completed {
            completions_won += 1;
            assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Succeeded));
        } else {
            assert_eq!(completion.map(|_| ()), Err(AppError::StaleAttempt));
            assert_eq!(w.step_status(run, "a").await, Some(StepStatus::RetryWait));
        }
        w.assert_consistent(run).await;
        // Leave nothing ready behind for the next round to trip over.
        w.operate(run, sagat_application::runs::RunOperation::Cancel)
            .await
            .expect("cancel");
        w.assert_consistent(run).await;
    }
    let _ = completions_won; // either side may win a round; both outcomes are valid
}

#[tokio::test]
async fn a_worker_never_holds_more_tasks_than_it_declared() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("limited", "a", vec![activity("a")]))
        .await;
    for i in 0..5 {
        w.start(wf, &json!({"i": i})).await;
    }
    let session = w.session_as(&w.worker, &["act.a"], 2).await;
    assert_eq!(
        w.poll(&session, 10).await.len(),
        2,
        "capped by max_inflight"
    );
    assert!(w.poll(&session, 10).await.is_empty(), "already at capacity");

    let another = w.session_as(&w.worker, &["act.a"], 10).await;
    assert_eq!(
        w.poll(&another, 10).await.len(),
        3,
        "the rest goes to someone with room"
    );
}

#[tokio::test]
async fn a_worker_only_receives_the_activities_it_advertised() {
    let w = world().await;
    let (wf, _) = w.publish(chain("capabilities")).await;
    let run = w.start(wf, &json!({})).await;
    let wrong = w.session(&["act.other"]).await;
    assert!(w.poll(&wrong, 10).await.is_empty());
    let right = w.session(&["act.a"]).await;
    assert_eq!(w.one_task(&right).await.node.as_str(), "a");
    assert_eq!(w.run_status(run).await, RunStatus::Running);
}

#[tokio::test]
async fn heartbeats_from_a_worker_that_lost_its_lease_do_not_resurrect_it() {
    let w = world().await;
    let (wf, _) = w.publish(chain("zombie")).await;
    let run = w.start(wf, &json!({})).await;
    let zombie = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&zombie).await;
    w.age_lease(run, "a").await;
    w.scheduler.tick().await;

    assert_eq!(
        w.workers
            .heartbeat(&zombie.principal, &task_ref(&task))
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    assert_eq!(
        w.step_status(run, "a").await,
        Some(StepStatus::RetryWait),
        "a late heartbeat changed nothing"
    );
    w.assert_consistent(run).await;
}

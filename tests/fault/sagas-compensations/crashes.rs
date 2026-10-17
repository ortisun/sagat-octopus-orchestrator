//! A process that dies at each point between a failure, the plan, the lease, the effect and the
//! report leaves a state from which the saga goes on, and does nothing twice.

use sagat_application::execution::{Actor, Effects, Engine};
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::execution::{self, FailureCode, RunStatus, StepStatus};
use serde_json::json;

use crate::common::{ExternalSystem, quick, settlement, snapshot, worker};
use crate::support::engine::world;

#[tokio::test]
async fn a_failure_that_is_never_committed_does_not_start_a_compensation() {
    let w = world().await;
    let (wf, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    let before = snapshot(&w, run).await;

    // The failure is applied, planned and persisted inside a transaction that is abandoned, as when
    // the process dies between the last statement and COMMIT.
    let mut tx = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("exists");
    let engine = Engine::new(sagat_infra_ir::JsonEvaluator);
    let (node, fence) = (submit.instance.node().clone(), submit.fence);
    engine
        .run(
            &mut tx,
            Effects::by(Actor::Worker("doomed".into()), None),
            move |def, state, now| {
                let failure = execution::Failure {
                    fence,
                    code: FailureCode::parse("PAYMENT_DECLINED").expect("code"),
                    retryable: false,
                    jitter_sample: 0.0,
                };
                Ok(((), execution::fail_step(def, state, &node, failure, now)?))
            },
        )
        .await
        .expect("the command itself succeeded");
    drop(tx);

    assert_eq!(
        snapshot(&w, run).await,
        before,
        "nothing was written, nothing was planned"
    );
    assert_eq!(w.step_status(run, "reserve~c").await, None);
    // The worker's retry of the same report works as if the first never happened.
    w.fail(&s, &submit, false)
        .await
        .expect("the report is repeated");
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        Some(StepStatus::Ready)
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_restart_between_the_plan_and_the_lease_loses_nothing() {
    let w = world().await;
    let (wf, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("fails");
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        Some(StepStatus::Ready)
    );

    // The process that planned the compensation is gone. Another one, with nothing in memory,
    // leases it from what the database says.
    let replica = w.replica().await;
    let tasks = replica
        .workers
        .poll(&s.principal, s.worker_id, &s.token, 8, None)
        .await
        .expect("poll");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].activity.as_str(), "undo.reserve");
    assert_eq!(tasks[0].run_id, run);
    w.complete(&s, &tasks[0], &json!({})).await.expect("undo");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_crash_after_the_provider_released_and_before_the_report_releases_once() {
    let w = world().await;
    let (wf, _) = w.publish(quick(settlement())).await;
    let s = worker(&w).await;
    let run = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("fails");
    let provider = ExternalSystem::default();

    // Worker 1 releases the limit... and dies before reporting it.
    let first = w.one_task(&s).await;
    assert!(
        provider.perform(&first.idempotency_key),
        "the first attempt really released"
    );
    w.age_lease(run, "reserve~c").await;
    w.scheduler.tick().await;
    w.age_retry(run, "reserve~c").await;
    w.scheduler.tick().await;

    // Worker 2 gets the same compensation with the SAME key, and the provider recognises it.
    let second_session = worker(&w).await;
    let second = w.one_task(&second_session).await;
    assert_eq!(second.fence.attempt, 2);
    assert_eq!(second.idempotency_key, first.idempotency_key);
    assert!(
        !provider.perform(&second.idempotency_key),
        "recognised as a duplicate"
    );
    w.complete(&second_session, &second, &json!({}))
        .await
        .expect("report");

    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    assert_eq!(
        provider.effects_with_prefix("compensation:"),
        1,
        "one release in the world"
    );
    assert_eq!(
        provider.calls(&first.idempotency_key),
        2,
        "the platform asked twice (at-least-once)"
    );
    let completed = w
        .journal(run)
        .await
        .iter()
        .filter(|e| *e == "step.completed")
        .count();
    assert_eq!(completed, 2, "reserve and its compensation, once each");
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_worker_that_dies_during_the_lookup_is_replaced_and_its_late_answer_is_refused() {
    let w = world().await;
    let (wf, _) = w.publish(quick(settlement())).await;
    let s = worker(&w).await;
    let run = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let _lost = w.one_task(&s).await;
    w.age_lease(run, "submit").await;
    w.scheduler.tick().await;

    let doomed = w.one_task(&s).await;
    assert_eq!(doomed.activity.as_str(), "lookup.submit");
    w.age_lease(run, "submit~r").await;
    w.scheduler.tick().await;
    w.age_retry(run, "submit~r").await;
    w.scheduler.tick().await;

    let rescuer_session = worker(&w).await;
    let rescuer = w.one_task(&rescuer_session).await;
    assert_eq!(rescuer.activity.as_str(), "lookup.submit");
    assert_eq!(
        rescuer.fence.attempt,
        doomed.fence.attempt + 1,
        "the lookup has its own attempts"
    );

    let late = w
        .answer_reconciliation(&s, &doomed, "not_applied", None)
        .await;
    assert!(
        matches!(late, Err(sagat_application::error::AppError::StaleAttempt)),
        "{late:?}"
    );
    w.answer_reconciliation(
        &rescuer_session,
        &rescuer,
        "applied",
        Some(json!({"id": "p-1"})),
    )
    .await
    .expect("the live answer counts");
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::Succeeded)
    );
    let verdicts = w
        .journal(run)
        .await
        .iter()
        .filter(|e| *e == "reconciliation.concluded")
        .count();
    assert_eq!(
        verdicts, 1,
        "one verdict, from the attempt that held the lease"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn duplicate_reports_of_a_lookup_and_of_a_compensation_commit_once() {
    let w = world().await;
    let (wf, _) = w.publish(settlement()).await;
    let s = worker(&w).await;

    // A lookup, answered twice.
    let first = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let _lost = w.one_task(&s).await;
    w.age_lease(first, "submit").await;
    w.scheduler.tick().await;
    let lookup = w.one_task(&s).await;
    let ack = w
        .answer_reconciliation(&s, &lookup, "applied", None)
        .await
        .expect("answer");
    assert!(!ack.already_applied);
    let again = w
        .answer_reconciliation(&s, &lookup, "applied", None)
        .await
        .expect("duplicate");
    assert!(
        again.already_applied,
        "the repeat is acknowledged, not applied"
    );
    let notify = w.one_task(&s).await;
    w.complete(&s, &notify, &json!({})).await.expect("notify");
    assert_eq!(w.run_status(first).await, RunStatus::Succeeded);
    let concluded = w
        .journal(first)
        .await
        .iter()
        .filter(|e| *e == "reconciliation.concluded")
        .count();
    assert_eq!(concluded, 1);

    // A compensation, reported twice.
    let second = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("fails");
    let undo = w.one_task(&s).await;
    let ack = w.complete(&s, &undo, &json!({})).await.expect("undo");
    assert!(!ack.already_applied);
    let dup = w.complete(&s, &undo, &json!({})).await.expect("duplicate");
    assert!(dup.already_applied);
    assert_eq!(w.run_status(second).await, RunStatus::Compensated);
    let finished = w
        .journal(second)
        .await
        .iter()
        .filter(|e| *e == "run.finished")
        .count();
    assert_eq!(finished, 1);
    w.assert_consistent(first).await;
    w.assert_consistent(second).await;
}

#[tokio::test]
async fn a_run_waiting_for_a_person_is_still_waiting_after_every_process_restarted() {
    let w = world().await;
    let (wf, _) = w.publish(crate::common::quick(settlement())).await;
    let s = worker(&w).await;
    let run = w.start(wf, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("fails");
    for attempt in 1..=4 {
        let undo = w.one_task(&s).await;
        w.fail_with(&s, &undo, "PROVIDER_DOWN", true)
            .await
            .expect("down");
        if attempt < 4 {
            w.age_retry(run, "reserve~c").await;
            w.scheduler.tick().await;
        }
    }
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    let before = snapshot(&w, run).await;

    // Nothing in memory: a scheduler pass and a fresh replica change nothing.
    let replica = w.replica().await;
    w.scheduler.tick().await;
    assert!(
        replica
            .workers
            .poll(&s.principal, s.worker_id, &s.token, 8, None)
            .await
            .expect("poll")
            .is_empty()
    );
    assert_eq!(snapshot(&w, run).await, before);

    // The person decides through the other process, and the saga goes on.
    replica
        .runs
        .review(
            &w.admin,
            run,
            &sagat_domain::workflow::NodeId::parse_stored("reserve~c").expect("node"),
            &sagat_application::runs::ReviewAction::Skip,
            "released by hand",
            None,
            "restart-review-0001",
            &crate::support::engine::ctx(),
        )
        .await
        .expect("review");
    assert_eq!(w.run_status(run).await, RunStatus::Partial);
    w.assert_consistent(run).await;
}

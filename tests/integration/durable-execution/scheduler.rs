//! The passes that move runs forward without a caller: leases, retries, timers, waits, deadlines
//! (T002-013, T002-018; US02, US03; FR-002-004, FR-002-005, FR-002-007).

use sagat_application::error::AppError;
use sagat_domain::execution::{RunStatus, StepStatus};
use serde_json::json;

use crate::support::engine::{activity, chain, document, flaky, with, world};

// ------------------------------------------------------------------ US02: a worker dies

#[tokio::test]
async fn a_lost_worker_is_replaced_and_the_stale_generation_is_never_reused() {
    let w = world().await;
    let (wf, _) = w.publish(chain("crash-recovery")).await;
    let run = w.start(wf, &json!({})).await;
    let doomed = w.session(&["act.a", "act.b"]).await;
    let first = w.one_task(&doomed).await;
    assert_eq!((first.fence.attempt, first.fence.generation), (1, 1));

    // The worker dies: no heartbeat, the lease runs out.
    w.age_lease(run, "a").await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.leases_expired, 1);
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::RetryWait));
    assert!(
        w.poll(&doomed, 8).await.is_empty(),
        "nothing is claimable during the backoff"
    );

    // The default policy backs off before the next attempt; once it elapses the step is ready again.
    w.age_retry(run, "a").await;
    assert_eq!(w.scheduler.tick().await.retries_released, 1);
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Ready));

    let replacement = w.session(&["act.a", "act.b"]).await;
    let second = w.one_task(&replacement).await;
    assert_eq!(
        (second.fence.attempt, second.fence.generation),
        (2, 2),
        "a new generation"
    );
    assert_ne!(second.lease_token.expose(), first.lease_token.expose());

    // The dead worker comes back with its old lease: refused, and the run is untouched.
    assert_eq!(
        w.complete(&doomed, &first, &json!({"late": true}))
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    assert_eq!(
        w.workers
            .heartbeat(&doomed.principal, &crate::support::engine::task_ref(&first))
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Leased));

    w.complete(&replacement, &second, &json!({}))
        .await
        .expect("the new holder completes");
    let b = w.one_task(&replacement).await;
    w.complete(&replacement, &b, &json!({})).await.expect("b");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn heartbeats_keep_a_lease_alive_and_the_scheduler_leaves_it_alone() {
    let w = world().await;
    let (wf, _) = w.publish(chain("heartbeat")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;

    let beat = w
        .workers
        .heartbeat(&session.principal, &crate::support::engine::task_ref(&task))
        .await
        .expect("heartbeat");
    assert!(!beat.cancel_requested);
    assert_eq!(
        w.step_status(run, "a").await,
        Some(StepStatus::Running),
        "the first heartbeat marks the step started"
    );
    let report = w.scheduler.tick().await;
    assert_eq!(report.leases_expired, 0);

    // The lease the heartbeat pushed out lies in the future.
    let remaining: f64 = sqlx::query_scalar("SELECT extract(epoch FROM lease_expires_at - clock_timestamp())::float8 FROM steps WHERE node = 'a'")
        .fetch_one(&w.db.pool)
        .await
        .expect("remaining");
    assert!(remaining > 20.0 && remaining <= 30.0, "{remaining}");
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_attempt_that_exceeds_its_timeout_fails_even_while_heartbeats_continue() {
    let w = world().await;
    let doc = document(
        "slow-step",
        "a",
        vec![with(flaky("a", 2), "timeoutMs", json!(5000))],
    );
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.workers
        .heartbeat(&session.principal, &crate::support::engine::task_ref(&task))
        .await
        .expect("heartbeat");

    // The lease is fresh, but the attempt overran its own deadline.
    w.age_attempt_deadline(run, "a").await;
    assert_eq!(w.scheduler.tick().await.leases_expired, 1);
    let code: Option<String> =
        sqlx::query_scalar("SELECT failure_code FROM steps WHERE node = 'a'")
            .fetch_one(&w.db.pool)
            .await
            .expect("code");
    assert_eq!(code.as_deref(), Some("ATTEMPT_TIMEOUT"));
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::RetryWait));
    assert_eq!(
        w.complete(&session, &task, &json!({})).await.map(|_| ()),
        Err(AppError::StaleAttempt)
    );
}

#[tokio::test]
async fn a_lease_is_never_extended_beyond_the_attempt_deadline() {
    let w = world().await;
    let doc = document(
        "bounded",
        "a",
        vec![with(activity("a"), "timeoutMs", json!(2000))],
    );
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.workers
        .heartbeat(&session.principal, &crate::support::engine::task_ref(&task))
        .await
        .expect("heartbeat");
    let slack: f64 = sqlx::query_scalar("SELECT extract(epoch FROM attempt_deadline_at - lease_expires_at)::float8 FROM steps WHERE node = 'a'")
        .fetch_one(&w.db.pool)
        .await
        .expect("slack");
    assert!(
        slack >= 0.0,
        "the lease outlived the attempt deadline by {}s",
        -slack
    );
    let _ = run;
}

// ------------------------------------------------------------------ retries

#[tokio::test]
async fn a_retryable_failure_backs_off_and_is_retried_with_the_same_idempotency_key() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("retrying", "a", vec![flaky("a", 3)]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;

    let first = w.one_task(&session).await;
    let ack = w.fail(&session, &first, true).await.expect("fail");
    assert_eq!(ack.step_status, StepStatus::RetryWait);
    assert!(
        w.poll(&session, 8).await.is_empty(),
        "no task during the backoff"
    );

    w.age_retry(run, "a").await;
    w.scheduler.tick().await;
    let second = w.one_task(&session).await;
    assert_eq!(second.fence.attempt, 2);
    assert_eq!(
        second.idempotency_key, first.idempotency_key,
        "the external API can deduplicate the retry"
    );
    w.complete(&session, &second, &json!({}))
        .await
        .expect("succeeds on the retry");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn running_out_of_attempts_fails_the_run_with_the_workers_code() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("exhausted", "a", vec![flaky("a", 2)]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let first = w.one_task(&session).await;
    w.fail(&session, &first, true).await.expect("fail 1");
    w.age_retry(run, "a").await;
    w.scheduler.tick().await;
    let second = w.one_task(&session).await;
    let ack = w.fail(&session, &second, true).await.expect("fail 2");
    assert_eq!(
        (ack.step_status, ack.run_status),
        (StepStatus::Failed, RunStatus::Failed)
    );
    let code: Option<String> = sqlx::query_scalar("SELECT stop_code FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("code");
    assert_eq!(code.as_deref(), Some("PAYMENT_DECLINED"));
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_non_retryable_failure_takes_the_error_route_and_the_run_still_succeeds() {
    let w = world().await;
    let doc = document(
        "error-route",
        "a",
        vec![
            with(
                with(flaky("a", 5), "next", json!("b")),
                "onError",
                json!("cleanup"),
            ),
            activity("b"),
            activity("cleanup"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b", "act.cleanup"]).await;
    let a = w.one_task(&session).await;
    w.fail(&session, &a, false).await.expect("fail");
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Failed));
    assert_eq!(w.step_status(run, "b").await, None);
    let cleanup = w.one_task(&session).await;
    assert_eq!(cleanup.instance.node().as_str(), "cleanup");
    w.complete(&session, &cleanup, &json!({}))
        .await
        .expect("cleanup");
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Succeeded,
        "a handled failure is not a failed run"
    );
    w.assert_consistent(run).await;
}

// ------------------------------------------------------------------ US03: waiting costs nothing

#[tokio::test]
async fn a_one_hour_timer_holds_no_worker_and_resumes_when_due() {
    let w = world().await;
    let doc = document(
        "sleepy",
        "pause",
        vec![
            with(
                json!({"id": "pause", "type": "timer", "durationMs": 3_600_000}),
                "next",
                json!("after"),
            ),
            activity("after"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;

    assert_eq!(w.run_status(run).await, RunStatus::Waiting);
    assert_eq!(w.step_status(run, "pause").await, Some(StepStatus::Waiting));
    assert_eq!(w.step_status(run, "after").await, None);
    let due_in: f64 = sqlx::query_scalar("SELECT extract(epoch FROM due_at - clock_timestamp())::float8 FROM steps WHERE node = 'pause'")
        .fetch_one(&w.db.pool)
        .await
        .expect("due");
    assert!((3590.0..=3600.0).contains(&due_in), "{due_in}");

    // No process is allocated: nothing is claimable, no lease exists, and the scheduler has nothing to do.
    let session = w.session(&["act.after"]).await;
    assert!(w.poll(&session, 8).await.is_empty());
    let leased: i64 =
        sqlx::query_scalar("SELECT count(*) FROM steps WHERE status IN ('LEASED', 'RUNNING')")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(leased, 0);
    assert!(
        !w.scheduler.tick().await.did_work(),
        "a timer that is not due is left alone"
    );

    // An hour later (aged) the scheduler creates the task.
    w.age_due(run, "pause").await;
    assert_eq!(w.scheduler.tick().await.timers_fired, 1);
    assert_eq!(w.run_status(run).await, RunStatus::Running);
    let task = w.one_task(&session).await;
    assert_eq!(task.instance.node().as_str(), "after");
    w.complete(&session, &task, &json!({}))
        .await
        .expect("after");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn two_hundred_suspended_runs_are_two_hundred_rows_and_wake_in_one_pass_each() {
    let w = world().await;
    let doc = document(
        "many-sleepers",
        "pause",
        vec![
            with(
                json!({"id": "pause", "type": "timer", "durationMs": 3_600_000}),
                "next",
                json!("after"),
            ),
            activity("after"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    // Create the runs through the real service, a few hundred at a time.
    let runs = 200;
    let mut tasks = Vec::new();
    for i in 0..runs {
        let w = &w;
        tasks.push(async move { w.start(wf, &json!({"i": i})).await });
    }
    let started = futures_util::future::join_all(tasks).await;
    assert_eq!(started.len(), runs);
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE status = 'WAITING' AND waiting_kind = 'TIMER'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(waiting, runs as i64);
    sqlx::query("UPDATE steps SET due_at = clock_timestamp() - interval '1 second' WHERE waiting_kind = 'TIMER'").execute(&w.db.pool).await.expect("age");
    let mut fired = 0;
    loop {
        let report = w.scheduler.tick().await;
        if report.timers_fired == 0 {
            break;
        }
        fired += report.timers_fired;
    }
    assert_eq!(fired as usize, runs);
}

// ------------------------------------------------------------------ waits and deadlines

#[tokio::test]
async fn an_event_wait_that_times_out_takes_the_timeout_route() {
    let w = world().await;
    let doc = document(
        "wait-timeout",
        "w",
        vec![
            with(
                with(
                    json!({"id": "w", "type": "waitEvent", "eventType": "payment.confirmed", "correlationExpr": "input.paymentId", "timeoutMs": 300_000, "onTimeout": "review"}),
                    "next",
                    json!("notify"),
                ),
                "timeoutMs",
                json!(300_000),
            ),
            activity("notify"),
            activity("review"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({"paymentId": "p-7"})).await;
    assert_eq!(w.step_status(run, "w").await, Some(StepStatus::Waiting));
    let key: Option<String> =
        sqlx::query_scalar("SELECT correlation_key FROM steps WHERE node = 'w'")
            .fetch_one(&w.db.pool)
            .await
            .expect("key");
    assert_eq!(
        key.as_deref(),
        Some("p-7"),
        "the correlation key was evaluated from the run input"
    );

    assert!(!w.scheduler.tick().await.did_work());
    w.age_due(run, "w").await;
    assert_eq!(w.scheduler.tick().await.waits_timed_out, 1);
    let session = w.session(&["act.notify", "act.review"]).await;
    assert_eq!(
        w.one_task(&session).await.instance.node().as_str(),
        "review"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_run_past_its_deadline_is_timed_out_and_its_worker_is_told_to_stop() {
    let w = world().await;
    let (wf, _) = w.publish(chain("deadline")).await;
    let deadline = time::OffsetDateTime::now_utc() + time::Duration::hours(1);
    let run = w
        .start_with(wf, &json!({}), "deadline-key-1", Some(deadline))
        .await
        .expect("start");
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;

    w.age_run_deadline(run).await;
    assert_eq!(w.scheduler.tick().await.deadlines_enforced, 1);
    assert_eq!(
        w.run_status(run).await,
        RunStatus::Cancelling,
        "the leased attempt must acknowledge first"
    );
    let beat = w
        .workers
        .heartbeat(&session.principal, &crate::support::engine::task_ref(&task))
        .await
        .expect("heartbeat");
    assert!(beat.cancel_requested);
    w.fail(&session, &task, true)
        .await
        .expect("the worker acknowledges");
    assert_eq!(w.run_status(run).await, RunStatus::TimedOut);
    assert_eq!(
        w.step_status(run, "a").await,
        Some(StepStatus::Cancelled),
        "a stopped attempt is cancelled, not retried"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_worker_that_vanishes_during_cancellation_does_not_keep_the_run_open() {
    let w = world().await;
    let (wf, _) = w.publish(chain("vanishing")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let _task = w.one_task(&session).await;
    w.operate(run, sagat_application::runs::RunOperation::Cancel)
        .await
        .expect("cancel");
    assert_eq!(w.run_status(run).await, RunStatus::Cancelling);

    w.age_lease(run, "a").await;
    w.scheduler.tick().await;
    assert_eq!(w.run_status(run).await, RunStatus::Cancelled);
    w.assert_consistent(run).await;
}

// ------------------------------------------------------------------ the scheduler itself

#[tokio::test]
async fn several_schedulers_at_once_apply_each_transition_exactly_once() {
    let w = world().await;
    let doc = document(
        "contended",
        "pause",
        vec![
            with(
                json!({"id": "pause", "type": "timer", "durationMs": 1000}),
                "next",
                json!("after"),
            ),
            activity("after"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;
    w.age_due(run, "pause").await;

    let ticks = futures_util::future::join_all((0..8).map(|_| w.scheduler.tick())).await;
    let fired: u32 = ticks.iter().map(|r| r.timers_fired).sum();
    assert_eq!(fired, 1, "exactly one scheduler fired the timer: {ticks:?}");
    let timer_events = w
        .journal(run)
        .await
        .iter()
        .filter(|e| *e == "timer.fired")
        .count();
    assert_eq!(timer_events, 1);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_ready_step_whose_notification_was_lost_gets_a_new_one() {
    let w = world().await;
    let (wf, _) = w.publish(chain("rehint")).await;
    let run = w.start(wf, &json!({})).await;
    assert_eq!(w.outbox("task.ready").await, 1);
    assert!(
        !w.scheduler.tick().await.did_work(),
        "a fresh notification is not repeated"
    );

    w.age_hint(run, "a").await;
    assert_eq!(w.scheduler.tick().await.hints_renewed, 1);
    assert_eq!(w.outbox("task.ready").await, 2);
    // Claiming never depended on the notification.
    let session = w.session(&["act.a", "act.b"]).await;
    assert_eq!(w.one_task(&session).await.instance.node().as_str(), "a");
}

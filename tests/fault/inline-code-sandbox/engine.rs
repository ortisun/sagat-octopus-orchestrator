//! The container engine cannot be used, and the control plane cannot be reached: failures that are
//! the platform's, not the script's, and that must read as such.

use reqwest::Method;
use sagat_domain::scripts::Failure;
use sagat_sandbox_runner::StepOutcome;
use serde_json::json;
use std::time::Duration;

use crate::support::api::Options;
use crate::support::scripts::{Fault, Faulty, lab_configured, lab_on};

const FAST_RETRY: &str = r#"{"maxAttempts": 2, "initialDelayMs": 1, "maxDelayMs": 1, "strategy": "fixed", "jitter": false}"#;

#[tokio::test]
async fn a_build_the_engine_cannot_do_is_tried_three_times_and_then_says_it_was_the_sandbox() {
    let lab = lab_on(Options::default(), Faulty(Fault::Down), |_| {}).await;
    let script = lab
        .script("never-built", "typescript", "export default () => 1\n")
        .await;
    let (status, published) = lab
        .call(
            Method::POST,
            &format!("/namespaces/{}/scripts/{script}/versions", lab.ns),
            None,
        )
        .await;
    assert_eq!(status, 202, "{published}");
    for attempt in 1..=3 {
        assert!(lab.runner.build_once().await.expect("claimed and reported"));
        if attempt < 3 {
            // A build that failed for a reason that may pass waits before it is offered again.
            assert!(
                !lab.runner.build_once().await.expect("nothing yet"),
                "attempt {attempt} waits"
            );
            sqlx::query(
                "UPDATE script_build_jobs SET not_before = clock_timestamp() - interval '1 second'",
            )
            .execute(&lab.p.api.db.pool)
            .await
            .expect("let the wait pass");
        }
    }
    assert!(
        !lab.runner.build_once().await.expect("nothing left"),
        "three attempts, then it is over"
    );
    let (_, version) = lab
        .call(
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}/versions/1", lab.ns),
            None,
        )
        .await;
    assert_eq!(version["status"], "FAILED", "{version}");
    assert!(
        version["failure"]
            .as_str()
            .expect("failure")
            .starts_with("unavailable:"),
        "{version}"
    );
}

#[tokio::test]
async fn a_test_run_on_a_broken_engine_is_a_retryable_failure_of_the_platform_not_of_the_script() {
    let lab = lab_on(Options::default(), Faulty(Fault::Down), |_| {}).await;
    let script = lab
        .script("cannot-test", "typescript", "export default () => 1\n")
        .await;
    let run = lab.test(&script, &json!({})).await;
    assert_eq!(run["status"], "FAILED", "{run}");
    assert_eq!(run["failure"], "SCRIPT_SANDBOX_UNAVAILABLE", "{run}");
    assert_eq!(run["retryable"], true, "{run}");
}

#[tokio::test]
async fn a_step_the_engine_cannot_run_is_retried_and_ends_as_the_sandbox_unavailable() {
    let lab = lab_on(Options::default(), Faulty(Fault::CannotRun), |_| {}).await;
    let script = lab
        .script("cannot-run", "typescript", "export default () => 1\n")
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab
        .workflow(
            "wf-cannot-run",
            &digest,
            json!({"retry": serde_json::from_str::<serde_json::Value>(FAST_RETRY).expect("json")}),
        )
        .await;
    let run = lab.p.start(&workflow, &json!({})).await;
    // The first attempt fails retryably; the engine schedules another.
    lab.p.tick().await;
    let namespace = lab.runner.config().namespaces[0].clone();
    let step = lab
        .runner
        .poll_steps(&namespace)
        .await
        .expect("poll")
        .remove(0);
    assert_eq!(
        lab.runner.run_step(step).await.expect("run"),
        StepOutcome::Failed(Failure::Unavailable)
    );
    lab.p.tick().await;
    assert_eq!(lab.p.status(&run).await, "RUNNING", "a retry is waiting");
    lab.p.elapse_retry_delay(&run, "s").await;
    lab.p.tick().await;
    let step = lab
        .runner
        .poll_steps(&namespace)
        .await
        .expect("poll again")
        .remove(0);
    assert_eq!(
        lab.runner.run_step(step).await.expect("run"),
        StepOutcome::Failed(Failure::Unavailable)
    );
    lab.p.tick().await;
    assert_eq!(lab.p.status(&run).await, "FAILED");
    assert_eq!(
        lab.failure(&run, "s").await.as_deref(),
        Some("SCRIPT_SANDBOX_UNAVAILABLE")
    );
}

#[tokio::test]
async fn a_control_plane_that_cannot_be_reached_is_an_error_to_report_and_not_a_crash() {
    let lab = lab_configured(Options::default(), |config| {
        // Nothing listens here.
        config.api_url = "http://127.0.0.1:9/v1".to_owned();
        config.grpc_url = "http://127.0.0.1:9".to_owned();
    })
    .await;
    assert!(lab.runner.build_once().await.is_err());
    assert!(lab.runner.test_once().await.is_err());
    let namespace = lab.runner.config().namespaces[0].clone();
    assert!(lab.runner.poll_steps(&namespace).await.is_err());
    // The serving loop survives it, keeps asking, and stops when it is told to.
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(std::sync::Arc::clone(&lab.runner).serve(async move {
        let _ = stopped.await;
    }));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!serving.is_finished(), "still trying");
    stop.send(()).expect("stop");
    tokio::time::timeout(Duration::from_secs(10), serving)
        .await
        .expect("it stops")
        .expect("without panicking");
}

#[tokio::test]
async fn a_wrong_credential_is_refused_and_does_nothing() {
    let lab = lab_configured(Options::default(), |config| {
        config.credentials =
            sagat_sandbox_runner::api::Credentials::Static("not.a.token".to_owned());
    })
    .await;
    let error = lab.runner.build_once().await.expect_err("refused");
    assert!(
        matches!(
            error,
            sagat_sandbox_runner::api::RunnerError::Status { status: 401, .. }
        ),
        "{error}"
    );
}

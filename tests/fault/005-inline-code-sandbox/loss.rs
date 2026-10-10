//! A runner that vanishes: what it held goes to another, and what it says afterwards changes
//! nothing.

use reqwest::Method;
use sagat_application::worker::WorkerSettings;
use sagat_sandbox_runner::StepOutcome;
use serde_json::json;
use std::time::Duration;

use crate::support::api::Options;
use crate::support::scripts::{lab, lab_with};

const FAST_RETRY: &str = r#"{"maxAttempts": 3, "initialDelayMs": 1, "maxDelayMs": 1, "strategy": "fixed", "jitter": false}"#;

#[tokio::test]
async fn a_build_whose_runner_dies_is_taken_over_and_what_the_dead_one_says_late_changes_nothing() {
    let lab = lab().await;
    let script = lab
        .script(
            "dying-build",
            "typescript",
            "export default () => ({ ok: true });\n",
        )
        .await;
    let (status, published) = lab
        .call(
            Method::POST,
            &format!("/namespaces/{}/scripts/{script}/versions", lab.ns),
            None,
        )
        .await;
    assert_eq!(status, 202, "{published}");
    // The first runner takes the build, and is never heard from again.
    let lost = lab
        .runner
        .claim_build()
        .await
        .expect("claim")
        .expect("a build is waiting");
    sqlx::query(
        "UPDATE script_build_jobs SET lease_expires_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&lab.p.api.db.pool)
    .await
    .expect("age the lease");
    assert!(
        lab.runner
            .build_once()
            .await
            .expect("another runner takes it")
    );
    let (_, version) = lab
        .call(
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}/versions/1", lab.ns),
            None,
        )
        .await;
    assert_eq!(version["status"], "READY", "{version}");
    let digest = version["artifactDigest"].clone();
    // The dead runner wakes up and reports: refused, and the version is as the other left it.
    lab.runner
        .process_build(lost)
        .await
        .expect("a stale report is not an error");
    let (_, again) = lab
        .call(
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}/versions/1", lab.ns),
            None,
        )
        .await;
    assert_eq!(again["artifactDigest"], digest);
    let artifacts: i64 = sqlx::query_scalar("SELECT count(*) FROM script_artifacts")
        .fetch_one(&lab.p.api.db.pool)
        .await
        .expect("count");
    assert_eq!(artifacts, 1);
}

#[tokio::test]
async fn a_test_run_whose_runner_dies_is_run_again_by_another() {
    let lab = lab().await;
    let script = lab
        .script(
            "dying-test",
            "typescript",
            "export default (i: any) => ({ got: i });\n",
        )
        .await;
    let (status, queued) = lab
        .call(
            Method::POST,
            &format!("/namespaces/{}/scripts/{script}/test-runs", lab.ns),
            Some(&json!({"input": {"n": 1}})),
        )
        .await;
    assert_eq!(status, 202, "{queued}");
    let _lost = lab
        .runner
        .claim_test()
        .await
        .expect("claim")
        .expect("a test run");
    sqlx::query(
        "UPDATE script_test_runs SET lease_expires_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&lab.p.api.db.pool)
    .await
    .expect("age");
    assert!(lab.runner.test_once().await.expect("taken over"));
    let id = queued["id"].as_str().expect("id");
    let (_, done) = lab
        .call(
            Method::GET,
            &format!("/namespaces/{}/scripts/{script}/test-runs/{id}", lab.ns),
            None,
        )
        .await;
    assert_eq!(done["status"], "SUCCEEDED", "{done}");
    assert_eq!(done["result"], json!({"got": {"n": 1}}));
}

#[tokio::test]
async fn a_step_whose_runner_dies_runs_again_elsewhere_and_the_late_attempt_is_dropped() {
    let options = Options {
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(5),
            ..WorkerSettings::default()
        },
        ..Options::default()
    };
    let lab = lab_with(options).await;
    let script = lab
        .script(
            "twice",
            "typescript",
            "export default function (_i: unknown, octopus: any) {\n  console.log('key=' + octopus.idempotencyKey);\n  return { key: octopus.idempotencyKey };\n}\n",
        )
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab
        .workflow(
            "wf-twice",
            &digest,
            json!({"retry": serde_json::from_str::<serde_json::Value>(FAST_RETRY).expect("json")}),
        )
        .await;
    let run = lab.p.start(&workflow, &json!({})).await;
    lab.p.tick().await;
    let namespace = lab.runner.config().namespaces[0].clone();
    let mut first = lab.runner.poll_steps(&namespace).await.expect("poll");
    assert_eq!(first.len(), 1);
    let slow = first.remove(0);
    // The runner holding it goes quiet. The engine takes the step back and offers it again.
    lab.p.expire_lease(&run, "s").await;
    lab.p.tick().await;
    lab.p.elapse_retry_delay(&run, "s").await;
    lab.p.tick().await;
    let mut second = lab.runner.poll_steps(&namespace).await.expect("poll again");
    assert_eq!(second.len(), 1, "the step is offered again");
    let retry = second.remove(0);
    assert_eq!(
        lab.runner.run_step(retry).await.expect("run"),
        StepOutcome::Completed
    );
    lab.p.tick().await;
    assert_eq!(lab.p.status(&run).await, "SUCCEEDED");
    let kept = lab.output(&run, "s").await.expect("output");

    // The first attempt finally gets to run, and its report is dropped: the run already has an answer.
    assert_eq!(
        lab.runner.run_step(slow).await.expect("late"),
        StepOutcome::Abandoned
    );
    assert_eq!(lab.output(&run, "s").await, Some(kept.clone()));
    // A script runs at least once per attempt, and may run more than once per step; what makes an
    // external effect safe to repeat is the key it can put in the request, the same on every attempt.
    let key = kept["key"].as_str().expect("the key").to_owned();
    assert_eq!(key, format!("{run}:s"));
    // The attempt that lost its lease is stopped at the next beat of the lease if it is still
    // running, and recorded if it finished before: either way the record that exists says what
    // ran, with the key.
    let executions = lab.executions(&digest).await;
    assert!(
        (1..=2).contains(&executions.len()),
        "the second attempt is on record, the first if it got to finish: {executions:?}"
    );
    for execution in executions {
        assert!(
            execution["log"].as_str().expect("log").contains(&key),
            "{execution}"
        );
    }
}

#[tokio::test]
async fn a_runner_that_restarts_serves_what_the_one_before_it_never_started() {
    let mut lab = lab().await;
    let script = lab
        .script(
            "after-restart",
            "python",
            "def run(input, octopus):\n    return {'ok': True}\n",
        )
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab
        .workflow(
            "wf-restart",
            &digest,
            json!({"retry": serde_json::from_str::<serde_json::Value>(FAST_RETRY).expect("json")}),
        )
        .await;
    let run = lab.p.start(&workflow, &json!({})).await;
    lab.p.tick().await;
    // The first runner registers its session and leases the step ... and is killed before it runs it.
    let namespace = lab.runner.config().namespaces[0].clone();
    assert_eq!(
        lab.runner.poll_steps(&namespace).await.expect("poll").len(),
        1
    );
    lab.restart().await;
    lab.p.expire_lease(&run, "s").await;
    lab.p.tick().await;
    lab.p.elapse_retry_delay(&run, "s").await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    assert_eq!(lab.output(&run, "s").await, Some(json!({"ok": true})));
}

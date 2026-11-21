//! A runner that vanishes: builds and test runs can be taken over, but an unknown workflow
//! effect waits for review. What the stale runner says afterwards changes nothing.

use reqwest::Method;
use sagat_application::worker::WorkerSettings;
use sagat_sandbox_runner::StepOutcome;
use serde_json::json;
use std::time::Duration;

use crate::support::api::Options;
use crate::support::scripts::{Lab, lab, lab_with};

const FAST_RETRY: &str = r#"{"maxAttempts": 3, "initialDelayMs": 1, "maxDelayMs": 1, "strategy": "fixed", "jitter": false}"#;

async fn assert_lost_step_is_retained(lab: &Lab, run: &str) {
    assert_eq!(lab.p.status(run).await, "MANUAL_REVIEW");
    let waiting: (String, Option<String>, Option<String>, i32, bool) = sqlx::query_as(
        "SELECT status,waiting_kind,failure_code,attempt,next_attempt_at IS NULL \
         FROM steps WHERE run_id=$1 AND node='s'",
    )
    .bind(uuid::Uuid::parse_str(run).expect("run id"))
    .fetch_one(&lab.p.api.db.pool)
    .await
    .expect("retained script obligation");
    assert_eq!(
        waiting,
        (
            "WAITING".into(),
            Some("OPERATOR".into()),
            Some("LEASE_EXPIRED".into()),
            1,
            true,
        ),
        "an unknown script outcome consumes no second attempt or retry timer"
    );
    assert!(lab.output(run, "s").await.is_none());
    let finished: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE aggregate_id=$1 AND event_type='run.finished'",
    )
    .bind(uuid::Uuid::parse_str(run).expect("run id"))
    .fetch_one(&lab.p.api.db.pool)
    .await
    .expect("terminal event count");
    assert_eq!(finished, 0, "uncertainty is not a terminal outcome");
}

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
            "csharp",
            r#"
using System.Text.Json;
using Sagat.ScriptContract;

public sealed class Script : IInlineScript
{
    public ValueTask<JsonElement> RunAsync(JsonElement input, IOctopus octopus, CancellationToken cancellationToken)
        => ValueTask.FromResult(JsonSerializer.SerializeToElement(new { got = input }));
}
"#,
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
async fn a_step_whose_runner_dies_waits_for_review_and_the_late_attempt_is_dropped() {
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
    assert_eq!(slow.lease.idempotency_key, format!("{run}:s"));
    // The runner goes quiet. The engine cannot know whether the script had an external effect,
    // even though this fixture has not called run_step yet and the retry budget allows more.
    lab.p.expire_lease(&run, "s").await;
    lab.p.tick().await;
    assert_lost_step_is_retained(&lab, &run).await;
    lab.p.tick().await;
    assert!(
        lab.runner
            .poll_steps(&namespace)
            .await
            .expect("poll again")
            .is_empty(),
        "review never offers an automatic repeat"
    );
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id=$1")
        .bind(uuid::Uuid::parse_str(&run).expect("run id"))
        .fetch_one(&lab.p.api.db.pool)
        .await
        .expect("journal before the stale runner");

    // The stale runner cannot acquire script capabilities or settle the retained obligation.
    assert_eq!(
        lab.runner.run_step(slow).await.expect("late"),
        StepOutcome::Abandoned
    );
    lab.p.tick().await;
    assert_lost_step_is_retained(&lab, &run).await;
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id=$1")
        .bind(uuid::Uuid::parse_str(&run).expect("run id"))
        .fetch_one(&lab.p.api.db.pool)
        .await
        .expect("journal after the stale runner");
    assert_eq!(after, before, "a stale attempt cannot rewrite the journal");
    assert!(lab.executions(&digest).await.is_empty());

    // Stopping future work cannot turn the lost attempt into a known no-effect outcome.
    let (status, accepted) = lab
        .p
        .rest(
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(
                &json!({"action": "CANCEL", "reason": "stop while the script outcome is unknown"}),
            ),
        )
        .await;
    assert_eq!(status, 202, "{accepted}");
    lab.p.tick().await;
    assert_lost_step_is_retained(&lab, &run).await;
    assert!(
        lab.runner
            .poll_steps(&namespace)
            .await
            .expect("poll after cancellation")
            .is_empty()
    );
}

#[tokio::test]
async fn a_runner_that_restarts_preserves_unknown_work_and_serves_new_work() {
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
    assert_lost_step_is_retained(&lab, &run).await;
    assert!(
        lab.runner
            .poll_steps(&namespace)
            .await
            .expect("poll after restart")
            .is_empty(),
        "a new runner session does not make the old outcome known"
    );
    assert!(lab.executions(&digest).await.is_empty());

    // Restart still restores service for new work; it does not recover by repeating the lost work.
    let fresh = lab.p.start(&workflow, &json!({})).await;
    assert_eq!(lab.drive(&fresh).await, "SUCCEEDED");
    assert_eq!(lab.output(&fresh, "s").await, Some(json!({"ok": true})));
    assert_eq!(lab.executions(&digest).await.len(), 1);
    assert_lost_step_is_retained(&lab, &run).await;
}

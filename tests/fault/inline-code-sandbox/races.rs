//! A run is cancelled while its script is running, and a runner is asked for more than it has room
//! for.

use reqwest::Method;
use sagat_application::worker::WorkerSettings;
use sagat_sandbox_runner::StepOutcome;
use serde_json::json;
use std::time::Duration;

use crate::support::api::Options;
use crate::support::scripts::{lab_configured, lab_with};

#[tokio::test]
async fn cancelling_a_run_stops_its_script_and_leaves_no_container() {
    let options = Options {
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(3),
            ..WorkerSettings::default()
        },
        // The script asks for two minutes: the platform must allow them.
        script_ceiling: sagat_domain::scripts::Policy {
            limits: sagat_domain::scripts::Limits::new(
                256 * 1024 * 1024,
                1000,
                Duration::from_secs(300),
                64,
                64 * 1024,
                32 * 1024 * 1024,
            )
            .expect("limits"),
            ..sagat_domain::scripts::Policy::default()
        },
        ..Options::default()
    };
    let lab = lab_with(options).await;
    let script = lab
        .script_with(
            "sleeper",
            "python",
            "import time\ndef run(input, octopus):\n    time.sleep(120)\n    return {'woke': True}\n",
            json!({"policy": {"timeoutMs": 120000}}),
        )
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab.workflow("wf-sleeper", &digest, json!({})).await;
    let run = lab.p.start(&workflow, &json!({})).await;
    lab.p.tick().await;
    let namespace = lab.runner.config().namespaces[0].clone();
    let step = lab
        .runner
        .poll_steps(&namespace)
        .await
        .expect("poll")
        .remove(0);
    let runner = std::sync::Arc::clone(&lab.runner);
    let running = tokio::spawn(async move { runner.run_step(step).await });
    // The script is asleep in its container.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(!running.is_finished());

    let (status, accepted) = lab
        .p
        .rest(
            Method::POST,
            &format!("/runs/{run}/operations"),
            Some(&json!({"action": "CANCEL", "reason": "no longer needed"})),
        )
        .await;
    assert_eq!(status, 202, "{accepted}");
    // The next heartbeat tells the runner; it stops the script and reports nothing.
    let outcome = tokio::time::timeout(Duration::from_secs(20), running)
        .await
        .expect("the runner notices the cancellation")
        .expect("no panic")
        .expect("no error");
    assert_eq!(outcome, StepOutcome::Abandoned);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        lab.runner.sandbox().leftovers().await,
        0,
        "the container went with the step"
    );
    // The engine finishes the cancellation when it has seen that nothing holds the step any more.
    for _ in 0..5 {
        lab.p.tick().await;
        if lab.p.status(&run).await == "CANCELLED" {
            break;
        }
    }
    assert_eq!(lab.p.status(&run).await, "CANCELLED");
    assert!(
        lab.output(&run, "s").await.is_none(),
        "a cancelled script produced nothing"
    );
}

#[tokio::test]
async fn a_runner_takes_no_more_steps_than_it_has_room_to_run() {
    let lab = lab_configured(Options::default(), |config| config.max_concurrent = 1).await;
    let script = lab
        .script(
            "busy",
            "typescript",
            "export default () => ({ done: true });\n",
        )
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab.workflow("wf-busy", &digest, json!({})).await;
    let runs = [
        lab.p.start(&workflow, &json!({"n": 1})).await,
        lab.p.start(&workflow, &json!({"n": 2})).await,
        lab.p.start(&workflow, &json!({"n": 3})).await,
    ];
    lab.p.tick().await;
    let namespace = lab.runner.config().namespaces[0].clone();
    let first = lab.runner.poll_steps(&namespace).await.expect("poll");
    assert_eq!(first.len(), 1, "room for one");
    assert!(
        lab.runner
            .poll_steps(&namespace)
            .await
            .expect("poll")
            .is_empty(),
        "the room is taken when the step is leased, not when it starts: nothing more is leased \
         while the first one waits"
    );
    // While it runs the others wait on the queue, leased to nobody.
    for step in first {
        lab.runner.run_step(step).await.expect("run");
    }
    let mut finished = 1;
    while finished < 3 {
        for step in lab.runner.poll_steps(&namespace).await.expect("poll") {
            lab.runner.run_step(step).await.expect("run");
            finished += 1;
        }
    }
    for run in &runs {
        lab.p.tick().await;
        assert_eq!(lab.p.status(run).await, "SUCCEEDED");
    }
}

#[tokio::test]
async fn scripts_that_become_ready_at_different_times_are_all_served_by_the_one_session() {
    let lab = lab_with(Options::default()).await;
    let a = lab
        .script(
            "first",
            "typescript",
            "export default () => ({ who: 'a' });\n",
        )
        .await;
    let b = lab
        .script(
            "second",
            "python",
            "def run(input, octopus):\n    return {'who': 'b'}\n",
        )
        .await;
    let (digest_a, digest_b) = (lab.ready(&a).await, lab.ready(&b).await);
    let wf_a = lab.workflow("wf-a", &digest_a, json!({})).await;
    let wf_b = lab.workflow("wf-b", &digest_b, json!({})).await;
    let namespace = lab.runner.config().namespaces[0].clone();

    // Only the first script has a step ready: the runner registers for it.
    let run_a1 = lab.p.start(&wf_a, &json!({})).await;
    lab.p.tick().await;
    let held = lab.runner.poll_steps(&namespace).await.expect("poll");
    assert_eq!(held.len(), 1);

    // Then the second one has too, and so does another step of the first. The engine keeps one
    // session per name and a registration under it replaces the last: if the runner kept two, the
    // first would be dead and this poll would get nothing.
    let run_b = lab.p.start(&wf_b, &json!({})).await;
    let run_a2 = lab.p.start(&wf_a, &json!({})).await;
    lab.p.tick().await;
    let both = lab.runner.poll_steps(&namespace).await.expect("poll");
    assert_eq!(both.len(), 2, "a step of each script");

    for step in held.into_iter().chain(both) {
        lab.runner.run_step(step).await.expect("run");
    }
    for run in [&run_a1, &run_b, &run_a2] {
        lab.p.tick().await;
        assert_eq!(lab.p.status(run).await, "SUCCEEDED", "{run}");
    }
    assert_eq!(lab.output(&run_b, "s").await, Some(json!({"who": "b"})));
}

//! A worker offers the newest minor version of protocol 1 it speaks and the server answers with the
//! newest both speak, so an SDK can be upgraded before or after the server (US04). A different major
//! version, or something that is not a version, is refused as an invalid argument.

use sagat_api_grpc::pb::{HeartbeatRequest, TaskLease};
use serde_json::json;
use tonic::Code;

use crate::support::worker::{Worker, machine_code, platform, single_ir};

/// The newest minor version of protocol 1 this server speaks (`sagat_application::worker`).
const SERVER_MINOR: u32 = sagat_application::worker::PROTOCOL_MINOR;

async fn accepted_for(offered: &str) -> Result<String, tonic::Status> {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    let mut message = Worker::register_message(&["act.a"], 10);
    message
        .capabilities
        .as_mut()
        .expect("capabilities")
        .protocol_version = offered.into();
    w.client
        .register_worker(w.req(message))
        .await
        .map(|response| response.into_inner().accepted_protocol_version)
}

fn version(minor: u32) -> String {
    if minor == 0 {
        "1".to_owned()
    } else {
        format!("1.{minor}")
    }
}

#[tokio::test]
async fn a_worker_of_an_older_minor_version_is_served_at_its_own() {
    assert_eq!(accepted_for("1").await.expect("1"), "1");
    assert_eq!(accepted_for("1.0").await.expect("1.0"), "1");
    assert_eq!(accepted_for("1.1").await.expect("1.1"), "1.1");
}

#[tokio::test]
async fn a_worker_of_the_servers_own_version_gets_it() {
    let own = version(SERVER_MINOR);
    assert_eq!(accepted_for(&own).await.expect("own"), own);
}

#[tokio::test]
async fn a_worker_of_a_newer_minor_version_is_served_at_the_servers() {
    let own = version(SERVER_MINOR);
    for newer in [SERVER_MINOR + 1, SERVER_MINOR + 7, 999] {
        assert_eq!(
            accepted_for(&format!("1.{newer}")).await.expect("newer"),
            own,
            "1.{newer}"
        );
    }
}

#[tokio::test]
async fn another_major_version_or_a_malformed_one_is_refused() {
    let p = platform().await;
    let mut w = p.worker("wendy").await;
    for bad in [
        "",
        "2",
        "0",
        "2.0",
        "1.",
        "1.x",
        "01",
        "1.01",
        "1.2.3",
        " 1",
        "v1",
        "1,2",
        "1.-1",
        "1.99999999999",
    ] {
        let mut message = Worker::register_message(&["act.a"], 10);
        message
            .capabilities
            .as_mut()
            .expect("capabilities")
            .protocol_version = bad.into();
        let status = w
            .client
            .register_worker(w.req(message))
            .await
            .expect_err(bad);
        assert_eq!(status.code(), Code::InvalidArgument, "{bad:?}: {status:?}");
        assert_eq!(machine_code(&status), "UNSUPPORTED_PROTOCOL", "{bad:?}");
    }
}

/// What a worker of protocol 1.0 sends: the heartbeat that has neither the run, the step nor the attempt
/// (all three came with 1.1), only the task, the generation and the lease token.
fn heartbeat_of_1_0(worker: &Worker, lease: &TaskLease) -> HeartbeatRequest {
    let mut beat = worker.heartbeat_message(lease);
    beat.run_id.clear();
    beat.step_id.clear();
    beat.attempt = 0;
    beat
}

#[tokio::test]
async fn a_worker_of_protocol_1_0_keeps_its_lease_with_a_heartbeat_that_has_no_step_identity() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    let lease = w.one_task().await;

    let beat = w
        .client
        .heartbeat(w.req(heartbeat_of_1_0(&w, &lease)))
        .await
        .expect("a heartbeat of 1.0 is served")
        .into_inner();
    assert!(beat.lease_expires_at_epoch_ms >= lease.expires_at_epoch_ms);
    assert!(!beat.cancel_requested);

    // It is a heartbeat like any other: the first one starts the step and the holder completes it.
    let done = w
        .complete(&lease, &json!({"ok": true}))
        .await
        .expect("complete");
    assert_eq!(done.step_status, "SUCCEEDED");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}

#[tokio::test]
async fn a_heartbeat_of_1_0_is_held_to_the_lease_token_and_the_generation_all_the_same() {
    let p = platform().await;
    let wf = p.publish("single", &single_ir("single", None)).await;
    p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(&["act.a"]).await;
    let lease = w.one_task().await;

    let mut wrong_token = heartbeat_of_1_0(&w, &lease);
    wrong_token.lease_token = "f".repeat(64);
    let status = w
        .client
        .heartbeat(w.req(wrong_token))
        .await
        .expect_err("wrong token");
    assert_eq!(status.code(), Code::Aborted);
    assert_eq!(machine_code(&status), "STALE_ATTEMPT");

    let mut future = heartbeat_of_1_0(&w, &lease);
    future.generation = lease.generation + 1;
    future.task_id = format!("{}/a/{}", lease.run_id, future.generation);
    let status = w
        .client
        .heartbeat(w.req(future))
        .await
        .expect_err("another generation");
    assert_eq!(status.code(), Code::Aborted);
    assert_eq!(machine_code(&status), "STALE_ATTEMPT");

    // An attempt that is not a number is not "absent".
    let mut negative = heartbeat_of_1_0(&w, &lease);
    negative.attempt = -1;
    let status = w
        .client
        .heartbeat(w.req(negative))
        .await
        .expect_err("negative attempt");
    assert_eq!(status.code(), Code::InvalidArgument);

    // None of that touched the lease: the holder goes on.
    w.client
        .heartbeat(w.req(w.heartbeat_message(&lease)))
        .await
        .expect("the holder's heartbeat of 1.1");
}

/// The value of `worker_registrations_total{protocol="…"}` on the engine's admin listener.
async fn registrations_at(engine: &crate::support::worker::RunningEngine, protocol: &str) -> u64 {
    let text = engine.body("/metrics").await;
    let wanted = format!("worker_registrations_total{{protocol=\"{protocol}\"}} ");
    text.lines()
        .find_map(|line| line.strip_prefix(&wanted))
        .and_then(|value| value.trim().parse::<f64>().ok())
        .map_or(0, |value| value as u64)
}

#[tokio::test]
async fn the_versions_workers_were_served_at_are_counted_so_an_operator_knows_who_still_runs_an_old_one()
 {
    use crate::support::worker::{RunningEngine, engine_env};
    let p = platform().await;
    let engine = RunningEngine::start(engine_env(&p.api.db, &p.api.idp, &[])).await;
    let mut w = p.worker_at(&engine.grpc_url, "wendy").await;
    let (before_old, before_own) = (
        registrations_at(&engine, "1.1").await,
        registrations_at(&engine, &version(SERVER_MINOR)).await,
    );

    for (offered, accepted) in [("1.1", "1.1".to_owned()), ("1.99", version(SERVER_MINOR))] {
        let mut message = Worker::register_message(&["act.a"], 10);
        message
            .capabilities
            .as_mut()
            .expect("capabilities")
            .protocol_version = offered.into();
        let answer = w
            .client
            .register_worker(w.req(message))
            .await
            .expect(offered)
            .into_inner();
        assert_eq!(answer.accepted_protocol_version, accepted);
    }
    // A refusal is not a registration: it is in grpc_requests_total with its code.
    let mut refused = Worker::register_message(&["act.a"], 10);
    refused
        .capabilities
        .as_mut()
        .expect("capabilities")
        .protocol_version = "2".into();
    w.client
        .register_worker(w.req(refused))
        .await
        .expect_err("major 2");

    assert_eq!(registrations_at(&engine, "1.1").await, before_old + 1);
    assert_eq!(
        registrations_at(&engine, &version(SERVER_MINOR)).await,
        before_own + 1
    );
    assert_eq!(registrations_at(&engine, "2").await, 0);
}

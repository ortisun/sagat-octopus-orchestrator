//! A worker offers the newest minor version of protocol 1 it speaks and the server answers with the
//! newest both speak, so an SDK can be upgraded before or after the server (US04). A different major
//! version, or something that is not a version, is refused as an invalid argument.

use tonic::Code;

use crate::support::worker::{Worker, machine_code, platform};

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
        "", "2", "0", "2.0", "1.", "1.x", "01", "1.01", "1.2.3", " 1", "v1", "1,2", "1.-1",
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

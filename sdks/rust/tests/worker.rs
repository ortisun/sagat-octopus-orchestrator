#![allow(clippy::expect_used)]
use sagat_octopus_orchestrator_sdk::{TokenSource, WorkerHost, WorkerOptions};
#[tokio::test]
async fn invalid_worker_configuration_and_empty_registry_fail_before_connecting() {
    let mut options = WorkerOptions::new(
        "ftp://127.0.0.1:1",
        "demo",
        "rust-worker",
        TokenSource::bearer("private-token"),
    );
    assert_eq!(
        WorkerHost::new(options.clone()).expect_err("scheme").code,
        "VALIDATION_FAILED"
    );
    options.grpc_url = "http://127.0.0.1:1".into();
    options.max_inflight = 0;
    assert_eq!(
        WorkerHost::new(options.clone()).expect_err("credit").code,
        "VALIDATION_FAILED"
    );
    options.max_inflight = 10;
    let mut host = WorkerHost::new(options).expect("host");
    assert!(!format!("{host:?}").contains("private-token"));
    assert_eq!(
        host.start().await.expect_err("empty registry").code,
        "VALIDATION_FAILED"
    );
}

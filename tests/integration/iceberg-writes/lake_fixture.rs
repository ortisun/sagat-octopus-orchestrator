//! T035-004: the lake fixtures start by themselves and the fixture admin client creates what the platform
//! never creates (a bucket, a namespace, a table).

use crate::support::lake::{
    Column, EMPTY_PAYLOAD_SHA256, LakeAdmin, RestLake, Signing, hex, hmac_sha256,
    sigv4_authorization_host_date,
};

/// RFC 4231, test case 2. The signature below is only as good as this primitive.
#[test]
fn hmac_sha256_matches_the_rfc_4231_vector() {
    let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
    assert_eq!(
        hex(&mac),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

/// A key longer than the block size is hashed first (RFC 4231, test case 6).
#[test]
fn hmac_sha256_hashes_a_key_longer_than_a_block() {
    let mac = hmac_sha256(
        &[0xaa; 131],
        b"Test Using Larger Than Block-Size Key - Hash Key First",
    );
    assert_eq!(
        hex(&mac),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
}

/// The "get-vanilla" example of the AWS Signature Version 4 test suite.
#[test]
fn sigv4_matches_the_aws_get_vanilla_example() {
    let authorization = sigv4_authorization_host_date(&Signing {
        method: "GET",
        host: "example.amazonaws.com",
        path: "/",
        payload_sha256: EMPTY_PAYLOAD_SHA256,
        amz_date: "20150830T123600Z",
        access_key: "AKIDEXAMPLE",
        secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        region: "us-east-1",
        service: "service",
    });
    assert_eq!(
        authorization,
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
         SignedHeaders=host;x-amz-date, \
         Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
    );
}

/// RustFS answers `/health` before its storage layer is ready and refuses a bucket with
/// `503 Service not ready: waiting for storage_quorum` until it is (seen in 1 of 4 runs of the container case).
/// The admin client waits for that, and for nothing else.
#[tokio::test]
async fn create_bucket_waits_while_the_store_says_it_is_not_ready() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{Router, http::StatusCode, routing::put};

    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let app = Router::new().route(
        "/{bucket}",
        put(move || {
            let seen = counted.fetch_add(1, Ordering::SeqCst);
            async move {
                if seen < 3 {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "Service not ready: waiting for storage_quorum",
                    )
                } else {
                    (StatusCode::OK, "")
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a local port");
    let address = listener.local_addr().expect("local address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    LakeAdmin::new(&format!("http://{address}"), "http://127.0.0.1:0")
        .create_bucket("warehouse")
        .await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "three refusals, then the bucket"
    );
}

/// Any other refusal is a defect of the fixture or of the request, not something to wait for.
#[tokio::test]
#[should_panic(expected = "create bucket warehouse: 403")]
async fn create_bucket_does_not_wait_for_a_refusal_that_will_not_change() {
    use axum::{Router, http::StatusCode, routing::put};

    let app = Router::new().route(
        "/{bucket}",
        put(|| async { (StatusCode::FORBIDDEN, "SignatureDoesNotMatch") }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a local port");
    let address = listener.local_addr().expect("local address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    LakeAdmin::new(&format!("http://{address}"), "http://127.0.0.1:0")
        .create_bucket("warehouse")
        .await;
}

/// A container that dies after `start()` returned (a JVM killed under memory pressure, a bad flag) used to end in a
/// bare "REST catalog port" expectation, or in a five-minute wait. It now fails at once and says what the container
/// was doing: its exit code and the end of its log.
#[tokio::test]
async fn a_container_that_exited_is_reported_with_its_exit_code_and_its_log() {
    use futures_util::FutureExt;
    use std::panic::AssertUnwindSafe;
    use testcontainers_modules::testcontainers::{
        GenericImage, ImageExt, core::IntoContainerPort, runners::AsyncRunner,
    };

    use crate::support::lake::{RUSTFS, ready_container};

    let container = GenericImage::new(RUSTFS.0, RUSTFS.1)
        .with_exposed_port(9000.tcp())
        .with_cmd(["--no-such-flag"])
        .start()
        .await
        .expect("start RustFS with a flag it refuses");

    let outcome = AssertUnwindSafe(ready_container(
        &container,
        9000.tcp(),
        "/health",
        "RustFS fixture",
    ))
    .catch_unwind()
    .await;
    let payload = outcome.expect_err("a container that exited is not ready");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
        .expect("a text panic");

    assert!(
        message.contains("RustFS fixture is not running"),
        "names the container: {message}"
    );
    assert!(
        message.contains("exit code"),
        "gives the exit code: {message}"
    );
    assert!(
        message.contains("Usage: rustfs"),
        "shows what the container printed before it died: {message}"
    );
}

/// Real containers: RustFS and the Iceberg REST reference catalog on a private network.
#[tokio::test]
async fn the_admin_creates_a_table_the_catalog_can_load() {
    let lake = RestLake::start().await;
    let admin = lake.admin();

    admin.create_namespace("smoke").await;
    let created = admin
        .create_table(
            "smoke",
            "events",
            &[
                Column {
                    id: 1,
                    name: "id",
                    kind: "long",
                    required: true,
                },
                Column {
                    id: 2,
                    name: "payload",
                    kind: "string",
                    required: false,
                },
            ],
        )
        .await;
    let loaded = admin
        .load_table("smoke", "events")
        .await
        .expect("the created table is in the catalog");

    let location = loaded["metadata-location"]
        .as_str()
        .expect("a metadata location");
    assert_eq!(created["metadata-location"], loaded["metadata-location"]);
    assert!(
        location.starts_with("s3://warehouse/"),
        "the metadata is written to the fixture bucket: {location}"
    );
    assert!(
        admin.load_table("smoke", "missing").await.is_none(),
        "a table nobody created does not exist"
    );
}

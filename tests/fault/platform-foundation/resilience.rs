//! A crash inside a handler must not take the process down or leak details.

use axum::Router;
use axum::routing::get;
use sagat_api_http::{HttpLimits, protect};
use serde_json::Value;

#[allow(clippy::unused_async)]
async fn boom() -> &'static str {
    panic!("secret internal detail")
}

#[tokio::test]
async fn a_panicking_handler_becomes_a_500_envelope_and_the_server_keeps_serving() {
    let app = protect(
        Router::new()
            .route("/boom", get(boom))
            .route("/fine", get(|| async { "ok" })),
        HttpLimits::default(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    sagat_api_http::auth::install_crypto_provider();
    let client = reqwest::Client::new();

    let crashed = client
        .get(format!("http://{addr}/boom"))
        .send()
        .await
        .expect("send");

    assert_eq!(crashed.status(), 500);
    let trace = crashed.headers()["x-trace-id"]
        .to_str()
        .expect("ascii")
        .to_owned();
    let text = crashed.text().await.expect("body");
    assert!(
        !text.contains("secret internal detail"),
        "panic message leaked: {text}"
    );
    let body: Value = serde_json::from_str(&text).expect("json envelope");
    assert_eq!(body["code"], "INTERNAL");
    assert_eq!(body["retryable"], false);
    assert_eq!(body["traceId"], trace);

    let after = client
        .get(format!("http://{addr}/fine"))
        .send()
        .await
        .expect("send");
    assert_eq!(after.status(), 200, "the process survived the panic");
}

//! The control endpoint scenarios use to act on the server: move time, expire a lease.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::support::worker::Platform;

pub async fn serve(platform: Arc<Platform>) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/control", post(control))
        .with_state(platform);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the control endpoint");
    let addr = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, task)
}

async fn control(State(platform): State<Arc<Platform>>, Json(body): Json<Value>) -> Json<Value> {
    let field = |name: &str| {
        body.get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match field("action").as_str() {
        "expireLease" => platform.expire_lease(&field("run"), &field("node")).await,
        "elapseRetryDelay" => {
            platform
                .elapse_retry_delay(&field("run"), &field("node"))
                .await
        }
        "elapseTimer" => platform.elapse_timer(&field("run"), &field("node")).await,
        other => return Json(json!({"ok": false, "error": format!("unknown action {other}")})),
    }
    Json(json!({"ok": true}))
}

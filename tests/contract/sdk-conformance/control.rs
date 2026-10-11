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
        "expireLease" => expire_until_reclaimed(&platform, &field("run"), &field("node")).await,
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

/// Back-dates the lease until the platform has taken it back. Back-dating it once is not enough: a worker whose
/// heartbeat lands between the update and the scheduler's next look renews the lease, and nothing is lost.
async fn expire_until_reclaimed(platform: &Platform, run: &str, node: &str) {
    let run_id = uuid::Uuid::parse_str(run).expect("run id");
    for _ in 0..250 {
        platform.expire_lease(run, node).await;
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM steps WHERE run_id = $1 AND node = $2")
                .bind(run_id)
                .bind(node)
                .fetch_optional(&platform.api.db.pool)
                .await
                .ok()
                .flatten();
        if !matches!(status.as_deref(), Some("LEASED" | "RUNNING")) {
            return;
        }
    }
}

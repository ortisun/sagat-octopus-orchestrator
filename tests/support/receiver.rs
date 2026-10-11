//! A receiver for outbound webhooks: a local HTTP server that records every request and answers as
//! scripted, and a sender that reaches it (a real subscription cannot name a loopback address, which
//! no egress policy can allow, so the worker is given a sender that does not apply the policy; the
//! policy has its own tests).
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use sagat_application::outbound::{OutboundRequest, OutboundResponse, SendFailure, WebhookSender};

/// What the receiver does with the next request.
#[derive(Debug, Clone)]
pub enum Reply {
    Status(u16),
    /// A status with a `Retry-After` header.
    StatusAfter(u16, u64),
    /// A status with a body (the platform must never repeat it).
    StatusBody(u16, &'static str),
    /// A status with a body of this many bytes: a receiver that answers with a page.
    Big(u16, usize),
    /// Reads the request, waits this long, then answers 200: the sender gives up first.
    Slow(Duration),
}

/// One request the receiver saw.
#[derive(Debug, Clone)]
pub struct Seen {
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Default)]
struct Inner {
    script: VecDeque<Reply>,
    default: Option<u16>,
    seen: Vec<Seen>,
}

#[derive(Clone)]
pub struct Receiver {
    pub url: String,
    inner: Arc<Mutex<Inner>>,
}

async fn handle(
    State(inner): State<Arc<Mutex<Inner>>>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let reply = {
        let mut inner = inner.lock().expect("receiver state");
        inner.seen.push(Seen {
            headers: headers
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str().to_owned(),
                        v.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
            body: body.to_vec(),
        });
        inner
            .script
            .pop_front()
            .unwrap_or_else(|| Reply::Status(inner.default.unwrap_or(200)))
    };
    match reply {
        Reply::Status(code) => StatusCode::from_u16(code).expect("status").into_response(),
        Reply::Big(code, size) => (
            StatusCode::from_u16(code).expect("status"),
            "x".repeat(size),
        )
            .into_response(),
        Reply::StatusBody(code, text) => {
            (StatusCode::from_u16(code).expect("status"), text).into_response()
        }
        Reply::StatusAfter(code, seconds) => (
            StatusCode::from_u16(code).expect("status"),
            [("retry-after", seconds.to_string())],
        )
            .into_response(),
        Reply::Slow(wait) => {
            tokio::time::sleep(wait).await;
            StatusCode::OK.into_response()
        }
    }
}

impl Receiver {
    pub async fn start() -> Self {
        let inner = Arc::new(Mutex::new(Inner::default()));
        let app = Router::new()
            .route("/hook", post(handle))
            .with_state(Arc::clone(&inner));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            url: format!("http://{addr}/hook"),
            inner,
        }
    }

    /// The answers to the next requests, in order; then the default.
    pub fn script(&self, replies: impl IntoIterator<Item = Reply>) {
        self.inner.lock().expect("state").script = replies.into_iter().collect();
    }

    /// What it answers when the script is empty (200 until set).
    pub fn answer(&self, status: u16) {
        self.inner.lock().expect("state").default = Some(status);
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.inner.lock().expect("state").seen.clone()
    }
}

/// Reaches the receiver directly.
#[derive(Clone)]
pub struct LoopbackSender {
    client: reqwest::Client,
}

impl LoopbackSender {
    pub fn new() -> Self {
        // reqwest is built without a TLS provider; the process must choose one before any client.
        sagat_api_http::auth::install_crypto_provider();
        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("client"),
        }
    }
}

impl WebhookSender for LoopbackSender {
    async fn send(&self, request: &OutboundRequest) -> Result<OutboundResponse, SendFailure> {
        let mut builder = self.client.post(request.url.as_str());
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        match tokio::time::timeout(request.timeout, builder.body(request.body.clone()).send()).await
        {
            Err(_) => Err(SendFailure::Timeout),
            Ok(Err(_)) => Err(SendFailure::Network),
            Ok(Ok(response)) => Ok(OutboundResponse {
                status: response.status().as_u16(),
                retry_after: response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(Duration::from_secs),
            }),
        }
    }
}

/// Lets a client reach a server on this machine, which no policy can allow.
pub struct Open;

impl sagat_infra_egress::Guard for Open {
    fn check_destination(
        &self,
        _: &sagat_domain::egress::Destination<'_>,
    ) -> Result<(), sagat_domain::egress::Denial> {
        Ok(())
    }

    fn check_address(&self, _: std::net::IpAddr) -> Result<(), sagat_domain::egress::Denial> {
        Ok(())
    }
}

/// Builds clients that are not held to the policy.
pub struct OpenFactory;

impl sagat_infra_connectors::webhook::ClientFactory for OpenFactory {
    fn client(
        &self,
        _: &sagat_domain::egress::EgressPolicy,
    ) -> Result<sagat_infra_egress::EgressClient, sagat_infra_egress::EgressError> {
        sagat_infra_egress::EgressClient::with(
            std::sync::Arc::new(Open),
            std::sync::Arc::new(sagat_infra_egress::SystemResolver),
            sagat_infra_egress::EgressLimits {
                total_timeout: Duration::from_secs(35),
                ..sagat_infra_egress::EgressLimits::default()
            },
        )
    }
}

//! A fake OpenTelemetry Collector: receives OTLP/HTTP JSON, keeps what it receives, and can be told
//! to answer with a given status, so the suites can see which spans left the platform and what the
//! platform does when the collector is down.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, Once};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use sagat_telemetry::{OtlpClient, OtlpConfig, OtlpLayer, SpanSink};
use serde_json::Value;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::layer::SubscriberExt;

// `tracing` keeps one process-wide callsite interest cache even though scoped subscribers are
// thread-local. A concurrent test with no subscriber can register a new callsite through its empty
// default while a collector-backed test is serving requests. Keep a process baseline installed and
// isolate the scoped collector dispatchers for their full lifetime.
static COLLECTOR_SUBSCRIBER: Mutex<()> = Mutex::new(());
static TEST_TRACING: Once = Once::new();

pub struct CollectorGuard {
    subscriber: Option<DefaultGuard>,
    _exclusive: MutexGuard<'static, ()>,
}

impl Drop for CollectorGuard {
    fn drop(&mut self) {
        // Remove the thread-local dispatcher while this test still owns the exclusive fixture.
        drop(self.subscriber.take());
        tracing::callsite::rebuild_interest_cache();
    }
}

/// One span as the collector received it.
#[derive(Debug, Clone)]
pub struct Received {
    pub trace_id: String,
    pub span_id: String,
    pub parent: Option<String>,
    pub name: String,
    pub kind: u64,
    pub start: u128,
    pub end: u128,
    pub status: u64,
    pub attributes: HashMap<String, String>,
    pub events: Vec<String>,
    pub links: Vec<(String, String)>,
}

impl Received {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }
}

#[derive(Default)]
struct Inner {
    /// The status of each request in order; 200 once the list is empty.
    statuses: Mutex<VecDeque<u16>>,
    spans: Mutex<Vec<Received>>,
    requests: Mutex<u32>,
}

#[derive(Clone)]
pub struct Collector {
    pub url: String,
    inner: Arc<Inner>,
}

fn text(value: &Value) -> String {
    for key in ["stringValue", "intValue", "boolValue", "doubleValue"] {
        if let Some(found) = value.get(key) {
            return found
                .as_str()
                .map_or_else(|| found.to_string(), str::to_owned);
        }
    }
    String::new()
}

fn attributes(list: &Value) -> HashMap<String, String> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .map(|a| {
                    (
                        a["key"].as_str().unwrap_or_default().to_owned(),
                        text(&a["value"]),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn number(value: &Value) -> u128 {
    value.as_str().and_then(|s| s.parse().ok()).unwrap_or(0)
}

async fn receive(State(inner): State<Arc<Inner>>, body: axum::body::Bytes) -> StatusCode {
    *inner.requests.lock().expect("requests") += 1;
    let status = inner
        .statuses
        .lock()
        .expect("statuses")
        .pop_front()
        .unwrap_or(200);
    if status != 200 {
        return StatusCode::from_u16(status).expect("status");
    }
    let json: Value = serde_json::from_slice(&body).expect("the body is JSON");
    let mut spans = inner.spans.lock().expect("spans");
    for resource in json["resourceSpans"].as_array().into_iter().flatten() {
        for scope in resource["scopeSpans"].as_array().into_iter().flatten() {
            for span in scope["spans"].as_array().into_iter().flatten() {
                spans.push(Received {
                    trace_id: span["traceId"].as_str().unwrap_or_default().to_owned(),
                    span_id: span["spanId"].as_str().unwrap_or_default().to_owned(),
                    parent: span["parentSpanId"].as_str().map(str::to_owned),
                    name: span["name"].as_str().unwrap_or_default().to_owned(),
                    kind: span["kind"].as_u64().unwrap_or(0),
                    start: number(&span["startTimeUnixNano"]),
                    end: number(&span["endTimeUnixNano"]),
                    status: span["status"]["code"].as_u64().unwrap_or(0),
                    attributes: attributes(&span["attributes"]),
                    events: span["events"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|e| e["name"].as_str().unwrap_or_default().to_owned())
                        .collect(),
                    links: span["links"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|l| {
                            (
                                l["traceId"].as_str().unwrap_or_default().to_owned(),
                                l["spanId"].as_str().unwrap_or_default().to_owned(),
                            )
                        })
                        .collect(),
                });
            }
        }
    }
    StatusCode::OK
}

impl Collector {
    pub async fn start() -> Self {
        let inner = Arc::new(Inner::default());
        let app = Router::new()
            .route("/v1/traces", post(receive))
            .with_state(Arc::clone(&inner));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}/v1/traces", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { url, inner }
    }

    /// The next requests are answered with these statuses, in order.
    pub fn answer_with(&self, statuses: &[u16]) {
        *self.inner.statuses.lock().expect("statuses") = statuses.iter().copied().collect();
    }

    /// Answers 503 until [`Self::recover`].
    pub fn go_down(&self) {
        *self.inner.statuses.lock().expect("statuses") =
            std::iter::repeat_n(503, 100_000).collect();
    }

    pub fn recover(&self) {
        self.inner.statuses.lock().expect("statuses").clear();
    }

    pub fn requests(&self) -> u32 {
        *self.inner.requests.lock().expect("requests")
    }

    pub fn spans(&self) -> Vec<Received> {
        self.inner.spans.lock().expect("spans").clone()
    }

    pub fn named(&self, name: &str) -> Vec<Received> {
        self.spans()
            .into_iter()
            .filter(|s| s.name == name)
            .collect()
    }

    pub fn of_trace(&self, trace: &str) -> Vec<Received> {
        self.spans()
            .into_iter()
            .filter(|s| s.trace_id == trace)
            .collect()
    }

    /// Waits until at least `count` spans satisfy `keep`, up to ten seconds.
    pub async fn wait_for(&self, count: usize, keep: impl Fn(&Received) -> bool) -> Vec<Received> {
        for _ in 0..200 {
            let found: Vec<Received> = self.spans().into_iter().filter(&keep).collect();
            if found.len() >= count {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "fewer than {count} spans arrived after {} collector requests: {:#?}",
            self.requests(),
            self.spans()
        );
    }

    /// The client of this collector, for a test that exports by itself.
    pub fn client(&self) -> OtlpClient {
        let mut config = OtlpConfig::new(&self.url, "sagat-test");
        config.timeout = Duration::from_secs(2);
        OtlpClient::new(config).expect("client")
    }

    /// Exports the live spans of the current thread to this collector: the platform of a test is
    /// served on the test's own runtime, so everything it does is seen by this subscriber until the
    /// guard drops. Flush with [`SpanSink::shutdown`].
    pub fn install(&self, ratio: f64) -> (CollectorGuard, SpanSink) {
        let (guard, sink, _) = self.install_inner(ratio, None);
        (guard, sink)
    }

    /// [`Self::install`] that also writes every log line, at any level, to the returned buffer, so
    /// a test can search the spans and the log text of the same requests.
    pub fn install_with_logs(
        &self,
        ratio: f64,
    ) -> (CollectorGuard, SpanSink, super::logs::Captured) {
        let captured = super::logs::Captured::default();
        let (guard, sink, _) = self.install_inner(ratio, Some(captured.clone()));
        (guard, sink, captured)
    }

    fn install_inner(
        &self,
        ratio: f64,
        logs: Option<super::logs::Captured>,
    ) -> (CollectorGuard, SpanSink, Option<super::logs::Captured>) {
        // `tracing-core` optimizes its first dispatcher through the default of whichever thread
        // registers a new callsite. A concurrent test with no subscriber can otherwise cache that
        // callsite as disabled after this scoped dispatcher was installed. The empty global
        // registry keeps callsites enabled; this test's scoped layer still owns all export.
        TEST_TRACING.call_once(|| {
            tracing::subscriber::set_global_default(tracing_subscriber::registry())
                .expect("install the test tracing registry");
        });
        let exclusive = COLLECTOR_SUBSCRIBER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut config = OtlpConfig::new(&self.url, "sagat-test");
        config.timeout = Duration::from_secs(2);
        config.flush_interval = Duration::from_millis(100);
        config.sample_ratio = ratio;
        let sink = sagat_telemetry::start(OtlpClient::new(config).expect("client"));
        let layered = tracing_subscriber::registry().with(OtlpLayer::new(sink.clone(), ratio));
        let subscriber = match logs.clone() {
            Some(writer) => tracing::subscriber::set_default(
                layered.with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(writer)
                        .with_ansi(false),
                ),
            ),
            None => tracing::subscriber::set_default(layered),
        };
        tracing::callsite::rebuild_interest_cache();
        (
            CollectorGuard {
                subscriber: Some(subscriber),
                _exclusive: exclusive,
            },
            sink,
            logs,
        )
    }
}

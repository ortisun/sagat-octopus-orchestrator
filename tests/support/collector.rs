//! A fake OpenTelemetry Collector: receives OTLP/HTTP JSON, keeps what it receives, and can be told
//! to answer with a given status, so the suites can see which spans left the platform and what the
//! platform does when the collector is down. It takes the three endpoints of OTLP/HTTP: the traces
//! (`/v1/traces`, which the statuses, [`Collector::spans`], [`Collector::bodies`] and
//! [`Collector::requests`] are about) and the logs and metrics (`/v1/logs`, `/v1/metrics`, always
//! answered 200 and kept apart, so that a suite that reads the traces sees what it saw before).
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
    pub status_message: String,
    pub attributes: HashMap<String, String>,
    pub events: Vec<String>,
    pub links: Vec<(String, String)>,
}

impl Received {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }
}

/// What one of the endpoints of the logs and metrics received.
#[derive(Default)]
struct Signal {
    /// Every request, kept bodies or not.
    requests: u32,
    /// The bodies, as text, once [`Collector::keep_bodies`] was asked.
    bodies: Vec<String>,
}

#[derive(Default)]
struct Inner {
    /// The status of each request in order; 200 once the list is empty.
    statuses: Mutex<VecDeque<u16>>,
    spans: Mutex<Vec<Received>>,
    /// The accepted requests as they came over the wire, so that a test can search what the exporter sent and not only
    /// what this collector chose to parse of it. Kept only after [`Collector::keep_bodies`]: a suite that does not search
    /// the wire does not pay for every body it receives.
    bodies: Mutex<Vec<String>>,
    keep_bodies: std::sync::atomic::AtomicBool,
    /// The requests of `/v1/traces` (the other two endpoints count theirs apart).
    requests: Mutex<u32>,
    logs: Mutex<Signal>,
    metrics: Mutex<Signal>,
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
    if inner.keep_bodies.load(std::sync::atomic::Ordering::Relaxed) {
        inner
            .bodies
            .lock()
            .expect("bodies")
            .push(String::from_utf8_lossy(&body).into_owned());
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
                    status_message: span["status"]["message"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
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

fn keep_signal(inner: &Inner, signal: &Mutex<Signal>, body: &[u8]) -> StatusCode {
    let mut signal = signal.lock().expect("signal");
    signal.requests += 1;
    if inner.keep_bodies.load(std::sync::atomic::Ordering::Relaxed) {
        signal
            .bodies
            .push(String::from_utf8_lossy(body).into_owned());
    }
    StatusCode::OK
}

async fn receive_logs(State(inner): State<Arc<Inner>>, body: axum::body::Bytes) -> StatusCode {
    keep_signal(&inner, &inner.logs, &body)
}

async fn receive_metrics(State(inner): State<Arc<Inner>>, body: axum::body::Bytes) -> StatusCode {
    keep_signal(&inner, &inner.metrics, &body)
}

impl Collector {
    pub async fn start() -> Self {
        let inner = Arc::new(Inner::default());
        let app = Router::new()
            .route("/v1/traces", post(receive))
            .route("/v1/logs", post(receive_logs))
            .route("/v1/metrics", post(receive_metrics))
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

    /// From now on the bodies of the requests are kept (the default is not to keep them).
    pub fn keep_bodies(&self) {
        self.inner
            .keep_bodies
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The bodies of the requests the collector accepted since [`Self::keep_bodies`], as text: the OTLP/HTTP JSON the exporter
    /// sent. A string that went through JSON is in it as JSON writes it: a search for a text with a quote, a backslash or a
    /// character outside ASCII must search the escaped form too.
    pub fn bodies(&self) -> Vec<String> {
        self.inner.bodies.lock().expect("bodies").clone()
    }

    /// The requests `/v1/logs` received, kept bodies or not.
    pub fn log_requests(&self) -> u32 {
        self.inner.logs.lock().expect("logs").requests
    }

    /// The requests `/v1/metrics` received, kept bodies or not.
    pub fn metric_requests(&self) -> u32 {
        self.inner.metrics.lock().expect("metrics").requests
    }

    /// The bodies `/v1/logs` received since [`Self::keep_bodies`], as [`Self::bodies`] keeps those of the traces.
    pub fn log_bodies(&self) -> Vec<String> {
        self.inner.logs.lock().expect("logs").bodies.clone()
    }

    /// The bodies `/v1/metrics` received since [`Self::keep_bodies`], as [`Self::bodies`] keeps those of the traces.
    pub fn metric_bodies(&self) -> Vec<String> {
        self.inner.metrics.lock().expect("metrics").bodies.clone()
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

    /// The exporter's settings an operator gets by pointing `OTEL_EXPORTER_OTLP_ENDPOINT` at this collector: the
    /// traces, the logs and the metrics endpoints all set (`OtlpConfig::from_lookup`), with the short timeout of the
    /// other clients of this file.
    pub fn signals_config(&self) -> OtlpConfig {
        let base = self
            .url
            .strip_suffix("/v1/traces")
            .expect("the url of the traces")
            .to_owned();
        let mut config = OtlpConfig::from_lookup(
            &|name: &str| (name == "OTEL_EXPORTER_OTLP_ENDPOINT").then(|| base.clone()),
            "sagat-test",
        )
        .expect("the settings of the common endpoint")
        .expect("an exporter is configured");
        config.timeout = Duration::from_secs(2);
        config
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
        let (guard, sink, _) = self.install_inner(ratio, None, false);
        (guard, sink)
    }

    /// [`Self::install`] that also writes every log line, at any level, to the returned buffer, so
    /// a test can search the spans and the log text of the same requests.
    pub fn install_with_logs(
        &self,
        ratio: f64,
    ) -> (CollectorGuard, SpanSink, super::logs::Captured) {
        let captured = super::logs::Captured::default();
        let (guard, sink, _) = self.install_inner(ratio, Some(captured.clone()), false);
        (guard, sink, captured)
    }

    /// [`Self::install_with_logs`] whose exporter has the three endpoints of this collector set, as
    /// [`Self::signals_config`] gives them, and the client of those settings, for whatever exports
    /// logs or gauges (`OtlpClient::export_log`, `OtlpClient::export_gauge`) or runs a projector.
    pub fn install_with_signals(
        &self,
        ratio: f64,
    ) -> (CollectorGuard, SpanSink, super::logs::Captured, OtlpClient) {
        let captured = super::logs::Captured::default();
        let (guard, sink, _) = self.install_inner(ratio, Some(captured.clone()), true);
        let mut config = self.signals_config();
        config.sample_ratio = ratio;
        (
            guard,
            sink,
            captured,
            OtlpClient::new(config).expect("client"),
        )
    }

    fn install_inner(
        &self,
        ratio: f64,
        logs: Option<super::logs::Captured>,
        signals: bool,
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
        let mut config = if signals {
            self.signals_config()
        } else {
            OtlpConfig::new(&self.url, "sagat-test")
        };
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

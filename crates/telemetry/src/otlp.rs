//! OTLP/HTTP (JSON) export of spans to an OpenTelemetry Collector (FR-008-001, G6).
//!
//! The wire format is the standard one, so the standard Collector (or any backend that accepts
//! OTLP/HTTP) is the other end. Two ways in lead to it, because spans come from two places (D160):
//!
//! * [`SpanSink`] takes the spans of the work a process does while a request is open (the HTTP and
//!   gRPC hops, a scheduler pass, a delivery). It never waits: a full queue drops the span and
//!   counts it, because telemetry must not slow or fail the work it describes.
//! * [`OtlpClient::export`] sends a batch and says whether it arrived. The projector, which derives
//!   the spans of runs and attempts from the journal, uses it directly and moves its progress
//!   forward only on success, so a collector that is down delays those spans and loses none.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::sync::{Notify, mpsc};
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::redact::{REDACTED, is_sensitive_name, scrub};
use crate::span::{SpanId, keeps};
use crate::trace::TraceId;

/// An attribute value.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    Str(String),
    Int(i64),
    Bool(bool),
    Double(f64),
}

impl From<&str> for AttrValue {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}
impl From<String> for AttrValue {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}
impl From<i64> for AttrValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}
impl From<u32> for AttrValue {
    fn from(value: u32) -> Self {
        Self::Int(i64::from(value))
    }
}
impl From<bool> for AttrValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

/// How the span relates to its callers (OTLP `SpanKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    Internal,
    Server,
    Client,
    Producer,
    Consumer,
}

impl SpanKind {
    const fn code(self) -> u8 {
        match self {
            Self::Internal => 1,
            Self::Server => 2,
            Self::Client => 3,
            Self::Producer => 4,
            Self::Consumer => 5,
        }
    }
}

/// The outcome of the span (OTLP `Status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanStatus {
    Unset,
    Ok,
    Error(String),
}

/// A point in a span's life.
#[derive(Debug, Clone, PartialEq)]
pub struct SpanEvent {
    pub name: String,
    pub unix_nano: u64,
    pub attributes: Vec<(String, AttrValue)>,
}

/// A reference to a span of another trace (an operator's action that acted on a run).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanLink {
    pub trace_id: TraceId,
    pub span_id: SpanId,
}

/// One finished span.
#[derive(Debug, Clone, PartialEq)]
pub struct SpanData {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub parent: Option<SpanId>,
    pub name: String,
    pub kind: SpanKind,
    pub start_unix_nano: u64,
    pub end_unix_nano: u64,
    pub attributes: Vec<(String, AttrValue)>,
    pub events: Vec<SpanEvent>,
    pub links: Vec<SpanLink>,
    pub status: SpanStatus,
}

/// Most attributes written on a span; the rest are cut.
pub const MAX_ATTRIBUTES: usize = 48;
/// Most events and links written on a span.
pub const MAX_SPAN_ITEMS: usize = 32;

fn attribute(key: &str, value: &AttrValue) -> Value {
    let value = match value {
        AttrValue::Str(text) => {
            if is_sensitive_name(key) {
                json!({ "stringValue": REDACTED })
            } else {
                json!({ "stringValue": scrub(text) })
            }
        }
        // OTLP/JSON carries 64-bit integers as strings.
        AttrValue::Int(number) => json!({ "intValue": number.to_string() }),
        AttrValue::Bool(flag) => json!({ "boolValue": flag }),
        AttrValue::Double(number) if number.is_finite() => json!({ "doubleValue": number }),
        AttrValue::Double(_) => json!({ "stringValue": "NaN" }),
    };
    json!({ "key": key, "value": value })
}

fn attributes(list: &[(String, AttrValue)]) -> Vec<Value> {
    list.iter()
        .take(MAX_ATTRIBUTES)
        .map(|(key, value)| attribute(key, value))
        .collect()
}

fn encode_span(span: &SpanData) -> Value {
    let mut out = Map::new();
    out.insert("traceId".to_owned(), json!(span.trace_id.to_string()));
    out.insert("spanId".to_owned(), json!(span.span_id.to_string()));
    if let Some(parent) = span.parent {
        out.insert("parentSpanId".to_owned(), json!(parent.to_string()));
    }
    out.insert("name".to_owned(), json!(scrub(&span.name)));
    out.insert("kind".to_owned(), json!(span.kind.code()));
    out.insert(
        "startTimeUnixNano".to_owned(),
        json!(span.start_unix_nano.to_string()),
    );
    out.insert(
        "endTimeUnixNano".to_owned(),
        json!(span.end_unix_nano.max(span.start_unix_nano).to_string()),
    );
    out.insert("attributes".to_owned(), json!(attributes(&span.attributes)));
    if !span.events.is_empty() {
        let events: Vec<Value> = span
            .events
            .iter()
            .take(MAX_SPAN_ITEMS)
            .map(|event| {
                json!({
                    "timeUnixNano": event.unix_nano.to_string(),
                    "name": scrub(&event.name),
                    "attributes": attributes(&event.attributes),
                })
            })
            .collect();
        out.insert("events".to_owned(), json!(events));
    }
    if !span.links.is_empty() {
        let links: Vec<Value> = span
            .links
            .iter()
            .take(MAX_SPAN_ITEMS)
            .map(|link| json!({ "traceId": link.trace_id.to_string(), "spanId": link.span_id.to_string() }))
            .collect();
        out.insert("links".to_owned(), json!(links));
    }
    match &span.status {
        SpanStatus::Unset => {}
        SpanStatus::Ok => {
            out.insert("status".to_owned(), json!({ "code": 1 }));
        }
        SpanStatus::Error(message) => {
            out.insert(
                "status".to_owned(),
                json!({ "code": 2, "message": scrub(message) }),
            );
        }
    }
    Value::Object(out)
}

/// Who sends the spans.
#[derive(Clone)]
pub struct OtlpConfig {
    /// The full URL spans are posted to (`.../v1/traces`).
    pub endpoint: String,
    /// Headers sent with every request, such as an API key. Never logged.
    pub headers: Vec<(String, String)>,
    pub service_name: String,
    pub service_version: String,
    /// The share of traces kept, decided from the trace id (see [`keeps`]).
    pub sample_ratio: f64,
    /// Spans waiting for the exporter before new ones are dropped.
    pub queue_capacity: usize,
    /// Spans in one request.
    pub batch_size: usize,
    /// The longest a span waits in the queue for a batch to fill.
    pub flush_interval: Duration,
    /// How long one request may take.
    pub timeout: Duration,
    /// Times a retryable failure is tried again.
    pub retries: u32,
}

impl OtlpConfig {
    /// The defaults for `endpoint`.
    #[must_use]
    pub fn new(endpoint: impl Into<String>, service_name: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            headers: Vec::new(),
            service_name: service_name.into(),
            service_version: env!("CARGO_PKG_VERSION").to_owned(),
            sample_ratio: 1.0,
            queue_capacity: 2048,
            batch_size: 256,
            flush_interval: Duration::from_secs(2),
            timeout: Duration::from_secs(10),
            retries: 3,
        }
    }
}

impl fmt::Debug for OtlpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OtlpConfig")
            .field("endpoint", &crate::redact::scrub(&self.endpoint))
            .field("headers", &format_args!("{} (hidden)", self.headers.len()))
            .field("service_name", &self.service_name)
            .field("sample_ratio", &self.sample_ratio)
            .field("queue_capacity", &self.queue_capacity)
            .finish_non_exhaustive()
    }
}

/// Why a batch did not arrive.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    /// The collector could not be reached, or said to come back later. Trying again may work.
    #[error("the collector is not available ({0})")]
    Unavailable(String),
    /// The collector said the batch itself is wrong (`400`, `413`, `422`): sending it again changes
    /// nothing. Any other answer is about the collector, the credentials or the address, and not
    /// about the batch, and is [`ExportError::Unavailable`].
    #[error("the collector refused the batch (HTTP {0})")]
    Rejected(u16),
}

/// Sends batches of spans to the collector.
#[derive(Clone)]
pub struct OtlpClient {
    http: reqwest::Client,
    config: Arc<OtlpConfig>,
    resource: Arc<Value>,
}

/// What the Collector returns when it cannot take the batch now: OTLP/HTTP names 429, 502, 503 and
/// 504; a server error or a timeout is the same kind of thing.
fn retryable(status: u16) -> bool {
    matches!(status, 408 | 429) || (500..=599).contains(&status)
}

/// What the Collector returns when the batch is the problem. These are the only verdicts: a `401`
/// or a `404` says the setting is wrong, and the spans, which the journal still holds, wait for
/// someone to fix it instead of being dropped.
const BATCH_REFUSED: [u16; 3] = [400, 413, 422];
const BACKOFF_START: Duration = Duration::from_millis(200);
const BACKOFF_CAP: Duration = Duration::from_secs(5);

fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // An error means the process installed its own provider first, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

impl OtlpClient {
    /// A client for `config`.
    ///
    /// # Errors
    /// [`ExportError::Unavailable`] if the HTTP stack cannot be built.
    pub fn new(config: OtlpConfig) -> Result<Self, ExportError> {
        install_crypto_provider();
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.timeout)
            // The collector is where the operator said; a redirect or a proxy from the environment
            // would send the spans somewhere else.
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| {
                ExportError::Unavailable("the HTTP client could not be built".to_owned())
            })?;
        let resource = json!({
            "attributes": [
                { "key": "service.name", "value": { "stringValue": config.service_name } },
                { "key": "service.version", "value": { "stringValue": config.service_version } },
            ]
        });
        Ok(Self {
            http,
            config: Arc::new(config),
            resource: Arc::new(resource),
        })
    }

    /// The request body for `spans`.
    #[must_use]
    pub fn encode(&self, spans: &[SpanData]) -> Vec<u8> {
        let spans: Vec<Value> = spans.iter().map(encode_span).collect();
        let body = json!({
            "resourceSpans": [{
                "resource": *self.resource,
                "scopeSpans": [{
                    "scope": { "name": "sagat-telemetry", "version": env!("CARGO_PKG_VERSION") },
                    "spans": spans,
                }]
            }]
        });
        serde_json::to_vec(&body).unwrap_or_default()
    }

    /// Sends `spans`, trying again while the failure is one that may pass.
    ///
    /// # Errors
    /// [`ExportError`] once the batch is refused or the retries are used up.
    pub async fn export(&self, spans: &[SpanData]) -> Result<(), ExportError> {
        if spans.is_empty() {
            return Ok(());
        }
        let body = self.encode(spans);
        let mut delay = BACKOFF_START;
        let mut last = ExportError::Unavailable("not tried".to_owned());
        for attempt in 0..=self.config.retries {
            if attempt > 0 {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(BACKOFF_CAP);
            }
            let mut request = self
                .http
                .post(&self.config.endpoint)
                .header("content-type", "application/json")
                .body(body.clone());
            for (name, value) in &self.config.headers {
                request = request.header(name, value);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if response.status().is_success() {
                        return Ok(());
                    }
                    if BATCH_REFUSED.contains(&status) {
                        return Err(ExportError::Rejected(status));
                    }
                    if !retryable(status) {
                        // Asking again at once changes nothing; the caller asks again later.
                        return Err(ExportError::Unavailable(format!("HTTP {status}")));
                    }
                    if let Some(wait) = retry_after(&response) {
                        delay = wait.min(BACKOFF_CAP);
                    }
                    last = ExportError::Unavailable(format!("HTTP {status}"));
                }
                Err(error) => {
                    let kind = if error.is_timeout() {
                        "timeout"
                    } else if error.is_connect() {
                        "connection"
                    } else {
                        "transport"
                    };
                    last = ExportError::Unavailable(kind.to_owned());
                }
            }
        }
        Err(last)
    }
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Where the spans of live work are queued for the exporter.
#[derive(Clone)]
pub struct SpanSink {
    queue: mpsc::Sender<SpanData>,
    stop: Arc<Notify>,
    done: Arc<Notify>,
}

impl SpanSink {
    /// Queues `span` unless the queue is full, in which case it is dropped and counted.
    pub fn submit(&self, span: SpanData) {
        match self.queue.try_send(span) {
            Ok(()) => metrics::counter!("otel_spans_total", "outcome" => "queued").increment(1),
            Err(_) => metrics::counter!("otel_spans_total", "outcome" => "dropped").increment(1),
        }
    }

    /// Stops the exporter after it sends what is queued, waiting at most `deadline`.
    pub async fn shutdown(&self, deadline: Duration) {
        self.stop.notify_one();
        let _ = tokio::time::timeout(deadline, self.done.notified()).await;
    }
}

/// Starts the task that batches queued spans and sends them. Needs a Tokio runtime.
#[must_use]
pub fn start(client: OtlpClient) -> SpanSink {
    let config = Arc::clone(&client.config);
    let (queue, mut inbox) = mpsc::channel::<SpanData>(config.queue_capacity.max(1));
    let stop = Arc::new(Notify::new());
    let done = Arc::new(Notify::new());
    let (task_stop, task_done) = (Arc::clone(&stop), Arc::clone(&done));
    tokio::spawn(async move {
        let mut batch: Vec<SpanData> = Vec::with_capacity(config.batch_size);
        let mut tick = tokio::time::interval(config.flush_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                received = inbox.recv() => match received {
                    Some(span) => {
                        batch.push(span);
                        if batch.len() >= config.batch_size {
                            send(&client, &mut batch).await;
                        }
                    }
                    None => break,
                },
                _ = tick.tick() => send(&client, &mut batch).await,
                () = task_stop.notified() => {
                    while let Ok(span) = inbox.try_recv() {
                        batch.push(span);
                        if batch.len() >= config.batch_size {
                            send(&client, &mut batch).await;
                        }
                    }
                    break;
                }
            }
        }
        send(&client, &mut batch).await;
        task_done.notify_one();
    });
    SpanSink { queue, stop, done }
}

async fn send(client: &OtlpClient, batch: &mut Vec<SpanData>) {
    if batch.is_empty() {
        return;
    }
    let count = batch.len() as u64;
    let started = std::time::Instant::now();
    let outcome = match client.export(batch).await {
        Ok(()) => "ok",
        Err(ExportError::Rejected(_)) => "rejected",
        Err(ExportError::Unavailable(_)) => "unavailable",
    };
    metrics::counter!("otel_export_batches_total", "outcome" => outcome).increment(1);
    metrics::histogram!("otel_export_duration_seconds").record(started.elapsed().as_secs_f64());
    if outcome != "ok" {
        // The batch is given up on: a live span is not worth a queue that grows without bound.
        metrics::counter!("otel_spans_total", "outcome" => "lost").increment(count);
    }
    batch.clear();
}

/// The span of work in progress, kept on it until it closes.
struct Open {
    trace_id: TraceId,
    span_id: SpanId,
    parent: Option<SpanId>,
    sampled: bool,
    name: &'static str,
    kind: SpanKind,
    start: u64,
    attributes: Vec<(String, AttrValue)>,
    status: SpanStatus,
}

#[derive(Default)]
struct Fields {
    /// `otel.sampled = false` keeps a span (and the spans under it) out of the export whatever the
    /// ratio says: the span of a poll that belongs to no trace worth keeping.
    sampled: Option<bool>,
    trace_id: Option<TraceId>,
    span_id: Option<SpanId>,
    parent: Option<SpanId>,
    kind: Option<SpanKind>,
    error: Option<String>,
    attributes: Vec<(String, AttrValue)>,
}

impl Fields {
    fn set_attribute(&mut self, name: &str, value: AttrValue) {
        if let Some(slot) = self.attributes.iter_mut().find(|(key, _)| key == name) {
            slot.1 = value;
        } else {
            self.attributes.push((name.to_owned(), value));
        }
    }

    fn text(&mut self, name: &str, text: &str) {
        match name {
            "trace_id" => self.trace_id = TraceId::parse(text),
            "span_id" => self.span_id = SpanId::parse(text),
            "parent_span_id" => self.parent = SpanId::parse(text),
            "otel.kind" => {
                self.kind = Some(match text {
                    "server" => SpanKind::Server,
                    "client" => SpanKind::Client,
                    "producer" => SpanKind::Producer,
                    "consumer" => SpanKind::Consumer,
                    _ => SpanKind::Internal,
                });
            }
            "otel.status_description" => self.error = Some(text.to_owned()),
            "otel.status_code" => {
                if text == "error" && self.error.is_none() {
                    self.error = Some(String::new());
                }
            }
            _ => {
                let value = if is_sensitive_name(name) {
                    REDACTED.to_owned()
                } else {
                    scrub(text).into_owned()
                };
                self.set_attribute(name, AttrValue::Str(value));
            }
        }
    }
}

impl Visit for Fields {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.set_attribute(field.name(), AttrValue::Int(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        let value = i64::try_from(value).map_or(AttrValue::Str(value.to_string()), AttrValue::Int);
        self.set_attribute(field.name(), value);
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "otel.sampled" {
            self.sampled = Some(value);
            return;
        }
        self.set_attribute(field.name(), AttrValue::Bool(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.set_attribute(field.name(), AttrValue::Double(value));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.text(field.name(), value);
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.text(field.name(), &format!("{value:?}"));
    }
}

fn unix_nano_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

/// A layer that turns the platform's `tracing` spans into exported spans.
///
/// Only spans created by this workspace's crates (target `sagat_*`) are exported, so the spans of
/// a driver or a framework never leave the process. A span is exported when it has a trace: its own
/// `trace_id` field, or the trace of its parent. Its `span_id` and `parent_span_id` fields, when it
/// has them, are the ids it takes (so that a handler can know its span's id); otherwise they are
/// drawn, and the parent is the enclosing span.
pub struct OtlpLayer {
    sink: SpanSink,
    ratio: f64,
}

impl OtlpLayer {
    #[must_use]
    pub const fn new(sink: SpanSink, ratio: f64) -> Self {
        Self { sink, ratio }
    }
}

impl<S> Layer<S> for OtlpLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let meta = attrs.metadata();
        if !meta.target().starts_with("sagat_") {
            return;
        }
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        let Some(span) = ctx.span(id) else { return };
        let inherited = span.parent().and_then(|parent| {
            parent
                .extensions()
                .get::<Open>()
                .map(|open| (open.trace_id, open.span_id, open.sampled))
        });
        let (trace_id, parent, sampled) = match (fields.trace_id, inherited) {
            (Some(trace), _) => (trace, fields.parent, keeps(trace, self.ratio)),
            (None, Some((trace, parent_span, sampled))) => {
                (trace, fields.parent.or(Some(parent_span)), sampled)
            }
            (None, None) => return,
        };
        let sampled = sampled && fields.sampled.unwrap_or(true);
        span.extensions_mut().insert(Open {
            trace_id,
            span_id: fields.span_id.unwrap_or_else(SpanId::generate),
            parent,
            sampled,
            name: meta.name(),
            kind: fields.kind.unwrap_or(SpanKind::Internal),
            start: unix_nano_now(),
            attributes: fields.attributes,
            status: fields.error.map_or(SpanStatus::Unset, SpanStatus::Error),
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut extensions = span.extensions_mut();
        let Some(open) = extensions.get_mut::<Open>() else {
            return;
        };
        let mut fields = Fields::default();
        values.record(&mut fields);
        for (name, value) in fields.attributes {
            if let Some(slot) = open.attributes.iter_mut().find(|(key, _)| *key == name) {
                slot.1 = value;
            } else {
                open.attributes.push((name, value));
            }
        }
        if let Some(message) = fields.error {
            open.status = SpanStatus::Error(message);
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let Some(open) = span.extensions_mut().remove::<Open>() else {
            return;
        };
        if !open.sampled {
            return;
        }
        self.sink.submit(SpanData {
            trace_id: open.trace_id,
            span_id: open.span_id,
            parent: open.parent,
            name: open.name.to_owned(),
            kind: open.kind,
            start_unix_nano: open.start,
            end_unix_nano: unix_nano_now(),
            attributes: open.attributes,
            events: Vec::new(),
            links: Vec::new(),
            status: open.status,
        });
    }
}

/// A setting of the exporter that is wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid value for {name}: {reason}")]
pub struct OtlpConfigError {
    pub name: &'static str,
    pub reason: String,
}

fn invalid(name: &'static str, reason: impl Into<String>) -> OtlpConfigError {
    OtlpConfigError {
        name,
        reason: reason.into(),
    }
}

impl OtlpConfig {
    /// The exporter the environment asks for, with the standard `OTEL_*` variables: none when no
    /// endpoint is set (or `OTEL_SDK_DISABLED=true`), which is the default.
    ///
    /// Only `http/json` is spoken (D161): a protocol the exporter does not speak is an error, not a
    /// silent fallback, because spans sent to a port that expects another protocol are lost without
    /// a sign. The same for a sampler it does not know.
    ///
    /// # Errors
    /// [`OtlpConfigError`] for a value that is set and cannot be used.
    pub fn from_lookup(
        get: &dyn Fn(&str) -> Option<String>,
        default_service: &str,
    ) -> Result<Option<Self>, OtlpConfigError> {
        let var = |name: &str| {
            get(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        if var("OTEL_SDK_DISABLED").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
            return Ok(None);
        }
        let (name, endpoint) = if let Some(full) = var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") {
            ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", full)
        } else if let Some(base) = var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            (
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                format!("{}/v1/traces", base.trim_end_matches('/')),
            )
        } else {
            return Ok(None);
        };
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Err(invalid(name, "must be an http:// or https:// URL"));
        }
        let authority = endpoint.split("://").nth(1).unwrap_or_default();
        if authority
            .split('/')
            .next()
            .unwrap_or_default()
            .contains('@')
        {
            return Err(invalid(
                name,
                "must not carry a user name or password; use OTEL_EXPORTER_OTLP_HEADERS",
            ));
        }
        for protocol_var in [
            "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
            "OTEL_EXPORTER_OTLP_PROTOCOL",
        ] {
            if let Some(protocol) = var(protocol_var).filter(|p| p != "http/json") {
                return Err(invalid(
                    "OTEL_EXPORTER_OTLP_PROTOCOL",
                    format!("'{protocol}' is not spoken; this build exports http/json only"),
                ));
            }
        }
        let service = var("OTEL_SERVICE_NAME").unwrap_or_else(|| default_service.to_owned());
        let mut config = Self::new(endpoint, service);
        if let Some(headers) =
            var("OTEL_EXPORTER_OTLP_TRACES_HEADERS").or_else(|| var("OTEL_EXPORTER_OTLP_HEADERS"))
        {
            for pair in headers.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                let (key, value) = pair
                    .split_once('=')
                    .filter(|(k, _)| !k.trim().is_empty())
                    .ok_or_else(|| {
                        invalid(
                            "OTEL_EXPORTER_OTLP_HEADERS",
                            "expected name=value pairs separated by commas",
                        )
                    })?;
                config
                    .headers
                    .push((key.trim().to_owned(), value.trim().to_owned()));
            }
        }
        let ratio_arg = var("OTEL_TRACES_SAMPLER_ARG");
        config.sample_ratio = match var("OTEL_TRACES_SAMPLER").as_deref() {
            None | Some("always_on" | "parentbased_always_on") => 1.0,
            Some("always_off" | "parentbased_always_off") => 0.0,
            Some("traceidratio" | "parentbased_traceidratio") => ratio_arg
                .as_deref()
                .ok_or_else(|| {
                    invalid(
                        "OTEL_TRACES_SAMPLER_ARG",
                        "the ratio is required by this sampler",
                    )
                })?
                .parse::<f64>()
                .ok()
                .filter(|r| (0.0..=1.0).contains(r))
                .ok_or_else(|| {
                    invalid("OTEL_TRACES_SAMPLER_ARG", "must be a number from 0 to 1")
                })?,
            Some(other) => {
                return Err(invalid(
                    "OTEL_TRACES_SAMPLER",
                    format!("'{other}' is not supported (always_on, always_off, traceidratio)"),
                ));
            }
        };
        if let Some(raw) = var("SAGAT_OTLP_QUEUE_CAPACITY") {
            config.queue_capacity = raw
                .parse::<usize>()
                .ok()
                .filter(|n| (16..=65_536).contains(n))
                .ok_or_else(|| {
                    invalid(
                        "SAGAT_OTLP_QUEUE_CAPACITY",
                        "must be a number from 16 to 65536",
                    )
                })?;
        }
        if let Some(raw) = var("OTEL_EXPORTER_OTLP_TIMEOUT") {
            let millis = raw
                .parse::<u64>()
                .ok()
                .filter(|n| (100..=60_000).contains(n))
                .ok_or_else(|| {
                    invalid(
                        "OTEL_EXPORTER_OTLP_TIMEOUT",
                        "must be milliseconds from 100 to 60000",
                    )
                })?;
            config.timeout = Duration::from_millis(millis);
        }
        Ok(Some(config))
    }
}

//! Export of spans to a collector, against a collector that can be told to misbehave (G6).
#![allow(clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use serde_json::Value;
use tracing_subscriber::layer::SubscriberExt;

use sagat_telemetry::{
    AttrValue, ExportError, Metrics, OtlpClient, OtlpConfig, OtlpLayer, SpanData, SpanEvent,
    SpanId, SpanKind, SpanLink, SpanStatus, TraceId, start,
};

#[derive(Default)]
struct Collector {
    /// The status of each request, in order; 200 once the list is empty.
    statuses: Mutex<VecDeque<u16>>,
    bodies: Mutex<Vec<Value>>,
    headers: Mutex<Vec<HeaderMap>>,
}

async fn receive(
    State(c): State<Arc<Collector>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    c.headers.lock().expect("headers").push(headers);
    let status = c
        .statuses
        .lock()
        .expect("statuses")
        .pop_front()
        .unwrap_or(200);
    if status == 200 {
        c.bodies
            .lock()
            .expect("bodies")
            .push(serde_json::from_slice(&body).expect("the body is JSON"));
    }
    StatusCode::from_u16(status).expect("status")
}

async fn collector(statuses: &[u16]) -> (Arc<Collector>, String) {
    let state = Arc::new(Collector::default());
    state
        .statuses
        .lock()
        .expect("statuses")
        .extend(statuses.iter().copied());
    let app = Router::new()
        .route("/v1/traces", post(receive))
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = format!("http://{}/v1/traces", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (state, url)
}

fn a_span(name: &str) -> SpanData {
    SpanData {
        trace_id: TraceId::generate(),
        span_id: SpanId::generate(),
        parent: Some(SpanId::generate()),
        name: name.to_owned(),
        kind: SpanKind::Server,
        start_unix_nano: 1_700_000_000_000_000_000,
        end_unix_nano: 1_700_000_000_250_000_000,
        attributes: vec![
            ("sagat.node".to_owned(), AttrValue::from("charge")),
            ("sagat.attempt".to_owned(), AttrValue::from(3_u32)),
            ("sagat.retried".to_owned(), AttrValue::from(true)),
        ],
        events: vec![SpanEvent {
            name: "retry scheduled".to_owned(),
            unix_nano: 1_700_000_000_100_000_000,
            attributes: vec![("sagat.code".to_owned(), AttrValue::from("TIMEOUT"))],
        }],
        links: vec![SpanLink {
            trace_id: TraceId::generate(),
            span_id: SpanId::generate(),
        }],
        status: SpanStatus::Error("worker lost".to_owned()),
    }
}

fn config(url: &str) -> OtlpConfig {
    let mut config = OtlpConfig::new(url, "sagat-test");
    config.timeout = Duration::from_secs(2);
    config.retries = 3;
    config
}

fn spans_of(body: &Value) -> Vec<Value> {
    body["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
async fn a_span_is_written_the_way_otlp_json_wants_it() {
    let (collector, url) = collector(&[]).await;
    let client = OtlpClient::new(config(&url)).expect("client");
    let span = a_span("step.attempt");
    client
        .export(std::slice::from_ref(&span))
        .await
        .expect("exported");

    let body = collector.bodies.lock().expect("bodies")[0].clone();
    let resource = &body["resourceSpans"][0]["resource"]["attributes"];
    assert_eq!(resource[0]["key"], "service.name");
    assert_eq!(resource[0]["value"]["stringValue"], "sagat-test");
    let sent = &spans_of(&body)[0];
    assert_eq!(sent["traceId"], span.trace_id.to_string());
    assert_eq!(sent["spanId"], span.span_id.to_string());
    assert_eq!(
        sent["parentSpanId"],
        span.parent.expect("parent").to_string()
    );
    assert_eq!(sent["name"], "step.attempt");
    assert_eq!(sent["kind"], 2);
    // 64-bit integers travel as strings.
    assert_eq!(sent["startTimeUnixNano"], "1700000000000000000");
    assert_eq!(sent["endTimeUnixNano"], "1700000000250000000");
    let attributes = sent["attributes"].as_array().expect("attributes");
    assert!(
        attributes
            .iter()
            .any(|a| a["key"] == "sagat.attempt" && a["value"]["intValue"] == "3")
    );
    assert!(
        attributes
            .iter()
            .any(|a| a["key"] == "sagat.retried" && a["value"]["boolValue"] == true)
    );
    assert_eq!(sent["events"][0]["name"], "retry scheduled");
    assert_eq!(
        sent["links"][0]["traceId"],
        span.links[0].trace_id.to_string()
    );
    assert_eq!(sent["status"]["code"], 2);
    assert_eq!(sent["status"]["message"], "worker lost");
}

#[tokio::test]
async fn a_span_with_no_parent_has_no_parent_field_and_an_end_never_precedes_the_start() {
    let (collector, url) = collector(&[]).await;
    let client = OtlpClient::new(config(&url)).expect("client");
    let mut span = a_span("run");
    span.parent = None;
    span.end_unix_nano = span.start_unix_nano - 5;
    client.export(&[span.clone()]).await.expect("exported");
    let body = collector.bodies.lock().expect("bodies")[0].clone();
    let sent = &spans_of(&body)[0];
    assert!(sent.get("parentSpanId").is_none());
    assert_eq!(sent["endTimeUnixNano"], sent["startTimeUnixNano"]);
}

#[tokio::test]
async fn the_headers_of_the_configuration_are_sent_and_never_shown() {
    let (collector, url) = collector(&[]).await;
    let mut cfg = config(&url);
    cfg.headers = vec![(
        "authorization".to_owned(),
        "Bearer collector-key-123456".to_owned(),
    )];
    assert!(!format!("{cfg:?}").contains("collector-key"), "{cfg:?}");
    let client = OtlpClient::new(cfg).expect("client");
    client.export(&[a_span("x")]).await.expect("exported");
    let headers = collector.headers.lock().expect("headers")[0].clone();
    assert_eq!(headers["authorization"], "Bearer collector-key-123456");
    assert_eq!(headers["content-type"], "application/json");
}

#[tokio::test]
async fn a_secret_in_an_attribute_or_a_name_does_not_leave_the_process() {
    let (collector, url) = collector(&[]).await;
    let client = OtlpClient::new(config(&url)).expect("client");
    let mut span = a_span("calling postgres://sagat:hunter2@db/x");
    span.attributes.push((
        "lease_token".to_owned(),
        AttrValue::from("s3cr3t-lease-token"),
    ));
    span.attributes.push((
        "note".to_owned(),
        AttrValue::from("Bearer abcDEF1234567890xyz"),
    ));
    client.export(&[span]).await.expect("exported");
    let raw = serde_json::to_string(&collector.bodies.lock().expect("bodies")[0]).expect("json");
    for hidden in ["hunter2", "s3cr3t-lease-token", "abcDEF1234567890xyz"] {
        assert!(!raw.contains(hidden), "{hidden} was exported: {raw}");
    }
}

#[tokio::test]
async fn a_collector_that_says_to_come_back_is_tried_again_and_the_batch_arrives_once() {
    let (collector, url) = collector(&[503, 429]).await;
    let client = OtlpClient::new(config(&url)).expect("client");
    client
        .export(&[a_span("x")])
        .await
        .expect("exported after retries");
    assert_eq!(collector.headers.lock().expect("headers").len(), 3);
    assert_eq!(collector.bodies.lock().expect("bodies").len(), 1);
}

#[tokio::test]
async fn a_batch_the_collector_refuses_is_not_sent_again() {
    let (collector, url) = collector(&[400, 200]).await;
    let client = OtlpClient::new(config(&url)).expect("client");
    let error = client.export(&[a_span("x")]).await.expect_err("refused");
    assert_eq!(error, ExportError::Rejected(400));
    assert_eq!(collector.headers.lock().expect("headers").len(), 1);
}

#[tokio::test]
async fn a_collector_that_stays_down_ends_in_an_error_after_the_retries() {
    let (_collector, url) = collector(&[503, 503, 503, 503, 503]).await;
    let mut cfg = config(&url);
    cfg.retries = 2;
    let error = OtlpClient::new(cfg)
        .expect("client")
        .export(&[a_span("x")])
        .await
        .expect_err("down");
    assert!(matches!(error, ExportError::Unavailable(_)), "{error:?}");
    // Nothing listens on the port: the same.
    let mut gone = config("http://127.0.0.1:1/v1/traces");
    gone.retries = 1;
    let error = OtlpClient::new(gone)
        .expect("client")
        .export(&[a_span("x")])
        .await
        .expect_err("gone");
    assert!(matches!(error, ExportError::Unavailable(_)), "{error:?}");
}

#[tokio::test]
async fn an_empty_batch_sends_nothing() {
    let (collector, url) = collector(&[]).await;
    OtlpClient::new(config(&url))
        .expect("client")
        .export(&[])
        .await
        .expect("ok");
    assert!(collector.headers.lock().expect("headers").is_empty());
}

#[tokio::test]
async fn spans_queued_before_shutdown_are_sent_by_it() {
    let (collector, url) = collector(&[]).await;
    let mut cfg = config(&url);
    cfg.flush_interval = Duration::from_secs(3600);
    let sink = start(OtlpClient::new(cfg).expect("client"));
    for n in 0..5 {
        sink.submit(a_span(&format!("span-{n}")));
    }
    sink.shutdown(Duration::from_secs(5)).await;
    let sent: usize = collector
        .bodies
        .lock()
        .expect("bodies")
        .iter()
        .map(|b| spans_of(b).len())
        .sum();
    assert_eq!(sent, 5);
}

#[tokio::test]
async fn a_full_queue_drops_the_span_and_counts_it_instead_of_waiting() {
    // A collector that never answers keeps the exporter busy with its first batch.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let url = format!("http://{}/v1/traces", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let mut cfg = config(&url);
    cfg.queue_capacity = 2;
    cfg.batch_size = 1;
    cfg.timeout = Duration::from_secs(30);
    let sink = start(OtlpClient::new(cfg).expect("client"));

    let (metrics, recorder) = Metrics::detached(Duration::from_secs(60));
    let started = std::time::Instant::now();
    metrics::with_local_recorder(&recorder, || {
        for n in 0..50 {
            sink.submit(a_span(&format!("span-{n}")));
        }
    });
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "submitting waited for the exporter"
    );
    let page = metrics.render();
    assert!(
        page.contains("otel_spans_total{outcome=\"dropped\"}"),
        "{page}"
    );
    assert!(
        page.contains("otel_spans_total{outcome=\"queued\"}"),
        "{page}"
    );
}

fn exporting(
    url: &str,
    ratio: f64,
) -> (
    sagat_telemetry::SpanSink,
    impl tracing::Subscriber + Send + Sync,
) {
    let mut cfg = config(url);
    cfg.flush_interval = Duration::from_secs(3600);
    let sink = start(OtlpClient::new(cfg).expect("client"));
    let subscriber = tracing_subscriber::registry().with(OtlpLayer::new(sink.clone(), ratio));
    (sink, subscriber)
}

#[tokio::test]
async fn a_span_with_a_trace_is_exported_with_its_parent_and_the_status_it_ended_with() {
    let (collector, url) = collector(&[]).await;
    let (sink, subscriber) = exporting(&url, 1.0);
    let trace = TraceId::generate();
    let ingress = SpanId::generate();
    tracing::subscriber::with_default(subscriber, || {
        let request = tracing::info_span!(
            target: "sagat_test",
            "http.request",
            trace_id = %trace,
            span_id = %ingress,
            otel.kind = "server",
            route = "/runs",
            http.status_code = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
        );
        let _entered = request.enter();
        {
            let child = tracing::info_span!(target: "sagat_test", "journal.append", events = 3_u64);
            let _child = child.enter();
        }
        request.record("http.status_code", 503_u64);
        request.record("otel.status_code", "error");
    });
    sink.shutdown(Duration::from_secs(5)).await;

    let spans: Vec<Value> = collector
        .bodies
        .lock()
        .expect("bodies")
        .iter()
        .flat_map(spans_of)
        .collect();
    assert_eq!(spans.len(), 2, "{spans:?}");
    let request = spans
        .iter()
        .find(|s| s["name"] == "http.request")
        .expect("request span");
    let append = spans
        .iter()
        .find(|s| s["name"] == "journal.append")
        .expect("child span");
    assert_eq!(request["traceId"], trace.to_string());
    assert_eq!(request["spanId"], ingress.to_string());
    assert_eq!(request["kind"], 2);
    assert_eq!(request["status"]["code"], 2);
    let attrs = request["attributes"].as_array().expect("attributes");
    assert!(
        attrs
            .iter()
            .any(|a| a["key"] == "route" && a["value"]["stringValue"] == "/runs")
    );
    assert!(
        attrs
            .iter()
            .any(|a| a["key"] == "http.status_code" && a["value"]["intValue"] == "503")
    );
    assert_eq!(append["traceId"], trace.to_string());
    assert_eq!(append["parentSpanId"], ingress.to_string());
    assert!(
        append["attributes"]
            .as_array()
            .expect("attributes")
            .iter()
            .any(|a| a["key"] == "events")
    );
}

#[tokio::test]
async fn a_span_may_continue_a_trace_under_a_parent_it_names() {
    let (collector, url) = collector(&[]).await;
    let (sink, subscriber) = exporting(&url, 1.0);
    let (trace, parent) = (TraceId::generate(), SpanId::generate());
    tracing::subscriber::with_default(subscriber, || {
        let pass = tracing::info_span!(target: "sagat_test", "scheduler.pass", trace_id = %trace, parent_span_id = %parent);
        drop(pass.enter());
    });
    sink.shutdown(Duration::from_secs(5)).await;
    let spans: Vec<Value> = collector
        .bodies
        .lock()
        .expect("bodies")
        .iter()
        .flat_map(spans_of)
        .collect();
    assert_eq!(spans[0]["parentSpanId"], parent.to_string());
}

#[tokio::test]
async fn spans_of_other_libraries_and_spans_with_no_trace_never_leave_the_process() {
    let (collector, url) = collector(&[]).await;
    let (sink, subscriber) = exporting(&url, 1.0);
    tracing::subscriber::with_default(subscriber, || {
        // A driver's span, even with a trace field.
        let theirs =
            tracing::info_span!(target: "sqlx::query", "query", trace_id = %TraceId::generate());
        drop(theirs.enter());
        // Ours, but with no trace to belong to.
        let orphan = tracing::info_span!(target: "sagat_test", "orphan");
        drop(orphan.enter());
    });
    sink.shutdown(Duration::from_secs(5)).await;
    assert!(collector.bodies.lock().expect("bodies").is_empty());
}

#[tokio::test]
async fn the_ratio_decides_from_the_trace_id_and_a_dropped_trace_exports_none_of_its_spans() {
    let (collector, url) = collector(&[]).await;
    let (sink, subscriber) = exporting(&url, 0.0);
    tracing::subscriber::with_default(subscriber, || {
        let root =
            tracing::info_span!(target: "sagat_test", "root", trace_id = %TraceId::generate());
        let _root = root.enter();
        let child = tracing::info_span!(target: "sagat_test", "child");
        drop(child.enter());
    });
    sink.shutdown(Duration::from_secs(5)).await;
    assert!(collector.bodies.lock().expect("bodies").is_empty());
}

#[tokio::test]
async fn a_sensitive_field_of_a_span_is_exported_hidden() {
    let (collector, url) = collector(&[]).await;
    let (sink, subscriber) = exporting(&url, 1.0);
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!(target: "sagat_test", "grpc.worker", trace_id = %TraceId::generate(), lease_token = "s3cr3t-lease", method = "Poll");
        drop(span.enter());
    });
    sink.shutdown(Duration::from_secs(5)).await;
    let raw = serde_json::to_string(&*collector.bodies.lock().expect("bodies")).expect("json");
    assert!(!raw.contains("s3cr3t-lease"), "{raw}");
    assert!(raw.contains("Poll"));
}

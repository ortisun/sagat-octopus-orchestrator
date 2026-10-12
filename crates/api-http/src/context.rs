//! Request context: trace identity, security headers, error-envelope completion and access log.

use std::time::Instant;

use axum::body::Body;
use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::Response;
use sagat_telemetry::{SpanId, TraceContext, TraceId};
use serde::Serialize;
use serde_json::Value;
use tracing::Instrument;

use crate::error::{ErrorCode, ErrorParts};

static REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");
static TRACE_ID_HEADER: HeaderName = HeaderName::from_static("x-trace-id");
static NOSNIFF: HeaderValue = HeaderValue::from_static("nosniff");

/// The span of the request itself: what the work it starts (a run, a published event) hangs from.
#[derive(Debug, Clone, Copy)]
pub struct IngressSpan(pub SpanId);

/// The route template a request matched (low-cardinality label for logs and metrics).
#[derive(Debug, Clone)]
struct RouteTemplate(String);

/// Inner middleware: remembers the matched route template for the outer one.
pub async fn record_route(request: Request, next: Next) -> Response {
    let template = request
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned());
    let mut response = next.run(request).await;
    if let Some(template) = template {
        response.extensions_mut().insert(RouteTemplate(template));
    }
    response
}

/// The method as a metric label and log field: one of a closed set, `OTHER` for anything else.
///
/// HTTP/1 lets a client send any token as the method, and the recorder keeps every series it has
/// seen, so using the raw method would let an anonymous client grow memory without bound.
fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        _ => "OTHER",
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody<'a> {
    code: ErrorCode,
    message: &'a str,
    retryable: bool,
    trace_id: String,
    request_id: String,
    /// What to do about it (SC-008-004): from a closed table, never from the data of the request.
    remediation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a Value>,
}

/// Outermost middleware.
pub async fn request_context(mut request: Request, next: Next) -> Response {
    let trace = TraceContext::from_traceparent(
        request
            .headers()
            .get("traceparent")
            .and_then(|v| v.to_str().ok()),
    );
    let trace_id = trace.trace_id();
    // A trace may span many requests and its parent is caller-controlled. This id is always new.
    let request_id = TraceId::generate();
    request.extensions_mut().insert(trace);
    // The id is chosen here, not by the exporter, so that the handler can record it on what it
    // starts: a run's span hangs from the request that started it.
    let ingress = SpanId::generate();
    request.extensions_mut().insert(IngressSpan(ingress));

    let method = method_label(request.method());
    let span = tracing::info_span!(
        "http.request",
        trace_id = %trace_id,
        request_id = %request_id,
        span_id = %ingress,
        parent_span_id = trace.parent_span_id().map(|p| p.to_string()),
        otel.kind = "server",
        method,
        http.route = tracing::field::Empty,
        http.status_code = tracing::field::Empty,
        otel.status_code = tracing::field::Empty,
    );
    let started = Instant::now();

    let response = next.run(request).instrument(span.clone()).await;
    let route = response
        .extensions()
        .get::<RouteTemplate>()
        .map_or_else(|| "unmatched".to_owned(), |r| r.0.clone());
    let response = finalize(response, trace_id, request_id);
    span.record("http.route", route.as_str());
    span.record("http.status_code", response.status().as_u16());
    if response.status().is_server_error() {
        span.record("otel.status_code", "error");
    }

    // Low-cardinality labels only: the route TEMPLATE, never the path, tenant or subject, and a
    // method from a closed set (the server accepts any token as a method).
    let status = response.status().as_u16().to_string();
    metrics::counter!("http_requests_total", "method" => method, "route" => route.clone(), "status" => status).increment(1);
    metrics::histogram!("http_request_duration_seconds", "route" => route.clone())
        .record(started.elapsed().as_secs_f64());

    // Identifiers only: never headers, bodies, tokens or subjects.
    tracing::info!(
        parent: &span,
        route = %route,
        status = response.status().as_u16(),
        latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "request completed"
    );
    response
}

fn finalize(response: Response, trace_id: TraceId, request_id: TraceId) -> Response {
    let status = response.status();
    let (mut head, body) = response.into_parts();

    let body = if status.is_client_error() || status.is_server_error() {
        let parts = head
            .extensions
            .remove::<ErrorParts>()
            .unwrap_or_else(|| ErrorParts::synthesized(status));
        let envelope = ErrorBody {
            code: parts.code,
            message: &parts.message,
            retryable: parts.retryable,
            trace_id: trace_id.to_string(),
            request_id: request_id.to_string(),
            remediation: parts.code.remediation(
                parts
                    .details
                    .as_ref()
                    .and_then(|d| d.get("reason"))
                    .and_then(Value::as_str),
            ),
            details: parts.details.as_ref(),
        };
        // Serializing a plain struct cannot fail; fall back to an empty body just in case.
        let bytes = serde_json::to_vec(&envelope).unwrap_or_default();
        head.headers.remove(header::CONTENT_LENGTH);
        head.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        Body::from(bytes)
    } else {
        body
    };

    if let Ok(value) = HeaderValue::from_str(&trace_id.to_string()) {
        head.headers.insert(TRACE_ID_HEADER.clone(), value);
    }
    if let Ok(value) = HeaderValue::from_str(&request_id.to_string()) {
        head.headers.insert(REQUEST_ID_HEADER.clone(), value);
    }
    head.headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    head.headers
        .insert(header::X_CONTENT_TYPE_OPTIONS, NOSNIFF.clone());
    Response::from_parts(head, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::middleware::from_fn;
    use axum::routing::get;
    use std::collections::HashSet;

    #[tokio::test]
    async fn requests_with_the_same_parent_get_distinct_server_ids_on_every_response() {
        let app = axum::Router::new()
            .route("/ok", get(|| async { StatusCode::NO_CONTENT }))
            .route(
                "/failure",
                get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
            )
            .layer(from_fn(request_context));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::new();
        let mut ids = HashSet::new();
        let parent = "00-11111111111111111111111111111111-2222222222222222-01";
        for path in ["/ok", "/failure", "/missing", "/failure"] {
            let response = client
                .get(format!("http://{address}{path}"))
                .header("traceparent", parent)
                .header("x-request-id", "attacker-chosen-secret")
                .send()
                .await
                .expect("response");
            let request_id = response
                .headers()
                .get("x-request-id")
                .expect("every response carries the server request id")
                .to_str()
                .expect("ASCII")
                .to_owned();
            assert_eq!(request_id.len(), 32);
            assert!(request_id.bytes().all(|b| b.is_ascii_hexdigit()));
            assert_ne!(request_id, "11111111111111111111111111111111");
            assert!(ids.insert(request_id.clone()), "new id for each request");
            assert_eq!(
                response.headers()["x-trace-id"],
                "11111111111111111111111111111111"
            );
            if response.status().is_client_error() || response.status().is_server_error() {
                let body: Value = serde_json::from_slice(&response.bytes().await.expect("body"))
                    .expect("envelope");
                assert_eq!(body["requestId"], request_id);
                assert_eq!(body["traceId"], "11111111111111111111111111111111");
                assert!(body["remediation"].as_str().is_some_and(|s| !s.is_empty()));
                assert!(!body.to_string().contains("attacker-chosen-secret"));
            }
        }
        server.abort();
    }
}

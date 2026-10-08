//! Request context: trace identity, security headers, error-envelope completion and access log.

use std::time::Instant;

use axum::body::Body;
use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use sagat_telemetry::{TraceContext, TraceId};
use serde::Serialize;
use serde_json::Value;
use tracing::Instrument;

use crate::error::{ErrorCode, ErrorParts};

static TRACE_ID_HEADER: HeaderName = HeaderName::from_static("x-trace-id");
static NOSNIFF: HeaderValue = HeaderValue::from_static("nosniff");

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody<'a> {
    code: ErrorCode,
    message: &'a str,
    retryable: bool,
    trace_id: String,
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
    request.extensions_mut().insert(trace);

    let method = request.method().clone();
    let span = tracing::info_span!("http.request", trace_id = %trace_id, %method);
    let started = Instant::now();

    let response = next.run(request).instrument(span.clone()).await;
    let route = response
        .extensions()
        .get::<RouteTemplate>()
        .map_or_else(|| "unmatched".to_owned(), |r| r.0.clone());
    let response = finalize(response, trace_id);

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

fn finalize(response: Response, trace_id: TraceId) -> Response {
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
    head.headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    head.headers
        .insert(header::X_CONTENT_TYPE_OPTIONS, NOSNIFF.clone());
    Response::from_parts(head, body)
}

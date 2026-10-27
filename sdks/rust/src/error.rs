use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::OnceLock, time::Duration};
#[derive(Deserialize)]
struct Catalog {
    errors: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    code: String,
    retryable: bool,
    remediation: String,
}
fn entry(code: &str) -> Option<&'static Entry> {
    static CATALOG: OnceLock<Vec<Entry>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            serde_json::from_str::<Catalog>(include_str!("error-catalog.json"))
                .map(|c| c.errors)
                .unwrap_or_default()
        })
        .iter()
        .find(|entry| entry.code == code)
}
/// Canonical SDK failure. Debug omits response bodies and provider/transport details.
#[derive(Clone)]
pub struct SagatError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub trace_id: Option<String>,
    pub details: Box<Value>,
    pub status: Option<u16>,
    pub retry_after: Option<Duration>,
    pub remediation: &'static str,
    provider_failed: bool,
}
impl std::fmt::Debug for SagatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SagatError")
            .field("code", &self.code)
            .field("status", &self.status)
            .field("retryable", &self.retryable)
            .field("trace_id", &self.trace_id)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for SagatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for SagatError {}
impl SagatError {
    pub fn grpc_status(&self) -> Option<&str> {
        self.details.get("grpcStatus").and_then(Value::as_str)
    }
    pub(crate) fn new(code: &str, message: &str) -> Self {
        let catalog = entry(code);
        Self {
            code: catalog.map_or("INTERNAL", |e| e.code.as_str()),
            message: message.chars().take(512).collect(),
            retryable: catalog.is_some_and(|e| e.retryable),
            trace_id: None,
            details: Box::new(Value::Null),
            status: None,
            retry_after: None,
            provider_failed: false,
            remediation: catalog
                .map_or("Consult the API reference and report the trace id.", |e| {
                    e.remediation.as_str()
                }),
        }
    }
    pub(crate) fn provider_failure() -> Self {
        let mut error = Self::new("UNAUTHENTICATED", "the token provider failed");
        error.provider_failed = true;
        error.details = Box::new(json!({"reason":"TOKEN_PROVIDER_FAILED"}));
        error
    }
    pub(crate) fn is_provider_failure(&self) -> bool {
        self.provider_failed
    }
    pub(crate) fn may_retry(&self) -> bool {
        self.provider_failed
            || (self.retryable
                && matches!(
                    self.code,
                    "TRANSPORT" | "UNAVAILABLE" | "RATE_LIMITED" | "INTERNAL"
                ))
    }
    pub(crate) fn from_http(
        status: u16,
        headers: &reqwest::header::HeaderMap,
        body: &[u8],
    ) -> Self {
        if (300..400).contains(&status) {
            let mut error = Self::new("PROTOCOL", "the server redirected the call");
            error.status = Some(status);
            error.trace_id = headers
                .get("x-trace-id")
                .and_then(|v| v.to_str().ok())
                .and_then(trace_id);
            return error;
        }
        let envelope = serde_json::from_slice::<Value>(body)
            .ok()
            .filter(Value::is_object);
        let given = envelope
            .as_ref()
            .and_then(|v| v["code"].as_str())
            .filter(|code| {
                matches!(
                    *code,
                    "BAD_REQUEST"
                        | "UNAUTHENTICATED"
                        | "FORBIDDEN"
                        | "NOT_FOUND"
                        | "CONFLICT"
                        | "VALIDATION_FAILED"
                        | "RATE_LIMITED"
                        | "QUOTA_EXCEEDED"
                        | "UNAVAILABLE"
                        | "INTERNAL"
                )
            });
        let code = given.unwrap_or(match status {
            400 | 413 | 415 => "BAD_REQUEST",
            401 => "UNAUTHENTICATED",
            403 => "FORBIDDEN",
            404 | 405 | 501 => "NOT_FOUND",
            409 => "CONFLICT",
            422 => "VALIDATION_FAILED",
            429 => "RATE_LIMITED",
            408 | 502..=504 => "UNAVAILABLE",
            _ => "INTERNAL",
        });
        let mut error = Self::new(code, "the server refused the request");
        error.status = Some(status);
        if let Some(envelope) = envelope.as_ref() {
            if given.is_some()
                && let Some(message) = envelope["message"].as_str()
            {
                error.message = message.chars().take(512).collect();
            }
            if let Some(retryable) = envelope["retryable"].as_bool() {
                error.retryable = retryable;
            }
            if let Some(reason) = envelope["details"]["reason"].as_str() {
                error.details =
                    Box::new(json!({"reason":reason.chars().take(128).collect::<String>()}));
            }
        }
        if (400..500).contains(&status) && status != 408 && status != 429 {
            error.retryable = false;
        }
        if !matches!(
            code,
            "TRANSPORT" | "UNAVAILABLE" | "RATE_LIMITED" | "INTERNAL"
        ) {
            error.retryable = false;
        }
        error.trace_id = envelope
            .as_ref()
            .and_then(|v| v["traceId"].as_str())
            .and_then(trace_id)
            .or_else(|| {
                headers
                    .get("x-trace-id")
                    .and_then(|v| v.to_str().ok())
                    .and_then(trace_id)
            });
        if let Some(raw) = headers
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            && !raw.is_empty()
            && raw.bytes().all(|b| b.is_ascii_digit())
        {
            error.retry_after = Some(Duration::from_secs(
                raw.parse::<u64>().unwrap_or(u64::MAX).min(60),
            ));
        }
        let unsupported = status == 501
            || (status == 404 && given.is_none())
            || (status == 404 && error.details["reason"] == "ROUTE_NOT_FOUND")
            || (status == 405 && error.details["reason"] == "METHOD_NOT_ALLOWED");
        if unsupported {
            error.code = "NOT_FOUND";
            error.retryable = false;
            error.details = Box::new(json!({"reason":"UNSUPPORTED_BY_SERVER"}));
            error.remediation =
                entry("NOT_FOUND").map_or(error.remediation, |e| e.remediation.as_str());
        }
        error
    }
    /// Stable reason supplied by the server or a local SDK refusal. It never grants retry authority.
    pub fn reason(&self) -> Option<&str> {
        self.details.get("reason").and_then(Value::as_str)
    }
    pub fn as_json(&self) -> Value {
        json!({"code":self.code,"message":self.message,"retryable":self.retryable,"traceId":self.trace_id,"reason":self.details.get("reason"),"status":self.grpc_status().map_or_else(||json!(self.status),|s|json!(s)),"retryAfterMs":self.retry_after.map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64),"remediation":self.remediation})
    }
}
pub(crate) fn trace_id(value: &str) -> Option<String> {
    (value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

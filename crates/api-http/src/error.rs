//! The API's single error envelope (FR-001-006).
//!
//! Handlers return [`ApiError`]; it carries everything except the trace id, which the
//! request-context middleware adds so the id in the body always equals the `X-Trace-Id` header.
//! The middleware also wraps framework-generated errors (unknown route, bad method, oversized
//! body, timeout, panic) in the same envelope, so a client never sees a different error shape.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sagat_application::definitions::IrError;
use sagat_application::error::{AppError, ConflictKind};
use sagat_domain::scripts::ScriptError;
use serde::Serialize;
use serde_json::{Value, json};

use crate::auth::AuthError;

/// Stable, closed set of machine-readable codes (mirrors `Error.code` in the OpenAPI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    BadRequest,
    Unauthenticated,
    Forbidden,
    NotFound,
    Conflict,
    ValidationFailed,
    RateLimited,
    /// A limit of the tenant (or of the platform) was reached; unlike `RateLimited`, waiting does
    /// not help until something is released.
    QuotaExceeded,
    Unavailable,
    Internal,
}

impl ErrorCode {
    /// What a person (or a program) can do about it, in a sentence that names no data. The same for
    /// every error of the code, refined by the `reason` the error carries when that changes what to
    /// do: a closed table, so that what a failure tells a caller can be reviewed in one place and
    /// can never carry a payload, a secret or another tenant's name (SC-008-004).
    #[must_use]
    pub fn remediation(self, reason: Option<&str>) -> &'static str {
        match (self, reason.unwrap_or_default()) {
            (Self::Conflict, "IDEMPOTENCY_KEY_REUSED") => {
                "Use a new Idempotency-Key for a different request, or send the same request again with its key."
            }
            (Self::Conflict, "REVISION_CONFLICT") => {
                "Read the resource again, apply your change to what is there now and send it with the new revision."
            }
            (Self::Conflict, "STATE_CONFLICT") => {
                "Read the resource again: someone else may have already done this, or it is in a state that does not allow it."
            }
            (Self::NotFound, "ROUTE_NOT_FOUND") | (Self::BadRequest, "METHOD_NOT_ALLOWED") => {
                "This server does not have that call: check the path and method, and the version of the API it speaks."
            }
            (Self::BadRequest, _) => {
                "Correct the request (headers, body, query) and send it again."
            }
            (Self::Unauthenticated, _) => {
                "Send a valid bearer token issued for this platform, and check that it has not expired."
            }
            (Self::Forbidden, _) => {
                "Ask an administrator of the tenant for the permission the call needs."
            }
            (Self::NotFound, _) => {
                "Check the identifier and the namespace; a resource of another tenant is not visible."
            }
            (Self::Conflict, _) => {
                "Read the resource again and decide again: it is not in the state this call assumed."
            }
            (Self::ValidationFailed, _) => {
                "Correct what `details` names (a problem, a path) and send the request again."
            }
            (Self::RateLimited, _) => "Wait for the time in Retry-After and send fewer requests.",
            (Self::QuotaExceeded, _) => {
                "Release something the limit counts, or ask an administrator to raise the limit."
            }
            (Self::Unavailable, _) => {
                "Try again with a growing pause; if it lasts, check the platform's readiness and report the trace id."
            }
            (Self::Internal, _) => {
                "Try again once; if it happens again, report the trace id to the operator of the platform."
            }
        }
    }
}

/// An error before the trace id is attached. Travels in the response extensions.
#[derive(Debug, Clone)]
pub struct ErrorParts {
    pub status: StatusCode,
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    pub details: Option<Value>,
}

impl ErrorParts {
    /// Envelope for an error that no handler produced (router, layers).
    pub(crate) fn synthesized(status: StatusCode) -> Self {
        let (code, message, retryable) = match status.as_u16() {
            404 => (ErrorCode::NotFound, "resource not found", false),
            405 => (ErrorCode::BadRequest, "method not allowed", false),
            408 | 502..=504 => (
                ErrorCode::Unavailable,
                "service temporarily unavailable",
                true,
            ),
            413 => (ErrorCode::BadRequest, "request body is too large", false),
            415 => (ErrorCode::BadRequest, "unsupported media type", false),
            422 => (
                ErrorCode::ValidationFailed,
                "request could not be processed",
                false,
            ),
            400..=499 => (ErrorCode::BadRequest, "bad request", false),
            _ => (ErrorCode::Internal, "internal error", false),
        };
        // A call the server does not have says so, so that an SDK newer than the server can tell "this call
        // does not exist here" from "this run does not exist" (SDK SPEC §9).
        let details = match status.as_u16() {
            404 => Some(serde_json::json!({"reason": "ROUTE_NOT_FOUND"})),
            405 => Some(serde_json::json!({"reason": "METHOD_NOT_ALLOWED"})),
            _ => None,
        };
        Self {
            status,
            code,
            message: message.to_owned(),
            retryable,
            details,
        }
    }
}

/// Error type of all handlers and extractors.
#[derive(Debug, Clone)]
pub struct ApiError {
    parts: ErrorParts,
    retry_after: Option<u32>,
    www_authenticate: Option<&'static str>,
}

impl ApiError {
    fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            parts: ErrorParts {
                status,
                code,
                message: message.into(),
                retryable: false,
                details: None,
            },
            retry_after: None,
            www_authenticate: None,
        }
    }

    /// No credentials at all.
    #[must_use]
    pub fn missing_credentials() -> Self {
        let mut e = Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthenticated,
            "authentication required",
        );
        e.www_authenticate = Some("Bearer realm=\"sagat-octopus-orchestrator\"");
        e
    }

    /// Credentials were presented but refused. The reason is deliberately not disclosed.
    #[must_use]
    pub fn invalid_credentials() -> Self {
        let mut e = Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthenticated,
            "authentication required",
        );
        e.www_authenticate =
            Some("Bearer realm=\"sagat-octopus-orchestrator\", error=\"invalid_token\"");
        e
    }

    /// A request to a public webhook URL that did not verify. Every reason looks the same, and
    /// there is no challenge: nothing the caller could present would be a bearer token.
    #[must_use]
    pub fn webhook_rejected() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthenticated,
            "the request could not be verified",
        )
    }

    #[must_use]
    pub fn bad_request(message: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::BadRequest, message)
    }

    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::ValidationFailed,
            message,
        )
    }

    #[must_use]
    pub fn not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "resource not found",
        )
    }

    /// The tenant exceeded its request budget; `retry_after` is when a token will be available.
    #[must_use]
    pub fn rate_limited(retry_after: std::time::Duration) -> Self {
        let mut e = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "too many requests; slow down and retry",
        );
        e.parts.retryable = true;
        // Whole seconds, rounded up, never zero: a 0 would invite an immediate retry storm.
        e.retry_after = Some(
            u32::try_from(retry_after.as_secs_f64().ceil() as u64)
                .unwrap_or(u32::MAX)
                .max(1),
        );
        e
    }

    /// The client stopped sending its request body; it may retry.
    #[must_use]
    pub fn request_timeout() -> Self {
        let mut e = Self::new(
            StatusCode::REQUEST_TIMEOUT,
            ErrorCode::Unavailable,
            "the request body was not received in time",
        );
        e.parts.retryable = true;
        e
    }

    /// Dependency down; the client may retry.
    #[must_use]
    pub fn unavailable() -> Self {
        Self::unavailable_after(5)
    }

    /// Temporarily unable to serve; the client may retry after `seconds` (at least one).
    #[must_use]
    pub fn unavailable_after(seconds: u32) -> Self {
        let mut e = Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            "service temporarily unavailable",
        );
        e.parts.retryable = true;
        e.retry_after = Some(seconds.max(1));
        e
    }

    #[must_use]
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.parts.status = status;
        self
    }

    /// Names why in a form a client can branch on (`details.reason`).
    #[must_use]
    pub fn with_reason(mut self, reason: &'static str) -> Self {
        self.parts.details = Some(json!({ "reason": reason }));
        self
    }

    /// The error as it will be reported, for transports other than HTTP (the gRPC adapter maps the
    /// same classification onto its own status codes, so both agree on what an error means).
    #[must_use]
    pub fn parts(&self) -> &ErrorParts {
        &self.parts
    }

    /// Whole seconds after which the caller may retry, when the error says so.
    #[must_use]
    pub fn retry_after_seconds(&self) -> Option<u32> {
        self.retry_after
    }
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        tracing::debug!(%error, "bearer token refused");
        metrics::counter!("auth_failures_total", "reason" => error.label()).increment(1);
        match error {
            AuthError::Unavailable => Self::unavailable(),
            AuthError::Malformed | AuthError::Expired | AuthError::Invalid(_) => {
                Self::invalid_credentials()
            }
        }
    }
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::Unauthenticated => Self::invalid_credentials(),
            AppError::Forbidden { permission } => {
                let mut e = Self::new(
                    StatusCode::FORBIDDEN,
                    ErrorCode::Forbidden,
                    "the caller's role does not allow this operation",
                );
                e.parts.details = Some(json!({ "requiredPermission": permission.as_str() }));
                e
            }
            AppError::NotFound => Self::not_found(),
            AppError::SecretUnavailable { name } => {
                let mut e = Self::new(
                    StatusCode::FORBIDDEN,
                    ErrorCode::Forbidden,
                    "a secret does not exist or may not be used",
                );
                e.parts.details = Some(json!({ "reason": "SECRET_NOT_USABLE", "secret": name }));
                e
            }
            AppError::NotBound => {
                let mut e = Self::new(
                    StatusCode::FORBIDDEN,
                    ErrorCode::Forbidden,
                    "this identity is not bound to that namespace and activity",
                );
                e.parts.details = Some(json!({ "reason": "NOT_BOUND" }));
                e
            }
            AppError::Conflict(kind) => {
                let message = match kind {
                    ConflictKind::TenantSlug => "a tenant with this slug already exists",
                    ConflictKind::WorkspaceSlug => "a workspace with this slug already exists",
                    ConflictKind::NamespaceSlug => "a namespace with this slug already exists",
                    ConflictKind::WorkflowName => "a workflow with this name already exists",
                    ConflictKind::EventEndpointSlug => {
                        "an event endpoint with this slug already exists"
                    }
                    ConflictKind::EventIdReused => {
                        "this event id was already used for a different payload"
                    }
                    ConflictKind::WebhookSubscriptionName => {
                        "a webhook subscription with this name already exists"
                    }
                    ConflictKind::ConnectorSlug => "a connector with this slug already exists",
                    ConflictKind::ScriptName => "a script with this name already exists",
                };
                let mut e = Self::new(StatusCode::CONFLICT, ErrorCode::Conflict, message);
                e.parts.details = Some(match kind {
                    ConflictKind::WorkflowName
                    | ConflictKind::WebhookSubscriptionName
                    | ConflictKind::ScriptName => {
                        json!({ "field": "name" })
                    }
                    ConflictKind::EventIdReused => json!({ "reason": "EVENT_ID_REUSED" }),
                    _ => json!({ "field": "slug" }),
                });
                e
            }
            AppError::RevisionConflict { current } => {
                let mut e = Self::new(
                    StatusCode::CONFLICT,
                    ErrorCode::Conflict,
                    "the draft was changed since it was read; fetch it again and retry",
                );
                e.parts.details =
                    Some(json!({ "reason": "REVISION_CONFLICT", "currentRevision": current }));
                e
            }
            AppError::IdempotencyKeyReused => {
                let mut e = Self::new(
                    StatusCode::CONFLICT,
                    ErrorCode::Conflict,
                    "this Idempotency-Key was already used for a different request",
                );
                e.parts.details = Some(json!({ "reason": "IDEMPOTENCY_KEY_REUSED" }));
                e
            }
            AppError::InvalidDefinition(error) => {
                let mut e = Self::validation(error.to_string());
                e.parts.details = Some(definition_details(&error));
                e
            }
            AppError::NotAllowed(message) => {
                let mut e = Self::validation(message);
                if message == sagat_application::worker::UNSUPPORTED_PROTOCOL {
                    e.parts.details = Some(json!({ "reason": "UNSUPPORTED_PROTOCOL" }));
                }
                e
            }
            AppError::StaleAttempt => {
                let mut e = Self::new(
                    StatusCode::CONFLICT,
                    ErrorCode::Conflict,
                    "the attempt is no longer current",
                );
                e.parts.details = Some(json!({ "reason": "STALE_ATTEMPT" }));
                e
            }
            AppError::StateConflict(message) => {
                let mut e = Self::new(StatusCode::CONFLICT, ErrorCode::Conflict, message);
                e.parts.details = Some(json!({ "reason": "STATE_CONFLICT" }));
                e
            }
            AppError::Invalid(inner) => Self::validation(inner.to_string()),
            AppError::InvalidScript(inner) => {
                let mut e = Self::validation(inner.to_string());
                e.parts.details = Some(json!({ "reason": script_reason(inner) }));
                e
            }
            AppError::QuotaExceeded(what) => {
                // `what` is always a literal of this codebase, so the label set is closed.
                metrics::counter!("quota_exceeded_total", "limit" => what).increment(1);
                let mut e = Self::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    ErrorCode::QuotaExceeded,
                    "a limit was reached; release something and retry",
                );
                e.parts.details = Some(json!({ "limit": what }));
                e
            }
            AppError::Unavailable => Self::unavailable(),
            AppError::PayloadTooLarge(what) => {
                let mut e = Self::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    ErrorCode::BadRequest,
                    "the payload is larger than this endpoint takes",
                );
                e.parts.details = Some(json!({ "reason": "PAYLOAD_TOO_LARGE", "limit": what }));
                e
            }
            AppError::Busy { retry_after_secs } => {
                let mut e = Self::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    ErrorCode::RateLimited,
                    "the platform is catching up on earlier events; retry later",
                );
                e.parts.retryable = true;
                e.parts.details = Some(json!({ "reason": "BACKLOG" }));
                e.retry_after = Some(retry_after_secs.max(1));
                e
            }
            AppError::Internal => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                "internal error",
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = self.parts.status;
        if let Some(challenge) = self.www_authenticate {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static(challenge),
            );
        }
        if let Some(seconds) = self.retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        response.extensions_mut().insert(self.parts);
        response
    }
}

/// The stable reason a script was refused, for a client to branch on.
const fn script_reason(error: ScriptError) -> &'static str {
    match error {
        ScriptError::AboveCeiling => "ABOVE_CEILING",
        ScriptError::LanguageNotBuilt => "LANGUAGE_NOT_BUILT",
        ScriptError::InvalidSchema => "SCHEMA",
        ScriptError::InvalidSource | ScriptError::InvalidLockfile => "SOURCE",
        ScriptError::InvalidMemory
        | ScriptError::InvalidCpu
        | ScriptError::InvalidWallTime
        | ScriptError::InvalidProcesses
        | ScriptError::InvalidOutputLimit
        | ScriptError::InvalidScratch
        | ScriptError::InvalidPolicy => "LIMIT",
        ScriptError::InvalidRetention => "RETENTION",
        ScriptError::UnknownLanguage => "LANGUAGE",
        ScriptError::InvalidDigest | ScriptError::InvalidVersion => "IDENTIFIER",
        ScriptError::NotATrustedArtifact => "UNTRUSTED_ARTIFACT",
        ScriptError::BundleMismatch => "BUNDLE_MISMATCH",
    }
}

/// Structured detail for a rejected workflow document: paths and node ids, never document values
/// beyond the clipped fragments the codec already limited.
fn definition_details(error: &IrError) -> Value {
    match error {
        IrError::Schema(issues) => json!({
            "reason": "SCHEMA",
            "issues": issues
                .iter()
                .map(|i| json!({ "path": i.path, "message": i.message }))
                .collect::<Vec<_>>(),
        }),
        IrError::Unsupported { node, kind, .. } => {
            json!({ "reason": "UNSUPPORTED_NODE", "node": node, "kind": kind })
        }
        IrError::Expression { node, field, .. } => {
            json!({ "reason": "EXPRESSION", "node": node, "field": field })
        }
        IrError::NameMismatch { .. } => json!({ "reason": "NAME_MISMATCH" }),
        IrError::UnknownScript { digest } => {
            json!({ "reason": "UNKNOWN_SCRIPT", "artifactDigest": digest })
        }
        IrError::TooLarge => json!({ "reason": "TOO_LARGE" }),
        IrError::Malformed => json!({ "reason": "MALFORMED" }),
        IrError::Invalid(_) => json!({ "reason": "GRAPH" }),
    }
}

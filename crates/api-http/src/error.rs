//! The API's single error envelope (FR-001-006).
//!
//! Handlers return [`ApiError`]; it carries everything except the trace id, which the
//! request-context middleware adds so the id in the body always equals the `X-Trace-Id` header.
//! The middleware also wraps framework-generated errors (unknown route, bad method, oversized
//! body, timeout, panic) in the same envelope, so a client never sees a different error shape.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sagat_application::error::{AppError, ConflictKind};
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
    Unavailable,
    Internal,
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
        Self {
            status,
            code,
            message: message.to_owned(),
            retryable,
            details: None,
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

    /// Dependency down; the client may retry.
    #[must_use]
    pub fn unavailable() -> Self {
        let mut e = Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            "service temporarily unavailable",
        );
        e.parts.retryable = true;
        e.retry_after = Some(5);
        e
    }

    #[must_use]
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.parts.status = status;
        self
    }
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        tracing::debug!(%error, "bearer token refused");
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
            AppError::Conflict(kind) => {
                let message = match kind {
                    ConflictKind::TenantSlug => "a tenant with this slug already exists",
                    ConflictKind::WorkspaceSlug => "a workspace with this slug already exists",
                    ConflictKind::NamespaceSlug => "a namespace with this slug already exists",
                };
                let mut e = Self::new(StatusCode::CONFLICT, ErrorCode::Conflict, message);
                e.parts.details = Some(json!({ "field": "slug" }));
                e
            }
            AppError::Invalid(inner) => Self::validation(inner.to_string()),
            AppError::Unavailable => Self::unavailable(),
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

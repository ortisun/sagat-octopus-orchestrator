//! Error mapping: one classification of failures (the HTTP adapter's [`ApiError`]) onto gRPC
//! status codes, so both transports agree on what an error means.

use sagat_api_http::error::{ApiError, ErrorCode};
use tonic::metadata::MetadataValue;
use tonic::{Code, Status};

/// Trailing metadata carrying the stable machine-readable code (`STALE_ATTEMPT`, ...).
pub const ERROR_METADATA: &str = "x-sagat-error";
/// Trailing metadata with the earliest useful retry delay, in milliseconds.
pub const RETRY_AFTER_METADATA: &str = "retry-after-ms";

/// The gRPC status of an error. The message is the API's safe message: it never carries payloads,
/// tokens or the reason an identity was refused.
#[must_use]
pub fn to_status(error: &ApiError) -> Status {
    let parts = error.parts();
    let code = match parts.status.as_u16() {
        401 => Code::Unauthenticated,
        403 => Code::PermissionDenied,
        404 => Code::NotFound,
        409 => Code::Aborted,
        400 | 413 | 415 | 422 => Code::InvalidArgument,
        408 | 502..=504 => Code::Unavailable,
        429 => Code::ResourceExhausted,
        _ => Code::Internal,
    };
    let mut status = Status::new(code, parts.message.clone());
    let machine = parts
        .details
        .as_ref()
        .and_then(|details| details.get("reason"))
        .and_then(|reason| reason.as_str())
        .map_or_else(|| machine_code(parts.code), str::to_owned);
    if let Ok(value) = MetadataValue::try_from(machine.as_str()) {
        status.metadata_mut().insert(ERROR_METADATA, value);
    }
    if let Some(seconds) = error.retry_after_seconds() {
        status.metadata_mut().insert(
            RETRY_AFTER_METADATA,
            MetadataValue::from(u64::from(seconds) * 1000),
        );
    }
    status
}

fn machine_code(code: ErrorCode) -> String {
    match code {
        ErrorCode::BadRequest => "BAD_REQUEST",
        ErrorCode::Unauthenticated => "UNAUTHENTICATED",
        ErrorCode::Forbidden => "FORBIDDEN",
        ErrorCode::NotFound => "NOT_FOUND",
        ErrorCode::Conflict => "CONFLICT",
        ErrorCode::ValidationFailed => "VALIDATION_FAILED",
        ErrorCode::RateLimited => "RATE_LIMITED",
        ErrorCode::Unavailable => "UNAVAILABLE",
        ErrorCode::Internal => "INTERNAL",
    }
    .to_owned()
}

/// A malformed request field. The message names the field, never its value.
#[must_use]
pub fn invalid(message: &str) -> Status {
    let mut status = Status::invalid_argument(message);
    status.metadata_mut().insert(
        ERROR_METADATA,
        MetadataValue::from_static("INVALID_ARGUMENT"),
    );
    status
}

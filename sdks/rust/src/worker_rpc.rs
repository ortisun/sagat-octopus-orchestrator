//! Authenticated public worker transport; raw status text never becomes an SDK error.
use crate::{SagatError, TokenSource, protocol::worker_service_client::WorkerServiceClient};
use serde_json::json;
use std::time::Duration;
use tonic::{
    Code, Request, Status,
    transport::{Channel, Endpoint},
};

pub(crate) fn error_of(method: &str, status: Status) -> SagatError {
    let raw_reason = status
        .metadata()
        .get("x-sagat-error")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("");
    let reason: String = raw_reason.chars().take(128).collect();
    let code = match status.code() {
        Code::Unauthenticated => "UNAUTHENTICATED",
        Code::PermissionDenied => match reason.as_str() {
            "NOT_BOUND" => "NOT_BOUND",
            "SECRET_NOT_USABLE" => "SECRET_NOT_USABLE",
            _ => "FORBIDDEN",
        },
        Code::InvalidArgument
            if reason == "UNSUPPORTED_PROTOCOL"
                || (method == "RegisterWorker"
                    && status
                        .message()
                        .to_ascii_lowercase()
                        .contains("unsupported protocol")) =>
        {
            "UNSUPPORTED_PROTOCOL"
        }
        Code::InvalidArgument => "VALIDATION_FAILED",
        Code::Aborted => match reason.as_str() {
            "IDEMPOTENCY_KEY_REUSED" => "IDEMPOTENCY_KEY_REUSED",
            "" | "STALE_ATTEMPT" => "STALE_ATTEMPT",
            _ => "CONFLICT",
        },
        Code::FailedPrecondition => "STALE_ATTEMPT",
        Code::NotFound => "NOT_FOUND",
        Code::ResourceExhausted if reason == "QUOTA_EXCEEDED" => "QUOTA_EXCEEDED",
        Code::ResourceExhausted => "RATE_LIMITED",
        Code::Unavailable | Code::DeadlineExceeded if reason == "UNAVAILABLE" => "UNAVAILABLE",
        Code::Unavailable | Code::DeadlineExceeded => "TRANSPORT",
        Code::Cancelled => "CANCELLED",
        Code::Unimplemented if method == "StreamTasks" => "UNSUPPORTED_PROTOCOL",
        Code::OutOfRange
            if matches!(method, "ResolveSecrets" | "ReadBlob" | "WriteBlob")
                && is_bounded_frame_overflow(&status) =>
        {
            "PROTOCOL"
        }
        Code::DataLoss => "PROTOCOL",
        _ => "INTERNAL",
    };
    let mut error = SagatError::new(
        code,
        "the worker service refused or could not complete the call",
    );
    error.details = Box::new(
        json!({"reason":if reason.is_empty(){None}else{Some(reason)},"grpcStatus":status_name(status.code())}),
    );
    error.trace_id = status
        .metadata()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .and_then(crate::error::trace_id);
    if let Some(value) = status
        .metadata()
        .get("retry-after-ms")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
    {
        error.retry_after = Some(Duration::from_millis(
            value.parse::<u64>().unwrap_or(u64::MAX).min(60_000),
        ));
    }
    error
}
// Tonic 0.14.6 exposes its local frame-budget rejection only as this fixed diagnostic, with
// no typed provenance. Match it narrowly for these RPCs; other remote OUT_OF_RANGE statuses keep
// the canonical INTERNAL mapping. No authentication or retry authority is inferred from text.
fn is_bounded_frame_overflow(status: &Status) -> bool {
    if status.metadata().get("x-sagat-error").is_some() {
        return false;
    }
    let Some(rest) = status
        .message()
        .strip_prefix("Error, decoded message length too large: found ")
    else {
        return false;
    };
    let Some((length, limit)) = rest.split_once(" bytes, the limit is: ") else {
        return false;
    };
    limit == "524288 bytes"
        && !length.is_empty()
        && length.bytes().all(|b| b.is_ascii_digit())
        && length.parse::<u64>().is_ok_and(|v| v > 512 * 1024)
}
fn status_name(code: Code) -> &'static str {
    match code {
        Code::Ok => "OK",
        Code::Cancelled => "CANCELLED",
        Code::Unknown => "UNKNOWN",
        Code::InvalidArgument => "INVALID_ARGUMENT",
        Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        Code::NotFound => "NOT_FOUND",
        Code::AlreadyExists => "ALREADY_EXISTS",
        Code::PermissionDenied => "PERMISSION_DENIED",
        Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Code::FailedPrecondition => "FAILED_PRECONDITION",
        Code::Aborted => "ABORTED",
        Code::OutOfRange => "OUT_OF_RANGE",
        Code::Unimplemented => "UNIMPLEMENTED",
        Code::Internal => "INTERNAL",
        Code::Unavailable => "UNAVAILABLE",
        Code::DataLoss => "DATA_LOSS",
        Code::Unauthenticated => "UNAUTHENTICATED",
    }
}
#[derive(Clone)]
pub(crate) struct Rpc {
    pub client: WorkerServiceClient<Channel>,
    token: TokenSource,
    timeout: Duration,
    telemetry: crate::Telemetry,
}
impl Rpc {
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
    pub async fn connect(
        url: &str,
        token: TokenSource,
        timeout: Duration,
        tls: &crate::TlsOptions,
        telemetry: crate::Telemetry,
        suffix: Option<&str>,
    ) -> Result<Self, SagatError> {
        let mut endpoint = Endpoint::from_shared(url.to_owned())
            .map_err(|_| SagatError::new("VALIDATION_FAILED", "invalid worker endpoint"))?
            .connect_timeout(timeout)
            .timeout(timeout)
            .user_agent(suffix.filter(|s| !s.is_empty()).map_or_else(
                || format!("sagat-sdk-rust/{}", crate::VERSION),
                |s| format!("sagat-sdk-rust/{} {s}", crate::VERSION),
            ))
            .map_err(|_| SagatError::new("VALIDATION_FAILED", "invalid user agent"))?;
        if url.starts_with("https://") {
            endpoint = endpoint
                .tls_config(crate::tls::grpc_configuration(tls)?)
                .map_err(|_| {
                    SagatError::new("VALIDATION_FAILED", "invalid worker TLS configuration")
                })?;
        }
        let channel = tokio::time::timeout(timeout, endpoint.connect())
            .await
            .map_err(|_| transport())?
            .map_err(|_| transport())?;
        Ok(Self {
            client: WorkerServiceClient::new(channel).max_decoding_message_size(16 * 1024 * 1024),
            token,
            timeout,
            telemetry,
        })
    }
    pub async fn request<T>(&self, value: T, traceparent: &str) -> Result<Request<T>, SagatError> {
        self.request_with(|| Ok(value), traceparent).await
    }
    pub async fn request_with<T, F>(
        &self,
        make: F,
        traceparent: &str,
    ) -> Result<Request<T>, SagatError>
    where
        F: FnOnce() -> Result<T, SagatError>,
    {
        let token = tokio::time::timeout(self.timeout, self.token.get())
            .await
            .map_err(|_| SagatError::provider_failure())??;
        let mut request = Request::new(make()?);
        let mut bearer =
            tonic::metadata::MetadataValue::try_from(format!("Bearer {}", token.as_str()))
                .map_err(|_| SagatError::provider_failure())?;
        bearer.set_sensitive(true);
        request.metadata_mut().insert("authorization", bearer);
        let traceparent = self
            .telemetry
            .trace(crate::client::valid_traceparent(traceparent).then(|| traceparent.to_owned()));
        if let Some(traceparent) = traceparent {
            request.metadata_mut().insert(
                "traceparent",
                traceparent
                    .parse()
                    .map_err(|_| SagatError::new("PROTOCOL", "invalid lease trace context"))?,
            );
        }
        request.set_timeout(self.timeout);
        Ok(request)
    }
}
fn transport() -> SagatError {
    SagatError::new("TRANSPORT", "no answer from the worker service")
}
#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    #[test]
    fn grpc_errors_preserve_closed_codes_and_bounded_metadata_without_raw_messages() {
        for (status, reason, expected) in [
            (tonic::Code::Aborted, "STALE_ATTEMPT", "STALE_ATTEMPT"),
            (
                tonic::Code::Aborted,
                "IDEMPOTENCY_KEY_REUSED",
                "IDEMPOTENCY_KEY_REUSED",
            ),
            (tonic::Code::PermissionDenied, "NOT_BOUND", "NOT_BOUND"),
            (
                tonic::Code::PermissionDenied,
                "SECRET_NOT_USABLE",
                "SECRET_NOT_USABLE",
            ),
            (
                tonic::Code::InvalidArgument,
                "UNSUPPORTED_PROTOCOL",
                "UNSUPPORTED_PROTOCOL",
            ),
            (
                tonic::Code::ResourceExhausted,
                "QUOTA_EXCEEDED",
                "QUOTA_EXCEEDED",
            ),
            (tonic::Code::Unavailable, "UNAVAILABLE", "UNAVAILABLE"),
            (tonic::Code::Unavailable, "", "TRANSPORT"),
            (tonic::Code::FailedPrecondition, "", "STALE_ATTEMPT"),
            (tonic::Code::DataLoss, "", "PROTOCOL"),
        ] {
            let mut failure = tonic::Status::new(status, "private-value-from-provider");
            failure
                .metadata_mut()
                .insert("x-sagat-error", reason.parse().expect("test metadata"));
            failure.metadata_mut().insert(
                "retry-after-ms",
                "99999999999999999999999999999999999999"
                    .parse()
                    .expect("test metadata"),
            );
            failure.metadata_mut().insert(
                "x-trace-id",
                "0123456789abcdef0123456789abcdef"
                    .parse()
                    .expect("test metadata"),
            );
            let error = error_of("Heartbeat", failure);
            assert_eq!(error.code, expected);
            assert!(!format!("{error:?} {error}").contains("private-value"));
            assert_eq!(error.retry_after, Some(std::time::Duration::from_secs(60)));
            assert_eq!(
                error.trace_id.as_deref(),
                Some("0123456789abcdef0123456789abcdef")
            );
        }
    }
    #[test]
    fn stream_fallback_cannot_be_enabled_by_a_permission_failure() {
        let unsupported = error_of("StreamTasks", tonic::Status::unimplemented("hidden"));
        let forbidden = error_of("StreamTasks", tonic::Status::permission_denied("hidden"));
        assert_eq!(unsupported.code, "UNSUPPORTED_PROTOCOL");
        assert_eq!(forbidden.code, "FORBIDDEN");
        assert!(!forbidden.may_retry());
    }
}

#[cfg(test)]
mod deferred_request_tests {
    #![allow(clippy::expect_used)]
    use super::*;
    #[tokio::test]
    async fn request_materialization_does_not_run_while_token_authentication_is_pending() {
        let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        let token =
            TokenSource::dynamic(std::future::pending::<Result<String, crate::TokenProviderError>>);
        let rpc = Rpc {
            client: WorkerServiceClient::new(channel),
            token,
            timeout: Duration::from_secs(30),
            telemetry: crate::Telemetry::default(),
        };
        let constructed = std::sync::atomic::AtomicBool::new(false);
        let result = tokio::time::timeout(
            Duration::from_millis(1),
            rpc.request_with(
                || {
                    constructed.store(true, std::sync::atomic::Ordering::Release);
                    Ok(())
                },
                "",
            ),
        )
        .await;
        assert!(result.is_err());
        assert!(!constructed.load(std::sync::atomic::Ordering::Acquire));
    }
}

#[cfg(test)]
mod secret_decode_errors {
    use super::*;
    #[test]
    fn a_known_local_secret_decode_overflow_is_protocol_without_reclassifying_server_statuses() {
        let diagnostic = "Error, decoded message length too large: found 614425 bytes, the limit is: 524288 bytes";
        let local = error_of("ResolveSecrets", Status::out_of_range(diagnostic));
        assert_eq!(local.code, "PROTOCOL");
        assert!(!local.retryable);
        let remote = error_of(
            "ResolveSecrets",
            Status::out_of_range("a server field is out of range"),
        );
        assert_eq!(remote.code, "INTERNAL");
        let other = error_of("PollTasks", Status::out_of_range(diagnostic));
        assert_eq!(other.code, "INTERNAL");
    }
}

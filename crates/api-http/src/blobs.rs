//! Streaming run-input uploads. Staging never grants a worker permission to read the bytes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use sagat_application::blobs::{BlobIoError, BlobWriteError, WriteRequest};
use sagat_application::error::AppError;
use sagat_domain::blobs::ContentType;
use sagat_domain::workflow::Digest;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio_stream::StreamExt;

use crate::admission::admit_resolved_namespace;
use crate::error::ApiError;
use crate::exec::Exec;
use crate::extract::Caller;

/// Per-process upload concurrency and total duration, in addition to the HTTP tenant budgets.
#[derive(Debug, Clone, Copy)]
pub struct HttpBlobTransfer {
    pub max_concurrent: usize,
    pub timeout: Duration,
}

impl Default for HttpBlobTransfer {
    fn default() -> Self {
        Self {
            max_concurrent: 32,
            timeout: Duration::from_secs(600),
        }
    }
}

#[derive(Debug)]
pub struct BlobTransfers {
    pub(crate) limits: HttpBlobTransfer,
    slots: Arc<Semaphore>,
}

impl BlobTransfers {
    pub(crate) fn new(limits: HttpBlobTransfer) -> Arc<Self> {
        Arc::new(Self {
            slots: Arc::new(Semaphore::new(limits.max_concurrent.max(1))),
            limits,
        })
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &'static str) -> Result<Option<&'a str>, ApiError> {
    let mut values = headers.get_all(name).iter();
    match (values.next(), values.next()) {
        (None, None) => Ok(None),
        (Some(value), None) => value
            .to_str()
            .map(Some)
            .map_err(|_| ApiError::bad_request("invalid blob upload header")),
        _ => Err(ApiError::bad_request("duplicate blob upload header")),
    }
}

fn caused_by<E: std::error::Error + 'static>(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut source = Some(error);
    while let Some(error) = source {
        if error.is::<E>() {
            return true;
        }
        source = error.source();
    }
    false
}

pub(crate) async fn upload<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let namespace = header(&headers, "x-octopus-namespace")?
        .filter(|v| !v.is_empty() && v.len() <= 63)
        .ok_or_else(|| ApiError::bad_request("the X-Octopus-Namespace header is required"))?;
    let content_type = header(&headers, "content-type")?
        .map(ContentType::parse)
        .transpose()
        .map_err(|_| ApiError::validation("invalid blob content type"))?
        .unwrap_or_else(ContentType::octet_stream);
    let expected_size = header(&headers, "x-octopus-blob-size")?
        .map(|text| {
            if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ApiError::validation("invalid expected blob size"));
            }
            text.parse::<u64>()
                .map_err(|_| ApiError::validation("invalid expected blob size"))
        })
        .transpose()?;
    let expected_digest = header(&headers, "x-octopus-blob-sha256")?
        .map(|text| {
            Digest::from_hex(text)
                .ok_or_else(|| ApiError::validation("invalid expected blob digest"))
        })
        .transpose()?;
    let blobs = state
        .runs()
        .input_blobs()
        .ok_or_else(ApiError::unavailable)?;
    let access = state
        .runs()
        .input_blob_access(&caller.session.principal, namespace)
        .await?;
    admit_resolved_namespace(&state, &caller, access.namespace_id).await?;
    let _slot = state
        .blob_transfers()
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::unavailable_after(1))?;
    let expires = Duration::from_secs(caller.token_expires_at.saturating_sub(
        u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp()).unwrap_or(0),
    ));
    let timeout = state.blob_transfers().limits.timeout.min(expires);
    let body_timed_out = Arc::new(AtomicBool::new(false));
    let mark_timeout = body_timed_out.clone();
    let stream = body.into_data_stream().map(move |chunk| {
        chunk.map_err(|error| {
            if caused_by::<http_body_util::LengthLimitError>(&error) {
                BlobIoError::TooLarge
            } else {
                if caused_by::<tower_http::timeout::TimeoutError>(&error) {
                    mark_timeout.store(true, Ordering::Relaxed);
                }
                BlobIoError::Interrupted
            }
        })
    });
    // Cancellation can leave an uncertain provider PUT; the reserved catalog tombstone makes
    // that durable cleanup work. Neither a tenant nor a transfer slot outlives this deadline.
    let descriptor = tokio::time::timeout(
        timeout,
        blobs.write_input(
            access,
            caller.session.principal.subject().as_str(),
            WriteRequest {
                content_type,
                expected_size,
                expected_digest,
            },
            Box::pin(stream),
        ),
    )
    .await
    .map_err(|_| ApiError::request_timeout())?
    .map_err(|error| match error {
        BlobWriteError::TooLarge => ApiError::from(AppError::PayloadTooLarge("blob")),
        BlobWriteError::QuotaExceeded => ApiError::from(AppError::QuotaExceeded("pending blobs")),
        BlobWriteError::SizeMismatch => ApiError::validation("blob size does not match"),
        BlobWriteError::DigestMismatch => ApiError::validation("blob digest does not match"),
        BlobWriteError::Interrupted if body_timed_out.load(Ordering::Relaxed) => {
            ApiError::request_timeout()
        }
        BlobWriteError::Interrupted => ApiError::bad_request("blob upload was interrupted"),
        BlobWriteError::Unavailable | BlobWriteError::Internal => ApiError::unavailable(),
    })?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": descriptor.id().to_string(), "size": descriptor.size(),
            "sha256": descriptor.digest().to_hex(), "contentType": descriptor.content_type().as_str(),
        })),
    ))
}

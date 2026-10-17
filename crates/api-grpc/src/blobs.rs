//! Blobs over the worker protocol (FR-002-008, ADR-0023): the streams, the limits on transfers and
//! the way a blob failure reaches a worker.
//!
//! Ordinary uploads stream to storage while the use case counts and hashes bytes. Managed database
//! results have a separate hard bound (4 MiB maximum) and are collected for pre-storage credential
//! scanning. Both paths hold a transfer permit and enforce a finite deadline.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use sagat_api_http::error::ApiError;
use sagat_application::blobs::{
    BlobIoError, BlobReadError, BlobService, BlobWriteError, ByteStream, WriteRequest,
};
use sagat_application::error::AppError;
use sagat_domain::blobs::ContentType;
use sagat_domain::workflow::Digest;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, Sleep};
use tokio_stream::{Stream, StreamExt};
use tonic::metadata::MetadataValue;
use tonic::{Status, Streaming};
use tracing::Instrument;

use crate::pb;
use crate::status::{ERROR_METADATA, to_status};

/// Human-readable recovery guidance for integrity failures. This is deliberately local: the
/// shared status mapper only owns metadata emitted by every API error.
const REMEDIATION_METADATA: &str = "x-sagat-remediation";

/// Largest chunk either side sends. It fits the message size the server accepts, so an upload needs
/// no special case in the transport.
pub const BLOB_CHUNK_BYTES: usize = 256 * 1024;

/// How much blob traffic one process carries.
#[derive(Debug, Clone, Copy)]
pub struct BlobTransfer {
    /// Uploads and downloads running at once. The next one is told to retry shortly.
    pub max_concurrent: usize,
    /// The longest one transfer may take, from the first message to the last.
    pub timeout: Duration,
}

impl Default for BlobTransfer {
    fn default() -> Self {
        Self {
            max_concurrent: 32,
            timeout: Duration::from_secs(600),
        }
    }
}

/// The blob use cases together with the limits on moving them.
#[derive(Clone)]
pub(crate) struct BlobAccess {
    pub(crate) service: BlobService,
    transfers: Arc<Semaphore>,
    pub(crate) timeout: Duration,
}

impl BlobAccess {
    pub(crate) fn new(service: BlobService, transfer: BlobTransfer) -> Self {
        Self {
            service,
            transfers: Arc::new(Semaphore::new(transfer.max_concurrent.max(1))),
            timeout: transfer.timeout,
        }
    }

    /// A place among the transfers, or the refusal that tells the worker to come back in a moment.
    pub(crate) fn permit(&self) -> Result<OwnedSemaphorePermit, Status> {
        Arc::clone(&self.transfers)
            .try_acquire_owned()
            .map_err(|_| to_status(&ApiError::rate_limited(Duration::from_secs(1))))
    }
}

// ---------------------------------------------------------------------------------- uploading

/// The header of an upload turned into what the use case needs.
pub(crate) fn write_request(header: &pb::WriteBlobHeader) -> Result<WriteRequest, Status> {
    let content_type = if header.content_type.is_empty() {
        ContentType::octet_stream()
    } else {
        ContentType::parse(&header.content_type)
            .map_err(|_| invalid_upload("content_type is not a media type"))?
    };
    let expected_size = match header.expected_size {
        0 => None,
        n => Some(u64::try_from(n).map_err(|_| invalid_upload("expected_size is negative"))?),
    };
    let expected_digest = if header.expected_sha256.is_empty() {
        None
    } else {
        Some(
            Digest::from_hex(&header.expected_sha256.to_ascii_lowercase())
                .ok_or_else(|| invalid_upload("expected_sha256 is not 64 hex digits"))?,
        )
    };
    Ok(WriteRequest {
        content_type,
        expected_size,
        expected_digest,
    })
}

/// What went wrong with an upload that the use case can only report as "interrupted".
#[derive(Debug, Default)]
pub(crate) struct UploadFlags {
    /// The client sent something that is not a chunk of the agreed size.
    violation: AtomicBool,
    /// The time for the whole transfer ran out.
    timed_out: AtomicBool,
}

impl UploadFlags {
    pub(crate) fn violation(&self) -> bool {
        self.violation.load(Ordering::Acquire)
    }

    pub(crate) fn timed_out(&self) -> bool {
        self.timed_out.load(Ordering::Acquire)
    }
}

/// The chunks that follow the header of an upload, as a body.
///
/// The time limit is a property of the body, not a timeout around the call: the call is then ended
/// by an error like any other failed upload, and the use case removes what it had begun. A call that
/// is simply dropped would leave the reservation and the half-written object to the sweep.
struct UploadBody {
    inner: Streaming<pb::WriteBlobRequest>,
    flags: Arc<UploadFlags>,
    deadline: Pin<Box<Sleep>>,
}

impl Stream for UploadBody {
    type Item = Result<Bytes, BlobIoError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.deadline.as_mut().poll(cx).is_ready() {
            self.flags.timed_out.store(true, Ordering::Release);
            return Poll::Ready(Some(Err(BlobIoError::Interrupted)));
        }
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Err(_))) => Poll::Ready(Some(Err(BlobIoError::Interrupted))),
            Poll::Ready(Some(Ok(message))) => match message.part {
                Some(pb::write_blob_request::Part::Chunk(chunk))
                    if chunk.len() <= BLOB_CHUNK_BYTES =>
                {
                    Poll::Ready(Some(Ok(chunk)))
                }
                // A second header, an empty message or a chunk over the agreed size.
                _ => {
                    self.flags.violation.store(true, Ordering::Release);
                    Poll::Ready(Some(Err(BlobIoError::Interrupted)))
                }
            },
        }
    }
}

/// The rest of an upload as a body, bounded in time, and the flags that say why it ended early.
pub(crate) fn upload_body(
    stream: Streaming<pb::WriteBlobRequest>,
    timeout: Duration,
) -> (ByteStream, Arc<UploadFlags>) {
    let flags = Arc::new(UploadFlags::default());
    let body = UploadBody {
        inner: stream,
        flags: Arc::clone(&flags),
        deadline: Box::pin(tokio::time::sleep_until(Instant::now() + timeout)),
    };
    (Box::pin(body), flags)
}

/// Bounded exception for managed results: no unscanned prefix can enter object storage.
/// Reading through EOF is mandatory so the blob service's digest verifier can reject corruption.
pub(crate) async fn collect_database_result(
    mut body: ByteStream,
    limit: usize,
    timeout: Duration,
) -> Result<zeroize::Zeroizing<Vec<u8>>, Status> {
    if limit == 0 || limit > sagat_domain::database::DatabaseLimits::MAX.max_result_bytes as usize {
        return Err(invalid_upload("invalid managed result limit"));
    }
    tokio::time::timeout(timeout, async {
        let mut content = zeroize::Zeroizing::new(Vec::new());
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|error| match error {
                BlobIoError::Corrupt => data_loss(),
                BlobIoError::TooLarge => invalid_upload("managed database result is too large"),
                BlobIoError::Interrupted => {
                    invalid_upload("managed result transfer was interrupted")
                }
            })?;
            if chunk.len() > limit.saturating_sub(content.len()) {
                return Err(invalid_upload("managed database result is too large"));
            }
            content.extend_from_slice(&chunk);
        }
        Ok(content)
    })
    .await
    .map_err(|_| to_status(&ApiError::request_timeout()))?
}

// -------------------------------------------------------------------------------- downloading

/// A download: the header, then the store's stream in chunks of at most [`BLOB_CHUNK_BYTES`]. It
/// holds its place among the transfers until it is dropped, ends with an error rather than silently
/// when the time is up, and never ends cleanly if the content failed verification.
pub(crate) struct Download {
    receiver: tokio::sync::mpsc::Receiver<Result<pb::ReadBlobResponse, Status>>,
    deadline: Pin<Box<Sleep>>,
    producer: tokio::task::AbortHandle,
    verified: Arc<AtomicBool>,
    done: bool,
}

impl Download {
    pub(crate) fn new(
        header: pb::ReadBlobHeader,
        mut body: ByteStream,
        permit: OwnedSemaphorePermit,
        auth: sagat_api_http::Authenticated,
        timeout: Duration,
    ) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let deadline = Instant::now() + timeout;
        let verified = Arc::new(AtomicBool::new(false));
        let completed = Arc::clone(&verified);
        let span = tracing::Span::current();
        // The producer owns both admission guards. Its deadline is driven by the runtime even if
        // HTTP/2 backpressure prevents the response body from ever being polled again.
        let producer = tokio::spawn(async move {
            let started = Instant::now();
            let _permit = permit;
            let _auth = auth;
            let transfer = async {
                if sender.send(Ok(pb::ReadBlobResponse {
                    part: Some(pb::read_blob_response::Part::Header(header)),
                })).await.is_err() { return; }
                while let Some(next) = body.next().await {
                    let mut bytes = match next {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            let status = match error {
                                BlobIoError::Corrupt => data_loss(),
                                BlobIoError::Interrupted | BlobIoError::TooLarge =>
                                    to_status(&ApiError::from(AppError::Unavailable)),
                            };
                            let _ = sender.send(Err(status)).await;
                            return;
                        }
                    };
                    while !bytes.is_empty() {
                        let chunk = bytes.split_to(bytes.len().min(BLOB_CHUNK_BYTES));
                        let length = chunk.len();
                        if sender.send(Ok(pb::ReadBlobResponse {
                            part: Some(pb::read_blob_response::Part::Chunk(chunk)),
                        })).await.is_err() { return; }
                        metrics::counter!("blob_bytes_total", "direction" => "read")
                            .increment(length as u64);
                    }
                }
                completed.store(true, Ordering::Release);
            };
            let result = tokio::time::timeout_at(deadline, transfer).await;
            let outcome = if result.is_err() { "timeout" } else if completed.load(Ordering::Acquire) { "completed" } else { "interrupted" };
            metrics::counter!("blob_transfers_total", "direction" => "read", "outcome" => outcome).increment(1);
            metrics::histogram!("blob_transfer_duration_seconds", "direction" => "read", "outcome" => outcome).record(started.elapsed().as_secs_f64());
        }.instrument(span));
        Self {
            receiver,
            deadline: Box::pin(tokio::time::sleep_until(deadline)),
            producer: producer.abort_handle(),
            verified,
            done: false,
        }
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        self.producer.abort();
    }
}

impl Stream for Download {
    type Item = Result<pb::ReadBlobResponse, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        if self.deadline.as_mut().poll(cx).is_ready() {
            self.done = true;
            self.producer.abort();
            return Poll::Ready(Some(Err(to_status(&ApiError::request_timeout()))));
        }
        match self.receiver.poll_recv(cx) {
            Poll::Ready(None) => {
                self.done = true;
                if self.verified.load(Ordering::Acquire) {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Err(to_status(&ApiError::from(AppError::Unavailable)))))
                }
            }
            Poll::Ready(Some(Err(error))) => {
                self.done = true;
                Poll::Ready(Some(Err(error)))
            }
            result => result,
        }
    }
}

fn data_loss() -> Status {
    metrics::counter!("blob_corrupt_total").increment(1);
    let mut status = Status::data_loss("the stored content failed its integrity check");
    status
        .metadata_mut()
        .insert(ERROR_METADATA, MetadataValue::from_static("BLOB_CORRUPT"));
    if let Ok(text) =
        MetadataValue::try_from("Do not use what was read; report the trace id to the operator.")
    {
        status.metadata_mut().insert(REMEDIATION_METADATA, text);
    }
    status
}

pub(crate) fn write_failure(error: BlobWriteError) -> Status {
    let app = match error {
        BlobWriteError::TooLarge => AppError::NotAllowed("the blob is larger than the maximum"),
        BlobWriteError::SizeMismatch | BlobWriteError::DigestMismatch => {
            AppError::NotAllowed("the blob does not match the size or digest that was stated")
        }
        BlobWriteError::QuotaExceeded => AppError::QuotaExceeded("uploaded blobs not yet used"),
        BlobWriteError::Interrupted => AppError::NotAllowed("the upload was interrupted"),
        BlobWriteError::Unavailable => AppError::Unavailable,
        BlobWriteError::Internal => AppError::Internal,
    };
    upload_guidance(to_status(&ApiError::from(app)))
}

fn invalid_upload(message: &'static str) -> Status {
    upload_guidance(crate::status::invalid(message))
}

fn upload_guidance(mut status: Status) -> Status {
    status.metadata_mut().insert(
        REMEDIATION_METADATA,
        MetadataValue::from_static(
            "Check the upload limits, content type, size and SHA-256; retry the complete upload.",
        ),
    );
    status
}

pub(crate) fn read_failure(error: BlobReadError) -> Status {
    let app = match error {
        BlobReadError::NotFound => AppError::NotFound,
        BlobReadError::Unavailable => AppError::Unavailable,
        BlobReadError::Lost => AppError::Internal,
    };
    to_status(&ApiError::from(app))
}

/// The refusal of a blob call on an instance that has no blob store configured.
pub(crate) fn not_configured() -> Status {
    to_status(&ApiError::from(AppError::NotAllowed(
        "blobs are not configured on this instance",
    )))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn managed_collection_requires_bounded_verified_eof_before_returning_bytes() {
        let parts = [Ok(Bytes::from_static(b"ab")), Ok(Bytes::from_static(b"cd"))];
        let content = collect_database_result(
            Box::pin(tokio_stream::iter(parts)),
            4,
            Duration::from_secs(1),
        )
        .await
        .expect("exact bound");
        assert_eq!(content.as_slice(), b"abcd");
        let parts = [
            Ok(Bytes::from_static(b"ab")),
            Ok(Bytes::from_static(b"cde")),
        ];
        assert!(
            collect_database_result(
                Box::pin(tokio_stream::iter(parts)),
                4,
                Duration::from_secs(1)
            )
            .await
            .is_err()
        );
        let parts = [Ok(Bytes::from_static(b"abcd")), Err(BlobIoError::Corrupt)];
        assert_eq!(
            collect_database_result(
                Box::pin(tokio_stream::iter(parts)),
                4,
                Duration::from_secs(1)
            )
            .await
            .expect_err("must consume digest verdict")
            .code(),
            tonic::Code::DataLoss
        );
        assert_eq!(
            collect_database_result(
                Box::pin(tokio_stream::pending()),
                4,
                Duration::from_millis(1)
            )
            .await
            .expect_err("stalled producer")
            .code(),
            tonic::Code::Unavailable
        );
    }

    #[tokio::test]
    async fn a_panicked_download_producer_cannot_look_like_verified_eof() {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let producer = tokio::spawn(async move {
            let _sender = sender;
            panic!("injected backend stream panic");
        });
        let abort = producer.abort_handle();
        assert!(producer.await.expect_err("producer panic").is_panic());
        let mut stream = Download {
            receiver,
            deadline: Box::pin(tokio::time::sleep(Duration::from_secs(60))),
            producer: abort,
            verified: Arc::new(AtomicBool::new(false)),
            done: false,
        };
        let status = stream
            .next()
            .await
            .expect("typed failure")
            .expect_err("not successful EOF");
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(stream.next().await.is_none());
    }
}

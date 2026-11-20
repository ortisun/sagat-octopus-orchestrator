use super::*;
use crate::{BlobDescriptor, BlobOutput, WriteBlobOptions};
use futures_util::Stream;
use sha2::{Digest, Sha256};
use tokio::io::AsyncRead;
#[derive(Default)]
struct Measurement {
    size: u64,
    digest: Sha256,
    eof: bool,
    failed: bool,
}
impl Capabilities {
    pub async fn write_blob<R: AsyncRead + Unpin + Send + 'static>(
        &self,
        source: R,
        options: WriteBlobOptions,
    ) -> Result<BlobOutput, SagatError> {
        let source: std::pin::Pin<Box<dyn AsyncRead + Send>> = Box::pin(source);
        self.check()?;
        if self.minor < 3 {
            return Err(SagatError::new(
                "UNSUPPORTED_PROTOCOL",
                "blobs require worker protocol 1.3",
            ));
        }
        if options.expected_size.is_some_and(|v| v > i64::MAX as u64)
            || options
                .expected_sha256
                .as_deref()
                .is_some_and(|v| !crate::types::valid_digest(v))
            || options.content_type.is_empty()
            || options.content_type.len() > 1024
            || options.content_type.bytes().any(|b| b.is_ascii_control())
        {
            return Err(SagatError::new(
                "VALIDATION_FAILED",
                "invalid blob upload options",
            ));
        }
        let auth = self
            .authorization
            .lock()
            .map_err(|_| ended())?
            .as_ref()
            .cloned()
            .ok_or_else(ended)?;
        let deadline = tokio::time::Instant::now() + auth.rpc.timeout();
        let measured = Arc::new(Mutex::new(Measurement::default()));
        let observe = measured.clone();
        let expected_size = options.expected_size;
        let expected_sha = options.expected_sha256.clone();
        let envelope = tokio::select! {biased;_=self.ended.cancelled()=>return Err(ended()),result=tokio::time::timeout_at(deadline,auth.rpc.request((),&auth.request.traceparent))=>result.map_err(|_|SagatError::provider_failure())??};
        self.check()?;
        let operation: futures_util::future::BoxFuture<
            '_,
            Result<pb::WriteBlobResponse, SagatError>,
        > = Box::pin(async move {
            let stream = {
                let scope = &auth.request;
                let header = pb::WriteBlobHeader {
                    task_id: scope.task_id.clone(),
                    run_id: scope.run_id.clone(),
                    step_id: scope.step_id.clone(),
                    attempt: scope.attempt,
                    generation: scope.generation,
                    lease_token: scope.lease_token.clone(),
                    content_type: options.content_type,
                    expected_size: options.expected_size.unwrap_or(0) as i64,
                    expected_sha256: options.expected_sha256.unwrap_or_default(),
                    instance: scope.instance.clone(),
                };
                UploadStream {
                    header: Some(header),
                    source: tokio_util::io::ReaderStream::with_capacity(source, 256 * 1024),
                    measured: observe,
                }
            };
            let (metadata, extensions, ()) = envelope.into_parts();
            let request = tonic::Request::from_parts(metadata, extensions, stream);
            let mut client = auth
                .rpc
                .client
                .clone()
                .max_decoding_message_size(512 * 1024);
            client
                .write_blob(request)
                .await
                .map(|r| r.into_inner())
                .map_err(|e| error_of("WriteBlob", e))
        });
        let reply = tokio::select! {biased;_=self.ended.cancelled()=>return Err(ended()),result=tokio::time::timeout_at(deadline,operation)=>result.map_err(|_|SagatError::new("TRANSPORT","the blob upload timed out"))??};
        self.check()?;
        let state = measured.lock().map_err(|_| invalid())?;
        if state.failed {
            return Err(SagatError::new(
                "TRANSPORT",
                "the blob upload source failed",
            ));
        }
        let digest = format!("{:x}", state.digest.clone().finalize());
        if !state.eof
            || reply.size < 0
            || reply.size as u64 != state.size
            || reply.sha256 != digest
            || expected_size.is_some_and(|v| v != state.size)
            || expected_sha.as_deref().is_some_and(|v| v != digest)
        {
            return Err(invalid());
        }
        let id = uuid::Uuid::parse_str(&reply.blob_id).map_err(|_| invalid())?;
        let descriptor =
            BlobDescriptor::new(id, reply.size as u64, reply.sha256, reply.content_type)
                .map_err(|_| invalid())?;
        Ok(BlobOutput(descriptor))
    }
}
fn invalid() -> SagatError {
    SagatError::new(
        "PROTOCOL",
        "the blob response does not match the transferred bytes",
    )
}

struct UploadStream {
    header: Option<pb::WriteBlobHeader>,
    source: tokio_util::io::ReaderStream<std::pin::Pin<Box<dyn AsyncRead + Send>>>,
    measured: Arc<Mutex<Measurement>>,
}
impl Stream for UploadStream {
    type Item = pb::WriteBlobRequest;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;
        let this = self.get_mut();
        if let Some(header) = this.header.take() {
            return Poll::Ready(Some(pb::WriteBlobRequest {
                part: Some(pb::write_blob_request::Part::Header(header)),
            }));
        }
        match std::pin::Pin::new(&mut this.source).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(bytes))) => {
                let valid = if let Ok(mut state) = this.measured.lock() {
                    if let Some(size) = state
                        .size
                        .checked_add(bytes.len() as u64)
                        .filter(|v| *v <= i64::MAX as u64)
                    {
                        state.size = size;
                        state.digest.update(&bytes);
                        true
                    } else {
                        state.failed = true;
                        false
                    }
                } else {
                    false
                };
                Poll::Ready(valid.then(|| pb::WriteBlobRequest {
                    part: Some(pb::write_blob_request::Part::Chunk(bytes.to_vec())),
                }))
            }
            Poll::Ready(Some(Err(_))) => {
                if let Ok(mut state) = this.measured.lock() {
                    state.failed = true;
                }
                Poll::Ready(None)
            }
            Poll::Ready(None) => {
                if let Ok(mut state) = this.measured.lock() {
                    state.eof = true;
                }
                Poll::Ready(None)
            }
        }
    }
}

impl Drop for UploadStream {
    fn drop(&mut self) {
        if let Some(header) = &mut self.header {
            header.lease_token.zeroize();
        }
    }
}

pub struct BlobReader {
    stream: tonic::Streaming<pb::ReadBlobResponse>,
    expected: BlobDescriptor,
    digest: Sha256,
    size: u64,
    ended: futures_util::future::BoxFuture<'static, ()>,
    timeout: std::pin::Pin<Box<tokio::time::Sleep>>,
    done: bool,
    verified: bool,
}
impl std::fmt::Debug for BlobReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlobReader")
            .field("verified", &self.verified)
            .finish_non_exhaustive()
    }
}
impl BlobReader {
    pub fn is_verified(&self) -> bool {
        self.verified
    }
}
impl Stream for BlobReader {
    type Item = Result<bytes::Bytes, SagatError>;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::{future::Future as _, task::Poll};
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        if this.ended.as_mut().poll(cx).is_ready() {
            this.done = true;
            return Poll::Ready(Some(Err(ended())));
        }
        if this.timeout.as_mut().poll(cx).is_ready() {
            this.done = true;
            return Poll::Ready(Some(Err(SagatError::new(
                "TRANSPORT",
                "the blob read timed out",
            ))));
        }
        match std::pin::Pin::new(&mut this.stream).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Err(error))) => {
                this.done = true;
                Poll::Ready(Some(Err(error_of("ReadBlob", error))))
            }
            Poll::Ready(Some(Ok(frame))) => {
                let Some(pb::read_blob_response::Part::Chunk(bytes)) = frame.part else {
                    this.done = true;
                    return Poll::Ready(Some(Err(invalid())));
                };
                let Some(size) = this
                    .size
                    .checked_add(bytes.len() as u64)
                    .filter(|v| *v <= this.expected.size)
                else {
                    this.done = true;
                    return Poll::Ready(Some(Err(invalid())));
                };
                if bytes.len() > 256 * 1024 {
                    this.done = true;
                    return Poll::Ready(Some(Err(invalid())));
                }
                this.size = size;
                this.digest.update(&bytes);
                Poll::Ready(Some(Ok(bytes::Bytes::from(bytes))))
            }
            Poll::Ready(None) => {
                this.done = true;
                if this.size != this.expected.size
                    || format!("{:x}", this.digest.clone().finalize()) != this.expected.sha256
                {
                    return Poll::Ready(Some(Err(invalid())));
                }
                this.verified = true;
                Poll::Ready(None)
            }
        }
    }
}
impl Capabilities {
    pub async fn read_blob(&self, descriptor: &BlobDescriptor) -> Result<BlobReader, SagatError> {
        self.check()?;
        descriptor.check()?;
        if self.minor < 3 {
            return Err(SagatError::new(
                "UNSUPPORTED_PROTOCOL",
                "blobs require worker protocol 1.3",
            ));
        }
        let auth = self
            .authorization
            .lock()
            .map_err(|_| ended())?
            .as_ref()
            .cloned()
            .ok_or_else(ended)?;
        let deadline = tokio::time::Instant::now() + auth.rpc.timeout();
        let envelope = tokio::select! {biased;_=self.ended.cancelled()=>return Err(ended()),result=tokio::time::timeout_at(deadline,auth.rpc.request((),&auth.request.traceparent))=>result.map_err(|_|SagatError::provider_failure())??};
        self.check()?;
        let scope = &auth.request;
        let message = pb::ReadBlobRequest {
            task_id: scope.task_id.clone(),
            run_id: scope.run_id.clone(),
            step_id: scope.step_id.clone(),
            attempt: scope.attempt,
            generation: scope.generation,
            lease_token: scope.lease_token.clone(),
            blob_id: descriptor.id.to_string(),
            instance: scope.instance.clone(),
        };
        let (metadata, extensions, ()) = envelope.into_parts();
        let request = tonic::Request::from_parts(metadata, extensions, message);
        let operation = async {
            let mut client = auth
                .rpc
                .client
                .clone()
                .max_decoding_message_size(512 * 1024);
            let mut stream = client
                .read_blob(request)
                .await
                .map_err(|e| error_of("ReadBlob", e))?
                .into_inner();
            let first = stream
                .message()
                .await
                .map_err(|e| error_of("ReadBlob", e))?
                .ok_or_else(invalid)?;
            let Some(pb::read_blob_response::Part::Header(header)) = first.part else {
                return Err(invalid());
            };
            if uuid::Uuid::parse_str(&header.blob_id).ok() != Some(descriptor.id)
                || header.size < 0
                || header.size as u64 != descriptor.size
                || header.sha256 != descriptor.sha256
                || header.content_type != descriptor.content_type
            {
                return Err(invalid());
            }
            Ok(stream)
        };
        let stream = tokio::select! {biased;_=self.ended.cancelled()=>return Err(ended()),result=tokio::time::timeout_at(deadline,operation)=>result.map_err(|_|SagatError::new("TRANSPORT","the blob read timed out"))??};
        self.check()?;
        Ok(BlobReader {
            stream,
            expected: descriptor.clone(),
            digest: Sha256::new(),
            size: 0,
            ended: Box::pin(self.ended.clone().cancelled_owned()),
            timeout: Box::pin(tokio::time::sleep_until(deadline)),
            done: false,
            verified: false,
        })
    }
}

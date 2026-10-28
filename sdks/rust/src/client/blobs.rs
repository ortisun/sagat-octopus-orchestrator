use super::*;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncRead;
#[derive(Clone, Debug)]
pub struct UploadRunInputOptions {
    pub call: CallOptions,
    pub content_type: String,
    pub expected_size: Option<u64>,
    pub expected_sha256: Option<String>,
}
impl Default for UploadRunInputOptions {
    fn default() -> Self {
        Self {
            call: CallOptions::default(),
            content_type: "application/octet-stream".into(),
            expected_size: None,
            expected_sha256: None,
        }
    }
}
#[derive(Default)]
struct Measured {
    size: u64,
    digest: Sha256,
    eof: bool,
}
impl Client {
    pub async fn start_with_blob(
        &self,
        workflow: &str,
        descriptor: &crate::BlobDescriptor,
        options: impl Into<StartOptions>,
    ) -> Result<Run, SagatError> {
        descriptor.check()?;
        let options = options.into();
        let mut body = start_body(workflow, &options)?;
        let request_key = key(&options.call)?;
        body["workflowId"] = self
            .resolve_workflow(workflow, options.call.clone())
            .await?
            .into();
        body["inputBlobRef"] = json!(descriptor.id);
        self.execute(
            WireRequest::new(Method::POST, &["runs"], 202)
                .json(&body)?
                .keyed(request_key),
            options.call,
        )
        .await?
        .parse(Run::parse)
    }
    pub async fn rerun_as_new(
        &self,
        run: &str,
        reason: &str,
        options: impl Into<OperationOptions>,
    ) -> Result<OperationResult, SagatError> {
        self.operate(run, "RERUN_AS_NEW", reason, options).await
    }
    /// Consumes source once. Authentication, connection and response failures never replay it.
    pub async fn upload_run_input<R: AsyncRead + Unpin + Send + 'static>(
        &self,
        source: R,
        mut options: UploadRunInputOptions,
    ) -> Result<crate::BlobDescriptor, SagatError> {
        options.call.traceparent = self.options.telemetry.trace(options.call.traceparent);
        let started = std::time::Instant::now();
        self.options.telemetry.emit(
            crate::EventKind::Started,
            "upload",
            crate::SdkTransport::Rest,
            None,
            None,
        );
        let result = self.upload_once(source, options).await;
        self.options.telemetry.emit(
            if result.is_ok() {
                crate::EventKind::Succeeded
            } else {
                crate::EventKind::Failed
            },
            "upload",
            crate::SdkTransport::Rest,
            Some(started.elapsed()),
            result.as_ref().err().map(|e| e.code),
        );
        result
    }
    async fn upload_once<R: AsyncRead + Unpin + Send + 'static>(
        &self,
        source: R,
        options: UploadRunInputOptions,
    ) -> Result<crate::BlobDescriptor, SagatError> {
        if options.call.cancel.is_cancelled() {
            return Err(cancelled());
        }
        if options
            .call
            .traceparent
            .as_deref()
            .is_some_and(|v| !valid_traceparent(v))
        {
            return Err(SagatError::new(
                "BAD_REQUEST",
                "the trace context is invalid",
            ));
        }
        if options.expected_size.is_some_and(|v| v > i64::MAX as u64)
            || options
                .expected_sha256
                .as_deref()
                .is_some_and(|v| !crate::types::valid_digest(v))
        {
            return Err(validation());
        }
        let content_type =
            HeaderValue::from_str(&options.content_type).map_err(|_| validation())?;
        let timeout = options.call.timeout.unwrap_or(self.options.timeout);
        if !valid_timeout(timeout) {
            return Err(validation());
        }
        let deadline = tokio::time::Instant::now() + timeout;
        let token = tokio::select! {biased;_=options.call.cancel.cancelled()=>return Err(cancelled()),result=tokio::time::timeout_at(deadline,self.options.token.get())=>result.map_err(|_|SagatError::provider_failure())??};
        let mut bearer = HeaderValue::from_str(&format!("Bearer {}", token.as_str()))
            .map_err(|_| SagatError::provider_failure())?;
        bearer.set_sensitive(true);
        let measured = Arc::new(Mutex::new(Measured::default()));
        let observe = measured.clone();
        let source = tokio_util::io::ReaderStream::with_capacity(source, 256 * 1024);
        let stream =
            futures_util::stream::unfold((source, observe), |(mut source, state)| async move {
                match source.next().await {
                    Some(Ok(bytes)) => {
                        let result = state
                            .lock()
                            .map_err(|_| std::io::Error::other("upload measurement failed"))
                            .and_then(|mut state| {
                                state.size = state
                                    .size
                                    .checked_add(bytes.len() as u64)
                                    .filter(|v| *v <= i64::MAX as u64)
                                    .ok_or_else(|| std::io::Error::other("upload size exceeded"))?;
                                state.digest.update(&bytes);
                                Ok(())
                            })
                            .map(|()| bytes);
                        Some((result, (source, state)))
                    }
                    Some(Err(_)) => Some((
                        Err(std::io::Error::other("the upload source failed")),
                        (source, state),
                    )),
                    None => {
                        if let Ok(mut state) = state.lock() {
                            state.eof = true;
                        }
                        None
                    }
                }
            });
        let mut target = self.api.clone();
        target
            .path_segments_mut()
            .map_err(|_| validation())?
            .push("run-input-blobs");
        let agent = self
            .options
            .user_agent_suffix
            .as_deref()
            .filter(|s| !s.is_empty())
            .map_or_else(
                || format!("sagat-sdk-rust/{}", crate::VERSION),
                |s| format!("sagat-sdk-rust/{} {s}", crate::VERSION),
            );
        let mut request = self
            .http
            .post(target)
            .header(AUTHORIZATION, bearer)
            .header("accept", "application/json")
            .header("content-type", content_type)
            .header("x-octopus-namespace", &self.options.namespace)
            .header("user-agent", agent)
            .body(reqwest::Body::wrap_stream(stream));
        if let Some(trace) = &options.call.traceparent {
            request = request.header("traceparent", trace);
        }
        if let Some(size) = options.expected_size {
            request = request.header("x-octopus-blob-size", size);
        }
        if let Some(digest) = &options.expected_sha256 {
            request = request.header("x-octopus-blob-sha256", digest);
        }
        let response = tokio::select! {biased;_=options.call.cancel.cancelled()=>return Err(cancelled()),result=tokio::time::timeout_at(deadline,request.send())=>result.map_err(|_|transport())?.map_err(|_|transport())?};
        let descriptor = self
            .consume_response(response, 201, deadline, &options.call.cancel)
            .await?
            .parse(crate::BlobDescriptor::parse)?;
        let state = measured.lock().map_err(|_| protocol())?;
        let digest = format!("{:x}", state.digest.clone().finalize());
        if !state.eof
            || descriptor.size != state.size
            || descriptor.sha256 != digest
            || options.expected_size.is_some_and(|v| v != state.size)
            || options
                .expected_sha256
                .as_deref()
                .is_some_and(|v| v != digest)
        {
            return Err(protocol());
        }
        Ok(descriptor)
    }
}

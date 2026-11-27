use super::{Fault, Plan};
use futures_util::stream::BoxStream;
use tonic::{Request, Response, Status, transport::Channel};
#[allow(dead_code, clippy::all)]
mod pb {
    tonic::include_proto!("sagat_octopus_orchestrator.v1");
}

// A step-log batch (feature 041): only counts print, never a message, an exception, a field, a token
// or the identity of the attempt (FR-041-007 (d)).
impl std::fmt::Debug for pb::AppendStepLogsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppendStepLogsRequest")
            .field("records", &self.records.len())
            .finish_non_exhaustive()
    }
}
// The last records of an attempt on its report (design 041 section 2a): its counts only.
impl std::fmt::Debug for pb::StepLogCarry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepLogCarry")
            .field("records", &self.records.len())
            .field("highest_sequence", &self.highest_sequence)
            .finish()
    }
}
impl std::fmt::Debug for pb::StepLogRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepLogRecord")
            .field("sequence", &self.sequence)
            .field("fields", &self.fields.len())
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for pb::StepLogField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StepLogField { <redacted> }")
    }
}
impl std::fmt::Debug for pb::step_log_field::Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StepLogField::Value(<redacted>)")
    }
}

#[cfg(test)]
mod step_log_debug_tests {
    use super::pb;

    #[test]
    fn the_debug_of_a_step_log_batch_prints_counts_and_no_text() {
        let request = pb::AppendStepLogsRequest {
            session_token: "canary-session-token".into(),
            lease_token: "canary-lease-token".into(),
            records: vec![pb::StepLogRecord {
                sequence: 3,
                message: "canary-message".into(),
                exception: "canary-exception".into(),
                fields: vec![pb::StepLogField {
                    key: "canary-key".into(),
                    value: Some(pb::step_log_field::Value::StringValue(
                        "canary-value".into(),
                    )),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        for text in [
            format!("{request:?}"),
            format!("{request:#?}"),
            format!("{:?}", request.records[0].fields[0].value),
        ] {
            assert!(!text.contains("canary"), "{text}");
        }
        assert!(format!("{request:?}").contains("records: 1"));
    }

    /// A report holding a carry (design 041 section 2a) prints the carry's counts, never a record.
    #[test]
    fn the_debug_of_a_report_that_holds_a_carry_prints_counts_and_no_text() {
        let carry = pb::StepLogCarry {
            records: vec![pb::StepLogRecord {
                sequence: 3,
                message: "canary-message".into(),
                ..Default::default()
            }],
            highest_sequence: 5,
        };
        let complete = pb::CompleteStepRequest {
            step_logs: Some(carry.clone()),
            ..Default::default()
        };
        let fail = pb::FailStepRequest {
            step_logs: Some(carry),
            ..Default::default()
        };
        for text in [format!("{complete:?}"), format!("{fail:#?}")] {
            assert!(!text.contains("canary"), "{text}");
            assert!(
                text.contains("records: 1") && text.contains("highest_sequence: 5"),
                "{text}"
            );
        }
    }
}
#[derive(Clone)]
struct Proxy {
    client: pb::worker_service_client::WorkerServiceClient<Channel>,
    plan: Plan,
}
fn failure(fault: &Fault) -> Status {
    let code = if fault.action == "failBefore" {
        tonic::Code::Unavailable
    } else {
        match fault.code.as_deref().unwrap_or("UNAVAILABLE") {
            "UNAUTHENTICATED" => tonic::Code::Unauthenticated,
            "PERMISSION_DENIED" => tonic::Code::PermissionDenied,
            "ABORTED" => tonic::Code::Aborted,
            "INVALID_ARGUMENT" => tonic::Code::InvalidArgument,
            "FAILED_PRECONDITION" => tonic::Code::FailedPrecondition,
            "RESOURCE_EXHAUSTED" => tonic::Code::ResourceExhausted,
            "NOT_FOUND" => tonic::Code::NotFound,
            "UNIMPLEMENTED" => tonic::Code::Unimplemented,
            "DEADLINE_EXCEEDED" => tonic::Code::DeadlineExceeded,
            "INTERNAL" => tonic::Code::Internal,
            _ => tonic::Code::Unavailable,
        }
    };
    let mut error = Status::new(code, "injected");
    if let Some(reason) = &fault.reason
        && let Ok(value) = reason.parse()
    {
        error.metadata_mut().insert("x-sagat-error", value);
    }
    if let Some(ms) = fault.retry_after_ms
        && let Ok(value) = ms.to_string().parse()
    {
        error.metadata_mut().insert("retry-after-ms", value);
    }
    error
}
impl Proxy {
    async fn unary<T: Send, U, F, Fut>(
        &self,
        method: &str,
        request: Request<T>,
        forward: F,
    ) -> Result<Response<U>, Status>
    where
        F: FnOnce(pb::worker_service_client::WorkerServiceClient<Channel>, Request<T>) -> Fut
            + Send,
        Fut: std::future::Future<Output = Result<Response<U>, Status>> + Send,
    {
        let fault = self.plan.next("grpc", method);
        if let Some(fault) = &fault
            && matches!(fault.action.as_str(), "failBefore" | "status")
        {
            return Err(failure(fault));
        }
        let response = forward(self.client.clone(), request).await?;
        if fault.is_some_and(|f| f.action == "dropResponse") {
            return Err(Status::unavailable("injected lost response"));
        }
        Ok(response)
    }
}
#[tonic::async_trait]
impl pb::worker_service_server::WorkerService for Proxy {
    async fn register_worker(
        &self,
        request: Request<pb::RegisterWorkerRequest>,
    ) -> Result<Response<pb::RegisterWorkerResponse>, Status> {
        self.unary(
            "RegisterWorker",
            request,
            |mut client, request| async move { client.register_worker(request).await },
        )
        .await
    }
    async fn poll_tasks(
        &self,
        request: Request<pb::PollTasksRequest>,
    ) -> Result<Response<pb::PollTasksResponse>, Status> {
        self.unary("PollTasks", request, |mut client, request| async move {
            client.poll_tasks(request).await
        })
        .await
    }
    async fn heartbeat(
        &self,
        request: Request<pb::HeartbeatRequest>,
    ) -> Result<Response<pb::HeartbeatResponse>, Status> {
        self.unary("Heartbeat", request, |mut client, request| async move {
            client.heartbeat(request).await
        })
        .await
    }
    async fn complete_step(
        &self,
        request: Request<pb::CompleteStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        self.unary("CompleteStep", request, |mut client, request| async move {
            client.complete_step(request).await
        })
        .await
    }
    async fn fail_step(
        &self,
        request: Request<pb::FailStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        self.unary("FailStep", request, |mut client, request| async move {
            client.fail_step(request).await
        })
        .await
    }
    async fn resolve_secrets(
        &self,
        request: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolveSecretsResponse>, Status> {
        self.unary(
            "ResolveSecrets",
            request,
            |mut client, request| async move { client.resolve_secrets(request).await },
        )
        .await
    }
    async fn append_step_logs(
        &self,
        _: Request<pb::AppendStepLogsRequest>,
    ) -> Result<Response<pb::AppendStepLogsResponse>, Status> {
        Err(Status::unimplemented("AppendStepLogs"))
    }
    async fn resolve_database(
        &self,
        request: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolvedDatabase>, Status> {
        self.unary(
            "ResolveDatabase",
            request,
            |mut client, request| async move { client.resolve_database(request).await },
        )
        .await
    }
    type StreamTasksStream = BoxStream<'static, Result<pb::PollTasksResponse, Status>>;
    async fn stream_tasks(
        &self,
        request: Request<pb::PollTasksRequest>,
    ) -> Result<Response<Self::StreamTasksStream>, Status> {
        let fault = self.plan.next("grpc", "StreamTasks");
        if let Some(fault) = &fault
            && matches!(fault.action.as_str(), "failBefore" | "status")
        {
            return Err(failure(fault));
        }
        let mut client = self.client.clone();
        let stream = client.stream_tasks(request).await?.into_inner();
        let action = fault.map(|f| f.action).unwrap_or_default();
        let stream = futures_util::stream::try_unfold(
            (stream, action, false),
            |(mut stream, action, drop_next)| async move {
                if drop_next {
                    return Err(Status::unavailable("injected interrupted stream"));
                }
                if action == "dropResponse" {
                    while stream.message().await?.is_some() {}
                    return Err(Status::unavailable("injected lost stream"));
                }
                match stream.message().await? {
                    Some(frame) => {
                        let drop_next = action == "dropStreamAfterFirst" && !frame.tasks.is_empty();
                        Ok(Some((frame, (stream, action, drop_next))))
                    }
                    None => Ok(None),
                }
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }
    type ReadBlobStream = BoxStream<'static, Result<pb::ReadBlobResponse, Status>>;
    async fn read_blob(
        &self,
        request: Request<pb::ReadBlobRequest>,
    ) -> Result<Response<Self::ReadBlobStream>, Status> {
        let mut client = self.client.clone();
        let stream = client.read_blob(request).await?.into_inner();
        Ok(Response::new(Box::pin(stream)))
    }
    async fn write_blob(
        &self,
        request: Request<tonic::Streaming<pb::WriteBlobRequest>>,
    ) -> Result<Response<pb::WriteBlobResponse>, Status> {
        let (metadata, extensions, mut stream) = request.into_parts();
        let mut frames = Vec::new();
        let mut bytes = 0usize;
        while let Some(frame) = stream.message().await? {
            bytes = bytes.saturating_add(prost::Message::encoded_len(&frame));
            if bytes > 16 * 1024 * 1024 {
                return Err(Status::resource_exhausted("fixture upload limit"));
            }
            frames.push(frame);
        }
        let mut client = self.client.clone();
        client
            .write_blob(Request::from_parts(
                metadata,
                extensions,
                futures_util::stream::iter(frames),
            ))
            .await
    }
    // Protocol 1.10 (design 044, workflow as code): the Rust SDK offers no workflow as code, so the
    // fault proxy serves none of the workflow procedures (as it does for AppendStepLogs).
    async fn complete_workflow_task(
        &self,
        _: Request<pb::CompleteWorkflowTaskRequest>,
    ) -> Result<Response<pb::CompleteWorkflowTaskResponse>, Status> {
        Err(Status::unimplemented("CompleteWorkflowTask"))
    }
    async fn fail_workflow_task(
        &self,
        _: Request<pb::FailWorkflowTaskRequest>,
    ) -> Result<Response<pb::FailWorkflowTaskResponse>, Status> {
        Err(Status::unimplemented("FailWorkflowTask"))
    }
    type ReadHistoryStream = BoxStream<'static, Result<pb::ReadHistoryResponse, Status>>;
    async fn read_history(
        &self,
        _: Request<pb::ReadHistoryRequest>,
    ) -> Result<Response<Self::ReadHistoryStream>, Status> {
        Err(Status::unimplemented("ReadHistory"))
    }
    async fn respond_worker_query(
        &self,
        _: Request<pb::RespondWorkerQueryRequest>,
    ) -> Result<Response<pb::RespondWorkerQueryResponse>, Status> {
        Err(Status::unimplemented("RespondWorkerQuery"))
    }
    async fn respond_replay_check(
        &self,
        _: Request<pb::RespondReplayCheckRequest>,
    ) -> Result<Response<pb::RespondReplayCheckResponse>, Status> {
        Err(Status::unimplemented("RespondReplayCheck"))
    }
    async fn release_sticky(
        &self,
        _: Request<pb::ReleaseStickyRequest>,
    ) -> Result<Response<pb::ReleaseStickyResponse>, Status> {
        Err(Status::unimplemented("ReleaseSticky"))
    }
}
pub async fn start(
    url: &str,
    plan: Plan,
) -> Result<(String, tokio::task::JoinHandle<()>), Box<dyn std::error::Error>> {
    let client = pb::worker_service_client::WorkerServiceClient::connect(url.to_owned())
        .await?
        .max_decoding_message_size(16 * 1024 * 1024)
        .max_encoding_message_size(16 * 1024 * 1024);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(
                pb::worker_service_server::WorkerServiceServer::new(Proxy { client, plan })
                    .max_decoding_message_size(16 * 1024 * 1024)
                    .max_encoding_message_size(16 * 1024 * 1024),
            )
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await;
    });
    Ok((endpoint, task))
}

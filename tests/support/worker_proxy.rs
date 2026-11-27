//! Faults at the real gRPC boundary, preserving the durable engine and target execution.
//!
//! A proxy of the worker service that forwards each call to a real API and can lose or hold what comes back. First
//! written for the database runner; moved here by design 041's task T041-8, which teaches it two faults of its own:
//! `lose_append_ack` forwards one `AppendStepLogs` and drops its answer, and `lose_report_ack` forwards one
//! `CompleteStep` or `FailStep` and drops its answer (used by the carry's tests, task T041-36). Each fault is armed for
//! one call and disarms itself. Declared by the targets that use it through `#[path]`, not by `tests/support/mod.rs`.
#![allow(dead_code)]
use sagat_api_grpc::pb::{
    self,
    worker_service_client::WorkerServiceClient,
    worker_service_server::{WorkerService, WorkerServiceServer},
};
use std::{
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_stream::{Stream, StreamExt};
use tonic::{
    Request, Response, Status,
    transport::{Channel, Server},
};
#[derive(Default)]
pub struct Faults {
    pub lose_complete_ack: AtomicBool,
    /// Forward the next `AppendStepLogs`, then answer `UNAVAILABLE` as a lost answer would (design 041, task T041-8).
    pub lose_append_ack: AtomicBool,
    /// Forward the next `CompleteStep` or `FailStep`, then answer `UNAVAILABLE` as a lost answer would (task T041-8,
    /// for the carry of task T041-36).
    pub lose_report_ack: AtomicBool,
    /// The `AppendStepLogs` answers dropped by `lose_append_ack`.
    pub lost_appends: AtomicUsize,
    /// The report answers dropped by `lose_report_ack`.
    pub lost_reports: AtomicUsize,
    pub fail_heartbeat: AtomicBool,
    pub wrong_namespace: AtomicBool,
    pub heartbeats: AtomicUsize,
    pub receipts: AtomicUsize,
    pub block_complete: AtomicBool,
    pub blocked_completions: AtomicUsize,
    pub measure: AtomicBool,
    pub resolutions: Mutex<Vec<Duration>>,
    pub completions: Mutex<Vec<Duration>>,
}
struct Proxy {
    client: WorkerServiceClient<Channel>,
    faults: Arc<Faults>,
}
pub struct Running {
    pub url: String,
    pub faults: Arc<Faults>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub async fn start(url: &str) -> Running {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url_out = format!("http://{}", listener.local_addr().unwrap());
    let faults = Arc::new(Faults::default());
    let proxy = Proxy {
        client: WorkerServiceClient::connect(url.to_owned()).await.unwrap(),
        faults: faults.clone(),
    };
    let task = tokio::spawn(async move {
        Server::builder()
            .add_service(WorkerServiceServer::new(proxy))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Running {
        url: url_out,
        faults,
        task,
    }
}
impl Proxy {
    /// The answer of a report that reached the server, dropped once when `lose_report_ack` is armed: the fault is
    /// spent by the call whatever the server answered.
    fn lose_report(
        &self,
        response: Result<Response<pb::StepCommandResponse>, Status>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        if self.faults.lose_report_ack.swap(false, Ordering::SeqCst) {
            self.faults.lost_reports.fetch_add(1, Ordering::SeqCst);
            return Err(Status::unavailable("lost report answer"));
        }
        response
    }
}
#[tonic::async_trait]
impl WorkerService for Proxy {
    async fn register_worker(
        &self,
        r: Request<pb::RegisterWorkerRequest>,
    ) -> Result<Response<pb::RegisterWorkerResponse>, Status> {
        self.client.clone().register_worker(r).await
    }
    async fn poll_tasks(
        &self,
        r: Request<pb::PollTasksRequest>,
    ) -> Result<Response<pb::PollTasksResponse>, Status> {
        self.client.clone().poll_tasks(r).await
    }
    type StreamTasksStream =
        Pin<Box<dyn Stream<Item = Result<pb::PollTasksResponse, Status>> + Send>>;
    async fn stream_tasks(
        &self,
        _: Request<pb::PollTasksRequest>,
    ) -> Result<Response<Self::StreamTasksStream>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn heartbeat(
        &self,
        r: Request<pb::HeartbeatRequest>,
    ) -> Result<Response<pb::HeartbeatResponse>, Status> {
        self.faults.heartbeats.fetch_add(1, Ordering::SeqCst);
        if self.faults.fail_heartbeat.load(Ordering::SeqCst) {
            return Err(Status::unavailable("injected heartbeat outage"));
        }
        self.client.clone().heartbeat(r).await
    }
    async fn complete_step(
        &self,
        r: Request<pb::CompleteStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        if self.faults.block_complete.load(Ordering::SeqCst) {
            self.faults
                .blocked_completions
                .fetch_add(1, Ordering::SeqCst);
            return std::future::pending().await;
        }
        let started = Instant::now();
        let response = match self.client.clone().complete_step(r).await {
            Ok(response) => response,
            Err(status) => return self.lose_report(Err(status)),
        };
        if self.faults.measure.load(Ordering::SeqCst) {
            let mut samples = self.faults.completions.lock().unwrap();
            assert!(samples.len() < 512);
            samples.push(started.elapsed());
        }
        if response.get_ref().already_applied {
            self.faults.receipts.fetch_add(1, Ordering::SeqCst);
        }
        if self.faults.lose_complete_ack.swap(false, Ordering::SeqCst) {
            return Err(Status::unavailable("lost committed ack"));
        }
        self.lose_report(Ok(response))
    }
    async fn fail_step(
        &self,
        r: Request<pb::FailStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        let response = self.client.clone().fail_step(r).await;
        self.lose_report(response)
    }
    async fn resolve_secrets(
        &self,
        _: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolveSecretsResponse>, Status> {
        Err(Status::unimplemented(
            "runner must use managed capabilities",
        ))
    }
    async fn append_step_logs(
        &self,
        r: Request<pb::AppendStepLogsRequest>,
    ) -> Result<Response<pb::AppendStepLogsResponse>, Status> {
        let response = self.client.clone().append_step_logs(r).await;
        // Armed for one call: spent by it whatever the server answered.
        if self.faults.lose_append_ack.swap(false, Ordering::SeqCst) {
            self.faults.lost_appends.fetch_add(1, Ordering::SeqCst);
            return Err(Status::unavailable("lost step-log answer"));
        }
        response
    }
    async fn resolve_database(
        &self,
        r: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolvedDatabase>, Status> {
        let started = Instant::now();
        let mut response = self.client.clone().resolve_database(r).await?;
        if self.faults.measure.load(Ordering::SeqCst) {
            let mut samples = self.faults.resolutions.lock().unwrap();
            assert!(samples.len() < 512);
            samples.push(started.elapsed());
        }
        if self.faults.wrong_namespace.load(Ordering::SeqCst) {
            response.get_mut().namespace_id = uuid::Uuid::new_v4().to_string();
        }
        Ok(response)
    }
    async fn write_blob(
        &self,
        r: Request<tonic::Streaming<pb::WriteBlobRequest>>,
    ) -> Result<Response<pb::WriteBlobResponse>, Status> {
        let (metadata, extensions, stream) = r.into_parts();
        self.client
            .clone()
            .write_blob(Request::from_parts(
                metadata,
                extensions,
                stream.filter_map(Result::ok),
            ))
            .await
    }
    type ReadBlobStream = Pin<Box<dyn Stream<Item = Result<pb::ReadBlobResponse, Status>> + Send>>;
    async fn read_blob(
        &self,
        _: Request<pb::ReadBlobRequest>,
    ) -> Result<Response<Self::ReadBlobStream>, Status> {
        Err(Status::unimplemented("runner does not fetch run data"))
    }
}

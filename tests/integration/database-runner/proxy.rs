//! Faults at the real gRPC boundary, preserving the durable engine and target execution.
use sagat_api_grpc::pb::{
    self,
    worker_service_client::WorkerServiceClient,
    worker_service_server::{WorkerService, WorkerServiceServer},
};
use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio_stream::{Stream, StreamExt};
use tonic::{
    Request, Response, Status,
    transport::{Channel, Server},
};
#[derive(Default)]
pub struct Faults {
    pub lose_complete_ack: AtomicBool,
    pub fail_heartbeat: AtomicBool,
    pub wrong_namespace: AtomicBool,
    pub heartbeats: AtomicUsize,
    pub receipts: AtomicUsize,
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
        let response = self.client.clone().complete_step(r).await?;
        if response.get_ref().already_applied {
            self.faults.receipts.fetch_add(1, Ordering::SeqCst);
        }
        if self.faults.lose_complete_ack.swap(false, Ordering::SeqCst) {
            return Err(Status::unavailable("lost committed ack"));
        }
        Ok(response)
    }
    async fn fail_step(
        &self,
        r: Request<pb::FailStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        self.client.clone().fail_step(r).await
    }
    async fn resolve_secrets(
        &self,
        _: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolveSecretsResponse>, Status> {
        Err(Status::unimplemented(
            "runner must use managed capabilities",
        ))
    }
    async fn resolve_database(
        &self,
        r: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolvedDatabase>, Status> {
        let mut response = self.client.clone().resolve_database(r).await?;
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

//! A worker service that answers every call at once: what the transport tests exercise is who gets
//! to talk to it and for how long, not what it says.

use sagat_api_grpc::pb::worker_service_server::WorkerService;
use sagat_api_grpc::pb::{
    CompleteStepRequest, FailStepRequest, HeartbeatRequest, HeartbeatResponse, PollTasksRequest,
    PollTasksResponse, RegisterWorkerRequest, RegisterWorkerResponse, ResolveSecretsRequest,
    ResolveSecretsResponse, StepCommandResponse,
};
use tonic::{Request, Response, Status};

pub struct Stub;

#[tonic::async_trait]
impl WorkerService for Stub {
    async fn register_worker(
        &self,
        _: Request<RegisterWorkerRequest>,
    ) -> Result<Response<RegisterWorkerResponse>, Status> {
        Ok(Response::new(RegisterWorkerResponse::default()))
    }
    async fn poll_tasks(
        &self,
        _: Request<PollTasksRequest>,
    ) -> Result<Response<PollTasksResponse>, Status> {
        Ok(Response::new(PollTasksResponse::default()))
    }
    async fn heartbeat(
        &self,
        _: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        Ok(Response::new(HeartbeatResponse::default()))
    }
    async fn complete_step(
        &self,
        _: Request<CompleteStepRequest>,
    ) -> Result<Response<StepCommandResponse>, Status> {
        Ok(Response::new(StepCommandResponse::default()))
    }
    async fn fail_step(
        &self,
        _: Request<FailStepRequest>,
    ) -> Result<Response<StepCommandResponse>, Status> {
        Ok(Response::new(StepCommandResponse::default()))
    }
    async fn resolve_secrets(
        &self,
        _: Request<ResolveSecretsRequest>,
    ) -> Result<Response<ResolveSecretsResponse>, Status> {
        Ok(Response::new(ResolveSecretsResponse::default()))
    }
}

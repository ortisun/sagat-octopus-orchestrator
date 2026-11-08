//! A scripted worker service for the tests of the runner's shell (feature 035, T035-058, D035-143).
//!
//! It is the real transport (a tonic server on a loopback port, the runner's own `Rpc` client) with a service that records what it
//! is asked and answers from a script, so the shell is tested end to end except for the control plane. A test waits on what the
//! service has seen ([`Fake::until`]), woken by each call, with a bound: no sleeps.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sagat_api_grpc::pb;
use sagat_api_grpc::pb::worker_service_server::WorkerService;
use sagat_api_grpc::{AdmissionProvider, GrpcConfig, serve, worker_service};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::{Notify, oneshot};
use tonic::{Request, Response, Status};

/// What the service answers to a heartbeat.
pub(crate) type HeartbeatAnswer =
    Box<dyn Fn(&pb::HeartbeatRequest) -> Result<pb::HeartbeatResponse, Status> + Send + Sync>;

/// Everything the service has seen, and what it will answer.
#[derive(Default)]
pub(crate) struct Script {
    pub registered: Vec<pb::RegisterWorkerRequest>,
    pub polled: Vec<pb::PollTasksRequest>,
    pub resolved: Vec<pb::ResolveSecretsRequest>,
    pub heartbeats: Vec<pb::HeartbeatRequest>,
    /// Every report the service was sent, the ones it refused too: a retry is the same request again.
    pub completed: Vec<pb::CompleteStepRequest>,
    pub failed: Vec<pb::FailStepRequest>,
    /// Handed out by the polls, in order, as many as each poll asks for.
    pub tasks: VecDeque<pb::TaskLease>,
    /// The answer to `ResolveDatabase`; `None` is `NOT_FOUND`.
    pub database: Option<pb::ResolvedDatabase>,
    /// The answer to a heartbeat; by default the lease is extended by a minute and nothing is cancelled.
    pub heartbeat: Option<HeartbeatAnswer>,
    /// The first reports are refused with these statuses, in order, before one is accepted.
    pub report_errors: VecDeque<Status>,
}

/// The service, and the handle a test keeps to read and script it.
#[derive(Clone, Default)]
pub(crate) struct Fake {
    script: Arc<Mutex<Script>>,
    changed: Arc<Notify>,
}

impl Fake {
    /// Reads or changes the script.
    pub fn with<T>(&self, f: impl FnOnce(&mut Script) -> T) -> T {
        f(&mut self.script.lock().unwrap())
    }

    /// Waits until `condition` holds of what the service has seen. Woken by every call; bounded, and panics naming `what`.
    pub async fn until(&self, what: &str, condition: impl Fn(&Script) -> bool) {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                // registered before the condition is read, so a call between the read and the wait is not missed
                notified.as_mut().enable();
                if condition(&self.script.lock().unwrap()) {
                    return;
                }
                notified.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(20), wait)
            .await
            .is_err()
        {
            panic!("the service never saw {what}");
        }
    }

    fn touched(&self) {
        self.changed.notify_waiters();
    }

    fn report_error(&self) -> Option<Status> {
        self.script.lock().unwrap().report_errors.pop_front()
    }
}

impl AdmissionProvider for Fake {}

#[tonic::async_trait]
impl WorkerService for Fake {
    type StreamTasksStream = tokio_stream::Empty<Result<pb::PollTasksResponse, Status>>;
    type ReadBlobStream = tokio_stream::Empty<Result<pb::ReadBlobResponse, Status>>;

    async fn register_worker(
        &self,
        request: Request<pb::RegisterWorkerRequest>,
    ) -> Result<Response<pb::RegisterWorkerResponse>, Status> {
        self.with(|script| script.registered.push(request.into_inner()));
        self.touched();
        Ok(Response::new(pb::RegisterWorkerResponse {
            worker_id: uuid::Uuid::from_u128(7).to_string(),
            session_token: "session-token".into(),
            accepted_protocol_version: "1.6".into(),
        }))
    }

    async fn poll_tasks(
        &self,
        request: Request<pb::PollTasksRequest>,
    ) -> Result<Response<pb::PollTasksResponse>, Status> {
        let request = request.into_inner();
        let tasks = self.with(|script| {
            let room = usize::try_from(request.max_tasks).unwrap_or(0);
            let tasks: Vec<_> = (0..room).map_while(|_| script.tasks.pop_front()).collect();
            script.polled.push(request);
            tasks
        });
        self.touched();
        Ok(Response::new(pb::PollTasksResponse {
            tasks,
            ..Default::default()
        }))
    }

    async fn resolve_database(
        &self,
        request: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolvedDatabase>, Status> {
        let answer = self.with(|script| {
            script.resolved.push(request.into_inner());
            script.database.clone()
        });
        self.touched();
        answer
            .map(Response::new)
            .ok_or_else(|| Status::not_found("nothing scripted"))
    }

    async fn heartbeat(
        &self,
        request: Request<pb::HeartbeatRequest>,
    ) -> Result<Response<pb::HeartbeatResponse>, Status> {
        let request = request.into_inner();
        let answer = self.with(|script| {
            let answer = script.heartbeat.as_ref().map_or_else(
                || {
                    Ok(pb::HeartbeatResponse {
                        lease_expires_at_epoch_ms: epoch_ms_in(Duration::from_secs(60)),
                        cancel_requested: false,
                    })
                },
                |answer| answer(&request),
            );
            script.heartbeats.push(request);
            answer
        });
        self.touched();
        answer.map(Response::new)
    }

    async fn complete_step(
        &self,
        request: Request<pb::CompleteStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        self.with(|script| script.completed.push(request.into_inner()));
        self.touched();
        match self.report_error() {
            Some(status) => Err(status),
            None => Ok(Response::new(pb::StepCommandResponse::default())),
        }
    }

    async fn fail_step(
        &self,
        request: Request<pb::FailStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        self.with(|script| script.failed.push(request.into_inner()));
        self.touched();
        match self.report_error() {
            Some(status) => Err(status),
            None => Ok(Response::new(pb::StepCommandResponse::default())),
        }
    }

    async fn stream_tasks(
        &self,
        _: Request<pb::PollTasksRequest>,
    ) -> Result<Response<Self::StreamTasksStream>, Status> {
        Err(Status::unimplemented("the fake has no stream"))
    }

    async fn resolve_secrets(
        &self,
        _: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolveSecretsResponse>, Status> {
        Err(Status::unimplemented("the fake has no secrets"))
    }

    async fn write_blob(
        &self,
        _: Request<tonic::Streaming<pb::WriteBlobRequest>>,
    ) -> Result<Response<pb::WriteBlobResponse>, Status> {
        Err(Status::unimplemented("the fake has no blobs"))
    }

    async fn read_blob(
        &self,
        _: Request<pb::ReadBlobRequest>,
    ) -> Result<Response<Self::ReadBlobStream>, Status> {
        Err(Status::unimplemented("the fake has no blobs"))
    }
}

/// The instant `after` from now, in the milliseconds since the epoch a lease carries.
pub(crate) fn epoch_ms_in(after: Duration) -> i64 {
    i64::try_from((time::OffsetDateTime::now_utc() + after).unix_timestamp_nanos() / 1_000_000)
        .unwrap()
}

/// A running service. Dropping it stops it.
pub(crate) struct Server {
    pub addr: SocketAddr,
    _stop: oneshot::Sender<()>,
}

/// Serves `fake` on a loopback port.
pub(crate) async fn serve_fake(fake: Fake) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = serve(
            listener,
            worker_service(fake),
            GrpcConfig::default(),
            async {
                let _ = stopped.await;
            },
        )
        .await;
    });
    Server { addr, _stop: stop }
}

/// The directory of a runner under test: its token, and the configuration it reads.
pub(crate) struct Home {
    /// Held so that the directory outlives the runner that reads it.
    pub _directory: tempfile::TempDir,
    pub config: PathBuf,
}

/// The namespace the test runner serves.
pub(crate) const NAMESPACE_ID: &str = "00000000-0000-0000-0000-000000000001";
pub(crate) const NAMESPACE_SLUG: &str = "payments";

/// The configuration of a runner that serves `server`, with `lake` as its lake section when there is one.
pub(crate) fn home(server: &Server, lake: Option<serde_json::Value>) -> Home {
    let directory = tempfile::tempdir().expect("a directory");
    let token = directory.path().join("token");
    std::fs::write(&token, "identity-token").expect("the token");
    let limits = json!({"maxStatementBytes":16384,"maxParameters":64,"maxParameterBytes":65536,
        "maxRows":1000,"maxResultBytes":1048576,"timeoutMs":10000,"connectTimeoutMs":5000,"poolSize":4,"idleTimeoutMs":30000});
    let mut document = json!({
        "grpcUrl": format!("http://{}", server.addr), "tokenFile": token, "name": "runner",
        "maxConcurrent": 4, "pollIntervalMs": 100, "rpcTimeoutMs": 2000, "developmentLoopback": true,
        "deployment": {"allowCidrs": ["10.0.0.0/8"], "allowPorts": [5432], "denyCidrs": [], "limits": limits},
        "namespaces": [{"id": NAMESPACE_ID, "slug": NAMESPACE_SLUG, "allowCidrs": ["10.2.0.0/16"],
            "allowPorts": [5432], "limits": limits}]
    });
    if let Some(lake) = lake {
        document["lake"] = lake;
    }
    let config = directory.path().join("runner.json");
    std::fs::write(&config, serde_json::to_vec(&document).unwrap()).expect("the configuration");
    Home {
        _directory: directory,
        config,
    }
}

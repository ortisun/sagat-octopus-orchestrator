//! Capacity limits of the whole worker service (finding "gRPC capacity isolation" of the security
//! review): how many connections it keeps and how many calls it runs at once, over real sockets.
//!
//! The service behind the listener is a stub whose calls park until the test lets them go, so the
//! tests hold the engine at an exact level of load without a database or a clock.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::worker_service_server::WorkerService;
use sagat_api_grpc::pb::{
    CompleteStepRequest, FailStepRequest, HeartbeatRequest, HeartbeatResponse, PollTasksRequest,
    PollTasksResponse, RegisterWorkerRequest, RegisterWorkerResponse, ResolveSecretsRequest,
    ResolveSecretsResponse, StepCommandResponse,
};
use sagat_api_grpc::{ERROR_METADATA, GrpcConfig, RETRY_AFTER_METADATA, serve, worker_service};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, oneshot};
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Request, Response, Status};

/// Every call parks until a permit of `gate` is released, and is counted when it enters.
struct Stub {
    gate: Arc<Semaphore>,
    entered: Arc<AtomicUsize>,
}

impl Stub {
    async fn park(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        if let Ok(permit) = self.gate.acquire().await {
            permit.forget();
        }
    }
}

#[tonic::async_trait]
impl WorkerService for Stub {
    async fn register_worker(
        &self,
        _: Request<RegisterWorkerRequest>,
    ) -> Result<Response<RegisterWorkerResponse>, Status> {
        self.park().await;
        Ok(Response::new(RegisterWorkerResponse::default()))
    }
    async fn poll_tasks(
        &self,
        _: Request<PollTasksRequest>,
    ) -> Result<Response<PollTasksResponse>, Status> {
        self.park().await;
        Ok(Response::new(PollTasksResponse::default()))
    }
    async fn heartbeat(
        &self,
        _: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        self.park().await;
        Ok(Response::new(HeartbeatResponse::default()))
    }
    async fn complete_step(
        &self,
        _: Request<CompleteStepRequest>,
    ) -> Result<Response<StepCommandResponse>, Status> {
        self.park().await;
        Ok(Response::new(StepCommandResponse::default()))
    }
    async fn resolve_secrets(
        &self,
        _: Request<ResolveSecretsRequest>,
    ) -> Result<Response<ResolveSecretsResponse>, Status> {
        self.park().await;
        Ok(Response::new(ResolveSecretsResponse::default()))
    }
    async fn fail_step(
        &self,
        _: Request<FailStepRequest>,
    ) -> Result<Response<StepCommandResponse>, Status> {
        self.park().await;
        Ok(Response::new(StepCommandResponse::default()))
    }
}

struct Harness {
    url: String,
    gate: Arc<Semaphore>,
    entered: Arc<AtomicUsize>,
    _stop: oneshot::Sender<()>,
}

async fn serve_stub(config: GrpcConfig) -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let gate = Arc::new(Semaphore::new(0));
    let entered = Arc::new(AtomicUsize::new(0));
    let stub = Stub {
        gate: Arc::clone(&gate),
        entered: Arc::clone(&entered),
    };
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = serve(listener, worker_service(stub), config, async {
            let _ = stopped.await;
        })
        .await;
    });
    Harness {
        url,
        gate,
        entered,
        _stop: stop,
    }
}

async fn client(url: &str) -> WorkerServiceClient<Channel> {
    WorkerServiceClient::new(
        Endpoint::from_shared(url.to_owned())
            .expect("endpoint")
            .connect()
            .await
            .expect("connect"),
    )
}

async fn entered(h: &Harness, n: usize) {
    for _ in 0..400 {
        if h.entered.load(Ordering::SeqCst) >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "only {} calls reached the service, wanted {n}",
        h.entered.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn a_full_engine_sheds_calls_that_look_for_work_before_calls_that_keep_work_alive() {
    // 8 calls at once; calls that look for work are shed above 6, the other two places are reserved.
    let h = serve_stub(GrpcConfig {
        max_in_flight: 8,
        ..GrpcConfig::default()
    })
    .await;
    let client = client(&h.url).await;

    let mut parked = Vec::new();
    for _ in 0..6 {
        let mut c = client.clone();
        parked.push(tokio::spawn(async move {
            c.poll_tasks(PollTasksRequest::default()).await
        }));
    }
    entered(&h, 6).await;

    // The seventh poll is shed: a typed error with the retry hint, and it never reached the service.
    let shed = client
        .clone()
        .poll_tasks(PollTasksRequest::default())
        .await
        .expect_err("shed");
    assert_eq!(shed.code(), Code::ResourceExhausted);
    assert!(shed.metadata().get(RETRY_AFTER_METADATA).is_some());
    assert_eq!(
        shed.metadata()
            .get(ERROR_METADATA)
            .and_then(|v| v.to_str().ok()),
        Some("RATE_LIMITED")
    );
    let register = client
        .clone()
        .register_worker(RegisterWorkerRequest::default())
        .await
        .expect_err("registering looks for work too");
    assert_eq!(register.code(), Code::ResourceExhausted);
    assert_eq!(
        h.entered.load(Ordering::SeqCst),
        6,
        "shed before the service"
    );

    // The reserve is for heartbeats and reports.
    for _ in 0..2 {
        let mut c = client.clone();
        parked.push(tokio::spawn(async move {
            c.heartbeat(HeartbeatRequest::default())
                .await
                .map(|_| PollTasksResponse::default().into())
        }));
    }
    entered(&h, 8).await;
    for call in [
        client
            .clone()
            .heartbeat(HeartbeatRequest::default())
            .await
            .map(|_| ()),
        client
            .clone()
            .complete_step(CompleteStepRequest::default())
            .await
            .map(|_| ()),
        client
            .clone()
            .fail_step(FailStepRequest::default())
            .await
            .map(|_| ()),
    ] {
        assert_eq!(
            call.expect_err("completely full").code(),
            Code::ResourceExhausted
        );
    }

    // Everything admitted finishes, and the places are given back: the next call is served.
    h.gate.add_permits(8);
    for call in parked {
        call.await
            .expect("task")
            .expect("the admitted calls succeed");
    }
    h.gate.add_permits(1);
    client
        .clone()
        .poll_tasks(PollTasksRequest::default())
        .await
        .expect("served once there is room");
}

#[tokio::test]
async fn a_connection_beyond_the_cap_waits_for_a_place_instead_of_being_served() {
    let h = serve_stub(GrpcConfig {
        max_connections: 2,
        ..GrpcConfig::default()
    })
    .await;
    h.gate.add_permits(1000);
    let mut a = client(&h.url).await;
    let mut b = client(&h.url).await;
    a.poll_tasks(PollTasksRequest::default()).await.expect("a");
    b.poll_tasks(PollTasksRequest::default()).await.expect("b");

    // A third connection is not accepted while two are open: its call gets no answer.
    let mut c = WorkerServiceClient::new(
        Endpoint::from_shared(h.url.clone())
            .expect("endpoint")
            .connect_lazy(),
    );
    let waiting = tokio::time::timeout(
        Duration::from_millis(600),
        c.poll_tasks(PollTasksRequest::default()),
    )
    .await;
    assert!(waiting.is_err(), "served although the server was full");

    // When one closes, the waiting connection takes its place.
    drop(a);
    let mut served = false;
    for _ in 0..20 {
        let attempt = tokio::time::timeout(
            Duration::from_millis(500),
            c.poll_tasks(PollTasksRequest::default()),
        )
        .await;
        if matches!(attempt, Ok(Ok(_))) {
            served = true;
            break;
        }
    }
    assert!(
        served,
        "the place that was freed was never given to the waiting connection"
    );
    b.poll_tasks(PollTasksRequest::default())
        .await
        .expect("the connection that stayed is unaffected");
}

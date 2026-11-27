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
use sagat_api_grpc::{
    AdmissionProvider, ERROR_METADATA, GrpcConfig, RETRY_AFTER_METADATA, WorkerApi, serve,
    worker_service,
};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, oneshot};
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Request, Response, Status};

/// Every call parks until a permit of `gate` is released, and is counted when it enters.
struct Stub {
    gate: Arc<Semaphore>,
    entered: Arc<AtomicUsize>,
    /// The acknowledgements among them.
    acknowledgements: Arc<AtomicUsize>,
}

impl AdmissionProvider for Stub {}

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
    type StreamTasksStream = tokio_stream::Empty<Result<PollTasksResponse, Status>>;

    async fn write_blob(
        &self,
        _: Request<tonic::Streaming<sagat_api_grpc::pb::WriteBlobRequest>>,
    ) -> Result<Response<sagat_api_grpc::pb::WriteBlobResponse>, Status> {
        self.park().await;
        Ok(Response::new(
            sagat_api_grpc::pb::WriteBlobResponse::default(),
        ))
    }
    type ReadBlobStream = tokio_stream::Empty<Result<sagat_api_grpc::pb::ReadBlobResponse, Status>>;
    async fn read_blob(
        &self,
        _: Request<sagat_api_grpc::pb::ReadBlobRequest>,
    ) -> Result<Response<Self::ReadBlobStream>, Status> {
        self.park().await;
        Ok(Response::new(tokio_stream::empty()))
    }

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
    async fn stream_tasks(
        &self,
        _: Request<PollTasksRequest>,
    ) -> Result<Response<Self::StreamTasksStream>, Status> {
        self.park().await;
        Ok(Response::new(tokio_stream::empty()))
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
    async fn append_step_logs(
        &self,
        _: Request<sagat_api_grpc::pb::AppendStepLogsRequest>,
    ) -> Result<Response<sagat_api_grpc::pb::AppendStepLogsResponse>, Status> {
        Err(Status::unimplemented("AppendStepLogs"))
    }
    async fn acknowledge_tasks(
        &self,
        _: Request<sagat_api_grpc::pb::AcknowledgeTasksRequest>,
    ) -> Result<Response<sagat_api_grpc::pb::AcknowledgeTasksResponse>, Status> {
        self.acknowledgements.fetch_add(1, Ordering::SeqCst);
        self.park().await;
        Ok(Response::new(
            sagat_api_grpc::pb::AcknowledgeTasksResponse::default(),
        ))
    }
    async fn resolve_database(
        &self,
        _: Request<ResolveSecretsRequest>,
    ) -> Result<Response<sagat_api_grpc::pb::ResolvedDatabase>, Status> {
        Ok(Response::new(
            sagat_api_grpc::pb::ResolvedDatabase::default(),
        ))
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
    acknowledgements: Arc<AtomicUsize>,
    _stop: oneshot::Sender<()>,
}

async fn serve_stub(config: GrpcConfig) -> Harness {
    serve_stub_with(config, |api| api).await
}

/// [`serve_stub`] with the test seams of `WorkerApi` set by `seams`.
async fn serve_stub_with(
    config: GrpcConfig,
    seams: impl FnOnce(WorkerApi<Stub>) -> WorkerApi<Stub> + Send + 'static,
) -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let gate = Arc::new(Semaphore::new(0));
    let entered = Arc::new(AtomicUsize::new(0));
    let acknowledgements = Arc::new(AtomicUsize::new(0));
    let stub = Stub {
        gate: Arc::clone(&gate),
        entered: Arc::clone(&entered),
        acknowledgements: Arc::clone(&acknowledgements),
    };
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = serve(listener, seams(worker_service(stub)), config, async {
            let _ = stopped.await;
        })
        .await;
    });
    Harness {
        url,
        gate,
        entered,
        acknowledgements,
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

fn acknowledgement() -> sagat_api_grpc::pb::AcknowledgeTasksRequest {
    sagat_api_grpc::pb::AcknowledgeTasksRequest::default()
}

/// Feature 006, Task 006-049 (`stream-completion.md`, "The server's acknowledgement", step 1): with the compiled minor raised
/// to 1.8, four of a connection's 64 places are the acknowledgements': 64 transfers in flight on one connection take 60, and
/// four acknowledgements on that connection are served beside them while the four other transfers wait for a place.
#[tokio::test]
async fn from_1_8_acknowledgements_find_a_place_behind_64_transfers_on_their_connection() {
    let h = serve_stub_with(GrpcConfig::default(), |api| {
        api.with_compiled_minor_for_tests(
            sagat_application::worker::DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR,
        )
    })
    .await;
    let connection = client(&h.url).await;
    let mut calls = tokio::task::JoinSet::new();
    for _ in 0..64 {
        let mut c = connection.clone();
        calls.spawn(async move {
            c.read_blob(sagat_api_grpc::pb::ReadBlobRequest::default())
                .await
                .map(|_| ())
        });
    }
    entered(&h, 60).await;
    for _ in 0..4 {
        let mut c = connection.clone();
        calls.spawn(async move { c.acknowledge_tasks(acknowledgement()).await.map(|_| ()) });
    }
    // the four acknowledgements enter on the places of the reserve while the transfers hold the other 60 (without the reserve
    // the 64 transfers would hold every place and no acknowledgement would enter); the four transfers beyond 60 still wait
    tokio::time::timeout(Duration::from_secs(3), async {
        while h.acknowledgements.load(Ordering::SeqCst) < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("four acknowledgements served beside 60 transfers");
    assert_eq!(
        h.entered.load(Ordering::SeqCst),
        64,
        "60 transfers and 4 acknowledgements"
    );
    h.gate.add_permits(1000);
    entered(&h, 68).await;
    while let Some(call) = calls.join_next().await {
        call.expect("the call's task").expect("served");
    }
}

/// With the minor at 7, a start with the release-0.2 configuration (`concurrency_per_connection` 64) gives every class,
/// `Acknowledge` included, every place of a connection: 64 acknowledgements are in flight on one connection at once.
#[tokio::test]
async fn below_1_8_every_class_has_every_place_of_a_connection() {
    let h = serve_stub(GrpcConfig::default()).await;
    let connection = client(&h.url).await;
    let mut calls = tokio::task::JoinSet::new();
    for _ in 0..64 {
        let mut c = connection.clone();
        calls.spawn(async move { c.acknowledge_tasks(acknowledgement()).await.map(|_| ()) });
    }
    entered(&h, 64).await;
    h.gate.add_permits(1000);
    while let Some(call) = calls.join_next().await {
        call.expect("the call's task").expect("served");
    }
}

/// A connection with more calls than its places and its streams (130 heartbeats parked on one connection) delays nobody
/// else's: an acknowledgement on another connection, as a host of 1.8 sends it, reaches its handler, and the conformance
/// platform's hook sees the two connections apart, by an ordinal and never an address (Task 006-058).
#[tokio::test]
async fn a_full_connection_never_delays_an_acknowledgement_on_another() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::<(String, u64)>::new()));
    let probe_seen = Arc::clone(&seen);
    let h = serve_stub_with(GrpcConfig::default(), move |api| {
        api.with_compiled_minor_for_tests(
            sagat_application::worker::DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR,
        )
        .with_connection_probe(Arc::new(move |method: &str, connection: u64| {
            probe_seen.lock().expect("the probe's record").push((
                method.rsplit('/').next().unwrap_or_default().to_owned(),
                connection,
            ));
        }))
    })
    .await;
    let busy = client(&h.url).await;
    let mut calls = tokio::task::JoinSet::new();
    for _ in 0..130 {
        let mut c = busy.clone();
        calls.spawn(async move { c.heartbeat(HeartbeatRequest::default()).await.map(|_| ()) });
    }
    // 60 places of the busy connection are the heartbeats'; the rest of the 130 wait for a place or a stream
    entered(&h, 60).await;
    let mut acknowledgements = client(&h.url).await;
    let answered = tokio::spawn(async move {
        acknowledgements
            .acknowledge_tasks(acknowledgement())
            .await
            .map(|_| ())
    });
    // it reaches its handler, past every limit, within the acknowledgement's bound (3 s from its arrival)
    tokio::time::timeout(Duration::from_secs(3), entered(&h, 61))
        .await
        .expect("served within the acknowledgement's bound");
    h.gate.add_permits(1000);
    answered.await.expect("the call's task").expect("served");
    while let Some(call) = calls.join_next().await {
        call.expect("the call's task").expect("served");
    }
    let seen = seen.lock().expect("the probe's record").clone();
    let busy_connection: std::collections::BTreeSet<u64> = seen
        .iter()
        .filter(|(method, _)| method == "Heartbeat")
        .map(|&(_, connection)| connection)
        .collect();
    let acknowledgement_connection: std::collections::BTreeSet<u64> = seen
        .iter()
        .filter(|(method, _)| method == "AcknowledgeTasks")
        .map(|&(_, connection)| connection)
        .collect();
    assert_eq!(busy_connection.len(), 1, "{seen:?}");
    assert_eq!(acknowledgement_connection.len(), 1, "{seen:?}");
    assert!(
        busy_connection.is_disjoint(&acknowledgement_connection),
        "{seen:?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|(method, _)| method == "Heartbeat")
            .count(),
        130
    );
}

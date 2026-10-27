#![allow(clippy::expect_used, clippy::panic)]
use crate::{protocol as pb, *};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    sync::{Arc, Mutex},
    time::Duration,
};
use tonic::{Request, Response, Status};
#[derive(Default)]
struct Control {
    forged_register_once: AtomicBool,
    forged_poll_once: AtomicBool,
    oversized_secret: AtomicBool,
    secret_failure_once: AtomicBool,
    hold_secret: AtomicBool,
    secret_entered: tokio::sync::Notify,
    secret_release: tokio::sync::Notify,
    task_kind: AtomicU8,
    accepted_minor: AtomicU8,
    failed_seen: tokio::sync::Notify,
    blob_bytes: Mutex<Vec<u8>>,
    corrupt_blob_read: AtomicBool,
    blob_header: Mutex<Option<pb::WriteBlobHeader>>,
    secret_calls: AtomicUsize,
    poll_calls: AtomicUsize,
    poll_seen: tokio::sync::Notify,
    registrations: AtomicUsize,
    heartbeat_mode: AtomicU8,
    heartbeats: Mutex<Vec<pb::HeartbeatRequest>>,
    failed: Mutex<Vec<pb::FailStepRequest>>,
}
#[derive(Clone)]
struct Service {
    control: Arc<Control>,
    leases: Arc<Mutex<VecDeque<pb::TaskLease>>>,
    reports: tokio::sync::mpsc::UnboundedSender<pb::CompleteStepRequest>,
    registrations: Arc<Mutex<Vec<pb::RegisterWorkerRequest>>>,
}
#[tonic::async_trait]
impl pb::worker_service_server::WorkerService for Service {
    async fn register_worker(
        &self,
        r: Request<pb::RegisterWorkerRequest>,
    ) -> Result<Response<pb::RegisterWorkerResponse>, Status> {
        self.control.registrations.fetch_add(1, Ordering::AcqRel);
        if self
            .control
            .forged_register_once
            .swap(false, Ordering::AcqRel)
        {
            return Err(forged_provider_error());
        }
        assert!(r.metadata().contains_key("authorization"));
        self.registrations
            .lock()
            .expect("registrations")
            .push(r.into_inner());
        Ok(Response::new(pb::RegisterWorkerResponse {
            worker_id: "worker-1".into(),
            session_token: "private-session".into(),
            accepted_protocol_version: format!(
                "1.{}",
                match self.control.accepted_minor.load(Ordering::Acquire) {
                    0 => 5,
                    minor => minor,
                }
            ),
        }))
    }
    async fn poll_tasks(
        &self,
        r: Request<pb::PollTasksRequest>,
    ) -> Result<Response<pb::PollTasksResponse>, Status> {
        self.control.poll_calls.fetch_add(1, Ordering::AcqRel);
        self.control.poll_seen.notify_one();
        if self.control.forged_poll_once.swap(false, Ordering::AcqRel) {
            return Err(forged_provider_error());
        }
        let credit = r.into_inner().max_tasks;
        let mut leases = self.leases.lock().expect("leases");
        let tasks = (0..credit)
            .filter_map(|_| leases.pop_front())
            .map(|mut task| {
                match self.control.task_kind.load(Ordering::Acquire) {
                    1 => task.task_kind = "COMPENSATION".into(),
                    2 => task.task_kind = "RECONCILIATION".into(),
                    _ => {}
                }
                if !task.task_kind.is_empty() {
                    task.target_node = "forward".into();
                    task.target_idempotency_key = "original-effect-key".into();
                }
                task
            })
            .collect();
        Ok(Response::new(pb::PollTasksResponse {
            tasks,
            retry_after_ms: 1000,
        }))
    }
    type StreamTasksStream = tokio_stream::Empty<Result<pb::PollTasksResponse, Status>>;
    async fn stream_tasks(
        &self,
        _: Request<pb::PollTasksRequest>,
    ) -> Result<Response<Self::StreamTasksStream>, Status> {
        Err(Status::unimplemented("stream unavailable"))
    }
    async fn heartbeat(
        &self,
        request: Request<pb::HeartbeatRequest>,
    ) -> Result<Response<pb::HeartbeatResponse>, Status> {
        self.control
            .heartbeats
            .lock()
            .expect("beats")
            .push(request.into_inner());
        let mode = self.control.heartbeat_mode.load(Ordering::Acquire);
        if mode == 1 {
            return Err(Status::failed_precondition("stale"));
        }
        Ok(Response::new(pb::HeartbeatResponse {
            lease_expires_at_epoch_ms: 0,
            cancel_requested: mode == 2,
        }))
    }
    async fn complete_step(
        &self,
        r: Request<pb::CompleteStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        self.reports.send(r.into_inner()).expect("receiver");
        Ok(Response::new(pb::StepCommandResponse {
            already_applied: false,
            ..Default::default()
        }))
    }
    async fn fail_step(
        &self,
        request: Request<pb::FailStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        self.control
            .failed
            .lock()
            .expect("failure")
            .push(request.into_inner());
        self.control.failed_seen.notify_one();
        Ok(Response::new(pb::StepCommandResponse::default()))
    }
    async fn resolve_secrets(
        &self,
        _: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolveSecretsResponse>, Status> {
        self.control.secret_calls.fetch_add(1, Ordering::AcqRel);
        if self
            .control
            .secret_failure_once
            .swap(false, Ordering::AcqRel)
        {
            return Err(Status::unavailable("provider unavailable"));
        }
        if self.control.hold_secret.load(Ordering::Acquire) {
            self.control.secret_entered.notify_one();
            self.control.secret_release.notified().await;
        }
        if self.control.oversized_secret.load(Ordering::Acquire) {
            return Ok(Response::new(pb::ResolveSecretsResponse {
                secrets: vec![pb::ResolvedSecret {
                    name: "database".into(),
                    value: vec![1; 600 * 1024],
                    version: 1,
                }],
            }));
        }
        Ok(Response::new(pb::ResolveSecretsResponse {
            secrets: vec![pb::ResolvedSecret {
                name: "database".into(),
                value: b"private-db-password".to_vec(),
                version: 1,
            }],
        }))
    }
    async fn resolve_database(
        &self,
        _: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolvedDatabase>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn write_blob(
        &self,
        request: Request<tonic::Streaming<pb::WriteBlobRequest>>,
    ) -> Result<Response<pb::WriteBlobResponse>, Status> {
        use sha2::Digest as _;
        let mut stream = request.into_inner();
        let first = stream
            .message()
            .await?
            .ok_or_else(|| Status::invalid_argument("header"))?;
        let Some(pb::write_blob_request::Part::Header(header)) = first.part else {
            return Err(Status::invalid_argument("header"));
        };
        let mut bytes = Vec::new();
        while let Some(frame) = stream.message().await? {
            let Some(pb::write_blob_request::Part::Chunk(chunk)) = frame.part else {
                return Err(Status::invalid_argument("chunk"));
            };
            assert!(chunk.len() <= 256 * 1024);
            bytes.extend_from_slice(&chunk);
        }
        *self.control.blob_header.lock().expect("header") = Some(header);
        *self.control.blob_bytes.lock().expect("bytes") = bytes.clone();
        Ok(Response::new(pb::WriteBlobResponse {
            blob_id: "019b5d6e-0000-7000-8000-000000000001".into(),
            size: bytes.len() as i64,
            sha256: format!("{:x}", sha2::Sha256::digest(&bytes)),
            content_type: "application/octet-stream".into(),
        }))
    }
    type ReadBlobStream = std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<pb::ReadBlobResponse, Status>> + Send>,
    >;
    async fn read_blob(
        &self,
        request: Request<pb::ReadBlobRequest>,
    ) -> Result<Response<Self::ReadBlobStream>, Status> {
        use sha2::Digest as _;
        let request = request.into_inner();
        assert_eq!(
            request.instance.as_ref().expect("scope").instance_path[0].iteration_index,
            3
        );
        let mut bytes = self.control.blob_bytes.lock().expect("bytes").clone();
        let header = pb::ReadBlobHeader {
            blob_id: request.blob_id,
            size: bytes.len() as i64,
            sha256: format!("{:x}", sha2::Sha256::digest(&bytes)),
            content_type: "application/octet-stream".into(),
        };
        if self.control.corrupt_blob_read.load(Ordering::Acquire)
            && let Some(first) = bytes.first_mut()
        {
            *first ^= 1;
        }
        let mut frames = vec![Ok(pb::ReadBlobResponse {
            part: Some(pb::read_blob_response::Part::Header(header)),
        })];
        for chunk in bytes.chunks(256 * 1024) {
            frames.push(Ok(pb::ReadBlobResponse {
                part: Some(pb::read_blob_response::Part::Chunk(chunk.to_vec())),
            }));
        }
        Ok(Response::new(Box::pin(futures_util::stream::iter(frames))))
    }
}
#[tokio::test]
async fn a_native_worker_registers_runs_and_reports_the_exact_scoped_lease() {
    let instance = pb::ExecutionInstanceIdentity {
        instance_path: vec![pb::ScopePathSegment {
            scope_node_id: "lines".into(),
            iteration_index: 3,
        }],
        definition_node_id: "send".into(),
    };
    let lease = pb::TaskLease {
        task_id: "task-1".into(),
        run_id: "run-1".into(),
        step_id: "step-1".into(),
        activity_name: "echo".into(),
        attempt: 2,
        generation: 7,
        lease_token: "private-lease".into(),
        expires_at_epoch_ms: (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
            as i64
            + 30_000,
        input_json: r#"{"node":"send","input":{"exact":9007199254740993},"steps":{}}"#.into(),
        instance: Some(instance.clone()),
        traceparent: "00-0123456789abcdef0123456789abcdef-0123456789abcdef-01".into(),
        ..Default::default()
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let registrations = Arc::new(Mutex::new(Vec::new()));
    let service = Service {
        control: Arc::default(),
        leases: Arc::new(Mutex::new(VecDeque::from([lease]))),
        reports: tx,
        registrations: registrations.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let endpoint = format!("http://{}", listener.local_addr().expect("address"));
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(pb::worker_service_server::WorkerServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
    );
    let options = WorkerOptions::new(
        endpoint,
        "demo",
        "rust-worker",
        TokenSource::bearer("fixture-token"),
    );
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("echo", |ctx: ActivityContext, input: Value| async move {
        assert_eq!(ctx.task.node, "send");
        assert_eq!(
            ctx.task.instance.as_ref().expect("scope").instance_path[0].iteration_index,
            3
        );
        Ok(input)
    })
    .expect("activity");
    host.start().await.expect("start");
    let result = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("report timeout")
        .expect("report");
    assert_eq!(result.task_id, "task-1");
    assert_eq!(result.generation, 7);
    assert_eq!(result.attempt, 2);
    assert_eq!(result.lease_token, "private-lease");
    assert_eq!(result.instance, Some(instance));
    assert_eq!(result.idempotency_key, "done-task-1");
    assert_eq!(
        serde_json::from_str::<Value>(&result.output_json).expect("output")["exact"].as_u64(),
        Some(9007199254740993)
    );
    assert_eq!(host.protocol_version(), "1.5");
    host.stop(Duration::from_secs(1)).await.expect("stop");
    assert_eq!(
        registrations.lock().expect("registration")[0]
            .capabilities
            .as_ref()
            .expect("caps")
            .languages,
        ["rust"]
    );
    server.abort();
}

struct Fixture {
    options: WorkerOptions,
    complete: tokio::sync::mpsc::UnboundedReceiver<pb::CompleteStepRequest>,
    control: Arc<Control>,
    server: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn fixture(mode: u8) -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let options = WorkerOptions::new(
        format!("http://{}", listener.local_addr().expect("address")),
        "demo",
        "rust-fault",
        TokenSource::bearer("fixture-token"),
    );
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let control = Arc::new(Control::default());
    control.heartbeat_mode.store(mode, Ordering::Release);
    let lease = pb::TaskLease {
        task_id: "task-held".into(),
        run_id: "run-held".into(),
        step_id: "step-held".into(),
        activity_name: "held".into(),
        attempt: 1,
        generation: 1,
        lease_token: "private-lease".into(),
        expires_at_epoch_ms: -1,
        input_json: r#"{"node":"held","input":{}}"#.into(),
        instance: Some(pb::ExecutionInstanceIdentity {
            definition_node_id: "held".into(),
            instance_path: vec![pb::ScopePathSegment {
                scope_node_id: "lines".into(),
                iteration_index: 3,
            }],
        }),
        ..Default::default()
    };
    let service = Service {
        control: control.clone(),
        leases: Arc::new(Mutex::new(VecDeque::from([lease]))),
        reports: tx,
        registrations: Arc::default(),
    };
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(pb::worker_service_server::WorkerServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
    );
    Fixture {
        options,
        complete: rx,
        control,
        server,
    }
}
#[tokio::test]
async fn server_staleness_cancels_the_handler_and_drops_its_eventual_result() {
    let mut fixture = fixture(1).await;
    fixture.options.heartbeat_interval = Some(Duration::from_millis(1));
    let mut host = WorkerHost::new(fixture.options.clone()).expect("host");
    let (finished, done) = tokio::sync::oneshot::channel();
    let finished = Arc::new(Mutex::new(Some(finished)));
    host.activity("held", move |ctx: ActivityContext, _: Value| {
        let finished = finished.clone();
        async move {
            ctx.cancellation.cancelled().await;
            assert_eq!(
                ctx.cancellation_reason(),
                Some(CancellationReason::LeaseLost)
            );
            if let Some(tx) = finished.lock().expect("sender").take() {
                tx.send(()).expect("observer");
            }
            Ok(json!({"should":"drop"}))
        }
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(2), done)
        .await
        .expect("heartbeat cancellation")
        .expect("handler");
    host.stop(Duration::from_secs(1)).await.expect("drain");
    assert!(fixture.complete.try_recv().is_err());
    assert!(fixture.control.failed.lock().expect("failures").is_empty());
    assert!(!fixture.control.heartbeats.lock().expect("beats").is_empty());
}
#[tokio::test]
async fn run_cancellation_is_cooperative_and_does_not_discard_a_returned_value() {
    let mut fixture = fixture(2).await;
    fixture.options.heartbeat_interval = Some(Duration::from_millis(1));
    let mut host = WorkerHost::new(fixture.options.clone()).expect("host");
    host.activity("held", |ctx: ActivityContext, _: Value| async move {
        ctx.cancellation.cancelled().await;
        assert_eq!(
            ctx.cancellation_reason(),
            Some(CancellationReason::RunCancelled)
        );
        assert_eq!(
            ctx.deadline_epoch_ms(),
            -1,
            "zero heartbeat expiry is ignored"
        );
        Ok(json!({"completed":true}))
    })
    .expect("handler");
    host.start().await.expect("start");
    let report = tokio::time::timeout(Duration::from_secs(2), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    assert_eq!(report.output_json, r#"{"completed":true}"#);
    host.stop(Duration::from_secs(1)).await.expect("drain");
}
#[tokio::test]
async fn shutdown_cancellation_is_not_reported_as_a_permanent_activity_failure() {
    let mut fixture = fixture(0).await;
    let mut host = WorkerHost::new(fixture.options.clone()).expect("host");
    let started = Arc::new(tokio::sync::Notify::new());
    let entered = started.clone();
    host.activity("held", move |ctx: ActivityContext, _: Value| {
        let entered = entered.clone();
        async move {
            entered.notify_one();
            ctx.cancellation.cancelled().await;
            assert_eq!(
                ctx.cancellation_reason(),
                Some(CancellationReason::Shutdown)
            );
            Err::<Value, _>(ActivityFailure::cancelled())
        }
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .expect("handler entered");
    let pending = host.running.as_ref().expect("running").tracker.clone();
    assert_eq!(
        host.stop(Duration::ZERO)
            .await
            .expect_err("shutdown grace")
            .code,
        "CANCELLED"
    );
    pending.close();
    tokio::time::timeout(Duration::from_secs(1), pending.wait())
        .await
        .expect("all handler cleanup finished");
    assert!(fixture.complete.try_recv().is_err());
    assert!(fixture.control.failed.lock().expect("failures").is_empty());
}

#[tokio::test]
async fn secrets_resolve_once_concurrently_and_retained_views_cannot_read_after_return() {
    let mut fixture = fixture(0).await;
    let mut host = WorkerHost::new(fixture.options.clone()).expect("host");
    let retained = Arc::new(Mutex::new(None));
    let retained_context = Arc::new(Mutex::new(None));
    let save_context = retained_context.clone();
    let saved = retained.clone();
    host.activity("held", move |ctx: ActivityContext, _: Value| {
        let saved = saved.clone();
        let save_context = save_context.clone();
        async move {
            *save_context.lock().expect("context") = Some(ctx.clone());
            let (one, two) = tokio::join!(ctx.secrets(), ctx.secrets());
            let one = one.expect("first resolution");
            let two = two.expect("same resolution");
            let value = one.get("database").expect("active").expect("secret");
            assert_eq!(value.as_bytes(), b"private-db-password");
            assert!(!format!("{value:?} {one:?}").contains("private-db-password"));
            assert_eq!(two.names().expect("names"), ["database"]);
            *saved.lock().expect("saved") = Some(one);
            Ok(json!({"used":true}))
        }
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(2), fixture.complete.recv())
        .await
        .expect("reported")
        .expect("complete");
    host.stop(Duration::from_secs(1)).await.expect("drain");
    assert_eq!(fixture.control.secret_calls.load(Ordering::Acquire), 1);
    assert!(
        !retained_context
            .lock()
            .expect("context")
            .as_ref()
            .expect("saved context")
            .capabilities
            .as_ref()
            .expect("capability")
            .authorization_retained()
    );
    assert_eq!(
        retained
            .lock()
            .expect("view")
            .as_ref()
            .expect("saved")
            .get("database")
            .expect_err("ended attempt")
            .code,
        "VALIDATION_FAILED"
    );
}

#[tokio::test]
async fn an_explicit_unsupported_stream_surfaces_a_terminal_worker_error() {
    let fixture = fixture(0).await;
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Stream;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", |_: ActivityContext, _: Value| async { Ok(()) })
        .expect("activity");
    host.start().await.expect("registered");
    let failure = tokio::time::timeout(Duration::from_secs(1), host.wait_for_failure())
        .await
        .expect("fatal error surfaced");
    assert_eq!(failure.code, "UNSUPPORTED_PROTOCOL");
    host.stop(Duration::from_secs(1)).await.expect("stop");
}
#[tokio::test]
async fn shutdown_grace_does_not_add_an_unrequested_two_second_delay() {
    let fixture = fixture(0).await;
    let mut host = WorkerHost::new(fixture.options.clone()).expect("host");
    let entered = Arc::new(tokio::sync::Notify::new());
    let started = entered.clone();
    let release = Arc::new(tokio::sync::Notify::new());
    let held = release.clone();
    host.activity("held", move |_: ActivityContext, _: Value| {
        let started = started.clone();
        let held = held.clone();
        async move {
            started.notify_one();
            held.notified().await;
            Ok(())
        }
    })
    .expect("activity");
    host.start().await.expect("registered");
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .expect("handler entered");
    let pending = host.running.as_ref().expect("running").tracker.clone();
    let result = tokio::time::timeout(Duration::from_millis(100), host.stop(Duration::ZERO)).await;
    release.notify_one();
    pending.close();
    tokio::time::timeout(Duration::from_secs(1), pending.wait())
        .await
        .expect("late handler finished");
    assert!(
        result.is_ok(),
        "zero grace must not wait an additional two seconds"
    );
}

#[tokio::test]
async fn a_local_receive_token_failure_retries_without_replacing_a_valid_session() {
    let fixture = fixture(0).await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = attempts.clone();
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    options.heartbeat_interval = Some(Duration::from_secs(10));
    options.retry.initial_delay = Duration::from_millis(1);
    options.token = TokenSource::dynamic(move || {
        let n = counter.fetch_add(1, Ordering::AcqRel);
        async move {
            if n == 2 {
                Err(TokenProviderError)
            } else {
                Ok("fixture-token".to_owned())
            }
        }
    });
    let mut host = WorkerHost::new(options).expect("host");
    let release = Arc::new(tokio::sync::Notify::new());
    let held = release.clone();
    host.activity("held", move |_: ActivityContext, _: Value| {
        let held = held.clone();
        async move {
            held.notified().await;
            Ok(())
        }
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.control.poll_calls.load(Ordering::Acquire) < 2 {
            fixture.control.poll_seen.notified().await;
        }
    })
    .await
    .expect("receive recovered");
    release.notify_one();
    host.stop(Duration::from_secs(1)).await.expect("drain");
    assert_eq!(
        fixture.control.registrations.load(Ordering::Acquire),
        1,
        "a local provider failure is not a server session refusal"
    );
}

fn forged_provider_error() -> Status {
    let mut error = Status::unauthenticated("server credentials refused");
    error.metadata_mut().insert(
        "x-sagat-error",
        "TOKEN_PROVIDER_FAILED".parse().expect("metadata"),
    );
    error
}
#[tokio::test]
async fn server_reasons_cannot_spoof_local_token_provenance_during_refresh_or_recovery() {
    let mut fixture = fixture(0).await;
    fixture
        .control
        .forged_register_once
        .store(true, Ordering::Release);
    fixture
        .control
        .forged_poll_once
        .store(true, Ordering::Release);
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    options.token = TokenSource::dynamic(|| async { Ok("fixture-token".to_owned()) });
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", |_: ActivityContext, _: Value| async { Ok(()) })
        .expect("activity");
    host.start()
        .await
        .expect("a server refusal refreshes dynamic credentials once");
    tokio::select! {
        result=tokio::time::timeout(Duration::from_secs(3),fixture.complete.recv())=>{result.expect("recovered receive").expect("report");},
        error=host.wait_for_failure()=>{panic!("server-controlled reason must not end recovery: {error:?}");},
    }
    host.stop(Duration::from_secs(1)).await.expect("drain");
    assert_eq!(fixture.control.registrations.load(Ordering::Acquire), 3);
}

#[tokio::test]
async fn failed_secret_resolution_is_not_cached_for_the_rest_of_the_attempt() {
    let mut fixture = fixture(0).await;
    fixture
        .control
        .secret_failure_once
        .store(true, Ordering::Release);
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", |ctx: ActivityContext, _: Value| async move {
        assert_eq!(
            ctx.secrets()
                .await
                .expect_err("first resolution fails")
                .code,
            "TRANSPORT"
        );
        assert_eq!(
            ctx.secrets()
                .await
                .expect("new request")
                .names()
                .expect("names"),
            ["database"]
        );
        ctx.secrets().await.expect("cached success");
        Ok(())
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    host.stop(Duration::from_secs(1)).await.expect("drain");
    assert_eq!(fixture.control.secret_calls.load(Ordering::Acquire), 2);
}
#[tokio::test]
async fn handler_return_revokes_a_secret_rpc_already_in_flight() {
    let mut fixture = fixture(0).await;
    fixture.control.hold_secret.store(true, Ordering::Release);
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    let pending = Arc::new(Mutex::new(None));
    let captured = pending.clone();
    let control = fixture.control.clone();
    host.activity("held", move |ctx: ActivityContext, _: Value| {
        let captured = captured.clone();
        let control = control.clone();
        async move {
            let resolver = tokio::spawn(async move { ctx.secrets().await });
            control.secret_entered.notified().await;
            *captured.lock().expect("resolver") = Some(resolver);
            Ok(())
        }
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    fixture.control.secret_release.notify_one();
    let resolver = pending
        .lock()
        .expect("resolver")
        .take()
        .expect("saved future");
    let result = tokio::time::timeout(Duration::from_secs(1), resolver)
        .await
        .expect("revoked RPC")
        .expect("task");
    assert_eq!(
        result
            .expect_err("late secrets cannot reach application")
            .code,
        "VALIDATION_FAILED"
    );
    host.stop(Duration::from_secs(1)).await.expect("drain");
}

#[tokio::test]
async fn secret_responses_have_a_smaller_decode_budget_than_task_inputs() {
    let mut fixture = fixture(0).await;
    fixture
        .control
        .oversized_secret
        .store(true, Ordering::Release);
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", |ctx: ActivityContext, _: Value| async move {
        let error = ctx.secrets().await.expect_err("transport decode budget");
        assert_eq!(error.code, "PROTOCOL");
        assert!(!error.retryable);
        assert_eq!(
            error.grpc_status(),
            Some("OUT_OF_RANGE"),
            "the transport refuses the frame before semantic cache validation"
        );
        Ok(())
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    host.stop(Duration::from_secs(1)).await.expect("drain");
}

#[tokio::test]
async fn worker_blob_output_is_a_typed_reference_with_the_original_scoped_capability() {
    let mut fixture = fixture(0).await;
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", |ctx: ActivityContext, _: Value| async move {
        let output = ctx
            .write_blob(&b"worker-blob"[..], WriteBlobOptions::default())
            .await?;
        assert_eq!(output.descriptor().size, 11);
        Ok(output)
    })
    .expect("handler");
    host.start().await.expect("start");
    let complete = tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    assert!(complete.output_json.is_empty());
    assert_eq!(
        complete.output_blob_ref,
        "019b5d6e-0000-7000-8000-000000000001"
    );
    assert!(!complete.output_sha256.is_empty());
    host.stop(Duration::from_secs(1)).await.expect("drain");
    let header = fixture.control.blob_header.lock().expect("header");
    let header = header.as_ref().expect("write header");
    assert_eq!(header.task_id, "task-held");
    assert_eq!(header.lease_token, "private-lease");
    assert_eq!(header.generation, 1);
    assert_eq!(
        header
            .instance
            .as_ref()
            .expect("original scope")
            .instance_path[0]
            .iteration_index,
        3
    );
}

#[tokio::test]
async fn a_json_blob_lookalike_is_reported_as_inline_json() {
    let mut fixture = fixture(0).await;
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held",|_:ActivityContext,_:Value|async {Ok(json!({"$blob":{"id":"019b5d6e-0000-7000-8000-000000000001","size":11,"sha256":"not-an-authority","contentType":"application/octet-stream"}}))}).expect("handler");
    host.start().await.expect("start");
    let complete = tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    assert!(complete.output_blob_ref.is_empty());
    assert!(complete.output_sha256.is_empty());
    assert!(
        serde_json::from_str::<Value>(&complete.output_json)
            .expect("inline")
            .get("$blob")
            .is_some()
    );
    host.stop(Duration::from_secs(1)).await.expect("drain");
    assert!(
        fixture
            .control
            .blob_header
            .lock()
            .expect("header")
            .is_none()
    );
}
#[tokio::test]
async fn worker_blob_calls_are_revoked_while_waiting_for_fresh_credentials() {
    let mut fixture = fixture(0).await;
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    options.max_inflight = 1;
    options.heartbeat_interval = Some(Duration::from_secs(10));
    let entered = Arc::new(tokio::sync::Notify::new());
    let observed = entered.clone();
    let tokens = Arc::new(AtomicUsize::new(0));
    options.token = TokenSource::dynamic(move || {
        let number = tokens.fetch_add(1, Ordering::AcqRel);
        let entered = entered.clone();
        async move {
            if number == 2 {
                entered.notify_one();
                std::future::pending::<()>().await;
            }
            Ok("fixture-token".into())
        }
    });
    let pending = Arc::new(Mutex::new(None));
    let captured = pending.clone();
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", move |ctx: ActivityContext, _: Value| {
        let observed = observed.clone();
        let captured = captured.clone();
        async move {
            let upload = tokio::spawn(async move {
                ctx.write_blob(&b"worker-blob"[..], WriteBlobOptions::default())
                    .await
            });
            observed.notified().await;
            *captured.lock().expect("upload") = Some(upload);
            Ok(())
        }
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    let upload = pending.lock().expect("upload").take().expect("saved");
    let error = tokio::time::timeout(Duration::from_secs(1), upload)
        .await
        .expect("revoked")
        .expect("task")
        .expect_err("attempt ended");
    assert_eq!(error.code, "VALIDATION_FAILED");
    assert!(
        fixture
            .control
            .blob_header
            .lock()
            .expect("header")
            .is_none()
    );
    host.stop(Duration::from_secs(1)).await.expect("drain");
}

#[tokio::test]
async fn a_retained_blob_marker_is_refused_when_the_session_only_speaks_protocol_1_2() {
    let mut fixture = fixture(0).await;
    fixture.control.accepted_minor.store(2, Ordering::Release);
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let marker = BlobOutput(
        BlobDescriptor::new(
            uuid::Uuid::parse_str("019b5d6e-0000-7000-8000-000000000001").expect("id"),
            1,
            "a".repeat(64),
            "application/octet-stream",
        )
        .expect("retained descriptor"),
    );
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", move |_: ActivityContext, _: Value| {
        let marker = marker.clone();
        async move { Ok(marker) }
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::select! {_=fixture.control.failed_seen.notified()=>{},result=fixture.complete.recv()=>{assert!(result.is_none(),"a legacy session must not complete with a blob reference");}}
    host.stop(Duration::from_secs(1)).await.expect("drain");
    let failed = fixture.control.failed.lock().expect("failed");
    assert_eq!(failed[0].error_code, "UNSUPPORTED_PROTOCOL");
    assert!(!failed[0].retryable);
}

#[tokio::test]
async fn worker_blob_reads_verify_the_header_bytes_digest_and_clean_eof() {
    use futures_util::StreamExt as _;
    let mut fixture = fixture(0).await;
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", |ctx: ActivityContext, _: Value| async move {
        let blob = ctx
            .write_blob(&b"worker-blob"[..], WriteBlobOptions::default())
            .await?;
        let mut reader = ctx.read_blob(blob.descriptor()).await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = reader.next().await {
            bytes.extend_from_slice(&chunk?);
        }
        assert!(reader.is_verified());
        assert_eq!(bytes, b"worker-blob");
        Ok(blob)
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    host.stop(Duration::from_secs(1)).await.expect("drain");
}

#[tokio::test]
async fn corrupt_blob_bytes_fail_before_a_verified_eof() {
    use futures_util::StreamExt as _;
    let mut fixture = fixture(0).await;
    fixture
        .control
        .corrupt_blob_read
        .store(true, Ordering::Release);
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", |ctx: ActivityContext, _: Value| async move {
        let blob = ctx
            .write_blob(&b"worker-blob"[..], WriteBlobOptions::default())
            .await?;
        let mut reader = ctx.read_blob(blob.descriptor()).await?;
        let mut failure = None;
        while let Some(chunk) = reader.next().await {
            if let Err(error) = chunk {
                failure = Some(error);
                break;
            }
        }
        assert_eq!(failure.expect("digest failure").code, "PROTOCOL");
        assert!(!reader.is_verified());
        Ok(())
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    host.stop(Duration::from_secs(1)).await.expect("drain");
}
#[tokio::test]
async fn a_retained_blob_reader_cannot_yield_data_after_the_handler_returns() {
    use futures_util::StreamExt as _;
    let mut fixture = fixture(0).await;
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let retained = Arc::new(Mutex::new(None));
    let saved = retained.clone();
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", move |ctx: ActivityContext, _: Value| {
        let saved = saved.clone();
        async move {
            let blob = ctx
                .write_blob(&b"worker-blob"[..], WriteBlobOptions::default())
                .await?;
            let reader = ctx.read_blob(blob.descriptor()).await?;
            *saved.lock().expect("reader") = Some(reader);
            Ok(())
        }
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    host.stop(Duration::from_secs(1)).await.expect("drain");
    let mut reader = retained.lock().expect("reader").take().expect("retained");
    assert_eq!(
        reader
            .next()
            .await
            .expect("revocation")
            .expect_err("attempt ended")
            .code,
        "VALIDATION_FAILED"
    );
    assert!(!reader.is_verified());
}
#[tokio::test]
async fn a_reconciler_returns_the_canonical_applied_envelope() {
    let mut fixture = fixture(0).await;
    fixture.control.task_kind.store(2, Ordering::Release);
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.reconciler("held", |ctx: ActivityContext, _: Value| async move {
        assert_eq!(ctx.task.task_kind, TaskKind::Reconciliation);
        assert_eq!(ctx.task.target_node, "forward");
        assert_eq!(ctx.task.target_idempotency_key, "original-effect-key");
        Ok(Verdict::applied(Some(json!({"receipt":1}))))
    })
    .expect("reconciler");
    host.start().await.expect("start");
    let result = tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    assert_eq!(
        serde_json::from_str::<Value>(&result.output_json).expect("verdict"),
        json!({"effect":"applied","output":{"receipt":1}})
    );
    host.stop(Duration::from_secs(1)).await.expect("drain");
}

#[tokio::test]
async fn reconciliation_does_not_accept_an_untyped_json_lookalike() {
    let mut fixture = fixture(0).await;
    fixture.control.task_kind.store(2, Ordering::Release);
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", |_: ActivityContext, _: Value| async {
        Ok(json!({"effect":"applied","output":1}))
    })
    .expect("handler");
    host.start().await.expect("start");
    tokio::time::timeout(
        Duration::from_secs(3),
        fixture.control.failed_seen.notified(),
    )
    .await
    .expect("failure");
    host.stop(Duration::from_secs(1)).await.expect("drain");
    let failures = fixture.control.failed.lock().expect("failures");
    assert_eq!(failures[0].error_code, "RECONCILIATION_ANSWER_INVALID");
    assert!(!failures[0].retryable);
    assert!(fixture.complete.try_recv().is_err());
}
#[tokio::test]
async fn compensation_preserves_its_forward_target_and_uses_the_same_lease_protocol() {
    let mut fixture = fixture(0).await;
    fixture.control.task_kind.store(1, Ordering::Release);
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.compensation("held", |ctx: ActivityContext, _: Value| async move {
        assert_eq!(ctx.task.task_kind, TaskKind::Compensation);
        assert_eq!(ctx.task.target_node, "forward");
        assert_eq!(ctx.task.target_idempotency_key, "original-effect-key");
        Ok(())
    })
    .expect("handler");
    host.start().await.expect("start");
    let result = tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    assert_eq!(result.task_id, "task-held");
    assert!(result.output_json.is_empty());
    host.stop(Duration::from_secs(1)).await.expect("drain");
}

#[tokio::test]
async fn a_panicking_metrics_observer_does_not_drop_a_leased_activity() {
    let mut fixture = fixture(0).await;
    let mut options = fixture.options.clone();
    options.transport = WorkerTransport::Poll;
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    options.telemetry.on_event = Some(Arc::new(move |_| {
        observed.fetch_add(1, Ordering::AcqRel);
        panic!("observer failure");
    }));
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("held", |_: ActivityContext, _: Value| async {
        Ok(json!({"done":true}))
    })
    .expect("handler");
    host.start().await.expect("start survives observer");
    let complete = tokio::time::timeout(Duration::from_secs(3), fixture.complete.recv())
        .await
        .expect("report")
        .expect("complete");
    assert_eq!(
        serde_json::from_str::<Value>(&complete.output_json).expect("output"),
        json!({"done":true})
    );
    host.stop(Duration::from_secs(1)).await.expect("drain");
    assert!(count.load(Ordering::Acquire) >= 4);
}

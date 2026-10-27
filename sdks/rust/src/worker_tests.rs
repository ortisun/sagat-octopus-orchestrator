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
            accepted_protocol_version: "1.5".into(),
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
        Ok(Response::new(pb::PollTasksResponse {
            tasks: (0..credit).filter_map(|_| leases.pop_front()).collect(),
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
        _: Request<tonic::Streaming<pb::WriteBlobRequest>>,
    ) -> Result<Response<pb::WriteBlobResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
    type ReadBlobStream = tokio_stream::Empty<Result<pb::ReadBlobResponse, Status>>;
    async fn read_blob(
        &self,
        _: Request<pb::ReadBlobRequest>,
    ) -> Result<Response<Self::ReadBlobStream>, Status> {
        Err(Status::unimplemented("unused"))
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

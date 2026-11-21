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
    scoped_twins: AtomicBool,
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
    // A poll answers every lease it holds, whatever the credit asked for.
    overfill: AtomicBool,
    // StreamTasks is served (otherwise it is UNIMPLEMENTED): each call sends the frames queued
    // then, and then keeps the stream open until the client ends it.
    streaming: AtomicBool,
    stream_frames: Mutex<VecDeque<Vec<pb::TaskLease>>>,
    // The answers of the next FailStep calls, in order, before they are accepted.
    fail_step_errors: Mutex<VecDeque<Status>>,
    // FailStep is held (after it was recorded) until `fail_step_release` is cancelled, and the
    // calls held now and at most are counted.
    hold_fail_step: AtomicBool,
    fail_step_release: CancellationToken,
    fail_steps_held: AtomicUsize,
    fail_step_peak: AtomicUsize,
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
        let credit = if self.control.overfill.load(Ordering::Acquire) {
            i32::MAX
        } else {
            r.into_inner().max_tasks
        };
        let mut leases = self.leases.lock().expect("leases");
        if self.control.scoped_twins.swap(false, Ordering::AcqRel)
            && let Some(task) = leases.front().cloned()
        {
            leases.push_back(task.clone());
            let mut other = task;
            other.instance.as_mut().expect("scope").instance_path[0].iteration_index = 4;
            leases.push_back(other);
        }
        let tasks = (0..credit)
            .map_while(|_| leases.pop_front())
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
    type StreamTasksStream = std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<pb::PollTasksResponse, Status>> + Send>,
    >;
    async fn stream_tasks(
        &self,
        _: Request<pb::PollTasksRequest>,
    ) -> Result<Response<Self::StreamTasksStream>, Status> {
        use futures_util::StreamExt as _;
        if !self.control.streaming.load(Ordering::Acquire) {
            return Err(Status::unimplemented("stream unavailable"));
        }
        let frames: Vec<_> = self
            .control
            .stream_frames
            .lock()
            .expect("frames")
            .drain(..)
            .map(|tasks| {
                Ok(pb::PollTasksResponse {
                    tasks,
                    retry_after_ms: 0,
                })
            })
            .collect();
        Ok(Response::new(Box::pin(
            futures_util::stream::iter(frames).chain(futures_util::stream::pending()),
        )))
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
        if self.control.hold_fail_step.load(Ordering::Acquire) {
            // Counted down when the call ends, also when the client gives it up and the server
            // drops it.
            struct Held<'a>(&'a AtomicUsize);
            impl Drop for Held<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let held = self.control.fail_steps_held.fetch_add(1, Ordering::AcqRel) + 1;
            let _held = Held(&self.control.fail_steps_held);
            self.control
                .fail_step_peak
                .fetch_max(held, Ordering::AcqRel);
            self.control.fail_step_release.cancelled().await;
        }
        if let Some(status) = self
            .control
            .fail_step_errors
            .lock()
            .expect("failure script")
            .pop_front()
        {
            return Err(status);
        }
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
        input_json: r#"{"node":"send","input":{"exact":9007199254740993},"steps":{"before":{"output":{"output":"business-field"}},"explicit-null":{"output":null},"no-output":{"status":"SUCCEEDED"},"malformed":false}}"#.into(),
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
        Ok(json!({"exact":input["exact"],"retained":ctx.task.steps}))
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
    assert_eq!(
        serde_json::from_str::<Value>(&result.output_json).expect("output")["retained"],
        json!({"before":{"output":"business-field"},"explicit-null":null})
    );
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
async fn concurrent_scopes_with_the_same_legacy_task_id_are_distinct_but_duplicates_are_dropped() {
    let mut fixture = fixture(0).await;
    fixture.options.max_inflight = 3;
    fixture.control.scoped_twins.store(true, Ordering::Release);
    let mut host = WorkerHost::new(fixture.options.clone()).expect("host");
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let entered = barrier.clone();
    host.activity("held",move|ctx:ActivityContext,_:Value|{let entered=entered.clone();async move{
        tokio::select!{_=entered.wait()=>Ok(json!({"index":ctx.task.instance.as_ref().expect("scope").instance_path[0].iteration_index})),_=ctx.cancellation.cancelled()=>Err(ActivityFailure::cancelled())}
    }}).expect("handler");
    host.start().await.expect("start");
    let ready = tokio::time::timeout(Duration::from_secs(2), barrier.wait()).await;
    if ready.is_err() {
        host.stop(Duration::ZERO).await.expect("cleanup");
    }
    assert!(
        ready.is_ok(),
        "both scoped instances must enter before either is released"
    );
    let mut indices = Vec::new();
    for _ in 0..2 {
        let report = tokio::time::timeout(Duration::from_secs(2), fixture.complete.recv())
            .await
            .expect("report timeout")
            .expect("report");
        indices.push(report.instance.expect("scope").instance_path[0].iteration_index);
    }
    host.stop(Duration::from_secs(1)).await.expect("drain");
    indices.sort();
    assert_eq!(indices, [3, 4]);
    assert!(
        fixture.complete.try_recv().is_err(),
        "the identical lease must not execute twice"
    );
}

#[tokio::test]
async fn native_worker_tls_verifies_trust_name_and_mutual_client_identity() {
    use rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
        KeyUsagePurpose,
    };
    let authority_key = KeyPair::generate().expect("CA key");
    let mut params = CertificateParams::new(Vec::new()).expect("CA params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let authority = params.self_signed(&authority_key).expect("CA");
    let ca_pem = authority.pem();
    let issue = |client: bool| {
        let key = KeyPair::generate().expect("leaf key");
        let mut params = CertificateParams::new(if client {
            vec![]
        } else {
            vec!["localhost".to_owned()]
        })
        .expect("leaf params");
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![if client {
            ExtendedKeyUsagePurpose::ClientAuth
        } else {
            ExtendedKeyUsagePurpose::ServerAuth
        }];
        (
            params
                .signed_by(&key, &authority, &authority_key)
                .expect("leaf")
                .pem(),
            key.serialize_pem(),
        )
    };
    let (server_cert, server_key) = issue(false);
    let (client_cert, client_key) = issue(true);
    let control = Arc::new(Control::default());
    let (reports, _) = tokio::sync::mpsc::unbounded_channel();
    let service = Service {
        control: control.clone(),
        leases: Arc::default(),
        reports,
        registrations: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("owned TLS listener");
    let endpoint = format!("https://{}", listener.local_addr().expect("address"));
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .tls_config(
                tonic::transport::ServerTlsConfig::new()
                    .identity(tonic::transport::Identity::from_pem(
                        server_cert,
                        server_key,
                    ))
                    .client_ca_root(tonic::transport::Certificate::from_pem(ca_pem.clone())),
            )
            .expect("server TLS")
            .add_service(pb::worker_service_server::WorkerServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
    );
    for (trusted, name, identity, accepted) in [
        (false, "localhost", true, false),
        (true, "wrong.invalid", true, false),
        (true, "localhost", false, false),
        (true, "localhost", true, true),
    ] {
        let mut options = WorkerOptions::new(
            endpoint.clone(),
            "demo",
            "tls-worker",
            TokenSource::bearer("fixture-token"),
        );
        options.timeout = Duration::from_secs(1);
        options.retry.max_attempts = 1;
        if trusted {
            options.tls.ca_certificates.push(ca_pem.as_bytes().to_vec());
        }
        options.tls.server_name = Some(name.into());
        if identity {
            options.tls.identity =
                Some(TlsIdentity::from_pem(&client_cert, &client_key).expect("identity"));
        }
        let mut worker = WorkerHost::new(options).expect("worker");
        worker
            .activity("tls.probe", |_: ActivityContext, _: Value| async { Ok(()) })
            .expect("activity");
        let result = worker.start().await;
        assert_eq!(
            result.is_ok(),
            accepted,
            "trust={trusted}, name={name}, identity={identity}"
        );
        worker.stop(Duration::ZERO).await.expect("stop");
    }
    assert_eq!(control.registrations.load(Ordering::Acquire), 1);
    server.abort();
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
    host.stop(Duration::ZERO)
        .await
        .expect("reaching shutdown grace is successful cooperative shutdown");
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

// The hand-back of `sdks/SPEC.md` section 6.2: on a session of protocol 1.8 or later, a lease the
// host received and will not run is reported with FailStep TASK_NOT_STARTED, before any handler of
// it runs, once, and never for a task whose handler started; below 1.8 it lapses.

const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

/// The lease of run `n`, with a lease token of its own.
fn numbered(n: u32) -> pb::TaskLease {
    pb::TaskLease {
        task_id: format!("run-{n}/charge/1"),
        run_id: format!("run-{n}"),
        step_id: "charge".into(),
        activity_name: "charge".into(),
        attempt: 1,
        generation: 1,
        lease_token: format!("LEASE-TOKEN-{n}"),
        expires_at_epoch_ms: (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
            as i64
            + 30_000,
        input_json: r#"{"node":"charge","input":{}}"#.into(),
        traceparent: TRACEPARENT.into(),
        ..Default::default()
    }
}
/// A sibling iteration of the collection `each`: the same task id as its siblings, its own index.
fn sibling(index: u32) -> pb::TaskLease {
    pb::TaskLease {
        instance: Some(pb::ExecutionInstanceIdentity {
            definition_node_id: "charge".into(),
            instance_path: vec![pb::ScopePathSegment {
                scope_node_id: "each".into(),
                iteration_index: index,
            }],
        }),
        ..numbered(1)
    }
}
struct Served {
    options: WorkerOptions,
    complete: tokio::sync::mpsc::UnboundedReceiver<pb::CompleteStepRequest>,
    control: Arc<Control>,
    leases: Arc<Mutex<VecDeque<pb::TaskLease>>>,
    server: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}
impl Drop for Served {
    fn drop(&mut self) {
        self.server.abort();
    }
}
/// A fake worker service and the options of a host whose session negotiates protocol 1.8, the first
/// minor on which a host hands a lease back (decision D006-080).
async fn serve(leases: Vec<pb::TaskLease>, max_inflight: usize) -> Served {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let mut options = WorkerOptions::new(
        format!("http://{}", listener.local_addr().expect("address")),
        "demo",
        "rust-hand-back",
        TokenSource::bearer("fixture-token"),
    );
    options.max_inflight = max_inflight;
    options.protocol_version = "1.8".into();
    options.retry.initial_delay = Duration::from_millis(5);
    options.retry.max_delay = Duration::from_millis(20);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let control = Arc::new(Control::default());
    control.accepted_minor.store(8, Ordering::Release);
    let leases = Arc::new(Mutex::new(VecDeque::from(leases)));
    let service = Service {
        control: control.clone(),
        leases: leases.clone(),
        reports: tx,
        registrations: Arc::default(),
    };
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(pb::worker_service_server::WorkerServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
    );
    Served {
        options,
        complete: rx,
        control,
        leases,
        server,
    }
}
impl Served {
    /// Serves StreamTasks, with one receive window made of these frames.
    fn stream(&self, frames: Vec<Vec<pb::TaskLease>>) {
        self.control.streaming.store(true, Ordering::Release);
        self.control
            .stream_frames
            .lock()
            .expect("frames")
            .extend(frames);
    }
    fn failures(&self) -> Vec<pb::FailStepRequest> {
        self.control.failed.lock().expect("failures").clone()
    }
    /// The task ids of the FailStep TASK_NOT_STARTED calls, sorted, after checking that each is
    /// the whole hand-back: retryable, the fixed message, the lease's fence and the failure's key.
    fn handed_back(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .failures()
            .into_iter()
            .filter(|r| r.error_code == "TASK_NOT_STARTED")
            .map(|r| {
                assert!(r.retryable, "a hand-back is retryable");
                assert_eq!(r.safe_message, "the host did not start this task");
                assert_eq!(r.idempotency_key, format!("failed-{}", r.task_id));
                assert_eq!(
                    r.lease_token,
                    format!("LEASE-TOKEN-{}", r.run_id.trim_start_matches("run-"))
                );
                assert_eq!(r.step_id, "charge");
                assert_eq!((r.attempt, r.generation), (1, 1));
                assert_eq!(r.traceparent, TRACEPARENT);
                r.task_id
            })
            .collect();
        ids.sort();
        ids
    }
    fn beats_of(&self, task: &str) -> usize {
        self.control
            .heartbeats
            .lock()
            .expect("beats")
            .iter()
            .filter(|beat| beat.task_id == task)
            .count()
    }
    async fn completed(&mut self) -> pb::CompleteStepRequest {
        tokio::time::timeout(Duration::from_secs(10), self.complete.recv())
            .await
            .expect("completion timeout")
            .expect("completion")
    }
}
/// Waits for a condition the host brings about on its own tasks; it is not an ordering.
async fn until(what: &str, condition: impl Fn() -> bool) {
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out waiting for {what}");
}
/// A handler that records the task ids it ran and holds them until `release` is cancelled.
fn held(
    host: &mut WorkerHost,
    release: CancellationToken,
) -> (Arc<Mutex<Vec<String>>>, Arc<tokio::sync::Notify>) {
    let ran = Arc::new(Mutex::new(Vec::new()));
    let started = Arc::new(tokio::sync::Notify::new());
    let (seen, entered) = (ran.clone(), started.clone());
    host.activity("charge", move |ctx: ActivityContext, _: Value| {
        seen.lock().expect("ran").push(ctx.task.task_id.clone());
        entered.notify_one();
        let release = release.clone();
        async move {
            tokio::select! {
                _ = release.cancelled() => Ok(1),
                _ = ctx.cancellation.cancelled() => Err(ActivityFailure::cancelled()),
            }
        }
    })
    .expect("handler");
    (ran, started)
}
/// The barrier of the receive loop: it reports the `nth` answer or frame that carries leases on
/// `read` and blocks the loop on `proceed` before the loop looks at it.
fn hold_nth(
    host: &mut WorkerHost,
    nth: usize,
) -> (
    tokio::sync::mpsc::UnboundedReceiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    let (read, held) = tokio::sync::mpsc::unbounded_channel();
    let (proceed, wait) = std::sync::mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    let answers = AtomicUsize::new(0);
    host.on_frame_received(move |leases| {
        if leases > 0 && answers.fetch_add(1, Ordering::AcqRel) + 1 == nth {
            let _ = read.send(());
            let _ = wait.lock().expect("barrier").recv();
        }
    });
    (held, proceed)
}

// Below protocol 1.8 the host drops the same leases and reports none of them: a server of 1.6 or
// 1.7 would record TASK_NOT_STARTED as an ordinary failure that spends the attempt, so the lease
// lapses as before (decision D006-080).
#[tokio::test]
async fn below_protocol_1_8_a_lease_the_host_drops_lapses_without_a_hand_back() {
    let warnings = Warnings::default();
    let _logs = warnings.install();
    let mut served = serve(vec![], 1).await;
    served.control.accepted_minor.store(6, Ordering::Release);
    served.stream(vec![vec![numbered(1), numbered(2), numbered(3)]]);
    let mut options = served.options.clone();
    options.protocol_version = "1.6".into();
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("charge", |_: ActivityContext, _: Value| async { Ok(1) })
        .expect("handler");
    host.start().await.expect("start");
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    // Stop waits for every hand-back on its way, so a hand-back, had one been queued, has reached
    // the fake.
    host.stop(Duration::from_secs(5)).await.expect("stop");
    assert!(
        served.failures().is_empty(),
        "a dropped lease lapses below protocol 1.8, it is never handed back"
    );
    assert_eq!(
        warnings.tasks("the host drops a task it did not start; its lease lapses"),
        ["run-2/charge/1", "run-3/charge/1"],
        "a warning names each dropped task and says that its lease lapses"
    );
}

// From protocol 1.8 the same leases are handed back (decision D006-080).
#[tokio::test]
async fn from_protocol_1_8_a_lease_the_host_drops_is_handed_back() {
    let mut served = serve(vec![], 1).await;
    served.stream(vec![vec![numbered(1), numbered(2), numbered(3)]]);
    let mut host = WorkerHost::new(served.options.clone()).expect("host");
    host.activity("charge", |_: ActivityContext, _: Value| async { Ok(1) })
        .expect("handler");
    host.start().await.expect("start");
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    host.stop(Duration::from_secs(5)).await.expect("stop");
    assert_eq!(served.handed_back(), ["run-2/charge/1", "run-3/charge/1"]);
}

#[tokio::test]
async fn a_poll_answer_beyond_the_credit_is_handed_back() {
    let mut served = serve(vec![numbered(1), numbered(2), numbered(3)], 1).await;
    served.control.overfill.store(true, Ordering::Release);
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    options.heartbeat_interval = Some(Duration::from_millis(20));
    let mut host = WorkerHost::new(options).expect("host");
    let release = CancellationToken::new();
    let (ran, _) = held(&mut host, release.clone());
    host.start().await.expect("start");
    until("the two hand-backs", || served.failures().len() == 2).await;
    until("a beat of the kept task", || {
        served.beats_of("run-1/charge/1") > 0
    })
    .await;
    release.cancel();
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    host.stop(Duration::from_secs(5)).await.expect("stop");
    assert_eq!(served.handed_back(), ["run-2/charge/1", "run-3/charge/1"]);
    assert_eq!(*ran.lock().expect("ran"), ["run-1/charge/1"]);
    assert_eq!(
        served.beats_of("run-2/charge/1") + served.beats_of("run-3/charge/1"),
        0,
        "a lease that is handed back is never beaten for"
    );
}

#[tokio::test]
async fn a_stream_frame_beyond_the_credit_is_handed_back_and_the_started_task_is_not() {
    let mut served = serve(vec![], 1).await;
    served.stream(vec![vec![numbered(1), numbered(2), numbered(3)]]);
    let mut host = WorkerHost::new(served.options.clone()).expect("host");
    let release = CancellationToken::new();
    let (ran, _) = held(&mut host, release.clone());
    host.start().await.expect("start");
    until("the two hand-backs", || served.failures().len() == 2).await;
    release.cancel();
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    host.stop(Duration::from_secs(5)).await.expect("stop");
    assert_eq!(served.handed_back(), ["run-2/charge/1", "run-3/charge/1"]);
    assert_eq!(
        served.failures().len(),
        2,
        "the started task is never handed back"
    );
    assert_eq!(*ran.lock().expect("ran"), ["run-1/charge/1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn a_stream_frame_read_while_stopping_is_handed_back_whole() {
    let served = serve(vec![], 4).await;
    served.stream(vec![vec![numbered(1), numbered(2)]]);
    let mut host = WorkerHost::new(served.options.clone()).expect("host");
    let release = CancellationToken::new();
    let (ran, _) = held(&mut host, release.clone());
    let (mut read, proceed) = hold_nth(&mut host, 1);
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(10), read.recv())
        .await
        .expect("the frame is read");
    // The loop holds the frame it read while the stop begins; it looks at it only then.
    let receive = host.running.as_ref().expect("running").receive.clone();
    let (stopped, ()) = tokio::join!(host.stop(Duration::from_secs(5)), async move {
        receive.cancelled().await;
        proceed.send(()).expect("the loop is held");
    });
    release.cancel();
    stopped.expect("stop");
    // Stop returned: the hand-backs were made before it did.
    assert_eq!(served.handed_back(), ["run-1/charge/1", "run-2/charge/1"]);
    assert!(
        ran.lock().expect("ran").is_empty(),
        "a host that is stopping starts nothing new"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn a_poll_answer_that_arrives_while_stopping_is_handed_back_and_the_running_task_is_not() {
    let mut served = serve(vec![numbered(1)], 4).await;
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    let release = CancellationToken::new();
    let (ran, started) = held(&mut host, release.clone());
    // The second poll answer that carries leases is held in the loop until the stop has begun.
    let (mut read, proceed) = hold_nth(&mut host, 2);
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(10), started.notified())
        .await
        .expect("the first task starts");
    served
        .leases
        .lock()
        .expect("leases")
        .extend([numbered(2), numbered(3)]);
    tokio::time::timeout(Duration::from_secs(10), read.recv())
        .await
        .expect("the next answer is read");
    let receive = host.running.as_ref().expect("running").receive.clone();
    let failures = served.control.clone();
    let (stopped, ()) = tokio::join!(host.stop(Duration::from_secs(5)), async move {
        receive.cancelled().await;
        proceed.send(()).expect("the loop is held");
        until("the hand-backs", || {
            failures.failed.lock().expect("failures").len() == 2
        })
        .await;
        release.cancel();
    });
    stopped.expect("stop");
    assert_eq!(served.handed_back(), ["run-2/charge/1", "run-3/charge/1"]);
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    assert_eq!(
        served.failures().len(),
        2,
        "the running task reports its own result and is never handed back"
    );
    assert_eq!(*ran.lock().expect("ran"), ["run-1/charge/1"]);
}

#[tokio::test]
async fn a_duplicate_lease_is_ignored_never_handed_back() {
    let mut served = serve(vec![], 1).await;
    // The second is the running task's own lease again; the fourth repeats a lease handed back.
    served.stream(vec![vec![
        numbered(1),
        numbered(1),
        numbered(2),
        numbered(2),
    ]]);
    let mut host = WorkerHost::new(served.options.clone()).expect("host");
    let release = CancellationToken::new();
    let (ran, _) = held(&mut host, release.clone());
    host.start().await.expect("start");
    until("the hand-back", || !served.failures().is_empty()).await;
    release.cancel();
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    host.stop(Duration::from_secs(5)).await.expect("stop");
    assert_eq!(served.handed_back(), ["run-2/charge/1"]);
    assert_eq!(
        served.failures().len(),
        1,
        "a duplicate is never reported, a lease is handed back once"
    );
    assert_eq!(*ran.lock().expect("ran"), ["run-1/charge/1"]);
    assert!(served.complete.try_recv().is_err());
}

#[tokio::test]
async fn a_lease_of_a_running_task_in_a_later_answer_is_ignored_never_handed_back() {
    let mut served = serve(vec![numbered(1)], 2).await;
    served.control.overfill.store(true, Ordering::Release);
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    let release = CancellationToken::new();
    let (ran, started) = held(&mut host, release.clone());
    host.start().await.expect("start");
    tokio::time::timeout(Duration::from_secs(10), started.notified())
        .await
        .expect("the first task starts");
    // The next answer repeats the running task's lease, then carries one task within the credit
    // and one beyond it: the repeated lease, whose fence may be the running handler's, is never
    // reported.
    served
        .leases
        .lock()
        .expect("leases")
        .extend([numbered(1), numbered(3), numbered(4)]);
    until("the hand-back", || served.failures().len() == 1).await;
    release.cancel();
    let mut completed = vec![
        served.completed().await.task_id,
        served.completed().await.task_id,
    ];
    host.stop(Duration::from_secs(5)).await.expect("stop");
    completed.sort();
    assert_eq!(completed, ["run-1/charge/1", "run-3/charge/1"]);
    assert_eq!(served.handed_back(), ["run-4/charge/1"]);
    assert_eq!(served.failures().len(), 1);
    assert_eq!(
        *ran.lock().expect("ran"),
        ["run-1/charge/1", "run-3/charge/1"]
    );
}

#[tokio::test]
async fn sibling_iterations_that_share_a_task_id_are_not_duplicates() {
    let mut served = serve(vec![], 2).await;
    served.stream(vec![vec![sibling(0), sibling(1), sibling(2)]]);
    let mut host = WorkerHost::new(served.options.clone()).expect("host");
    host.activity("charge", |_: ActivityContext, _: Value| async { Ok(1) })
        .expect("handler");
    host.start().await.expect("start");
    let mut indices = Vec::new();
    for _ in 0..2 {
        let report = served.completed().await;
        indices.push(report.instance.expect("scope").instance_path[0].iteration_index);
    }
    until("the iteration beyond the credit is handed back", || {
        served.failures().len() == 1
    })
    .await;
    host.stop(Duration::from_secs(5)).await.expect("stop");
    indices.sort();
    assert_eq!(indices, [0, 1], "both iterations run");
    assert_eq!(served.handed_back(), ["run-1/charge/1"]);
    let handed_back = served.failures().remove(0);
    let instance = handed_back
        .instance
        .expect("the instance identity under 1.5");
    assert_eq!(instance.definition_node_id, "charge");
    assert_eq!(instance.instance_path[0].iteration_index, 2);
}

#[tokio::test]
async fn a_hand_back_is_retried_with_the_same_key_and_ends_quietly_when_stale() {
    let mut served = serve(vec![numbered(1), numbered(2), numbered(3)], 1).await;
    served.control.overfill.store(true, Ordering::Release);
    let mut stale = Status::aborted("stale");
    stale
        .metadata_mut()
        .insert("x-sagat-error", "STALE_ATTEMPT".parse().expect("reason"));
    served
        .control
        .fail_step_errors
        .lock()
        .expect("script")
        .extend([Status::unavailable("down"), stale]);
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    options.telemetry.on_event = Some(Arc::new(move |event: SdkEvent| {
        seen.lock().expect("events").push((event.kind, event.code));
    }));
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("charge", |_: ActivityContext, _: Value| async { Ok(1) })
        .expect("handler");
    host.start().await.expect("start");
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    let count = |kind: EventKind, code: Option<&str>| {
        events
            .lock()
            .expect("events")
            .iter()
            .filter(|(k, c)| *k == kind && *c == code)
            .count()
    };
    until("one hand-back counted as a failure and one stale", || {
        count(EventKind::Failed, Some("TASK_NOT_STARTED")) == 1
            && count(EventKind::Stale, Some("STALE_ATTEMPT")) == 1
    })
    .await;
    host.stop(Duration::from_secs(5)).await.expect("stop");
    // Three calls for two leases: one answered UNAVAILABLE and repeated with its key, one stale.
    let calls = served.failures();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|c| c.error_code == "TASK_NOT_STARTED"));
    let mut keys: Vec<_> = calls.iter().map(|c| c.idempotency_key.as_str()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys, ["failed-run-2/charge/1", "failed-run-3/charge/1"]);
    assert_eq!(count(EventKind::Failed, Some("TASK_NOT_STARTED")), 1);
    assert_eq!(count(EventKind::Stale, Some("STALE_ATTEMPT")), 1);
    assert_eq!(
        count(EventKind::Received, None),
        1,
        "a lease that is handed back was never taken"
    );
}

#[tokio::test]
async fn a_lease_the_in_flight_table_cannot_record_is_handed_back() {
    let served = serve(vec![numbered(1)], 2).await;
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    let release = CancellationToken::new();
    let (ran, _) = held(&mut host, release.clone());
    let tasks = host.tasks.clone();
    let poisoned = std::thread::spawn(move || {
        let _held = tasks.lock();
        panic!("a panic while the in-flight table is held poisons it");
    })
    .join();
    assert!(poisoned.is_err());
    host.start().await.expect("start");
    until("the hand-back", || served.failures().len() == 1).await;
    host.stop(Duration::from_secs(5)).await.expect("stop");
    assert_eq!(served.handed_back(), ["run-1/charge/1"]);
    assert!(ran.lock().expect("ran").is_empty());
}

#[tokio::test]
async fn a_lease_without_a_free_slot_is_handed_back() {
    let served = serve(vec![numbered(1), numbered(2)], 2).await;
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    let release = CancellationToken::new();
    let (ran, _) = held(&mut host, release.clone());
    // The slots are taken between the credit and the dispatch: the leases are beyond the free slots.
    let permits = host.permits.clone();
    host.on_frame_received(move |leases| {
        if leases > 0 {
            permits.forget_permits(permits.available_permits());
        }
    });
    host.start().await.expect("start");
    until("the hand-backs", || served.failures().len() == 2).await;
    host.stop(Duration::from_secs(5)).await.expect("stop");
    assert_eq!(served.handed_back(), ["run-1/charge/1", "run-2/charge/1"]);
    assert!(ran.lock().expect("ran").is_empty());
}

#[tokio::test]
async fn a_frame_after_the_credit_is_used_up_is_read_and_handed_back() {
    let mut served = serve(vec![], 1).await;
    // The first frame uses the whole credit; the second, in the same window, is beyond it.
    served.stream(vec![vec![numbered(1)], vec![numbered(2), numbered(3)]]);
    let mut host = WorkerHost::new(served.options.clone()).expect("host");
    let release = CancellationToken::new();
    let (ran, _) = held(&mut host, release.clone());
    host.start().await.expect("start");
    until("the two hand-backs", || served.failures().len() == 2).await;
    release.cancel();
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    host.stop(Duration::from_secs(5)).await.expect("stop");
    assert_eq!(served.handed_back(), ["run-2/charge/1", "run-3/charge/1"]);
    assert_eq!(served.failures().len(), 2);
    assert_eq!(*ran.lock().expect("ran"), ["run-1/charge/1"]);
}

#[tokio::test]
async fn the_rest_of_a_frame_over_the_input_budget_is_handed_back() {
    let mut served = serve(vec![], 3).await;
    let heavy = |n: u32| pb::TaskLease {
        input_json: format!(
            r#"{{"node":"charge","input":"{}"}}"#,
            "x".repeat(5 * 1024 * 1024)
        ),
        ..numbered(n)
    };
    served.stream(vec![vec![heavy(1), heavy(2), numbered(3)]]);
    let mut host = WorkerHost::new(served.options.clone()).expect("host");
    let release = CancellationToken::new();
    let (ran, _) = held(&mut host, release.clone());
    host.start().await.expect("start");
    until("the hand-backs", || served.failures().len() == 2).await;
    // The broken window is a receive failure the host backs off from, not the end of the host.
    assert!(host.failure.borrow().is_none());
    assert!(!host.running.as_ref().expect("running").join.is_finished());
    release.cancel();
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    host.stop(Duration::from_secs(5)).await.expect("stop");
    // The lease over the budget and the rest of the frame after it.
    assert_eq!(served.handed_back(), ["run-2/charge/1", "run-3/charge/1"]);
    assert_eq!(*ran.lock().expect("ran"), ["run-1/charge/1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn at_most_eight_hand_backs_are_on_their_way_at_once_and_the_others_wait_their_turn() {
    let mut served = serve((1..=10_000).map(numbered).collect(), 1).await;
    served.control.overfill.store(true, Ordering::Release);
    served.control.hold_fail_step.store(true, Ordering::Release);
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("charge", |_: ActivityContext, _: Value| async { Ok(1) })
        .expect("handler");
    host.start().await.expect("start");
    let held = served.control.clone();
    until("eight hand-backs held at the server", || {
        held.fail_steps_held.load(Ordering::Acquire) >= 8
    })
    .await;
    // While the eight are held no ninth may arrive. Its absence cannot be awaited: the server is
    // watched for a while, which orders nothing (the eight are held on the barrier whatever the
    // delay).
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(served.failures().len(), 8);
    assert_eq!(served.control.fail_step_peak.load(Ordering::Acquire), 8);
    served.control.fail_step_release.cancel();
    let waited = tokio::time::timeout(Duration::from_secs(60), async {
        while served.failures().len() < 9_999 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        waited.is_ok(),
        "every lease beyond the first is handed back"
    );
    host.stop(Duration::from_secs(10)).await.expect("stop");
    assert_eq!(served.control.fail_step_peak.load(Ordering::Acquire), 8);
    let mut ids: Vec<_> = served.failures().into_iter().map(|r| r.task_id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 9_999);
    assert!(!ids.contains(&"run-1/charge/1".to_string()));
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
}

#[tokio::test]
async fn a_lease_leased_again_while_its_hand_back_is_on_its_way_is_ignored() {
    let mut served = serve(vec![numbered(1), numbered(2)], 1).await;
    served.control.overfill.store(true, Ordering::Release);
    served.control.hold_fail_step.store(true, Ordering::Release);
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    let ran = Arc::new(Mutex::new(Vec::new()));
    let seen = ran.clone();
    host.activity("charge", move |ctx: ActivityContext, _: Value| {
        seen.lock().expect("ran").push(ctx.task.task_id.clone());
        async { Ok(1) }
    })
    .expect("handler");
    host.start().await.expect("start");
    let held = served.control.clone();
    until(
        "the hand-back of the second lease is held at the server",
        || held.fail_steps_held.load(Ordering::Acquire) == 1,
    )
    .await;
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    // A later answer leases the second task again while its TASK_NOT_STARTED is on its way. The
    // poll after that answer shows the host has looked at it.
    served.leases.lock().expect("leases").push_back(numbered(2));
    let leases = served.leases.clone();
    until("the answer that repeats the lease", || {
        leases.lock().expect("leases").is_empty()
    })
    .await;
    let polls = served.control.poll_calls.load(Ordering::Acquire);
    until("the poll after it", || {
        held.poll_calls.load(Ordering::Acquire) > polls
    })
    .await;
    served.control.fail_step_release.cancel();
    until("the hand-back ends", || {
        held.fail_steps_held.load(Ordering::Acquire) == 0
    })
    .await;
    host.stop(Duration::from_secs(5)).await.expect("stop");
    assert_eq!(served.handed_back(), ["run-2/charge/1"]);
    assert_eq!(served.failures().len(), 1);
    assert_eq!(*ran.lock().expect("ran"), ["run-1/charge/1"]);
    assert!(served.complete.try_recv().is_err());
}

/// The warnings the host logs, as their message and task field, on the thread of a test whose
/// runtime has one thread (`tracing::subscriber::set_default` holds for that thread only).
#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<(String, String)>>>);
struct WarningFields(String, String);
impl tracing::field::Visit for WarningFields {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "task" {
            self.1 = value.to_owned();
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Warnings {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::WARN {
            let mut fields = WarningFields(String::new(), String::new());
            event.record(&mut fields);
            self.0.lock().expect("warnings").push((fields.0, fields.1));
        }
    }
}
impl Warnings {
    fn install(&self) -> tracing::subscriber::DefaultGuard {
        use tracing_subscriber::layer::SubscriberExt as _;
        tracing::subscriber::set_default(tracing_subscriber::registry().with(self.clone()))
    }
    /// The task ids of the warnings whose message contains `text`, sorted.
    fn tasks(&self, text: &str) -> Vec<String> {
        let mut tasks: Vec<String> = self
            .0
            .lock()
            .expect("warnings")
            .iter()
            .filter(|(message, _)| message.contains(text))
            .map(|(_, task)| task.clone())
            .collect();
        tasks.sort();
        tasks
    }
}

#[tokio::test]
async fn hand_backs_still_on_their_way_when_the_grace_ends_lapse_and_are_never_reported() {
    let warnings = Warnings::default();
    let _logs = warnings.install();
    let mut served = serve((1..=21).map(numbered).collect(), 1).await;
    served.control.overfill.store(true, Ordering::Release);
    served.control.hold_fail_step.store(true, Ordering::Release);
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    options.telemetry.on_event = Some(Arc::new(move |event: SdkEvent| {
        seen.lock().expect("events").push((event.kind, event.code));
    }));
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("charge", |_: ActivityContext, _: Value| async { Ok(1) })
        .expect("handler");
    host.start().await.expect("start");
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    let held = served.control.clone();
    until("eight hand-backs held, twelve queued", || {
        held.fail_steps_held.load(Ordering::Acquire) == 8
    })
    .await;
    // The twenty hand-backs cannot end within the grace: eight are held at the server, the others
    // wait for them.
    let grace = Duration::from_millis(300);
    let clock = std::time::Instant::now();
    host.stop(grace).await.expect("stop");
    let took = clock.elapsed();
    assert!(took < grace + Duration::from_secs(1), "stop took {took:?}");
    let expected: Vec<String> = {
        let mut ids: Vec<String> = (2..=21).map(|n| format!("run-{n}/charge/1")).collect();
        ids.sort();
        ids
    };
    let lapsed = warnings.clone();
    until(
        "every hand-back that was not made lapses, held and queued alike",
        || lapsed.tasks("its lease lapses") == expected,
    )
    .await;
    served.control.fail_step_release.cancel();
    until("the server lets the held calls go", || {
        held.fail_steps_held.load(Ordering::Acquire) == 0
    })
    .await;
    // Every hand-back ended lapsed, so no other call can follow: eight reached the server once
    // each, none was counted as made.
    let mut calls: Vec<_> = served.failures().into_iter().map(|r| r.task_id).collect();
    assert_eq!(calls.len(), 8);
    calls.sort();
    calls.dedup();
    assert_eq!(calls.len(), 8);
    assert!(
        !events
            .lock()
            .expect("events")
            .iter()
            .any(|(kind, code)| *kind == EventKind::Failed && *code == Some("TASK_NOT_STARTED")),
        "a hand-back that lapsed is never counted as made"
    );
}

#[tokio::test]
async fn a_hand_back_that_cannot_be_made_is_logged_and_its_lease_lapses() {
    let warnings = Warnings::default();
    let _logs = warnings.install();
    let mut served = serve(vec![numbered(1), numbered(2)], 1).await;
    served.control.overfill.store(true, Ordering::Release);
    served
        .control
        .fail_step_errors
        .lock()
        .expect("script")
        .extend((0..4).map(|_| Status::unavailable("down")));
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("charge", |_: ActivityContext, _: Value| async { Ok(1) })
        .expect("handler");
    host.start().await.expect("start");
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    let logged = warnings.clone();
    until("the hand-back that cannot be made is logged", || {
        logged.tasks("could not be made") == ["run-2/charge/1"]
    })
    .await;
    host.stop(Duration::from_secs(5)).await.expect("stop");
    // Every attempt the retry policy allows (four), with the same key, and then nothing more.
    let calls = served.failures();
    assert_eq!(calls.len(), 4);
    assert!(
        calls
            .iter()
            .all(|c| c.idempotency_key == "failed-run-2/charge/1")
    );
}

#[tokio::test]
async fn a_hand_back_queued_while_the_worker_leaves_is_handed_back_once() {
    let mut served = serve(vec![numbered(1), numbered(2)], 1).await;
    served.control.overfill.store(true, Ordering::Release);
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("charge", |_: ActivityContext, _: Value| async { Ok(1) })
        .expect("handler");
    // One worker place, and that worker held once it has found the queue empty and before it gives
    // up its place.
    host.hand_backs.bound.store(1, Ordering::Release);
    let (found_empty, mut leaving) = tokio::sync::oneshot::channel();
    let (release, held) = tokio::sync::oneshot::channel();
    *super::locked(&host.hand_backs.leaving) = Some((found_empty, held));
    host.start().await.expect("start");
    // The worker hands back run-2, finds the queue empty and is held there.
    tokio::time::timeout(Duration::from_secs(10), &mut leaving)
        .await
        .expect("the worker leaves")
        .expect("the barrier");
    assert_eq!(served.handed_back(), ["run-2/charge/1"]);
    assert_eq!(served.completed().await.task_id, "run-1/charge/1");
    // The next answer runs run-3 and hands back run-4, which finds the only place still taken.
    served
        .leases
        .lock()
        .expect("leases")
        .extend([numbered(3), numbered(4)]);
    let leases = served.leases.clone();
    until("the next answer", || {
        leases.lock().expect("leases").is_empty()
    })
    .await;
    let polls = served.control.poll_calls.load(Ordering::Acquire);
    let control = served.control.clone();
    until("the poll after it", || {
        control.poll_calls.load(Ordering::Acquire) > polls
    })
    .await;
    assert_eq!(served.failures().len(), 1, "run-4 waits for a worker");
    release.send(()).expect("the worker is held");
    until("the queued hand-back", || served.failures().len() == 2).await;
    assert_eq!(served.completed().await.task_id, "run-3/charge/1");
    host.stop(Duration::from_secs(5)).await.expect("stop");
    // Never stranded, never twice.
    assert_eq!(served.handed_back(), ["run-2/charge/1", "run-4/charge/1"]);
    assert_eq!(served.failures().len(), 2);
}

/// A handler fails with the code reserved for the host, and the next lease is beyond the credit:
/// the handler's failure is reported as WORKER_ERROR with its own message and retryable flag, and
/// the real hand-back still carries TASK_NOT_STARTED (`sdks/SPEC.md` section 6.2).
async fn a_handler_failing_with_the_reserved_code(retryable: bool) {
    let served = serve(vec![numbered(1), numbered(2)], 1).await;
    served.control.overfill.store(true, Ordering::Release);
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    options.telemetry.on_event = Some(Arc::new(move |event: SdkEvent| {
        seen.lock().expect("events").push((event.kind, event.code));
    }));
    let mut host = WorkerHost::new(options).expect("host");
    host.activity("charge", move |_: ActivityContext, _: Value| async move {
        Err::<i32, _>(if retryable {
            ActivityFailure::retryable("TASK_NOT_STARTED", "custom")
        } else {
            ActivityFailure::permanent("TASK_NOT_STARTED", "custom")
        })
    })
    .expect("handler");
    host.start().await.expect("start");
    until("the handler's failure and the hand-back", || {
        served.failures().len() == 2
    })
    .await;
    host.stop(Duration::from_secs(5)).await.expect("stop");
    let handler = served
        .failures()
        .into_iter()
        .find(|r| r.task_id == "run-1/charge/1")
        .expect("the handler's failure");
    assert_eq!(
        (
            handler.error_code.as_str(),
            handler.safe_message.as_str(),
            handler.retryable
        ),
        ("WORKER_ERROR", "custom", retryable)
    );
    assert_eq!(served.handed_back(), ["run-2/charge/1"]);
    let failed_with_the_code = events
        .lock()
        .expect("events")
        .iter()
        .filter(|(kind, code)| *kind == EventKind::Failed && *code == Some("TASK_NOT_STARTED"))
        .count();
    assert_eq!(
        failed_with_the_code, 1,
        "only the hand-back carries the code"
    );
}

#[tokio::test]
async fn a_retryable_handler_failure_cannot_send_task_not_started() {
    a_handler_failing_with_the_reserved_code(true).await;
}

#[tokio::test]
async fn a_permanent_handler_failure_cannot_send_task_not_started() {
    a_handler_failing_with_the_reserved_code(false).await;
}

#[tokio::test]
async fn a_compensation_or_a_reconciler_cannot_send_task_not_started() {
    let kind = |n: u32, activity: &str, task_kind: &str| pb::TaskLease {
        activity_name: activity.into(),
        task_kind: task_kind.into(),
        target_node: "charge".into(),
        ..numbered(n)
    };
    let served = serve(
        vec![
            kind(1, "refund", "COMPENSATION"),
            kind(2, "check", "RECONCILIATION"),
        ],
        2,
    )
    .await;
    let mut options = served.options.clone();
    options.transport = WorkerTransport::Poll;
    let mut host = WorkerHost::new(options).expect("host");
    host.compensation("refund", |_: ActivityContext, _: Value| async {
        Err::<i32, _>(ActivityFailure::retryable("TASK_NOT_STARTED", "custom"))
    })
    .expect("compensation");
    host.reconciler("check", |_: ActivityContext, _: Value| async {
        Err(ActivityFailure::permanent("TASK_NOT_STARTED", "custom"))
    })
    .expect("reconciler");
    host.start().await.expect("start");
    until("both failures", || served.failures().len() == 2).await;
    host.stop(Duration::from_secs(5)).await.expect("stop");
    let mut reported: Vec<_> = served
        .failures()
        .into_iter()
        .map(|r| (r.task_id, r.error_code, r.safe_message, r.retryable))
        .collect();
    reported.sort();
    assert_eq!(
        reported,
        [
            (
                "run-1/charge/1".to_string(),
                "WORKER_ERROR".to_string(),
                "custom".to_string(),
                true
            ),
            (
                "run-2/charge/1".to_string(),
                "WORKER_ERROR".to_string(),
                "custom".to_string(),
                false
            ),
        ]
    );
}

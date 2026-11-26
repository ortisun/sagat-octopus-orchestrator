//! The worker protocol over gRPC: authenticate the worker, check the request, call the use case,
//! map the outcome. No business rule lives here; the lease, fencing and idempotency semantics are
//! the application's (`WorkerService`).

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;
use std::time::Instant;

use sagat_api_http::error::ApiError;
use sagat_api_http::{Authenticated, Core, authenticate_bearer, parse_bearer};
use sagat_application::blobs::BlobService;
use sagat_application::clock::Clock;
use sagat_application::definitions::DefinitionStore;
use sagat_application::entropy::Entropy;
use sagat_application::error::{AccessDenial, AppError};
use sagat_application::execution::{ExpressionEvaluator, MAX_PAYLOAD_BYTES, PathExecutionStore};
use sagat_application::identity::RequestContext;
use sagat_application::runs::RunStore;
use sagat_application::worker::{
    BlobCompletionTarget, BlobOutput, Capabilities, ClaimedTask, StepAck, WorkerService,
    WorkerStore,
};
use sagat_domain::blobs::BlobId;
use sagat_domain::execution::{ExecutionInstanceId, InstancePath};
use sagat_domain::identity::Principal;
use sagat_domain::workflow::NodeId;
use sagat_telemetry::TraceContext;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tonic::{Code, Request, Response, Status};
use tracing::Instrument;

use crate::blobs as blob_failures;
use crate::blobs::{
    BlobAccess, BlobTransfer, Download, read_failure, upload_body, write_failure, write_request,
};
use crate::limits::{
    Admission, CappedConnectInfo, ConnectionDeadline, TaskStreamTicket, TaskStreams,
};
use crate::pb;
use crate::request::{Attempt, idempotency_key, worker_id};
use crate::server::AdmissionProvider;
use crate::status::{invalid, to_status};
use crate::undelivered::{Abandonment, DeliveryProbe, PendingLeases, WithdrawalSink, Withdrawals};
use tokio_util::sync::CancellationToken;

/// The gRPC adapter of [`WorkerService`]. `St` carries token verification, the budgets and the
/// identity lookup (the same [`Core`] the REST API authenticates with).
pub struct WorkerGrpc<St, S, C, E, R> {
    core: St,
    workers: Arc<WorkerService<S, C, E, R>>,
    blobs: Option<BlobAccess>,
    task_streams: Arc<TaskStreams>,
    admission: Admission,
    /// The withdrawals of leases that were committed and never handed to the transport (feature 006, decision D006-016).
    withdrawals: Withdrawals,
    /// A barrier after a claim committed (tests only, decision D006-008).
    probe: Option<DeliveryProbe>,
    /// A barrier before a run's transaction that leased commits (tests only, decisions D006-008 and D006-092).
    commit_probe: Option<DeliveryProbe>,
}

impl<St: Clone, S, C, E, R> Clone for WorkerGrpc<St, S, C, E, R> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            workers: Arc::clone(&self.workers),
            blobs: self.blobs.clone(),
            task_streams: Arc::clone(&self.task_streams),
            admission: self.admission.clone(),
            withdrawals: self.withdrawals.clone(),
            probe: self.probe.clone(),
            commit_probe: self.commit_probe.clone(),
        }
    }
}

impl<St, S, C, E, R> WorkerGrpc<St, S, C, E, R> {
    #[must_use]
    pub fn new(core: St, workers: WorkerService<S, C, E, R>) -> Self {
        let admission = Admission::new(1024, Duration::from_secs(30));
        Self {
            core,
            workers: Arc::new(workers),
            blobs: None,
            task_streams: TaskStreams::new(),
            admission,
            withdrawals: Withdrawals::default(),
            probe: None,
            commit_probe: None,
        }
    }

    /// The withdrawals in flight: a graceful shutdown waits for them once the server stopped serving (feature 006,
    /// decision D006-016).
    #[must_use]
    pub fn withdrawals(&self) -> Withdrawals {
        self.withdrawals.clone()
    }

    /// Runs at most `concurrency` withdrawals at once (decision D006-038: a quarter of the pool,
    /// [`crate::withdrawal_concurrency`]). Call it before [`Self::withdrawals`] is read.
    #[must_use]
    pub fn with_withdrawal_concurrency(mut self, concurrency: usize) -> Self {
        self.withdrawals = Withdrawals::bounded(concurrency);
        self
    }

    /// Holds every claim at `probe` after each run's transaction committed leases, before the next run is claimed and
    /// before its leases are queued or answered. For the tests of the withdrawal only (decision D006-008): production never
    /// sets one.
    #[must_use]
    pub fn with_delivery_probe(mut self, probe: DeliveryProbe) -> Self {
        self.probe = Some(probe);
        self
    }

    /// Holds every claim at `probe` just before a run's transaction that leased commits, before the claim asks whether its
    /// consumer is gone (decision D006-092). For the tests of the rollback only (decision D006-008): production never sets one.
    #[must_use]
    pub fn with_commit_probe(mut self, probe: DeliveryProbe) -> Self {
        self.commit_probe = Some(probe);
        self
    }

    /// Serves `WriteBlob` and `ReadBlob`, and accepts `output_blob_ref`. Without it those calls are
    /// refused as "not configured on this instance".
    #[must_use]
    pub fn with_blobs(mut self, service: BlobService, transfer: BlobTransfer) -> Self {
        self.blobs = Some(BlobAccess::new(service, transfer));
        self
    }

    /// Refuses every call while `gate` is closed (`UNAVAILABLE`, with the reason `BINARY_CAPABILITY_MISSING` when the
    /// platform requires a capability this binary lacks): the worker protocol of a process that may not serve
    /// (`FR-018-052`). Another replica serves the worker.
    #[must_use]
    pub fn with_serving_gate(self, gate: Arc<sagat_application::instances::ServingGate>) -> Self {
        self.admission.set_serving_gate(gate);
        self
    }

    fn blob_access(&self) -> Result<&BlobAccess, Status> {
        self.blobs
            .as_ref()
            .ok_or_else(blob_failures::not_configured)
    }

    pub(crate) fn admission(&self) -> Admission {
        self.admission.clone()
    }
}

impl<St, S, C, E, R> AdmissionProvider for WorkerGrpc<St, S, C, E, R> {
    fn grpc_admission(&self) -> Option<Admission> {
        Some(self.admission())
    }
}

const STREAM_WINDOW: Duration = Duration::from_secs(5);

/// A stream of a capability call that asks the gate again before each item it hands out (feature 006, decisions D006-098
/// and D006-100): before the first, so a call admitted at the edge of the capability window hands nothing out after it, and
/// before every later one (a cheap read of the gate), so a long download stops when the window lapses.
struct CheckedFirst<S> {
    inner: S,
    admission: Admission,
    method: &'static str,
    refused: bool,
}

impl<S, T> Stream for CheckedFirst<S>
where
    S: Stream<Item = Result<T, Status>> + Unpin,
{
    type Item = Result<T, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.refused {
            return Poll::Ready(None);
        }
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(item))) => {
                if let Some(refused) = self.admission.capability_refusal(self.method) {
                    // the item is dropped: nothing of it leaves
                    self.refused = true;
                    return Poll::Ready(Some(Err(refused)));
                }
                Poll::Ready(Some(Ok(item)))
            }
            other => other,
        }
    }
}

/// How long before the hard end of a receive window the producer stops claiming and queues its terminal hint. It is
/// 5% of the window: measured under CPU load the hint took up to 15 ms to reach a reader that was keeping up, and the
/// 10 ms this was before was lost twice in 34 runs of the contract test under load.
const TERMINAL_HINT_MARGIN: Duration = Duration::from_millis(250);

/// When the producer of a stream must be done: the hard deadline of the body when the session token expires first (the
/// stream is cut there), otherwise [`TERMINAL_HINT_MARGIN`] earlier, so that a reader that is keeping up receives the
/// terminal hint before the body's own deadline discards anything still queued.
fn producer_end(
    body_deadline: tokio::time::Instant,
    token_expires_first: bool,
) -> tokio::time::Instant {
    if token_expires_first {
        body_deadline
    } else {
        body_deadline
            .checked_sub(TERMINAL_HINT_MARGIN)
            .unwrap_or(body_deadline)
    }
}

fn stream_deadline_frame(
    token_expired: bool,
    idle_retry: Duration,
) -> Result<pb::PollTasksResponse, Status> {
    if token_expired {
        Err(api(&ApiError::invalid_credentials()))
    } else {
        Ok(pb::PollTasksResponse {
            tasks: Vec::new(),
            retry_after_ms: i32::try_from(idle_retry.as_millis())
                .unwrap_or(i32::MAX)
                .max(1),
        })
    }
}

/// One item of the queue between the producer and the body: the frame, and the leases it carries until it is handed to the
/// transport (decision D006-002).
struct Frame {
    item: Result<pb::PollTasksResponse, Status>,
    leases: PendingLeases,
}

impl Frame {
    const fn plain(item: Result<pb::PollTasksResponse, Status>) -> Self {
        Self {
            item,
            leases: PendingLeases::none(),
        }
    }

    /// The frame is handed to the transport: its leases are no longer this process's to withdraw.
    fn handed(self) -> Result<pb::PollTasksResponse, Status> {
        self.leases.handed();
        self.item
    }
}

struct TaskBody {
    inner: mpsc::Receiver<Frame>,
    producer: tokio::task::JoinHandle<()>,
    /// Tells the producer to stop: it is never aborted, so a claim it is in finishes and its leases reach a frame or the
    /// withdrawal (decision D006-002).
    stop: CancellationToken,
    _ticket: Option<TaskStreamTicket>,
    _connection_deadline: Option<ConnectionDeadline>,
    token_expiry: Pin<Box<tokio::time::Sleep>>,
    window_deadline: Pin<Box<tokio::time::Sleep>>,
    expiry_reported: bool,
    deadline_reported: bool,
    terminal_delivered: bool,
    reason: Arc<AtomicU8>,
    started: Instant,
}

fn capped_connection<T>(request: &Request<T>) -> Option<CappedConnectInfo> {
    request
        .extensions()
        .get::<CappedConnectInfo>()
        .cloned()
        .or_else(|| {
            request
                .extensions()
                .get::<tonic::transport::server::TlsConnectInfo<CappedConnectInfo>>()
                .map(|info| info.get_ref().clone())
        })
}

impl Stream for TaskBody {
    type Item = Result<pb::PollTasksResponse, Status>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if !this.expiry_reported && this.token_expiry.as_mut().poll(cx).is_ready() {
            this.expiry_reported = true;
            this.reason.store(5, Ordering::Release);
            this.stop.cancel();
            this.inner.close();
            // a drained frame drops its leases into the withdrawal
            while this.inner.try_recv().is_ok() {}
            return Poll::Ready(Some(Err(api(&ApiError::invalid_credentials()))));
        }
        if this.deadline_reported {
            // The window is over and its last item was given: nothing follows, and the stopped producer is not asked why.
            return Poll::Ready(None);
        }
        if this.window_deadline.as_mut().poll(cx).is_ready() {
            this.deadline_reported = true;
            let completed_reason = this.reason.load(Ordering::Acquire);
            this.stop.cancel();
            // A terminal hint that the producer queued before the deadline is still the right ending for a reader that
            // was merely late to come for it. It carries no task, so handing it over late cannot lose a lease. Anything
            // else that is queued is discarded: a transport that stalled past the deadline sees no old frame.
            let late_hint = match this.inner.try_recv() {
                Ok(Frame {
                    item: Ok(frame),
                    leases,
                }) if frame.tasks.is_empty() && frame.retry_after_ms > 0 => {
                    leases.handed();
                    Some(frame)
                }
                // a frame with leases, discarded here, drops them into the withdrawal
                _ => None,
            };
            this.inner.close();
            while this.inner.try_recv().is_ok() {}
            if completed_reason != 2 && !(late_hint.is_some() && matches!(completed_reason, 3 | 4))
            {
                this.reason.store(4, Ordering::Release);
            }
            if let Some(frame) = late_hint {
                this.terminal_delivered = true;
                return Poll::Ready(Some(Ok(frame)));
            }
            if this.terminal_delivered || completed_reason == 2 {
                return Poll::Ready(None);
            }
            return Poll::Ready(Some(Err(Status::deadline_exceeded(
                "the task receive window ended",
            ))));
        }
        let next = this
            .inner
            .poll_recv(cx)
            .map(|frame| frame.map(Frame::handed));
        if matches!(
            &next,
            Poll::Ready(Some(Ok(frame))) if frame.tasks.is_empty() && frame.retry_after_ms > 0
        ) {
            this.terminal_delivered = true;
        }
        if matches!(next, Poll::Ready(None))
            && let Poll::Ready(Err(error)) = Pin::new(&mut this.producer).poll(cx)
        {
            this.reason.store(8, Ordering::Release);
            tracing::error!(%error, "worker task stream producer failed");
        }
        next
    }
}

impl Drop for TaskBody {
    fn drop(&mut self) {
        // Told, never aborted: the producer finishes the claim it is in, and the leases it holds, and those still queued
        // (dropped with the queue), go to the withdrawal (decision D006-002).
        self.stop.cancel();
        let reason = match self.reason.load(Ordering::Acquire) {
            2 => "credit",
            3 => "bytes",
            4 => "window",
            5 => "unauthenticated",
            6 => "refused",
            7 => "unavailable",
            8 => "internal",
            _ => "cancelled",
        };
        metrics::counter!("worker_task_streams_closed_total", "reason" => reason).increment(1);
        metrics::histogram!("worker_task_stream_duration_seconds")
            .record(self.started.elapsed().as_secs_f64());
    }
}

fn app(error: AppError) -> Status {
    api(&ApiError::from(error))
}

fn api(error: &ApiError) -> Status {
    if let Some(denial) = error.access_denial() {
        // Leaves the denial for `observe`, which knows the call and, once the call authenticated,
        // the caller (gap G9).
        let _ = DENIAL.try_with(|slot| {
            *slot.denial.lock().unwrap_or_else(PoisonError::into_inner) = Some(denial);
        });
    }
    to_status(error)
}

/// What a call learns about itself on the way, for the audit of a denial.
#[derive(Default)]
struct DenialNote {
    principal: Mutex<Option<Principal>>,
    denial: Mutex<Option<AccessDenial>>,
}

tokio::task_local! {
    static DENIAL: Arc<DenialNote>;
}

/// The RPC as named in the audit trail: the service and the method, never the request.
fn rpc_name(method: &str) -> String {
    format!("grpc WorkerService/{method}")
}

fn trace_of<T>(request: &Request<T>) -> TraceContext {
    TraceContext::from_traceparent(
        request
            .metadata()
            .get("traceparent")
            .and_then(|value| value.to_str().ok()),
    )
}

const fn code_label(code: Code) -> &'static str {
    match code {
        Code::Ok => "ok",
        Code::Cancelled => "cancelled",
        Code::Unknown => "unknown",
        Code::InvalidArgument => "invalid_argument",
        Code::DeadlineExceeded => "deadline_exceeded",
        Code::NotFound => "not_found",
        Code::AlreadyExists => "already_exists",
        Code::PermissionDenied => "permission_denied",
        Code::ResourceExhausted => "resource_exhausted",
        Code::FailedPrecondition => "failed_precondition",
        Code::Aborted => "aborted",
        Code::OutOfRange => "out_of_range",
        Code::Unimplemented => "unimplemented",
        Code::Internal => "internal",
        Code::Unavailable => "unavailable",
        Code::DataLoss => "data_loss",
        Code::Unauthenticated => "unauthenticated",
    }
}

/// Runs one call under a span carrying the trace id, and counts and times it. The `method` label
/// comes from a fixed set of RPC names, so the metric cardinality is fixed.
async fn observe<St: Core, T>(
    core: &St,
    method: &'static str,
    trace: TraceContext,
    call: impl Future<Output = Result<T, Status>>,
) -> Result<Response<T>, Status> {
    let started = Instant::now();
    // A call that carries no `traceparent` (a poll, a registration) is a trace of its own that nobody
    // asked for: it is logged under its id but not exported, or every idle worker would add a
    // one-span trace per poll. A report of an attempt carries the attempt's span (the lease gave it)
    // and joins the run's trace under it.
    let span = tracing::info_span!(
        "grpc.worker",
        method,
        trace_id = %trace.trace_id(),
        span_id = %sagat_telemetry::SpanId::generate(),
        parent_span_id = trace.parent_span_id().map(|p| p.to_string()),
        otel.kind = "server",
        otel.sampled = trace.is_continuation(),
        grpc.status_code = tracing::field::Empty,
        otel.status_code = tracing::field::Empty,
    );
    let note = Arc::new(DenialNote::default());
    let result = DENIAL
        .scope(Arc::clone(&note), call.instrument(span.clone()))
        .await;
    let denied = note
        .denial
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let caller = note
        .principal
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let refused = result
        .as_ref()
        .err()
        .is_some_and(|status| status.code() == Code::PermissionDenied);
    if let (true, Some(denial), Some(principal)) = (refused, denied, caller) {
        let ctx = RequestContext {
            trace_id: Some(trace.trace_id().to_string()),
            span_id: None,
        };
        let _ = sagat_api_http::denial::spawn_audit_access_denied(
            core,
            &principal,
            &rpc_name(method),
            denial,
            &ctx,
        )
        .instrument(span.clone())
        .await;
    }
    let code = result.as_ref().err().map_or(Code::Ok, Status::code);
    span.record("grpc.status_code", code_label(code));
    if code == Code::Internal || code == Code::Unknown {
        span.record("otel.status_code", "error");
    }
    metrics::counter!("grpc_requests_total", "method" => method, "code" => code_label(code))
        .increment(1);
    metrics::histogram!("grpc_request_duration_seconds", "method" => method)
        .record(started.elapsed().as_secs_f64());
    if code == Code::Internal {
        tracing::error!(method, "worker call failed internally");
    }
    result.map(Response::new)
}

impl<St, S, C, E, R> WorkerGrpc<St, S, C, E, R>
where
    St: Core,
{
    /// Who is calling: the bearer token of the `authorization` metadata, verified and resolved to a
    /// member of a tenant. The returned value holds the tenant's concurrency slot until dropped.
    async fn authenticate<T>(&self, request: &Request<T>) -> Result<Authenticated, Status> {
        let token = self.bearer(request)?;
        let authenticated = authenticate_bearer(&self.core, token)
            .await
            .map_err(|e| api(&e))?;
        let _ = DENIAL.try_with(|note| {
            *note
                .principal
                .lock()
                .unwrap_or_else(PoisonError::into_inner) =
                Some(authenticated.session.principal.clone());
        });
        Ok(authenticated)
    }

    fn bearer<'a, T>(&self, request: &'a Request<T>) -> Result<&'a str, Status> {
        let mut values = request.metadata().get_all("authorization").iter();
        let (Some(value), None) = (values.next(), values.next()) else {
            return Err(api(&ApiError::missing_credentials()));
        };
        let value = value
            .to_str()
            .map_err(|_| api(&ApiError::invalid_credentials()))?;
        parse_bearer(value).map_err(|e| api(&e))
    }
}

impl<St, S, C, E, R> WorkerGrpc<St, S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore + 'static,
    C: Clock + 'static,
    E: ExpressionEvaluator + 'static,
    R: Entropy + 'static,
{
    /// Where a lease that was committed and not handed to the transport goes (decision D006-001).
    fn withdrawal_sink(&self) -> WithdrawalSink {
        WithdrawalSink::new(Arc::clone(&self.workers), self.withdrawals.clone())
    }
}

#[tonic::async_trait]
impl<St, S, C, E, R> pb::worker_service_server::WorkerService for WorkerGrpc<St, S, C, E, R>
where
    St: Core,
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore + 'static,
    C: Clock + 'static,
    E: ExpressionEvaluator + 'static,
    R: Entropy + 'static,
{
    type StreamTasksStream =
        Pin<Box<dyn Stream<Item = Result<pb::PollTasksResponse, Status>> + Send + 'static>>;

    async fn register_worker(
        &self,
        request: Request<pb::RegisterWorkerRequest>,
    ) -> Result<Response<pb::RegisterWorkerResponse>, Status> {
        observe(&self.core, "RegisterWorker", trace_of(&request), async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let wanted = message
                .capabilities
                .as_ref()
                .ok_or_else(|| invalid("capabilities are required"))?;
            let capabilities = Capabilities {
                activities: wanted.activities.clone(),
                languages: wanted.languages.clone(),
                pools: wanted.pools.clone(),
                max_inflight: u32::try_from(wanted.max_inflight)
                    .map_err(|_| invalid("max_inflight must be positive"))?,
                protocol_version: wanted.protocol_version.clone(),
                // protocol 1.8: stored on the session from each registration (Task 006-048)
                declines_acknowledgement: wanted.declines_acknowledgement,
            };
            let registered = self
                .workers
                .register(
                    &auth.session.principal,
                    &message.namespace,
                    &message.worker_name,
                    &capabilities,
                )
                .await
                .map_err(app)?;
            // Which version workers are served at: the operator's answer to "can an old minor be dropped".
            // The accepted version is one of the few the server speaks, so the label stays small.
            metrics::counter!(
                "worker_registrations_total",
                "protocol" => registered.accepted_protocol_version.clone()
            )
            .increment(1);
            Ok(pb::RegisterWorkerResponse {
                worker_id: registered.worker_id.to_string(),
                session_token: registered.session_token.expose().to_owned(),
                accepted_protocol_version: registered.accepted_protocol_version,
                // Feature 041: sent only at an accepted minor of 7, which T041-7 offers.
                step_logs: None,
            })
        })
        .await
    }

    async fn poll_tasks(
        &self,
        request: Request<pb::PollTasksRequest>,
    ) -> Result<Response<pb::PollTasksResponse>, Status> {
        let trace = trace_of(&request);
        // A poll that carries no `traceparent` is a trace of its own that nobody asked for. The
        // leases it makes belong to the traces of the runs they are for, and the timeline of a run
        // says so: only a poll that continues a trace puts its own on the lease.
        let request_trace = trace
            .is_continuation()
            .then(|| trace.trace_id().to_string());
        observe(&self.core, "PollTasks", trace, async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let worker = worker_id(&message.worker_id)?;
            let max_tasks = u32::try_from(message.max_tasks)
                .map_err(|_| invalid("max_tasks must not be negative"))?;
            // The claim runs in a task of its own: a client that goes away drops this handler, and with it, before,
            // the leases the claim had already committed. Its leases come back here in a guard, or, when the handler is
            // gone, the guard is dropped with the task's output and they are withdrawn (decisions D006-001, D006-017). The
            // claim leases no further run once the handler is gone (D006-035), keeps the tenant's slot until it ends
            // (D006-037), and runs on the tracker a graceful shutdown waits for (D006-036).
            let workers = Arc::clone(&self.workers);
            let principal = auth.session.principal.clone();
            let slot = auth.slot;
            let session_token = message.session_token.clone();
            let sink = self.withdrawal_sink();
            // cancelled when this handler is dropped: what the claim and a probe watch (decision D006-008)
            let abandoned = CancellationToken::new();
            let _abandon_on_drop = abandoned.clone().drop_guard();
            let watch = ConsumerWatch {
                gone: abandoned,
                probe: self.probe.clone(),
                commit_probe: self.commit_probe.clone(),
            };
            let claim = self.withdrawals.spawn(async move {
                let _slot = slot;
                let tasks = workers
                    .poll_bounded(
                        &principal,
                        worker,
                        &session_token,
                        max_tasks,
                        sagat_application::worker::MAX_POLL_BYTES,
                        request_trace.as_deref(),
                        &watch,
                    )
                    .await?;
                let leases = PendingLeases::of(principal.tenant_id(), &tasks, &sink);
                Ok::<_, AppError>((tasks, leases))
            });
            let (tasks, leases) = claim
                .await
                .map_err(|_| Status::internal("the claim of the poll failed"))?
                .map_err(app)?;
            let retry_after_ms = if tasks.is_empty() {
                i32::try_from(self.workers.settings().idle_retry.as_millis()).unwrap_or(i32::MAX)
            } else {
                0
            };
            let response = pb::PollTasksResponse {
                tasks: tasks.iter().map(lease).collect(),
                retry_after_ms,
            };
            // handed to the transport (gap G006-001 if the answer is lost from here)
            leases.handed();
            Ok(response)
        })
        .await
    }

    async fn stream_tasks(
        &self,
        request: Request<pb::PollTasksRequest>,
    ) -> Result<Response<Self::StreamTasksStream>, Status> {
        let trace = trace_of(&request);
        let request_trace = trace
            .is_continuation()
            .then(|| trace.trace_id().to_string());
        observe(&self.core, "StreamTasks", trace, async {
            let message = request.get_ref();
            let max_tasks = u32::try_from(message.max_tasks)
                .map_err(|_| invalid("max_tasks must be between 1 and 64"))?;
            if !(1..=64).contains(&max_tasks) {
                return Err(invalid("max_tasks must be between 1 and 64"));
            }
            let worker = worker_id(&message.worker_id)?;
            let bearer = self.bearer(&request)?.to_owned();
            let connection = capped_connection(&request);
            let auth = self.authenticate(&request).await?;
            self.workers
                .stream_session(&auth.session.principal, worker, &message.session_token)
                .await
                .map_err(app)?;
            let tenant = auth.session.principal.tenant_id();
            let subject = auth.session.principal.subject().as_str().to_owned();
            let expires_at = auth.expires_at;
            let ticket = self
                .task_streams
                .admit(tenant, &subject)
                .map_err(|refused| {
                    metrics::counter!(
                        "worker_task_streams_refused_total",
                        "scope" => refused.label()
                    )
                    .increment(1);
                    metrics::counter!(
                        "worker_task_streams_closed_total",
                        "reason" => "refused"
                    )
                    .increment(1);
                    api(&ApiError::rate_limited(Duration::from_secs(1)))
                })?;
            // The normal request permit belongs only to authentication/session validation. The
            // dedicated ticket below owns the response-body lifetime.
            drop(auth);

            let session_token = message.session_token.clone();
            let core = self.core.clone();
            let workers = Arc::clone(&self.workers);
            let admission = self.admission.clone();
            let idle_retry = workers.settings().idle_retry;
            let (sender, receiver) = mpsc::channel(1);
            let reason = Arc::new(AtomicU8::new(1));
            let producer_reason = Arc::clone(&reason);
            let admitted_at = tokio::time::Instant::now();
            let now = OffsetDateTime::now_utc();
            let expiry_nanos = i128::from(expires_at) * 1_000_000_000;
            let until_expiry_nanos = expiry_nanos
                .saturating_sub(now.unix_timestamp_nanos())
                .max(0);
            let until_expiry =
                Duration::from_nanos(u64::try_from(until_expiry_nanos).unwrap_or(u64::MAX));
            let token_deadline = admitted_at + until_expiry;
            let window_deadline = admitted_at + STREAM_WINDOW;
            let token_expires_first = token_deadline <= window_deadline;
            let body_deadline = window_deadline.min(token_deadline);
            // Finish production just before the hard body deadline. A normal reader receives its
            // terminal hint; a stalled transport is cut at the hard deadline and exposes no old
            // buffered frame afterwards.
            let producer_end = producer_end(body_deadline, token_expires_first);
            let stop = CancellationToken::new();
            let stopped = stop.clone();
            let sink = self.withdrawal_sink();
            let watch = ConsumerWatch {
                gone: stop.clone(),
                probe: self.probe.clone(),
                commit_probe: self.commit_probe.clone(),
            };
            // on the tracker a graceful shutdown waits for, so that a claim in progress and its withdrawal end before
            // the pool closes (decision D006-036)
            let producer = self.withdrawals.spawn(async move {
                let mut credit = max_tasks;
                let mut bytes_left = sagat_application::worker::MAX_POLL_BYTES;
                loop {
                    if stopped.is_cancelled() {
                        break;
                    }
                    if credit == 0 {
                        producer_reason.store(2, Ordering::Release);
                        break;
                    }
                    if bytes_left == 0 {
                        tokio::select! {
                            biased;
                            () = stopped.cancelled() => {}
                            reserved = tokio::time::timeout_at(producer_end, sender.clone().reserve_owned()) => {
                                if let Ok(Ok(place)) = reserved {
                                    place.send(Frame::plain(Ok(pb::PollTasksResponse {
                                        tasks: Vec::new(),
                                        retry_after_ms: i32::try_from(idle_retry.as_millis())
                                            .unwrap_or(i32::MAX)
                                            .max(1),
                                    })));
                                }
                            }
                        }
                        producer_reason.store(3, Ordering::Release);
                        break;
                    }
                    if tokio::time::Instant::now() >= producer_end {
                        if let Ok(place) = sender.clone().try_reserve_owned() {
                            place.send(Frame::plain(stream_deadline_frame(
                                token_expires_first,
                                idle_retry,
                            )));
                        }
                        producer_reason
                            .store(if token_expires_first { 5 } else { 4 }, Ordering::Release);
                        break;
                    }
                    // Reserve the sole queue place before authentication or a database claim.
                    let place = tokio::select! {
                        biased;
                        () = stopped.cancelled() => break,
                        reserved = tokio::time::timeout_at(producer_end, sender.clone().reserve_owned()) => {
                            match reserved {
                                Ok(Ok(place)) => place,
                                Ok(Err(_)) => break,
                                Err(_) => {
                                    producer_reason.store(
                                        if token_expires_first { 5 } else { 4 },
                                        Ordering::Release,
                                    );
                                    break;
                                }
                            }
                        }
                    };
                    if let Some(refusal) = admission.not_serving() {
                        place.send(Frame::plain(Err(api(&refusal))));
                        producer_reason.store(7, Ordering::Release);
                        break;
                    }
                    let Some(claim_ticket) = admission.admit_discovery() else {
                        metrics::counter!(
                            "worker_task_streams_refused_total",
                            "scope" => "discovery"
                        )
                        .increment(1);
                        metrics::counter!(
                            "worker_task_stream_claims_total",
                            "outcome" => "error"
                        )
                        .increment(1);
                        place.send(Frame::plain(Err(api(&ApiError::rate_limited(
                            Duration::from_secs(1),
                        )))));
                        producer_reason.store(6, Ordering::Release);
                        break;
                    };
                    let authenticated = tokio::select! {
                        biased;
                        () = stopped.cancelled() => break,
                        authenticated = tokio::time::timeout_at(
                            producer_end,
                            authenticate_bearer(&core, &bearer),
                        ) => match authenticated {
                            Ok(Ok(authenticated)) => authenticated,
                            Ok(Err(error)) => {
                                let status = api(&error);
                                producer_reason.store(
                                    match status.code() {
                                        Code::Unauthenticated | Code::PermissionDenied => 5,
                                        Code::ResourceExhausted => 6,
                                        Code::Unavailable => 7,
                                        _ => 8,
                                    },
                                    Ordering::Release,
                                );
                                place.send(Frame::plain(Err(status)));
                                metrics::counter!(
                                    "worker_task_stream_claims_total",
                                    "outcome" => "error"
                                )
                                .increment(1);
                                break;
                            }
                            Err(_) => {
                                place.send(Frame::plain(stream_deadline_frame(
                                    token_expires_first,
                                    idle_retry,
                                )));
                                producer_reason
                                    .store(if token_expires_first { 5 } else { 4 }, Ordering::Release);
                                break;
                            }
                        }
                    };
                    // The claim is never cut off: it commits one transaction per run, and a claim dropped between two
                    // commits would lose the leases it had committed (decision D006-002). The store's statement and
                    // lock timeouts bound it; what it returns past the window goes to the withdrawal below.
                    let claimed = workers
                        .poll_bounded(
                            &authenticated.session.principal,
                            worker,
                            &session_token,
                            credit,
                            bytes_left,
                            request_trace.as_deref(),
                            &watch,
                        )
                        .await;
                    let tasks = match claimed {
                        Ok(tasks) => tasks,
                        Err(error) => {
                            // The role may have been lowered since the stream opened: the
                            // refusal is a denial like any other, but this task is outside the
                            // call that `observe` watches.
                            let refusal = ApiError::from(error);
                            if let Some(denial) = refusal.access_denial() {
                                let ctx = RequestContext {
                                    trace_id: request_trace.clone(),
                                    span_id: None,
                                };
                                let _ = sagat_api_http::denial::spawn_audit_access_denied(
                                    &core,
                                    &authenticated.session.principal,
                                    &rpc_name("StreamTasks"),
                                    denial,
                                    &ctx,
                                )
                                .await;
                            }
                            let status = api(&refusal);
                            producer_reason.store(
                                match status.code() {
                                    Code::Unauthenticated | Code::PermissionDenied => 5,
                                    Code::Unavailable => 7,
                                    _ => 8,
                                },
                                Ordering::Release,
                            );
                            place.send(Frame::plain(Err(status)));
                            metrics::counter!(
                                "worker_task_stream_claims_total",
                                "outcome" => "error"
                            )
                            .increment(1);
                            break;
                        }
                    };
                    drop(authenticated);
                    drop(claim_ticket);
                    if tasks.is_empty() {
                        drop(place);
                        metrics::counter!(
                            "worker_task_stream_claims_total",
                            "outcome" => "empty"
                        )
                        .increment(1);
                        tokio::select! {
                            biased;
                            () = stopped.cancelled() => break,
                            () = tokio::time::sleep_until(
                                (tokio::time::Instant::now() + idle_retry).min(producer_end),
                            ) => {}
                        }
                        continue;
                    }
                    // From the commit of the claim the leases travel in a guard: dropped before the frame is handed to the
                    // transport, they are withdrawn (decision D006-001).
                    let leases = PendingLeases::of(tenant, &tasks, &sink);
                    let used = tasks.iter().map(|task| task.input.len()).sum::<usize>();
                    credit = credit.saturating_sub(tasks.len() as u32);
                    bytes_left = bytes_left.saturating_sub(used);
                    if stopped.is_cancelled() {
                        // the body is gone or past its window: nothing more is queued
                        drop(place);
                        drop(leases);
                        break;
                    }
                    // counted only when queued: a lease that went to the withdrawal was never streamed
                    metrics::counter!(
                        "worker_task_stream_claims_total",
                        "outcome" => "ready"
                    )
                    .increment(1);
                    metrics::counter!("worker_task_stream_tasks_total")
                        .increment(tasks.len() as u64);
                    place.send(Frame {
                        item: Ok(pb::PollTasksResponse {
                            tasks: tasks.iter().map(lease).collect(),
                            retry_after_ms: 0,
                        }),
                        leases,
                    });
                }
            });

            let body: Self::StreamTasksStream = Box::pin(TaskBody {
                inner: receiver,
                producer,
                stop,
                _ticket: Some(ticket),
                _connection_deadline: connection
                    .map(|connection| connection.arm(body_deadline + Duration::from_millis(10))),
                token_expiry: Box::pin(tokio::time::sleep_until(token_deadline)),
                window_deadline: Box::pin(tokio::time::sleep_until(body_deadline)),
                expiry_reported: false,
                deadline_reported: false,
                terminal_delivered: false,
                reason,
                started: Instant::now(),
            });
            Ok(body)
        })
        .await
    }

    async fn resolve_secrets(
        &self,
        request: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolveSecretsResponse>, Status> {
        let trace = trace_of(&request);
        observe(&self.core, "ResolveSecrets", trace, async {
            // a silent registry serves no capability call (decision D006-098)
            if let Some(refused) = self.admission.capability_refusal("ResolveSecrets") {
                return Err(refused);
            }
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let mut attempt = Attempt::parse(
                &message.task_id,
                &message.run_id,
                &message.step_id,
                message.attempt,
                message.generation,
                &message.lease_token,
            )?;
            attach_instance(
                &mut attempt,
                message.instance.as_ref(),
                self.workers.supports_scoped_worker_protocol(),
            )?;
            let resolved = self
                .workers
                .resolve_secrets(
                    &auth.session.principal,
                    &attempt.task_ref(),
                    Some(&trace.trace_id().to_string()),
                )
                .await
                .map_err(app)?;
            // asked again before the first byte of the values (decision D006-098)
            if let Some(refused) = self.admission.capability_refusal("ResolveSecrets") {
                return Err(refused);
            }
            Ok(pb::ResolveSecretsResponse {
                secrets: resolved
                    .iter()
                    .map(|secret| pb::ResolvedSecret {
                        name: secret.name.to_string(),
                        value: secret.value.expose().to_vec(),
                        version: secret.version,
                    })
                    .collect(),
            })
        })
        .await
    }

    // Feature 041 builds this in T041-7 (`crates/api-grpc/src/step_logs.rs`); until then a
    // worker that sends a batch stops capture for the attempt, as against an older replica.
    async fn append_step_logs(
        &self,
        _: Request<pb::AppendStepLogsRequest>,
    ) -> Result<Response<pb::AppendStepLogsResponse>, Status> {
        Err(Status::unimplemented("AppendStepLogs"))
    }
    // Protocol 1.8 (delivery acknowledgement): served from Task 006-049; until then, as against an older replica.
    async fn acknowledge_tasks(
        &self,
        _: Request<pb::AcknowledgeTasksRequest>,
    ) -> Result<Response<pb::AcknowledgeTasksResponse>, Status> {
        Err(Status::unimplemented("AcknowledgeTasks"))
    }
    async fn resolve_database(
        &self,
        request: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolvedDatabase>, Status> {
        let trace = trace_of(&request);
        observe(&self.core, "ResolveDatabase", trace, async {
            if let Some(refused) = self.admission.capability_refusal("ResolveDatabase") {
                return Err(refused);
            }
            let auth = self.authenticate(&request).await?;
            let m = request.get_ref();
            let mut attempt = Attempt::parse(
                &m.task_id,
                &m.run_id,
                &m.step_id,
                m.attempt,
                m.generation,
                &m.lease_token,
            )?;
            attach_instance(
                &mut attempt,
                m.instance.as_ref(),
                self.workers.supports_scoped_worker_protocol(),
            )?;
            let resolved = self
                .workers
                .resolve_database(
                    &auth.session.principal,
                    &attempt.task_ref(),
                    Some(&trace.trace_id().to_string()),
                )
                .await
                .map_err(app)?;
            if let Some(refused) = self.admission.capability_refusal("ResolveDatabase") {
                return Err(refused);
            }
            crate::database::response(resolved).map_err(app)
        })
        .await
    }

    async fn heartbeat(
        &self,
        request: Request<pb::HeartbeatRequest>,
    ) -> Result<Response<pb::HeartbeatResponse>, Status> {
        observe(&self.core, "Heartbeat", trace_of(&request), async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            if !message.worker_id.is_empty() {
                worker_id(&message.worker_id)?;
            }
            let mut attempt = Attempt::parse_heartbeat(
                &message.task_id,
                &message.run_id,
                &message.step_id,
                message.attempt,
                message.generation,
                &message.lease_token,
            )?;
            attach_instance(
                &mut attempt,
                message.instance.as_ref(),
                self.workers.supports_scoped_worker_protocol(),
            )?;
            let ack = self
                .workers
                .heartbeat(&auth.session.principal, &attempt.task_ref())
                .await
                .map_err(app)?;
            Ok(pb::HeartbeatResponse {
                // `0` when this lease already reported the attempt (G141, SDK SPEC §6.4).
                lease_expires_at_epoch_ms: ack.lease_expires_at.map_or(0, epoch_ms),
                cancel_requested: ack.cancel_requested,
            })
        })
        .await
    }

    async fn complete_step(
        &self,
        request: Request<pb::CompleteStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        observe(&self.core, "CompleteStep", trace_of(&request), async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let mut attempt = Attempt::parse(
                &message.task_id,
                &message.run_id,
                &message.step_id,
                message.attempt,
                message.generation,
                &message.lease_token,
            )?;
            attach_instance(
                &mut attempt,
                message.instance.as_ref(),
                self.workers.supports_scoped_worker_protocol(),
            )?;
            let key = idempotency_key(&message.idempotency_key)?;
            if !message.output_blob_ref.is_empty() {
                if !message.output_json.is_empty() {
                    return Err(invalid("send output_json or output_blob_ref, not both"));
                }
                let blobs = self.blob_access()?;
                let id = message
                    .output_blob_ref
                    .parse::<BlobId>()
                    .map_err(|_| invalid("output_blob_ref is not a blob id"))?;
                let principal = &auth.session.principal;
                let scope = match self
                    .workers
                    .prepare_blob_completion(
                        principal,
                        &attempt.task_ref(),
                        id,
                        &message.output_sha256,
                        key,
                    )
                    .await
                    .map_err(app)?
                {
                    BlobCompletionTarget::Replayed(ack) => return Ok(step_response(&ack)),
                    BlobCompletionTarget::Ready(scope) => scope,
                };
                let described = blobs
                    .service
                    .describe_for_commit(scope, id)
                    .await
                    .map_err(blob_failures::read_failure)?;
                if !message.output_sha256.is_empty()
                    && !message
                        .output_sha256
                        .eq_ignore_ascii_case(&described.digest().to_hex())
                {
                    return Err(invalid("output_sha256 does not match the blob"));
                }
                let limit = self
                    .workers
                    .database_blob_limit(principal, &attempt.task_ref())
                    .await
                    .map_err(app)?;
                let mut _place = None;
                let content = if let Some(limit) = limit {
                    if described.size() > limit as u64 {
                        return Err(invalid("managed database result is too large"));
                    }
                    _place = Some(blobs.permit()?);
                    let (actual, body) = blobs
                        .service
                        .read(scope.access, id)
                        .await
                        .map_err(read_failure)?;
                    if actual != described {
                        return Err(invalid("managed blob descriptor changed"));
                    }
                    Some(blob_failures::collect_database_result(body, limit, blobs.timeout).await?)
                } else {
                    None
                };
                let ack = self
                    .workers
                    .complete_with_blob(
                        principal,
                        &attempt.task_ref(),
                        BlobOutput {
                            descriptor: &described,
                            expected_digest: &message.output_sha256,
                            content: content.as_ref().map(|bytes| bytes.as_slice()),
                        },
                        key,
                    )
                    .await
                    .map_err(app)?;
                return Ok(step_response(&ack));
            }
            let output = output_of(&message.output_json, &message.output_sha256)?;
            let ack = self
                .workers
                .complete(&auth.session.principal, &attempt.task_ref(), &output, key)
                .await
                .map_err(app)?;
            Ok(step_response(&ack))
        })
        .await
    }

    async fn write_blob(
        &self,
        request: Request<tonic::Streaming<pb::WriteBlobRequest>>,
    ) -> Result<Response<pb::WriteBlobResponse>, Status> {
        let trace = trace_of(&request);
        observe(&self.core, "WriteBlob", trace, async {
            let started = Instant::now();
            if let Some(refused) = self.admission.capability_refusal("WriteBlob") {
                return Err(refused);
            }
            let auth = self.authenticate(&request).await?;
            let blobs = self.blob_access()?;
            let _place = blobs.permit()?;
            let mut stream = request.into_inner();

            // The header comes first and names the attempt; nothing is read before the lease holds.
            let first = tokio::time::timeout(
                blobs.timeout.saturating_sub(started.elapsed()),
                stream.message(),
            )
            .await
            .map_err(|_| to_status(&ApiError::request_timeout()))?
            .map_err(|_| invalid("the upload could not be read"))?;
            let Some(pb::write_blob_request::Part::Header(header)) = first.and_then(|m| m.part)
            else {
                return Err(invalid("the first message of an upload must be the header"));
            };
            let mut attempt = Attempt::parse(
                &header.task_id,
                &header.run_id,
                &header.step_id,
                header.attempt,
                header.generation,
                &header.lease_token,
            )?;
            attach_instance(
                &mut attempt,
                header.instance.as_ref(),
                self.workers.supports_scoped_worker_protocol(),
            )?;
            let principal = &auth.session.principal;
            let write = write_request(&header)?;
            let limit = self
                .workers
                .database_blob_limit(principal, &attempt.task_ref())
                .await
                .map_err(app)?;
            if limit.is_some() && write.content_type.as_str() != "application/json" {
                return Err(invalid("managed database results require application/json"));
            }
            let scope = if limit.is_none() {
                Some(
                    self.workers
                        .check_blob_upload(principal, &attempt.task_ref())
                        .await
                        .map_err(app)?,
                )
            } else {
                None
            };
            if limit
                .is_some_and(|limit| write.expected_size.is_some_and(|size| size > limit as u64))
            {
                return Err(invalid("managed database result is too large"));
            }
            let (mut body, flags) =
                upload_body(stream, blobs.timeout.saturating_sub(started.elapsed()));
            let scope = if let Some(limit) = limit {
                let collected = blob_failures::collect_database_result(
                    body,
                    limit,
                    blobs.timeout.saturating_sub(started.elapsed()),
                )
                .await;
                if flags.timed_out() {
                    return Err(to_status(&ApiError::request_timeout()));
                }
                if flags.violation() {
                    return Err(invalid(
                        "after the header an upload carries only chunks of at most 256 KiB",
                    ));
                }
                let content = collected?;
                if started.elapsed() >= blobs.timeout {
                    return Err(to_status(&ApiError::request_timeout()));
                }
                let scope = tokio::time::timeout(
                    blobs.timeout.saturating_sub(started.elapsed()),
                    self.workers.check_database_blob_upload(
                        principal,
                        &attempt.task_ref(),
                        &content,
                    ),
                )
                .await
                .map_err(|_| to_status(&ApiError::request_timeout()))?
                .map_err(app)?;
                body = Box::pin(tokio_stream::iter([Ok(bytes::Bytes::copy_from_slice(
                    &content,
                ))]));
                scope
            } else {
                scope.ok_or_else(|| app(AppError::Internal))?
            };
            // asked again before the store's write starts (decision D006-098)
            if let Some(refused) = self.admission.capability_refusal("WriteBlob") {
                return Err(refused);
            }
            let storing = blobs
                .service
                .write(scope, principal.subject().as_str(), write, body);
            let stored = if limit.is_some() {
                if started.elapsed() >= blobs.timeout {
                    return Err(to_status(&ApiError::request_timeout()));
                }
                // A cancelled PUT can have an unknown outcome. The existing durable reservation
                // and abandoned-upload sweep reconcile it; unconfirmed bytes are never usable.
                tokio::time::timeout(blobs.timeout.saturating_sub(started.elapsed()), storing)
                    .await
                    .map_err(|_| to_status(&ApiError::request_timeout()))?
            } else {
                storing.await
            };
            // Why the body ended early is told by the body; the use case only knows it was cut.
            if flags.timed_out() {
                return Err(to_status(&ApiError::request_timeout()));
            }
            if flags.violation() {
                return Err(invalid(
                    "after the header an upload carries only chunks of at most 256 KiB",
                ));
            }
            let blob = stored.map_err(write_failure)?;
            Ok(pb::WriteBlobResponse {
                blob_id: blob.id().to_string(),
                size: i64::try_from(blob.size()).unwrap_or(i64::MAX),
                sha256: blob.digest().to_hex(),
                content_type: blob.content_type().as_str().to_owned(),
            })
        })
        .await
    }

    type ReadBlobStream = Pin<Box<dyn Stream<Item = Result<pb::ReadBlobResponse, Status>> + Send>>;

    async fn read_blob(
        &self,
        request: Request<pb::ReadBlobRequest>,
    ) -> Result<Response<Self::ReadBlobStream>, Status> {
        let trace = trace_of(&request);
        observe(&self.core, "ReadBlob", trace, async {
            if let Some(refused) = self.admission.capability_refusal("ReadBlob") {
                return Err(refused);
            }
            let auth = self.authenticate(&request).await?;
            let blobs = self.blob_access()?;
            let place = blobs.permit()?;
            let message = request.get_ref();
            let mut attempt = Attempt::parse(
                &message.task_id,
                &message.run_id,
                &message.step_id,
                message.attempt,
                message.generation,
                &message.lease_token,
            )?;
            attach_instance(
                &mut attempt,
                message.instance.as_ref(),
                self.workers.supports_scoped_worker_protocol(),
            )?;
            let id = message
                .blob_id
                .parse::<BlobId>()
                .map_err(|_| invalid("blob_id is not a blob id"))?;
            let principal = &auth.session.principal;
            // The check names the blob: an ordinary step is held to its lease, a lake attempt to the blobs its lease named, and the
            // store is not asked about a blob the check did not allow (D035-144).
            let scope = self
                .workers
                .check_blob_read(
                    principal,
                    &attempt.task_ref(),
                    id,
                    Some(&trace.trace_id().to_string()),
                )
                .await
                .map_err(app)?;
            let (descriptor, body) = blobs
                .service
                .read(scope.access, id)
                .await
                .map_err(read_failure)?;
            self.workers
                .audit_blob_read(
                    principal,
                    &attempt.task_ref(),
                    id,
                    Some(&trace.trace_id().to_string()),
                )
                .await
                .map_err(app)?;
            let header = pb::ReadBlobHeader {
                blob_id: descriptor.id().to_string(),
                size: i64::try_from(descriptor.size()).unwrap_or(i64::MAX),
                sha256: descriptor.digest().to_hex(),
                content_type: descriptor.content_type().as_str().to_owned(),
            };
            // the gate is asked again before each item of the download (decisions D006-098 and D006-100)
            let stream: Self::ReadBlobStream = Box::pin(CheckedFirst {
                inner: Download::new(header, body, place, auth, blobs.timeout),
                admission: self.admission.clone(),
                method: "ReadBlob",
                refused: false,
            });
            Ok(stream)
        })
        .await
    }

    async fn fail_step(
        &self,
        request: Request<pb::FailStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        observe(&self.core, "FailStep", trace_of(&request), async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let mut attempt = Attempt::parse(
                &message.task_id,
                &message.run_id,
                &message.step_id,
                message.attempt,
                message.generation,
                &message.lease_token,
            )?;
            attach_instance(
                &mut attempt,
                message.instance.as_ref(),
                self.workers.supports_scoped_worker_protocol(),
            )?;
            let key = idempotency_key(&message.idempotency_key)?;
            if message.error_code.len() > 128 || message.safe_message.len() > 2048 {
                return Err(invalid("error_code or safe_message is too long"));
            }
            let ack = self
                .workers
                .fail(
                    &auth.session.principal,
                    &attempt.task_ref(),
                    &message.error_code,
                    message.retryable,
                    key,
                )
                .await
                .map_err(app)?;
            Ok(step_response(&ack))
        })
        .await
    }
}

/// The inline output of a completed step. An empty output means "no value" and is recorded as JSON
/// `null`; a stated `output_sha256` must match the bytes received (end-to-end integrity).
fn output_of(output_json: &str, stated_sha256: &str) -> Result<Vec<u8>, Status> {
    if output_json.len() > MAX_PAYLOAD_BYTES {
        return Err(invalid("output_json is larger than 256 KiB"));
    }
    if !stated_sha256.is_empty() {
        let digest = Sha256::digest(output_json.as_bytes());
        let actual = digest
            .iter()
            .fold(String::with_capacity(64), |mut hex, byte| {
                use std::fmt::Write as _;
                let _ = write!(hex, "{byte:02x}");
                hex
            });
        if !actual.eq_ignore_ascii_case(stated_sha256) {
            return Err(invalid("output_sha256 does not match output_json"));
        }
    }
    Ok(if output_json.is_empty() {
        b"null".to_vec()
    } else {
        output_json.as_bytes().to_vec()
    })
}

fn step_response(ack: &StepAck) -> pb::StepCommandResponse {
    pb::StepCommandResponse {
        command_id: ack.command_id.clone(),
        already_applied: ack.already_applied,
        step_status: ack.step_status.as_str().to_owned(),
        run_status: ack.run_status.as_str().to_owned(),
    }
}

fn instance_identity(instance: &ExecutionInstanceId) -> pb::ExecutionInstanceIdentity {
    pb::ExecutionInstanceIdentity {
        instance_path: instance
            .path()
            .segments()
            .iter()
            .map(|segment| pb::ScopePathSegment {
                scope_node_id: segment.scope_node().to_string(),
                iteration_index: segment.iteration_index(),
            })
            .collect(),
        definition_node_id: instance.node().to_string(),
    }
}

#[cfg(test)]
fn root_identity(node: &NodeId) -> pb::ExecutionInstanceIdentity {
    instance_identity(&ExecutionInstanceId::root(node.clone()))
}

fn parse_instance_identity(
    identity: Option<&pb::ExecutionInstanceIdentity>,
    expected_node: &NodeId,
) -> Result<ExecutionInstanceId, Status> {
    let Some(identity) = identity else {
        return Ok(ExecutionInstanceId::root(expected_node.clone()));
    };
    if identity.definition_node_id != expected_node.as_str() {
        return Err(invalid(
            "instance.definition_node_id does not match task_id",
        ));
    }
    let mut path = InstancePath::root();
    for segment in &identity.instance_path {
        let scope_node = NodeId::parse(&segment.scope_node_id)
            .map_err(|_| invalid("instance.instance_path has a malformed scope_node_id"))?;
        path = path
            .child(scope_node, segment.iteration_index)
            .map_err(|_| invalid("instance.instance_path exceeds its bounds"))?;
    }
    Ok(ExecutionInstanceId::in_path(path, expected_node.clone()))
}

fn require_supported_identity(
    identity: &ExecutionInstanceId,
    supports_scoped_identity: bool,
) -> Result<(), Status> {
    if identity.path().is_root() || supports_scoped_identity {
        Ok(())
    } else {
        Err(Status::unimplemented(
            "scoped execution identity requires worker protocol 1.5 support",
        ))
    }
}

fn attach_instance(
    attempt: &mut Attempt,
    identity: Option<&pb::ExecutionInstanceIdentity>,
    supports_scoped_identity: bool,
) -> Result<(), Status> {
    let parsed = parse_instance_identity(identity, attempt.instance.node())?;
    require_supported_identity(&parsed, supports_scoped_identity)?;
    attempt.instance = parsed;
    Ok(())
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn old_and_explicit_root_reports_are_accepted() {
        let node = NodeId::parse("charge").expect("node");
        assert!(parse_instance_identity(None, &node).is_ok());
        assert!(parse_instance_identity(Some(&root_identity(&node)), &node).is_ok());
    }

    #[test]
    fn an_unimplemented_scope_or_mismatched_root_is_rejected() {
        let node = NodeId::parse("charge").expect("node");
        let other = NodeId::parse("refund").expect("node");
        assert!(parse_instance_identity(Some(&root_identity(&other)), &node).is_err());

        let scoped = pb::ExecutionInstanceIdentity {
            instance_path: vec![pb::ScopePathSegment {
                scope_node_id: "lines".into(),
                iteration_index: 0,
            }],
            definition_node_id: node.to_string(),
        };
        let parsed = parse_instance_identity(Some(&scoped), &node).expect("valid scoped identity");
        let error = require_supported_identity(&parsed, false)
            .expect_err("scoped execution remains disabled");
        assert_eq!(error.code(), Code::Unimplemented);
        assert!(require_supported_identity(&parsed, true).is_ok());
    }

    #[test]
    fn a_wire_path_is_bounded_and_round_trips_structurally() {
        let node = NodeId::parse("charge").expect("node");
        let identity = pb::ExecutionInstanceIdentity {
            instance_path: (0..8)
                .map(|index| pb::ScopePathSegment {
                    scope_node_id: format!("scope{index}"),
                    iteration_index: 9_999,
                })
                .collect(),
            definition_node_id: node.to_string(),
        };
        let parsed = parse_instance_identity(Some(&identity), &node).expect("bounded identity");
        assert_eq!(parsed.path().depth(), 8);
        assert_eq!(instance_identity(&parsed), identity);

        let mut too_deep = identity;
        too_deep.instance_path.push(pb::ScopePathSegment {
            scope_node_id: "overflow".into(),
            iteration_index: 0,
        });
        assert!(parse_instance_identity(Some(&too_deep), &node).is_err());

        let bad_index = pb::ExecutionInstanceIdentity {
            instance_path: vec![pb::ScopePathSegment {
                scope_node_id: "lines".into(),
                iteration_index: 10_000,
            }],
            definition_node_id: node.to_string(),
        };
        assert!(parse_instance_identity(Some(&bad_index), &node).is_err());
    }

    #[test]
    fn an_absent_identity_keeps_the_legacy_root() {
        let node = NodeId::parse("charge").expect("node");
        let parsed = parse_instance_identity(None, &node).expect("legacy root");
        assert!(parsed.path().is_root());
        assert_eq!(parsed.node(), &node);
    }

    #[test]
    fn a_scoped_lease_carries_the_exact_instance_identity() {
        let run_id = sagat_domain::execution::RunId::new(uuid::Uuid::from_u128(1));
        let node = NodeId::parse("charge").expect("node");
        let scope = NodeId::parse("lines").expect("scope");
        let instance =
            ExecutionInstanceId::in_path(InstancePath::root().child(scope, 7).expect("path"), node);
        let task = ClaimedTask {
            run_id,
            instance: instance.clone(),
            step_id: uuid::Uuid::from_u128(2),
            fence: sagat_domain::execution::Fence {
                attempt: 3,
                generation: 4,
            },
            lease_token: sagat_application::entropy::Token::from_bytes([1; 32]).expect("token"),
            expires_at: OffsetDateTime::UNIX_EPOCH,
            activity: sagat_domain::workflow::ActivityName::parse("payments.charge")
                .expect("activity"),
            input: b"{}".to_vec(),
            input_blob: None,
            idempotency_key: "scoped-charge".into(),
            trace_id: None,
            kind: sagat_application::worker::TaskKind::Forward,
            target: None,
            mark: None,
            attempt_deadline: None,
        };

        let encoded = lease(&task);
        assert_eq!(encoded.instance, Some(instance_identity(&instance)));
        assert_eq!(encoded.step_id, task.step_id.to_string());
        assert_eq!(encoded.attempt, 3);
        assert_eq!(encoded.generation, 4);
    }
}

fn lease(task: &ClaimedTask) -> pb::TaskLease {
    let instance = Some(instance_identity(&task.instance));
    let target_instance = task
        .target
        .as_ref()
        .map(|target| instance_identity(&target.instance));
    pb::TaskLease {
        task_id: task.task_id(),
        run_id: task.run_id.to_string(),
        step_id: task.step_id.to_string(),
        attempt: i32::try_from(task.fence.attempt).unwrap_or(i32::MAX),
        generation: i64::try_from(task.fence.generation).unwrap_or(i64::MAX),
        lease_token: task.lease_token.expose().to_owned(),
        expires_at_epoch_ms: epoch_ms(task.expires_at),
        activity_name: task.activity.to_string(),
        input_json: String::from_utf8_lossy(&task.input).into_owned(),
        input_blob_ref: task.input_blob.map(|id| id.to_string()).unwrap_or_default(),
        traceparent: traceparent(task),
        // The tenant is implicit in the worker's identity; its slug is not disclosed to code that
        // does not need it.
        tenant_alias: String::new(),
        idempotency_key: task.idempotency_key.clone(),
        task_kind: task.kind.as_str().to_owned(),
        target_node: task
            .target
            .as_ref()
            .map(|t| t.instance.node().to_string())
            .unwrap_or_default(),
        target_idempotency_key: task
            .target
            .as_ref()
            .map(|t| t.idempotency_key.clone())
            .unwrap_or_default(),
        instance,
        target_instance,
        // protocol 1.8 (Task 006-047): a lease the claim marked provisional, and its attempt deadline when that comes first
        acknowledge_before_start: task.acknowledge_before_start(),
        attempt_deadline_epoch_ms: task.attempt_deadline.map_or(0, epoch_ms),
    }
}

/// The run's trace continued into the worker: same trace id, and the span of this attempt, the one
/// the projector exports for it, so what the worker does under it (its own spans, the calls it makes
/// to a provider) is a child of the attempt (D162).
fn traceparent(task: &ClaimedTask) -> String {
    let Some(trace) = task
        .trace_id
        .as_deref()
        .and_then(sagat_telemetry::TraceId::parse)
    else {
        return String::new();
    };
    let span = sagat_telemetry::attempt_span_id(task.step_id, task.fence.attempt);
    sagat_telemetry::traceparent(trace, span, true)
}

fn epoch_ms(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos() / 1_000_000).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt;

    #[test]
    fn an_empty_output_is_json_null_and_a_stated_digest_is_checked() {
        assert_eq!(output_of("", "").expect("ok"), b"null");
        assert_eq!(output_of("{\"a\":1}", "").expect("ok"), b"{\"a\":1}");
        // sha256("{}")
        let digest = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
        assert!(output_of("{}", digest).is_ok());
        assert!(output_of("{}", &digest.to_uppercase()).is_ok());
        assert!(output_of("{ }", digest).is_err());
        assert!(output_of("{}", "00").is_err());
    }

    #[test]
    fn oversized_output_is_refused() {
        let big = "1".repeat(MAX_PAYLOAD_BYTES + 1);
        assert!(output_of(&big, "").is_err());
    }

    #[tokio::test]
    async fn a_stalled_body_discards_its_buffer_at_the_transport_deadline() {
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(Frame::plain(Ok(pb::PollTasksResponse {
                tasks: vec![pb::TaskLease::default()],
                retry_after_ms: 0,
            })))
            .await
            .expect("queue frame");
        drop(sender);
        let mut body = TaskBody {
            inner: receiver,
            producer: tokio::spawn(std::future::pending()),
            stop: CancellationToken::new(),
            _ticket: None,
            _connection_deadline: None,
            token_expiry: Box::pin(tokio::time::sleep(Duration::from_secs(60))),
            window_deadline: Box::pin(tokio::time::sleep(Duration::from_millis(20))),
            expiry_reported: false,
            deadline_reported: false,
            terminal_delivered: false,
            reason: Arc::new(AtomicU8::new(1)),
            started: Instant::now(),
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        let status = body
            .next()
            .await
            .expect("deadline status")
            .expect_err("the queued frame is no longer exposed");
        assert_eq!(status.code(), Code::DeadlineExceeded);
        assert!(body.next().await.is_none(), "the stale frame was discarded");
    }
}

#[cfg(test)]
mod terminal_hint_margin {
    use super::*;

    /// Found on 2026-11-05 (contract_grpc_durable_execution, `task_stream_byte_credit_is_cumulative_across_frames`, in a
    /// full CI run and again locally with sixteen CPU burners on a sixteen-core machine): the hint is queued at the end
    /// of the production window and the body discards what is queued when its own deadline fires, so any delay of the
    /// reader or the connection task longer than the margin turns the hint into `DeadlineExceeded`. Under load the hint
    /// took up to 15 ms to reach the reader, and once more than the 10 ms the margin was.
    #[test]
    fn the_hint_is_queued_well_before_the_hard_deadline_of_a_window() {
        let deadline = tokio::time::Instant::now() + STREAM_WINDOW;
        let end = producer_end(deadline, false);
        assert!(end < deadline);
        assert!(
            deadline - end >= Duration::from_millis(100),
            "a reader may be late by tens of milliseconds on a loaded machine: the margin is {:?}",
            deadline - end
        );
        assert!(
            deadline - end <= STREAM_WINDOW / 10,
            "the margin takes from the window the producer has to claim in: {:?}",
            deadline - end
        );
    }

    /// A body whose window ended 10 ms ago with `frame` still queued: the reader was late, not the transport stalled.
    async fn late_body(frame: Option<pb::PollTasksResponse>) -> (TaskBody, mpsc::Sender<Frame>) {
        let (sender, receiver) = mpsc::channel(1);
        if let Some(frame) = frame {
            sender
                .send(Frame::plain(Ok(frame)))
                .await
                .expect("queue the frame");
        }
        let body = TaskBody {
            inner: receiver,
            producer: tokio::spawn(std::future::pending()),
            stop: CancellationToken::new(),
            _ticket: None,
            _connection_deadline: None,
            token_expiry: Box::pin(tokio::time::sleep(Duration::from_secs(60))),
            window_deadline: Box::pin(tokio::time::sleep(Duration::from_millis(20))),
            expiry_reported: false,
            deadline_reported: false,
            terminal_delivered: false,
            reason: Arc::new(AtomicU8::new(1)),
            started: Instant::now(),
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        (body, sender)
    }

    /// The race itself, without a clock: the producer queued the hint in time and the reader came for it after the
    /// body's deadline. The hint carries no task, so handing it over late cannot lose a lease, and the reader sees what
    /// a reader that kept up sees. A frame with tasks is still discarded
    /// (`a_stalled_body_discards_its_buffer_at_the_transport_deadline`).
    #[tokio::test]
    async fn a_terminal_hint_queued_before_the_deadline_reaches_a_reader_that_came_for_it_late() {
        use tokio_stream::StreamExt as _;
        let hint = pb::PollTasksResponse {
            tasks: Vec::new(),
            retry_after_ms: 1000,
        };
        let (mut body, _sender) = late_body(Some(hint)).await;
        let frame = body
            .next()
            .await
            .expect("a last item")
            .expect("the hint, not a deadline status");
        assert!(frame.tasks.is_empty());
        assert_eq!(frame.retry_after_ms, 1000);
        assert!(
            body.next().await.is_none(),
            "the stream ends after the hint"
        );
        assert!(body.terminal_delivered);
        assert_eq!(
            body.reason.load(Ordering::Acquire),
            4,
            "the window ended it, and nothing failed"
        );
    }

    /// The producer says why it ended before it queues nothing more; a hint handed over late must not rewrite that.
    #[tokio::test]
    async fn a_late_hint_of_a_byte_budget_that_ran_out_keeps_the_reason_of_the_producer() {
        use tokio_stream::StreamExt as _;
        let hint = pb::PollTasksResponse {
            tasks: Vec::new(),
            retry_after_ms: 1000,
        };
        let (mut body, _sender) = late_body(Some(hint)).await;
        body.reason.store(3, Ordering::Release);
        body.next().await.expect("a last item").expect("the hint");
        assert_eq!(body.reason.load(Ordering::Acquire), 3);
    }

    /// Only a frame with no task is handed over late. A frame that carries a task and, strangely, a retry hint too, is
    /// discarded like any other: the task would be a lease the reader can no longer be sure to honour.
    #[tokio::test]
    async fn a_frame_with_a_task_is_discarded_even_if_it_carries_a_retry_hint() {
        use tokio_stream::StreamExt as _;
        let (mut body, _sender) = late_body(Some(pb::PollTasksResponse {
            tasks: vec![pb::TaskLease::default()],
            retry_after_ms: 1000,
        }))
        .await;
        let status = body
            .next()
            .await
            .expect("a last item")
            .expect_err("a frame with tasks is not a hint");
        assert_eq!(status.code(), Code::DeadlineExceeded);
    }

    /// A stream whose credit ran out ends without an error when its window ends, and says so in its reason.
    #[tokio::test]
    async fn a_window_whose_credit_ran_out_ends_quietly_and_keeps_its_reason() {
        use tokio_stream::StreamExt as _;
        let (mut body, _sender) = late_body(None).await;
        body.reason.store(2, Ordering::Release);
        assert!(body.next().await.is_none());
        assert_eq!(body.reason.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn a_window_that_ended_with_nothing_queued_is_still_a_deadline_status() {
        use tokio_stream::StreamExt as _;
        let (mut body, _sender) = late_body(None).await;
        let status = body
            .next()
            .await
            .expect("a last item")
            .expect_err("no hint was queued in time");
        assert_eq!(status.code(), Code::DeadlineExceeded);
        assert!(body.next().await.is_none());
    }

    #[tokio::test]
    async fn an_empty_frame_without_a_retry_hint_is_not_a_terminal_hint() {
        use tokio_stream::StreamExt as _;
        let (mut body, _sender) = late_body(Some(pb::PollTasksResponse {
            tasks: Vec::new(),
            retry_after_ms: 0,
        }))
        .await;
        let status = body
            .next()
            .await
            .expect("a last item")
            .expect_err("only a frame that tells the reader when to come back ends a window");
        assert_eq!(status.code(), Code::DeadlineExceeded);
    }

    #[test]
    fn a_token_that_expires_first_ends_production_at_its_own_deadline_with_no_margin() {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        assert_eq!(producer_end(deadline, true), deadline);
    }
}

/// Feature 006 (decisions D006-001, D006-002 and D006-008): a lease the body never handed to the transport goes to the
/// withdrawal, however the frame that carried it was discarded. The queue slot is held by the test: nothing reads it.
#[cfg(test)]
mod withdrawal_of_undelivered_frames {
    use super::*;
    use sagat_application::entropy::Token;
    use sagat_application::worker::UndeliveredLease;
    use sagat_domain::execution::{Fence, RunId};
    use sagat_domain::identity::TenantId;
    use tokio_stream::StreamExt as _;

    fn undelivered(n: u8) -> UndeliveredLease {
        UndeliveredLease {
            tenant_id: TenantId::new(uuid::Uuid::from_u128(1)),
            run_id: RunId::generate(),
            instance: ExecutionInstanceId::root(NodeId::parse("a").expect("node")),
            step_id: uuid::Uuid::from_u128(u128::from(n)),
            fence: Fence {
                attempt: 1,
                generation: u64::from(n),
            },
            lease_token: Token::from_bytes([n; 32]).expect("token"),
        }
    }

    fn frame_with(leases: Vec<UndeliveredLease>, sink: &WithdrawalSink) -> Frame {
        Frame {
            item: Ok(pb::PollTasksResponse {
                tasks: leases.iter().map(|_| pb::TaskLease::default()).collect(),
                retry_after_ms: 0,
            }),
            leases: PendingLeases::recorded(leases, sink),
        }
    }

    fn passed() -> Pin<Box<tokio::time::Sleep>> {
        let now = tokio::time::Instant::now();
        Box::pin(tokio::time::sleep_until(
            now.checked_sub(Duration::from_millis(1)).unwrap_or(now),
        ))
    }

    fn far() -> Pin<Box<tokio::time::Sleep>> {
        Box::pin(tokio::time::sleep(Duration::from_secs(3600)))
    }

    fn body(
        receiver: mpsc::Receiver<Frame>,
        window_deadline: Pin<Box<tokio::time::Sleep>>,
        token_expiry: Pin<Box<tokio::time::Sleep>>,
    ) -> (TaskBody, CancellationToken) {
        let stop = CancellationToken::new();
        (
            TaskBody {
                inner: receiver,
                producer: tokio::spawn(std::future::pending()),
                stop: stop.clone(),
                _ticket: None,
                _connection_deadline: None,
                token_expiry,
                window_deadline,
                expiry_reported: false,
                deadline_reported: false,
                terminal_delivered: false,
                reason: Arc::new(AtomicU8::new(1)),
                started: Instant::now(),
            },
            stop,
        )
    }

    fn fences(kept: &std::sync::Mutex<Vec<UndeliveredLease>>) -> Vec<Fence> {
        kept.lock().expect("lock").iter().map(|l| l.fence).collect()
    }

    #[tokio::test]
    async fn a_frame_discarded_at_the_window_deadline_hands_its_leases_to_the_withdrawal() {
        let (sink, kept) = WithdrawalSink::recording();
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(frame_with(vec![undelivered(1), undelivered(2)], &sink))
            .await
            .expect("queue");
        let (mut body, stop) = body(receiver, passed(), far());
        let status = body
            .next()
            .await
            .expect("a last item")
            .expect_err("the stale frame is not exposed");
        assert_eq!(status.code(), Code::DeadlineExceeded);
        assert_eq!(
            fences(&kept),
            [
                Fence {
                    attempt: 1,
                    generation: 1
                },
                Fence {
                    attempt: 1,
                    generation: 2
                }
            ]
        );
        assert!(
            stop.is_cancelled(),
            "the producer is told to stop, not aborted"
        );
        assert!(!body.producer.is_finished(), "nothing aborted the producer");
        drop(sender);
    }

    #[tokio::test]
    async fn a_queue_drained_at_token_expiry_hands_its_leases_to_the_withdrawal() {
        let (sink, kept) = WithdrawalSink::recording();
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(frame_with(vec![undelivered(3)], &sink))
            .await
            .expect("queue");
        let (mut body, stop) = body(receiver, far(), passed());
        let status = body.next().await.expect("an item").expect_err("expired");
        assert_eq!(status.code(), Code::Unauthenticated);
        assert_eq!(
            fences(&kept),
            [Fence {
                attempt: 1,
                generation: 3
            }]
        );
        assert!(stop.is_cancelled());
        drop(sender);
    }

    #[tokio::test]
    async fn a_body_dropped_with_a_queued_frame_hands_its_leases_to_the_withdrawal() {
        let (sink, kept) = WithdrawalSink::recording();
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(frame_with(vec![undelivered(4)], &sink))
            .await
            .expect("queue");
        let (body, stop) = body(receiver, far(), far());
        drop(body);
        assert_eq!(
            fences(&kept),
            [Fence {
                attempt: 1,
                generation: 4
            }]
        );
        assert!(stop.is_cancelled());
        drop(sender);
    }

    /// The producer reserved its place, the body went away, and the frame is sent into the closed queue: its leases are
    /// withdrawn when the queue is freed.
    #[tokio::test]
    async fn a_frame_sent_into_a_closed_queue_hands_its_leases_to_the_withdrawal() {
        let (sink, kept) = WithdrawalSink::recording();
        let (sender, receiver) = mpsc::channel(1);
        let place = sender.clone().reserve_owned().await.expect("a place");
        let (body, _stop) = body(receiver, far(), far());
        drop(body);
        place.send(frame_with(vec![undelivered(5)], &sink));
        drop(sender);
        assert_eq!(
            fences(&kept),
            [Fence {
                attempt: 1,
                generation: 5
            }]
        );
    }

    #[tokio::test]
    async fn a_frame_handed_to_the_transport_withdraws_nothing() {
        let (sink, kept) = WithdrawalSink::recording();
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(frame_with(vec![undelivered(6)], &sink))
            .await
            .expect("queue");
        let (mut body, _stop) = body(receiver, far(), far());
        let frame = body.next().await.expect("an item").expect("the frame");
        assert_eq!(frame.tasks.len(), 1);
        drop(body);
        drop(sender);
        assert!(
            kept.lock().expect("lock").is_empty(),
            "the frame was handed over"
        );
    }
}

/// What a claim of this API watches (decisions D006-035, D006-092 and D006-008): its consumer, gone when the stream's producer
/// is told to stop or the unary handler is dropped, the test's probe after each run's commit, and the test's probe just before
/// the commit of a run's transaction that leased.
struct ConsumerWatch {
    gone: CancellationToken,
    probe: Option<DeliveryProbe>,
    commit_probe: Option<DeliveryProbe>,
}

impl sagat_application::worker::ClaimWatch for ConsumerWatch {
    fn consumer_gone(&self) -> bool {
        self.gone.is_cancelled()
    }

    fn committed(&self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        match &self.probe {
            Some(probe) => probe(Abandonment(self.gone.clone())),
            None => Box::pin(std::future::ready(())),
        }
    }

    fn before_commit(&self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        match &self.commit_probe {
            Some(probe) => probe(Abandonment(self.gone.clone())),
            None => Box::pin(std::future::ready(())),
        }
    }
}

/// The review of 2026-11-25 (decision D006-100): a capability stream asks the gate before every item, not only at its first
/// poll; once the window lapses the item is dropped, the refusal is the stream's last item, and nothing follows.
#[cfg(test)]
mod checked_items {
    use super::*;
    use sagat_application::instances::ServingGate;
    use tokio_stream::StreamExt as _;

    #[tokio::test]
    async fn a_capability_stream_stops_at_the_first_item_after_the_window_lapses() {
        let gate = Arc::new(ServingGate::closed());
        gate.decided_by_the_registry();
        gate.open(1);
        gate.decided_at(std::time::Instant::now());
        let admission = Admission::new(8, Duration::from_secs(30));
        admission.set_serving_gate(Arc::clone(&gate));
        let (sender, receiver) = mpsc::channel::<Result<u32, Status>>(4);
        let mut stream = CheckedFirst {
            inner: tokio_stream::wrappers::ReceiverStream::new(receiver),
            admission,
            method: "ReadBlob",
            refused: false,
        };
        sender.send(Ok(1)).await.expect("queued");
        assert_eq!(stream.next().await.map(|item| item.ok()), Some(Some(1)));
        // the window lapses during the download: the next item never leaves
        let lapsed = std::time::Instant::now()
            .checked_sub(Duration::from_secs(26))
            .expect("an instant 26 s ago");
        // a decision never moves back: forget it, then decide it 26 s ago
        gate.decided_by_the_registry();
        gate.decided_at(lapsed);
        sender.send(Ok(2)).await.expect("queued");
        sender.send(Ok(3)).await.expect("queued");
        let refused = stream
            .next()
            .await
            .expect("an item")
            .expect_err("the refusal, not the item");
        assert_eq!(refused.code(), Code::Unavailable);
        assert_eq!(
            refused
                .metadata()
                .get(crate::status::ERROR_METADATA)
                .and_then(|value| value.to_str().ok()),
            Some("REGISTRY_SILENT")
        );
        assert!(stream.next().await.is_none(), "nothing follows the refusal");
    }
}

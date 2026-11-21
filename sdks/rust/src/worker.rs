use crate::{
    RetryPolicy, SagatError, TlsOptions, TokenSource, Value,
    activity::*,
    protocol as pb,
    worker_rpc::{Rpc, error_of},
};
use futures_util::{FutureExt as _, future::BoxFuture};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerTransport {
    Auto,
    Poll,
    Stream,
}
#[derive(Clone)]
pub struct WorkerOptions {
    pub grpc_url: String,
    pub namespace: String,
    pub worker_name: String,
    pub token: TokenSource,
    pub max_inflight: usize,
    pub pools: Vec<String>,
    pub protocol_version: String,
    pub transport: WorkerTransport,
    pub timeout: Duration,
    pub heartbeat_interval: Option<Duration>,
    pub retry: RetryPolicy,
    pub tls: TlsOptions,
    pub telemetry: crate::Telemetry,
    pub user_agent_suffix: Option<String>,
}
impl WorkerOptions {
    pub fn new(
        url: impl Into<String>,
        namespace: impl Into<String>,
        name: impl Into<String>,
        token: TokenSource,
    ) -> Self {
        Self {
            grpc_url: url.into(),
            namespace: namespace.into(),
            worker_name: name.into(),
            token,
            max_inflight: 10,
            pools: vec![],
            protocol_version: "1.5".into(),
            transport: WorkerTransport::Auto,
            timeout: Duration::from_secs(30),
            heartbeat_interval: None,
            retry: RetryPolicy::default(),
            tls: TlsOptions::default(),
            telemetry: crate::Telemetry::default(),
            user_agent_suffix: None,
        }
    }
}
impl std::fmt::Debug for WorkerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerOptions")
            .field("endpoint", &"<configured>")
            .field("max_inflight", &self.max_inflight)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}
enum ActivityOutput {
    None,
    Json(String),
    Blob(crate::BlobDescriptor),
}
impl std::fmt::Debug for ActivityOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("NoValue"),
            Self::Json(value) => f
                .debug_struct("JsonOutput")
                .field("bytes", &value.len())
                .finish(),
            Self::Blob(value) => value.fmt(f),
        }
    }
}
impl ActivityOutput {
    fn wire(self) -> (String, String, String) {
        match self {
            Self::None => (String::new(), String::new(), String::new()),
            Self::Json(value) => (value, String::new(), String::new()),
            Self::Blob(value) => (String::new(), value.id.to_string(), value.sha256),
        }
    }
}
type Handler = dyn Fn(ActivityContext, Value) -> BoxFuture<'static, Result<ActivityOutput, ActivityFailure>>
    + Send
    + Sync;
/// Called with the number of leases after a poll answer or a stream frame is read and before it is
/// looked at; `None` except in the tests, which hold the receive loop there while `stop` begins.
type FrameHook = dyn Fn(usize) + Send + Sync;
/// The step failure code of a lease the host received and hands back without starting it
/// (`sdks/SPEC.md` section 6.2). It is never raised to a caller and is not in the error catalog.
const TASK_NOT_STARTED: &str = "TASK_NOT_STARTED";
/// Why a lease the host received is handed back: it arrived after `stop` began, it is beyond the
/// credit of the poll or the receive window or beyond the free slots, it is in the rest of a frame
/// that broke the input budget of the window, or the host could not start it.
const HAND_BACK_STOPPING: &str = "stopping";
const HAND_BACK_CREDIT: &str = "credit";
const HAND_BACK_MALFORMED: &str = "malformed";
const HAND_BACK_NOT_STARTED: &str = "not_started";
/// The serialized task inputs a receive window may carry in all (`stream-completion.md`).
const MAX_STREAM_INPUT_BYTES: usize = 8 << 20;
/// At most this many hand-backs are on their way at once; the others wait their turn in the order
/// received (`sdks/SPEC.md` section 6.2, decision D006-026): a server that answers a poll with
/// thousands of leases does not start thousands of reports.
const MAX_HAND_BACKS: usize = 8;
/// The hand-backs of a host: the queue in the order received, how many of the `MAX_HAND_BACKS`
/// workers drain it, and the keys of the leases on their way, queued ones included. A lease under
/// one of these keys is a duplicate (decision D006-027), but the keys hold no slot.
#[derive(Default)]
struct HandBacks {
    queue: Mutex<VecDeque<HandBackJob>>,
    workers: AtomicUsize,
    keys: Mutex<BTreeSet<TaskKey>>,
}
struct HandBackJob {
    session: Session,
    lease: pb::TaskLease,
    key: TaskKey,
}
impl HandBacks {
    /// Takes one of the `MAX_HAND_BACKS` places of a worker, if one is free.
    fn take_worker(&self) -> bool {
        let mut workers = self.workers.load(Ordering::Acquire);
        while workers < MAX_HAND_BACKS {
            match self.workers.compare_exchange_weak(
                workers,
                workers + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(now) => workers = now,
            }
        }
        false
    }
}
/// A poisoned table or queue is still used: what it holds stays valid, and a lease must never be
/// lost or taken for a new one because another thread panicked.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
struct Running {
    receive: CancellationToken,
    life: CancellationToken,
    join: tokio::task::JoinHandle<()>,
    tracker: TaskTracker,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TaskKey {
    id: String,
    generation: i64,
    instance_path: Vec<(String, u32)>,
}
#[derive(Clone)]
struct Core {
    options: WorkerOptions,
    handlers: Arc<BTreeMap<String, Arc<Handler>>>,
    permits: Arc<Semaphore>,
    tasks: Arc<Mutex<BTreeMap<TaskKey, ActivityContext>>>,
    hand_backs: Arc<HandBacks>,
    wake: Arc<Notify>,
    protocol: Arc<Mutex<String>>,
    failure: tokio::sync::watch::Sender<Option<SagatError>>,
    frame_received: Option<Arc<FrameHook>>,
}
pub struct WorkerHost {
    options: WorkerOptions,
    handlers: BTreeMap<String, Arc<Handler>>,
    permits: Arc<Semaphore>,
    tasks: Arc<Mutex<BTreeMap<TaskKey, ActivityContext>>>,
    hand_backs: Arc<HandBacks>,
    protocol: Arc<Mutex<String>>,
    failure: tokio::sync::watch::Sender<Option<SagatError>>,
    running: Option<Running>,
    frame_received: Option<Arc<FrameHook>>,
}
impl std::fmt::Debug for WorkerHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerHost")
            .field("options", &self.options)
            .field("active", &self.running.is_some())
            .finish_non_exhaustive()
    }
}
fn invalid() -> SagatError {
    SagatError::new(
        "VALIDATION_FAILED",
        "invalid worker configuration or activity registration",
    )
}
fn minor(version: &str) -> Option<u32> {
    if version == "1" {
        return Some(0);
    }
    let value = version.strip_prefix("1.")?;
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}
impl WorkerHost {
    pub fn new(options: WorkerOptions) -> Result<Self, SagatError> {
        if options.user_agent_suffix.as_deref().is_some_and(|s| {
            s.len() > 128 || !s.is_ascii() || s.bytes().any(|b| b.is_ascii_control())
        }) {
            return Err(invalid());
        }
        let url = url::Url::parse(&options.grpc_url).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.port() == Some(0)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
            || !(1..=1000).contains(&options.max_inflight)
            || options.namespace.is_empty()
            || options.namespace.len() > 63
            || !options
                .namespace
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || options.worker_name.is_empty()
            || options.worker_name.len() > 128
            || options.timeout.is_zero()
            || options.timeout > Duration::from_secs(86400)
            || options
                .heartbeat_interval
                .is_some_and(|v| v.is_zero() || v > Duration::from_secs(86400))
            || !(1..=100).contains(&options.retry.max_attempts)
            || options.retry.max_delay > Duration::from_secs(60)
            || options.retry.initial_delay > Duration::from_secs(60)
            || options.protocol_version.is_empty()
            || options.protocol_version.len() > 32
            || !options.protocol_version.is_ascii()
            || options
                .protocol_version
                .bytes()
                .any(|b| b.is_ascii_control())
        {
            return Err(invalid());
        }
        Ok(Self {
            permits: Arc::new(Semaphore::new(options.max_inflight)),
            options,
            handlers: BTreeMap::new(),
            tasks: Arc::default(),
            hand_backs: Arc::default(),
            protocol: Arc::default(),
            failure: tokio::sync::watch::channel(None).0,
            running: None,
            frame_received: None,
        })
    }
    /// Installs the test barrier of the receive loop (`FrameHook`); it takes effect at the next start.
    #[cfg(test)]
    pub(crate) fn on_frame_received(&mut self, hook: impl Fn(usize) + Send + Sync + 'static) {
        self.frame_received = Some(Arc::new(hook));
    }
    pub fn activity<I, O, F, Fut>(&mut self, name: &str, handler: F) -> Result<(), SagatError>
    where
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
        F: Fn(ActivityContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, ActivityFailure>> + Send + 'static,
    {
        if self.running.is_some()
            || name.is_empty()
            || name.len() > 128
            || self.handlers.contains_key(name)
            || self.handlers.len() >= 256
        {
            return Err(invalid());
        }
        let handler = Arc::new(handler);
        self.handlers.insert(
            name.into(),
            Arc::new(move |ctx, input| {
                let handler = handler.clone();
                async move {
                    let input = serde_json::from_value::<I>(input).map_err(|_| {
                        ActivityFailure::permanent(
                            "INPUT_INVALID",
                            "the activity input does not match its declared type",
                        )
                    })?;
                    let kind = ctx.task.task_kind;
                    let capability = ctx.capabilities.clone();
                    let result = handler(ctx, input).await;
                    if let Some(capability) = capability {
                        capability.invalidate();
                    }
                    let output = result?;
                    if kind == TaskKind::Reconciliation
                        && std::any::TypeId::of::<O>() != std::any::TypeId::of::<crate::Verdict>()
                    {
                        return Err(ActivityFailure::permanent(
                            "RECONCILIATION_ANSWER_INVALID",
                            "a reconciler must return a typed verdict",
                        ));
                    }
                    if std::any::TypeId::of::<O>() == std::any::TypeId::of::<()>() {
                        return Ok(ActivityOutput::None);
                    }
                    if let Some(blob) =
                        (&output as &dyn std::any::Any).downcast_ref::<crate::BlobOutput>()
                    {
                        return Ok(ActivityOutput::Blob(blob.descriptor().clone()));
                    }
                    let encoded =
                        crate::json::encode(&output, 256 * 1024).map_err(|error| match error {
                            crate::json::EncodingError::TooLarge => ActivityFailure::permanent(
                                "PAYLOAD_TOO_LARGE",
                                "the activity output exceeds 256 KiB",
                            ),
                            crate::json::EncodingError::Invalid => ActivityFailure::permanent(
                                "OUTPUT_INVALID",
                                "the activity output is not JSON",
                            ),
                        })?;
                    Ok(ActivityOutput::Json(encoded))
                }
                .boxed()
            }),
        );
        Ok(())
    }
    pub fn compensation<I, O, F, Fut>(&mut self, name: &str, handler: F) -> Result<(), SagatError>
    where
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
        F: Fn(ActivityContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, ActivityFailure>> + Send + 'static,
    {
        self.activity(name, handler)
    }
    pub fn reconciler<I, F, Fut>(&mut self, name: &str, handler: F) -> Result<(), SagatError>
    where
        I: DeserializeOwned + Send + 'static,
        F: Fn(ActivityContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<crate::Verdict, ActivityFailure>> + Send + 'static,
    {
        self.activity(name, handler)
    }
    /// Wait for a non-retryable discovery failure. Existing handlers keep their original leases.
    pub async fn wait_for_failure(&self) -> SagatError {
        let mut receiver = self.failure.subscribe();
        loop {
            if let Some(error) = receiver.borrow_and_update().clone() {
                return error;
            }
            if receiver.changed().await.is_err() {
                return SagatError::new("CANCELLED", "the worker host ended");
            }
        }
    }
    pub fn protocol_version(&self) -> String {
        self.protocol.lock().map(|v| v.clone()).unwrap_or_default()
    }
    pub async fn start(&mut self) -> Result<(), SagatError> {
        if let Some(running) = &self.running {
            if running.join.is_finished() {
                return Err(self.failure.borrow().clone().unwrap_or_else(|| {
                    SagatError::new("CANCELLED", "the worker receive loop ended")
                }));
            }
            return Ok(());
        }
        self.failure.send_replace(None);
        if self.handlers.is_empty() {
            return Err(invalid());
        }
        let url = url::Url::parse(&self.options.grpc_url).map_err(|_| invalid())?;
        if url.scheme() == "http"
            && !url.host_str().is_some_and(|h| {
                h == "localhost"
                    || h.parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
        {
            let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                tracing::warn!(
                    host = url.host_str().unwrap_or_default(),
                    "Sagat worker uses plaintext HTTP"
                )
            }));
        }
        let rpc = Rpc::connect(
            &self.options.grpc_url,
            self.options.token.clone(),
            self.options.timeout,
            &self.options.tls,
            self.options.telemetry.clone(),
            self.options.user_agent_suffix.as_deref(),
        )
        .await?;
        let core = Core {
            options: self.options.clone(),
            handlers: Arc::new(self.handlers.clone()),
            permits: self.permits.clone(),
            tasks: self.tasks.clone(),
            hand_backs: self.hand_backs.clone(),
            wake: Arc::default(),
            protocol: self.protocol.clone(),
            failure: self.failure.clone(),
            frame_received: self.frame_received.clone(),
        };
        let session = register(&core, rpc).await?;
        if core.options.transport == WorkerTransport::Stream && session.minor < 4 {
            return Err(SagatError::new(
                "UNSUPPORTED_PROTOCOL",
                "task streaming requires worker protocol 1.4",
            ));
        }
        let receive = CancellationToken::new();
        let life = CancellationToken::new();
        let tracker = TaskTracker::new();
        let join = tokio::spawn(receive_loop(
            core,
            session,
            receive.clone(),
            life.clone(),
            tracker.clone(),
        ));
        self.running = Some(Running {
            receive,
            life,
            join,
            tracker,
        });
        Ok(())
    }
    pub async fn stop(&mut self, grace: Duration) -> Result<(), SagatError> {
        let Some(running) = self.running.take() else {
            return Ok(());
        };
        struct EndLife(CancellationToken);
        impl Drop for EndLife {
            fn drop(&mut self) {
                self.0.cancel();
            }
        }
        let _end_life = EndLife(running.life.clone());
        running.receive.cancel();
        let _ = running.join.await;
        running.tracker.close();
        let drained = tokio::time::timeout(grace, running.tracker.wait())
            .await
            .is_ok();
        if !drained && let Ok(tasks) = self.tasks.lock() {
            for task in tasks.values() {
                task.cancel(CancellationReason::Shutdown);
            }
        }
        running.life.cancel();
        Ok(())
    }
}
impl Drop for WorkerHost {
    fn drop(&mut self) {
        if let Some(running) = &self.running {
            running.receive.cancel();
            running.life.cancel();
            if let Ok(tasks) = self.tasks.lock() {
                for task in tasks.values() {
                    task.cancel(CancellationReason::Shutdown);
                }
            }
        }
    }
}
#[derive(Clone)]
struct Session {
    rpc: Rpc,
    worker_id: String,
    session_token: String,
    minor: u32,
}
async fn register(core: &Core, mut rpc: Rpc) -> Result<Session, SagatError> {
    let body = pb::RegisterWorkerRequest {
        worker_name: core.options.worker_name.clone(),
        namespace: core.options.namespace.clone(),
        capabilities: Some(pb::WorkerCapabilities {
            languages: vec!["rust".into()],
            activities: core.handlers.keys().cloned().collect(),
            pools: core.options.pools.clone(),
            max_inflight: core.options.max_inflight as i32,
            protocol_version: core.options.protocol_version.clone(),
        }),
    };
    let mut refreshed = false;
    for attempt in 0..core.options.retry.max_attempts {
        let request = rpc.request(body.clone(), "").await;
        let result = match request {
            Ok(request) => rpc
                .client
                .register_worker(request)
                .await
                .map(|r| r.into_inner())
                .map_err(|e| error_of("RegisterWorker", e)),
            Err(error) => Err(error),
        };
        match result {
            Ok(reply) => {
                let accepted = minor(&reply.accepted_protocol_version)
                    .filter(|m| Some(*m) <= minor(&core.options.protocol_version))
                    .ok_or_else(|| {
                        SagatError::new("PROTOCOL", "invalid accepted worker protocol")
                    })?;
                // The canonical 1.6 protocol is the 1.5 ordinary-worker path plus managed database
                // RPCs restricted to database-runner identities. Unknown future versions are refused.
                if accepted > 6 {
                    return Err(SagatError::new(
                        "UNSUPPORTED_PROTOCOL",
                        "the accepted worker protocol is newer than this SDK understands",
                    ));
                }
                if reply.worker_id.is_empty() || reply.session_token.is_empty() {
                    return Err(SagatError::new(
                        "PROTOCOL",
                        "incomplete worker registration",
                    ));
                }
                if let Ok(mut value) = core.protocol.lock() {
                    *value = reply.accepted_protocol_version;
                }
                core.options.telemetry.emit(
                    crate::EventKind::Registered,
                    "register",
                    crate::SdkTransport::Grpc,
                    None,
                    None,
                );
                return Ok(Session {
                    rpc,
                    worker_id: reply.worker_id,
                    session_token: reply.session_token,
                    minor: accepted,
                });
            }
            Err(error) => {
                if error.code == "UNAUTHENTICATED"
                    && !error.is_provider_failure()
                    && !refreshed
                    && core.options.token.can_refresh()
                {
                    refreshed = true;
                    continue;
                }
                if !error.may_retry() || attempt + 1 == core.options.retry.max_attempts {
                    return Err(error);
                }
                tokio::time::sleep(core.options.retry.after(attempt, error.retry_after)).await;
            }
        }
    }
    Err(SagatError::new(
        "UNAUTHENTICATED",
        "worker registration could not refresh credentials",
    ))
}
async fn wait(core: &Core, receive: &CancellationToken, delay: Duration) {
    tokio::select! {biased;_=receive.cancelled()=>{},_=core.wake.notified()=>{},_=tokio::time::sleep(delay)=>{}}
}
fn receive_delay(ms: i32) -> Duration {
    let ms = if ms <= 0 { 1000 } else { ms.clamp(50, 60000) } as u64;
    let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]);
    Duration::from_millis((ms * 80 / 100 + ms * jitter * 40 / 25500).clamp(50, 60000))
}
async fn receive_loop(
    core: Core,
    mut session: Session,
    receive: CancellationToken,
    life: CancellationToken,
    tracker: TaskTracker,
) {
    let mut stream_disabled = false;
    let mut failures = 0u32;
    let mut just_registered = false;
    while !receive.is_cancelled() {
        let credit = core.permits.available_permits().min(64);
        if credit == 0 {
            wait(&core, &receive, Duration::from_secs(1)).await;
            continue;
        }
        core.options.telemetry.emit(
            crate::EventKind::Started,
            "receive",
            crate::SdkTransport::Grpc,
            None,
            None,
        );
        let request = pb::PollTasksRequest {
            worker_id: session.worker_id.clone(),
            session_token: session.session_token.clone(),
            max_tasks: credit as i32,
        };
        let streaming = core.options.transport != WorkerTransport::Poll
            && session.minor >= 4
            && !stream_disabled;
        let mut client = session.rpc.client.clone();
        // Set when the server broke the receive contract (credit or input budget): the window ends
        // with VALIDATION_FAILED, which is a receive failure to back off from, never the end of
        // the host.
        let mut contract_broken = false;
        let fetch = async {
            let request = session.rpc.request(request, "").await?;
            if streaming {
                let mut stream = client
                    .stream_tasks(request)
                    .await
                    .map_err(|e| error_of("StreamTasks", e))?
                    .into_inner();
                let mut remaining = credit;
                let mut delay = 1000;
                let mut bytes = 0usize;
                let mut window = BTreeSet::new();
                let deadline =
                    tokio::time::Instant::now() + core.options.timeout.min(Duration::from_secs(6));
                loop {
                    let frame = tokio::time::timeout_at(deadline, stream.message())
                        .await
                        .map_err(|_| SagatError::new("TRANSPORT", "task receive window ended"))?
                        .map_err(|e| error_of("StreamTasks", e))?;
                    let Some(frame) = frame else {
                        break;
                    };
                    if let Some(hook) = &core.frame_received {
                        hook(frame.tasks.len());
                    }
                    delay = frame.retry_after_ms;
                    // A frame is looked at in the same poll that read it: `stop` drops this future
                    // only at an await, so a frame is never lost once read. One read after `stop`
                    // began (another frame was already buffered) is handed back whole.
                    if receive.is_cancelled() {
                        for task in frame.tasks {
                            let why = Some(HAND_BACK_STOPPING);
                            take(&core, &session, &mut window, task, why, &life, &tracker);
                        }
                        break;
                    }
                    // The host runs at most the credit of the window, within its input budget: the
                    // lease that breaks either and the rest of the frame after it are handed back,
                    // and the window ends. The window is read to its end, so that a lease the
                    // server sends beyond the credit is handed back, never left to lapse.
                    let mut why = None;
                    let mut broken = None;
                    for task in frame.tasks {
                        let size = task.input_json.len();
                        if why.is_none() && remaining == 0 {
                            why = Some(HAND_BACK_CREDIT);
                            broken = Some(SagatError::new(
                                "VALIDATION_FAILED",
                                "task stream exceeded receive credit",
                            ));
                        } else if why.is_none() && size > MAX_STREAM_INPUT_BYTES - bytes {
                            why = Some(HAND_BACK_MALFORMED);
                            broken = Some(SagatError::new(
                                "VALIDATION_FAILED",
                                "task stream exceeded its input budget",
                            ));
                        }
                        if take(&core, &session, &mut window, task, why, &life, &tracker) {
                            bytes += size;
                            remaining -= 1;
                        }
                    }
                    if let Some(error) = broken {
                        contract_broken = true;
                        return Err(error);
                    }
                }
                Ok((credit - remaining, delay))
            } else {
                let response = client
                    .poll_tasks(request)
                    .await
                    .map_err(|e| error_of("PollTasks", e))?
                    .into_inner();
                if let Some(hook) = &core.frame_received {
                    hook(response.tasks.len());
                }
                // The host runs at most as many tasks as it asked for, and nothing once `stop`
                // began: the rest of the answer is handed back (`sdks/SPEC.md` section 6.2).
                let stopping = receive.is_cancelled();
                let mut answer = BTreeSet::new();
                let mut count = 0;
                for task in response.tasks {
                    let why = if stopping {
                        Some(HAND_BACK_STOPPING)
                    } else {
                        (count >= credit).then_some(HAND_BACK_CREDIT)
                    };
                    if take(&core, &session, &mut answer, task, why, &life, &tracker) {
                        count += 1;
                    }
                }
                Ok((count, response.retry_after_ms))
            }
        };
        let result: Result<(usize, i32), SagatError> =
            tokio::select! {biased;_=receive.cancelled()=>break,result=fetch=>result};
        match result {
            Ok((count, delay)) => {
                failures = 0;
                just_registered = false;
                if count == 0 || (streaming && count < credit) {
                    wait(&core, &receive, receive_delay(delay)).await;
                }
            }
            Err(error)
                if streaming
                    && error.grpc_status() == Some("UNIMPLEMENTED")
                    && core.options.transport == WorkerTransport::Auto =>
            {
                stream_disabled = true;
                wait(&core, &receive, receive_delay(0)).await;
            }
            Err(error) if error.code == "UNAUTHENTICATED" && !error.is_provider_failure() => {
                if just_registered {
                    wait(
                        &core,
                        &receive,
                        core.options
                            .retry
                            .after(failures, None)
                            .min(Duration::from_secs(30)),
                    )
                    .await;
                    failures = failures.saturating_add(1);
                }
                core.options.telemetry.emit(
                    crate::EventKind::Reconnect,
                    "receive",
                    crate::SdkTransport::Grpc,
                    None,
                    Some(error.code),
                );
                let next = tokio::select! {biased;_=receive.cancelled()=>break,result=register(&core,session.rpc.clone())=>result};
                match next {
                    Ok(next) => {
                        session = next;
                        stream_disabled = false;
                        just_registered = true;
                    }
                    Err(error) if !error.may_retry() => {
                        core.failure.send_replace(Some(error));
                        break;
                    }
                    Err(_) => {
                        wait(&core, &receive, Duration::from_secs(1)).await;
                    }
                }
            }
            Err(error) if !error.may_retry() && !contract_broken => {
                core.failure.send_replace(Some(error));
                break;
            }
            Err(error) => {
                failures = failures.saturating_add(1);
                wait(
                    &core,
                    &receive,
                    core.options
                        .retry
                        .after(failures, None)
                        .min(Duration::from_secs(30))
                        .max(
                            error
                                .retry_after
                                .unwrap_or_default()
                                .min(Duration::from_secs(60)),
                        ),
                )
                .await;
            }
        }
    }
}
fn identity(value: &Option<pb::ExecutionInstanceIdentity>) -> Option<ExecutionInstanceIdentity> {
    value.as_ref().map(|v| ExecutionInstanceIdentity {
        definition_node_id: v.definition_node_id.clone(),
        instance_path: v
            .instance_path
            .iter()
            .map(|p| crate::ScopeSegment {
                scope_node_id: p.scope_node_id.clone(),
                iteration_index: p.iteration_index,
            })
            .collect(),
    })
}
/// What identifies a task while it runs: the legacy task id is `run/node/generation`, which sibling
/// iterations of a collection share, so from protocol 1.5 the instance path is part of it.
fn task_key(lease: &pb::TaskLease, minor: u32) -> TaskKey {
    TaskKey {
        id: lease.task_id.clone(),
        generation: lease.generation,
        instance_path: lease
            .instance
            .as_ref()
            .filter(|_| minor >= 5)
            .map(|identity| {
                identity
                    .instance_path
                    .iter()
                    .map(|part| (part.scope_node_id.clone(), part.iteration_index))
                    .collect()
            })
            .unwrap_or_default(),
    }
}
fn warn_duplicate(task: &str) {
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        tracing::warn!(
            task,
            "a task that is in flight or was already received was leased again: ignored"
        )
    }));
}
/// Dispatches a lease of a poll answer or of a stream receive window, or hands it back when `why`
/// is set, and says whether it took a slot of the credit (it was dispatched, or the host tried to
/// start it). A lease whose key is in flight, or was already seen in the same answer or window, is a
/// duplicate: it is ignored and never reported, since its fence may be the one of the running
/// handler (`sdks/SPEC.md` section 6.2). A poisoned in-flight table is still read for this check;
/// `dispatch` then cannot record the lease in it and hands it back.
fn take(
    core: &Core,
    session: &Session,
    seen: &mut BTreeSet<TaskKey>,
    lease: pb::TaskLease,
    why: Option<&'static str>,
    life: &CancellationToken,
    tracker: &TaskTracker,
) -> bool {
    let key = task_key(&lease, session.minor);
    // A poisoned table is still read: a lease of a running task must never be taken for a new one.
    let in_flight =
        locked(&core.tasks).contains_key(&key) || locked(&core.hand_backs.keys).contains(&key);
    if in_flight || !seen.insert(key.clone()) {
        warn_duplicate(&lease.task_id);
        return false;
    }
    match why {
        Some(why) => {
            hand_back(core, session, lease, key, why, life, tracker);
            false
        }
        None => {
            dispatch(core, session, key, lease, life, tracker);
            true
        }
    }
}
/// Reports a lease the host received and will not run: `FailStep` `TASK_NOT_STARTED`, retryable,
/// before any handler of it runs (`sdks/SPEC.md` section 6.2). The server gives the step back
/// without consuming the attempt when it can; a lease left to lapse would be an unknown outcome.
/// It is the failure report of every task, with its retries, made by one of at most
/// `MAX_HAND_BACKS` workers in the order received; nothing here waits, so the receive loop never
/// blocks on the bound. The workers run in the tracker, so `stop` waits for the queue like a task
/// in flight while the channel is still open.
fn hand_back(
    core: &Core,
    session: &Session,
    lease: pb::TaskLease,
    key: TaskKey,
    why: &'static str,
    life: &CancellationToken,
    tracker: &TaskTracker,
) {
    if !locked(&core.hand_backs.keys).insert(key.clone()) {
        // `take` looked for the key just before, and only the loop hands back: a lease is handed
        // back at most once.
        warn_duplicate(&lease.task_id);
        return;
    }
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
        tracing::warn!(
            task = lease.task_id.as_str(),
            reason = why,
            "the host hands back a task it did not start"
        )
    }));
    locked(&core.hand_backs.queue).push_back(HandBackJob {
        session: session.clone(),
        lease,
        key,
    });
    // Every worker busy: the one that finishes first takes this hand-back up.
    if core.hand_backs.take_worker() {
        tracker.spawn(drain_hand_backs(core.clone(), life.clone()));
    }
}
async fn drain_hand_backs(core: Core, life: CancellationToken) {
    loop {
        loop {
            let Some(job) = locked(&core.hand_backs.queue).pop_front() else {
                break;
            };
            let failure =
                ActivityFailure::retryable(TASK_NOT_STARTED, "the host did not start this task");
            let reported = report(&core, &job.session, &job.lease, None, Err(failure));
            // `stop` gives up after its grace: what is left lapses, the queued hand-backs and the
            // one on its way alike (its call is dropped, so it is never made after `stop`).
            let made = tokio::select! {biased;_=life.cancelled()=>false,_=reported=>true};
            if !made {
                let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    tracing::warn!(
                        task = job.lease.task_id.as_str(),
                        "the host stopped before it handed back a task: its lease lapses"
                    )
                }));
            }
            locked(&core.hand_backs.keys).remove(&job.key);
        }
        core.hand_backs.workers.fetch_sub(1, Ordering::AcqRel);
        // A hand-back queued while this worker was leaving may have found every place taken: it
        // is taken up again here.
        if locked(&core.hand_backs.queue).is_empty() || !core.hand_backs.take_worker() {
            return;
        }
    }
}
fn dispatch(
    core: &Core,
    session: &Session,
    key: TaskKey,
    lease: pb::TaskLease,
    life: &CancellationToken,
    tracker: &TaskTracker,
) {
    let Ok(permit) = core.permits.clone().try_acquire_owned() else {
        // No free slot, although the credit was counted from them: the lease is beyond the free
        // slots (`sdks/SPEC.md` section 6.2, item 2).
        hand_back(core, session, lease, key, HAND_BACK_CREDIT, life, tracker);
        return;
    };
    let document = serde_json::from_str::<Value>(&lease.input_json)
        .ok()
        .filter(Value::is_object);
    let doc = document.as_ref().unwrap_or(&Value::Null);
    let task = TaskInfo {
        activity_name: lease.activity_name.clone(),
        task_id: lease.task_id.clone(),
        run_id: lease.run_id.clone(),
        step_id: lease.step_id.clone(),
        node: doc["node"].as_str().unwrap_or(&lease.step_id).into(),
        attempt: lease.attempt,
        generation: lease.generation,
        idempotency_key: lease.idempotency_key.clone(),
        traceparent: lease.traceparent.clone(),
        task_kind: if session.minor < 2 {
            TaskKind::Forward
        } else {
            match lease.task_kind.as_str() {
                "COMPENSATION" => TaskKind::Compensation,
                "RECONCILIATION" => TaskKind::Reconciliation,
                _ => TaskKind::Forward,
            }
        },
        target_node: if session.minor >= 2 {
            lease.target_node.clone()
        } else {
            String::new()
        },
        target_idempotency_key: if session.minor >= 2 {
            lease.target_idempotency_key.clone()
        } else {
            String::new()
        },
        instance: if session.minor >= 5 {
            identity(&lease.instance)
        } else {
            None
        },
        target_instance: if session.minor >= 5 {
            identity(&lease.target_instance)
        } else {
            None
        },
        steps: Value::Object(
            doc.get("steps")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter_map(|(name, step)| {
                    step.as_object()
                        .and_then(|fields| fields.get("output"))
                        .map(|output| (name.clone(), output.clone()))
                })
                .collect(),
        ),
        item: doc.get("item").cloned(),
        index: doc.get("index").and_then(Value::as_u64),
    };
    let ctx = ActivityContext {
        capabilities: Some(Arc::new(crate::capabilities::Capabilities::new(
            session.rpc.clone(),
            session.minor,
            lease.clone(),
        ))),
        task: Arc::new(task),
        cancellation: CancellationToken::new(),
        reason: Arc::new(AtomicU8::new(0)),
        lost: Arc::new(AtomicBool::new(false)),
        expires: Arc::new(AtomicI64::new(lease.expires_at_epoch_ms)),
    };
    match core.tasks.lock() {
        Ok(mut tasks) => {
            if tasks.contains_key(&key) {
                warn_duplicate(&lease.task_id);
                return;
            }
            tasks.insert(key.clone(), ctx.clone());
        }
        Err(_) => {
            // The in-flight table is poisoned: the task could not be recorded, so it is not started.
            drop(permit);
            hand_back(
                core,
                session,
                lease,
                key,
                HAND_BACK_NOT_STARTED,
                life,
                tracker,
            );
            return;
        }
    }
    let core = core.clone();
    let session = session.clone();
    let life = life.clone();
    core.options.telemetry.emit(
        crate::EventKind::Received,
        "activity",
        crate::SdkTransport::Grpc,
        None,
        None,
    );
    tracker.spawn(async move {
        let started=std::time::Instant::now();
        let _guard=ctx.capabilities.as_ref().map(|cap|crate::capabilities::AttemptGuard(cap.clone()));
        let finish=CancellationToken::new();
        let heartbeat=tokio::spawn(heartbeat(session.clone(),lease.clone(),ctx.clone(),core.options.heartbeat_interval,core.options.telemetry.clone(),finish.clone(),life.clone()));
        let handler=core.handlers.get(&lease.activity_name).cloned();
        let result=match (handler,document) {
            (None,_)=>Err(ActivityFailure::permanent("NO_HANDLER","the worker has no handler for this activity")),
            (_,None)=>Err(ActivityFailure::permanent("INPUT_INVALID","invalid step input document")),
            (Some(handler),Some(document))=>{
                let future=std::panic::catch_unwind(AssertUnwindSafe(||handler(ctx.clone(),document.get("input").cloned().unwrap_or(Value::Null))));
                match future {Ok(future)=>AssertUnwindSafe(future).catch_unwind().await.unwrap_or_else(|_|Err(ActivityFailure::retryable("ACTIVITY_ERROR","the activity panicked"))),Err(_)=>Err(ActivityFailure::retryable("ACTIVITY_ERROR","the activity panicked"))}
            }
        };
        core.options.telemetry.emit(crate::EventKind::ActivityDuration,"activity",crate::SdkTransport::Grpc,Some(started.elapsed()),result.as_ref().err().map(|_|"ACTIVITY_ERROR"));
        if let Some(cap)=&ctx.capabilities {cap.invalidate();}
        if !ctx.lost.load(Ordering::Acquire) && !matches!(&result,Err(e) if e.cancelled && ctx.cancellation_reason()==Some(CancellationReason::Shutdown)) {
            report(&core,&session,&lease,Some(&ctx),result).await;
        }
        finish.cancel();let _=heartbeat.await;
        if let Ok(mut tasks)=core.tasks.lock(){tasks.remove(&key);}
        drop(permit);core.wake.notify_one();
    });
}
async fn heartbeat(
    session: Session,
    lease: pb::TaskLease,
    ctx: ActivityContext,
    override_interval: Option<Duration>,
    telemetry: crate::Telemetry,
    finish: CancellationToken,
    life: CancellationToken,
) {
    let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
    let interval = override_interval.unwrap_or_else(|| {
        Duration::from_millis(
            (lease.expires_at_epoch_ms.saturating_sub(now) / 3).clamp(500, 10000) as u64,
        )
    });
    loop {
        tokio::select! {biased;_=finish.cancelled()=>break,_=life.cancelled()=>break,_=tokio::time::sleep(interval)=>{}}
        let body = pb::HeartbeatRequest {
            task_id: lease.task_id.clone(),
            worker_id: session.worker_id.clone(),
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            run_id: if session.minor >= 1 {
                lease.run_id.clone()
            } else {
                String::new()
            },
            step_id: if session.minor >= 1 {
                lease.step_id.clone()
            } else {
                String::new()
            },
            attempt: if session.minor >= 1 { lease.attempt } else { 0 },
            instance: if session.minor >= 5 {
                lease.instance.clone()
            } else {
                None
            },
        };
        let beat = async {
            let request = session.rpc.request(body, &lease.traceparent).await?;
            session
                .rpc
                .client
                .clone()
                .heartbeat(request)
                .await
                .map(|r| r.into_inner())
                .map_err(|e| error_of("Heartbeat", e))
        };
        let result = tokio::select! {biased;_=finish.cancelled()=>break,_=life.cancelled()=>break,result=beat=>result};
        telemetry.emit(
            crate::EventKind::Heartbeat,
            "heartbeat",
            crate::SdkTransport::Grpc,
            None,
            result.as_ref().err().map(|e| e.code),
        );
        match result {
            Ok(reply) => {
                if reply.lease_expires_at_epoch_ms > 0 {
                    ctx.expires
                        .store(reply.lease_expires_at_epoch_ms, Ordering::Release);
                }
                if reply.cancel_requested {
                    ctx.cancel(CancellationReason::RunCancelled);
                }
            }
            Err(error) if error.code == "STALE_ATTEMPT" => {
                ctx.cancel(CancellationReason::LeaseLost);
                break;
            }
            Err(_) => {}
        }
    }
}
/// Reports the result of a task; `ctx` is `None` for a lease that is handed back, which has no
/// handler to cancel when the lease is no longer this host's.
async fn report(
    core: &Core,
    session: &Session,
    lease: &pb::TaskLease,
    ctx: Option<&ActivityContext>,
    result: Result<ActivityOutput, ActivityFailure>,
) {
    let result = match result {
        Ok(ActivityOutput::Blob(_)) if session.minor < 3 => Err(ActivityFailure::permanent(
            "UNSUPPORTED_PROTOCOL",
            "blob outputs require worker protocol 1.3",
        )),
        result => result,
    };
    let instance = if session.minor >= 5 {
        lease.instance.clone()
    } else {
        None
    };
    let (complete, fail) = match result {
        Ok(output) => {
            let (output_json, output_blob_ref, output_sha256) = output.wire();
            (
                Some(pb::CompleteStepRequest {
                    task_id: lease.task_id.clone(),
                    run_id: lease.run_id.clone(),
                    step_id: lease.step_id.clone(),
                    attempt: lease.attempt,
                    generation: lease.generation,
                    lease_token: lease.lease_token.clone(),
                    idempotency_key: format!("done-{}", lease.task_id),
                    output_json,
                    output_blob_ref,
                    output_sha256,
                    traceparent: lease.traceparent.clone(),
                    instance,
                }),
                None,
            )
        }
        Err(error) => {
            let code = if !error.code.is_empty()
                && error.code.len() <= 64
                && error
                    .code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            {
                error.code
            } else {
                "WORKER_ERROR".into()
            };
            (
                None,
                Some(pb::FailStepRequest {
                    task_id: lease.task_id.clone(),
                    run_id: lease.run_id.clone(),
                    step_id: lease.step_id.clone(),
                    attempt: lease.attempt,
                    generation: lease.generation,
                    lease_token: lease.lease_token.clone(),
                    idempotency_key: format!("failed-{}", lease.task_id),
                    error_code: code,
                    safe_message: error.safe_message.chars().take(512).collect(),
                    retryable: error.retryable,
                    traceparent: lease.traceparent.clone(),
                    instance,
                }),
            )
        }
    };
    for attempt in 0..core.options.retry.max_attempts.min(5) {
        if ctx.is_some_and(|ctx| ctx.lost.load(Ordering::Acquire)) {
            return;
        }
        let mut client = session.rpc.client.clone();
        let result = if let Some(body) = &complete {
            match session.rpc.request(body.clone(), &lease.traceparent).await {
                Ok(request) => client
                    .complete_step(request)
                    .await
                    .map_err(|e| error_of("CompleteStep", e)),
                Err(e) => Err(e),
            }
        } else if let Some(body) = &fail {
            match session.rpc.request(body.clone(), &lease.traceparent).await {
                Ok(request) => client
                    .fail_step(request)
                    .await
                    .map_err(|e| error_of("FailStep", e)),
                Err(e) => Err(e),
            }
        } else {
            return;
        };
        match result {
            Ok(_) => {
                core.options.telemetry.emit(
                    if complete.is_some() {
                        crate::EventKind::Completed
                    } else {
                        crate::EventKind::Failed
                    },
                    "report",
                    crate::SdkTransport::Grpc,
                    None,
                    // A hand-back is told from the other failures by its code; the code of a
                    // handler's failure is not a bounded value and is not carried.
                    fail.as_ref()
                        .filter(|body| body.error_code == TASK_NOT_STARTED)
                        .map(|_| TASK_NOT_STARTED),
                );
                return;
            }
            Err(error) if error.code == "STALE_ATTEMPT" => {
                core.options.telemetry.emit(
                    crate::EventKind::Stale,
                    "report",
                    crate::SdkTransport::Grpc,
                    None,
                    Some(error.code),
                );
                if let Some(ctx) = ctx {
                    ctx.cancel(CancellationReason::LeaseLost);
                }
                return;
            }
            Err(error)
                if error.may_retry() && attempt + 1 < core.options.retry.max_attempts.min(5) =>
            {
                tokio::time::sleep(core.options.retry.after(attempt, error.retry_after)).await;
            }
            Err(error) => {
                // The report cannot be made: the lease lapses (for a hand-back, `sdks/SPEC.md`
                // section 6.2). The task id and the code only, never the lease token.
                let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    tracing::warn!(
                        task = lease.task_id.as_str(),
                        code = error.code,
                        "the report of a task could not be made: its lease lapses"
                    )
                }));
                return;
            }
        }
    }
}

#[cfg(test)]
mod serialization_tests {
    #![allow(clippy::expect_used)]
    use super::*;
    #[tokio::test]
    async fn typed_activity_outputs_refuse_non_finite_numbers_in_nested_values() {
        let options = WorkerOptions::new(
            "http://localhost:1",
            "demo",
            "test",
            TokenSource::bearer("fixture"),
        );
        let mut host = WorkerHost::new(options).expect("host");
        host.activity("bad", |_: ActivityContext, _: Value| async {
            Ok(vec![f64::NAN])
        })
        .expect("register");
        let task = TaskInfo {
            activity_name: "bad".into(),
            task_id: String::new(),
            run_id: String::new(),
            step_id: String::new(),
            node: String::new(),
            attempt: 1,
            generation: 1,
            idempotency_key: String::new(),
            traceparent: String::new(),
            task_kind: TaskKind::Forward,
            target_node: String::new(),
            target_idempotency_key: String::new(),
            instance: None,
            target_instance: None,
            steps: Value::Null,
            item: None,
            index: None,
        };
        let context = ActivityContext {
            capabilities: None,
            task: Arc::new(task),
            cancellation: CancellationToken::new(),
            reason: Arc::new(AtomicU8::new(0)),
            lost: Arc::new(AtomicBool::new(false)),
            expires: Arc::new(AtomicI64::new(0)),
        };
        let result = host.handlers["bad"](context, Value::Null)
            .await
            .expect_err("NaN must not silently become null");
        assert_eq!(result.code, "OUTPUT_INVALID");
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod worker_tests;

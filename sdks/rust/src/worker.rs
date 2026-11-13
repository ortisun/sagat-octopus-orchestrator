use crate::{
    RetryPolicy, SagatError, TlsOptions, TokenSource, Value,
    activity::*,
    protocol as pb,
    worker_rpc::{Rpc, error_of},
};
use futures_util::{FutureExt as _, future::BoxFuture};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    future::Future,
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicU8, Ordering},
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
    wake: Arc<Notify>,
    protocol: Arc<Mutex<String>>,
    failure: tokio::sync::watch::Sender<Option<SagatError>>,
}
pub struct WorkerHost {
    options: WorkerOptions,
    handlers: BTreeMap<String, Arc<Handler>>,
    permits: Arc<Semaphore>,
    tasks: Arc<Mutex<BTreeMap<TaskKey, ActivityContext>>>,
    protocol: Arc<Mutex<String>>,
    failure: tokio::sync::watch::Sender<Option<SagatError>>,
    running: Option<Running>,
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
            protocol: Arc::default(),
            failure: tokio::sync::watch::channel(None).0,
            running: None,
        })
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
            wake: Arc::default(),
            protocol: self.protocol.clone(),
            failure: self.failure.clone(),
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
                    delay = frame.retry_after_ms;
                    for task in frame.tasks.into_iter().take(remaining) {
                        dispatch(&core, &session, task, &life, &tracker);
                        remaining -= 1;
                    }
                    if remaining == 0 {
                        break;
                    }
                }
                Ok((credit - remaining, delay))
            } else {
                let response = client
                    .poll_tasks(request)
                    .await
                    .map_err(|e| error_of("PollTasks", e))?
                    .into_inner();
                let count = response.tasks.len().min(credit);
                for task in response.tasks.into_iter().take(credit) {
                    dispatch(&core, &session, task, &life, &tracker);
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
            Err(error) if !error.may_retry() => {
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
fn dispatch(
    core: &Core,
    session: &Session,
    lease: pb::TaskLease,
    life: &CancellationToken,
    tracker: &TaskTracker,
) {
    let Ok(permit) = core.permits.clone().try_acquire_owned() else {
        return;
    };
    let key = TaskKey {
        id: lease.task_id.clone(),
        generation: lease.generation,
        instance_path: lease
            .instance
            .as_ref()
            .filter(|_| session.minor >= 5)
            .map(|identity| {
                identity
                    .instance_path
                    .iter()
                    .map(|part| (part.scope_node_id.clone(), part.iteration_index))
                    .collect()
            })
            .unwrap_or_default(),
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
    if let Ok(mut tasks) = core.tasks.lock() {
        if tasks.contains_key(&key) {
            return;
        }
        tasks.insert(key.clone(), ctx.clone());
    } else {
        return;
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
            report(&core,&session,&lease,&ctx,result).await;
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
async fn report(
    core: &Core,
    session: &Session,
    lease: &pb::TaskLease,
    ctx: &ActivityContext,
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
        if ctx.lost.load(Ordering::Acquire) {
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
                    None,
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
                ctx.cancel(CancellationReason::LeaseLost);
                return;
            }
            Err(error)
                if error.may_retry() && attempt + 1 < core.options.retry.max_attempts.min(5) =>
            {
                tokio::time::sleep(core.options.retry.after(attempt, error.retry_after)).await;
            }
            Err(_) => return,
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

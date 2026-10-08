//! The worker protocol (`docs/architecture/worker-runtime-protocol.md`): register, claim, heartbeat,
//! complete and fail, all fenced by the lease.
//!
//! NATS (or polling) only *hints* that work exists. The lease is granted here, under the run's row
//! lock, by one canonical operation: the step moves `READY → LEASED`, its generation increases and a
//! secret token is handed to exactly one worker. Anything the worker later reports must present the
//! attempt, the generation and that token; a report from an older holder is refused with
//! [`AppError::StaleAttempt`] and changes nothing, although the external effect it describes may
//! have happened (hence idempotency keys).

use std::future::Future;
use std::time::Duration;

use sagat_domain::execution::{
    self, FailureCode, Fence, Progress, RunEvent, RunId, RunStatus, StepStatus,
};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Slug, TenantId};
use sagat_domain::workflow::{ActivityName, NodeId, NodeKind};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::clock::Clock;
use crate::definitions::{DefinitionStore, Idempotency};
use crate::entropy::{Entropy, Token};
use crate::error::{AppError, StoreError};
use crate::execution::{
    Actor, Effects, Engine, EngineError, ExpressionEvaluator, LeaseGrant, LeaseRenewal,
    MAX_PAYLOAD_BYTES, RunTx, step_id,
};
use crate::runs::{Lock, RunStore, execution_error};

/// The only protocol version this server speaks.
pub const PROTOCOL_VERSION: &str = "1";
/// Most activities one worker may advertise.
pub const MAX_ACTIVITIES: usize = 256;
/// Most tasks one poll may claim.
pub const MAX_POLL: u32 = 64;

/// Tunable timings of the worker protocol.
#[derive(Debug, Clone, Copy)]
pub struct WorkerSettings {
    /// How long a lease lasts without a heartbeat (`lease_ttl`, 30 s by default).
    pub lease_ttl: Duration,
    /// How long a worker session lasts without use.
    pub session_ttl: Duration,
    /// What a worker is told to wait before polling again when nothing was available.
    pub idle_retry: Duration,
}

impl Default for WorkerSettings {
    fn default() -> Self {
        Self {
            lease_ttl: Duration::from_secs(30),
            session_ttl: Duration::from_secs(3600),
            idle_retry: Duration::from_millis(500),
        }
    }
}

/// What a worker says it can do.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub activities: Vec<String>,
    pub languages: Vec<String>,
    pub pools: Vec<String>,
    pub max_inflight: u32,
    pub protocol_version: String,
}

/// A worker session as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub subject: String,
    pub worker_name: String,
    pub activities: Vec<String>,
    pub max_inflight: u32,
    pub expires_at: OffsetDateTime,
}

/// A session about to be created. Only the hash of the token is stored.
#[derive(Debug, Clone)]
pub struct NewSession {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub subject: String,
    pub worker_name: String,
    pub token_hash: Vec<u8>,
    pub activities: Vec<String>,
    pub languages: Vec<String>,
    pub pools: Vec<String>,
    pub max_inflight: u32,
    pub ttl: Duration,
}

/// Storage port for workers. Implemented by `sagat-infra-postgres`.
pub trait WorkerStore: Send + Sync {
    fn create_session(
        &self,
        session: &NewSession,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The live session `id` of `tenant` whose token hashes to `token_hash`, renewing its expiry.
    fn use_session(
        &self,
        tenant: TenantId,
        id: Uuid,
        token_hash: &[u8],
        ttl: Duration,
    ) -> impl Future<Output = Result<Option<SessionRecord>, StoreError>> + Send;

    /// Runs of one namespace that have a `READY` step for one of `activities`, oldest first.
    fn claimable_runs(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        activities: &[String],
        limit: u32,
    ) -> impl Future<Output = Result<Vec<RunId>, StoreError>> + Send;

    /// Steps currently leased to the worker session `id`.
    fn inflight(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> impl Future<Output = Result<u32, StoreError>> + Send;
}

/// The result of registering.
#[derive(Debug, Clone)]
pub struct Registered {
    pub worker_id: Uuid,
    pub session_token: Token,
    pub accepted_protocol_version: &'static str,
    pub expires_at: OffsetDateTime,
}

/// A task handed to a worker.
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub run_id: RunId,
    pub node: NodeId,
    pub step_id: Uuid,
    pub fence: Fence,
    pub lease_token: Token,
    pub expires_at: OffsetDateTime,
    pub activity: ActivityName,
    pub input: Vec<u8>,
    pub idempotency_key: String,
    pub trace_id: Option<String>,
}

impl ClaimedTask {
    /// The identifier a worker echoes back: `run/node/generation`.
    #[must_use]
    pub fn task_id(&self) -> String {
        format!("{}/{}/{}", self.run_id, self.node, self.fence.generation)
    }
}

/// Identifies the attempt a worker is reporting on.
#[derive(Debug, Clone)]
pub struct TaskRef<'a> {
    pub run_id: RunId,
    pub node: &'a NodeId,
    pub fence: Fence,
    pub lease_token: &'a str,
}

/// Outcome of a step command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepAck {
    pub command_id: String,
    pub already_applied: bool,
    pub step_status: StepStatus,
    pub run_status: RunStatus,
}

/// Outcome of a heartbeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatAck {
    pub cancel_requested: bool,
}

/// The worker protocol use cases.
pub struct WorkerService<S, C, E, R> {
    store: S,
    clock: C,
    engine: Engine<E>,
    entropy: R,
    settings: WorkerSettings,
}

impl<S, C, E, R> std::fmt::Debug for WorkerService<S, C, E, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerService").finish_non_exhaustive()
    }
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    pub const fn new(
        store: S,
        clock: C,
        engine: Engine<E>,
        entropy: R,
        settings: WorkerSettings,
    ) -> Self {
        Self {
            store,
            clock,
            engine,
            entropy,
            settings,
        }
    }

    #[must_use]
    pub const fn settings(&self) -> &WorkerSettings {
        &self.settings
    }

    /// Opens a session for a worker identity.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `worker:execute`; [`AppError::NotFound`] for an unknown
    /// namespace; [`AppError::NotAllowed`] for an unsupported protocol version or a bad capability.
    pub async fn register(
        &self,
        principal: &Principal,
        namespace: &str,
        worker_name: &str,
        capabilities: &Capabilities,
    ) -> Result<Registered, AppError> {
        principal.authorize(Permission::WorkerExecute)?;
        if capabilities.protocol_version != PROTOCOL_VERSION {
            return Err(AppError::NotAllowed("unsupported protocol version"));
        }
        if capabilities.activities.is_empty() || capabilities.activities.len() > MAX_ACTIVITIES {
            return Err(AppError::NotAllowed(
                "a worker must advertise between 1 and 256 activities",
            ));
        }
        if capabilities
            .activities
            .iter()
            .any(|a| ActivityName::parse(a).is_err())
        {
            return Err(AppError::NotAllowed(
                "an advertised activity name is invalid",
            ));
        }
        if !(1..=1000).contains(&capabilities.max_inflight) {
            return Err(AppError::NotAllowed(
                "max_inflight must be between 1 and 1000",
            ));
        }
        if worker_name.is_empty()
            || worker_name.len() > 120
            || worker_name.chars().any(char::is_control)
        {
            return Err(AppError::NotAllowed(
                "the worker name must be 1 to 120 printable characters",
            ));
        }
        let slug = Slug::parse(namespace).map_err(|_| AppError::NotFound)?;
        let namespace = self
            .store
            .find_namespace_by_slug(principal.tenant_id(), &slug)
            .await?
            .ok_or(AppError::NotFound)?;
        let token = Token::from_bytes(self.entropy.token()).ok_or(AppError::Internal)?;
        let id = Uuid::now_v7();
        let ttl = self.settings.session_ttl;
        self.store
            .create_session(&NewSession {
                id,
                tenant_id: principal.tenant_id(),
                namespace_id: namespace.id(),
                subject: principal.subject().as_str().to_owned(),
                worker_name: worker_name.to_owned(),
                token_hash: token.hash(),
                activities: dedup(&capabilities.activities),
                languages: dedup(&capabilities.languages),
                pools: dedup(&capabilities.pools),
                max_inflight: capabilities.max_inflight,
                ttl,
            })
            .await?;
        Ok(Registered {
            worker_id: id,
            session_token: token,
            accepted_protocol_version: PROTOCOL_VERSION,
            expires_at: self.clock.now() + ttl,
        })
    }

    async fn session(
        &self,
        principal: &Principal,
        worker_id: Uuid,
        token: &str,
    ) -> Result<SessionRecord, AppError> {
        principal.authorize(Permission::WorkerExecute)?;
        let session = self
            .store
            .use_session(
                principal.tenant_id(),
                worker_id,
                &Token::hash_of(token),
                self.settings.session_ttl,
            )
            .await?
            .ok_or(AppError::Unauthenticated)?;
        // A session belongs to the identity that opened it.
        if session.subject != principal.subject().as_str() {
            return Err(AppError::Unauthenticated);
        }
        Ok(session)
    }

    /// Claims up to `max_tasks` ready steps for the session, granting a lease on each.
    ///
    /// Candidate runs come from an index scan; each is then locked with `SKIP LOCKED`, so two
    /// workers never wait for each other and never receive the same step.
    ///
    /// # Errors
    /// [`AppError::Unauthenticated`] for an unknown, expired or foreign session.
    pub async fn poll(
        &self,
        principal: &Principal,
        worker_id: Uuid,
        session_token: &str,
        max_tasks: u32,
        trace_id: Option<&str>,
    ) -> Result<Vec<ClaimedTask>, AppError> {
        let session = self.session(principal, worker_id, session_token).await?;
        let inflight = self.store.inflight(session.tenant_id, session.id).await?;
        let room = session.max_inflight.saturating_sub(inflight);
        let wanted = max_tasks.clamp(1, MAX_POLL).min(room);
        if wanted == 0 {
            return Ok(Vec::new());
        }
        let candidates = self
            .store
            .claimable_runs(
                session.tenant_id,
                session.namespace_id,
                &session.activities,
                wanted.saturating_mul(4).min(64),
            )
            .await?;
        let mut claimed = Vec::new();
        for run in candidates {
            let remaining = wanted as usize - claimed.len();
            if remaining == 0 {
                break;
            }
            let Some(mut tx) = self
                .store
                .begin(session.tenant_id, run, Lock::SkipLocked)
                .await?
            else {
                continue; // locked by someone else, or gone
            };
            let ready: Vec<(NodeId, ActivityName, Option<Duration>)> = {
                let (def, state) = tx.parts();
                state
                    .steps()
                    .filter(|(_, step)| step.status() == StepStatus::Ready)
                    .filter_map(|(node, _)| {
                        let n = def.node(node)?;
                        match n.kind() {
                            NodeKind::Activity { activity, .. }
                                if session.activities.iter().any(|a| a == activity.as_str()) =>
                            {
                                Some((node.clone(), activity.clone(), n.timeout()))
                            }
                            _ => None,
                        }
                    })
                    .take(remaining)
                    .collect()
            };
            for (node, activity, timeout) in ready {
                let task = self
                    .lease_one(&mut tx, &session, &node, activity, timeout, trace_id)
                    .await;
                match task {
                    Ok(task) => claimed.push(task),
                    // The run stopped, paused or changed under us: skip it, keep the others.
                    Err(AppError::StateConflict(_)) => break,
                    Err(other) => return Err(other),
                }
            }
            tx.commit().await?;
        }
        Ok(claimed)
    }

    async fn lease_one<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        session: &SessionRecord,
        node: &NodeId,
        activity: ActivityName,
        timeout: Option<Duration>,
        trace_id: Option<&str>,
    ) -> Result<ClaimedTask, AppError> {
        let token = Token::from_bytes(self.entropy.token()).ok_or(AppError::Internal)?;
        let mut effects = Effects::by(
            Actor::Worker(session.id.to_string()),
            trace_id.map(str::to_owned),
        );
        effects.lease = Some(LeaseGrant {
            node: node.clone(),
            token_hash: token.hash(),
            worker_id: session.id.to_string(),
            ttl: self.settings.lease_ttl,
            attempt_timeout: timeout,
        });
        let fence = self
            .engine
            .run(tx, effects, |_, state, _| {
                let (fence, event) = execution::lease_step(state, node)?;
                Ok((
                    fence,
                    Progress {
                        events: vec![event],
                        pending: None,
                    },
                ))
            })
            .await
            .map_err(engine_error)?;
        let info = tx.info().clone();
        let expires_at = tx.now() + self.settings.lease_ttl;
        let run_id = info.run_id;
        let version = info.version_id;
        let (data, template) = {
            let template = {
                let (def, _) = tx.parts();
                match def.node(node).map(sagat_domain::workflow::Node::kind) {
                    Some(NodeKind::Activity {
                        idempotency_template,
                        ..
                    }) => idempotency_template.clone(),
                    _ => None,
                }
            };
            (tx.data().await?.clone(), template)
        };
        let idempotency_key = match template {
            Some(template) => self
                .engine
                .evaluator()
                .idempotency_key(&template, &data, node)
                .unwrap_or_else(|failure| {
                    tracing::warn!(error = %failure, node = %node, "idempotency template failed; using the default key");
                    format!("{run_id}:{node}")
                }),
            None => format!("{run_id}:{node}"),
        };
        Ok(ClaimedTask {
            run_id,
            node: node.clone(),
            step_id: step_id(version, run_id, node),
            fence,
            lease_token: token,
            expires_at,
            activity,
            input: self.engine.evaluator().task_input(&data, node),
            idempotency_key,
            trace_id: info.trace_id,
        })
    }

    /// Renews the lease and reports whether the run asked the worker to stop.
    ///
    /// # Errors
    /// [`AppError::StaleAttempt`] when the caller no longer holds the lease.
    pub async fn heartbeat(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
    ) -> Result<HeartbeatAck, AppError> {
        principal.authorize(Permission::WorkerExecute)?;
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        self.verify(&mut tx, task).await?;
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.renewal = Some(LeaseRenewal {
            node: task.node.clone(),
            ttl: self.settings.lease_ttl,
        });
        let node = task.node.clone();
        let fence = task.fence;
        let cancel_requested = self
            .engine
            .run(&mut tx, effects, move |_, state, _| {
                let beat = execution::heartbeat(state, &node, fence)?;
                Ok((
                    beat.cancel_requested,
                    Progress {
                        events: beat.event.into_iter().collect(),
                        pending: None,
                    },
                ))
            })
            .await
            .map_err(engine_error)?;
        tx.commit().await?;
        Ok(HeartbeatAck { cancel_requested })
    }

    /// Records a successful attempt. A repeat with the same `idempotency_key` returns the original
    /// outcome with `already_applied` set, even if the run has moved on.
    ///
    /// # Errors
    /// [`AppError::StaleAttempt`] when the caller no longer holds the lease;
    /// [`AppError::NotAllowed`] for an invalid output;
    /// [`AppError::IdempotencyKeyReused`] when the key was used for a different report.
    pub async fn complete(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        output: &[u8],
        idempotency_key: &str,
    ) -> Result<StepAck, AppError> {
        principal.authorize(Permission::WorkerExecute)?;
        if output.len() > MAX_PAYLOAD_BYTES || !self.engine.evaluator().is_json(output) {
            return Err(AppError::NotAllowed(
                "a step output must be JSON of at most 256 KiB",
            ));
        }
        let idempotency = Idempotency {
            scope: format!("step:{}:{}", task.run_id, task.node),
            key: idempotency_key.to_owned(),
            fingerprint: fingerprint(&[
                b"complete",
                &task.fence.attempt.to_be_bytes(),
                &task.fence.generation.to_be_bytes(),
                output,
            ]),
        };
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if let Some(stored) = tx.replay(&idempotency).await? {
            return Ok(parse_ack(&stored, true, task.run_id));
        }
        self.verify(&mut tx, task).await?;
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.outputs = vec![(task.node.clone(), output.to_vec())];
        let node = task.node.clone();
        let fence = task.fence;
        self.engine
            .run(&mut tx, effects, move |def, state, now| {
                let progress = execution::complete_step(def, state, &node, fence, now)?;
                Ok(((), progress))
            })
            .await
            .map_err(engine_error)?;
        self.finish(tx, task, idempotency).await
    }

    /// Records a failed attempt; the engine retries it, routes it, or fails the run.
    ///
    /// # Errors
    /// As [`Self::complete`].
    pub async fn fail(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        error_code: &str,
        retryable: bool,
        idempotency_key: &str,
    ) -> Result<StepAck, AppError> {
        principal.authorize(Permission::WorkerExecute)?;
        let code = FailureCode::parse(error_code).unwrap_or_else(|_| FailureCode::worker_error());
        let idempotency = Idempotency {
            scope: format!("step:{}:{}", task.run_id, task.node),
            key: idempotency_key.to_owned(),
            fingerprint: fingerprint(&[
                b"fail",
                &task.fence.attempt.to_be_bytes(),
                &task.fence.generation.to_be_bytes(),
                code.as_str().as_bytes(),
                &[u8::from(retryable)],
            ]),
        };
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if let Some(stored) = tx.replay(&idempotency).await? {
            return Ok(parse_ack(&stored, true, task.run_id));
        }
        self.verify(&mut tx, task).await?;
        let effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        let node = task.node.clone();
        let fence = task.fence;
        let jitter = self.entropy.unit();
        self.engine
            .run(&mut tx, effects, move |def, state, now| {
                let progress = execution::fail_step(
                    def,
                    state,
                    &node,
                    execution::Failure {
                        fence,
                        code,
                        retryable,
                        jitter_sample: jitter,
                    },
                    now,
                )?;
                Ok(((), progress))
            })
            .await
            .map_err(engine_error)?;
        self.finish(tx, task, idempotency).await
    }

    async fn open(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        lock: Lock,
    ) -> Result<S::Tx, AppError> {
        // A run of another tenant is indistinguishable from a missing one: a stale attempt.
        self.store
            .begin(principal.tenant_id(), task.run_id, lock)
            .await?
            .ok_or(AppError::StaleAttempt)
    }

    /// The presented token must be the one hashed into the step's current lease.
    async fn verify<Tx: RunTx>(&self, tx: &mut Tx, task: &TaskRef<'_>) -> Result<(), AppError> {
        if tx
            .lease_matches(task.node, &Token::hash_of(task.lease_token))
            .await?
        {
            Ok(())
        } else {
            Err(AppError::StaleAttempt)
        }
    }

    /// Persists the idempotency record next to the outcome and commits.
    async fn finish(
        &self,
        mut tx: S::Tx,
        task: &TaskRef<'_>,
        idempotency: Idempotency,
    ) -> Result<StepAck, AppError> {
        let (step_status, run_status, version) = {
            let (_, state) = tx.parts();
            let step = state
                .step(task.node)
                .map(sagat_domain::execution::StepState::status);
            (
                step.unwrap_or(StepStatus::Cancelled),
                state.status(),
                state.version(),
            )
        };
        // The record is written by a no-event persist so it shares the transaction.
        let mut record = Effects::by(Actor::Worker("ack".into()), None);
        record.idempotency = Some((
            idempotency,
            format!(
                "{}|{}|{}",
                step_status.as_str(),
                run_status.as_str(),
                version
            ),
        ));
        tx.persist(&[], record).await?;
        tx.commit().await?;
        Ok(StepAck {
            command_id: format!("{}:{}", task.run_id, version),
            already_applied: false,
            step_status,
            run_status,
        })
    }
}

fn parse_ack(stored: &str, already_applied: bool, run: RunId) -> StepAck {
    let mut parts = stored.split('|');
    let step_status = parts
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(StepStatus::Cancelled);
    let run_status = parts
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(RunStatus::Running);
    let version = parts.next().unwrap_or("0");
    StepAck {
        command_id: format!("{run}:{version}"),
        already_applied,
        step_status,
        run_status,
    }
}

fn dedup(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = values.to_vec();
    out.sort();
    out.dedup();
    out
}

/// A stale or illegal attempt is [`AppError::StaleAttempt`]; the rest is as for the API.
fn engine_error(error: EngineError) -> AppError {
    match error {
        EngineError::Store(store) => store.into(),
        EngineError::Execution(
            sagat_domain::execution::ExecutionError::StaleAttempt { .. }
            | sagat_domain::execution::ExecutionError::IllegalTransition { .. }
            | sagat_domain::execution::ExecutionError::RunFinished(_),
        ) => AppError::StaleAttempt,
        EngineError::Execution(other) => execution_error(&other),
    }
}

fn fingerprint(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend_from_slice(&(part.len() as u64).to_be_bytes());
        out.extend_from_slice(part);
    }
    out
}

/// Names the events of a progress; handy in diagnostics.
#[must_use]
pub fn describe(events: &[RunEvent]) -> Vec<&'static str> {
    events.iter().map(RunEvent::name).collect()
}

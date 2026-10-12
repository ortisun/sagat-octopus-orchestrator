//! The sandbox runner (`FR-005-002`, `FR-005-004`, `FR-005-005`, `FR-005-007`).
//!
//! It holds the container engine and nothing else: no database, no secrets of its own, no
//! credentials but the token of one tenant's `runner` identity. It takes work from the control
//! plane and gives back what happened:
//!
//! * a **build**: source in, a signed artifact out (the control plane signs; the runner makes);
//! * a **test run**: a draft and an input in, a result, a log and inferred schemas out;
//! * a **script step**: the runner is a worker of the platform's own protocol for the activities
//!   named after artifact digests, so leases, heartbeats, retries and timeouts are the engine's.
//!
//! Before anything runs, the artifact's digest is checked against its bytes and its signature
//! against the keys the runner trusts, so a row planted in the database is not code the runner
//! will execute. What a script may reach is decided here and enforced here: no network, a request
//! through the namespace's egress policy, the secrets its step declared.

pub mod api;
pub mod cli;
pub mod config;
pub mod host;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use sagat_api_grpc::pb;
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_application::sandbox::{
    BuildFailure, BuildRequest, RunOutcome, RunRequest, Sandbox, digest_of,
};
use sagat_application::script_signing::{SigningError, Verifier};
use sagat_domain::identity::TenantId;
use sagat_domain::scripts::{
    Digest, Failure, Language, Lockfile, Policy, Source, digest_of_activity,
};
use sagat_infra_sandbox::{artifact_contract, artifact_policy};
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};
use tonic::metadata::MetadataValue;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};

use crate::api::{
    ControlPlane, ExecutionReport, IncidentTarget, RunnerError, TestJob, TestResult, policy_of,
};
pub use crate::config::{NamespaceTarget, RunnerConfig};
use crate::host::ScriptHost;

/// The longest a lease is assumed to last for the sake of beating it: the engine's default is 30 s.
/// The expiry is read against this machine's clock, and a clock that is behind would otherwise
/// stretch the interval past the lease.
const MAX_LEASE_BEAT: Duration = Duration::from_secs(30);
/// Times a completion or a failure is told to the engine before the runner gives up on it (the
/// lease then lapses and the engine decides what to do with the step).
const ENGINE_TRIES: usize = 3;
/// Steps one poll leases, at most.
const MAX_POLL: usize = 100;
/// A session is renewed after this long, well inside the engine's session lifetime.
const SESSION_AGE: Duration = Duration::from_secs(1800);
/// Artifacts kept verified in memory.
const MAX_CACHED_ARTIFACTS: usize = 128;
/// The languages this runner knows how to build, in the order they are looked at.
const LANGUAGES: [Language; 3] = [Language::TypeScript, Language::Python, Language::CSharp];

/// Why the secrets of a step were not taken.
enum SecretsFailure {
    /// The engine said the step may not have them: a failure of the step, for good.
    Refused,
    /// The engine did not answer, or the runner's own credentials were refused: not the step's fault.
    Unavailable,
}

/// Whether an error talking to the control plane says something about the runner or the network
/// rather than about what was asked for: such a step goes back to the queue instead of failing.
fn is_transient(error: &RunnerError) -> bool {
    match error {
        RunnerError::Transport | RunnerError::Credentials => true,
        // The runner's own token being refused (401/403) is for an operator to fix, and the step
        // should still be there when they have.
        RunnerError::Status { status, .. } => matches!(status, 401 | 403 | 408 | 429 | 500..=599),
        RunnerError::Decode | RunnerError::Sandbox(_) => false,
    }
}

/// An artifact whose digest and signature were checked.
#[derive(Debug)]
struct Loaded {
    language: Language,
    bytes: Vec<u8>,
    policy: Policy,
    input_schema: Option<Vec<u8>>,
    output_schema: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct Session {
    worker_id: String,
    token: String,
    activities: BTreeSet<String>,
    since: Instant,
}

/// A step leased to this runner.
#[derive(Debug, Clone)]
pub struct StepTask {
    pub namespace: NamespaceTarget,
    worker_id: String,
    pub lease: pb::TaskLease,
    /// The room this step takes in the runner, held from the moment it is leased to the moment it
    /// ends: a step that waited for room would hold a lease nobody is beating.
    permit: Option<Arc<OwnedSemaphorePermit>>,
}

/// How a step ended, for the one that ran it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Completed,
    Failed(Failure),
    /// The engine took the lease back or cancelled the step: nothing is reported.
    Abandoned,
}

pub struct Runner<S> {
    config: RunnerConfig,
    api: ControlPlane,
    sandbox: S,
    toolchains: BTreeMap<Language, String>,
    verifier: RwLock<Verifier>,
    grpc: tokio::sync::Mutex<Option<WorkerServiceClient<Channel>>>,
    /// One worker session per namespace: the engine keeps one per (identity, name), so a second
    /// registration under the same name replaces the first, token and advertisement.
    sessions: Mutex<HashMap<String, Session>>,
    artifacts: Mutex<HashMap<String, Arc<Loaded>>>,
    warmed: Mutex<HashSet<Language>>,
    permits: Arc<Semaphore>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<S: Sandbox + 'static> Runner<S> {
    /// Finds out which languages this machine can build and run, and gets ready to serve.
    ///
    /// # Errors
    /// [`RunnerError`] when the control plane client cannot be made or no language can be built.
    pub async fn start(config: RunnerConfig, sandbox: S) -> Result<Arc<Self>, RunnerError> {
        let api = ControlPlane::new(
            &config.api_url,
            config.credentials.clone(),
            config.ca_pem.as_deref(),
        )?;
        let mut toolchains = BTreeMap::new();
        for language in LANGUAGES {
            match sandbox.toolchain(language).await {
                Ok(toolchain) => {
                    toolchains.insert(language, toolchain);
                }
                Err(_) => {
                    tracing::warn!(%language, "no toolchain here: this language is not served")
                }
            }
        }
        if toolchains.is_empty() {
            return Err(RunnerError::Sandbox(
                "no language can be built here: is the container engine reachable?".to_owned(),
            ));
        }
        let verifier = Verifier::new(config.trusted_keys.clone());
        let permits = Arc::new(Semaphore::new(config.max_concurrent));
        Ok(Arc::new(Self {
            config,
            api,
            sandbox,
            toolchains,
            verifier: RwLock::new(verifier),
            grpc: tokio::sync::Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            artifacts: Mutex::new(HashMap::new()),
            warmed: Mutex::new(HashSet::new()),
            permits,
        }))
    }

    #[must_use]
    pub const fn sandbox(&self) -> &S {
        &self.sandbox
    }

    #[must_use]
    pub const fn config(&self) -> &RunnerConfig {
        &self.config
    }

    #[must_use]
    pub fn languages(&self) -> Vec<Language> {
        self.toolchains.keys().copied().collect()
    }

    // ------------------------------------------------------------------------------- leases

    /// Starts beating a lease: the task runs until it is aborted, and the receiver is told if the
    /// holder no longer has the lease (the engine says so, or the step was cancelled).
    fn beating<B, Fut>(
        lease: Duration,
        beat: B,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::sync::oneshot::Receiver<()>,
    )
    where
        B: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = Result<bool, RunnerError>> + Send + 'static,
    {
        let (lost_tx, lost_rx) = tokio::sync::oneshot::channel::<()>();
        let every = (lease / 3).max(Duration::from_secs(1));
        let ticker = tokio::spawn(async move {
            let mut interval = tokio::time::interval(every);
            interval.tick().await;
            loop {
                interval.tick().await;
                match beat().await {
                    Ok(true) => {}
                    Ok(false) => {
                        let _ = lost_tx.send(());
                        return;
                    }
                    // A call that failed may succeed on the next beat; the lease is still ours
                    // until it runs out.
                    Err(error) => tracing::warn!(%error, "a heartbeat failed"),
                }
            }
        });
        (ticker, lost_rx)
    }

    /// Runs `work` while the lease is kept alive; `None` if the lease was lost first (the work is
    /// dropped, which removes its container).
    async fn held<T, B, Fut>(
        &self,
        lease: Duration,
        beat: B,
        work: impl Future<Output = T>,
    ) -> Option<T>
    where
        B: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = Result<bool, RunnerError>> + Send + 'static,
    {
        let (ticker, mut lost) = Self::beating(lease, beat);
        let out = tokio::select! {
            out = work => Some(out),
            _ = &mut lost => None,
        };
        ticker.abort();
        out
    }

    // -------------------------------------------------------------------------------- builds

    /// Takes one build, if there is one, and does it. `true` when there was one.
    ///
    /// # Errors
    /// [`RunnerError`] when the control plane cannot be spoken to.
    pub async fn build_once(self: &Arc<Self>) -> Result<bool, RunnerError> {
        let Some(job) = self.claim_build().await? else {
            return Ok(false);
        };
        self.process_build(job).await?;
        Ok(true)
    }

    /// Asks for the oldest build of a language this runner can build.
    ///
    /// # Errors
    /// [`RunnerError`].
    pub async fn claim_build(&self) -> Result<Option<api::BuildJob>, RunnerError> {
        // Asked again every time: a tag that moved to another image is another toolchain, and a
        // build made on it must not be taken for one made on the old.
        let mut toolchains: BTreeMap<String, String> = BTreeMap::new();
        for language in self.toolchains.keys() {
            match self.sandbox.toolchain(*language).await {
                Ok(toolchain) => {
                    toolchains.insert(language.as_str().to_owned(), toolchain);
                }
                Err(_) => {
                    tracing::warn!(%language, "the toolchain cannot be identified now; its builds are not claimed")
                }
            }
        }
        if toolchains.is_empty() {
            return Ok(None);
        }
        self.api.claim_build(&toolchains).await
    }

    /// Builds what was claimed and reports how it went.
    ///
    /// # Errors
    /// [`RunnerError`] when the report cannot be made.
    pub async fn process_build(self: &Arc<Self>, job: api::BuildJob) -> Result<(), RunnerError> {
        let invalid = |what: &str| RunnerError::Sandbox(format!("a build job with {what}"));
        let language: Language = job
            .language
            .parse()
            .map_err(|_| invalid("an unknown language"))?;
        let request = BuildRequest {
            language,
            source: Source::parse(&job.source).map_err(|_| invalid("an unusable source"))?,
            lockfile: job
                .lockfile
                .as_deref()
                .map(Lockfile::parse)
                .transpose()
                .map_err(|_| invalid("an unusable lockfile"))?,
            policy: policy_of(&job.policy)?,
            input_schema: schema_bytes(job.input_schema.as_ref()),
            output_schema: schema_bytes(job.output_schema.as_ref()),
        };
        let started = Instant::now();
        let beat = {
            let (me, id, token) = (
                Arc::clone(self),
                job.job_id.clone(),
                job.lease_token.clone(),
            );
            move || {
                let (me, id, token) = (Arc::clone(&me), id.clone(), token.clone());
                async move { me.api.heartbeat_build(&id, &token).await }
            }
        };
        let lease = Duration::from_secs(job.lease_seconds);
        let Some(built) = self.held(lease, beat, self.sandbox.build(request)).await else {
            tracing::warn!(job = %job.job_id, "the build lease was lost; the build is abandoned");
            return Ok(());
        };
        metrics::histogram!("script_build_seconds", "language" => language.as_str())
            .record(started.elapsed().as_secs_f64());
        let reported = match built {
            Ok(built) => {
                metrics::counter!("script_builds_total", "outcome" => "built").increment(1);
                self.api
                    .complete_build_built(
                        &job.job_id,
                        &job.lease_token,
                        &built.artifact,
                        &built.sbom,
                        &built.toolchain,
                        &built.log,
                    )
                    .await
            }
            Err(failure) => {
                let (kind, message) = match failure {
                    BuildFailure::Rejected(message) => ("rejected", message),
                    BuildFailure::Lockfile(message) => ("lockfile", message),
                    BuildFailure::Unsupported => {
                        ("unsupported", "no builder for this language".to_owned())
                    }
                    BuildFailure::Limit => {
                        ("limit", "the build used more than it was given".to_owned())
                    }
                    BuildFailure::Unavailable => {
                        ("unavailable", "the sandbox could not be used".to_owned())
                    }
                };
                metrics::counter!("script_builds_total", "outcome" => kind).increment(1);
                self.api
                    .complete_build_failed(&job.job_id, &job.lease_token, kind, &message, "")
                    .await
            }
        };
        match reported {
            Err(error) if error.is_stale() => Ok(()),
            other => other,
        }
    }

    // ------------------------------------------------------------------------- test runs

    /// Takes one test run, if there is one, and does it. `true` when there was one.
    ///
    /// # Errors
    /// [`RunnerError`] when the control plane cannot be spoken to.
    pub async fn test_once(self: &Arc<Self>) -> Result<bool, RunnerError> {
        let Some(job) = self.claim_test().await? else {
            return Ok(false);
        };
        self.process_test(job).await?;
        Ok(true)
    }

    /// # Errors
    /// [`RunnerError`].
    pub async fn claim_test(&self) -> Result<Option<TestJob>, RunnerError> {
        self.api.claim_test(&self.languages()).await
    }

    /// Builds the draft, runs it with the input, and reports what came of it.
    ///
    /// # Errors
    /// [`RunnerError`] when the report cannot be made.
    pub async fn process_test(self: &Arc<Self>, job: TestJob) -> Result<(), RunnerError> {
        let invalid = |what: &str| RunnerError::Sandbox(format!("a test run with {what}"));
        let language: Language = job
            .language
            .parse()
            .map_err(|_| invalid("an unknown language"))?;
        let policy = policy_of(&job.policy)?.within(&self.config.ceiling);
        let build = BuildRequest {
            language,
            source: Source::parse(&job.source).map_err(|_| invalid("an unusable source"))?,
            lockfile: job
                .lockfile
                .as_deref()
                .map(Lockfile::parse)
                .transpose()
                .map_err(|_| invalid("an unusable lockfile"))?,
            policy,
            input_schema: schema_bytes(job.input_schema.as_ref()),
            output_schema: schema_bytes(job.output_schema.as_ref()),
        };
        let lease = Duration::from_secs(job.lease_seconds);
        let beat = {
            let (me, id, token) = (
                Arc::clone(self),
                job.run_id.clone(),
                job.lease_token.clone(),
            );
            move || {
                let (me, id, token) = (Arc::clone(&me), id.clone(), token.clone());
                async move { me.api.heartbeat_test(&id, &token).await }
            }
        };
        let work = self.test_work(&job, build, policy);
        let Some(result) = self.held(lease, beat, work).await else {
            tracing::warn!("the test run lease was lost; the test run is abandoned");
            return Ok(());
        };
        match self
            .api
            .complete_test(&job.run_id, &job.lease_token, &result)
            .await
        {
            Err(error) if error.is_stale() => Ok(()),
            other => other,
        }
    }

    async fn test_work(&self, job: &TestJob, build: BuildRequest, policy: Policy) -> TestResult {
        let language = build.language;
        let built = match self.sandbox.build(build).await {
            Ok(built) => built,
            Err(failure) => {
                let unavailable = matches!(failure, BuildFailure::Unavailable);
                let message = match failure {
                    BuildFailure::Rejected(m) | BuildFailure::Lockfile(m) => m,
                    BuildFailure::Unsupported => "no builder for this language".to_owned(),
                    BuildFailure::Limit => "the build used more than it was given".to_owned(),
                    BuildFailure::Unavailable => "the sandbox could not be used".to_owned(),
                };
                return TestResult {
                    failure: Some(
                        if unavailable {
                            Failure::Unavailable
                        } else {
                            Failure::BuildRejected
                        }
                        .code()
                        .to_owned(),
                    ),
                    log: message,
                    ..TestResult::default()
                };
            }
        };
        // A test run may reach the network only if the script's policy and the namespace's say so;
        // it never has the secrets of the namespace.
        let host = ScriptHost::live(
            self.api.clone(),
            job.namespace_id.clone(),
            IncidentTarget::Test {
                namespace_id: job.namespace_id.clone(),
                test_run_id: job.run_id.clone(),
                lease_token: job.lease_token.clone(),
            },
            matches!(policy.egress, sagat_domain::scripts::Egress::Namespace),
            HashMap::new(),
        );
        let outcome = self
            .sandbox
            .run(
                RunRequest {
                    language,
                    artifact: built.artifact,
                    input: serde_json::to_vec(&job.input).unwrap_or_default(),
                    input_schema: schema_bytes(job.input_schema.as_ref()),
                    output_schema: schema_bytes(job.output_schema.as_ref()),
                    policy,
                    secrets_to_hide: Vec::new(),
                    idempotency_key: None,
                },
                &host,
            )
            .await;
        self.observe(language, &outcome);
        test_result(&outcome)
    }

    // ------------------------------------------------------------------------------- steps

    async fn client(&self) -> Result<WorkerServiceClient<Channel>, RunnerError> {
        let mut slot = self.grpc.lock().await;
        if let Some(client) = slot.as_ref() {
            return Ok(client.clone());
        }
        let mut endpoint = Endpoint::from_shared(self.config.grpc_url.clone())
            .map_err(|_| RunnerError::Transport)?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30));
        if self.config.grpc_url.starts_with("https://") {
            let mut tls = ClientTlsConfig::new();
            if let Some(pem) = &self.config.ca_pem {
                tls = tls.ca_certificate(Certificate::from_pem(pem));
            }
            endpoint = endpoint
                .tls_config(tls)
                .map_err(|_| RunnerError::Transport)?;
        }
        let channel = endpoint
            .connect()
            .await
            .map_err(|_| RunnerError::Transport)?;
        let client = WorkerServiceClient::new(channel);
        *slot = Some(client.clone());
        Ok(client)
    }

    async fn forget_client(&self) {
        *self.grpc.lock().await = None;
    }

    async fn authed<T>(&self, message: T) -> Result<tonic::Request<T>, RunnerError> {
        let token = self.config.credentials.token().await?;
        let mut request = tonic::Request::new(message);
        request.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {token}"))
                .map_err(|_| RunnerError::Credentials)?,
        );
        Ok(request)
    }

    /// Makes sure every activity in `pending` is advertised by the namespace's session.
    ///
    /// The engine keeps one session per (identity, name) and a registration under a name that has
    /// one replaces it, token and advertisement alike. So the runner has one session per namespace
    /// and, when something new is pending, registers it again with the new activities *and* the
    /// ones it already advertised (the pending ones first, if there are more than a session holds).
    async fn cover(
        &self,
        namespace: &NamespaceTarget,
        pending: &[String],
    ) -> Result<(), RunnerError> {
        let advertise: Vec<String> = {
            let mut sessions = lock(&self.sessions);
            if sessions
                .get(&namespace.slug)
                .is_some_and(|s| s.since.elapsed() >= SESSION_AGE)
            {
                sessions.remove(&namespace.slug);
            }
            let current = sessions.get(&namespace.slug);
            if pending
                .iter()
                .all(|a| current.is_some_and(|s| s.activities.contains(a)))
            {
                return Ok(());
            }
            let mut union: BTreeSet<String> = pending
                .iter()
                .take(sagat_application::worker::MAX_ACTIVITIES)
                .cloned()
                .collect();
            for kept in current.iter().flat_map(|s| s.activities.iter()) {
                if union.len() >= sagat_application::worker::MAX_ACTIVITIES {
                    break;
                }
                union.insert(kept.clone());
            }
            union.into_iter().collect()
        };
        let languages: Vec<String> = self
            .languages()
            .iter()
            .map(|l| l.as_str().to_owned())
            .collect();
        let name: String = format!("{}/{}", self.config.name, namespace.slug)
            .chars()
            .take(120)
            .collect();
        let request = self
            .authed(pb::RegisterWorkerRequest {
                worker_name: name,
                namespace: namespace.slug.clone(),
                capabilities: Some(pb::WorkerCapabilities {
                    languages,
                    activities: advertise.clone(),
                    pools: vec![],
                    max_inflight: i32::try_from(self.config.max_concurrent).unwrap_or(i32::MAX),
                    protocol_version: "1".to_owned(),
                }),
            })
            .await?;
        let mut client = self.client().await?;
        let response = client.register_worker(request).await.map_err(|status| {
            tracing::warn!(code = ?status.code(), "the worker session was refused");
            RunnerError::Transport
        })?;
        let response = response.into_inner();
        lock(&self.sessions).insert(
            namespace.slug.clone(),
            Session {
                worker_id: response.worker_id,
                token: response.session_token,
                activities: advertise.into_iter().collect(),
                since: Instant::now(),
            },
        );
        Ok(())
    }

    /// Leases the script steps that are ready in a namespace, at most as many as this runner has
    /// room to run. The room is taken here, before the lease, and goes with each step.
    ///
    /// # Errors
    /// [`RunnerError`] when the control plane or the engine cannot be spoken to.
    pub async fn poll_steps(
        &self,
        namespace: &NamespaceTarget,
    ) -> Result<Vec<StepTask>, RunnerError> {
        let room = self.permits.available_permits().min(MAX_POLL);
        if room == 0 {
            return Ok(Vec::new());
        }
        let pending = self.api.pending(&namespace.id.to_string()).await?;
        if pending.is_empty() {
            return Ok(Vec::new());
        }
        // Taken, not just looked at: another namespace polled a moment later would otherwise lease
        // for the same room.
        let Ok(mut reserved) =
            Arc::clone(&self.permits).try_acquire_many_owned(u32::try_from(room).unwrap_or(1))
        else {
            return Ok(Vec::new());
        };
        self.cover(namespace, &pending).await?;
        let Some(session) = lock(&self.sessions).get(&namespace.slug).cloned() else {
            return Ok(Vec::new());
        };
        let request = self
            .authed(pb::PollTasksRequest {
                worker_id: session.worker_id.clone(),
                session_token: session.token.clone(),
                max_tasks: i32::try_from(room).unwrap_or(1),
            })
            .await?;
        let mut client = self.client().await?;
        let mut tasks = Vec::new();
        match client.poll_tasks(request).await {
            Ok(response) => {
                for lease in response.into_inner().tasks {
                    let Some(permit) = reserved.split(1) else {
                        break;
                    };
                    tasks.push(StepTask {
                        namespace: namespace.clone(),
                        worker_id: session.worker_id.clone(),
                        lease,
                        permit: Some(Arc::new(permit)),
                    });
                }
            }
            Err(status) => {
                use tonic::Code::{DeadlineExceeded, Unavailable};
                if matches!(status.code(), Unavailable | DeadlineExceeded) {
                    self.forget_client().await;
                    return Err(RunnerError::Transport);
                }
                // The session is gone (expired, or the binding changed): a new one is made, with
                // what it advertised, on the next round. Only that one is dropped: a newer
                // registration would have replaced it.
                tracing::warn!(code = ?status.code(), "the worker session was refused; it is dropped");
                let mut sessions = lock(&self.sessions);
                if sessions
                    .get(&namespace.slug)
                    .is_some_and(|s| s.token == session.token)
                {
                    sessions.remove(&namespace.slug);
                }
            }
        }
        Ok(tasks)
    }

    /// Checks and keeps an artifact: its bytes are what its digest says, and the platform signed
    /// it for this tenant.
    async fn load(&self, digest: &Digest) -> Result<Arc<Loaded>, RunnerError> {
        if let Some(found) = lock(&self.artifacts).get(digest.as_str()) {
            return Ok(Arc::clone(found));
        }
        let view = self.api.artifact(digest).await?;
        let untrusted = |why: &str| RunnerError::Sandbox(format!("untrusted artifact: {why}"));
        let bytes = view.bytes()?;
        if digest_of(&bytes) != *digest || view.digest != digest.as_str() {
            return Err(untrusted("the bytes are not what the digest says"));
        }
        let language: Language = view
            .language
            .parse()
            .map_err(|_| untrusted("unknown language"))?;
        let tenant: TenantId = view
            .tenant_id
            .parse()
            .map_err(|_| untrusted("unknown tenant"))?;
        let signature = view.signature_bytes()?;
        self.verify(&view.key_id, tenant, digest, language, &signature)
            .await?;
        let policy = artifact_policy(&bytes).ok_or_else(|| untrusted("no usable policy"))?;
        let (input_schema, output_schema) = artifact_contract(&bytes);
        let loaded = Arc::new(Loaded {
            language,
            bytes,
            policy,
            input_schema,
            output_schema,
        });
        let mut cache = lock(&self.artifacts);
        if cache.len() >= MAX_CACHED_ARTIFACTS {
            cache.clear();
        }
        cache.insert(digest.as_str().to_owned(), Arc::clone(&loaded));
        Ok(loaded)
    }

    async fn verify(
        &self,
        key_id: &str,
        tenant: TenantId,
        digest: &Digest,
        language: Language,
        signature: &[u8],
    ) -> Result<(), RunnerError> {
        let check =
            |verifier: &Verifier| verifier.verify(key_id, tenant, digest, language, signature);
        match check(&*self.verifier.read().await) {
            Ok(()) => return Ok(()),
            // Trust on first use is once: with no keys configured the first key the control plane
            // names is kept, and a key that shows up later is not believed on its word (a rotation
            // is a restart, or a configured key).
            Err(SigningError::UnknownKey)
                if self.config.trusted_keys.is_empty()
                    && self.verifier.read().await.keys().is_empty() => {}
            Err(_) => {
                return Err(RunnerError::Sandbox(
                    "untrusted artifact: the signature does not verify".to_owned(),
                ));
            }
        }
        // No keys were configured: the control plane is asked for its own (trust on first use),
        // once for a key the runner has not seen.
        let key = self.api.signing_key().await?;
        {
            let mut verifier = self.verifier.write().await;
            if verifier.keys().is_empty() {
                *verifier = Verifier::new(vec![key]);
            }
        }
        check(&*self.verifier.read().await).map_err(|_| {
            RunnerError::Sandbox("untrusted artifact: the signature does not verify".to_owned())
        })
    }

    fn observe(&self, language: Language, outcome: &RunOutcome) {
        let name = language.as_str();
        // A run that never reached a container (an input that broke its schema, an artifact that
        // could not be loaded) took no time to start and says nothing about starting.
        if outcome.start > Duration::ZERO {
            let temperature = if lock(&self.warmed).insert(language) {
                "cold"
            } else {
                "warm"
            };
            metrics::histogram!("script_start_seconds", "language" => name, "temperature" => temperature)
                .record(outcome.start.as_secs_f64());
            metrics::histogram!("script_run_seconds", "language" => name)
                .record(outcome.wall.as_secs_f64());
        }
        if let Some(peak) = outcome.memory_peak_bytes {
            #[allow(clippy::cast_precision_loss)] // a gauge of megabytes
            metrics::histogram!("script_memory_peak_bytes", "language" => name).record(peak as f64);
        }
        let result = outcome
            .result
            .as_ref()
            .err()
            .map_or("succeeded", |f| f.code());
        metrics::counter!("script_runs_total", "language" => name, "outcome" => result)
            .increment(1);
        for entry in &outcome.denied {
            let kind = if entry.starts_with("fetch") {
                "fetch"
            } else if entry.starts_with("secret") {
                "secret"
            } else {
                "limit"
            };
            metrics::counter!("script_denied_total", "kind" => kind).increment(1);
        }
    }

    /// Runs one leased step to its end: fetch and check the artifact, take the secrets it declared,
    /// run it, keep the record of what happened, and complete or fail the step.
    ///
    /// What cannot be told apart from a fault on this side (the control plane or the engine did not
    /// answer, the runner's own credentials were refused) fails the step as *unavailable*, which the
    /// engine tries again; only what is wrong with the script, its artifact or its secrets fails it
    /// for good.
    ///
    /// # Errors
    /// [`RunnerError`] when the engine cannot be told how it ended.
    pub async fn run_step(self: &Arc<Self>, task: StepTask) -> Result<StepOutcome, RunnerError> {
        let _room = task.permit.clone();
        let lease = &task.lease;
        let Some(digest) = digest_of_activity(&lease.activity_name) else {
            return self
                .fail_step(&task, Failure::Untrusted, "not a script step")
                .await;
        };
        let loaded = match self.load(&digest).await {
            Ok(loaded) => loaded,
            Err(error) if is_transient(&error) => {
                tracing::warn!(%error, %digest, "an artifact could not be fetched; the step goes back");
                return self
                    .fail_step(
                        &task,
                        Failure::Unavailable,
                        "the artifact could not be fetched",
                    )
                    .await;
            }
            Err(error) => {
                tracing::error!(%error, %digest, "an artifact was refused; the step is failed");
                return self
                    .fail_step(&task, Failure::Untrusted, "the artifact is not trusted")
                    .await;
            }
        };
        let secrets = match self.secrets_of(&task).await {
            Ok(secrets) => secrets,
            Err(SecretsFailure::Refused) => {
                return self
                    .fail_step(&task, Failure::Denied, "a declared secret is not available")
                    .await;
            }
            Err(SecretsFailure::Unavailable) => {
                return self
                    .fail_step(
                        &task,
                        Failure::Unavailable,
                        "the secrets of the step could not be taken",
                    )
                    .await;
            }
        };
        let policy = loaded.policy.within(&self.config.ceiling);
        let namespace_id = task.namespace.id.to_string();
        let host = ScriptHost::live(
            self.api.clone(),
            namespace_id.clone(),
            IncidentTarget::Production {
                namespace_id,
                run_id: lease.run_id.clone(),
                step_id: lease.step_id.clone(),
                attempt: lease.attempt,
                generation: lease.generation,
                lease_token: lease.lease_token.clone(),
            },
            matches!(policy.egress, sagat_domain::scripts::Egress::Namespace),
            secrets,
        );
        let request = RunRequest {
            language: loaded.language,
            artifact: loaded.bytes.clone(),
            input: lease.input_json.clone().into_bytes(),
            input_schema: loaded.input_schema.clone(),
            output_schema: loaded.output_schema.clone(),
            policy,
            secrets_to_hide: host.secret_values(),
            idempotency_key: Some(lease.idempotency_key.clone()).filter(|k| !k.is_empty()),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let beat = {
            let (me, task, cancelled) = (Arc::clone(self), task.clone(), Arc::clone(&cancelled));
            move || {
                let (me, task, cancelled) = (Arc::clone(&me), task.clone(), Arc::clone(&cancelled));
                async move { me.heartbeat_step(&task, &cancelled).await }
            }
        };
        // The lease is beaten while the script runs (and the script is stopped if the lease is
        // lost) and also while the record is kept, which can take a while when the control plane
        // is slow: a lease that lapsed meanwhile would run the script again elsewhere. What ran is
        // recorded even if the lease is lost during the record: it may have had effects.
        let (ticker, mut lost) = Self::beating(lease_ttl(lease), beat);
        let ran = tokio::select! {
            outcome = self.sandbox.run(request, &host) => Some(outcome),
            _ = &mut lost => None,
        };
        let Some(outcome) = ran else {
            ticker.abort();
            if cancelled.load(Ordering::SeqCst) {
                // The run is being cancelled: the script is stopped and the stop is reported, as
                // the worker protocol asks, so the run does not wait for the lease to lapse.
                tracing::info!(task = %lease.task_id, "the run was cancelled; the script is stopped");
                self.report_failure(&task, "STEP_CANCELLED", false, "the run was cancelled")
                    .await?;
                return Ok(StepOutcome::Abandoned);
            }
            tracing::warn!(task = %lease.task_id, "the step was taken back; the script is stopped");
            return Ok(StepOutcome::Abandoned);
        };
        self.observe(loaded.language, &outcome);
        self.keep_record(&task, &digest, &outcome).await;
        ticker.abort();
        match outcome.result {
            Ok(output) => {
                let output_json = String::from_utf8_lossy(&output).into_owned();
                if self.complete_step(&task, output_json).await? {
                    Ok(StepOutcome::Completed)
                } else {
                    Ok(StepOutcome::Abandoned)
                }
            }
            Err(failure) => {
                self.fail_step(&task, failure, failure_message(failure))
                    .await
            }
        }
    }

    async fn secrets_of(&self, task: &StepTask) -> Result<HashMap<String, String>, SecretsFailure> {
        let lease = &task.lease;
        let request = self
            .authed(pb::ResolveSecretsRequest {
                task_id: lease.task_id.clone(),
                run_id: lease.run_id.clone(),
                step_id: lease.step_id.clone(),
                attempt: lease.attempt,
                generation: lease.generation,
                lease_token: lease.lease_token.clone(),
                traceparent: lease.traceparent.clone(),
            })
            .await
            .map_err(|_| SecretsFailure::Unavailable)?;
        let mut client = self
            .client()
            .await
            .map_err(|_| SecretsFailure::Unavailable)?;
        let response = client.resolve_secrets(request).await.map_err(|status| {
            use tonic::Code::{FailedPrecondition, InvalidArgument, NotFound, PermissionDenied};
            if matches!(
                status.code(),
                PermissionDenied | NotFound | InvalidArgument | FailedPrecondition
            ) {
                SecretsFailure::Refused
            } else {
                SecretsFailure::Unavailable
            }
        });
        let response = match response {
            Ok(response) => response,
            Err(failure) => {
                if matches!(failure, SecretsFailure::Unavailable) {
                    self.forget_client().await;
                }
                return Err(failure);
            }
        };
        Ok(response
            .into_inner()
            .secrets
            .into_iter()
            .map(|s| (s.name, String::from_utf8_lossy(&s.value).into_owned()))
            .collect())
    }

    /// Beats the lease of a step. `Ok(false)` when it is no longer ours; `cancelled` is set when the
    /// engine asked for the step to stop (the run is being cancelled), which is told apart from a
    /// lease that was lost because the protocol has the worker report the first.
    async fn heartbeat_step(
        &self,
        task: &StepTask,
        cancelled: &AtomicBool,
    ) -> Result<bool, RunnerError> {
        let lease = &task.lease;
        let request = self
            .authed(pb::HeartbeatRequest {
                task_id: lease.task_id.clone(),
                worker_id: task.worker_id.clone(),
                generation: lease.generation,
                lease_token: lease.lease_token.clone(),
                run_id: lease.run_id.clone(),
                step_id: lease.step_id.clone(),
                attempt: lease.attempt,
            })
            .await?;
        let mut client = self.client().await?;
        match client.heartbeat(request).await {
            Ok(response) => {
                let cancel = response.into_inner().cancel_requested;
                if cancel {
                    cancelled.store(true, Ordering::SeqCst);
                }
                Ok(!cancel)
            }
            // The engine no longer knows the lease: it was reclaimed.
            Err(status)
                if status.code() == tonic::Code::Aborted
                    || status.code() == tonic::Code::FailedPrecondition =>
            {
                Ok(false)
            }
            Err(_) => Err(RunnerError::Transport),
        }
    }

    async fn keep_record(&self, task: &StepTask, digest: &Digest, outcome: &RunOutcome) {
        let lease = &task.lease;
        let report = ExecutionReport {
            namespace_id: task.namespace.id.to_string(),
            digest: digest.to_string(),
            run_id: lease.run_id.clone(),
            node: node_of(lease),
            attempt: u32::try_from(lease.attempt).unwrap_or(0),
            failure: outcome.result.as_ref().err().map(|f| f.code().to_owned()),
            exit_code: outcome.exit_code,
            log: outcome.log.clone(),
            log_truncated: outcome.log_truncated,
            denied: outcome.denied.clone(),
            wall_ms: millis(outcome.wall),
            start_ms: millis(outcome.start),
            memory_peak_bytes: outcome.memory_peak_bytes,
            trace_id: trace_id(&lease.traceparent),
        };
        for attempt in 0..3 {
            match self.api.record_execution(&report).await {
                Ok(()) => return,
                Err(error) => {
                    tracing::warn!(%error, attempt, "the record of an execution could not be kept");
                    tokio::time::sleep(Duration::from_millis(200 * (attempt + 1))).await;
                }
            }
        }
        metrics::counter!("script_records_lost_total").increment(1);
    }

    /// `false` when the engine says the attempt is no longer this runner's (the lease was lost and
    /// the step went to someone else): what the script did is not reported.
    ///
    /// A call the engine could not answer is made again, with the same idempotency key, so a
    /// completion that did arrive is not told twice and one that did not is not lost.
    async fn complete_step(
        &self,
        task: &StepTask,
        output_json: String,
    ) -> Result<bool, RunnerError> {
        let lease = &task.lease;
        for attempt in 0..ENGINE_TRIES {
            let request = self
                .authed(pb::CompleteStepRequest {
                    task_id: lease.task_id.clone(),
                    run_id: lease.run_id.clone(),
                    step_id: lease.step_id.clone(),
                    attempt: lease.attempt,
                    generation: lease.generation,
                    lease_token: lease.lease_token.clone(),
                    idempotency_key: format!("done-{}", lease.task_id),
                    output_json: output_json.clone(),
                    output_blob_ref: String::new(),
                    output_sha256: String::new(),
                    traceparent: lease.traceparent.clone(),
                })
                .await?;
            let mut client = self.client().await?;
            match client.complete_step(request).await {
                Ok(_) => return Ok(true),
                Err(status) if status.code() == tonic::Code::Aborted => {
                    tracing::warn!(
                        "the engine says the attempt is stale; the completion is dropped"
                    );
                    return Ok(false);
                }
                Err(status) => {
                    tracing::warn!(code = ?status.code(), attempt, "the engine refused the completion of a step");
                    if !self.engine_may_answer(&status, attempt).await {
                        return Err(RunnerError::Transport);
                    }
                }
            }
        }
        Err(RunnerError::Transport)
    }

    async fn fail_step(
        &self,
        task: &StepTask,
        failure: Failure,
        message: &str,
    ) -> Result<StepOutcome, RunnerError> {
        if self
            .report_failure(task, failure.code(), failure.retryable(), message)
            .await?
        {
            Ok(StepOutcome::Failed(failure))
        } else {
            Ok(StepOutcome::Abandoned)
        }
    }

    /// Tells the engine a step failed with `code`. `false` when the engine says the attempt is no
    /// longer this runner's: the report is dropped.
    async fn report_failure(
        &self,
        task: &StepTask,
        code: &str,
        retryable: bool,
        message: &str,
    ) -> Result<bool, RunnerError> {
        let lease = &task.lease;
        for attempt in 0..ENGINE_TRIES {
            let request = self
                .authed(pb::FailStepRequest {
                    task_id: lease.task_id.clone(),
                    run_id: lease.run_id.clone(),
                    step_id: lease.step_id.clone(),
                    attempt: lease.attempt,
                    generation: lease.generation,
                    lease_token: lease.lease_token.clone(),
                    idempotency_key: format!("failed-{}", lease.task_id),
                    error_code: code.to_owned(),
                    safe_message: message.to_owned(),
                    retryable,
                    traceparent: lease.traceparent.clone(),
                })
                .await?;
            let mut client = self.client().await?;
            match client.fail_step(request).await {
                Ok(_) => return Ok(true),
                Err(status) if status.code() == tonic::Code::Aborted => {
                    tracing::warn!("the engine says the attempt is stale; the failure is dropped");
                    return Ok(false);
                }
                Err(status) => {
                    tracing::warn!(code = ?status.code(), attempt, "the engine refused the failure of a step");
                    if !self.engine_may_answer(&status, attempt).await {
                        return Err(RunnerError::Transport);
                    }
                }
            }
        }
        Err(RunnerError::Transport)
    }

    /// Whether a call the engine refused is worth making again (it did not answer, or said to try
    /// later); waits a little first. A refusal that is an answer is not.
    async fn engine_may_answer(&self, status: &tonic::Status, attempt: usize) -> bool {
        use tonic::Code::{DeadlineExceeded, Internal, ResourceExhausted, Unavailable};
        let transient = matches!(
            status.code(),
            Unavailable | DeadlineExceeded | ResourceExhausted | Internal
        );
        if !transient || attempt + 1 >= ENGINE_TRIES {
            return false;
        }
        if matches!(status.code(), Unavailable | DeadlineExceeded) {
            self.forget_client().await;
        }
        tokio::time::sleep(Duration::from_millis(200 * (attempt as u64 + 1))).await;
        true
    }

    // ------------------------------------------------------------------------------ the loop

    /// Serves until `stop` completes: takes builds, test runs and steps as there is room, then
    /// waits for what is running to end.
    pub async fn serve(self: Arc<Self>, stop: impl Future<Output = ()> + Send + 'static) {
        let mut stop = Box::pin(stop);
        let mut running = tokio::task::JoinSet::new();
        loop {
            while running.try_join_next().is_some() {}
            let mut busy = false;
            // Builds and test runs.
            while let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() {
                let build = self.claim_build().await;
                let test = if matches!(build, Ok(None)) {
                    Some(self.claim_test().await)
                } else {
                    None
                };
                match (build, test) {
                    (Ok(Some(job)), _) => {
                        busy = true;
                        let me = Arc::clone(&self);
                        running.spawn(async move {
                            let _permit = permit;
                            if let Err(error) = me.process_build(job).await {
                                tracing::warn!(%error, "a build could not be reported");
                            }
                        });
                    }
                    (_, Some(Ok(Some(job)))) => {
                        busy = true;
                        let me = Arc::clone(&self);
                        running.spawn(async move {
                            let _permit = permit;
                            if let Err(error) = me.process_test(job).await {
                                tracing::warn!(%error, "a test run could not be reported");
                            }
                        });
                    }
                    (Err(error), _) | (_, Some(Err(error))) => {
                        tracing::warn!(%error, "the control plane could not be asked for work");
                        break;
                    }
                    _ => break,
                }
            }
            // Script steps.
            for namespace in &self.config.namespaces {
                match self.poll_steps(namespace).await {
                    Ok(steps) => {
                        for step in steps {
                            busy = true;
                            let me = Arc::clone(&self);
                            running.spawn(async move {
                                if let Err(error) = me.run_step(step).await {
                                    tracing::warn!(%error, "a step could not be reported");
                                }
                            });
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, ns = %namespace.slug, "steps could not be polled")
                    }
                }
            }
            let pause = if busy {
                Duration::from_millis(20)
            } else {
                self.config.poll_interval
            };
            tokio::select! {
                () = tokio::time::sleep(pause) => {}
                () = &mut stop => break,
            }
        }
        // Wind down: what is running is allowed to finish.
        while running.join_next().await.is_some() {}
    }
}

fn schema_bytes(value: Option<&serde_json::Value>) -> Option<Vec<u8>> {
    value.and_then(|v| serde_json::to_vec(v).ok())
}

/// How long the engine will hold the lease without a heartbeat, read from the lease itself.
fn lease_ttl(lease: &pb::TaskLease) -> Duration {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let left = lease.expires_at_epoch_ms.saturating_sub(now);
    Duration::from_millis(u64::try_from(left).unwrap_or(0))
        .clamp(Duration::from_secs(3), MAX_LEASE_BEAT)
}

/// The node a step is of: the step document says so; the id of the step stands in if it does not.
fn node_of(lease: &pb::TaskLease) -> String {
    serde_json::from_str::<serde_json::Value>(&lease.input_json)
        .ok()
        .and_then(|document| document["node"].as_str().map(str::to_owned))
        .filter(|node| !node.is_empty() && node.len() <= 64)
        .unwrap_or_else(|| lease.step_id.clone())
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// The trace id of a W3C `traceparent`, if it is one.
fn trace_id(traceparent: &str) -> Option<String> {
    let mut parts = traceparent.split('-');
    let (_version, id) = (parts.next()?, parts.next()?);
    (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| id.to_ascii_lowercase())
}

/// What a step failure says to the engine: the kind, never the script's own words, which may hold
/// the data it was handed. The detail is in the execution record, which has a retention.
fn failure_message(failure: Failure) -> &'static str {
    match failure {
        Failure::Error => "the script raised an error; see its execution record",
        Failure::Timeout => "the script ran out of time",
        Failure::OutOfMemory => "the script used more memory than it was given",
        Failure::OutputLimit => "the script wrote more than it may",
        Failure::Denied => "the script asked for something it may not have",
        Failure::SchemaViolation => "the input or the result does not match the declared schema",
        Failure::NoResult => "the script ended without a result",
        Failure::Unavailable => "the sandbox could not run the script",
        Failure::BuildRejected => "the script does not build",
        Failure::Untrusted => "the artifact is not one the platform signed",
    }
}

fn test_result(outcome: &RunOutcome) -> TestResult {
    let (result, failure) = match &outcome.result {
        Ok(bytes) => match serde_json::from_slice(bytes) {
            Ok(value) => (Some(value), None),
            Err(_) => (None, Some(Failure::NoResult.code().to_owned())),
        },
        Err(failure) => (None, Some(failure.code().to_owned())),
    };
    let schema = |bytes: &Option<Vec<u8>>| {
        bytes
            .as_deref()
            .and_then(|b| serde_json::from_slice(b).ok())
    };
    TestResult {
        result,
        failure,
        log: outcome.log.clone(),
        log_truncated: outcome.log_truncated,
        denied: outcome.denied.clone(),
        exit_code: outcome.exit_code,
        wall_ms: millis(outcome.wall),
        start_ms: millis(outcome.start),
        memory_peak_bytes: outcome.memory_peak_bytes,
        inferred_input_schema: schema(&outcome.inferred_input_schema),
        inferred_output_schema: schema(&outcome.inferred_output_schema),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_runner_or_the_network_did_is_told_apart_from_what_the_step_did() {
        let status = |status| RunnerError::Status {
            status,
            code: String::new(),
            reason: String::new(),
        };
        for transient in [
            RunnerError::Transport,
            RunnerError::Credentials,
            status(401),
            status(403),
            status(408),
            status(429),
            status(500),
            status(503),
        ] {
            assert!(is_transient(&transient), "{transient}");
        }
        for permanent in [
            status(404),
            status(422),
            RunnerError::Decode,
            RunnerError::Sandbox("untrusted artifact: bad".to_owned()),
        ] {
            assert!(!is_transient(&permanent), "{permanent}");
        }
    }

    #[test]
    fn a_clock_that_is_behind_cannot_stretch_the_beat_past_the_lease() {
        let lease = |ahead_ms: i64| pb::TaskLease {
            expires_at_epoch_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
                + ahead_ms,
            ..pb::TaskLease::default()
        };
        assert!(lease_ttl(&lease(60 * 60 * 1000)) <= Duration::from_secs(30));
        assert_eq!(lease_ttl(&lease(-5_000)), Duration::from_secs(3));
        assert!((19..=20).contains(&lease_ttl(&lease(20_000)).as_secs()));
    }

    #[test]
    fn a_trace_id_is_read_from_a_traceparent_and_nothing_else_is() {
        assert_eq!(
            trace_id("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01").as_deref(),
            Some("0af7651916cd43dd8448eb211c80319c")
        );
        for bad in [
            "",
            "garbage",
            "00-short-b7ad6b7169203331-01",
            "00-zzf7651916cd43dd8448eb211c80319c-b7-01",
        ] {
            assert_eq!(trace_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_failure_never_repeats_what_the_script_said() {
        for failure in [
            Failure::Error,
            Failure::Timeout,
            Failure::OutOfMemory,
            Failure::OutputLimit,
            Failure::Denied,
            Failure::SchemaViolation,
            Failure::NoResult,
            Failure::Unavailable,
            Failure::BuildRejected,
            Failure::Untrusted,
        ] {
            let message = failure_message(failure);
            assert!(!message.is_empty() && message.len() < 100, "{message}");
        }
    }
}

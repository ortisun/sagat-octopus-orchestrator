//! Scripts: the drafts authors edit, the immutable versions they publish, the builds that turn a
//! version into a signed artifact, the test runs that try a draft without publishing it, and the
//! records of what ran (`FR-005-001..008`).
//!
//! Two services share this module because they share its records. [`ScriptService`] is what a
//! person uses: write a draft, try it, publish it. [`RunnerService`] is what the sandbox's runner
//! uses: take a build or a test run, fetch an artifact, report an execution. The control plane
//! never runs a script and never touches the container engine; the runner never touches the
//! database. They meet in the records and in the leases that fence a job to one holder at a time.

use std::future::Future;
use std::time::Duration;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Slug, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::scripts::{
    BuildJobId, Digest, Egress, Failure, JobStatus, Language, Limits, Lockfile, MAX_SCHEMA_BYTES,
    Policy, Retention, ScriptError, ScriptId, ScriptVersionId, Source, TestRunId, VersionNumber,
    VersionStatus,
};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::entropy::{Entropy, Token};
use crate::error::{AppError, ConflictKind, StoreError};
use crate::identity::RequestContext;
use crate::quotas::QuotaLookup;
use crate::sandbox::{MAX_RESULT_BYTES, build_key, digest_of};
use crate::script_signing::Signer;

/// Longest input a test run takes, in bytes: what a result may be, so a script can return its input.
pub const MAX_INPUT_BYTES: usize = MAX_RESULT_BYTES;
/// Largest artifact accepted from a build, in bytes.
pub const MAX_ARTIFACT_BYTES: usize = 2 * 1024 * 1024;
/// Largest software bill of materials accepted from a build, in bytes.
pub const MAX_SBOM_BYTES: usize = 256 * 1024;
/// Longest log kept of a build, in bytes.
pub const MAX_BUILD_LOG_BYTES: usize = 64 * 1024;
/// How long a build or a test run may be held without a sign of life.
pub const LEASE: Duration = Duration::from_secs(60);
/// How many times a build is tried when the sandbox could not be used before it is given up on.
pub const MAX_BUILD_ATTEMPTS: u32 = 3;
/// How long a test run may wait for a runner before it is failed as unavailable.
pub const TEST_QUEUE_WAIT: Duration = Duration::from_secs(120);
/// Most scripts, versions or runs one listing returns.
pub const MAX_LIST: u32 = 200;
/// Most builds reused from the cache in one claim before the runner is told there is nothing.
const MAX_CACHE_HITS_PER_CLAIM: usize = 16;
/// Most refusals one execution record turns into audit entries.
const MAX_VIOLATIONS_AUDITED: usize = 10;

// ---------------------------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------------------------

/// What an author edits, and what a version freezes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub source: Source,
    pub lockfile: Option<Lockfile>,
    /// JSON Schema the input must satisfy, if the author declared one.
    pub input_schema: Option<Vec<u8>>,
    /// JSON Schema the result must satisfy, if the author declared one.
    pub output_schema: Option<Vec<u8>>,
    pub policy: Policy,
    /// How long the record of an execution is kept.
    pub retention: Retention,
}

impl Draft {
    /// What makes two drafts the same version: every field, each length-prefixed so none can be
    /// shifted into another.
    #[must_use]
    pub fn content_digest(&self, language: Language) -> Digest {
        let mut hash = Sha256::new();
        for part in [
            b"sagat/script-version/v1".as_slice(),
            language.as_str().as_bytes(),
            self.source.as_str().as_bytes(),
            self.lockfile
                .as_ref()
                .map_or("", Lockfile::as_str)
                .as_bytes(),
            self.input_schema.as_deref().unwrap_or_default(),
            self.output_schema.as_deref().unwrap_or_default(),
            self.policy.canonical().as_bytes(),
            self.retention.get().to_string().as_bytes(),
        ] {
            hash.update((part.len() as u64).to_be_bytes());
            hash.update(part);
        }
        Digest::from_bytes(hash.finalize().into())
    }
}

/// A script and its draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRecord {
    pub id: ScriptId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub name: Slug,
    pub language: Language,
    pub draft: Draft,
    /// Counts the changes of the draft: a change is refused if it was made against an older one.
    pub revision: u64,
    pub latest_version: Option<VersionNumber>,
    pub created_by: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A published version: immutable, whatever happens to the draft after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRecord {
    pub id: ScriptVersionId,
    pub script_id: ScriptId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub number: VersionNumber,
    pub language: Language,
    pub status: VersionStatus,
    pub draft: Draft,
    /// Of everything that defines the version; two publications with the same one are one version.
    pub content_digest: Digest,
    /// Of what the build was made from (source, lockfile, toolchain, policy). Set by the build.
    pub build_key: Option<Digest>,
    /// Of the artifact, once built. What a workflow pins.
    pub artifact: Option<Digest>,
    pub key_id: Option<String>,
    pub toolchain: Option<String>,
    pub build_log: Option<String>,
    /// Why the build produced nothing, when it did not.
    pub failure: Option<String>,
    pub created_by: String,
    pub created_at: OffsetDateTime,
    pub built_at: Option<OffsetDateTime>,
}

/// A version to be created. The store gives it its number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVersion {
    /// The revision of the draft this version is a copy of: the publication is refused if the
    /// draft has moved on, so a version is always exactly what the author saw.
    pub draft_revision: u64,
    pub id: ScriptVersionId,
    pub job_id: BuildJobId,
    pub script_id: ScriptId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub language: Language,
    pub draft: Draft,
    pub content_digest: Digest,
    pub created_by: String,
    pub created_at: OffsetDateTime,
}

/// A built, signed artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRecord {
    pub tenant_id: TenantId,
    pub digest: Digest,
    pub language: Language,
    pub content: Vec<u8>,
    pub signature: Vec<u8>,
    pub key_id: String,
    pub sbom: Vec<u8>,
    pub toolchain: String,
    pub created_at: OffsetDateTime,
}

/// A trial of a draft in the sandbox, without publishing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRunRecord {
    pub id: TestRunId,
    pub script_id: ScriptId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub language: Language,
    pub status: JobStatus,
    /// The draft as it was when the run was asked for.
    pub draft: Draft,
    pub input: Vec<u8>,
    pub result: Option<Vec<u8>>,
    pub failure: Option<Failure>,
    pub log: String,
    pub log_truncated: bool,
    pub denied: Vec<String>,
    pub exit_code: Option<i64>,
    pub wall_ms: Option<u64>,
    pub start_ms: Option<u64>,
    pub memory_peak_bytes: Option<u64>,
    pub inferred_input_schema: Option<Vec<u8>>,
    pub inferred_output_schema: Option<Vec<u8>>,
    pub attempts: u32,
    pub created_by: String,
    pub created_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
}

/// How an execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Succeeded,
    Failed,
}

impl ExecutionStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
        }
    }
}

/// What a run of a published script did, as the runner reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewExecution {
    pub namespace_id: NamespaceId,
    pub digest: Digest,
    /// The step that ran it.
    pub run_id: Option<Uuid>,
    pub node: Option<String>,
    pub attempt: Option<u32>,
    pub status: ExecutionStatus,
    pub failure: Option<Failure>,
    pub exit_code: Option<i64>,
    /// Already redacted by the runner; redacted again before it is kept.
    pub log: String,
    pub log_truncated: bool,
    pub denied: Vec<String>,
    pub wall_ms: u64,
    pub start_ms: u64,
    pub memory_peak_bytes: Option<u64>,
    pub trace_id: Option<String>,
}

/// An execution as kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRecord {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub execution: NewExecution,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

/// The job that builds a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildJobRecord {
    pub id: BuildJobId,
    pub version_id: ScriptVersionId,
    pub attempts: u32,
    pub lease_expires_at: OffsetDateTime,
}

/// A build taken by a runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedBuild {
    pub job: BuildJobRecord,
    pub version: VersionRecord,
}

/// A test run taken by a runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedTest {
    pub run: TestRunRecord,
    pub lease_expires_at: OffsetDateTime,
}

/// What a build reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildReport {
    Built {
        artifact: Vec<u8>,
        sbom: Vec<u8>,
        toolchain: String,
        log: String,
    },
    Failed {
        kind: BuildFailureKind,
        message: String,
        log: String,
    },
}

/// Why a build produced nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildFailureKind {
    /// The source does not compile or parse.
    Rejected,
    /// The lockfile is not one the platform accepts.
    Lockfile,
    /// The build used more than it was given.
    Limit,
    /// The language has no builder in this runner.
    Unsupported,
    /// The sandbox could not be used. Tried again, up to [`MAX_BUILD_ATTEMPTS`] times.
    Unavailable,
}

impl BuildFailureKind {
    /// Whether the build may be tried again by another claim.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Unavailable)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rejected => "rejected",
            Self::Lockfile => "lockfile",
            Self::Limit => "limit",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
        }
    }
}

/// What a test run reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestReport {
    pub result: Result<Vec<u8>, Failure>,
    pub log: String,
    pub log_truncated: bool,
    pub denied: Vec<String>,
    pub exit_code: Option<i64>,
    pub wall_ms: u64,
    pub start_ms: u64,
    pub memory_peak_bytes: Option<u64>,
    pub inferred_input_schema: Option<Vec<u8>>,
    pub inferred_output_schema: Option<Vec<u8>>,
}

/// How completing a job went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finished {
    /// This call finished it.
    Done,
    /// It was finished already by the same holder (a retried completion).
    AlreadyDone,
    /// Another runner holds it now, or it is gone: the report is not accepted.
    Stale,
}

/// A change of a draft: where, to what, and against which revision.
#[derive(Debug, Clone, Copy)]
pub struct DraftUpdate<'a> {
    pub tenant: TenantId,
    pub namespace: NamespaceId,
    pub id: ScriptId,
    pub draft: &'a Draft,
    pub expected_revision: u64,
    pub now: OffsetDateTime,
}

/// A build held by a runner: the proof it presents (the hash of its token) and the moment it acts.
#[derive(Debug, Clone, Copy)]
pub struct Hold<'a> {
    pub tenant: TenantId,
    pub job: BuildJobId,
    pub token_hash: &'a [u8],
    pub now: OffsetDateTime,
}

/// What a delete did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deleted {
    Done,
    NotFound,
    /// A script with a version is kept: a workflow may pin its artifact.
    HasVersions,
}

// ---------------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------------

/// Why a schema was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaProblem {
    TooLarge,
    NotJson,
    NotAnObject,
    Invalid,
}

/// Checks a JSON Schema when an author writes it. Implemented by `sagat-infra-schema`.
pub trait SchemaChecker: Send + Sync {
    /// # Errors
    /// Why the bytes are not a schema the platform can use.
    fn check(&self, schema: &[u8]) -> Result<(), SchemaProblem>;
}

/// Storage port of the scripts, as the people who use them see them. Implemented by
/// `sagat-infra-postgres`.
pub trait ScriptStore: NamespaceDirectory + QuotaLookup {
    /// Creates a script. The name is unique in the namespace, and the namespace holds at most
    /// `limit` scripts: both are decided in the same transaction as the insert.
    fn create_script(
        &self,
        script: &ScriptRecord,
        limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Scripts of a namespace by name, after `after` (a name), at most `limit`.
    fn scripts(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<&str>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ScriptRecord>, StoreError>> + Send;

    fn script(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        id: ScriptId,
    ) -> impl Future<Output = Result<Option<ScriptRecord>, StoreError>> + Send;

    /// Replaces the draft if it is still at `expected_revision`.
    ///
    /// # Errors
    /// [`StoreError::RevisionConflict`] when it is not; `Ok(None)` when there is no such script.
    fn update_draft(
        &self,
        update: DraftUpdate<'_>,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Option<ScriptRecord>, StoreError>> + Send;

    fn delete_script(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        id: ScriptId,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Deleted, StoreError>> + Send;

    /// Makes a version of the script and queues its build, or returns the latest version when it
    /// is the same one (`created` is then `false`). At most `jobs_limit` builds and test runs of
    /// the namespace may be unfinished. `Ok(None)` when there is no such script.
    fn publish_version(
        &self,
        version: &NewVersion,
        jobs_limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Option<(VersionRecord, bool)>, StoreError>> + Send;

    fn versions(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        script: ScriptId,
        before: Option<VersionNumber>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<VersionRecord>, StoreError>> + Send;

    fn version(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        script: ScriptId,
        number: VersionNumber,
    ) -> impl Future<Output = Result<Option<VersionRecord>, StoreError>> + Send;

    /// Queues a test run. At most `jobs_limit` builds and test runs of the namespace may be
    /// unfinished. A test run that has waited longer than [`TEST_QUEUE_WAIT`] is failed here first.
    fn create_test_run(
        &self,
        run: &TestRunRecord,
        jobs_limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    fn test_run(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        script: ScriptId,
        id: TestRunId,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<TestRunRecord>, StoreError>> + Send;

    fn test_runs(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        script: ScriptId,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<TestRunRecord>, StoreError>> + Send;

    /// What ran, newest first, optionally of one artifact.
    fn executions(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        digest: Option<&Digest>,
        before: Option<Uuid>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ExecutionRecord>, StoreError>> + Send;

    /// The digests among `digests` that no ready version of the tenant has built (for the check a
    /// workflow makes when it is published).
    fn missing_artifacts(
        &self,
        tenant: TenantId,
        digests: &[Digest],
    ) -> impl Future<Output = Result<Vec<Digest>, StoreError>> + Send;
}

/// Storage port of the runner's side. Implemented by `sagat-infra-postgres`. Every method is
/// scoped to the tenant of the runner's identity.
pub trait RunnerStore: NamespaceDirectory {
    /// Takes the oldest build of one of `languages` that nobody holds (never taken, or whose lease
    /// ran out), holds it for `lease` under `token_hash`, and counts the attempt. A build whose
    /// attempts are used up is failed instead of being handed out again.
    fn claim_build(
        &self,
        tenant: TenantId,
        languages: &[Language],
        token_hash: &[u8],
        lease: Duration,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<ClaimedBuild>, StoreError>> + Send;

    /// The build held under `token_hash`, if it still is.
    fn held_build(
        &self,
        tenant: TenantId,
        job: BuildJobId,
        token_hash: &[u8],
    ) -> impl Future<Output = Result<Option<ClaimedBuild>, StoreError>> + Send;

    fn heartbeat_build(
        &self,
        tenant: TenantId,
        job: BuildJobId,
        token_hash: &[u8],
        lease: Duration,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<OffsetDateTime>, StoreError>> + Send;

    /// A ready version of the tenant built with `key`: the artifact it made.
    fn built_with(
        &self,
        tenant: TenantId,
        key: &Digest,
    ) -> impl Future<Output = Result<Option<Digest>, StoreError>> + Send;

    /// Finishes a held build by reusing an artifact that already exists.
    fn finish_build_cached(
        &self,
        hold: Hold<'_>,
        key: &Digest,
        artifact: &Digest,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Finished, StoreError>> + Send;

    /// Finishes a held build with a new artifact: stores it (once per digest), marks the version
    /// ready with its signature, and the job succeeded, in one transaction.
    fn finish_build(
        &self,
        hold: Hold<'_>,
        built: &BuiltArtifact,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Finished, StoreError>> + Send;

    /// Finishes a held build that produced nothing: the version fails with the reason, or, when
    /// `retry` and there are attempts left, the job goes back to the queue.
    fn fail_build(
        &self,
        hold: Hold<'_>,
        reason: &str,
        log: &str,
        retry: bool,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Finished, StoreError>> + Send;

    fn claim_test(
        &self,
        tenant: TenantId,
        languages: &[Language],
        token_hash: &[u8],
        lease: Duration,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<ClaimedTest>, StoreError>> + Send;

    fn heartbeat_test(
        &self,
        tenant: TenantId,
        run: TestRunId,
        token_hash: &[u8],
        lease: Duration,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<Option<OffsetDateTime>, StoreError>> + Send;

    fn finish_test(
        &self,
        tenant: TenantId,
        run: TestRunId,
        token_hash: &[u8],
        report: &TestReport,
        now: OffsetDateTime,
        audits: &[AuditRecord],
    ) -> impl Future<Output = Result<Finished, StoreError>> + Send;

    fn artifact(
        &self,
        tenant: TenantId,
        digest: &Digest,
    ) -> impl Future<Output = Result<Option<ArtifactRecord>, StoreError>> + Send;

    /// How long the executions of the artifact are kept, by the newest version that built it.
    fn retention_of(
        &self,
        tenant: TenantId,
        digest: &Digest,
    ) -> impl Future<Output = Result<Option<Retention>, StoreError>> + Send;

    /// Keeps the record of an execution (once per run, node and attempt) and the audit entries of
    /// what the script was refused. `false` when it was kept already.
    fn record_execution(
        &self,
        record: &ExecutionRecord,
        audits: &[AuditRecord],
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

/// An artifact with the signature the control plane made of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltArtifact {
    pub artifact: ArtifactRecord,
    pub build_key: Digest,
    pub log: String,
}

// ---------------------------------------------------------------------------------------------
// The author's service
// ---------------------------------------------------------------------------------------------

/// What an author asks of the limits of a script. Anything left out is the platform's default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PolicyRequest {
    pub memory_bytes: Option<u64>,
    pub cpu_millis: Option<u32>,
    pub wall: Option<Duration>,
    pub processes: Option<u32>,
    pub output_bytes: Option<usize>,
    pub scratch_bytes: Option<u64>,
    pub egress: Option<Egress>,
}

impl PolicyRequest {
    fn resolve(&self, base: &Policy, ceiling: &Policy) -> Result<Policy, ScriptError> {
        let l = &base.limits;
        let limits = Limits::new(
            self.memory_bytes.unwrap_or(l.memory_bytes()),
            self.cpu_millis.unwrap_or(l.cpu_millis()),
            self.wall.unwrap_or(l.wall()),
            self.processes.unwrap_or(l.processes()),
            self.output_bytes.unwrap_or(l.output_bytes()),
            self.scratch_bytes.unwrap_or(l.scratch_bytes()),
        )?;
        let policy = Policy {
            limits,
            egress: self.egress.unwrap_or(base.egress),
        };
        // Asked for no more than the platform allows, or refused: silently giving less would make
        // the author believe in a limit they do not have.
        if policy.within(ceiling) == policy {
            Ok(policy)
        } else {
            Err(ScriptError::AboveCeiling)
        }
    }
}

/// What an author gives to create a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSpec {
    pub name: Slug,
    pub language: Language,
    pub source: Source,
    pub lockfile: Option<Lockfile>,
    pub input_schema: Option<Vec<u8>>,
    pub output_schema: Option<Vec<u8>>,
    pub policy: PolicyRequest,
    pub retention: Option<Retention>,
}

/// A change to a draft: what is `Some` replaces what was there; a schema or the lockfile that is
/// `Some(None)` is removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftChange {
    pub source: Option<Source>,
    pub lockfile: Option<Option<Lockfile>>,
    pub input_schema: Option<Option<Vec<u8>>>,
    pub output_schema: Option<Option<Vec<u8>>>,
    pub policy: Option<PolicyRequest>,
    pub retention: Option<Retention>,
}

/// A version published, or found to be the latest already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub version: VersionRecord,
    pub created: bool,
}

/// The use cases of the people who write scripts.
pub struct ScriptService<S, C, K> {
    store: S,
    clock: C,
    schemas: K,
    ceiling: Policy,
}

impl<S, C, K> ScriptService<S, C, K>
where
    S: ScriptStore,
    C: Clock,
    K: SchemaChecker,
{
    /// `ceiling` is the most any script may be given: what the platform's sandbox can afford.
    #[must_use]
    pub const fn new(store: S, clock: C, schemas: K, ceiling: Policy) -> Self {
        Self {
            store,
            clock,
            schemas,
            ceiling,
        }
    }

    /// The most any script may be given.
    #[must_use]
    pub const fn ceiling(&self) -> &Policy {
        &self.ceiling
    }

    fn check_schema(&self, schema: Option<&[u8]>) -> Result<(), AppError> {
        match schema {
            Some(bytes) if bytes.len() > MAX_SCHEMA_BYTES => {
                Err(AppError::InvalidScript(ScriptError::InvalidSchema))
            }
            Some(bytes) => self
                .schemas
                .check(bytes)
                .map_err(|_| AppError::InvalidScript(ScriptError::InvalidSchema)),
            None => Ok(()),
        }
    }

    /// Creates a script with its first draft.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:write`; [`AppError::NotFound`];
    /// [`AppError::InvalidScript`] for a language not built, a schema that is not one, or a limit
    /// out of bounds or above the ceiling; [`AppError::Conflict`] for a name taken;
    /// [`AppError::QuotaExceeded`].
    pub async fn create(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        spec: ScriptSpec,
        ctx: &RequestContext,
    ) -> Result<ScriptRecord, AppError> {
        principal.authorize(Permission::ScriptWrite)?;
        if !spec.language.is_built() {
            return Err(AppError::InvalidScript(ScriptError::LanguageNotBuilt));
        }
        self.check_schema(spec.input_schema.as_deref())?;
        self.check_schema(spec.output_schema.as_deref())?;
        let policy = spec
            .policy
            .resolve(&Policy::default(), &self.ceiling)
            .map_err(AppError::InvalidScript)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let now = self.clock.now();
        let script = ScriptRecord {
            id: ScriptId::generate(),
            tenant_id: principal.tenant_id(),
            namespace_id: namespace,
            name: spec.name,
            language: spec.language,
            draft: Draft {
                source: spec.source,
                lockfile: spec.lockfile,
                input_schema: spec.input_schema,
                output_schema: spec.output_schema,
                policy,
                retention: spec.retention.unwrap_or(Retention::DEFAULT),
            },
            revision: 1,
            latest_version: None,
            created_by: principal.subject().as_str().to_owned(),
            created_at: now,
            updated_at: now,
        };
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::Scripts,
            )
            .await?;
        let audit = self.audit(
            principal,
            AuditAction::ScriptCreated,
            ResourceType::Script,
            script.id.to_string(),
            format!("{} script '{}'", script.language, script.name),
            ctx,
        );
        self.store.create_script(&script, limit, &audit).await?;
        Ok(script)
    }

    /// The scripts of a namespace, by name.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:read`; [`AppError::NotFound`].
    pub async fn list(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        after: Option<&str>,
        limit: u32,
        ctx: &RequestContext,
    ) -> Result<Vec<ScriptRecord>, AppError> {
        principal.authorize(Permission::ScriptRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .scripts(
                principal.tenant_id(),
                namespace,
                after,
                limit.clamp(1, MAX_LIST),
            )
            .await?)
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `script:read`; [`AppError::NotFound`].
    pub async fn get(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        ctx: &RequestContext,
    ) -> Result<ScriptRecord, AppError> {
        principal.authorize(Permission::ScriptRead)?;
        self.load(principal, namespace, id, ctx).await
    }

    async fn load(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        ctx: &RequestContext,
    ) -> Result<ScriptRecord, AppError> {
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        self.store
            .script(principal.tenant_id(), namespace, id)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Changes the draft of a script. Versions already published are not touched.
    ///
    /// # Errors
    /// As [`Self::create`], and [`AppError::RevisionConflict`] when the draft is no longer at
    /// `expected_revision`.
    pub async fn update(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        change: DraftChange,
        expected_revision: u64,
        ctx: &RequestContext,
    ) -> Result<ScriptRecord, AppError> {
        principal.authorize(Permission::ScriptWrite)?;
        let script = self.load(principal, namespace, id, ctx).await?;
        if script.revision != expected_revision {
            return Err(AppError::RevisionConflict {
                current: script.revision,
            });
        }
        let mut draft = script.draft.clone();
        if let Some(source) = change.source {
            draft.source = source;
        }
        if let Some(lockfile) = change.lockfile {
            draft.lockfile = lockfile;
        }
        if let Some(schema) = change.input_schema {
            self.check_schema(schema.as_deref())?;
            draft.input_schema = schema;
        }
        if let Some(schema) = change.output_schema {
            self.check_schema(schema.as_deref())?;
            draft.output_schema = schema;
        }
        if let Some(request) = change.policy {
            draft.policy = request
                .resolve(&draft.policy, &self.ceiling)
                .map_err(AppError::InvalidScript)?;
        }
        if let Some(retention) = change.retention {
            draft.retention = retention;
        }
        let audit = self.audit(
            principal,
            AuditAction::ScriptChanged,
            ResourceType::Script,
            id.to_string(),
            format!("draft of '{}' changed", script.name),
            ctx,
        );
        self.store
            .update_draft(
                DraftUpdate {
                    tenant: principal.tenant_id(),
                    namespace,
                    id,
                    draft: &draft,
                    expected_revision,
                    now: self.clock.now(),
                },
                &audit,
            )
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Deletes a script that has no version. A script with a version stays: a workflow may pin its
    /// artifact, and an artifact is never taken from under a workflow.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:write`; [`AppError::NotFound`];
    /// [`AppError::StateConflict`] when it has a version.
    pub async fn delete(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::ScriptWrite)?;
        let script = self.load(principal, namespace, id, ctx).await?;
        let audit = self.audit(
            principal,
            AuditAction::ScriptDeleted,
            ResourceType::Script,
            id.to_string(),
            format!("script '{}' deleted", script.name),
            ctx,
        );
        match self
            .store
            .delete_script(principal.tenant_id(), namespace, id, &audit)
            .await?
        {
            Deleted::Done => Ok(()),
            Deleted::NotFound => Err(AppError::NotFound),
            Deleted::HasVersions => Err(AppError::StateConflict(
                "a script with a published version cannot be deleted",
            )),
        }
    }

    /// Publishes the draft as a version and queues its build. Publishing a draft that is the same
    /// as the latest version is not another version.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:publish`; [`AppError::NotFound`];
    /// [`AppError::QuotaExceeded`] when the namespace has too many builds and test runs in progress.
    pub async fn publish(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        ctx: &RequestContext,
    ) -> Result<Published, AppError> {
        principal.authorize(Permission::ScriptPublish)?;
        let script = self.load(principal, namespace, id, ctx).await?;
        let now = self.clock.now();
        let version = NewVersion {
            draft_revision: script.revision,
            id: ScriptVersionId::generate(),
            job_id: BuildJobId::generate(),
            script_id: id,
            tenant_id: principal.tenant_id(),
            namespace_id: namespace,
            language: script.language,
            content_digest: script.draft.content_digest(script.language),
            draft: script.draft.clone(),
            created_by: principal.subject().as_str().to_owned(),
            created_at: now,
        };
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::ScriptJobs,
            )
            .await?;
        let audit = self.audit(
            principal,
            AuditAction::ScriptPublished,
            ResourceType::ScriptVersion,
            version.id.to_string(),
            format!("a version of '{}' was published", script.name),
            ctx,
        );
        let (version, created) = self
            .store
            .publish_version(&version, limit, &audit)
            .await?
            .ok_or(AppError::NotFound)?;
        Ok(Published { version, created })
    }

    /// The versions of a script, newest first.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:read`; [`AppError::NotFound`].
    pub async fn versions(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        before: Option<VersionNumber>,
        limit: u32,
        ctx: &RequestContext,
    ) -> Result<Vec<VersionRecord>, AppError> {
        principal.authorize(Permission::ScriptRead)?;
        self.load(principal, namespace, id, ctx).await?;
        Ok(self
            .store
            .versions(
                principal.tenant_id(),
                namespace,
                id,
                before,
                limit.clamp(1, MAX_LIST),
            )
            .await?)
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `script:read`; [`AppError::NotFound`].
    pub async fn version(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        number: VersionNumber,
        ctx: &RequestContext,
    ) -> Result<VersionRecord, AppError> {
        principal.authorize(Permission::ScriptRead)?;
        self.load(principal, namespace, id, ctx).await?;
        self.store
            .version(principal.tenant_id(), namespace, id, number)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Asks for the draft to be run in the sandbox with `input` (JSON), without publishing it. The
    /// run has no secrets of the namespace: whatever it prints or returns is shown to the author,
    /// and what an author may read is not what a production secret may be shown to.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:write`; [`AppError::NotFound`];
    /// [`AppError::PayloadTooLarge`]; [`AppError::QuotaExceeded`].
    pub async fn test(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        input: Vec<u8>,
        ctx: &RequestContext,
    ) -> Result<TestRunRecord, AppError> {
        principal.authorize(Permission::ScriptWrite)?;
        if input.len() > MAX_INPUT_BYTES {
            return Err(AppError::PayloadTooLarge("a test input is at most 240 KiB"));
        }
        let script = self.load(principal, namespace, id, ctx).await?;
        let mut draft = script.draft.clone();
        // What the platform allows now, not what it allowed when the draft was written.
        draft.policy = draft.policy.within(&self.ceiling);
        let run = TestRunRecord {
            id: TestRunId::generate(),
            script_id: id,
            tenant_id: principal.tenant_id(),
            namespace_id: namespace,
            language: script.language,
            status: JobStatus::Queued,
            draft,
            input,
            result: None,
            failure: None,
            log: String::new(),
            log_truncated: false,
            denied: Vec::new(),
            exit_code: None,
            wall_ms: None,
            start_ms: None,
            memory_peak_bytes: None,
            inferred_input_schema: None,
            inferred_output_schema: None,
            attempts: 0,
            created_by: principal.subject().as_str().to_owned(),
            created_at: self.clock.now(),
            finished_at: None,
        };
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::ScriptJobs,
            )
            .await?;
        let audit = self.audit(
            principal,
            AuditAction::ScriptTested,
            ResourceType::Script,
            id.to_string(),
            format!("test run {} of '{}'", run.id, script.name),
            ctx,
        );
        self.store.create_test_run(&run, limit, &audit).await?;
        Ok(run)
    }

    /// A test run. One that waited too long for a runner reads as failed.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:write`; [`AppError::NotFound`].
    pub async fn test_run(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        run: TestRunId,
        ctx: &RequestContext,
    ) -> Result<TestRunRecord, AppError> {
        principal.authorize(Permission::ScriptWrite)?;
        self.load(principal, namespace, id, ctx).await?;
        self.store
            .test_run(principal.tenant_id(), namespace, id, run, self.clock.now())
            .await?
            .ok_or(AppError::NotFound)
    }

    /// The test runs of a script, newest first.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:write`; [`AppError::NotFound`].
    pub async fn test_runs(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        id: ScriptId,
        limit: u32,
        ctx: &RequestContext,
    ) -> Result<Vec<TestRunRecord>, AppError> {
        principal.authorize(Permission::ScriptWrite)?;
        self.load(principal, namespace, id, ctx).await?;
        Ok(self
            .store
            .test_runs(
                principal.tenant_id(),
                namespace,
                id,
                limit.clamp(1, MAX_LIST),
            )
            .await?)
    }

    /// What published scripts did, newest first, optionally of one artifact.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:read`; [`AppError::NotFound`].
    pub async fn executions(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        digest: Option<&Digest>,
        before: Option<Uuid>,
        limit: u32,
        ctx: &RequestContext,
    ) -> Result<Vec<ExecutionRecord>, AppError> {
        principal.authorize(Permission::ScriptRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .executions(
                principal.tenant_id(),
                namespace,
                digest,
                before,
                limit.clamp(1, MAX_LIST),
            )
            .await?)
    }

    /// The digests among `digests` that are not a ready artifact of the caller's tenant. What a
    /// workflow that pins scripts is checked against when it is published.
    ///
    /// # Errors
    /// A store failure.
    pub async fn missing(
        &self,
        tenant: TenantId,
        digests: &[Digest],
    ) -> Result<Vec<Digest>, AppError> {
        Ok(self.store.missing_artifacts(tenant, digests).await?)
    }

    fn audit(
        &self,
        principal: &Principal,
        action: AuditAction,
        resource: ResourceType,
        id: String,
        reason: String,
        ctx: &RequestContext,
    ) -> AuditRecord {
        AuditRecord::by(
            principal,
            action,
            resource,
            id,
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(reason)
    }
}

// ---------------------------------------------------------------------------------------------
// The runner's service
// ---------------------------------------------------------------------------------------------

/// A build handed to a runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildAssignment {
    pub job: BuildJobId,
    /// Presented back with every call about the job; shown once, kept only as a hash.
    pub lease_token: String,
    pub lease: Duration,
    pub version: ScriptVersionId,
    pub script: ScriptId,
    pub language: Language,
    pub source: Source,
    pub lockfile: Option<Lockfile>,
    /// What the build is held to: what the author asked, under the ceiling of the platform.
    pub policy: Policy,
    pub build_key: Digest,
}

/// A test run handed to a runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestAssignment {
    pub run: TestRunRecord,
    pub lease_token: String,
    pub lease: Duration,
}

/// What a runner asks for an artifact and what it needs to trust it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactView {
    pub artifact: ArtifactRecord,
}

/// The use cases of the sandbox's runner.
pub struct RunnerService<S, C, E> {
    store: S,
    clock: C,
    entropy: E,
    signer: Signer,
    ceiling: Policy,
}

impl<S, C, E> RunnerService<S, C, E>
where
    S: RunnerStore,
    C: Clock,
    E: Entropy,
{
    #[must_use]
    pub const fn new(store: S, clock: C, entropy: E, signer: Signer, ceiling: Policy) -> Self {
        Self {
            store,
            clock,
            entropy,
            signer,
            ceiling,
        }
    }

    /// The public key whose signatures the runners should trust.
    #[must_use]
    pub fn public_key(&self) -> (String, Vec<u8>) {
        (self.signer.key_id().to_owned(), self.signer.public_key())
    }

    fn token(&self) -> Result<Token, AppError> {
        Token::from_bytes(self.entropy.token()).ok_or(AppError::Internal)
    }

    /// Takes the oldest build of a language the runner has a toolchain for. A build whose key is
    /// one that was built already is finished here with the artifact that exists, and the next one
    /// is looked at: the runner only ever sees work that needs doing.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:build`.
    pub async fn claim_build(
        &self,
        principal: &Principal,
        toolchains: &[(Language, String)],
        ctx: &RequestContext,
    ) -> Result<Option<BuildAssignment>, AppError> {
        principal.authorize(Permission::ScriptBuild)?;
        let languages: Vec<Language> = toolchains.iter().map(|(language, _)| *language).collect();
        if languages.is_empty() {
            return Ok(None);
        }
        for _ in 0..MAX_CACHE_HITS_PER_CLAIM {
            let token = self.token()?;
            let now = self.clock.now();
            let Some(claimed) = self
                .store
                .claim_build(principal.tenant_id(), &languages, &token.hash(), LEASE, now)
                .await?
            else {
                return Ok(None);
            };
            let version = &claimed.version;
            let Some((_, toolchain)) = toolchains.iter().find(|(l, _)| *l == version.language)
            else {
                return Err(AppError::Internal);
            };
            let policy = version.draft.policy.within(&self.ceiling);
            let key = build_key(
                version.language,
                &version.draft.source,
                version.draft.lockfile.as_ref(),
                toolchain,
                &policy,
            );
            if let Some(artifact) = self.store.built_with(principal.tenant_id(), &key).await? {
                let audit = self.built_audit(principal, version, &artifact, "reused", ctx);
                match self
                    .store
                    .finish_build_cached(
                        Hold {
                            tenant: principal.tenant_id(),
                            job: claimed.job.id,
                            token_hash: &token.hash(),
                            now,
                        },
                        &key,
                        &artifact,
                        &audit,
                    )
                    .await?
                {
                    Finished::Done | Finished::AlreadyDone => {
                        metrics::counter!("script_builds_total", "outcome" => "cached")
                            .increment(1);
                        continue;
                    }
                    Finished::Stale => continue,
                }
            }
            return Ok(Some(BuildAssignment {
                job: claimed.job.id,
                lease_token: token.expose().to_owned(),
                lease: LEASE,
                version: version.id,
                script: version.script_id,
                language: version.language,
                source: version.draft.source.clone(),
                lockfile: version.draft.lockfile.clone(),
                policy,
                build_key: key,
            }));
        }
        Ok(None)
    }

    /// Extends the lease of a build. `None` when the runner no longer holds it.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:build`.
    pub async fn heartbeat_build(
        &self,
        principal: &Principal,
        job: BuildJobId,
        lease_token: &str,
    ) -> Result<Option<Duration>, AppError> {
        principal.authorize(Permission::ScriptBuild)?;
        Ok(self
            .store
            .heartbeat_build(
                principal.tenant_id(),
                job,
                &Token::hash_of(lease_token),
                LEASE,
                self.clock.now(),
            )
            .await?
            .map(|_| LEASE))
    }

    /// Accepts what a runner made of a build: signs the artifact and marks the version ready, or
    /// records why there is none.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:build`; [`AppError::StaleAttempt`] when the runner
    /// no longer holds the build; [`AppError::PayloadTooLarge`].
    pub async fn complete_build(
        &self,
        principal: &Principal,
        job: BuildJobId,
        lease_token: &str,
        report: BuildReport,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::ScriptBuild)?;
        let hash = Token::hash_of(lease_token);
        let tenant = principal.tenant_id();
        let now = self.clock.now();
        let claimed = self
            .store
            .held_build(tenant, job, &hash)
            .await?
            .ok_or(AppError::StaleAttempt)?;
        let version = &claimed.version;
        let finished = match report {
            BuildReport::Built {
                artifact,
                sbom,
                toolchain,
                log,
            } => {
                if artifact.len() > MAX_ARTIFACT_BYTES || sbom.len() > MAX_SBOM_BYTES {
                    return Err(AppError::PayloadTooLarge("an artifact is at most 2 MiB"));
                }
                // The key is computed again from the version and the toolchain the runner built
                // with, never taken from the runner: it is what a later build is compared to.
                let policy = version.draft.policy.within(&self.ceiling);
                let key = build_key(
                    version.language,
                    &version.draft.source,
                    version.draft.lockfile.as_ref(),
                    &toolchain,
                    &policy,
                );
                let digest = digest_of(&artifact);
                let signature = self.signer.sign(tenant, &digest, version.language);
                let built = BuiltArtifact {
                    artifact: ArtifactRecord {
                        tenant_id: tenant,
                        digest: digest.clone(),
                        language: version.language,
                        content: artifact,
                        signature,
                        key_id: self.signer.key_id().to_owned(),
                        sbom,
                        toolchain,
                        created_at: now,
                    },
                    build_key: key,
                    log: clip(&log, MAX_BUILD_LOG_BYTES),
                };
                let audit = self.built_audit(principal, version, &digest, "built", ctx);
                self.store
                    .finish_build(
                        Hold {
                            tenant,
                            job,
                            token_hash: &hash,
                            now,
                        },
                        &built,
                        &audit,
                    )
                    .await?
            }
            BuildReport::Failed { kind, message, log } => {
                let reason = format!("{}: {}", kind.as_str(), clip(&message, 1024));
                let audit = AuditRecord::by(
                    principal,
                    AuditAction::ScriptBuilt,
                    ResourceType::ScriptVersion,
                    version.id.to_string(),
                    ctx.trace_id.clone(),
                    now,
                )
                .with_reason(format!("build failed ({})", kind.as_str()));
                self.store
                    .fail_build(
                        Hold {
                            tenant,
                            job,
                            token_hash: &hash,
                            now,
                        },
                        &reason,
                        &clip(&log, MAX_BUILD_LOG_BYTES),
                        kind.retryable(),
                        &audit,
                    )
                    .await?
            }
        };
        metrics::counter!("script_builds_total", "outcome" => "completed").increment(1);
        match finished {
            Finished::Done | Finished::AlreadyDone => Ok(()),
            Finished::Stale => Err(AppError::StaleAttempt),
        }
    }

    /// Takes the oldest test run of a language the runner can run.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:build`.
    pub async fn claim_test(
        &self,
        principal: &Principal,
        languages: &[Language],
    ) -> Result<Option<TestAssignment>, AppError> {
        principal.authorize(Permission::ScriptBuild)?;
        if languages.is_empty() {
            return Ok(None);
        }
        let token = self.token()?;
        let claimed = self
            .store
            .claim_test(
                principal.tenant_id(),
                languages,
                &token.hash(),
                LEASE,
                self.clock.now(),
            )
            .await?;
        Ok(claimed.map(|claimed| TestAssignment {
            run: claimed.run,
            lease_token: token.expose().to_owned(),
            lease: LEASE,
        }))
    }

    /// Extends the lease of a test run. `None` when the runner no longer holds it.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:build`.
    pub async fn heartbeat_test(
        &self,
        principal: &Principal,
        run: TestRunId,
        lease_token: &str,
    ) -> Result<Option<Duration>, AppError> {
        principal.authorize(Permission::ScriptBuild)?;
        Ok(self
            .store
            .heartbeat_test(
                principal.tenant_id(),
                run,
                &Token::hash_of(lease_token),
                LEASE,
                self.clock.now(),
            )
            .await?
            .map(|_| LEASE))
    }

    /// Accepts the outcome of a test run.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:build`; [`AppError::StaleAttempt`];
    /// [`AppError::PayloadTooLarge`].
    pub async fn complete_test(
        &self,
        principal: &Principal,
        run: TestRunId,
        lease_token: &str,
        mut report: TestReport,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::ScriptBuild)?;
        if report
            .result
            .as_ref()
            .is_ok_and(|r| r.len() > MAX_RESULT_BYTES)
        {
            return Err(AppError::PayloadTooLarge("a result is at most 240 KiB"));
        }
        report.log = clip(&report.log, 1024 * 1024);
        report.denied.truncate(MAX_VIOLATIONS_AUDITED);
        let now = self.clock.now();
        let audits: Vec<AuditRecord> = report
            .denied
            .iter()
            .map(|what| self.violation_audit(principal, run.to_string(), what, ctx))
            .collect();
        match self
            .store
            .finish_test(
                principal.tenant_id(),
                run,
                &Token::hash_of(lease_token),
                &report,
                now,
                &audits,
            )
            .await?
        {
            Finished::Done | Finished::AlreadyDone => Ok(()),
            Finished::Stale => Err(AppError::StaleAttempt),
        }
    }

    /// An artifact of the caller's tenant, with the signature to verify before running it.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:execute`; [`AppError::NotFound`].
    pub async fn artifact(
        &self,
        principal: &Principal,
        digest: &Digest,
    ) -> Result<ArtifactView, AppError> {
        principal.authorize(Permission::ScriptExecute)?;
        let artifact = self
            .store
            .artifact(principal.tenant_id(), digest)
            .await?
            .ok_or(AppError::NotFound)?;
        Ok(ArtifactView { artifact })
    }

    /// Keeps the record of an execution, and audits what the script was refused (once per script,
    /// kind and destination in a window: a script in a loop cannot grow the trail).
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `script:execute`; [`AppError::NotFound`] for an artifact
    /// the tenant does not have or a namespace it does not own.
    pub async fn record_execution(
        &self,
        principal: &Principal,
        mut execution: NewExecution,
        ctx: &RequestContext,
    ) -> Result<bool, AppError> {
        principal.authorize(Permission::ScriptExecute)?;
        let tenant = principal.tenant_id();
        require_namespace(
            &self.store,
            &self.clock,
            principal,
            execution.namespace_id,
            ctx,
        )
        .await?;
        let retention = self
            .store
            .retention_of(tenant, &execution.digest)
            .await?
            .ok_or(AppError::NotFound)?;
        execution.log = clip(&execution.log, 1024 * 1024);
        execution.denied.truncate(MAX_VIOLATIONS_AUDITED);
        let now = self.clock.now();
        let audits: Vec<AuditRecord> = execution
            .denied
            .iter()
            .map(|what| self.violation_audit(principal, execution.digest.to_string(), what, ctx))
            .collect();
        let record = ExecutionRecord {
            id: Uuid::now_v7(),
            tenant_id: tenant,
            expires_at: now + time::Duration::days(i64::from(retention.get())),
            created_at: now,
            execution,
        };
        let kept = self.store.record_execution(&record, &audits).await?;
        metrics::counter!(
            "script_executions_total",
            "status" => record.execution.status.as_str()
        )
        .increment(1);
        Ok(kept)
    }

    fn violation_audit(
        &self,
        principal: &Principal,
        resource: String,
        what: &str,
        ctx: &RequestContext,
    ) -> AuditRecord {
        AuditRecord::by(
            principal,
            AuditAction::ScriptViolation,
            ResourceType::Script,
            resource,
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        // What the script was refused: the kind and the destination, never the path or a secret's
        // value.
        .with_reason(clip(what, 200))
    }

    fn built_audit(
        &self,
        principal: &Principal,
        version: &VersionRecord,
        artifact: &Digest,
        how: &str,
        ctx: &RequestContext,
    ) -> AuditRecord {
        AuditRecord::by(
            principal,
            AuditAction::ScriptBuilt,
            ResourceType::ScriptVersion,
            version.id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(format!("{how}: {artifact}"))
    }
}

/// `text` cut to at most `max` bytes, on a character.
fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Maps a name conflict raised by the store.
#[must_use]
pub const fn script_name_conflict() -> AppError {
    AppError::Conflict(ConflictKind::ScriptName)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(source: &str) -> Draft {
        Draft {
            source: Source::parse(source).expect("source"),
            lockfile: None,
            input_schema: None,
            output_schema: None,
            policy: Policy::default(),
            retention: Retention::DEFAULT,
        }
    }

    #[test]
    fn two_drafts_are_one_version_only_if_every_field_is_the_same() {
        let base = draft("export default () => 1");
        let key = base.content_digest(Language::TypeScript);
        assert_eq!(key, base.clone().content_digest(Language::TypeScript));
        assert_ne!(key, base.content_digest(Language::Python), "language");
        let mut other = base.clone();
        other.source = Source::parse("export default () => 2").expect("source");
        assert_ne!(key, other.content_digest(Language::TypeScript), "source");
        let mut other = base.clone();
        other.lockfile = Some(Lockfile::parse("{}").expect("lockfile"));
        assert_ne!(key, other.content_digest(Language::TypeScript), "lockfile");
        let mut other = base.clone();
        other.input_schema = Some(b"{}".to_vec());
        assert_ne!(
            key,
            other.content_digest(Language::TypeScript),
            "input schema"
        );
        let mut other = base.clone();
        other.output_schema = Some(b"{}".to_vec());
        assert_ne!(
            key,
            other.content_digest(Language::TypeScript),
            "output schema"
        );
        let mut other = base.clone();
        other.policy.egress = Egress::Namespace;
        assert_ne!(key, other.content_digest(Language::TypeScript), "policy");
        let mut other = base.clone();
        other.retention = Retention::days(30).expect("retention");
        assert_ne!(key, other.content_digest(Language::TypeScript), "retention");
        // The fields cannot be shifted into one another.
        let mut a = base.clone();
        a.input_schema = Some(b"ab".to_vec());
        a.output_schema = Some(b"c".to_vec());
        let mut b = base;
        b.input_schema = Some(b"a".to_vec());
        b.output_schema = Some(b"bc".to_vec());
        assert_ne!(
            a.content_digest(Language::TypeScript),
            b.content_digest(Language::TypeScript)
        );
    }

    #[test]
    fn a_policy_request_fills_in_the_defaults_and_stops_at_the_ceiling() {
        let ceiling = Policy::default();
        let none = PolicyRequest::default();
        assert_eq!(
            none.resolve(&Policy::default(), &ceiling),
            Ok(Policy::default())
        );
        let less = PolicyRequest {
            memory_bytes: Some(64 * 1024 * 1024),
            wall: Some(Duration::from_secs(5)),
            ..PolicyRequest::default()
        };
        let policy = less
            .resolve(&Policy::default(), &ceiling)
            .expect("less is allowed");
        assert_eq!(policy.limits.memory_bytes(), 64 * 1024 * 1024);
        assert_eq!(policy.limits.wall(), Duration::from_secs(5));
        let more = PolicyRequest {
            memory_bytes: Some(512 * 1024 * 1024),
            ..PolicyRequest::default()
        };
        assert_eq!(
            more.resolve(&Policy::default(), &ceiling),
            Err(ScriptError::AboveCeiling)
        );
        let network = PolicyRequest {
            egress: Some(Egress::Namespace),
            ..PolicyRequest::default()
        };
        assert_eq!(
            network.resolve(&Policy::default(), &ceiling),
            Err(ScriptError::AboveCeiling),
            "a ceiling without a network is not widened by a request"
        );
        let out_of_bounds = PolicyRequest {
            memory_bytes: Some(1),
            ..PolicyRequest::default()
        };
        assert_eq!(
            out_of_bounds.resolve(&Policy::default(), &ceiling),
            Err(ScriptError::InvalidMemory)
        );
    }

    #[test]
    fn a_text_is_cut_on_a_character() {
        assert_eq!(clip("héllo", 2), "h");
        assert_eq!(clip("héllo", 3), "hé");
        assert_eq!(clip("abc", 10), "abc");
    }
}

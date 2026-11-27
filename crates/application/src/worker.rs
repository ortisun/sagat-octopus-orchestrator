//! The worker protocol (`docs/architecture/worker-runtime-protocol.md`): register, claim, heartbeat,
//! complete and fail, all fenced by the lease.
//!
//! NATS (or polling) only *hints* that work exists. The lease is granted here, under the run's row
//! lock, by one canonical operation: the step moves `READY → LEASED`, its generation increases and a
//! secret token is handed to exactly one worker. Anything the worker later reports must present the
//! attempt, the generation and that token; a report from an older holder is refused with
//! [`AppError::StaleAttempt`] and changes nothing, although the external effect it describes may
//! have happened (hence idempotency keys).

pub mod acknowledgement;
mod binding_checks;
mod claim_counters;
mod database;
mod lake_complete;
pub mod lake_input;
pub mod lake_lease;
pub mod lake_probe_input;
pub mod lake_result;
mod output;
mod poll_order;
mod scoped;
mod scoped_capabilities;
mod scoped_claims;
mod scoped_output_refusal;
mod scoped_reports;
mod secret_resolution;
mod secrets;
mod withdrawal;

use claim_counters::{ClaimProgress, count_claim};
pub use output::BlobOutput;
use poll_order::{PollsInFlight, start_past};
use secrets::SecretReport;
pub use withdrawal::{
    HAND_BACK_PROTOCOL_MINOR, HandBackVerdict, TASK_NOT_STARTED, UndeliveredLease,
    WithdrawalOutcome, count as count_undelivered, hand_back, hand_back_verdict,
};

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::binding::BindingProfile;
use sagat_domain::execution::{
    self, ExecutionInstanceId, FailureCode, Fence, Progress, RunEvent, RunId, RunStatus, StepStatus,
};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Role, Slug, TenantId};
use sagat_domain::workflow::{ActivityName, NodeKind};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::blobs::{BlobAccess, BlobScope};
use crate::clock::Clock;
use crate::definitions::{DefinitionStore, Idempotency};
use crate::entropy::{Entropy, Token};
use crate::error::{AppError, StoreError};
use crate::execution::{
    Actor, ClaimMark, Effects, Engine, EngineError, ExpressionEvaluator, LeaseGrant, LeaseMarking,
    LeaseRenewal, MAX_PAYLOAD_BYTES, MAX_RUN_OUTPUT_BYTES, MarkOutcome, MarkSkip,
    PathExecutionStore, PathExecutionTx, PoolCheck, ReportedLease, RunTx,
    instance_compensation_key, instance_reconciliation_key, instance_step_id,
};
use crate::quotas::QuotaLookup;
use crate::runs::{Lock, RunStore, execution_error};
use crate::secrets::{SecretProblem, SecretState, check_usable};
use sagat_domain::binding::ActivityPattern;
use sagat_domain::blobs::BlobId;
use sagat_domain::quota::QuotaResource;
use sagat_domain::secrets::{SecretCipher, SecretName, SecretValue};

/// The newest minor version of worker protocol 1 this server speaks. Every minor version adds fields
/// and calls and removes none, so a worker of any minor version of protocol 1 can be served
/// (`negotiate_protocol`). 1.1 added the step identity to `Heartbeat`, 1.2 `task_kind` and
/// `ResolveSecrets`; 1.3 streaming blobs; 1.4 bounded task streams; 1.5 scoped execution identity;
/// 1.6 managed database capabilities (the combined protocol: a 1.6 session also speaks 1.5).
pub const PROTOCOL_MINOR: u32 = 6;
/// The newest minor a storage adapter may negotiate before it proves scoped claim and report.
pub const ROOT_PROTOCOL_MINOR: u32 = 4;
/// Scoped execution identity is allocated to worker protocol 1.5. A store must explicitly declare
/// that its claim and report transactions address scoped instances before this minor is negotiated.
pub const SCOPED_PROTOCOL_MINOR: u32 = 5;
/// Managed database capabilities first became available in protocol 1.6 (decision D600).
pub const DATABASE_PROTOCOL_MINOR: u32 = 6;

/// Delivery acknowledgement (feature 006, ADR-0099): the minor that marks a lease provisional until its host acknowledges
/// it, and that reserves `TASK_NOT_STARTED` for the host (decision D006-061). Negotiated only once the compiled ceiling,
/// [`PROTOCOL_MINOR`], reaches it (Task 006-057) and the store proves the paths of 1.8.
pub const DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR: u32 = HAND_BACK_PROTOCOL_MINOR;

/// What a storage adapter proves it serves, each the precondition of a minor (minors are cumulative proofs, decision
/// D006-041).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoreProofs {
    /// The scoped claim and report path (1.5).
    scoped: bool,
    /// The managed database path (1.6).
    database: bool,
    /// The paths of delivery acknowledgement (1.8): the marks, the acknowledgement, the capability record and the
    /// withdrawals of a marked lease.
    delivery_acknowledgement: bool,
}

/// The newest minor this session may negotiate, never above `compiled` (this build's [`PROTOCOL_MINOR`]). Scoped
/// identity (1.5) needs a store that proves the scoped claim and report path; the managed database minor (1.6) is a
/// superset of 1.5 for ordinary workers, so it needs both proofs. The dedicated database runner only ever handles root
/// work, so it reaches 1.6 on a store that proves the database path alone, and it stays there (ADR-0099 decision 11).
/// Delivery acknowledgement (1.8) is answered to an ordinary worker when the minors below it are and the store proves
/// it (decision D006-105).
fn protocol_ceiling(role: Role, proofs: StoreProofs, compiled: u32) -> u32 {
    let proven = if proofs.database && (proofs.scoped || role == Role::DatabaseRunner) {
        if proofs.delivery_acknowledgement && role != Role::DatabaseRunner {
            DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR
        } else {
            DATABASE_PROTOCOL_MINOR
        }
    } else if proofs.scoped {
        SCOPED_PROTOCOL_MINOR
    } else {
        ROOT_PROTOCOL_MINOR
    };
    proven.min(compiled)
}
/// Streaming blob references first became safe to hand to workers in protocol 1.3.
pub const BLOB_PROTOCOL_MINOR: u32 = 3;
/// Server-streaming task receive first became available in protocol 1.4.
pub const STREAM_PROTOCOL_MINOR: u32 = 4;

/// Reserved managed activities require the dedicated machine identity and capability protocol.
///
/// A dedicated runner is of one **profile**, and the profile is the set of activities it registers (feature 035,
/// D035-135): the SQL profile is `sagat.database.postgresql`, the lake profile is `sagat.database.iceberg.write`. The two
/// are never mixed and an ordinary activity is never offered beside them; the patterns of the runner's binding decide
/// which of the two it may register, as they decide every other registration. An activity the server does not serve yet
/// (the revert, the destinations and the browse) is refused here, so that a runner is not left waiting for work that cannot
/// come; the probe of an Iceberg connection is served, and a lake runner offers it beside its write (D035-150).
#[must_use]
pub fn managed_capabilities_allowed(role: Role, activities: &[String], minor: u32) -> bool {
    use sagat_domain::database::MANAGED_DATABASE_PREFIX;
    if role == Role::DatabaseRunner {
        minor >= DATABASE_PROTOCOL_MINOR && managed_profile(activities).is_some()
    } else {
        activities
            .iter()
            .all(|a| !a.starts_with(MANAGED_DATABASE_PREFIX))
    }
}

/// Whether the credentials of a step of `kind` may be handed to a session that offers `offered`, whose binding has the rows of
/// `bindings` (their profiles, one per row): the session offers the managed activities of the profile of the kind, and the binding
/// is of that profile alone. A binding with no row, or with rows that disagree, is not a binding of the profile. Registration and
/// the lease already reach only such a session; this is the check in the transaction that delivers the secret, which does not trust
/// that the rows it reads are the ones registration saw (D035-139).
#[must_use]
pub fn profile_fence(
    kind: sagat_domain::database::ManagedKind,
    offered: &[String],
    bindings: &[BindingProfile],
) -> bool {
    managed_profile(offered) == Some(kind.profile())
        && offered.iter().any(|activity| activity == kind.activity())
        && !bindings.is_empty()
        && bindings.iter().all(|profile| *profile == kind.profile())
}

/// The profile a registration is for: `database` when it offers `sagat.database.postgresql` alone, `lake` when it offers the Iceberg
/// write, the Iceberg probe or both (`sagat.database.iceberg.write`, `sagat.database.lake.probe`; a lake runner that has a probe to run
/// offers it beside its write, and the offer is what tells the server it may be handed one, D035-150), and `None` for anything else (an
/// empty list, a mix of profiles, an ordinary activity, an activity the server does not serve yet). The binding of the identity must
/// carry the same profile (D035-057).
#[must_use]
pub fn managed_profile(activities: &[String]) -> Option<BindingProfile> {
    use sagat_domain::database::{
        ICEBERG_PROBE_ACTIVITY, ICEBERG_WRITE_ACTIVITY, MANAGED_DATABASE_ACTIVITY,
    };
    if activities.is_empty() {
        None
    } else if activities.iter().all(|a| a == MANAGED_DATABASE_ACTIVITY) {
        Some(BindingProfile::Database)
    } else if activities
        .iter()
        .all(|a| a == ICEBERG_WRITE_ACTIVITY || a == ICEBERG_PROBE_ACTIVITY)
    {
        Some(BindingProfile::Lake)
    } else {
        None
    }
}

/// What registration says when it refuses the protocol version (a stable reason for SDKs).
pub const UNSUPPORTED_PROTOCOL: &str = "unsupported protocol version";

/// What a worker offers is `1` or `1.<minor>`; what is agreed is the newest version both speak:
/// the worker's own if it is not newer than the server's, the server's otherwise (so an SDK that
/// is ahead of its server runs at the server's version and an SDK that is behind runs at its own).
/// `None` for another major version and for anything that is not a version.
#[must_use]
pub fn negotiate_protocol(offered: &str) -> Option<String> {
    negotiate_protocol_details(offered, PROTOCOL_MINOR).map(|(version, _)| version)
}

fn negotiate_protocol_details(offered: &str, server_minor: u32) -> Option<(String, u32)> {
    let minor = match offered.split_once('.') {
        None if offered == "1" => 0,
        Some(("1", minor))
            if !minor.is_empty()
                && minor.len() <= 6
                && minor.bytes().all(|b| b.is_ascii_digit())
                && (minor == "0" || !minor.starts_with('0')) =>
        {
            minor.parse::<u32>().ok()?
        }
        _ => return None,
    };
    let agreed = minor.min(server_minor);
    let version = if agreed == 0 {
        "1".to_owned()
    } else {
        format!("1.{agreed}")
    };
    Some((version, agreed))
}
/// Most activities one worker may advertise.
pub const MAX_ACTIVITIES: usize = 256;
/// Most tasks one poll may claim.
pub const MAX_POLL: u32 = 64;
/// Most live sessions one worker identity may hold. A session is identified by the identity and the
/// worker name, so registering again replaces the previous one instead of adding a row.
pub const MAX_SESSIONS_PER_IDENTITY: u32 = 32;
/// Runs that may fail to be leased in one poll before it gives up (a store that is failing, not a
/// run).
const MAX_CLAIM_FAILURES: u8 = 3;
/// Most entries in the `languages` and `pools` a worker advertises.
pub const MAX_LABELS: usize = 16;
/// Longest `languages` or `pools` entry.
pub const MAX_LABEL_LEN: usize = 64;
/// Most task input one poll hands out (the sum over its tasks); steps that do not fit stay ready for
/// the next poll. Each task carries the run's data, so without this a poll of many steps of a run
/// with large outputs would be many times the run's size.
pub const MAX_POLL_BYTES: usize = 8 * 1024 * 1024;
/// A serialized task context must fit in an otherwise empty poll. Stored JSON can grow when it is
/// rendered (notably numeric exponents), so the retained-output limit cannot replace this check.
pub const MAX_TASK_INPUT_BYTES: usize = MAX_POLL_BYTES;

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
    /// The host asks for no flagged lease (protocol 1.8, `WorkerCapabilities.declines_acknowledgement`): stored on the
    /// session from each registration, so one without it clears it (decision D006-105).
    pub declines_acknowledgement: bool,
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
    /// The minor version actually negotiated with the server, rather than the version offered.
    pub accepted_protocol_minor: u32,
    pub expires_at: OffsetDateTime,
    /// Its identity is being drained in this namespace: it is given no new work.
    pub draining: bool,
    /// What its last registration said: the host asks for no flagged lease (protocol 1.8). Read by the claim, which
    /// marks no lease of a declining session (`declined`).
    pub declines_acknowledgement: bool,
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
    pub accepted_protocol_minor: u32,
    /// Whether the host declines acknowledgement: the upsert writes it on insert and on replacement alike.
    pub declines_acknowledgement: bool,
    pub ttl: Duration,
    /// Live sessions the tenant may hold (a quota), judged atomically with the insert.
    pub tenant_limit: u64,
}

/// Storage port for workers. Implemented by `sagat-infra-postgres`.
pub trait WorkerStore: QuotaLookup + Send + Sync {
    /// Whether this adapter can claim and mutate a scoped execution instance transactionally.
    /// The default keeps scoped work unavailable until an adapter proves the full storage path.
    fn supports_scoped_worker_protocol(&self) -> bool {
        false
    }

    /// Managed capabilities require an independently proven adapter boundary.
    fn supports_database_worker_protocol(&self) -> bool {
        false
    }

    /// Whether this adapter stores step logs (design 041, protocol 1.7): the store of `AppendStepLogs` and of a
    /// report's carry. False by default, so that an adapter that has not proven the storage keeps workers at 1.6.
    fn supports_step_log_protocol(&self) -> bool {
        false
    }

    /// Whether this adapter serves every store path of delivery acknowledgement (protocol 1.8): the marks the claim
    /// writes, the acknowledgement, the capability record and the withdrawals of a marked lease. A constant of the
    /// adapter; without it no session negotiates 1.8 whatever the compiled ceiling (decision D006-105).
    fn supports_delivery_acknowledgement(&self) -> bool {
        false
    }

    /// The activity patterns `subject` is bound to in `namespace` (canonical text, see
    /// `sagat_domain::binding`). Empty when the identity has no binding there.
    fn bound_patterns(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;

    /// Records that a worker asked for work it is not bound to (best effort, off the response
    /// path, collapsed when repeated).
    fn record_unbound(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Stores the session, replacing the one the same identity opened under the same worker name, and
    /// returns the id of the stored session (the replaced one keeps its id, so the steps it has
    /// leased stay counted against it). [`StoreError::QuotaExceeded`] when the identity already
    /// holds [`MAX_SESSIONS_PER_IDENTITY`] live sessions under other names.
    ///
    /// The binding is judged again inside the transaction that stores the session, under the lock
    /// that changing a binding takes: `None` when it no longer covers every activity of the session
    /// (it was narrowed after the caller looked). Without that, a revocation committing between the
    /// caller's check and this insert would find no session to close, and the session would outlive it.
    fn create_session(
        &self,
        session: &NewSession,
    ) -> impl Future<Output = Result<Option<Uuid>, StoreError>> + Send;

    /// The live session `id` of `tenant` whose token hashes to `token_hash`, renewing its expiry.
    /// Acquire the session guard before sampling the database clock; a session that expired while
    /// waiting must return `None` without renewal. Keep the guard through renewal commit.
    fn use_session(
        &self,
        tenant: TenantId,
        id: Uuid,
        token_hash: &[u8],
        ttl: Duration,
    ) -> impl Future<Output = Result<Option<SessionRecord>, StoreError>> + Send;

    /// The same bounded, quota/pool/protocol-aware discovery window as `claimable_runs`, retaining
    /// exact path coordinates so scoped claims never need whole-run restoration to find work.
    /// `capacity` selects the part of the window a guarded worker asks for.
    #[allow(clippy::too_many_arguments)] // one bounded discovery window and its judging context
    fn claimable_paths(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        activities: &[String],
        limit: u32,
        accepted_protocol_minor: u32,
        subject: &str,
        capacity: CapacityWindow,
    ) -> impl Future<
        Output = Result<Vec<(RunId, sagat_domain::execution::InstancePath)>, StoreError>,
    > + Send;

    /// Runs of one namespace that have a `READY` step for one of `activities`, oldest first. Steps
    /// that declare a secret restricted to worker pools the identity `subject` holds none of are
    /// left out of the window, bar a few per activity that the claim refuses and audits (G48).
    #[allow(clippy::too_many_arguments)] // one bounded discovery window and its judging context
    fn claimable_runs(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        activities: &[String],
        limit: u32,
        accepted_protocol_minor: u32,
        subject: &str,
        capacity: CapacityWindow,
    ) -> impl Future<Output = Result<Vec<RunId>, StoreError>> + Send;
}

/// Which part of the candidate window a poll asks for when it enforces ChildRoot possession
/// (ADR-0075, G715). The window is only a hint: it is never wider than the possession
/// prerequisite in the sense that matters (nothing the prerequisite would grant is left out of
/// `Eligible`), and the claim transaction stays the authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityWindow {
    /// Every run, as before the prerequisite existed. The only window of a worker without the guard.
    Any,
    /// Runs the prerequisite does not certainly refuse.
    Eligible,
    /// Runs it certainly refuses: an ACTIVE ChildRoot member that is not the holder of its group.
    Ineligible,
}

/// Runs of the `Ineligible` window a guarded poll still reads, behind its eligible candidates.
/// Possession gates only the grant of a lease (O706), so such a run still needs its claim
/// transaction for deterministic internal progress, for instance failing a step whose input is
/// over the limit. The sample is bounded so stuck non-holders cannot crowd the window.
const INELIGIBLE_SAMPLE: u32 = 4;

/// The result of registering.
#[derive(Debug, Clone)]
pub struct Registered {
    pub worker_id: Uuid,
    pub session_token: Token,
    pub accepted_protocol_version: String,
    pub expires_at: OffsetDateTime,
}

/// A task handed to a worker.
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub run_id: RunId,
    pub instance: ExecutionInstanceId,
    pub step_id: Uuid,
    pub fence: Fence,
    pub lease_token: Token,
    pub expires_at: OffsetDateTime,
    pub activity: ActivityName,
    pub input: Vec<u8>,
    pub input_blob: Option<BlobId>,
    /// Stable for the step: the same on every attempt, so the provider can recognise a repeat. For a
    /// compensation it is stable per run, step and action (`compensation_key`).
    pub idempotency_key: String,
    pub trace_id: Option<String>,
    /// What the task is for: forward work, undoing a step, or finding out whether it happened.
    pub kind: TaskKind,
    /// For a compensation or a reconciliation: the forward step it is about, and the idempotency
    /// key that step's own attempts carried, which is what identifies the effect at the provider.
    pub target: Option<TaskTarget>,
    /// What the claim marked (protocol 1.8, Task 006-047): `Provisional` sets `acknowledge_before_start` on the lease.
    /// Counted once the claim's transaction commits, never before. `None`: the store wrote no mark (an adapter without
    /// them).
    pub mark: Option<MarkOutcome>,
    /// The attempt deadline of a provisional lease, when it comes before its expiry (`attempt_deadline_epoch_ms`); the
    /// stored expiry is not shortened.
    pub attempt_deadline: Option<OffsetDateTime>,
}

/// What a task is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Forward,
    Compensation,
    Reconciliation,
}

impl TaskKind {
    /// The name carried by the worker protocol.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "FORWARD",
            Self::Compensation => "COMPENSATION",
            Self::Reconciliation => "RECONCILIATION",
        }
    }
}

/// The forward step a compensation or a reconciliation is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTarget {
    pub instance: ExecutionInstanceId,
    pub idempotency_key: String,
}

impl ClaimedTask {
    /// The lease is provisional until its host acknowledges it (`acknowledge_before_start`, protocol 1.8).
    #[must_use]
    pub fn acknowledge_before_start(&self) -> bool {
        self.mark == Some(MarkOutcome::Provisional)
    }

    /// The legacy identifier a worker echoes back: `run/node/generation`.
    /// Protocol 1.5 carries the complete logical identity in the separate `instance` field.
    #[must_use]
    pub fn task_id(&self) -> String {
        format!(
            "{}/{}/{}",
            self.run_id,
            self.instance.node(),
            self.fence.generation
        )
    }
}

#[cfg(test)]
fn root_task_id(run_id: RunId, instance: &ExecutionInstanceId, generation: u64) -> Option<String> {
    instance
        .path()
        .is_root()
        .then(|| format!("{run_id}/{}/{generation}", instance.node()))
}

fn require_supported_task(
    instance: &ExecutionInstanceId,
    supports_scoped_identity: bool,
) -> Result<(), AppError> {
    if instance.path().is_root() || supports_scoped_identity {
        Ok(())
    } else {
        Err(AppError::StaleAttempt)
    }
}

/// What the caller of a claim says while the claim runs (feature 006, decisions D006-035 and D006-092 of
/// `specs/006-sdk-multilanguage/research.md`). A claim commits one transaction per run; between two of them nothing is half
/// done, so it stops there once its consumer is gone and returns what it already committed, which the caller hands to the
/// withdrawal. A run's transaction whose only writes are its leases is rolled back instead of committed when the consumer is
/// gone just before its commit.
pub trait ClaimWatch: Send + Sync {
    /// Whether the consumer of the claim is gone (a stream body dropped or past its window, a unary handler dropped);
    /// asked before each run's transaction and just before the commit of one that leased.
    fn consumer_gone(&self) -> bool;

    /// Awaited after each run's transaction committed at least one lease, before the next run is claimed: a test's barrier
    /// (decision D006-008). Production returns at once.
    fn committed(&self) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(std::future::ready(()))
    }

    /// Awaited just before the commit of a run's transaction that leased, before the claim asks again whether its consumer
    /// is gone (decision D006-092): a test's barrier (decision D006-008). Production returns at once.
    fn before_commit(&self) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(std::future::ready(()))
    }
}

/// Whether the run's transaction that is about to commit leases must be dropped instead, which rolls it back, because the
/// consumer of the claim went away while it ran (feature 006, decision D006-092): a claim that waited on a lock when its client
/// left would otherwise commit leases nobody receives. Only a transaction whose only writes are its leases is rolled back:
/// with `kept_writes` (a refusal's audit, a permanent failure) it commits and its leases go to the withdrawal. Waits first at
/// the test's barrier.
pub(crate) async fn roll_back_for_a_gone_consumer(
    watch: &dyn ClaimWatch,
    kept_writes: bool,
) -> bool {
    watch.before_commit().await;
    let gone = !kept_writes && watch.consumer_gone();
    if gone {
        metrics::counter!("worker_claim_skipped_total", "reason" => "consumer_gone").increment(1);
    }
    gone
}

/// Counts what the claims of `tasks` marked (protocol 1.8): `worker_lease_marks_total` for a provisional lease, and
/// `worker_lease_marks_skipped_total{reason}` for a lease of a session at 1.8 or later that was not. Called only after the
/// claim's transaction committed, so a transaction rolled back for a consumer gone before its commit counts nothing (decision
/// D006-092).
pub(crate) fn count_marks(tasks: &[ClaimedTask]) {
    for task in tasks {
        match task.mark {
            Some(MarkOutcome::Provisional) => {
                metrics::counter!("worker_lease_marks_total").increment(1)
            }
            Some(MarkOutcome::Skipped(reason)) => {
                metrics::counter!("worker_lease_marks_skipped_total", "reason" => reason.as_str())
                    .increment(1);
            }
            Some(MarkOutcome::Unreserved) | None => {}
        }
    }
}

/// A claim whose consumer never goes away: [`WorkerService::poll`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Unwatched;

impl ClaimWatch for Unwatched {
    fn consumer_gone(&self) -> bool {
        false
    }
}

/// Identifies the attempt a worker is reporting on.
#[derive(Clone)]
pub struct TaskRef<'a> {
    pub run_id: RunId,
    pub instance: &'a ExecutionInstanceId,
    /// The step identity the worker was handed; checked when present.
    pub step_id: Option<Uuid>,
    pub fence: Fence,
    pub lease_token: &'a str,
}

impl std::fmt::Debug for TaskRef<'_> {
    /// The lease token is a bearer secret: it is never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskRef")
            .field("run_id", &self.run_id)
            .field("instance", &self.instance)
            .field("step_id", &self.step_id)
            .field("fence", &self.fence)
            .field("lease_token", &"<redacted>")
            .finish()
    }
}

/// A secret handed to a worker for one attempt. The value prints as `<redacted>`.
#[derive(Debug)]
pub struct ResolvedSecret {
    pub name: SecretName,
    pub version: u32,
    pub value: SecretValue,
}

/// `text` as it would appear inside a JSON string: quotes, backslashes and control characters
/// escaped, and with `ascii_only` every non-ASCII character as `\uXXXX` (what serializers that
/// escape non-ASCII output do).
fn json_escaped(text: &str, ascii_only: bool) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c if ascii_only && !c.is_ascii() => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Shortest secret value the output scan looks for.
const MIN_SCANNED_SECRET_BYTES: usize = 8;

/// Outcome of a step command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepAck {
    pub command_id: String,
    pub already_applied: bool,
    pub step_status: StepStatus,
    pub run_status: RunStatus,
}

/// Result of authorizing one blob completion before catalog access.
/// Receipt lookup and a fresh report's scoped lease/scope checks share one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobCompletionTarget {
    Replayed(StepAck),
    Ready(BlobScope),
}

/// Outcome of a heartbeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatAck {
    pub cancel_requested: bool,
    /// When the lease now expires (never past the attempt's own deadline). `None` when this very
    /// lease already reported the attempt (G141): the beat was late, there is nothing to renew and
    /// the worker should stop beating.
    pub lease_expires_at: Option<OffsetDateTime>,
}

/// The lease of `task`, recorded with the report it makes (G141).
fn reported_lease(task: &TaskRef<'_>) -> ReportedLease {
    ReportedLease {
        instance: task.instance.clone(),
        token_hash: Token::hash_of(task.lease_token),
    }
}

/// The worker protocol use cases.
#[derive(Clone)]
pub struct WorkerService<S, C, E, R> {
    store: S,
    clock: C,
    engine: Engine<E>,
    entropy: R,
    settings: WorkerSettings,
    child_root_capacity_guard: bool,
    /// Opens secrets for workers; `None` when the instance has no key ring (secrets are off).
    vault: Option<crate::secrets::SecretReader>,
    database_credentials: Option<Arc<dyn crate::database::DatabaseCredentialsCodec>>,
    database_results: Option<Arc<dyn crate::database::DatabaseResultValidator>>,
    /// Rebuilds the receipt of a lake step; without it a lake step cannot be completed (feature 035, D035-142).
    lake_receipts: Option<Arc<dyn crate::database::LakeReceiptCodec>>,
    lake_probes: Option<Arc<dyn crate::database::LakeProbeCodec>>,
    /// The polls of each namespace in flight in this process, shared by every clone (design 038).
    polls_in_flight: Arc<PollsInFlight>,
    /// The gate whose recorded requirements say whether undelivered leases are withdrawn (feature 006, decision
    /// D006-004); `None`: never.
    lease_withdrawal: Option<Arc<crate::instances::ServingGate>>,
    /// The gate whose capability decision a capability use case asks just before it commits what it hands out (decision
    /// D006-100); none in a process with no registry.
    capability_gate: Option<Arc<crate::instances::ServingGate>>,
    /// `SAGAT_LEASE_WITHDRAWAL=off`: this process writes no new withdrawal (decision D006-032).
    lease_withdrawal_disabled: bool,
    /// The highest session minor a lease is judged by, at its claim (the marks of protocol 1.8, Task 006-047) and at its
    /// hand-back: this build's own ceiling (ADR-0099 decision 9). A session stored above it is read at it.
    hand_back_ceiling: u32,
}

impl<S, C, E, R> std::fmt::Debug for WorkerService<S, C, E, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerService").finish_non_exhaustive()
    }
}

/// Which stops a report fence lets through: a cancellation requested of the step, or a run that is stopping (decision
/// D006-097). A closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopAllowance {
    /// Every other report: a cancellation or a stop refuses it.
    None,
    /// The acknowledgement of a cancellation (`CANCELLED`, not retryable): it settles a cancelled lease.
    Cancellation,
    /// A host's hand-back (`TASK_NOT_STARTED`): let through only while the step is still `LEASED`, so it reaches the verdict
    /// of the withdrawal whatever the cancellation says (request (1) of decision D006-059). A step that started keeps the
    /// fence.
    HandBack,
}

impl StopAllowance {
    /// The allowance of a failure report with `raw_code` and `retryable`, as the host sent them.
    pub(crate) fn of_failure(raw_code: &str, retryable: bool) -> Self {
        if raw_code == "CANCELLED" && !retryable {
            Self::Cancellation
        } else if raw_code == TASK_NOT_STARTED {
            Self::HandBack
        } else {
            Self::None
        }
    }

    /// The allowance of a fence that lets an acknowledged cancellation through, or none.
    pub(crate) const fn cancellation(allowed: bool) -> Self {
        if allowed {
            Self::Cancellation
        } else {
            Self::None
        }
    }

    /// Whether a step in `status` may report past a cancellation or a stop. `HandBack` is granted only to a session whose
    /// minor reserves the code, judged at the build's ceiling (decision D006-100): below 1.8 the code may be a handler's, and
    /// a fleet below 1.8 keeps the outcome it had.
    pub(crate) fn allows(self, status: StepStatus) -> bool {
        match self {
            Self::None => false,
            Self::Cancellation => true,
            Self::HandBack => status == StepStatus::Leased,
        }
    }
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    pub fn new(
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
            child_root_capacity_guard: false,
            vault: None,
            database_credentials: None,
            database_results: None,
            lake_receipts: None,
            lake_probes: None,
            polls_in_flight: Arc::default(),
            lease_withdrawal: None,
            capability_gate: None,
            lease_withdrawal_disabled: false,
            hand_back_ceiling: PROTOCOL_MINOR,
        }
    }

    /// Enables the bounded ChildRoot lease prerequisite for a gated runtime/test harness.
    /// Production activation requires the separate coordinator/creator and rollout evidence.
    #[must_use]
    pub const fn with_child_root_capacity_guard(mut self) -> Self {
        self.child_root_capacity_guard = true;
        self
    }

    /// Both codecs are required before managed results can enter retention.
    #[must_use]
    pub fn with_database_content(
        mut self,
        credentials: Arc<dyn crate::database::DatabaseCredentialsCodec>,
        results: Arc<dyn crate::database::DatabaseResultValidator>,
    ) -> Self {
        self.database_credentials = Some(credentials);
        self.database_results = Some(results);
        self
    }

    /// The codec that rebuilds the receipt of an Iceberg write. Without it the completion of a lake step is refused: the server
    /// stores a receipt it built, never the bytes a runner sent.
    #[must_use]
    pub fn with_lake_receipts(
        mut self,
        receipts: Arc<dyn crate::database::LakeReceiptCodec>,
    ) -> Self {
        self.lake_receipts = Some(receipts);
        self
    }

    /// The codec that rebuilds the result of the probe of an Iceberg connection (D035-150). Without it the completion of a probe is refused:
    /// the server stores a result it built, never the bytes a runner sent.
    #[must_use]
    pub fn with_lake_probes(mut self, probes: Arc<dyn crate::database::LakeProbeCodec>) -> Self {
        self.lake_probes = Some(probes);
        self
    }

    pub fn with_secret_provider(
        mut self,
        provider: Arc<dyn crate::secret_provider::SecretProvider>,
    ) -> Self {
        self.vault = Some(
            self.vault
                .take()
                .unwrap_or_default()
                .with_provider(provider),
        );
        self
    }

    /// Lets workers resolve the secrets their steps declare, opened with `cipher`.
    #[must_use]
    pub fn with_vault(mut self, cipher: Arc<dyn SecretCipher>) -> Self {
        self.vault = Some(self.vault.take().unwrap_or_default().with_cipher(cipher));
        self
    }

    #[must_use]
    pub const fn settings(&self) -> &WorkerSettings {
        &self.settings
    }

    /// Whether the storage adapter has enabled the scoped worker protocol.
    #[must_use]
    pub fn supports_scoped_worker_protocol(&self) -> bool {
        self.store.supports_scoped_worker_protocol()
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
        let server_minor = protocol_ceiling(
            principal.role(),
            StoreProofs {
                scoped: self.store.supports_scoped_worker_protocol(),
                database: self.store.supports_database_worker_protocol(),
                delivery_acknowledgement: self.store.supports_delivery_acknowledgement(),
            },
            PROTOCOL_MINOR,
        );
        let Some((accepted_protocol_version, accepted_protocol_minor)) =
            negotiate_protocol_details(&capabilities.protocol_version, server_minor)
        else {
            return Err(AppError::NotAllowed(UNSUPPORTED_PROTOCOL));
        };
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
        if !managed_capabilities_allowed(
            principal.role(),
            &capabilities.activities,
            accepted_protocol_minor,
        ) {
            return Err(AppError::NotBound);
        }
        if !(1..=1000).contains(&capabilities.max_inflight) {
            return Err(AppError::NotAllowed(
                "max_inflight must be between 1 and 1000",
            ));
        }
        for (labels, what) in [
            (&capabilities.languages, "languages"),
            (&capabilities.pools, "pools"),
        ] {
            let label_ok = |label: &String| {
                !label.is_empty()
                    && label.len() <= MAX_LABEL_LEN
                    && label.chars().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '+')
                    })
            };
            if labels.len() > MAX_LABELS || !labels.iter().all(label_ok) {
                return Err(AppError::NotAllowed(match what {
                    "languages" => {
                        "languages: at most 16 entries of 1 to 64 letters, digits or . _ - : +"
                    }
                    _ => "pools: at most 16 entries of 1 to 64 letters, digits or . _ - : +",
                }));
            }
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
        // Fail closed: an identity serves only what a binding names (FR-009-002, G31).
        let activities = dedup(&capabilities.activities);
        let bound = self
            .store
            .bound_patterns(
                principal.tenant_id(),
                namespace.id(),
                principal.subject().as_str(),
            )
            .await?
            .iter()
            .map(|text| ActivityPattern::parse(text))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::Internal)?;
        if let Some(uncovered) = sagat_domain::binding::first_uncovered(&bound, &activities) {
            let record = AuditRecord::by(
                principal,
                AuditAction::WorkerDenied,
                ResourceType::WorkerBinding,
                namespace.id().to_string(),
                None,
                self.clock.now(),
            )
            .with_reason(format!("not bound to activity {uncovered}"));
            if let Err(error) = self.store.record_unbound(&record).await {
                tracing::error!(%error, action = %record.action(), "failed to record audit entry");
            }
            return Err(AppError::NotBound);
        }
        let token = Token::from_bytes(self.entropy.token()).ok_or(AppError::Internal)?;
        let proposed = Uuid::now_v7();
        let ttl = self.settings.session_ttl;
        let tenant_limit = self
            .store
            .effective_limit(principal.tenant_id(), None, QuotaResource::WorkerSessions)
            .await?;
        let created = self
            .store
            .create_session(&NewSession {
                id: proposed,
                tenant_id: principal.tenant_id(),
                namespace_id: namespace.id(),
                subject: principal.subject().as_str().to_owned(),
                worker_name: worker_name.to_owned(),
                token_hash: token.hash(),
                activities,
                languages: dedup(&capabilities.languages),
                pools: dedup(&capabilities.pools),
                max_inflight: capabilities.max_inflight,
                accepted_protocol_minor,
                declines_acknowledgement: capabilities.declines_acknowledgement,
                ttl,
                tenant_limit,
            })
            .await?;
        let Some(id) = created else {
            // Narrowed or removed between the check above and the store: the same answer, and the
            // same trail, as if it had been narrowed before.
            let record = AuditRecord::by(
                principal,
                AuditAction::WorkerDenied,
                ResourceType::WorkerBinding,
                namespace.id().to_string(),
                None,
                self.clock.now(),
            )
            .with_reason(
                "the registration was refused: the binding, its profile, the role or the protocol did not cover it, or the binding changed while the worker was registering",
            );
            if let Err(error) = self.store.record_unbound(&record).await {
                tracing::error!(%error, action = %record.action(), "failed to record audit entry");
            }
            return Err(AppError::NotBound);
        };
        Ok(Registered {
            worker_id: id,
            session_token: token,
            accepted_protocol_version,
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

    /// Validates that a session negotiated the task-stream protocol before response headers are
    /// returned. Claims still repeat session validation inside their transaction.
    pub async fn stream_session(
        &self,
        principal: &Principal,
        worker_id: Uuid,
        token: &str,
    ) -> Result<SessionRecord, AppError> {
        let session = self.session(principal, worker_id, token).await?;
        if session.accepted_protocol_minor < STREAM_PROTOCOL_MINOR {
            return Err(AppError::NotAllowed(UNSUPPORTED_PROTOCOL));
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
    ) -> Result<Vec<ClaimedTask>, AppError>
    where
        S: PathExecutionStore,
    {
        self.poll_bounded(
            principal,
            worker_id,
            session_token,
            max_tasks,
            MAX_POLL_BYTES,
            trace_id,
            &Unwatched,
        )
        .await
    }

    /// Claims through the canonical poll path while respecting a caller-owned aggregate byte
    /// budget. Task streams pass their remaining finite-window budget here; unary polling passes
    /// [`MAX_POLL_BYTES`]. The claim stops before its next run once `watch` says its consumer is gone
    /// (decision D006-035).
    #[allow(clippy::too_many_arguments)] // the claim's budgets, its trace and its watch travel together
    pub async fn poll_bounded(
        &self,
        principal: &Principal,
        worker_id: Uuid,
        session_token: &str,
        max_tasks: u32,
        max_bytes: usize,
        trace_id: Option<&str>,
        watch: &dyn ClaimWatch,
    ) -> Result<Vec<ClaimedTask>, AppError>
    where
        S: PathExecutionStore,
    {
        let session = self.session(principal, worker_id, session_token).await?;
        // A drained identity keeps its session (it renews it, it reports what it holds) and is given
        // nothing new: the operator waits for what is in flight to end, then stops the process.
        if session.draining {
            metrics::counter!("worker_polls_drained_total").increment(1);
            return Ok(Vec::new());
        }
        let wanted = max_tasks.clamp(1, MAX_POLL);
        let byte_budget = max_bytes.min(MAX_POLL_BYTES);
        if byte_budget == 0 {
            return Ok(Vec::new());
        }
        let mut candidates = self
            .store
            .claimable_paths(
                session.tenant_id,
                session.namespace_id,
                &session.activities,
                wanted.saturating_mul(4).min(64),
                session.accepted_protocol_minor,
                &session.subject,
                if self.child_root_capacity_guard {
                    CapacityWindow::Eligible
                } else {
                    CapacityWindow::Any
                },
            )
            .await?;
        // Polls of the namespace in flight together start at different points of the window; a poll
        // alone keeps the scan's order. Only the eligible candidates turn: the ineligible sample stays
        // behind every one of them (ADR-0075 activation condition d). The turn ends with the poll.
        let turn = self
            .polls_in_flight
            .enter(session.tenant_id, session.namespace_id);
        start_past(&mut candidates, turn.ahead, wanted as usize);
        if self.child_root_capacity_guard {
            // Behind every eligible candidate, never ahead of one (ADR-0075 activation condition d).
            let refused = self
                .store
                .claimable_paths(
                    session.tenant_id,
                    session.namespace_id,
                    &session.activities,
                    INELIGIBLE_SAMPLE,
                    session.accepted_protocol_minor,
                    &session.subject,
                    CapacityWindow::Ineligible,
                )
                .await?;
            candidates.extend(refused);
        }
        let mut claimed = Vec::new();
        self.claim(
            principal,
            &session,
            &Token::hash_of(session_token),
            candidates,
            wanted as usize,
            byte_budget,
            trace_id,
            watch,
            &mut claimed,
        )
        .await?;
        Ok(claimed)
    }

    /// Leases ready steps of the `candidates` runs into `claimed`, one transaction per run. A task is
    /// added only once its run's transaction has committed, so what is in `claimed` is leased and
    /// is returned to the worker even when a later run fails: its token exists only here, and
    /// dropping it would strand the step until its lease expires and burn an attempt.
    ///
    /// A run that cannot be leased (a corrupt journal, a constraint) is skipped, so one bad run never
    /// keeps a tenant's workers from the others. The error is returned only when nothing at all was
    /// claimed; an unavailable store ends the poll at once.
    #[allow(clippy::too_many_arguments)] // task, byte and authority budgets travel together
    async fn claim(
        &self,
        principal: &Principal,
        session: &SessionRecord,
        session_token_hash: &[u8],
        candidates: Vec<(RunId, sagat_domain::execution::InstancePath)>,
        wanted: usize,
        byte_budget: usize,
        trace_id: Option<&str>,
        watch: &dyn ClaimWatch,
        claimed: &mut Vec<ClaimedTask>,
    ) -> Result<(), AppError>
    where
        S: PathExecutionStore,
    {
        let mut bytes = 0usize;
        let mut failures = 0u8;
        let mut first_error = None;
        // Set by a claim that found, under the locks a drain and a session replacement wait for,
        // that this session can take nothing more. The rest of the window would find the same,
        // each candidate at the cost of loading its run.
        let mut progress = ClaimProgress::default();
        let fetched = candidates.len() as u64;
        let already = claimed.len();
        let mut probed = 0u64;
        for (run, path) in candidates {
            let remaining = wanted - claimed.len();
            if remaining == 0 || bytes >= byte_budget {
                break;
            }
            // nobody will receive more: what committed so far goes back to the caller and to the withdrawal
            if watch.consumer_gone() {
                break;
            }
            probed += 1;
            match self
                .claim_run(
                    principal,
                    session,
                    session_token_hash,
                    run,
                    path,
                    remaining,
                    byte_budget,
                    trace_id,
                    &mut bytes,
                    &mut progress,
                    watch,
                )
                .await
            {
                Ok(tasks) => {
                    let leased = !tasks.is_empty();
                    claimed.extend(tasks);
                    if leased {
                        watch.committed().await;
                    }
                    if progress.session_full {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, %run, "leasing the steps of a run failed");
                    let down = matches!(error, AppError::Unavailable);
                    first_error.get_or_insert(error);
                    failures += 1;
                    if down || failures >= MAX_CLAIM_FAILURES {
                        break;
                    }
                }
            }
        }
        count_claim(
            fetched,
            probed,
            progress.opened,
            (claimed.len() - already) as u64,
        );
        match first_error {
            Some(error) if claimed.is_empty() => Err(error),
            _ => Ok(()),
        }
    }

    /// Leases up to `remaining` ready steps of one run in one transaction and commits it.
    #[allow(clippy::too_many_arguments)] // one run receives the same bounded claim context
    async fn claim_root_run(
        &self,
        principal: &Principal,
        session: &SessionRecord,
        session_token_hash: &[u8],
        run: RunId,
        remaining: usize,
        byte_budget: usize,
        trace_id: Option<&str>,
        bytes: &mut usize,
        progress: &mut ClaimProgress,
        watch: &dyn ClaimWatch,
    ) -> Result<Vec<ClaimedTask>, AppError>
    where
        S: PathExecutionStore,
    {
        let mut tx = match self
            .store
            .begin_claim(session.tenant_id, run, &session.activities)
            .await
        {
            Ok(Some(tx)) => tx,
            // Locked by someone else, gone, or without a ready step by now.
            Ok(None) => return Ok(Vec::new()),
            // A run pinned to a definition this build cannot read (a rolled-back build meeting a
            // newer run) is not this poll's to serve and is not damaged. It is skipped for this
            // poll, counted and logged, and does not use up the failures the poll tolerates, so
            // runs this build can read are still served (G460).
            Err(StoreError::UnreadableDefinition) => {
                metrics::counter!(
                    "worker_claim_skipped_total",
                    "reason" => "definition_unreadable"
                )
                .increment(1);
                tracing::warn!(%run, "a run pinned to an unreadable definition was skipped by a poll");
                return Ok(Vec::new());
            }
            Err(error) => return Err(error.into()),
        };
        progress.opened += 1;
        // The identity may have been drained after the poll looked at its session: the check is
        // made again here, under the lock a drain waits for, so a drain that has returned has
        // stopped every poll that had not leased yet.
        if tx.worker_drained(&session.subject).await? {
            progress.session_full = true;
            return Ok(Vec::new());
        }
        let room = tx
            .worker_session_room(session.id, session_token_hash, &session.subject)
            .await?;
        let remaining = remaining.min(room as usize);
        if remaining == 0 {
            // The session holds all it declared (or is gone): decided under its row lock, and true
            // of every other candidate of this poll.
            progress.session_full = true;
            return Ok(Vec::new());
        }
        // Candidate selection is only a bounded hint. Recheck catalog-backed capability needs
        // under the run lock so a protocol-specific blob committed after that scan cannot be
        // leased to an older worker. An incompatible task stays READY and consumes no attempt.
        if tx.required_worker_protocol_minor().await? > session.accepted_protocol_minor {
            return Ok(Vec::new());
        }
        let mut ready: Vec<(ExecutionInstanceId, ActivityName, Option<Duration>)> = {
            let (def, state) = tx.parts();
            state
                .instances()
                .filter(|(instance, step)| {
                    step.status() == StepStatus::Ready && instance.path().is_root()
                })
                .filter_map(|(instance, _)| {
                    let n = def.node_for_instance(instance)?;
                    if n.managed_kind().is_some()
                        && (!instance.path().is_root()
                            || session.accepted_protocol_minor < DATABASE_PROTOCOL_MINOR)
                    {
                        return None;
                    }
                    match n.kind() {
                        NodeKind::Activity { activity, .. }
                            if session.activities.iter().any(|a| a == activity.as_str()) =>
                        {
                            Some((instance.clone(), activity.clone(), n.timeout()))
                        }
                        _ => None,
                    }
                })
                .collect()
        };
        // A run transaction retains every activity quota (and compensation breaker) lock until
        // commit. Node-id order can invert the same activities across workflows and deadlock.
        ready.sort_by(
            |(left_instance, left_activity, _), (right_instance, right_activity, _)| {
                left_activity
                    .as_str()
                    .cmp(right_activity.as_str())
                    .then_with(|| left_instance.cmp(right_instance))
            },
        );
        let mut leased = Vec::new();
        let mut run_bytes = 0usize;
        // Set by a write that is not a lease (a refusal's audit, a permanent failure of an input): it must not be rolled back
        // with the leases of a consumer that went away (decision D006-092).
        let mut kept_writes = false;
        for (instance, activity, timeout) in ready {
            // Every task carries the run's data; the response is bounded by what it carries.
            // `remaining` bounds what is *leased*, not what is looked at: a step the circuit
            // breaker holds back must not use up the place of one that can run.
            if *bytes + run_bytes >= byte_budget || leased.len() >= remaining {
                break;
            }
            // A secret that lists worker pools is not delivered to a worker outside them, so the
            // step is not handed to it either: it stays READY, without spending an attempt, for a
            // worker that may have the value (G48). Judged on the grants as they are in this
            // transaction, before any quota is reserved for the step.
            let declared = Self::declared_secrets_of(&mut tx, &instance);
            if !declared.is_empty()
                && let Some(withheld) = tx
                    .secret_pool_refusal(&declared, &session.subject, PoolCheck::Hint)
                    .await?
            {
                let attempt = tx
                    .parts()
                    .1
                    .instance(&instance)
                    .map_or(0, sagat_domain::execution::StepState::attempt)
                    .saturating_add(1);
                let effects = self.pool_refusal_effects(
                    principal,
                    tx.info(),
                    &instance,
                    attempt,
                    &withheld,
                    "lease",
                    trace_id,
                );
                kept_writes = true;
                tx.persist(&[], effects).await?;
                metrics::counter!("secret_delivery_refused_total", "stage" => "lease").increment(1);
                continue;
            }
            // Measure the actual serialized context before granting any lease. Raw output sizes
            // do not bound it: JSON number normalization can expand a compact stored document.
            let kind = if instance.path().is_root() {
                tx.parts()
                    .0
                    .node_for_instance(&instance)
                    .and_then(sagat_domain::workflow::Node::managed_kind)
            } else {
                None
            };
            let managed = kind.is_some();
            let input = if managed {
                b"{}".to_vec()
            } else {
                self.engine.evaluator().task_input_instance(tx.data().await?, &instance)
                    .map_err(|error| { tracing::error!(%error, instance = ?instance, "failed to materialize task input"); AppError::Internal })?
            };
            if input.len() > MAX_TASK_INPUT_BYTES {
                kept_writes = true;
                match self
                    .reject_large_input(&mut tx, session, &instance, trace_id)
                    .await
                {
                    Ok(()) | Err(AppError::StateConflict(_)) => break,
                    Err(other) => return Err(other),
                }
            }
            if input.len() > byte_budget - *bytes - run_bytes {
                // This task fits a fresh response. Leave it READY, without consuming an attempt,
                // and return every lease already made in this transaction.
                break;
            }
            // Input-limit failure above is internal progress with no external lease. Possession
            // gates admission below; any applicable G lock remains held through the batch commit.
            if self.child_root_capacity_guard && !tx.worker_capacity_available().await? {
                break;
            }
            if managed {
                let admitted = tx.database_claim(instance.node(), &session.subject).await;
                let denied = match admitted {
                    Ok(true) => false,
                    Ok(false)
                    | Err(
                        StoreError::DatabaseUnavailable
                        | StoreError::DatabaseLimit
                        | StoreError::LakeRefused { .. },
                    ) => true,
                    Err(error) => return Err(error.into()),
                };
                if denied {
                    metrics::counter!("database_claims_total", "outcome" => "refused").increment(1);
                    let actor = Principal::new(
                        session.tenant_id,
                        sagat_domain::identity::Subject::parse(&session.subject)
                            .map_err(|_| AppError::Internal)?,
                        Role::DatabaseRunner,
                    );
                    let mut effects = Effects::by(
                        Actor::Worker(session.subject.clone()),
                        trace_id.map(str::to_owned),
                    );
                    effects.audit = Some(
                        AuditRecord::by(
                            &actor,
                            AuditAction::DatabaseDenied,
                            ResourceType::Run,
                            run.to_string(),
                            trace_id.map(str::to_owned),
                            self.clock.now(),
                        )
                        .with_reason("managed database claim refused"),
                    );
                    kept_writes = true;
                    tx.persist(&[], effects).await?;
                    continue;
                }
            }
            // The attempt input of an Iceberg write is made from the run's own data, after the claim was admitted, so
            // that a runner which fails the role, pool or concurrency checks cannot cause a permanent failure. What cannot
            // be made is a permanent failure of the step, with a closed code, and no lease (D035-138).
            let (input, record_blobs) = match kind {
                Some(sagat_domain::database::ManagedKind::Lake) => {
                    match self.lake_lease_input(&mut tx, &instance).await? {
                        Ok(leased) => leased,
                        Err(failure) => {
                            kept_writes = true;
                            let code = FailureCode::parse(failure.code())
                                .map_err(|_| AppError::Internal)?;
                            match self
                                .reject_input(
                                    &mut tx,
                                    session,
                                    &instance,
                                    code,
                                    "lake_input",
                                    trace_id,
                                )
                                .await
                            {
                                Ok(()) | Err(AppError::StateConflict(_)) => break,
                                Err(other) => return Err(other),
                            }
                        }
                    }
                }
                // A probe's lease says which connection it tests, by the name and the revision it was admitted with, and nothing the run
                // holds: it reads no record and no blob (D035-150).
                Some(sagat_domain::database::ManagedKind::LakeProbe) => {
                    let facts = tx.lake_probe_facts(instance.node()).await?;
                    let document =
                        lake_probe_input::lake_probe_input(&lake_probe_input::ProbeFacts {
                            connection: &facts.connection,
                            revision: facts.revision,
                        });
                    (document, Vec::new())
                }
                Some(sagat_domain::database::ManagedKind::Sql) | None => (input, Vec::new()),
            };
            if matches!(
                kind,
                Some(
                    sagat_domain::database::ManagedKind::Lake
                        | sagat_domain::database::ManagedKind::LakeProbe
                )
            ) {
                if input.len() > MAX_TASK_INPUT_BYTES {
                    kept_writes = true;
                    match self
                        .reject_large_input(&mut tx, session, &instance, trace_id)
                        .await
                    {
                        Ok(()) | Err(AppError::StateConflict(_)) => break,
                        Err(other) => return Err(other),
                    }
                }
                if input.len() > byte_budget - *bytes - run_bytes {
                    break;
                }
            }
            // The quota decision and READY -> LEASED transition share this run transaction. A
            // reachable limit is counted under the activity's count lock, which makes concurrent
            // pollers exact; a full activity stays ready so this bounded candidate scan can still
            // serve another activity. The default limit is not counted.
            if !tx.activity_admitted(&activity).await? {
                metrics::counter!(
                    "quota_exceeded_total",
                    "limit" => "activity_concurrency"
                )
                .increment(1);
                continue;
            }
            // A provider whose compensations keep failing is left alone until its breaker allows
            // one probe; the step stays ready and loses nothing by waiting.
            if instance.node().is_compensation() && !tx.compensation_admitted(&activity).await? {
                continue;
            }
            // Retention is part of admitting the effect, not of its eventual report. A forward
            // lease also freezes any script compensation/reconciliation it may obligate. The
            // store holds the policy fence and journals the pins with the lease atomically.
            if !tx
                .script_execution_admitted_instance(&instance, &activity)
                .await?
            {
                metrics::counter!(
                    "quota_exceeded_total",
                    "limit" => QuotaResource::ScriptRetentionDays.limit_name()
                )
                .increment(1);
                continue;
            }
            match self
                .lease_one(
                    &mut tx, session, &instance, activity, timeout, input, trace_id,
                )
                .await
            {
                Ok(task) => {
                    // The blobs the document names are pinned to the attempt just granted, in the transaction that grants it: the
                    // allow-list of its `ReadBlob`, from the catalog's own word (D035-144).
                    if !record_blobs.is_empty() {
                        tx.pin_lake_blobs(instance.node(), task.fence, &record_blobs)
                            .await?;
                    }
                    run_bytes += task.input.len();
                    leased.push(task);
                }
                // The run stopped, paused or changed under us: skip it, keep the others.
                Err(AppError::StateConflict(_)) => break,
                Err(other) => return Err(other),
            }
        }
        if !leased.is_empty() && roll_back_for_a_gone_consumer(watch, kept_writes).await {
            return Ok(Vec::new());
        }
        tx.commit().await?;
        // the marks are counted once they are durable: a transaction rolled back above counts nothing (D006-092)
        count_marks(&leased);
        // A failed transaction contributes neither leases nor bytes to this response.
        *bytes += run_bytes;
        Ok(leased)
    }

    /// Records a deterministic preflight failure through the same domain attempt/failure policy
    /// as an activity report. Both journal events commit together; no live lease or token is ever
    /// published. Saga compensation and reconciliation retain their existing failure semantics.
    async fn reject_large_input<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        session: &SessionRecord,
        instance: &ExecutionInstanceId,
        trace_id: Option<&str>,
    ) -> Result<(), AppError> {
        self.reject_input(
            tx,
            session,
            instance,
            FailureCode::run_data_limit(),
            "run_data_limit",
            trace_id,
        )
        .await
    }

    /// As [`Self::reject_large_input`] for any input that cannot be made: the step fails permanently with `code`, and
    /// `reason` is the label of the metric. Nothing is leased and no token exists.
    async fn reject_input<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        session: &SessionRecord,
        instance: &ExecutionInstanceId,
        code: FailureCode,
        reason: &'static str,
        trace_id: Option<&str>,
    ) -> Result<(), AppError> {
        let effects = Effects::by(
            Actor::Worker(session.id.to_string()),
            trace_id.map(str::to_owned),
        );
        if instance.path().is_root() {
            let node = instance.node();
            self.engine
                .run(tx, effects, |def, state, now| {
                    let (fence, leased) = execution::lease_step(state, node)?;
                    let mut progress = execution::fail_step(
                        def,
                        state,
                        node,
                        execution::Failure {
                            fence,
                            code: code.clone(),
                            retryable: false,
                            jitter_sample: 0.0,
                        },
                        now,
                    )?;
                    progress.events.insert(0, leased);
                    Ok(((), progress))
                })
                .await
                .map_err(engine_error)?;
        } else {
            let instance = instance.clone();
            self.engine
                .run_addressed(tx, Vec::new(), effects, move |def, state, now| {
                    let (fence, leased) = execution::lease_instance(state, &instance)?;
                    let mut progress = execution::fail_instance(
                        def,
                        state,
                        &instance,
                        execution::Failure {
                            fence,
                            code,
                            retryable: false,
                            jitter_sample: 0.0,
                        },
                        now,
                    )?;
                    progress.events.insert(0, leased);
                    Ok(((), progress))
                })
                .await
                .map_err(engine_error)?;
        }
        metrics::counter!("worker_task_input_rejected_total", "reason" => reason).increment(1);
        Ok(())
    }

    /// The `input_json` of the lease of an Iceberg write and the blobs it names (to be pinned to the attempt the lease grants), or the
    /// closed failure that says why it cannot be made. The outer error is the store's (or a bug's); the inner one is the data's, and
    /// permanent.
    async fn lake_lease_input<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        instance: &ExecutionInstanceId,
    ) -> Result<Result<(Vec<u8>, Vec<BlobId>), sagat_domain::database::DatabaseFailure>, AppError>
    {
        let node = instance.node();
        let facts = tx.lake_lease_facts(node).await?;
        let info = tx.info().clone();
        let write = tx
            .parts()
            .0
            .node(node)
            .and_then(sagat_domain::workflow::Node::iceberg_write)
            .cloned()
            .ok_or(AppError::Internal)?;
        let workflow = tx.parts().0.name().as_str().to_owned();
        let attempts = tx
            .parts()
            .1
            .instance(instance)
            .map_or(0, sagat_domain::execution::StepState::attempt);
        // An earlier attempt has to be in the journal: without it the instant of the first one is unknown, and a document
        // that guessed it would let a retry look for what an earlier attempt wrote in the wrong window.
        if attempts > 0 && facts.first_leased_at.is_none() {
            return Err(AppError::Internal);
        }
        let first_attempt_started_at = facts.first_leased_at.unwrap_or_else(|| tx.now());
        let template = Self::task_template(tx.parts().0, instance);
        let data = tx.data().await?;
        let (_, effect_key, _) = self.task_identity(template, data, &info, instance)?;
        // What the source names is owned from here on, so that the run's data is not borrowed while the catalog is asked.
        let records = match lake_lease::named_records(self.engine.evaluator(), &write, data) {
            Ok(records) => records,
            Err(failure) => return Ok(Err(failure)),
        };
        let step_id = instance_step_id(info.version_id, info.run_id, instance).to_string();
        let run_id = info.run_id.to_string();
        let workflow_version = facts.workflow_version.to_string();
        let lake_facts = lake_input::LakeFacts {
            effect_key: &effect_key,
            first_attempt_started_at,
            run_id: &run_id,
            step_id: &step_id,
            workflow: &workflow,
            workflow_version: &workflow_version,
            prior_attempt_may_have_left_files: facts.first_leased_at.is_some(),
            allow_empty: lake_input::APPEND_ALLOWS_EMPTY,
        };
        // The catalog's word, in this transaction, on the blobs the source names (none for inline records): the document is made from
        // what it vouches for and not from the descriptors of the run's data.
        let ids = records.blob_ids();
        let found = if ids.is_empty() {
            Vec::new()
        } else {
            tx.lake_record_blobs(&ids).await?
        };
        Ok(lake_lease::lake_lease_document(
            &write,
            &lake_facts,
            records,
            &found,
            facts.connection_limits,
        )
        .map(|document| (document, ids)))
    }

    #[allow(clippy::too_many_arguments)] // the already-bounded input belongs to this lease
    async fn lease_one<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        session: &SessionRecord,
        instance: &ExecutionInstanceId,
        activity: ActivityName,
        timeout: Option<Duration>,
        input: Vec<u8>,
        trace_id: Option<&str>,
    ) -> Result<ClaimedTask, AppError> {
        let token = Token::from_bytes(self.entropy.token()).ok_or(AppError::Internal)?;
        let managed = tx
            .parts()
            .0
            .node_for_instance(instance)
            .is_some_and(|node| instance.path().is_root() && node.managed_kind().is_some());
        let mut effects = Effects::by(
            Actor::Worker(if managed {
                session.subject.clone()
            } else {
                session.id.to_string()
            }),
            trace_id.map(str::to_owned),
        );
        effects.lease = Some(LeaseGrant {
            instance: instance.clone(),
            token_hash: token.hash(),
            worker_id: session.id.to_string(),
            ttl: self.settings.lease_ttl,
            attempt_timeout: timeout,
            marking: self.lease_marking(session),
        });
        // A lake write that takes the result a SQL step retained reads it for the run: audited once with the lease, as reading it through
        // the API is, naming the node, the write, its connection and who started the run, and no value (D035-145). The lease is the one place
        // the result is read.
        let result_read = instance
            .path()
            .is_root()
            .then(|| tx.parts().0.node(instance.node()))
            .flatten()
            .and_then(sagat_domain::workflow::Node::iceberg_write)
            .and_then(|write| match write.records() {
                sagat_domain::database::RecordSource::DatabaseResult(source) => {
                    Some((source.clone(), write.connection().name.clone()))
                }
                _ => None,
            });
        if let Some((source, connection)) = result_read {
            let actor = Principal::new(
                session.tenant_id,
                sagat_domain::identity::Subject::parse(&session.subject)
                    .map_err(|_| AppError::Internal)?,
                Role::DatabaseRunner,
            );
            effects.audit = Some(
                AuditRecord::by(
                    &actor,
                    AuditAction::DatabaseResultRead,
                    ResourceType::Run,
                    tx.info().run_id.to_string(),
                    trace_id.map(str::to_owned),
                    self.clock.now(),
                )
                .with_reason(format!(
                    "node:{source} read by the lake write {} on the connection {connection}, for the run started by {}",
                    instance.node(),
                    tx.info().started_by
                )),
            );
        }
        let fence = if instance.path().is_root() {
            let node = instance.node();
            self.engine
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
                .map_err(engine_error)?
        } else {
            let instance = instance.clone();
            self.engine
                .run_addressed(tx, Vec::new(), effects, move |_, state, _| {
                    let (fence, event) = execution::lease_instance(state, &instance)?;
                    Ok((
                        fence,
                        sagat_domain::execution::AddressedProgress {
                            events: vec![event],
                        },
                    ))
                })
                .await
                .map_err(engine_error)?
        };
        let info = tx.info().clone();
        // The real expiry: a renewal never outlives the attempt deadline, so ask the row.
        let expires_at = tx
            .lease_expiry_instance(instance)
            .await?
            .unwrap_or_else(|| tx.now() + self.settings.lease_ttl);
        let template = Self::task_template(tx.parts().0, instance);
        let data = tx.data().await?;
        let input_blob = if managed { None } else { data.input_blob };
        let (kind, idempotency_key, target) =
            self.task_identity(template, data, &info, instance)?;
        let run_id = info.run_id;
        let version = info.version_id;
        let mark = tx.claim_mark(instance);
        Ok(ClaimedTask {
            run_id,
            step_id: instance_step_id(version, run_id, instance),
            instance: instance.clone(),
            fence,
            lease_token: token,
            expires_at,
            activity,
            input,
            input_blob,
            idempotency_key,
            trace_id: info.trace_id,
            kind,
            target,
            mark: mark.map(|mark| mark.outcome),
            attempt_deadline: mark.as_ref().and_then(ClaimMark::attempt_deadline),
        })
    }

    /// What the claim of a lease for `session` asks its statement to mark (protocol 1.8, Task 006-047, decision D006-106).
    /// A session is judged at this build's ceiling, so no lease is reserved before 1.8 negotiates; the engine's reasons come
    /// from the requirements as its gate last read them, its switch and the session's word.
    pub(crate) fn lease_marking(&self, session: &SessionRecord) -> LeaseMarking {
        if session.accepted_protocol_minor.min(self.hand_back_ceiling)
            < DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR
        {
            return LeaseMarking::Unreserved;
        }
        let gate = self.lease_withdrawal.as_deref();
        let requires = |capability, kinds: &[crate::instances::InstanceKind]| {
            gate.is_some_and(|gate| gate.requires(capability, kinds))
        };
        LeaseMarking::Reserved {
            skipped: MarkSkip::of_engine(
                requires(
                    crate::instances::Capability::DeliveryAcknowledgementV1,
                    &[crate::instances::InstanceKind::ExecutionEngine],
                ),
                requires(
                    crate::instances::Capability::LeaseWithdrawalV2,
                    &crate::instances::InstanceKind::ALL,
                ),
                !self.lease_withdrawal_disabled,
                session.declines_acknowledgement,
            ),
        }
    }

    fn task_template(
        def: &sagat_domain::workflow::WorkflowDefinition,
        instance: &ExecutionInstanceId,
    ) -> (
        Option<sagat_domain::workflow::NodeId>,
        TaskKind,
        Option<sagat_domain::workflow::Expression>,
        ExecutionInstanceId,
    ) {
        // The forward step a compensation or a reconciliation belongs to, and that step's template.
        let origin = def.node_for_instance(instance).map(|n| n.origin().clone());
        let forward = origin.as_ref().and_then(|o| o.forward().cloned());
        let kind = match origin {
            Some(sagat_domain::saga::NodeOrigin::Compensation(_)) => TaskKind::Compensation,
            Some(sagat_domain::saga::NodeOrigin::Reconciliation(_)) => TaskKind::Reconciliation,
            _ => TaskKind::Forward,
        };
        let templated_instance = ExecutionInstanceId::in_path(
            instance.path().clone(),
            forward.as_ref().unwrap_or(instance.node()).clone(),
        );
        let template = match def
            .node_for_instance(&templated_instance)
            .map(sagat_domain::workflow::Node::kind)
        {
            Some(NodeKind::Activity {
                idempotency_template,
                ..
            }) => idempotency_template.clone(),
            _ => None,
        };
        (forward, kind, template, templated_instance)
    }

    fn task_identity(
        &self,
        template: (
            Option<sagat_domain::workflow::NodeId>,
            TaskKind,
            Option<sagat_domain::workflow::Expression>,
            ExecutionInstanceId,
        ),
        data: &crate::execution::RunData,
        info: &crate::execution::RunInfo,
        instance: &ExecutionInstanceId,
    ) -> Result<(TaskKind, String, Option<TaskTarget>), AppError> {
        let run_id = info.run_id;
        let version = info.version_id;
        let (forward, kind, template, templated_instance) = template;
        // The key of the forward step: what identifies the effect at the provider.
        let forward_node = forward.as_ref().unwrap_or(instance.node());
        let forward_key = match template {
            Some(template) => match self.engine.evaluator().idempotency_key_instance(
                &template,
                data,
                &templated_instance,
            ) {
                Ok(key) => key,
                Err(failure) if instance.path().is_root() => {
                    tracing::warn!(error = %failure, node = %forward_node, "idempotency template failed; using the default key");
                    format!("{run_id}:{forward_node}")
                }
                Err(failure) => {
                    tracing::error!(error = %failure, instance = ?instance, "scoped idempotency context is unavailable");
                    return Err(AppError::Internal);
                }
            },
            None if instance.path().is_root() => format!("{run_id}:{forward_node}"),
            None => format!(
                "{run_id}:{}",
                instance_step_id(version, run_id, &templated_instance)
            ),
        };
        // The forward instance a compensation or a lookup is about lives at the task's own path:
        // the derived node shares the scope path of its forward node (D382).
        let (idempotency_key, target) = match (kind, forward) {
            (TaskKind::Compensation, Some(forward)) => {
                let forward = ExecutionInstanceId::in_path(instance.path().clone(), forward);
                (
                    instance_compensation_key(version, run_id, &forward),
                    Some(TaskTarget {
                        instance: forward,
                        idempotency_key: forward_key,
                    }),
                )
            }
            (TaskKind::Reconciliation, Some(forward)) => {
                let forward = ExecutionInstanceId::in_path(instance.path().clone(), forward);
                (
                    instance_reconciliation_key(version, run_id, &forward),
                    Some(TaskTarget {
                        instance: forward,
                        idempotency_key: forward_key,
                    }),
                )
            }
            _ => (forward_key, None),
        };
        Ok((kind, idempotency_key, target))
    }

    /// Renews the lease and reports whether the run asked the worker to stop.
    ///
    /// # Errors
    /// [`AppError::StaleAttempt`] when the caller no longer holds the lease.
    pub async fn heartbeat(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
    ) -> Result<HeartbeatAck, AppError>
    where
        S: PathExecutionStore,
    {
        principal.authorize(Permission::WorkerExecute)?;
        if !task.instance.path().is_root() {
            return self.heartbeat_path(principal, task).await;
        }
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if let Err(stale) = self.verify(&mut tx, task).await {
            // A beat already in flight when the same lease reported has nothing to renew.
            if matches!(stale, AppError::StaleAttempt) && self.reported_by(&mut tx, task).await? {
                self.ensure_bound(principal, &mut tx, task).await?;
                metrics::counter!("worker_heartbeats_after_report_total").increment(1);
                return Ok(HeartbeatAck {
                    cancel_requested: false,
                    lease_expires_at: None,
                });
            }
            return Err(stale);
        }
        self.ensure_bound(principal, &mut tx, task).await?;
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.renewal = Some(LeaseRenewal {
            instance: task.instance.clone(),
            ttl: self.settings.lease_ttl,
        });
        let node = task.instance.node().clone();
        let fence = task.fence;
        let cancel_requested = self
            .engine
            .run(&mut tx, effects, move |_, state, _| {
                let fence = if fence.attempt == 0 {
                    Fence {
                        attempt: state
                            .step(&node)
                            .map_or(0, sagat_domain::execution::StepState::attempt),
                        ..fence
                    }
                } else {
                    fence
                };
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
        let lease_expires_at = tx
            .lease_expiry_instance(task.instance)
            .await?
            .unwrap_or_else(|| tx.now());
        tx.commit().await?;
        Ok(HeartbeatAck {
            cancel_requested,
            lease_expires_at: Some(lease_expires_at),
        })
    }

    /// Resolves an exact receipt or fresh report scope under the same current authority.
    /// Scoped receipt misses require a live cancellation-tolerant fence before catalog access.
    ///
    /// # Errors
    /// Permission, binding and idempotency errors are identical to completion.
    pub async fn prepare_blob_completion(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        blob_id: sagat_domain::blobs::BlobId,
        expected_digest: &str,
        key: &str,
    ) -> Result<BlobCompletionTarget, AppError>
    where
        S: PathExecutionStore,
    {
        principal.authorize(Permission::WorkerExecute)?;
        if !task.instance.path().is_root() {
            let mut tx = self.open_worker_path(principal, task).await?;
            let identity = blob_report_identity(blob_id, expected_digest);
            let idempotency = completion_idempotency(task, key, &identity, true);
            if let Some(stored) = tx.replay(&idempotency).await? {
                return Ok(BlobCompletionTarget::Replayed(parse_ack(
                    &stored,
                    true,
                    task.run_id,
                )));
            }
            // A receipt is replayable after expiry; a new report must not reach the catalog
            // without its exact live lease, even when cancellation is draining the scope.
            Self::live_path_fence(&mut tx, task, StopAllowance::Cancellation).await?;
            let scope = Self::scope_from_info(&tx.snapshot().info, task);
            tx.commit().await?;
            return Ok(BlobCompletionTarget::Ready(scope));
        }
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if task
            .step_id
            .is_some_and(|claimed| claimed != Self::scope_of(&tx, task).step_id)
        {
            return Err(AppError::StaleAttempt);
        }
        if !Self::managed_database(&mut tx, task) {
            self.ensure_bound(principal, &mut tx, task).await?;
        }
        let identity = blob_report_identity(blob_id, expected_digest);
        let idempotency = completion_idempotency(task, key, &identity, true);
        if let Some(ack) = self
            .replayed(principal, &mut tx, task, &idempotency)
            .await?
        {
            return Ok(BlobCompletionTarget::Replayed(ack));
        }
        self.ensure_bound(principal, &mut tx, task).await?;
        Ok(BlobCompletionTarget::Ready(Self::scope_of(&tx, task)))
    }

    /// Replays a blob report's exact request identity, or authorizes a fresh preparation.
    /// Prefer `prepare_blob_completion` when proceeding to catalog access, so a duplicate cannot
    /// commit between separate receipt and report-scope lookups.
    ///
    /// # Errors
    /// Permission, binding, live-fence and idempotency errors match preparation.
    pub async fn replay_blob_completion(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        blob_id: sagat_domain::blobs::BlobId,
        expected_digest: &str,
        key: &str,
    ) -> Result<Option<StepAck>, AppError>
    where
        S: PathExecutionStore,
    {
        match self
            .prepare_blob_completion(principal, task, blob_id, expected_digest, key)
            .await?
        {
            BlobCompletionTarget::Replayed(ack) => Ok(Some(ack)),
            BlobCompletionTarget::Ready(_) => Ok(None),
        }
    }

    /// Whether `task` is an attempt that may still act: its lease is current and unexpired and the
    /// identity is bound to the work. Changes nothing. The data-plane calls that are not a report
    /// (reading and writing blobs) are authorized by this, exactly as secrets are.
    ///
    /// # Errors
    /// [`AppError::StaleAttempt`] when the lease is not current, [`AppError::NotBound`], or the
    /// permission and tenant errors of any worker call.
    pub async fn check_lease(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
    ) -> Result<BlobScope, AppError>
    where
        S: PathExecutionStore,
    {
        self.check_blob_access(principal, task, false, None).await
    }

    /// Whether `task` may read `blob` right now, before a byte of the store is touched. For an ordinary step this is
    /// [`Self::check_lease`]. A managed step reads no blob through it, except a lake attempt, and only a blob the lease of exactly
    /// this attempt named (feature 035, D035-144).
    pub async fn check_blob_read(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        blob: BlobId,
        trace_id: Option<&str>,
    ) -> Result<BlobScope, AppError>
    where
        S: PathExecutionStore,
    {
        self.check_blob_access(
            principal,
            task,
            false,
            Some(BlobUse {
                blob,
                trace_id,
                audit: false,
            }),
        )
        .await
    }

    /// Uploads cannot bypass the inline-output secret guard. Until streaming content scanning is
    /// available, attempts declaring secrets are refused before any bytes reach object storage.
    pub async fn check_blob_upload(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
    ) -> Result<BlobScope, AppError>
    where
        S: PathExecutionStore,
    {
        self.check_blob_access(principal, task, true, None).await
    }

    /// Audits access before any blob bytes are handed to a worker; repeated reads of the same
    /// blob by the same attempt collapse into one append-only record.
    pub async fn audit_blob_read(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        blob: BlobId,
        trace_id: Option<&str>,
    ) -> Result<(), AppError>
    where
        S: PathExecutionStore,
    {
        self.check_blob_access(
            principal,
            task,
            false,
            Some(BlobUse {
                blob,
                trace_id,
                audit: true,
            }),
        )
        .await
        .map(|_| ())
    }

    async fn check_blob_access(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        writing: bool,
        read: Option<BlobUse<'_>>,
    ) -> Result<BlobScope, AppError>
    where
        S: PathExecutionStore,
    {
        principal.authorize(Permission::WorkerExecute)?;
        let audit_read = read
            .filter(|read| read.audit)
            .map(|read| (read.blob, read.trace_id));
        if !task.instance.path().is_root() {
            return self
                .check_path_blob_access(principal, task, writing, audit_read)
                .await;
        }
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        self.verify(&mut tx, task).await?;
        self.ensure_bound(principal, &mut tx, task).await?;
        if !tx
            .lease_unexpired_instance(task.instance, &Token::hash_of(task.lease_token))
            .await?
        {
            return Err(AppError::StaleAttempt);
        }
        {
            let (_, state) = tx.parts();
            let step = state
                .instance(task.instance)
                .ok_or(AppError::StaleAttempt)?;
            if step.attempt() != task.fence.attempt || step.generation() != task.fence.generation {
                return Err(AppError::StaleAttempt);
            }
        }
        if let Some(kind) = Self::managed_kind(&mut tx, task) {
            // One door is open to a managed step: a lake attempt reads a blob the lease of exactly this attempt named, and nothing
            // else (D035-144). The check that names no blob, the upload and every SQL step stay refused.
            match (kind, writing, read) {
                (sagat_domain::database::ManagedKind::Lake, false, Some(read)) => {
                    if !self.lake_blob_allowed(&mut tx, task, read.blob).await? {
                        // A refusal is the answer whether or not it could be written down; the failure is logged, as for a denied
                        // registration.
                        let effects = self.lake_read_denial(principal, task, read.trace_id);
                        if let Err(error) = tx.persist(&[], effects).await {
                            tracing::error!(%error, "failed to record a refused lake blob read");
                        } else if let Err(error) = tx.commit().await {
                            tracing::error!(%error, "failed to record a refused lake blob read");
                        }
                        return Err(AppError::NotFound);
                    }
                }
                _ => {
                    return Err(AppError::NotAllowed(if writing {
                        "managed database results require content scanning"
                    } else {
                        "managed database inputs are resolved through the capability"
                    }));
                }
            }
        }
        if writing && !Self::declared_secrets(&mut tx, task).is_empty() {
            return Err(AppError::NotAllowed(
                "blob outputs with declared secrets require content scanning",
            ));
        }
        let scope = Self::scope_of(&tx, task);
        if let Some((blob, trace_id)) = audit_read {
            // the audit of a read commits only while the process may hand data out (decision D006-100)
            self.capability_window_holds()?;
            let effects = self.blob_read_effects(principal, task, scope, blob, trace_id);
            tx.persist(&[], effects).await?;
            tx.commit().await?;
        }
        Ok(scope)
    }

    /// Whether the lease of a lake attempt allows it to read `blob`. The lease, the binding, the owner of the session and the exact
    /// attempt and generation were checked by the caller; a read is held, as a report is (`attempt_authority_fence`), to a run that
    /// is still live: one that was told to stop, whose step has a cancel requested, or that finished is given nothing more. Then the
    /// lease of exactly this attempt and generation must have named the blob. `false` is answered as `NotFound`, the same as for a
    /// blob that does not exist, so the answer says nothing about what does (see [`Self::lake_read_denial`]).
    async fn lake_blob_allowed<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        task: &TaskRef<'_>,
        blob: BlobId,
    ) -> Result<bool, AppError> {
        let live = {
            let (_, state) = tx.parts();
            state
                .instance(task.instance)
                .is_some_and(|step| !step.cancel_requested())
                && state.stop_reason().is_none()
                && !state.is_finished()
        };
        if !live {
            return Err(AppError::StaleAttempt);
        }
        Ok(tx
            .lake_blob_pinned(task.instance.node(), task.fence, blob)
            .await?)
    }

    /// The refusal of a lake read, audited once for the run without naming the blob: an identical event is recorded once, so a runner
    /// that tries one blob after another cannot fill the trail.
    fn lake_read_denial(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        trace_id: Option<&str>,
    ) -> Effects {
        let mut effects = Effects::by(
            Actor::Worker(principal.subject().as_str().to_owned()),
            trace_id.map(str::to_owned),
        );
        effects.audit = Some(
            AuditRecord::by(
                principal,
                AuditAction::DatabaseDenied,
                ResourceType::Run,
                task.run_id.to_string(),
                trace_id.map(str::to_owned),
                self.clock.now(),
            )
            .with_reason("a lake attempt asked for a blob its lease did not name"),
        );
        effects
    }

    /// Resolves the report target. Scoped new reports require their exact live lease before
    /// catalog access; exact receipt replay is handled separately before this lookup.
    pub async fn blob_report_scope(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
    ) -> Result<BlobScope, AppError>
    where
        S: PathExecutionStore,
    {
        principal.authorize(Permission::WorkerExecute)?;
        if !task.instance.path().is_root() {
            let mut tx = self.open_worker_path(principal, task).await?;
            Self::live_path_fence(&mut tx, task, StopAllowance::Cancellation).await?;
            let scope = Self::scope_from_info(&tx.snapshot().info, task);
            tx.commit().await?;
            return Ok(scope);
        }
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        self.ensure_bound(principal, &mut tx, task).await?;
        let scope = Self::scope_of(&tx, task);
        if task.step_id.is_some_and(|claimed| claimed != scope.step_id) {
            return Err(AppError::StaleAttempt);
        }
        Ok(scope)
    }

    fn scope_of<Tx: RunTx>(tx: &Tx, task: &TaskRef<'_>) -> BlobScope {
        Self::scope_from_info(tx.info(), task)
    }

    fn scope_from_info(info: &crate::execution::RunInfo, task: &TaskRef<'_>) -> BlobScope {
        BlobScope {
            access: BlobAccess {
                tenant_id: info.tenant_id,
                namespace_id: info.namespace_id,
            },
            run_id: info.run_id,
            step_id: instance_step_id(info.version_id, info.run_id, task.instance),
            attempt: task.fence.attempt,
            generation: task.fence.generation,
        }
    }

    fn blob_read_effects(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        scope: BlobScope,
        blob: BlobId,
        trace_id: Option<&str>,
    ) -> Effects {
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.audit = Some(
            AuditRecord::by(
                principal,
                AuditAction::BlobRead,
                ResourceType::Blob,
                format!("{}/{}", scope.access.namespace_id, blob),
                trace_id.map(str::to_owned),
                self.clock.now(),
            )
            .with_reason(format!(
                "run {} node {} attempt {} generation {}{}",
                task.run_id,
                task.instance.node(),
                task.fence.attempt,
                task.fence.generation,
                if task.instance.path().is_root() {
                    String::new()
                } else {
                    format!(" step {}", scope.step_id)
                }
            )),
        );
        effects
    }

    async fn apply_scoped_or_root_failure(
        &self,
        tx: &mut S::Tx,
        task: &TaskRef<'_>,
        mut effects: Effects,
        failure: execution::Failure,
    ) -> Result<(), AppError> {
        effects.reported = Some(reported_lease(task));
        if task.instance.path().is_root() {
            let node = task.instance.node().clone();
            self.engine
                .run(tx, effects, move |def, state, now| {
                    Ok(((), execution::fail_step(def, state, &node, failure, now)?))
                })
                .await
                .map_err(engine_error)
        } else {
            let instance = task.instance.clone();
            self.engine
                .run_addressed(tx, Vec::new(), effects, move |def, state, now| {
                    Ok((
                        (),
                        execution::fail_instance(def, state, &instance, failure, now)?,
                    ))
                })
                .await
                .map_err(engine_error)
        }
    }

    async fn apply_scoped_or_root_completion(
        &self,
        tx: &mut S::Tx,
        task: &TaskRef<'_>,
        mut effects: Effects,
        output: Option<Vec<u8>>,
    ) -> Result<(), AppError> {
        effects.reported = Some(reported_lease(task));
        if task.instance.path().is_root() {
            if let Some(output) = output {
                effects.outputs = vec![(task.instance.node().clone(), output)];
            }
            let node = task.instance.node().clone();
            let fence = task.fence;
            self.engine
                .run(tx, effects, move |def, state, now| {
                    Ok(((), execution::complete_step(def, state, &node, fence, now)?))
                })
                .await
                .map_err(engine_error)
        } else {
            let outputs = output
                .map(|output| vec![(task.instance.clone(), output)])
                .unwrap_or_default();
            let instance = task.instance.clone();
            let fence = task.fence;
            self.engine
                .run_addressed(tx, outputs, effects, move |def, state, _| {
                    Ok((
                        (),
                        execution::complete_instance(def, state, &instance, fence)?,
                    ))
                })
                .await
                .map_err(engine_error)
        }
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
    ) -> Result<StepAck, AppError>
    where
        S: PathExecutionStore,
    {
        principal.authorize(Permission::WorkerExecute)?;
        let mut code =
            FailureCode::parse(error_code).unwrap_or_else(|_| FailureCode::worker_error());
        let idempotency = Idempotency {
            scope: report_scope(task),
            key: idempotency_key.to_owned(),
            fingerprint: fingerprint(&[
                b"fail",
                &task.fence.attempt.to_be_bytes(),
                &task.fence.generation.to_be_bytes(),
                code.as_str().as_bytes(),
                &[u8::from(retryable)],
            ]),
        };
        if !task.instance.path().is_root() {
            return self
                .report_path(
                    principal,
                    task,
                    scoped_reports::PathReport::Failure {
                        raw_code: error_code,
                        code,
                        retryable,
                    },
                    idempotency,
                )
                .await;
        }
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if let Some(ack) = self
            .replayed(principal, &mut tx, task, &idempotency)
            .await?
        {
            return Ok(ack);
        }
        self.verify(&mut tx, task).await?;
        self.ensure_bound(principal, &mut tx, task).await?;
        // Only closed managed failures may enter the payload-free journal.
        // safe_message is discarded by the transport; an arbitrary error_code is content too.
        if Self::managed_database(&mut tx, task)
            && sagat_domain::database::DatabaseFailure::parse(error_code).is_none()
        {
            return Err(AppError::NotAllowed(
                "managed database failures require a fixed code",
            ));
        }
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.reported = Some(reported_lease(task));
        // The code is stored in the journal and shown to everyone who reads the run, and a
        // credential fits the characters and the length it may have (`[A-Za-z0-9_.:-]`, 64): a
        // worker that reports its own secret as the reason it failed fails for good instead.
        let mut retryable = retryable;
        // a hand-back passes a cancellation only from a session whose minor reserves the code (decision D006-100)
        let stop = match StopAllowance::of_failure(error_code, retryable) {
            StopAllowance::HandBack if !self.root_reserves_the_code(&mut tx, task).await? => {
                StopAllowance::None
            }
            stop => stop,
        };
        let leaked = match self
            .checked_secret_report(
                principal,
                task,
                tx,
                &idempotency,
                error_code.as_bytes(),
                stop,
            )
            .await?
        {
            SecretReport::Ready(current, leaked) => {
                tx = current;
                leaked
            }
            SecretReport::Replayed(ack) => return Ok(ack),
        };
        if let Some(name) = leaked {
            metrics::counter!("secret_output_detected_total").increment(1);
            effects.audit =
                Some(self.leak_audit(principal, tx.info(), task, &name, "failure code"));
            code = FailureCode::secret_in_output();
            retryable = false;
        }
        // A host hands back a lease it never ran (decisions D006-005 and D006-030): with the requirement recorded, the step
        // still `LEASED` to this very lease, a session of a minor that reserves the code and the attempt below the cap, it is
        // withdrawn and the attempt given back; otherwise it is the ordinary failure below, with the flag the host sent
        // (decision D006-014). Never an unknown outcome. A report that leaked a secret carries `SECRET_IN_OUTPUT` by now, so it
        // never reaches the withdrawal.
        if code.as_str() == TASK_NOT_STARTED {
            match self.root_hand_back(&mut tx, task).await? {
                HandBackVerdict::Withdraw(before) => {
                    // the token handed back is dead for every call, heartbeats included (decision D006-086): it is not
                    // recorded as the lease that reported the step
                    effects.reported = None;
                    let node = task.instance.node().clone();
                    let fence = task.fence;
                    self.engine
                        .run(&mut tx, effects, move |def, state, now| {
                            Ok((
                                (),
                                execution::withdraw_lease(
                                    def,
                                    state,
                                    &node,
                                    fence,
                                    execution::WithdrawalCause::NotStarted,
                                    before,
                                    now,
                                )?,
                            ))
                        })
                        .await
                        .map_err(engine_error)?;
                    let ack = self.finish(tx, task, idempotency).await?;
                    count_undelivered(
                        execution::WithdrawalCause::NotStarted,
                        WithdrawalOutcome::Withdrawn,
                        1,
                    );
                    return Ok(ack);
                }
                // nothing recorded: the lease lapses as before (decision D006-089)
                HandBackVerdict::Lapse(outcome) => {
                    count_undelivered(execution::WithdrawalCause::NotStarted, outcome, 1);
                    return Err(AppError::StaleAttempt);
                }
                HandBackVerdict::Fail(outcome) => {
                    count_undelivered(execution::WithdrawalCause::NotStarted, outcome, 1)
                }
            }
        }
        let jitter = self.entropy.unit();
        let fallback_code = code.clone();
        let fallback_audit = effects.audit.clone();
        let completed = self
            .apply_scoped_or_root_failure(
                &mut tx,
                task,
                effects,
                execution::Failure {
                    fence: task.fence,
                    code,
                    retryable,
                    jitter_sample: jitter,
                },
            )
            .await;
        let result = match completed {
            Ok(()) => self.finish(tx, task, idempotency.clone()).await,
            Err(error) => {
                drop(tx);
                Err(error)
            }
        };
        self.recover_failed_storage_quota(
            principal,
            task,
            idempotency,
            fallback_code,
            fallback_audit,
            result,
        )
        .await
    }

    async fn recover_failed_storage_quota(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        idempotency: Idempotency,
        code: FailureCode,
        audit: Option<AuditRecord>,
        result: Result<StepAck, AppError>,
    ) -> Result<StepAck, AppError> {
        match result {
            Err(AppError::QuotaExceeded(what))
                if what == QuotaResource::TenantContentBytes.limit_name() =>
            {
                metrics::counter!("quota_exceeded_total", "limit" => what).increment(1);
                self.fail_report_for_storage_quota(principal, task, idempotency, code, audit)
                    .await
            }
            other => other,
        }
    }

    async fn fail_report_for_storage_quota(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        idempotency: Idempotency,
        code: FailureCode,
        audit: Option<AuditRecord>,
    ) -> Result<StepAck, AppError> {
        let mut tx = self.open(principal, task, Lock::Wait).await?;
        if let Some(ack) = self
            .replayed(principal, &mut tx, task, &idempotency)
            .await?
        {
            return Ok(ack);
        }
        self.verify(&mut tx, task).await?;
        self.ensure_bound(principal, &mut tx, task).await?;
        let mut effects = Effects::by(Actor::Worker(principal.subject().as_str().to_owned()), None);
        effects.reported = Some(reported_lease(task));
        effects.audit = audit;
        if task.instance.path().is_root() {
            let node = task.instance.node().clone();
            let fence = task.fence;
            self.engine
                .run(&mut tx, effects, move |def, state, now| {
                    let progress = execution::fail_report_for_storage_quota(
                        def, state, &node, fence, code, now,
                    )?;
                    Ok(((), progress))
                })
                .await
                .map_err(engine_error)?;
        } else {
            self.apply_scoped_or_root_failure(
                &mut tx,
                task,
                effects,
                execution::Failure {
                    fence: task.fence,
                    code: FailureCode::storage_quota_exceeded(),
                    retryable: false,
                    jitter_sample: 0.0,
                },
            )
            .await?;
        }
        self.finish(tx, task, idempotency).await
    }

    /// The audit record of a step that reported the value of one of its secrets in `what`.
    fn leak_audit(
        &self,
        principal: &Principal,
        info: &crate::execution::RunInfo,
        task: &TaskRef<'_>,
        name: &SecretName,
        what: &str,
    ) -> AuditRecord {
        AuditRecord::by(
            principal,
            AuditAction::SecretDenied,
            ResourceType::Secret,
            format!("{}/{name}", info.namespace_id),
            None,
            self.clock.now(),
        )
        .with_reason(format!(
            "{what} of {} on run {} contained the value of a secret",
            task.instance.node(),
            task.run_id
        ))
    }

    async fn open(
        &self,
        principal: &Principal,
        task: &TaskRef<'_>,
        lock: Lock,
    ) -> Result<S::Tx, AppError> {
        require_supported_task(task.instance, self.store.supports_scoped_worker_protocol())?;
        if let Some(tx) = self
            .store
            .begin(principal.tenant_id(), task.run_id, lock)
            .await?
        {
            return Ok(tx);
        }
        Err(self.missing_task_error(principal, task).await)
    }

    async fn missing_task_error(&self, principal: &Principal, task: &TaskRef<'_>) -> AppError {
        // A run of another tenant is indistinguishable from a missing one: a stale attempt. It is
        // audited all the same, like every other path that reaches for a foreign run.
        if let Ok(true) = self
            .store
            .run_exists_outside(principal.tenant_id(), task.run_id)
            .await
        {
            let record = AuditRecord::by(
                principal,
                AuditAction::CrossTenantAccessDenied,
                ResourceType::Run,
                task.run_id.to_string(),
                None,
                self.clock.now(),
            )
            .with_reason("worker call on a resource not accessible from this tenant");
            if let Err(error) = RunStore::record_denial(&self.store, &record).await {
                tracing::error!(%error, action = %record.action(), "failed to record audit entry");
            }
        }
        AppError::StaleAttempt
    }

    /// The answer stored for a repeated report, if there is one.
    ///
    /// Records are scoped by the lease token ([`report_scope`]), so a caller that does not hold the
    /// token never reaches another holder's records: whether a key exists is not an oracle for a
    /// worker that merely knows a run id, a node and a guessable key.
    async fn replayed(
        &self,
        principal: &Principal,
        tx: &mut S::Tx,
        task: &TaskRef<'_>,
        idempotency: &Idempotency,
    ) -> Result<Option<StepAck>, AppError> {
        if task.step_id.is_some_and(|claimed| {
            claimed != instance_step_id(tx.info().version_id, task.run_id, task.instance)
        }) {
            return Err(AppError::StaleAttempt);
        }
        if !Self::managed_database(tx, task) {
            self.ensure_bound(principal, tx, task).await?;
        }
        let Some(stored) = tx.replay(idempotency).await? else {
            return Ok(None);
        };
        if Self::managed_database(tx, task) {
            principal.authorize(Permission::DatabaseExecute)?;
            self.ensure_activity_bound(principal, tx, task).await?;
            if !tx
                .database_report_owns(
                    task.instance.node(),
                    task.fence,
                    principal.subject().as_str(),
                )
                .await?
            {
                return Err(AppError::NotBound);
            }
        }
        Ok(Some(parse_ack(&stored, true, task.run_id)))
    }

    fn managed_kind<Tx: RunTx>(
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Option<sagat_domain::database::ManagedKind> {
        if !task.instance.path().is_root() {
            return None;
        }
        tx.parts()
            .0
            .node_for_instance(task.instance)
            .and_then(sagat_domain::workflow::Node::managed_kind)
    }

    fn managed_database<Tx: RunTx>(tx: &mut Tx, task: &TaskRef<'_>) -> bool {
        Self::managed_kind(tx, task).is_some()
    }

    /// Whether the step is a managed write: a SQL statement that executes, or an Iceberg write (every lake step writes). What a write
    /// reports as done cannot be treated as undone when its report cannot be kept: its outcome is unknown, not failed.
    fn database_write<Tx: RunTx>(tx: &mut Tx, task: &TaskRef<'_>) -> bool {
        if !task.instance.path().is_root() {
            return false;
        }
        let node = tx.parts().0.node(task.instance.node());
        node.is_some_and(|node| {
            node.managed_kind() == Some(sagat_domain::database::ManagedKind::Lake)
                || node
                    .database_operation()
                    .is_some_and(|op| op.kind == sagat_domain::database::OperationKind::Execute)
        })
    }

    /// The secrets the step declares, from its definition.
    fn declared_secrets<Tx: RunTx>(tx: &mut Tx, task: &TaskRef<'_>) -> Vec<SecretName> {
        Self::declared_secrets_of(tx, task.instance)
    }

    fn declared_secrets_of<Tx: RunTx>(
        tx: &mut Tx,
        instance: &ExecutionInstanceId,
    ) -> Vec<SecretName> {
        let (def, _) = tx.parts();
        def.node_for_instance(instance)
            .map(|node| node.secrets().to_vec())
            .unwrap_or_default()
    }

    // Keep actor, execution identity and refusal stage explicit in this security audit helper.
    #[allow(clippy::too_many_arguments)]
    fn pool_refusal_effects(
        &self,
        principal: &Principal,
        info: &crate::execution::RunInfo,
        instance: &ExecutionInstanceId,
        attempt: u32,
        withheld: &SecretName,
        stage: &str,
        trace_id: Option<&str>,
    ) -> Effects {
        let mut effects = Effects::by(
            Actor::Worker(principal.subject().as_str().to_owned()),
            trace_id.map(str::to_owned),
        );
        effects.audit = Some(
            AuditRecord::by(
                principal,
                AuditAction::SecretDeliveryRefused,
                ResourceType::Secret,
                format!("{}/run {}", info.namespace_id, info.run_id),
                trace_id.map(str::to_owned),
                self.clock.now(),
            )
            .with_reason({
                let node = instance.node();
                let reason = format!("node {node} attempt {attempt}: {withheld} is not delivered outside its worker pools (at {stage})");
                if instance.path().is_root() {
                    reason
                } else {
                    format!("step {}: {reason}", instance_step_id(info.version_id, info.run_id, instance))
                }
            }),
        );
        effects
    }
    /// The caller must still be bound to the activity of the step it reports on. Evaluated at use,
    /// not only at registration: a binding that is removed stops a worker that already holds a
    /// lease from renewing it or reporting on it, with no session to wait for to expire.
    async fn ensure_bound<Tx: RunTx>(
        &self,
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<(), AppError> {
        let managed = Self::managed_database(tx, task);
        if managed
            && !tx
                .database_executor_owns(task.instance.node(), principal.subject().as_str())
                .await?
        {
            return Err(AppError::NotBound);
        }
        if managed {
            // a database runner is no member that executes work: its authority is the executor's ownership above
            return self.ensure_activity_bound(principal, tx, task).await;
        }
        let activity = {
            let (def, _) = tx.parts();
            match def
                .node_for_instance(task.instance)
                .map(sagat_domain::workflow::Node::kind)
            {
                Some(NodeKind::Activity { activity, .. }) => Some(activity.to_string()),
                _ => None,
            }
        };
        binding_checks::refusal(
            binding_checks::judge_binding(
                &mut binding_checks::RunBinding(tx),
                principal.subject().as_str(),
                activity.as_slice(),
            )
            .await,
        )
    }

    async fn ensure_activity_bound<Tx: RunTx>(
        &self,
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<(), AppError> {
        let activity = {
            let (def, _) = tx.parts();
            match def
                .node_for_instance(task.instance)
                .map(sagat_domain::workflow::Node::kind)
            {
                Some(NodeKind::Activity { activity, .. }) => activity.to_string(),
                _ => return Ok(()),
            }
        };
        let bound = tx
            .bound_patterns(principal.subject().as_str())
            .await?
            .iter()
            .map(|text| ActivityPattern::parse(text))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::Internal)?;
        if sagat_domain::binding::first_uncovered(&bound, &[activity]).is_some() {
            return Err(AppError::NotBound);
        }
        Ok(())
    }

    /// Whether `task` names the lease that reported the attempt of its step (G141), with the same
    /// step-id check as [`Self::verify`].
    async fn reported_by<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<bool, AppError> {
        if let Some(claimed) = task.step_id {
            let info = tx.info();
            if claimed != instance_step_id(info.version_id, info.run_id, task.instance) {
                return Ok(false);
            }
        }
        Ok(tx
            .reported_by(task.instance, &Token::hash_of(task.lease_token))
            .await?)
    }

    /// The presented token must be the one hashed into the step's current lease.
    async fn live_capability_fence<Tx: RunTx>(
        &self,
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<(), AppError> {
        self.attempt_authority_fence(principal, tx, task, StopAllowance::None)
            .await
    }

    /// Only an explicit cancellation acknowledgement, or a host's hand-back of a lease still `LEASED` (decision D006-097),
    /// may report past a cancellation or a stop. Other reports retain the live-capability fence, including a cancellation
    /// during provider I/O.
    async fn report_authority_fence<Tx: RunTx>(
        &self,
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
        stop: StopAllowance,
    ) -> Result<(), AppError> {
        self.attempt_authority_fence(principal, tx, task, stop)
            .await
    }

    async fn attempt_authority_fence<Tx: RunTx>(
        &self,
        principal: &Principal,
        tx: &mut Tx,
        task: &TaskRef<'_>,
        stop: StopAllowance,
    ) -> Result<(), AppError> {
        self.verify(tx, task).await?;
        // Binding authority precedes session/catalog/membership locks, matching revocation.
        self.ensure_bound(principal, tx, task).await?;
        let legacy_attempt = task.fence.attempt == 0 && !Self::managed_database(tx, task);
        let state = tx.parts().1;
        let step = state.instance(task.instance);
        let stop_allowed = step.is_some_and(|s| stop.allows(s.status()));
        // The stop rule is one domain function, which the step-log ingest's T2 applies too (OD-2026-11-16-11); a
        // report the stop allowance admits is the answer to a cancellation.
        let valid_step = step.is_some_and(|s| {
            (legacy_attempt || s.attempt() == task.fence.attempt)
                && s.generation() == task.fence.generation
                && sagat_domain::execution::report_fence_allows(
                    sagat_domain::execution::ReportFence {
                        cancel_requested: s.cancel_requested(),
                        run_stopping: state.stop_reason().is_some(),
                        run_finished: state.is_finished(),
                        derived_node: task.instance.node().is_derived(),
                    },
                    stop_allowed,
                )
        });
        if !valid_step
            || !tx
                .lease_unexpired_instance(task.instance, &Token::hash_of(task.lease_token))
                .await?
        {
            return Err(AppError::StaleAttempt);
        }
        Ok(())
    }

    async fn verify<Tx: RunTx>(&self, tx: &mut Tx, task: &TaskRef<'_>) -> Result<(), AppError> {
        // A step id that does not belong to this run and node is a misdirected report.
        if let Some(claimed) = task.step_id {
            let info = tx.info();
            if claimed != instance_step_id(info.version_id, info.run_id, task.instance) {
                return Err(AppError::StaleAttempt);
            }
        }
        if tx
            .lease_matches_instance(task.instance, &Token::hash_of(task.lease_token))
            .await?
        {
            Ok(())
        } else {
            Err(AppError::StaleAttempt)
        }
    }

    /// Refuses a capability use case whose process may no longer hand anything out (decision D006-100): asked inside its
    /// transaction just before the commit of its audit, of a credential's pin or of a read's audit, so a refusal rolls them
    /// back and nothing is recorded as handed out that was not.
    ///
    /// # Errors
    /// [`AppError::RegistrySilent`] while the capability window of the gate has lapsed.
    pub(super) fn capability_window_holds(&self) -> Result<(), AppError> {
        match &self.capability_gate {
            Some(gate) if gate.capability_window_lapsed(std::time::Instant::now()) => {
                Err(AppError::RegistrySilent)
            }
            _ => Ok(()),
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
                .instance(task.instance)
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

/// A read of one blob that a blob check is made for: the blob, the trace of the call, and whether the read is to be audited now.
#[derive(Clone, Copy)]
struct BlobUse<'a> {
    blob: BlobId,
    trace_id: Option<&'a str>,
    audit: bool,
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
            | sagat_domain::execution::ExecutionError::StaleInstanceAttempt { .. }
            | sagat_domain::execution::ExecutionError::IllegalTransition { .. }
            | sagat_domain::execution::ExecutionError::IllegalInstanceTransition { .. }
            | sagat_domain::execution::ExecutionError::RunFinished(_),
        ) => AppError::StaleAttempt,
        EngineError::Execution(other) => execution_error(&other),
    }
}

/// The idempotency scope of the reports of one attempt: the step plus a prefix of the hash of the
/// lease token the reporter presented. Another token, another scope, so a stale or foreign caller
/// finds nothing to replay (and is refused as stale) instead of learning that a key exists.
fn report_scope(task: &TaskRef<'_>) -> String {
    use std::fmt::Write as _;
    let hash = Token::hash_of(task.lease_token);
    let mut scope = if task.instance.path().is_root() {
        format!("step:{}:{}:", task.run_id, task.instance.node())
    } else {
        let mut scoped = format!("step:{}:scoped:", task.run_id);
        for segment in task.instance.path().segments() {
            let name = segment.scope_node().as_str();
            let _ = write!(
                scoped,
                "{}:{name}:{};",
                name.len(),
                segment.iteration_index()
            );
        }
        let node = task.instance.node().as_str();
        let _ = write!(scoped, "{}:{node}:", node.len());
        scoped
    };
    for byte in hash.iter().take(8) {
        let _ = write!(scope, "{byte:02x}");
    }
    scope
}

fn blob_report_identity(id: sagat_domain::blobs::BlobId, expected_digest: &str) -> Vec<u8> {
    fingerprint(&[
        id.to_string().as_bytes(),
        expected_digest.to_ascii_lowercase().as_bytes(),
    ])
}

fn completion_idempotency(
    task: &TaskRef<'_>,
    key: &str,
    payload: &[u8],
    blob: bool,
) -> Idempotency {
    Idempotency {
        scope: report_scope(task),
        key: key.to_owned(),
        fingerprint: fingerprint(&[
            if blob { b"complete_blob" } else { b"complete" },
            &task.fence.attempt.to_be_bytes(),
            &task.fence.generation.to_be_bytes(),
            payload,
        ]),
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

#[cfg(test)]
mod tests {
    use sagat_domain::blobs::BlobId;
    use sagat_domain::execution::{ExecutionInstanceId, Fence, InstancePath, RunId};
    use sagat_domain::workflow::NodeId;
    use uuid::Uuid;

    use super::{
        DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR, PROTOCOL_MINOR, ROOT_PROTOCOL_MINOR,
        SCOPED_PROTOCOL_MINOR, StoreProofs, TaskRef, blob_report_identity, json_escaped,
        negotiate_protocol, negotiate_protocol_details, report_scope, require_supported_task,
        root_task_id,
    };

    /// The proofs of a store, in the order of the minors they open.
    const fn proofs(scoped: bool, database: bool, delivery_acknowledgement: bool) -> StoreProofs {
        StoreProofs {
            scoped,
            database,
            delivery_acknowledgement,
        }
    }

    #[test]
    fn managed_support_does_not_enable_scoped_protocol_for_ordinary_workers() {
        use sagat_domain::identity::Role;
        let ceiling = |role, scoped, database| {
            super::protocol_ceiling(role, proofs(scoped, database, false), PROTOCOL_MINOR)
        };
        for role in [Role::Worker, Role::Runner, Role::Admin] {
            assert_eq!(ceiling(role, false, true), 4);
            assert_eq!(ceiling(role, true, false), 5);
            assert_eq!(ceiling(role, true, true), 6);
        }
        assert_eq!(ceiling(Role::DatabaseRunner, false, true), 6);
        assert_eq!(ceiling(Role::DatabaseRunner, false, false), 4);
    }

    /// Feature 006, Task 006-048 (decision D006-105): the negotiation that answers 1.8, with the compiled ceiling raised
    /// in this test only. This build's own ceiling stays below it, so no store's proof makes a session 1.8.
    #[test]
    fn delivery_acknowledgement_is_answered_only_above_the_compiled_ceiling_and_on_the_store_proof()
    {
        use sagat_domain::identity::Role;
        let raised = DELIVERY_ACKNOWLEDGEMENT_PROTOCOL_MINOR;
        for role in [Role::Worker, Role::Runner, Role::Admin] {
            // every proof and the raised ceiling: 1.8
            assert_eq!(
                super::protocol_ceiling(role, proofs(true, true, true), raised),
                8
            );
            // the proof of 1.8 without the minors below it opens nothing more
            assert_eq!(
                super::protocol_ceiling(role, proofs(false, true, true), raised),
                4
            );
            assert_eq!(
                super::protocol_ceiling(role, proofs(true, false, true), raised),
                5
            );
            // without the store's proof the raised ceiling stops below 1.8
            assert_eq!(
                super::protocol_ceiling(role, proofs(true, true, false), raised),
                6
            );
            // this build's ceiling answers its own minor whatever the store proves
            assert_eq!(
                super::protocol_ceiling(role, proofs(true, true, true), PROTOCOL_MINOR),
                PROTOCOL_MINOR
            );
        }
        // the dedicated database runner keeps 1.6 (ADR-0099 decision 11)
        assert_eq!(
            super::protocol_ceiling(Role::DatabaseRunner, proofs(true, true, true), raised),
            6
        );
        assert_eq!(
            super::protocol_ceiling(Role::DatabaseRunner, proofs(false, true, true), raised),
            6
        );
        // a worker that offers 1.8 is agreed at 1.8 under the raised ceiling and at this build's own below it
        assert_eq!(
            negotiate_protocol_details("1.8", raised),
            Some(("1.8".to_owned(), 8))
        );
        assert_eq!(
            negotiate_protocol_details("1.999", raised),
            Some(("1.8".to_owned(), 8))
        );
        // this build answers its own minor, below 1.8 (the compile-time assert of `withdrawal.rs` keeps it there)
        assert_eq!(
            negotiate_protocol("1.8"),
            Some(format!("1.{PROTOCOL_MINOR}"))
        );
    }

    #[test]
    fn blob_report_identity_normalizes_sha256_hex_case() {
        let id = BlobId::generate();
        let lowercase = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let uppercase = lowercase.to_ascii_uppercase();

        assert_eq!(
            blob_report_identity(id, lowercase),
            blob_report_identity(id, &uppercase)
        );
        assert_ne!(
            blob_report_identity(id, lowercase),
            blob_report_identity(BlobId::generate(), lowercase)
        );
    }

    #[test]
    fn the_newest_version_both_sides_speak_is_agreed() {
        assert_eq!(negotiate_protocol("1").as_deref(), Some("1"));
        assert_eq!(negotiate_protocol("1.0").as_deref(), Some("1"));
        assert_eq!(negotiate_protocol("1.1").as_deref(), Some("1.1"));
        let own = format!("1.{PROTOCOL_MINOR}");
        assert_eq!(negotiate_protocol(&own).as_deref(), Some(own.as_str()));
        // A worker that is ahead of the server runs at the server's version.
        assert_eq!(negotiate_protocol("1.999").as_deref(), Some(own.as_str()));
    }

    #[test]
    fn scoped_identity_negotiation_is_gated_by_storage_support() {
        assert_eq!(PROTOCOL_MINOR, 6);
        assert_eq!(ROOT_PROTOCOL_MINOR, 4);
        assert_eq!(negotiate_protocol("1.5").as_deref(), Some("1.5"));
        assert_eq!(
            negotiate_protocol_details("1.5", ROOT_PROTOCOL_MINOR),
            Some(("1.4".to_owned(), ROOT_PROTOCOL_MINOR))
        );
        assert_eq!(
            negotiate_protocol_details("1.999", SCOPED_PROTOCOL_MINOR),
            Some(("1.5".to_owned(), SCOPED_PROTOCOL_MINOR))
        );
    }

    #[test]
    fn legacy_task_ids_and_worker_entry_points_are_root_only() {
        let run = RunId::new(Uuid::from_u128(1));
        let node = NodeId::parse("charge").expect("node");
        let root = ExecutionInstanceId::root(node.clone());
        let scoped = ExecutionInstanceId::in_path(
            InstancePath::root()
                .child(NodeId::parse("lines").expect("scope"), 0)
                .expect("path"),
            node,
        );

        assert_eq!(root_task_id(run, &root, 3), Some(format!("{run}/charge/3")));
        assert_eq!(root_task_id(run, &scoped, 3), None);
        assert!(require_supported_task(&root, false).is_ok());
        assert!(matches!(
            require_supported_task(&scoped, false),
            Err(super::AppError::StaleAttempt)
        ));
        assert!(require_supported_task(&scoped, true).is_ok());
    }

    #[test]
    fn report_identity_keeps_root_keys_and_separates_scoped_members() {
        fn task(run: RunId, instance: &ExecutionInstanceId) -> TaskRef<'_> {
            TaskRef {
                run_id: run,
                instance,
                step_id: None,
                fence: Fence {
                    attempt: 1,
                    generation: 1,
                },
                lease_token: "super-secret-token",
            }
        }

        let run = RunId::new(Uuid::from_u128(1));
        let node = NodeId::parse("charge").expect("node");
        let scope = NodeId::parse("lines").expect("scope");
        let root = ExecutionInstanceId::root(node.clone());
        let first = ExecutionInstanceId::in_path(
            InstancePath::root().child(scope.clone(), 0).expect("path"),
            node.clone(),
        );
        let second =
            ExecutionInstanceId::in_path(InstancePath::root().child(scope, 1).expect("path"), node);
        assert!(report_scope(&task(run, &root)).starts_with(&format!("step:{run}:charge:")));
        assert_ne!(
            report_scope(&task(run, &first)),
            report_scope(&task(run, &second))
        );
        assert!(!format!("{:?}", task(run, &first)).contains("super-secret-token"));
    }

    #[test]
    fn anything_else_is_not_a_version_this_server_speaks() {
        for bad in [
            "",
            "0",
            "2",
            "2.0",
            "1.",
            ".1",
            "1.x",
            "01",
            "1.01",
            "1.2.3",
            " 1",
            "1 ",
            "v1",
            "1.-1",
            "1.+1",
            "1.1234567",
        ] {
            assert_eq!(negotiate_protocol(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_value_is_searched_for_as_it_would_be_written_in_json() {
        assert_eq!(json_escaped("plain-value-1", false), "plain-value-1");
        assert_eq!(json_escaped(r#"a"b\c"#, false), r#"a\"b\\c"#);
        assert_eq!(
            json_escaped("line\nbreak\ttab\r", false),
            r"line\nbreak\ttab\r"
        );
        assert_eq!(json_escaped("bell\u{7}", false), r"bell\u0007");
        // Non-ASCII is left alone by default and spelled out by serializers that escape it.
        assert_eq!(json_escaped("caf\u{e9}", false), "caf\u{e9}");
        assert_eq!(json_escaped("caf\u{e9}", true), "caf\\u00e9");
        // Beyond the BMP it is a surrogate pair.
        assert_eq!(json_escaped("\u{1f511}", true), "\\ud83d\\udd11");
    }
}

/// Feature 006, decision D006-092: the rollback for a gone consumer, its exception for kept writes, and its counter.
#[cfg(test)]
mod rollback_for_a_gone_consumer {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{ClaimWatch, roll_back_for_a_gone_consumer};

    struct Watch {
        gone: AtomicBool,
        barriers: AtomicUsize,
    }

    impl ClaimWatch for Watch {
        fn consumer_gone(&self) -> bool {
            self.gone.load(Ordering::SeqCst)
        }

        fn before_commit(
            &self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            self.barriers.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::ready(()))
        }
    }

    fn watch(gone: bool) -> Watch {
        Watch {
            gone: AtomicBool::new(gone),
            barriers: AtomicUsize::new(0),
        }
    }

    #[test]
    fn only_a_transaction_of_leases_alone_is_rolled_back_and_each_rollback_is_counted() {
        let (metrics, recorder) = sagat_telemetry::Metrics::detached(Duration::from_secs(60));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        let (gone, kept, present) = (watch(true), watch(true), watch(false));
        let decisions = metrics::with_local_recorder(&recorder, || {
            runtime.block_on(async {
                (
                    roll_back_for_a_gone_consumer(&gone, false).await,
                    roll_back_for_a_gone_consumer(&kept, true).await,
                    roll_back_for_a_gone_consumer(&present, false).await,
                )
            })
        });
        assert_eq!(decisions, (true, false, false));
        // the test's barrier is awaited in every case, before the question
        for each in [&gone, &kept, &present] {
            assert_eq!(each.barriers.load(Ordering::SeqCst), 1);
        }
        let page = metrics.render();
        assert!(
            page.contains("worker_claim_skipped_total{reason=\"consumer_gone\"} 1"),
            "{page}"
        );
    }
}

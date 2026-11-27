//! Delivery acknowledgement (worker protocol 1.8; feature 006, Task 006-049; Architecture decision 0099 decisions 3, 4, 6
//! and 7; `specs/006-sdk-multilanguage/stream-completion.md`, "The server's acknowledgement").
//!
//! A host of 1.8 acknowledges the flagged leases of one frame, or of one `PollTasks` answer, before it runs or calls anything
//! else for them. The acknowledgement is one transaction per call, for the tasks of one namespace, through a port of its own
//! ([`AcknowledgementTx`]). It locks rows, never the run: an unlocked read of the addressed rows whose token matches, the
//! binding checks under their locks (the function the run transactions call), one statement that locks the matched rows
//! `FOR NO KEY UPDATE OF steps SKIP LOCKED` and reads them with the run's stop reason, the server's start time and the
//! clock, the verdicts decided on the locked rows by one pure function ([`decide`]), and one update of the rows that win.
//! It renews nothing, applies no transition and writes no journal event and no outbox row.
//!
//! The call has a deadline of half the receive bound from its arrival; the adapter checks it before each statement and,
//! once it has passed, rolls the transaction back. Every transient failure is retryable (`UNAVAILABLE`): no transient error
//! is ever answered `NOT_HELD`.

use std::future::Future;
use std::time::{Duration, Instant};

use sagat_domain::database::MANAGED_DATABASE_PREFIX;
use sagat_domain::execution::{ExecutionInstanceId, RunId};
use sagat_domain::identity::{NamespaceId, Permission, Principal, TenantId};
use time::OffsetDateTime;
use uuid::Uuid;

use super::binding_checks::{BindingChecks, BindingFailure, BindingJudgement, judge_binding};
use super::{TaskKind, TaskRef, WorkerService};
use crate::clock::Clock;
use crate::entropy::{Entropy, Token};
use crate::error::{AccessDenial, AppError, StoreError};
use crate::execution::{ExpressionEvaluator, LeaseMarks};

/// Most leases one call acknowledges: the credit of one receive window.
pub const MAX_ACKNOWLEDGEMENT_TASKS: usize = 64;
/// The bound a host gives one receive call (`sdks/SPEC.md` §10), and the margin an acknowledged lease must have left before
/// both its deadlines.
pub const RECEIVE_BOUND: Duration = Duration::from_secs(6);
/// The deadline of one acknowledgement, from its arrival: half the receive bound, pool acquisition included.
pub const ACKNOWLEDGEMENT_DEADLINE: Duration = Duration::from_secs(3);
/// The lock and statement timeouts of the acknowledgement's transaction, set once: a call ends at most this long past its
/// deadline.
pub const ACKNOWLEDGEMENT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(1);

/// The envelope reason of a call whose deadline passed (`UNAVAILABLE`): ask again. Never `DEADLINE_EXCEEDED`, which the
/// host keeps for its own bound.
pub const REASON_ACKNOWLEDGEMENT_DEADLINE: &str = "ACKNOWLEDGEMENT_DEADLINE";
/// The envelope reason of a call on an engine that did not reach a writable database (`UNAVAILABLE`): it decides no
/// host's early give-up.
pub const REASON_ACKNOWLEDGEMENT_STORE_UNREACHABLE: &str = "ACKNOWLEDGEMENT_STORE_UNREACHABLE";
/// The envelope reason of an acknowledgement shed by admission (`RESOURCE_EXHAUSTED`).
pub const REASON_ACKNOWLEDGEMENT_SHED: &str = "ACKNOWLEDGEMENT_SHED";

/// When a call must be over, from its arrival in the outermost layer of the gRPC server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallDeadline(Instant);

impl CallDeadline {
    /// The deadline of an acknowledgement that arrived at `arrival`.
    #[must_use]
    pub fn of_arrival(arrival: Instant) -> Self {
        Self(arrival + ACKNOWLEDGEMENT_DEADLINE)
    }

    #[must_use]
    pub const fn at(instant: Instant) -> Self {
        Self(instant)
    }

    #[must_use]
    pub const fn instant(self) -> Instant {
        self.0
    }

    /// The time left at `now`, `None` once it has passed.
    #[must_use]
    pub fn left(self, now: Instant) -> Option<Duration> {
        self.0
            .checked_duration_since(now)
            .filter(|left| !left.is_zero())
    }

    #[must_use]
    pub fn passed(self) -> bool {
        self.left(Instant::now()).is_none()
    }
}

/// Why an acknowledgement call failed as a whole. Every one but `Refused` is retryable: the host asks again, and the rows
/// already written answer `ACKNOWLEDGED` again.
#[derive(Debug, thiserror::Error)]
pub enum AcknowledgementFailure {
    /// The call's deadline passed: the transaction was rolled back (`UNAVAILABLE`, `ACKNOWLEDGEMENT_DEADLINE`).
    #[error("the acknowledgement's deadline passed")]
    Deadline,
    /// The engine did not reach a writable database (`UNAVAILABLE`, `ACKNOWLEDGEMENT_STORE_UNREACHABLE`).
    #[error("the database could not be reached")]
    StoreUnreachable,
    /// A deadlock, a serialization failure, a lock or statement timeout: `UNAVAILABLE`, ask again.
    #[error("the acknowledgement failed transiently")]
    Transient,
    /// The request is malformed (`INVALID_ARGUMENT`), before any lock is taken: the message names what, never a value.
    #[error("malformed acknowledgement: {0}")]
    Malformed(&'static str),
    /// The call is refused: the permission or the membership (`PERMISSION_DENIED`), a store failure that is no transient
    /// one (`INTERNAL`).
    #[error(transparent)]
    Refused(#[from] AppError),
}

impl AcknowledgementFailure {
    /// The label of `worker_task_acknowledgement_failures_total{reason}`, a closed set.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Deadline => "deadline",
            Self::StoreUnreachable => "store",
            Self::Transient => "transient",
            Self::Malformed(_) => "malformed",
            Self::Refused(_) => "refused",
        }
    }
}

/// A call that failed as a whole, with the one binding refusal it found before it failed: the transport audits it whatever
/// the call's status.
#[derive(Debug)]
pub struct FailedAcknowledgement {
    pub failure: AcknowledgementFailure,
    pub denial: Option<AccessDenial>,
    /// The runs addressed with no matching row, found before the call failed: checked against the other tenants all the same.
    pub unmatched_runs: Vec<RunId>,
}

impl From<AcknowledgementFailure> for FailedAcknowledgement {
    fn from(failure: AcknowledgementFailure) -> Self {
        Self {
            failure,
            denial: None,
            unmatched_runs: Vec::new(),
        }
    }
}

/// What a port of the acknowledgement fails with.
#[derive(Debug, thiserror::Error)]
pub enum AcknowledgementStoreError {
    /// The call's deadline passed before a statement: the adapter rolled back.
    #[error("the acknowledgement's deadline passed")]
    Deadline,
    /// No writable database was reached: the transaction did not begin before the deadline, its pool acquisition failed,
    /// or a statement or the commit failed with a connection error or on a read-only server.
    #[error("the database could not be reached")]
    Unreachable,
    /// Any other failure of the store.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<AcknowledgementStoreError> for AcknowledgementFailure {
    fn from(error: AcknowledgementStoreError) -> Self {
        match error {
            AcknowledgementStoreError::Deadline => Self::Deadline,
            AcknowledgementStoreError::Unreachable => Self::StoreUnreachable,
            AcknowledgementStoreError::Store(StoreError::Unavailable { .. }) => Self::Transient,
            AcknowledgementStoreError::Store(StoreError::DatabaseUnavailable) => {
                Self::StoreUnreachable
            }
            AcknowledgementStoreError::Store(other) => Self::Refused(other.into()),
        }
    }
}

/// One lease of a request: where it lives and the hash of the token presented for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseProbe {
    pub run_id: RunId,
    pub instance: ExecutionInstanceId,
    pub token_hash: Vec<u8>,
}

/// An addressed row whose lease token, or last withdrawn token, is the one presented, read with no lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedLease {
    /// The position of its task in the request.
    pub index: usize,
    pub namespace: NamespaceId,
    /// The activity a worker must advertise for it; `None` for a node that is no activity.
    pub activity: Option<String>,
    /// The presented token is the row's current lease token.
    pub current: bool,
}

/// A row `lock_matched` holds, with what the decision reads of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedLease {
    pub index: usize,
    pub step_id: Uuid,
    /// The step is `LEASED` (not yet `RUNNING`).
    pub leased: bool,
    pub attempt: u32,
    pub kind: TaskKind,
    /// `lease_token_hash`; `None` once the lease ended.
    pub token_hash: Option<Vec<u8>>,
    /// `withdrawn_lease_token_hash`: the token of the last withdrawal of a marked lease.
    pub withdrawn_token_hash: Option<Vec<u8>>,
    /// The reservation, its deadlines, and whether it was claimed on the server that answers now.
    pub marks: LeaseMarks,
    /// `lease_offered_generation`.
    pub offered_generation: Option<u64>,
    /// `lease_acknowledged_at` is set.
    pub acknowledged: bool,
    pub cancel_requested: bool,
    /// The run has a stop reason.
    pub run_stopping: bool,
    /// The database's clock, sampled once the statement holds the row's lock.
    pub now: OffsetDateTime,
}

/// A row the acknowledgement wrote, with the clock of its update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrittenAcknowledgement {
    pub index: usize,
    pub now: OffsetDateTime,
    pub claimed_at: Option<OffsetDateTime>,
    pub expires_at: OffsetDateTime,
    pub attempt_deadline_at: Option<OffsetDateTime>,
}

/// The store of the acknowledgement: one transaction per call, begun under the call's deadline.
pub trait AcknowledgementStore: Send + Sync {
    type Tx: AcknowledgementTx;

    /// Takes a connection and begins the transaction within the time `deadline` leaves, with the one settings statement:
    /// `lock_timeout` and `statement_timeout` at [`ACKNOWLEDGEMENT_STATEMENT_TIMEOUT`], and `synchronous_commit` raised to
    /// `local` when it is `off`. The acquisition and the beginning fail as [`AcknowledgementStoreError::Unreachable`].
    fn begin_acknowledgement(
        &self,
        tenant: TenantId,
        deadline: CallDeadline,
    ) -> impl Future<Output = Result<Self::Tx, AcknowledgementStoreError>> + Send;

    /// One of `runs` that exists in a tenant other than `tenant` (one statement, on a connection of its own). Asked off the
    /// answer path: with no index on the run id alone it can scan the runs (decision D006-110).
    fn run_of_another_tenant(
        &self,
        tenant: TenantId,
        runs: &[RunId],
    ) -> impl Future<Output = Result<Option<RunId>, StoreError>> + Send;

    /// Records an access to a resource of another tenant (best effort, collapsed when repeated).
    fn record_cross_tenant(
        &self,
        audit: &sagat_domain::audit::AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// The statements of one acknowledgement, each checked against the call's deadline before it runs.
pub trait AcknowledgementTx: Send {
    /// One statement with no lock, by the step primary key: the addressed rows of this tenant whose current lease token,
    /// or last withdrawn token, hashes as presented.
    fn matched(
        &mut self,
        leases: &[LeaseProbe],
    ) -> impl Future<Output = Result<Vec<MatchedLease>, AcknowledgementStoreError>> + Send;

    /// The patterns of `subject` in `namespace`, under the binding's shared advisory lock.
    fn bound_patterns(
        &mut self,
        namespace: NamespaceId,
        subject: &str,
    ) -> impl Future<Output = Result<Vec<String>, AcknowledgementStoreError>> + Send;

    /// The membership of `subject`, under the namespace's shared secrets lock and the membership's row lock.
    fn current_worker_authorized(
        &mut self,
        namespace: NamespaceId,
        subject: &str,
    ) -> impl Future<Output = Result<bool, AcknowledgementStoreError>> + Send;

    /// One statement: the rows of `indices` locked by primary key `FOR NO KEY UPDATE OF steps SKIP LOCKED` in primary-key
    /// order, read with the run's stop reason, the server's start time and the clock. A row it could not lock is absent.
    fn lock_matched(
        &mut self,
        leases: &[LeaseProbe],
        indices: &[usize],
    ) -> impl Future<Output = Result<Vec<LockedLease>, AcknowledgementStoreError>> + Send;

    /// One update of the winning rows, which the transaction holds: `lease_acknowledged_at = clock_timestamp()`.
    fn acknowledge(
        &mut self,
        leases: &[LeaseProbe],
        winners: &[(usize, u64)],
    ) -> impl Future<Output = Result<Vec<WrittenAcknowledgement>, AcknowledgementStoreError>> + Send;

    fn commit(self) -> impl Future<Output = Result<(), AcknowledgementStoreError>> + Send;

    /// Rolls back and returns the connection (discarded when the rollback does not return in time).
    fn rollback(self) -> impl Future<Output = ()> + Send;
}

/// Why a lease is not this host's provisional lease (`ACKNOWLEDGEMENT_VERDICT_NOT_HELD`): the label `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotHeld {
    /// No addressed row holds the presented token with the request's attempt, generation and step identity.
    NoMatch,
    /// The token is the step's last withdrawn one (a probe's is).
    Withdrawn,
    /// Provisional, with less than one receive bound left before a deadline: the lost-lease pass withdraws it.
    TooLate,
    /// Held, but never marked provisional (or a node no worker is acknowledged for).
    NotProvisional,
    /// The identity is no longer bound to its activity.
    Unbound,
}

impl NotHeld {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoMatch => "no_match",
            Self::Withdrawn => "withdrawn",
            Self::TooLate => "too_late",
            Self::NotProvisional => "not_provisional",
            Self::Unbound => "unbound",
        }
    }
}

/// Why the host must run nothing and hand the lease back (`ACKNOWLEDGEMENT_VERDICT_CANCELLED`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cancelled {
    /// A `FORWARD` task whose step is asked to stop or whose run has a stop reason.
    StopAsked,
    /// Acknowledged before, or a mark that does not count, with less than one receive bound left.
    TooLate,
}

impl Cancelled {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StopAsked => "stop_asked",
            Self::TooLate => "too_late",
        }
    }
}

/// How an `ACKNOWLEDGED` answer came about: the label `verdict` of `worker_task_acknowledgements_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acknowledged {
    /// This call wrote the acknowledgement.
    Written,
    /// Acknowledged before, with the same token: nothing written.
    Duplicate,
    /// A provisional mark that no longer counts (another server, or none): an attempt, nothing written.
    Recovered,
}

/// The verdict of one locked row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Provisional, fenced, no stop, the margin left: write the acknowledgement.
    Win,
    /// Answer `ACKNOWLEDGED` and write nothing.
    Acknowledged(Acknowledged),
    Cancelled(Cancelled),
    NotHeld(NotHeld),
}

/// What the request presents of one lease, for the decision.
#[derive(Debug, Clone, Copy)]
pub struct Presented<'a> {
    pub attempt: u32,
    pub generation: u64,
    pub step_id: Option<Uuid>,
    pub token_hash: &'a [u8],
}

/// Whether `deadline` is at least one receive bound after `now`.
fn margin_before(deadline: OffsetDateTime, now: OffsetDateTime) -> bool {
    deadline - now >= RECEIVE_BOUND
}

/// The verdict of one row the acknowledgement holds locked, sampled at `now` (`stream-completion.md`, "The server's
/// acknowledgement", step 5). The token is compared here, not in the statement, so a row the lock skipped is always a row
/// another transaction held.
#[must_use]
pub fn decide(row: &LockedLease, presented: &Presented<'_>, now: OffsetDateTime) -> Decision {
    if row.token_hash.as_deref() != Some(presented.token_hash) {
        return Decision::NotHeld(
            if row.withdrawn_token_hash.as_deref() == Some(presented.token_hash) {
                NotHeld::Withdrawn
            } else {
                NotHeld::NoMatch
            },
        );
    }
    if !row.marks.held
        || row.marks.generation != presented.generation
        || row.attempt != presented.attempt
        || presented.step_id.is_some_and(|step| step != row.step_id)
    {
        return Decision::NotHeld(NotHeld::NoMatch);
    }
    if row.offered_generation != Some(row.marks.generation) {
        return Decision::NotHeld(NotHeld::NotProvisional);
    }
    let stop = row.kind == TaskKind::Forward && (row.cancel_requested || row.run_stopping);
    let margin = margin_before(row.marks.deadlines.expires_at, now)
        && row
            .marks
            .deadlines
            .attempt_deadline_at
            .is_none_or(|deadline| margin_before(deadline, now));
    let counts = row.marks.counts();
    if counts && !row.acknowledged {
        if !row.leased {
            // a host that heartbeated before its acknowledgement: the lease is no longer provisional, and the pass ends it
            // as any running lease (the heartbeat is the violation counted)
            return Decision::NotHeld(NotHeld::NotProvisional);
        }
        return if stop {
            Decision::Cancelled(Cancelled::StopAsked)
        } else if margin {
            Decision::Win
        } else {
            Decision::NotHeld(NotHeld::TooLate)
        };
    }
    let how = if counts {
        Acknowledged::Duplicate
    } else {
        Acknowledged::Recovered
    };
    if stop {
        Decision::Cancelled(Cancelled::StopAsked)
    } else if margin {
        Decision::Acknowledged(how)
    } else {
        Decision::Cancelled(Cancelled::TooLate)
    }
}

/// The answer for one task, in request order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcknowledgementVerdict {
    Acknowledged {
        how: Acknowledged,
        /// Before the lease's expiry, at the clock sample; never below zero.
        lease_time_left: Duration,
        /// Before the attempt's deadline, when it has one; never below zero.
        attempt_time_left: Option<Duration>,
    },
    NotHeld(NotHeld),
    Cancelled(Cancelled),
    /// Its row was held by another transaction: ask again for this task.
    Retry,
}

impl AcknowledgementVerdict {
    /// The labels `verdict` and `reason` of `worker_task_acknowledgements_total`, closed sets.
    #[must_use]
    pub const fn labels(&self) -> (&'static str, &'static str) {
        match self {
            Self::Acknowledged { how, .. } => match how {
                Acknowledged::Written => ("acknowledged", "none"),
                Acknowledged::Duplicate => ("duplicate", "none"),
                Acknowledged::Recovered => ("recovered", "none"),
            },
            Self::NotHeld(reason) => ("not_held", reason.as_str()),
            Self::Cancelled(reason) => ("cancelled", reason.as_str()),
            Self::Retry => ("retry", "none"),
        }
    }
}

/// What a call answers: one verdict per task in request order, the one binding refusal to audit, and the runs it addressed
/// with no matching row, for the transport to check against the other tenants off the answer path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcknowledgementAnswer {
    pub verdicts: Vec<AcknowledgementVerdict>,
    /// At most one per call, audited by the transport whatever the call's status.
    pub denial: Option<AccessDenial>,
    /// The run ids of the tasks with no matching row, distinct: [`WorkerService::audit_other_tenants`] records one that
    /// exists in another tenant, once per call.
    pub unmatched_runs: Vec<RunId>,
}

fn non_negative(from: OffsetDateTime, to: OffsetDateTime) -> Duration {
    let left = to - from;
    if left.is_negative() {
        Duration::ZERO
    } else {
        left.unsigned_abs()
    }
}

/// The checks of the request that need no store: 1 to 64 tasks with distinct task ids.
///
/// # Errors
/// What is wrong, as the `INVALID_ARGUMENT` message.
pub fn check_request<T: AsRef<str>>(task_ids: &[T]) -> Result<(), &'static str> {
    if task_ids.is_empty() || task_ids.len() > MAX_ACKNOWLEDGEMENT_TASKS {
        return Err("tasks must hold 1 to 64 tasks");
    }
    let mut seen = std::collections::BTreeSet::new();
    if !task_ids.iter().all(|id| seen.insert(id.as_ref())) {
        return Err("tasks must name each task_id once");
    }
    Ok(())
}

/// The binding checks over the acknowledgement's transaction, in the call's one namespace.
struct AcknowledgementBinding<'a, T> {
    tx: &'a mut T,
    namespace: NamespaceId,
}

impl<T: AcknowledgementTx> BindingChecks for AcknowledgementBinding<'_, T> {
    type Error = AcknowledgementStoreError;

    fn bound_patterns(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<Vec<String>, AcknowledgementStoreError>> + Send {
        self.tx.bound_patterns(self.namespace, subject)
    }

    fn current_worker_authorized(
        &mut self,
        subject: &str,
    ) -> impl Future<Output = Result<bool, AcknowledgementStoreError>> + Send {
        self.tx.current_worker_authorized(self.namespace, subject)
    }
}

fn count(verdicts: &[AcknowledgementVerdict]) {
    for verdict in verdicts {
        let (verdict, reason) = verdict.labels();
        metrics::counter!("worker_task_acknowledgements_total", "verdict" => verdict, "reason" => reason)
            .increment(1);
    }
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: AcknowledgementStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    /// Acknowledges the flagged leases of one frame or poll answer (protocol 1.8), one transaction for the call, before
    /// `deadline`.
    ///
    /// # Errors
    /// [`FailedAcknowledgement`]: the deadline, an unreachable store and a transient failure are retryable; a malformed
    /// request, a missing permission or a revoked membership refuse the call.
    pub async fn acknowledge_tasks(
        &self,
        principal: &Principal,
        tasks: &[TaskRef<'_>],
        deadline: CallDeadline,
    ) -> Result<AcknowledgementAnswer, FailedAcknowledgement> {
        let outcome = self.acknowledge_within(principal, tasks, deadline).await;
        match &outcome {
            Ok(answer) => count(&answer.verdicts),
            Err(FailedAcknowledgement { failure, .. }) => {
                if matches!(failure, AcknowledgementFailure::Deadline) {
                    metrics::counter!("worker_task_acknowledgements_total", "verdict" => "deadline", "reason" => "none")
                        .increment(u64::try_from(tasks.len()).unwrap_or(u64::MAX));
                }
                metrics::counter!("worker_task_acknowledgement_failures_total", "reason" => failure.label())
                    .increment(1);
            }
        }
        outcome
    }

    /// Records, at most once, that a call addressed a run of another tenant: the first of `runs` that exists in a tenant other
    /// than the caller's (`CrossTenantAccessDenied`, the resource named, nothing of the other tenant's). The transport runs it
    /// off the answer path, under its bounded gate of audit writes.
    ///
    /// # Errors
    /// A store failure, which the transport counts and drops.
    pub async fn audit_other_tenants(
        &self,
        principal: &Principal,
        runs: &[RunId],
        trace_id: Option<String>,
    ) -> Result<bool, StoreError> {
        if runs.is_empty() {
            return Ok(false);
        }
        let Some(run) = self
            .store
            .run_of_another_tenant(principal.tenant_id(), runs)
            .await?
        else {
            return Ok(false);
        };
        let record = sagat_domain::audit::AuditRecord::by(
            principal,
            sagat_domain::audit::AuditAction::CrossTenantAccessDenied,
            sagat_domain::audit::ResourceType::Run,
            run.to_string(),
            trace_id,
            self.clock.now(),
        )
        .with_reason("resource not accessible from this tenant");
        self.store.record_cross_tenant(&record).await?;
        Ok(true)
    }

    async fn acknowledge_within(
        &self,
        principal: &Principal,
        tasks: &[TaskRef<'_>],
        deadline: CallDeadline,
    ) -> Result<AcknowledgementAnswer, FailedAcknowledgement> {
        principal
            .authorize(Permission::WorkerExecute)
            .map_err(|error| AcknowledgementFailure::from(AppError::from(error)))?;
        if tasks.is_empty() || tasks.len() > MAX_ACKNOWLEDGEMENT_TASKS {
            return Err(FailedAcknowledgement::from(
                AcknowledgementFailure::Malformed("tasks must hold 1 to 64 tasks"),
            ));
        }
        let mut addresses = std::collections::BTreeSet::new();
        if !tasks
            .iter()
            .all(|task| addresses.insert((task.run_id.as_uuid(), task.instance.clone())))
        {
            // the update writes each row once: two tasks of one row would be one write for two answers
            return Err(FailedAcknowledgement::from(
                AcknowledgementFailure::Malformed("tasks must name each lease once"),
            ));
        }
        if deadline.passed() {
            return Err(AcknowledgementFailure::Deadline.into());
        }
        let probes: Vec<LeaseProbe> = tasks
            .iter()
            .map(|task| LeaseProbe {
                run_id: task.run_id,
                instance: task.instance.clone(),
                token_hash: Token::hash_of(task.lease_token),
            })
            .collect();
        let mut tx = self
            .store
            .begin_acknowledgement(principal.tenant_id(), deadline)
            .await
            .map_err(AcknowledgementFailure::from)?;
        let (mut denial, mut delays, mut unmatched_runs) = (None, Vec::new(), Vec::new());
        let outcome = Self::acknowledge_in(
            &mut tx,
            principal,
            tasks,
            &probes,
            &mut denial,
            &mut delays,
            &mut unmatched_runs,
        )
        .await;
        match outcome {
            Ok(mut answer) => {
                if let Err(error) = tx.commit().await {
                    return Err(FailedAcknowledgement {
                        failure: error.into(),
                        denial,
                        unmatched_runs,
                    });
                }
                // what committed only: a delay of an acknowledgement rolled back is no delay
                for delay in delays {
                    metrics::histogram!("worker_task_acknowledgement_delay_seconds")
                        .record(delay.as_secs_f64());
                }
                answer.denial = denial;
                answer.unmatched_runs = unmatched_runs;
                Ok(answer)
            }
            Err(failure) => {
                tx.rollback().await;
                Err(FailedAcknowledgement {
                    failure,
                    denial,
                    unmatched_runs,
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn acknowledge_in(
        tx: &mut S::Tx,
        principal: &Principal,
        tasks: &[TaskRef<'_>],
        probes: &[LeaseProbe],
        denial: &mut Option<AccessDenial>,
        delays: &mut Vec<Duration>,
        unmatched: &mut Vec<RunId>,
    ) -> Result<AcknowledgementAnswer, AcknowledgementFailure> {
        let mut verdicts = vec![AcknowledgementVerdict::NotHeld(NotHeld::NoMatch); tasks.len()];
        let mut answer = AcknowledgementAnswer {
            verdicts: Vec::new(),
            denial: None,
            unmatched_runs: Vec::new(),
        };
        let matched = tx.matched(probes).await?;
        // found before anything can fail the call: they are checked against the other tenants whatever it answers
        let mut unmatched_runs: Vec<RunId> = probes
            .iter()
            .enumerate()
            .filter(|(index, _)| !matched.iter().any(|m| m.index == *index))
            .map(|(_, probe)| probe.run_id)
            .collect();
        unmatched_runs.sort_unstable_by_key(|run| run.as_uuid());
        unmatched_runs.dedup();
        *unmatched = unmatched_runs;
        // The namespaces of the rows that hold a presented token now: a row matched by its withdrawn token alone is answered
        // `withdrawn` without a lock, and says nothing of where a live lease is.
        let mut namespaces: Vec<NamespaceId> = matched
            .iter()
            .filter(|m| m.current)
            .map(|m| m.namespace)
            .collect();
        namespaces.sort_unstable_by_key(|n| n.as_uuid());
        namespaces.dedup();
        if namespaces.len() > 1 {
            return Err(AcknowledgementFailure::Malformed(
                "tasks must belong to one namespace",
            ));
        }
        let Some(&namespace) = namespaces.first() else {
            for lease in &matched {
                verdicts[lease.index] = AcknowledgementVerdict::NotHeld(NotHeld::Withdrawn);
            }
            answer.verdicts = verdicts;
            return Ok(answer);
        };
        // Rows whose token is only the withdrawn one, and rows of a node no worker is acknowledged for, are answered without
        // any lock.
        let mut candidates = Vec::new();
        for lease in &matched {
            if !lease.current {
                verdicts[lease.index] = AcknowledgementVerdict::NotHeld(NotHeld::Withdrawn);
                continue;
            }
            match &lease.activity {
                Some(activity) if !activity.starts_with(MANAGED_DATABASE_PREFIX) => {
                    candidates.push((lease.index, activity.clone()));
                }
                _ => {
                    verdicts[lease.index] =
                        AcknowledgementVerdict::NotHeld(NotHeld::NotProvisional);
                }
            }
        }
        if candidates.is_empty() {
            answer.verdicts = verdicts;
            return Ok(answer);
        }
        let activities: Vec<String> = candidates.iter().map(|(_, a)| a.clone()).collect();
        let judged = judge_binding(
            &mut AcknowledgementBinding {
                tx: &mut *tx,
                namespace,
            },
            principal.subject().as_str(),
            &activities,
        )
        .await
        .map_err(|failure| match failure {
            BindingFailure::Store(error) => AcknowledgementFailure::from(error),
            BindingFailure::CorruptPattern => AppError::Internal.into(),
        })?;
        let covered = match judged {
            BindingJudgement::NotMember => {
                return Err(AppError::Forbidden {
                    permission: Permission::WorkerExecute,
                }
                .into());
            }
            BindingJudgement::Judged(covered) => covered,
        };
        let mut bound = Vec::new();
        for ((index, _), covered) in candidates.iter().zip(covered) {
            if covered {
                bound.push(*index);
            } else {
                verdicts[*index] = AcknowledgementVerdict::NotHeld(NotHeld::Unbound);
                *denial = Some(AccessDenial::Refused("NOT_BOUND"));
            }
        }
        if bound.is_empty() {
            answer.verdicts = verdicts;
            return Ok(answer);
        }
        let locked = tx.lock_matched(probes, &bound).await?;
        for index in &bound {
            verdicts[*index] = AcknowledgementVerdict::Retry;
        }
        let mut winners = Vec::new();
        for row in &locked {
            let task = &tasks[row.index];
            let presented = Presented {
                attempt: task.fence.attempt,
                generation: task.fence.generation,
                step_id: task.step_id,
                token_hash: &probes[row.index].token_hash,
            };
            verdicts[row.index] = match decide(row, &presented, row.now) {
                Decision::Win => {
                    winners.push((row.index, row.marks.generation));
                    // replaced by what the update returns
                    AcknowledgementVerdict::Retry
                }
                Decision::Acknowledged(how) => AcknowledgementVerdict::Acknowledged {
                    how,
                    lease_time_left: non_negative(row.now, row.marks.deadlines.expires_at),
                    attempt_time_left: row
                        .marks
                        .deadlines
                        .attempt_deadline_at
                        .map(|deadline| non_negative(row.now, deadline)),
                },
                Decision::Cancelled(reason) => AcknowledgementVerdict::Cancelled(reason),
                Decision::NotHeld(reason) => AcknowledgementVerdict::NotHeld(reason),
            };
        }
        if !winners.is_empty() {
            let written = tx.acknowledge(probes, &winners).await?;
            if written.len() != winners.len() {
                // the transaction holds every winning row: an update that misses one is a broken store
                return Err(AppError::Internal.into());
            }
            for row in written {
                if let Some(claimed_at) = row.claimed_at {
                    delays.push(non_negative(claimed_at, row.now));
                }
                verdicts[row.index] = AcknowledgementVerdict::Acknowledged {
                    how: Acknowledged::Written,
                    lease_time_left: non_negative(row.now, row.expires_at),
                    attempt_time_left: row
                        .attempt_deadline_at
                        .map(|deadline| non_negative(row.now, deadline)),
                };
            }
        }
        answer.verdicts = verdicts;
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::LeaseDeadlines;

    // any instant: the decision reads only differences from it
    const NOW: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;
    const TOKEN: &[u8] = b"token-hash";
    const OTHER: &[u8] = b"other-hash";

    fn seconds(n: i64) -> time::Duration {
        time::Duration::seconds(n)
    }

    /// A provisional `FORWARD` lease of generation 7, attempt 2, claimed on this server, 30 s before its expiry.
    fn provisional() -> LockedLease {
        LockedLease {
            index: 0,
            step_id: Uuid::from_u128(1),
            leased: true,
            attempt: 2,
            kind: TaskKind::Forward,
            token_hash: Some(TOKEN.to_vec()),
            withdrawn_token_hash: None,
            marks: LeaseMarks {
                deadlines: LeaseDeadlines {
                    expires_at: NOW + seconds(30),
                    attempt_deadline_at: None,
                },
                held: true,
                generation: 7,
                hand_back_generation: Some(7),
                claimed_at: Some(NOW - seconds(1)),
                claimed_on_this_server: true,
            },
            offered_generation: Some(7),
            acknowledged: false,
            cancel_requested: false,
            run_stopping: false,
            now: NOW,
        }
    }

    fn presented() -> Presented<'static> {
        Presented {
            attempt: 2,
            generation: 7,
            step_id: None,
            token_hash: TOKEN,
        }
    }

    fn decided(row: &LockedLease) -> Decision {
        decide(row, &presented(), NOW)
    }

    #[test]
    fn a_provisional_lease_with_its_fence_no_stop_and_the_margin_wins() {
        assert_eq!(decided(&provisional()), Decision::Win);
        let mut with_step = presented();
        with_step.step_id = Some(Uuid::from_u128(1));
        assert_eq!(decide(&provisional(), &with_step, NOW), Decision::Win);
    }

    #[test]
    fn the_margin_is_one_receive_bound_before_both_deadlines_and_exactly_six_seconds_is_enough() {
        let mut row = provisional();
        row.marks.deadlines.expires_at = NOW + seconds(6);
        assert_eq!(decided(&row), Decision::Win, "exactly the margin");
        row.marks.deadlines.expires_at = NOW + seconds(6) - time::Duration::microseconds(1);
        assert_eq!(
            decided(&row),
            Decision::NotHeld(NotHeld::TooLate),
            "before the expiry"
        );
        let mut row = provisional();
        row.marks.deadlines.attempt_deadline_at = Some(NOW + seconds(5));
        assert_eq!(
            decided(&row),
            Decision::NotHeld(NotHeld::TooLate),
            "before the attempt deadline"
        );
        row.marks.deadlines.attempt_deadline_at = Some(NOW + seconds(6));
        assert_eq!(decided(&row), Decision::Win);
    }

    #[test]
    fn a_stop_cancels_a_forward_lease_and_never_a_compensation_or_a_reconciliation() {
        for (cancel, stopping) in [(true, false), (false, true), (true, true)] {
            let mut row = provisional();
            row.cancel_requested = cancel;
            row.run_stopping = stopping;
            assert_eq!(decided(&row), Decision::Cancelled(Cancelled::StopAsked));
            for kind in [TaskKind::Compensation, TaskKind::Reconciliation] {
                row.kind = kind;
                assert_eq!(decided(&row), Decision::Win, "{kind:?}");
            }
        }
        // a stop is judged before the margin
        let mut row = provisional();
        row.cancel_requested = true;
        row.marks.deadlines.expires_at = NOW + seconds(1);
        assert_eq!(decided(&row), Decision::Cancelled(Cancelled::StopAsked));
    }

    #[test]
    fn a_repeated_acknowledgement_is_answered_again_under_the_same_margin_and_stop_rule() {
        let mut row = provisional();
        row.acknowledged = true;
        assert_eq!(
            decided(&row),
            Decision::Acknowledged(Acknowledged::Duplicate)
        );
        row.leased = false; // it heartbeated since: still the host's
        assert_eq!(
            decided(&row),
            Decision::Acknowledged(Acknowledged::Duplicate)
        );
        row.marks.deadlines.expires_at = NOW + seconds(5);
        assert_eq!(decided(&row), Decision::Cancelled(Cancelled::TooLate));
        row.marks.deadlines.expires_at = NOW + seconds(30);
        row.run_stopping = true;
        assert_eq!(decided(&row), Decision::Cancelled(Cancelled::StopAsked));
        row.kind = TaskKind::Compensation;
        assert_eq!(
            decided(&row),
            Decision::Acknowledged(Acknowledged::Duplicate)
        );
    }

    #[test]
    fn a_mark_that_does_not_count_is_an_attempt_recovered_under_the_same_rules() {
        let mut row = provisional();
        row.marks.claimed_on_this_server = false;
        assert_eq!(
            decided(&row),
            Decision::Acknowledged(Acknowledged::Recovered)
        );
        row.acknowledged = true;
        assert_eq!(
            decided(&row),
            Decision::Acknowledged(Acknowledged::Recovered)
        );
        row.marks.deadlines.attempt_deadline_at = Some(NOW + seconds(2));
        assert_eq!(decided(&row), Decision::Cancelled(Cancelled::TooLate));
        row.marks.deadlines.attempt_deadline_at = None;
        row.cancel_requested = true;
        assert_eq!(decided(&row), Decision::Cancelled(Cancelled::StopAsked));
        let mut other_generation = provisional();
        other_generation.marks.hand_back_generation = Some(6);
        assert_eq!(
            decided(&other_generation),
            Decision::Acknowledged(Acknowledged::Recovered)
        );
    }

    #[test]
    fn a_lease_never_offered_is_not_provisional() {
        let mut unmarked = provisional();
        unmarked.offered_generation = None;
        unmarked.marks.hand_back_generation = None;
        assert_eq!(
            decided(&unmarked),
            Decision::NotHeld(NotHeld::NotProvisional)
        );
        let mut reserved_only = provisional();
        reserved_only.offered_generation = None;
        assert_eq!(
            decided(&reserved_only),
            Decision::NotHeld(NotHeld::NotProvisional)
        );
        let mut an_older_offer = provisional();
        an_older_offer.offered_generation = Some(6);
        assert_eq!(
            decided(&an_older_offer),
            Decision::NotHeld(NotHeld::NotProvisional)
        );
        let mut heartbeated_first = provisional();
        heartbeated_first.leased = false;
        assert_eq!(
            decided(&heartbeated_first),
            Decision::NotHeld(NotHeld::NotProvisional)
        );
    }

    #[test]
    fn another_token_attempt_generation_or_step_is_not_held() {
        let mut row = provisional();
        row.token_hash = Some(OTHER.to_vec());
        assert_eq!(decided(&row), Decision::NotHeld(NotHeld::NoMatch));
        row.withdrawn_token_hash = Some(TOKEN.to_vec());
        assert_eq!(decided(&row), Decision::NotHeld(NotHeld::Withdrawn));
        row.token_hash = None;
        assert_eq!(
            decided(&row),
            Decision::NotHeld(NotHeld::Withdrawn),
            "a probe's"
        );
        let mut ended = provisional();
        ended.marks.held = false;
        assert_eq!(decided(&ended), Decision::NotHeld(NotHeld::NoMatch));
        for (attempt, generation) in [(1, 7), (3, 7), (2, 6), (2, 8)] {
            let mut fence = presented();
            fence.attempt = attempt;
            fence.generation = generation;
            assert_eq!(
                decide(&provisional(), &fence, NOW),
                Decision::NotHeld(NotHeld::NoMatch),
                "{attempt}/{generation}"
            );
        }
        let mut step = presented();
        step.step_id = Some(Uuid::from_u128(2));
        assert_eq!(
            decide(&provisional(), &step, NOW),
            Decision::NotHeld(NotHeld::NoMatch)
        );
    }

    #[test]
    fn the_request_carries_one_to_sixty_four_distinct_tasks() {
        assert!(check_request::<&str>(&[]).is_err());
        let ids: Vec<String> = (0..64).map(|i| format!("t{i}")).collect();
        assert!(check_request(&ids).is_ok());
        let too_many: Vec<String> = (0..65).map(|i| format!("t{i}")).collect();
        assert!(check_request(&too_many).is_err());
        assert!(check_request(&["a", "b", "a"]).is_err());
    }

    #[test]
    fn a_deadline_has_time_left_until_it_passes() {
        let start = Instant::now();
        let deadline = CallDeadline::of_arrival(start);
        assert_eq!(deadline.left(start), Some(ACKNOWLEDGEMENT_DEADLINE));
        assert_eq!(deadline.left(start + ACKNOWLEDGEMENT_DEADLINE), None);
        assert_eq!(deadline.left(start + ACKNOWLEDGEMENT_DEADLINE * 2), None);
    }

    #[test]
    fn the_time_left_is_never_below_zero() {
        assert_eq!(non_negative(NOW, NOW - seconds(3)), Duration::ZERO);
        assert_eq!(non_negative(NOW, NOW + seconds(3)), Duration::from_secs(3));
    }

    #[test]
    fn the_labels_are_a_closed_set() {
        use AcknowledgementVerdict as V;
        let lease = Duration::ZERO;
        let all = [
            V::Acknowledged {
                how: Acknowledged::Written,
                lease_time_left: lease,
                attempt_time_left: None,
            },
            V::Acknowledged {
                how: Acknowledged::Duplicate,
                lease_time_left: lease,
                attempt_time_left: None,
            },
            V::Acknowledged {
                how: Acknowledged::Recovered,
                lease_time_left: lease,
                attempt_time_left: None,
            },
            V::NotHeld(NotHeld::NoMatch),
            V::NotHeld(NotHeld::Withdrawn),
            V::NotHeld(NotHeld::TooLate),
            V::NotHeld(NotHeld::NotProvisional),
            V::NotHeld(NotHeld::Unbound),
            V::Cancelled(Cancelled::StopAsked),
            V::Cancelled(Cancelled::TooLate),
            V::Retry,
        ];
        let labels: Vec<_> = all.iter().map(AcknowledgementVerdict::labels).collect();
        assert_eq!(
            labels,
            [
                ("acknowledged", "none"),
                ("duplicate", "none"),
                ("recovered", "none"),
                ("not_held", "no_match"),
                ("not_held", "withdrawn"),
                ("not_held", "too_late"),
                ("not_held", "not_provisional"),
                ("not_held", "unbound"),
                ("cancelled", "stop_asked"),
                ("cancelled", "too_late"),
                ("retry", "none"),
            ]
        );
    }
}

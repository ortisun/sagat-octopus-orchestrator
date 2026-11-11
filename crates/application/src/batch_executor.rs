//! The executor of batch jobs (feature 018, `FR-018-029..031`): the selection of the runs, the
//! application of the action to each in its own step at the rate of the job, the cancellation and the
//! end of a job. One call of [`BatchExecutor::step`] does one thing to one job, and the pass of the
//! scheduler calls it for the jobs of the shards the replica owns.
//!
//! **No step holds two run locks.** The effect on a run is applied by the single-run use case under
//! the key `batch:{job}:{run}` and commits on its own; the outcome of the item, the counters and the
//! next slot of the job commit after it, under the fence of the shard. A crash in between costs a
//! repeat of the effect, which the key makes a no-op, and the next step records the outcome.
//!
//! **The creator is checked at every item**: if the subject that created the job no longer holds
//! `run:batch` and every permission of the action, the job fails `AUTHORITY_REVOKED` and nothing more
//! is applied.
//!
//! **A selection round that fails is counted and bounded** (G2093, D2197): the round rolled back its own work, and the
//! step records the failure in one fenced transaction of its own: the count of consecutive failed rounds goes up and the
//! job waits a backoff (5 seconds doubling to 5 minutes, as a refused item does) before its next round; the eleventh in a
//! row fails the job `SELECTION_FAILED` by the writes `AUTHORITY_REVOKED` uses. A round that picks resets the count. The
//! text of the error is never kept, logged or counted: it can quote a value of the selector. What the operator is given in its
//! place is the SQLSTATE of the error (a closed code, `StoreError::sqlstate`), in the line the step logs with the id of the job and
//! the count (D2203).

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::batch::{
    Action, DeferCause, FailureReason, JobState, Limits, Retry, SkipReason, retry,
};
use sagat_domain::identity::{Permission, Principal, Role, Subject};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::batch::{
    BatchStore, DueJob, ItemOutcome, ItemRecord, JobRecord, NextItem, Selected, SelectionFailure,
    action_permissions,
};
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::{IdentityStore, RequestContext};
use crate::schedules::ShardFence;

/// What a job asks of one run, with everything the single-run use case needs.
#[derive(Debug)]
pub struct ItemRequest<'a> {
    /// The creator of the job, with the role they hold now.
    pub principal: &'a Principal,
    pub job: &'a JobRecord,
    pub run: Uuid,
    /// The idempotency key of the effect: `batch:{job}:{run}`.
    pub key: &'a str,
    /// The signal payload or the update arguments.
    pub payload: Option<&'a [u8]>,
    pub ctx: &'a RequestContext,
}

/// What the action did to the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemEffect {
    Applied {
        /// The run a rerun created.
        new_run: Option<Uuid>,
    },
    /// The action did not apply to the run as it is now.
    Skip(SkipReason),
}

/// The single-run use cases, one per action. Implemented by the run service.
pub trait ItemActions: Send + Sync {
    /// Applies the action of `request.job` to `request.run` through its use case, under
    /// `request.key`. A refusal that means "not for this run" is [`ItemEffect::Skip`]; an
    /// [`AppError`] is a failure, a refusal to defer or the loss of a permission, as the executor
    /// reads it.
    fn apply(
        &self,
        request: &ItemRequest<'_>,
    ) -> impl Future<Output = Result<ItemEffect, AppError>> + Send;
}

/// How the executor selects, pages and retries.
#[derive(Debug, Clone, Copy)]
pub struct ExecutorSettings {
    /// Runs selected in one round.
    pub select_page: u32,
    /// Items cancelled in one drain round.
    pub drain_page: u32,
    /// Retries and their window; the rate and the size of a job come from the job.
    pub limits: Limits,
}

impl Default for ExecutorSettings {
    fn default() -> Self {
        Self {
            select_page: 500,
            drain_page: 500,
            limits: Limits::default(),
        }
    }
}

/// How a step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stepped {
    /// Nothing to do now: the slot has not come, the selection waits for a transaction, or no item is due.
    Idle,
    /// A step was taken.
    Acted,
    /// The lease of the shard is no longer held, or the job moved under this step: nothing changed.
    Refused(&'static str),
    /// A selection round failed and the failure was recorded: the job waits its backoff before the next round (a job that
    /// the failure ended is [`Stepped::Acted`]).
    Backoff,
}

pub struct BatchExecutor<S, A, I, C> {
    store: S,
    actions: A,
    identity: I,
    clock: C,
    settings: ExecutorSettings,
}

/// A closed `Error.code` for a failure the item carries.
fn error_code(error: &AppError) -> &'static str {
    match error {
        AppError::NotAllowed(_)
        | AppError::Invalid(_)
        | AppError::InvalidSchedule { .. }
        | AppError::InvalidBatch { .. }
        | AppError::InvalidDefinition(_)
        | AppError::InvalidStart { .. }
        | AppError::ResetRefused { .. }
        | AppError::InvalidScript(_) => "VALIDATION_FAILED",
        AppError::StateConflict(_)
        | AppError::BusinessIdConflict { .. }
        | AppError::SignalRefused { .. }
        | AppError::Conflict(_)
        | AppError::RevisionConflict { .. }
        | AppError::ScheduleConflict { .. }
        | AppError::BatchConflict { .. }
        | AppError::OutcomeUnresolved => "CONFLICT",
        AppError::IdempotencyKeyReused => "IDEMPOTENCY_KEY_REUSED",
        AppError::PayloadTooLarge(_) => "PAYLOAD_TOO_LARGE",
        AppError::DeclaredUpdate(error) => declared_update_code(error),
        _ => "INTERNAL",
    }
}

/// The `Error.code` the single `updateRun` call would have answered for what an Update refused.
fn declared_update_code(
    error: &crate::declared_operations::DeclaredOperationStoreError,
) -> &'static str {
    use crate::declared_operations::DeclaredOperationStoreError as Refused;
    use sagat_domain::execution::OperationError;
    match error {
        Refused::NotFound | Refused::DefinitionUnavailable => "NOT_FOUND",
        Refused::Forbidden => "FORBIDDEN",
        Refused::AddressUnavailable | Refused::ChainHeadChanged => "CONFLICT",
        Refused::Operation(
            OperationError::InvalidUpdateKey
            | OperationError::InvalidArguments
            | OperationError::EvaluationFailed
            | OperationError::StateSchemaChanged
            | OperationError::ValueTooLarge
            | OperationError::UnsupportedDisclosure,
        ) => "VALIDATION_FAILED",
        Refused::Operation(
            OperationError::UpdateKeyReused
            | OperationError::RunContinued
            | OperationError::RunNotMutable(_)
            | OperationError::RevisionExhausted,
        ) => "CONFLICT",
        Refused::Operation(OperationError::InvalidStoredOperation)
        | Refused::AuditMismatch
        | Refused::Storage(_) => "INTERNAL",
    }
}

impl<S, A, I, C> BatchExecutor<S, A, I, C>
where
    S: BatchStore,
    A: ItemActions,
    I: IdentityStore,
    C: Clock,
{
    pub fn new(store: S, actions: A, identity: I, clock: C, settings: ExecutorSettings) -> Self {
        register_selection_failure_series();
        Self {
            store,
            actions,
            identity,
            clock,
            settings,
        }
    }

    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Does one thing to the job: selects a page of runs, applies the action to the next item, drains
    /// a cancelled job, or ends the job.
    ///
    /// # Errors
    /// [`AppError::Unavailable`] or [`AppError::Internal`] when the store fails.
    pub async fn step(&self, due: &DueJob, fence: Option<ShardFence>) -> Result<Stepped, AppError> {
        let Some(job) = self.store.job(due.tenant_id, due.id).await? else {
            return Ok(Stepped::Idle);
        };
        let now = self.clock.now();
        match job.state {
            JobState::Selecting => self.select(&job, fence).await,
            JobState::Cancelling => self.drain(&job, fence, now).await,
            JobState::Running => self.item(&job, fence, now).await,
            JobState::Completed | JobState::Cancelled | JobState::Failed => Ok(Stepped::Idle),
        }
    }

    fn finished(&self, job: &JobRecord, reason: &'static str, now: OffsetDateTime) -> AuditRecord {
        // the platform ends a job: the record names the engine, not a person
        let engine = Principal::new(job.tenant_id, Subject::engine(), Role::Operator);
        AuditRecord::by(
            &engine,
            AuditAction::BatchFinished,
            ResourceType::BatchJob,
            job.id.to_string(),
            None,
            now,
        )
        .with_reason(reason)
    }

    /// A store refusal that means "the lease or the job moved": the step did not happen.
    fn refused(error: StoreError) -> Result<Stepped, AppError> {
        match error {
            StoreError::StateConflict(reason) => Ok(Stepped::Refused(reason)),
            other => Err(other.into()),
        }
    }

    async fn select(
        &self,
        job: &JobRecord,
        fence: Option<ShardFence>,
    ) -> Result<Stepped, AppError> {
        match self
            .store
            .select_round(
                job.tenant_id,
                job.id,
                fence.as_ref(),
                self.settings.select_page,
            )
            .await
        {
            Ok(Selected::Picked { .. }) => Ok(Stepped::Acted),
            Ok(Selected::Waiting | Selected::NotSelecting) => Ok(Stepped::Idle),
            // the lease or the job moved: the step did not happen, which is not a failure of the round
            Err(StoreError::StateConflict(reason)) => Ok(Stepped::Refused(reason)),
            // The round failed (a statement over its timeout, a deadlock victim, an error that will come again) and rolled
            // back its own work. The text of the error is not read: it can quote a value of the selector (D2203). What is
            // read is its SQLSTATE, a closed code (`StoreError::sqlstate`), which is what the operator is given in its place.
            Err(error) => {
                let sqlstate = error.sqlstate().map(str::to_owned);
                self.record_failed_round(job, fence, sqlstate.as_deref())
                    .await
            }
        }
    }

    /// Records the failed round in a transaction of its own under the fence: the count and the backoff, or, at the bound,
    /// the end of the job `SELECTION_FAILED` by the writes of `fail_job`. Only the id of the job, the number of failures and
    /// the SQLSTATE of the error reach the log, and only one of two closed words the metric. The store decides which of the
    /// two it is, because the count, the wait and the end must be one transaction (D2197); the audit record of the end is
    /// built here, as the other ends of a job are, and the store writes it only when the job ends: one small allocation for a
    /// round that failed, at most a few seconds apart for a job.
    ///
    /// The instant of the end is read **now**, after the round: the round began up to the statement timeout (ten seconds)
    /// before, and a job that ends `SELECTION_FAILED` was read `SELECTING` after the `finishedAt` its record gave when that
    /// was the reading the step began with (found on the compose stack, G2103).
    async fn record_failed_round(
        &self,
        job: &JobRecord,
        fence: Option<ShardFence>,
        sqlstate: Option<&str>,
    ) -> Result<Stepped, AppError> {
        let now = self.clock.now();
        let finished = self.finished(job, FailureReason::SelectionFailed.as_str(), now);
        let sqlstate = sqlstate.unwrap_or("none");
        match self
            .store
            .selection_failed(job.tenant_id, job.id, now, fence.as_ref(), &finished)
            .await
        {
            Ok(SelectionFailure::Retry { failures }) => {
                count_selection_failure("retry");
                tracing::warn!(
                    job = %job.id,
                    failures,
                    sqlstate = %sqlstate,
                    "a batch selection round failed; the job waits before the next one"
                );
                Ok(Stepped::Backoff)
            }
            Ok(SelectionFailure::Failed { failures }) => {
                count_selection_failure("failed");
                tracing::warn!(
                    job = %job.id,
                    failures,
                    sqlstate = %sqlstate,
                    "a batch job failed: its selection rounds kept failing"
                );
                Ok(Stepped::Acted)
            }
            Ok(SelectionFailure::NotSelecting) => Ok(Stepped::Idle),
            Err(error) => Self::refused(error),
        }
    }

    async fn drain(
        &self,
        job: &JobRecord,
        fence: Option<ShardFence>,
        now: OffsetDateTime,
    ) -> Result<Stepped, AppError> {
        let finished = self.finished(job, "CANCELLED", now);
        match self
            .store
            .drain_cancel(
                job.tenant_id,
                job.id,
                self.settings.drain_page,
                now,
                fence.as_ref(),
                &finished,
            )
            .await
        {
            Ok(drained) if drained.cancelled > 0 || drained.finished => {
                count_items(job.action, "cancelled", u64::from(drained.cancelled));
                Ok(Stepped::Acted)
            }
            Ok(_) => Ok(Stepped::Idle),
            Err(error) => Self::refused(error),
        }
    }

    async fn fail(
        &self,
        job: &JobRecord,
        reason: FailureReason,
        fence: Option<ShardFence>,
        now: OffsetDateTime,
    ) -> Result<Stepped, AppError> {
        let finished = self.finished(job, reason.as_str(), now);
        match self
            .store
            .fail_job(
                job.tenant_id,
                job.id,
                reason,
                now,
                fence.as_ref(),
                &finished,
            )
            .await
        {
            Ok(()) => Ok(Stepped::Acted),
            Err(error) => Self::refused(error),
        }
    }

    async fn item(
        &self,
        job: &JobRecord,
        fence: Option<ShardFence>,
        now: OffsetDateTime,
    ) -> Result<Stepped, AppError> {
        // the slot is durable: no item before it, and a job that ran late never catches up in a burst
        if now < job.next_slot_at {
            return Ok(Stepped::Idle);
        }
        // the creator, with what they hold today
        let creator = Subject::parse(&job.created_by).ok();
        let membership = match &creator {
            Some(subject) => {
                self.identity
                    .find_membership(job.tenant_id, subject)
                    .await?
            }
            None => None,
        };
        let allowed = membership.filter(|m| {
            m.role().grants(Permission::RunBatch)
                && action_permissions(job.action)
                    .iter()
                    .all(|needed| m.role().grants(*needed))
        });
        let (Some(creator), Some(membership)) = (creator, allowed) else {
            return self
                .fail(job, FailureReason::AuthorityRevoked, fence, now)
                .await;
        };
        match self.store.next_item(job.tenant_id, job.id, now).await? {
            NextItem::Waiting => Ok(Stepped::Idle),
            NextItem::Drained => {
                let finished = self.finished(job, "COMPLETED", now);
                match self
                    .store
                    .complete_job(job.tenant_id, job.id, now, fence.as_ref(), &finished)
                    .await
                {
                    Ok(()) => Ok(Stepped::Acted),
                    Err(error) => Self::refused(error),
                }
            }
            NextItem::Item(item) => {
                let principal = Principal::new(job.tenant_id, creator, membership.role());
                self.apply(job, &item, &principal, fence, now).await
            }
        }
    }

    async fn apply(
        &self,
        job: &JobRecord,
        item: &ItemRecord,
        principal: &Principal,
        fence: Option<ShardFence>,
        now: OffsetDateTime,
    ) -> Result<Stepped, AppError> {
        let payload = match &job.payload_digest {
            Some(digest) => Some(
                self.store
                    .payload(job.tenant_id, digest)
                    .await?
                    .ok_or(AppError::Internal)?,
            ),
            None => None,
        };
        let key = format!("batch:{}:{}", job.id, item.run_id);
        // a rerun makes a run with no request to take a trace from: the item begins one (D2087)
        let ctx = RequestContext::default().traced();
        let request = ItemRequest {
            principal,
            job,
            run: item.run_id,
            key: &key,
            payload: payload.as_deref(),
            ctx: &ctx,
        };
        // The last look before the run is touched: a replica that lost its lease, a job that was
        // cancelled or an item that has ended since they were read does not apply the action.
        if let Err(error) = self
            .store
            .ensure_live(job.tenant_id, job.id, item.ordinal, fence.as_ref())
            .await
        {
            return Self::refused(error);
        }
        let started = std::time::Instant::now();
        let applied = self.actions.apply(&request).await;
        metrics::histogram!("batch_item_duration_seconds", "action" => job.action.as_str().to_ascii_lowercase())
            .record(started.elapsed().as_secs_f64());
        let outcome = match applied {
            Ok(ItemEffect::Applied { new_run }) => ItemOutcome::Succeeded { new_run },
            Ok(ItemEffect::Skip(reason)) => ItemOutcome::Skipped(reason),
            // the creator lost a permission of the action while the job ran
            Err(AppError::Forbidden { .. }) => {
                return self
                    .fail(job, FailureReason::AuthorityRevoked, fence, now)
                    .await;
            }
            // the run is gone, or is not one the creator may see now
            Err(AppError::NotFound) => ItemOutcome::Skipped(SkipReason::HiddenFromCreator),
            Err(AppError::QuotaExceeded(_)) => self.refusal(item, DeferCause::QuotaExceeded, now),
            Err(AppError::Busy { .. }) => self.refusal(item, DeferCause::RateLimited, now),
            Err(AppError::Unavailable) => self.refusal(item, DeferCause::LockWait, now),
            Err(other) => ItemOutcome::Failed {
                code: error_code(&other),
            },
        };
        let finished = self.finished(job, "COMPLETED", now);
        match self
            .store
            .record_item(
                job.tenant_id,
                job.id,
                item.ordinal,
                &outcome,
                now,
                fence.as_ref(),
                &finished,
            )
            .await
        {
            Ok(_) => {
                count_items(job.action, outcome_label(&outcome), 1);
                Ok(Stepped::Acted)
            }
            Err(error) => Self::refused(error),
        }
    }

    /// An item the platform refused for now is tried again after a bounded backoff, and fails when
    /// it has been refused too often or for too long.
    fn refusal(&self, item: &ItemRecord, cause: DeferCause, now: OffsetDateTime) -> ItemOutcome {
        match retry(
            item.attempts + 1,
            item.first_refused_at.unwrap_or(now),
            now,
            &self.settings.limits,
        ) {
            Retry::After(wait) => ItemOutcome::Deferred {
                cause,
                retry_at: now + wait,
            },
            Retry::Exhausted => ItemOutcome::Failed {
                code: "RETRIES_EXHAUSTED",
            },
        }
    }
}

/// What became of an item, as the closed label of `batch_items_total`.
const fn outcome_label(outcome: &ItemOutcome) -> &'static str {
    match outcome {
        ItemOutcome::Succeeded { .. } => "succeeded",
        ItemOutcome::Skipped(_) => "skipped",
        ItemOutcome::Failed { .. } => "failed",
        ItemOutcome::Deferred { .. } => "deferred",
    }
}

/// Puts the two series of `batch_selection_failures_total` on the page at zero as soon as an executor exists. Prometheus's
/// `increase` does not count the first sample of a series, so a counter that appears with the value 1 has increased by nothing,
/// and `SagatBatchSelectionFailed` (`increase(…{outcome="failed"}[15m]) > 0`) would stay silent for the first job an engine
/// ends `SELECTION_FAILED` (found on the compose stack, G2102). An engine that never fails a selection shows two zeros.
fn register_selection_failure_series() {
    for outcome in ["retry", "failed"] {
        metrics::counter!("batch_selection_failures_total", "outcome" => outcome).absolute(0);
    }
}

/// Counts a failed selection round once its record is committed (`batch_selection_failures_total`). The label is one of two
/// closed words, `retry` (the job waits and tries again) and `failed` (the job ended `SELECTION_FAILED`); none is a tenant, a
/// job, a selector or an error.
fn count_selection_failure(outcome: &'static str) {
    metrics::counter!("batch_selection_failures_total", "outcome" => outcome).increment(1);
}

/// Counts items once their outcome is committed (`batch_items_total`). The labels are the closed
/// sets of the actions and the outcomes; none is a tenant, a job or a run.
fn count_items(action: Action, outcome: &'static str, n: u64) {
    if n > 0 {
        metrics::counter!(
            "batch_items_total",
            "action" => action.as_str().to_ascii_lowercase(),
            "outcome" => outcome
        )
        .increment(n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declared_operations::DeclaredOperationStoreError as Refused;
    use sagat_domain::execution::{OperationError, RunStatus};

    /// A failed `UPDATE` item carries the `Error.code` the single `updateRun` call answers.
    #[test]
    fn a_refused_update_carries_the_code_of_the_single_call() {
        let code = |error| error_code(&AppError::DeclaredUpdate(error));
        assert_eq!(code(Refused::Forbidden), "FORBIDDEN");
        assert_eq!(code(Refused::NotFound), "NOT_FOUND");
        assert_eq!(
            code(Refused::Operation(OperationError::InvalidArguments)),
            "VALIDATION_FAILED"
        );
        assert_eq!(
            code(Refused::Operation(OperationError::UpdateKeyReused)),
            "CONFLICT"
        );
        assert_eq!(
            code(Refused::Operation(OperationError::RunNotMutable(
                RunStatus::Succeeded
            ))),
            "CONFLICT"
        );
        assert_eq!(code(Refused::AuditMismatch), "INTERNAL");
    }
}

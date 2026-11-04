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

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::batch::{DeferCause, FailureReason, JobState, Limits, Retry, SkipReason, retry};
use sagat_domain::identity::{Permission, Principal, Role, Subject};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::batch::{
    BatchStore, DueJob, ItemOutcome, ItemRecord, JobRecord, NextItem, Selected, action_permissions,
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
        | AppError::InvalidScript(_) => "VALIDATION_FAILED",
        AppError::StateConflict(_)
        | AppError::Conflict(_)
        | AppError::RevisionConflict { .. }
        | AppError::ScheduleConflict { .. }
        | AppError::BatchConflict { .. }
        | AppError::OutcomeUnresolved => "CONFLICT",
        AppError::IdempotencyKeyReused => "IDEMPOTENCY_KEY_REUSED",
        AppError::PayloadTooLarge(_) => "PAYLOAD_TOO_LARGE",
        _ => "INTERNAL",
    }
}

impl<S, A, I, C> BatchExecutor<S, A, I, C>
where
    S: BatchStore,
    A: ItemActions,
    I: IdentityStore,
    C: Clock,
{
    pub const fn new(
        store: S,
        actions: A,
        identity: I,
        clock: C,
        settings: ExecutorSettings,
    ) -> Self {
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
            Ok(drained) if drained.cancelled > 0 || drained.finished => Ok(Stepped::Acted),
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
        let ctx = RequestContext::default();
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
        let outcome = match self.actions.apply(&request).await {
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
            Ok(_) => Ok(Stepped::Acted),
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

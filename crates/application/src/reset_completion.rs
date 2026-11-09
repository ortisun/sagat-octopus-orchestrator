//! The completion of the resets that wait for their source run to end (feature 018, US06, `FR-018-038`,
//! `T018-057`): `CANCEL_SOURCE`.
//!
//! One call of [`ResetCompleter::step`] looks at one entry whose source has ended and either creates its run or
//! fails it with a closed reason; the pass of the scheduler calls it for the entries of the shards the replica owns.
//!
//! **Fenced.** The run is created, and the entry completed, in one transaction that checks the lease of the shard
//! that owns the entry and locks the entry: a replica that was slow or cut off finds, where it would commit, that the
//! lease moved or that the entry was completed first, and commits nothing (two engines never complete one entry
//! twice). The failure of an entry is fenced the same way.
//!
//! **An entry whose source does not end expires** (D2059, owner decision OD-2026-11-08-1). An entry still
//! `PENDING_SOURCE` a whole window after its request, while its source is open, ends `FAILED` with the reason
//! [`SOURCE_DID_NOT_END`]: no run is made, the place in the namespace is freed (the count of the namespace is the count
//! of the entries that wait, so ending the entry is what frees it), and the source stays as the request left it,
//! cancelled. It is the same write as any other failure of an entry (one transaction, the fence, the lock of the entry),
//! with two conditions read under that lock: the entry is past the window on the clock the pass is given (the database's,
//! corrected by the engine) and its source is still open and not being changed by anyone. A source that has ended wins,
//! however late: the entry is then completed as always. An entry is never both expired and completed.
//!
//! **The requester is checked at the completion**, with the membership they hold then: a reset needs `run:operate`
//! and `run:start` (and the right to deliver again what it reapplies). Without them the entry fails
//! `AUTHORITY_REVOKED` and nothing is created.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{ResetId, RunId};
use sagat_domain::identity::{Principal, Role, Subject, TenantId};
use sagat_domain::sharding::Shard;
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::{IdentityStore, RequestContext};
use crate::runs::{PendingReset, RESET_ALREADY_COMPLETED};
use crate::schedules::{AUTHORITY_REVOKED, SHARD_LEASE_LOST, ShardFence};

/// What the completion of a pending reset came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// The run was created and the entry is `CREATED`.
    Created(RunId),
    /// The source has not ended: nothing was done, the entry waits.
    SourceOpen,
}

/// The completion of a pending reset through the single-run use case. Implemented by the run service.
pub trait ResetCompletions: Send + Sync {
    /// Creates the run of `entry` with the authority of `principal`, the requester as they are now, under `fence`.
    /// A refusal that is the answer of the platform (the point is not eligible, the requester lost a permission, a
    /// quota) is an [`AppError`] that [`failure_of`] reads; the lease lost or the entry completed first is a
    /// [`AppError::StateConflict`].
    fn complete(
        &self,
        principal: &Principal,
        entry: &PendingReset,
        fence: Option<ShardFence>,
        ctx: &RequestContext,
    ) -> impl Future<Output = Result<Completion, AppError>> + Send;
}

/// A reset whose source has ended and that waits to be completed, as the due scan sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueReset {
    pub tenant_id: TenantId,
    pub id: ResetId,
}

/// The reason an entry that waited for a source that never ended is failed with (D2059): written by the expiry, never by
/// the judgement of the completion, so it is not one of [`FAILURE_REASONS`].
pub const SOURCE_DID_NOT_END: &str = "SOURCE_DID_NOT_END";

/// The expiry of a pending reset, written under the fence of its shard.
#[derive(Debug, Clone, Copy)]
pub struct ResetExpiry<'a> {
    pub tenant_id: TenantId,
    pub id: ResetId,
    /// The entry expires only if it was requested at or before this instant. **The caller decides it**: it is the clock of
    /// the pass (an engine's is the database's, `DatabaseClock`) minus the window, and the store only compares the
    /// `created_at` of the entry with the value it is given, under the lock of the entry (a second reading of the same
    /// column, not a second source of time). This trait is the engine's own and nothing a tenant sends reaches it, so a
    /// caller that computed the instant wrongly would end entries wrongly: the clock the pass is given is what the tests
    /// pin (`an_engine_whose_clock_is_three_hours_off_reads_the_window_on_the_database_clock`).
    pub requested_by: OffsetDateTime,
    /// When the entry ended: the clock of the pass.
    pub at: OffsetDateTime,
    pub fence: Option<&'a ShardFence>,
    /// The audit record of the expiry, committed with it.
    pub audit: &'a AuditRecord,
}

/// What the store did with an expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    /// The entry is `FAILED` with [`SOURCE_DID_NOT_END`], and its audit record is written.
    Expired,
    /// The entry was requested after the instant the caller gave (it is not past its window by the caller's clock):
    /// nothing changed.
    NotDue,
    /// The source has ended: nothing changed, and the entry is completed as always (an ended source wins).
    SourceEnded,
    /// Another transaction holds the row of the source, so it may be ending: nothing changed, the pass looks again in
    /// its next round. The expiry never waits for a run (D2060).
    SourceBusy,
}

/// The failure of a pending reset, written under the fence of its shard.
#[derive(Debug, Clone, Copy)]
pub struct ResetFailure<'a> {
    pub tenant_id: TenantId,
    pub id: ResetId,
    /// One of the closed reasons of the contract.
    pub reason: &'static str,
    pub at: OffsetDateTime,
    pub fence: Option<&'a ShardFence>,
    /// The audit record of the failure, committed with it.
    pub audit: &'a AuditRecord,
}

/// Storage port of the pass that completes pending resets. Implemented by `sagat-infra-postgres`.
pub trait PendingResetStore: Send + Sync {
    /// The pending resets of the shards given whose source run has ended, oldest first, at most `limit` and at most
    /// `per_tenant` of any one tenant (so that the entries of one tenant that fail cannot fill the scan and starve the
    /// others), leaving out those named in `skipping` (the ones the pass is backing off from, D2006). An entry whose
    /// source is still open is not due: it waits, and nothing is read for it.
    fn due_resets(
        &self,
        shards: &[Shard],
        limit: u32,
        per_tenant: u32,
        skipping: &[ResetId],
    ) -> impl Future<Output = Result<Vec<DueReset>, StoreError>> + Send;

    /// The pending resets of the shards given that were requested at or before `requested_by` and whose source is still
    /// open, oldest first, at most `limit` and at most `per_tenant` of any one tenant, leaving out those named in
    /// `skipping` (D2061). It reads the index of the entries that wait, never the register as a whole.
    fn expired_resets(
        &self,
        shards: &[Shard],
        limit: u32,
        per_tenant: u32,
        skipping: &[ResetId],
        requested_by: OffsetDateTime,
    ) -> impl Future<Output = Result<Vec<DueReset>, StoreError>> + Send;

    /// Ends a pending entry whose source did not end in time, in one transaction that checks the fence, locks the entry,
    /// looks at the window and at the source (without waiting for it) and writes the failure and the audit record
    /// the way [`Self::fail_reset`] does. [`StoreError::StateConflict`] when the lease is not held or the entry is not
    /// pending any more.
    fn expire_reset(
        &self,
        expiry: &ResetExpiry<'_>,
    ) -> impl Future<Output = Result<Expiry, StoreError>> + Send;

    /// The entry, if it is still pending.
    fn pending_reset(
        &self,
        tenant: TenantId,
        id: ResetId,
    ) -> impl Future<Output = Result<Option<PendingReset>, StoreError>> + Send;

    /// Marks a pending entry `FAILED` with the reason, in one transaction that checks the fence and locks the entry.
    /// [`StoreError::StateConflict`] when the lease is not held or the entry is not pending any more.
    fn fail_reset(
        &self,
        failure: &ResetFailure<'_>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// How a step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completed {
    /// Nothing to do now: the entry is not pending any more, or its source has not ended.
    Idle,
    /// The run was created.
    Created,
    /// The entry was failed with this closed reason.
    Failed(&'static str),
    /// The entry waited a whole window for a source that did not end and was failed with [`SOURCE_DID_NOT_END`].
    Expired,
    /// The entry is past its window but the row of its source is in the hands of another transaction (which may be ending
    /// the run): nothing changed, and the pass leaves the entry out for a short time instead of taking its slot again at
    /// once (D2081).
    Busy,
    /// The lease of the shard is no longer held, or the entry was completed first: nothing changed.
    Refused(&'static str),
}

/// The closed reasons the completion of a pending reset fails with (the judgement of the platform on the journal the
/// source ended with), and the label each has in `workflow_reset_completions_total{outcome="failed"}`. The expiry has
/// its own, [`SOURCE_DID_NOT_END`], under `outcome="expired"`.
pub const FAILURE_REASONS: [&str; 6] = [
    "BOUNDARY_NO_LONGER_ELIGIBLE",
    "ADMISSION_REFUSED",
    "BUSINESS_ID_IN_USE",
    "AUTHORITY_REVOKED",
    "REEXECUTION_NOT_ACKNOWLEDGED",
    "REAPPLY_TOO_MANY",
];

/// Whether a conflict of state is one of the two races the pass knows: the entry was completed first, or the lease of
/// the shard is no longer held. Nothing changed in either, and the next round finds the entry as it is. Any other
/// conflict is not known to be harmless and is an error to look at (D2006).
#[must_use]
pub fn is_known_race(reason: &str) -> bool {
    reason == RESET_ALREADY_COMPLETED || reason == SHARD_LEASE_LOST
}

/// The reason a pending reset fails with for an error of the completion, or `None` when the error is not an answer
/// of the platform but a failure to retry (unavailability, a limit, storage, wiring, a race). Total over the errors a
/// completion can give: an error that is not listed is retried and never ends an entry. The store errors that the
/// conversion for a client turns into `NotFound` or `NotAllowed` were told apart before it, where the run is made
/// (`store_failure`): what reaches here as `NotFound` is a workflow or a namespace that is gone, and as `NotAllowed` a
/// refusal of the platform to admit the input (D2007).
#[must_use]
pub fn failure_of(error: &AppError) -> Option<&'static str> {
    match error {
        AppError::Forbidden { .. } => Some("AUTHORITY_REVOKED"),
        AppError::StateConflict(reason) if *reason == AUTHORITY_REVOKED => {
            Some("AUTHORITY_REVOKED")
        }
        AppError::ResetRefused { reason, .. } => Some(match *reason {
            "RESET_REEXECUTION_NOT_ACKNOWLEDGED" => "REEXECUTION_NOT_ACKNOWLEDGED",
            "RESET_REAPPLY_TOO_MANY" => "REAPPLY_TOO_MANY",
            // the point, the length of the prefix, the rules of its events, the kind of the source
            _ => "BOUNDARY_NO_LONGER_ELIGIBLE",
        }),
        AppError::BusinessIdConflict { .. } => Some("BUSINESS_ID_IN_USE"),
        AppError::QuotaExceeded(_)
        | AppError::NotAllowed(_)
        | AppError::InvalidStart { .. }
        | AppError::NotFound => Some("ADMISSION_REFUSED"),
        _ => None,
    }
}

/// The completer of pending resets.
pub struct ResetCompleter<S, A, I, C> {
    store: S,
    actions: A,
    identity: I,
    clock: C,
}

impl<S, A, I, C> ResetCompleter<S, A, I, C>
where
    S: PendingResetStore,
    A: ResetCompletions,
    I: IdentityStore,
    C: Clock,
{
    pub const fn new(store: S, actions: A, identity: I, clock: C) -> Self {
        Self {
            store,
            actions,
            identity,
            clock,
        }
    }

    pub const fn store(&self) -> &S {
        &self.store
    }

    /// The time the completer acts by: the database's, corrected by the engine, in a process (D2059).
    #[must_use]
    pub fn now(&self) -> OffsetDateTime {
        self.clock.now()
    }

    /// Ends an entry whose source did not end within the window, or leaves it where it is: `requested_by` is the clock
    /// of the pass minus the window. Nothing is created and the source is not touched.
    ///
    /// # Errors
    /// [`AppError::Unavailable`] or [`AppError::Internal`] when the store fails; the entry is tried again later and is
    /// never recorded as failed for it.
    pub async fn expire(
        &self,
        due: &DueReset,
        requested_by: OffsetDateTime,
        fence: Option<ShardFence>,
    ) -> Result<Completed, AppError> {
        let Some(entry) = self.store.pending_reset(due.tenant_id, due.id).await? else {
            return Ok(Completed::Idle);
        };
        let now = self.clock.now();
        let audit = engine_record(&entry, SOURCE_DID_NOT_END, now);
        let expiry = ResetExpiry {
            tenant_id: entry.tenant_id,
            id: entry.id,
            requested_by,
            at: now,
            fence: fence.as_ref(),
            audit: &audit,
        };
        match self.store.expire_reset(&expiry).await {
            Ok(Expiry::Expired) => {
                count("expired", SOURCE_DID_NOT_END);
                Ok(Completed::Expired)
            }
            Ok(Expiry::SourceBusy) => Ok(Completed::Busy),
            Ok(Expiry::NotDue | Expiry::SourceEnded) => Ok(Completed::Idle),
            Err(StoreError::StateConflict(why)) if is_known_race(why) => {
                Ok(Completed::Refused(why))
            }
            Err(other) => Err(other.into()),
        }
    }

    /// Completes one entry: creates its run, or fails it, or leaves it where it is.
    ///
    /// # Errors
    /// [`AppError::Unavailable`] or [`AppError::Internal`] when the store fails; the entry is tried again.
    pub async fn step(
        &self,
        due: &DueReset,
        fence: Option<ShardFence>,
    ) -> Result<Completed, AppError> {
        let Some(entry) = self.store.pending_reset(due.tenant_id, due.id).await? else {
            return Ok(Completed::Idle);
        };
        let now = self.clock.now();
        // the requester, with what they hold today
        let requester = Subject::parse(&entry.actor).ok();
        let membership = match &requester {
            Some(subject) => {
                self.identity
                    .find_membership(entry.tenant_id, subject)
                    .await?
            }
            None => None,
        };
        let (Some(requester), Some(membership)) = (requester, membership) else {
            return self.fail(&entry, "AUTHORITY_REVOKED", fence, now).await;
        };
        let principal = Principal::new(entry.tenant_id, requester, membership.role());
        // the run this makes has no request to take a trace from: it begins one, or it is never told to the collector (D2087)
        let ctx = RequestContext::default().traced();
        match self
            .actions
            .complete(&principal, &entry, fence.clone(), &ctx)
            .await
        {
            Ok(Completion::Created(_)) => {
                count("created", "none");
                Ok(Completed::Created)
            }
            Ok(Completion::SourceOpen) => Ok(Completed::Idle),
            Err(error) => match failure_of(&error) {
                Some(reason) => self.fail(&entry, reason, fence, now).await,
                None => match error {
                    // the lease is gone, or the entry was completed first: an ordinary race
                    AppError::StateConflict(reason) if is_known_race(reason) => {
                        Ok(Completed::Refused(reason))
                    }
                    // anything else is not known to be harmless: the pass logs it with the ids and backs off
                    other => Err(other),
                },
            },
        }
    }

    async fn fail(
        &self,
        entry: &PendingReset,
        reason: &'static str,
        fence: Option<ShardFence>,
        now: OffsetDateTime,
    ) -> Result<Completed, AppError> {
        let audit = engine_record(entry, reason, now);
        let failure = ResetFailure {
            tenant_id: entry.tenant_id,
            id: entry.id,
            reason,
            at: now,
            fence: fence.as_ref(),
            audit: &audit,
        };
        match self.store.fail_reset(&failure).await {
            Ok(()) => {
                count("failed", reason);
                Ok(Completed::Failed(reason))
            }
            Err(StoreError::StateConflict(why)) if is_known_race(why) => {
                Ok(Completed::Refused(why))
            }
            Err(other) => Err(other.into()),
        }
    }
}

/// The record of the platform ending an entry (a failure, or the expiry): the engine is the actor, not a person, the
/// words are closed (the id of the entry and the reason) and the justification of the requester is never in it. The
/// requester stays the actor of the request, whose records are written with it.
fn engine_record(entry: &PendingReset, reason: &'static str, now: OffsetDateTime) -> AuditRecord {
    let engine = Principal::new(entry.tenant_id, Subject::engine(), Role::Operator);
    AuditRecord::by(
        &engine,
        AuditAction::RunReset,
        ResourceType::Run,
        entry.source.to_string(),
        None,
        now,
    )
    .with_reason(format!("completes reset {}: failed {reason}", entry.id))
}

/// Counts a completion once it is committed (`workflow_reset_completions_total`). The labels are the closed sets
/// of the outcomes (`created`, `failed`, `expired`) and the reasons (`none`, the six of [`FAILURE_REASONS`] and, for
/// `expired` only, [`SOURCE_DID_NOT_END`]); none is a tenant, a run or a reset.
fn count(outcome: &'static str, reason: &'static str) {
    metrics::counter!("workflow_reset_completions_total", "outcome" => outcome, "reason" => reason)
        .increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::identity::Permission;

    #[test]
    fn what_the_requester_lost_ends_the_entry_with_that_reason() {
        for error in [
            AppError::Forbidden {
                permission: Permission::RunStart,
            },
            AppError::StateConflict(AUTHORITY_REVOKED),
        ] {
            assert_eq!(failure_of(&error), Some("AUTHORITY_REVOKED"), "{error:?}");
        }
    }

    #[test]
    fn a_refusal_of_the_reset_is_read_by_what_it_says_about_the_point() {
        let refused = |reason| AppError::ResetRefused {
            reason,
            event_type: None,
        };
        assert_eq!(
            failure_of(&refused("RESET_REEXECUTION_NOT_ACKNOWLEDGED")),
            Some("REEXECUTION_NOT_ACKNOWLEDGED")
        );
        assert_eq!(
            failure_of(&refused("RESET_REAPPLY_TOO_MANY")),
            Some("REAPPLY_TOO_MANY")
        );
        for reason in [
            "RESET_POINT_NOT_ELIGIBLE",
            "RESET_PREFIX_TOO_LONG",
            "RESET_RULE_MISSING",
            "RESET_NOT_SUPPORTED_FOR_SOURCE",
        ] {
            assert_eq!(
                failure_of(&refused(reason)),
                Some("BOUNDARY_NO_LONGER_ELIGIBLE"),
                "{reason}"
            );
        }
    }

    #[test]
    fn the_refusals_of_a_start_are_the_admission_and_the_business_id() {
        assert_eq!(
            failure_of(&AppError::BusinessIdConflict {
                reason: "BUSINESS_ID_IN_USE",
                holder: RunId::generate(),
                reusable_after: None,
            }),
            Some("BUSINESS_ID_IN_USE")
        );
        for error in [
            AppError::QuotaExceeded("active runs in the namespace"),
            AppError::NotAllowed("the run input is larger than 256 KiB"),
            AppError::NotFound,
        ] {
            assert_eq!(failure_of(&error), Some("ADMISSION_REFUSED"), "{error:?}");
        }
    }

    /// What is not an answer of the platform is tried again, never recorded as the outcome of the reset.
    #[test]
    fn a_failure_of_the_platform_or_a_lost_lease_is_not_an_outcome() {
        for error in [
            AppError::Unavailable,
            AppError::Internal,
            AppError::StateConflict("the shard lease is not the one of this schedule"),
            AppError::StateConflict(crate::runs::RESET_ALREADY_COMPLETED),
            AppError::IdempotencyKeyReused,
        ] {
            assert_eq!(failure_of(&error), None, "{error:?}");
        }
    }

    #[test]
    fn every_reason_an_error_maps_to_is_in_the_closed_list() {
        let errors = [
            AppError::Forbidden {
                permission: Permission::RunStart,
            },
            AppError::ResetRefused {
                reason: "RESET_POINT_NOT_ELIGIBLE",
                event_type: None,
            },
            AppError::ResetRefused {
                reason: "RESET_REEXECUTION_NOT_ACKNOWLEDGED",
                event_type: None,
            },
            AppError::ResetRefused {
                reason: "RESET_REAPPLY_TOO_MANY",
                event_type: None,
            },
            AppError::QuotaExceeded("x"),
            AppError::BusinessIdConflict {
                reason: "BUSINESS_ID_IN_USE",
                holder: RunId::generate(),
                reusable_after: None,
            },
        ];
        for error in &errors {
            let reason = failure_of(error).expect("an answer");
            assert!(FAILURE_REASONS.contains(&reason), "{reason}");
        }
    }

    #[test]
    fn only_the_two_known_races_are_taken_for_races() {
        assert!(is_known_race(RESET_ALREADY_COMPLETED));
        assert!(is_known_race(SHARD_LEASE_LOST));
        // the other text the fence can answer is a defect of the caller (a lease of another shard), and a conflict
        // the pass has never heard of is not known to be harmless
        for other in [
            "the shard lease is not the one of this schedule",
            "the schedule changed since the decision was made",
            "",
            "the shard lease is no longer held ",
        ] {
            assert!(!is_known_race(other), "{other:?}");
        }
        // and a conflict is never the end of an entry, whatever it says, unless it is the lost authority
        assert_eq!(
            failure_of(&AppError::StateConflict("a conflict nobody knows")),
            None
        );
        assert_eq!(failure_of(&AppError::StateConflict(SHARD_LEASE_LOST)), None);
    }

    #[test]
    fn what_is_not_an_answer_of_the_platform_never_ends_an_entry() {
        for error in [
            AppError::Unavailable,
            AppError::Internal,
            AppError::IdempotencyKeyReused,
            AppError::OutcomeUnresolved,
        ] {
            assert_eq!(failure_of(&error), None, "{error:?}");
        }
    }
}

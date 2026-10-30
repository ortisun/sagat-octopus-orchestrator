//! The firing of a schedule (feature 018, `FR-018-004..008`, `FR-018-012`, `FR-018-015`): what a
//! pass does with a schedule whose time has come.
//!
//! One call takes one step, and every step is one transaction that commits its decision together
//! with the new state of the schedule, under the shard lease the pass holds and a compare-and-swap on
//! the state the decision was made on. A step that loses either commits nothing; the next call
//! decides again from what is stored. Time is the database's: a schedule is due when the store says
//! so, and every instant here comes from the clock the caller injects, which the engine feeds from
//! the database.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{RunId, RunStatus};
use sagat_domain::identity::{Permission, Principal, Subject};
use sagat_domain::schedules::{
    ActionKind, Ceilings, Decision, Origin, Outcome, Reason, Release, ScheduleId, Standing,
    VersionPolicy, decide, failure_pauses, plan_catch_up, release,
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::clock::Clock;
use crate::definitions::Idempotency;
use crate::error::{AppError, StoreError};
use crate::identity::{IdentityStore, RequestContext};
use crate::schedules::{
    ActionDedupe, ActionView, AutoPause, BackfillProgress, DueBackfill, DueSchedule, ManualRequest,
    Reconcile, RecordedAction, ScheduleAdmission, ScheduleAfter, ScheduleCalendar,
    ScheduleDecision, ScheduleOrigin, ScheduleRecord, ScheduleState, ScheduleStore, ShardFence,
    next_times,
};

/// What a schedule starts, as the run service needs it.
#[derive(Debug, Clone)]
pub struct ScheduledStart<'a> {
    pub namespace: &'a str,
    pub workflow: WorkflowId,
    pub version: Option<WorkflowVersionId>,
    pub input: &'a [u8],
}

/// Starts the run of a schedule's action. Implemented by the run service, which admits the run and
/// commits the action with it ([`ScheduleAdmission`]).
pub trait ScheduledStarts: Send + Sync {
    fn start_scheduled(
        &self,
        principal: &Principal,
        request: &ScheduledStart<'_>,
        admission: ScheduleAdmission,
        ctx: &RequestContext,
    ) -> impl Future<Output = Result<RunId, AppError>> + Send;
}

/// How a call to [`ScheduleFiring::fire`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fired {
    /// Nothing is due, or nothing for this process to do.
    Idle,
    /// A step was taken; the schedule may have more to do.
    Acted,
    /// The step was not taken, for the reason named: another process acted first, the lease was
    /// lost, or the admission was refused and the time stays owed.
    Refused(&'static str),
}

/// `at` kept to the microsecond, the precision of the stored times.
fn micros(at: OffsetDateTime) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(at.unix_timestamp_nanos().div_euclid(1_000) * 1_000)
        .unwrap_or(at)
}

/// How long a refused time waits before it is tried again: as long as it is already late, between
/// 5 seconds and 5 minutes, so the wait grows by doubling and a long refusal ends in the catch-up
/// window rather than in a loop.
fn retry_delay(late: Duration) -> Duration {
    late.clamp(Duration::seconds(5), Duration::minutes(5))
}

/// A request to act on a schedule now (`FR-018-010`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerRequest<'a> {
    /// The overlap rule for this action only; `None` keeps the one of the schedule.
    pub overlap: Option<sagat_domain::schedules::Overlap>,
    /// The `Idempotency-Key` of the request: the same key and request give the same action.
    pub key: &'a str,
}

/// Acts on the schedules whose time has come.
pub struct ScheduleFiring<S, K, C, I, R> {
    store: S,
    calendar: K,
    clock: C,
    identity: I,
    starts: R,
    ceilings: Ceilings,
}

impl<S, K, C, I, R> ScheduleFiring<S, K, C, I, R>
where
    S: ScheduleStore,
    K: ScheduleCalendar,
    C: Clock,
    I: IdentityStore,
    R: ScheduledStarts,
{
    #[must_use]
    pub fn new(
        store: S,
        calendar: K,
        clock: C,
        identity: I,
        starts: R,
        ceilings: Ceilings,
    ) -> Self {
        Self {
            store,
            calendar,
            clock,
            identity,
            starts,
            ceilings: ceilings.clamped(),
        }
    }

    /// The store the firing reads and writes, for the pass that looks for what is due.
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Acts on a schedule now (`FR-018-010`): one `TRIGGERED` action at the time of the request, also
    /// while the schedule is paused. The overlap rule is the schedule's unless the request overrides
    /// it. The action never shares an identity with a scheduled time, never uses up a remaining
    /// action and leaves a time that is owed alone. The run is the schedule's (`startedBy` is
    /// `system:schedule:<id>`) under the role of the caller, so a caller who cannot start runs cannot
    /// make the schedule start one. The same `Idempotency-Key` and request give the same action.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `schedule:operate` and `run:start`; [`AppError::NotFound`];
    /// [`AppError::ScheduleConflict`] for a schedule a cron trigger owns before the handover;
    /// [`AppError::IdempotencyKeyReused`] for a key used by another request; the errors of the
    /// admission of a run (a full quota, a workflow with no published version) when the action
    /// would start one.
    pub async fn trigger(
        &self,
        caller: &Principal,
        id: ScheduleId,
        request: &TriggerRequest<'_>,
        ctx: &RequestContext,
    ) -> Result<ActionView, AppError> {
        caller.authorize(Permission::ScheduleOperate)?;
        caller.authorize(Permission::RunStart)?;
        let idempotency = Idempotency {
            scope: format!("schedule-trigger:{id}"),
            key: request.key.to_owned(),
            fingerprint: match request.overlap {
                None => b"overlap=schedule".to_vec(),
                Some(overlap) => format!("overlap={overlap:?}").into_bytes(),
            },
        };
        if let Some(done) = self
            .store
            .trigger_replay(caller.tenant_id(), &idempotency)
            .await?
        {
            return Ok(done);
        }
        match self
            .trigger_once(caller, id, request, &idempotency, ctx)
            .await
        {
            // a request with the same key, or another at the same instant, committed first: if it
            // was the same request this one is told what it did
            Err(AppError::StateConflict(reason)) => self
                .store
                .trigger_replay(caller.tenant_id(), &idempotency)
                .await?
                .ok_or(AppError::StateConflict(reason)),
            other => other,
        }
    }

    async fn trigger_once(
        &self,
        caller: &Principal,
        id: ScheduleId,
        request: &TriggerRequest<'_>,
        idempotency: &Idempotency,
        ctx: &RequestContext,
    ) -> Result<ActionView, AppError> {
        let record = self
            .store
            .schedule(caller.tenant_id(), id)
            .await?
            .filter(|r| r.state != ScheduleState::Deleted)
            .ok_or(AppError::NotFound)?;
        if record.origin == ScheduleOrigin::Trigger {
            return Err(AppError::ScheduleConflict {
                reason: "SCHEDULE_TRIGGER_HANDOVER_PENDING",
            });
        }
        // the instant is kept to the microsecond: that is what the identity of the action and the
        // stored times are, so a replay is answered with exactly what the first request was
        let now = micros(self.clock.now());
        let nominal = now;
        let standing = Standing {
            open_runs: self.open_runs(&record).await?,
            buffered: record.buffered,
            // a manual action is not limited by what is left
            remaining: None,
        };
        let policies = request
            .overlap
            .map_or(record.policies, |o| record.policies.with_overlap(o));
        let dedupe = ActionDedupe {
            idempotency: idempotency.clone(),
            resource: format!(
                "manual-{}",
                nominal.unix_timestamp_nanos().div_euclid(1_000)
            ),
        };
        let audit = AuditRecord::by(
            caller,
            AuditAction::ScheduleTriggered,
            ResourceType::Schedule,
            record.id.to_string(),
            None,
            now,
        )
        .with_reason("trigger-now");
        // whatever the schedule is about to do next is not touched by a manual action
        let after = ScheduleAfter {
            next_action_at: record.next_action_at,
            next_actual_at: record.next_actual_at,
            remaining_actions: record.remaining_actions,
            extra: Vec::new(),
        };
        match decide(&policies, &standing, &self.ceilings) {
            Decision::Start => {
                let principal = Principal::new(
                    record.tenant_id,
                    Subject::schedule(record.id.as_uuid()),
                    caller.role(),
                );
                let admission = ScheduleAdmission {
                    schedule: record.id,
                    nominal_at: nominal,
                    kind: ActionKind::Triggered,
                    revision: record.revision,
                    expected_state_version: record.state_version,
                    fence: None,
                    after,
                    manual: Some(ManualRequest { dedupe, audit }),
                    backfill: None,
                };
                self.start_run(&record, &principal, admission, ctx).await?;
                self.committed_action(caller, idempotency).await
            }
            // nothing is left to give cannot happen: the count was not given
            Decision::Exhausted => Err(AppError::Internal),
            other
            @ (Decision::Skip { .. } | Decision::Buffer { .. } | Decision::DropBufferFull) => {
                let (outcome, reason) = match other {
                    Decision::Skip { reason } => {
                        (Outcome::SkippedOverlap, reason.map(Reason::as_str))
                    }
                    Decision::Buffer { .. } => (Outcome::Buffered, None),
                    _ => (Outcome::DroppedBufferFull, None),
                };
                let decision = ScheduleDecision {
                    tenant_id: record.tenant_id,
                    schedule: record.id,
                    expected_state_version: record.state_version,
                    fence: None,
                    actions: vec![RecordedAction {
                        nominal_at: nominal,
                        kind: ActionKind::Triggered,
                        revision: record.revision,
                        outcome,
                        reason,
                        summary: None,
                    }],
                    after,
                    pause: None,
                    at: now,
                    dedupe: Some(dedupe),
                    backfill: None,
                };
                self.store.record_decision(&decision, Some(&audit)).await?;
                self.committed_action(caller, idempotency).await
            }
        }
    }

    /// How many of the runs the schedule tracks are still open: an ended one, or one that is gone,
    /// is closed even if the pass has not forgotten it yet.
    async fn open_runs(&self, record: &ScheduleRecord) -> Result<u32, AppError> {
        let statuses = self
            .store
            .run_statuses(record.tenant_id, &record.tracked_runs)
            .await?;
        let open = record
            .tracked_runs
            .iter()
            .filter(|run| {
                statuses
                    .iter()
                    .find(|(s, _)| s == *run)
                    .is_some_and(|(_, status)| !status.is_terminal())
            })
            .count();
        Ok(u32::try_from(open).unwrap_or(u32::MAX))
    }

    /// Takes the next step of a backfill (`FR-018-011`): the first nominal time of the range after
    /// the cursor, acted on under the overlap rule of the backfill, with the cursor and the counters
    /// committed in the same transaction. A time that already has an action is counted
    /// `ALREADY_ACTED` and never gets a second run; a backfill whose buffer is full waits for room
    /// instead of dropping times; one whose requester can no longer start runs is stopped. A crash
    /// resumes after the committed cursor.
    ///
    /// # Errors
    /// [`AppError::Unavailable`] or [`AppError::Internal`] when the store fails.
    pub async fn backfill_step(
        &self,
        due: &DueBackfill,
        fence: Option<ShardFence>,
    ) -> Result<Fired, AppError> {
        let Some(record) = self.store.schedule(due.tenant_id, due.schedule).await? else {
            return Ok(Fired::Idle);
        };
        if record.state == ScheduleState::Deleted {
            return Ok(Fired::Idle);
        }
        let Some(backfill) = self
            .store
            .backfill(due.tenant_id, due.schedule, due.id)
            .await?
        else {
            return Ok(Fired::Idle);
        };
        if !backfill.state.is_active() {
            return Ok(Fired::Idle);
        }
        let now = self.clock.now();
        let system = Subject::schedule(record.id.as_uuid());

        // whose authority the runs are started under: the requester's, today
        let authorized = match Subject::parse(&backfill.requested_by) {
            Ok(subject) => self
                .identity
                .find_membership(record.tenant_id, &subject)
                .await?
                .filter(|m| m.role().grants(Permission::RunStart)),
            Err(_) => None,
        };
        let Some(membership) = authorized else {
            let audit = self.audit(
                &record,
                &system,
                AuditAction::ScheduleBackfillCancelled,
                "authority revoked",
                now,
            );
            self.store
                .cancel_backfill(record.tenant_id, record.id, backfill.id, now, &audit)
                .await?;
            return Ok(Fired::Acted);
        };

        let after = backfill
            .cursor
            .unwrap_or(backfill.start - Duration::milliseconds(1));
        let next = self
            .calendar
            .occurrences(
                &record.spec,
                after,
                Some(backfill.end - Duration::milliseconds(1)),
                1,
            )
            .map_err(|_| AppError::Internal)?;
        let progress = |cursor, outcome, finished| BackfillProgress {
            id: backfill.id,
            cursor,
            outcome,
            finished,
        };
        let decision = |actions, progress| ScheduleDecision {
            tenant_id: record.tenant_id,
            schedule: record.id,
            expected_state_version: record.state_version,
            fence: fence.clone(),
            actions,
            after: ScheduleAfter {
                next_action_at: record.next_action_at,
                next_actual_at: record.next_actual_at,
                remaining_actions: record.remaining_actions,
                extra: Vec::new(),
            },
            pause: None,
            at: now,
            dedupe: None,
            backfill: Some(progress),
        };
        let Some(&time) = next.first() else {
            // no time is left in the range
            let done = progress(backfill.cursor.unwrap_or(backfill.start), None, true);
            return self.committed(
                self.store
                    .record_decision(&decision(Vec::new(), done), None)
                    .await,
            );
        };
        if self
            .store
            .action_exists(record.tenant_id, record.id, Origin::Spec, time)
            .await?
        {
            let seen = progress(time, Some("ALREADY_ACTED"), false);
            return self.committed(
                self.store
                    .record_decision(&decision(Vec::new(), seen), None)
                    .await,
            );
        }
        let standing = Standing {
            open_runs: self.open_runs(&record).await?,
            buffered: record.buffered,
            // a backfill is not limited by what is left
            remaining: None,
        };
        let policies = record.policies.with_overlap(backfill.overlap);
        match decide(&policies, &standing, &self.ceilings) {
            Decision::Start => {
                let principal = Principal::new(record.tenant_id, system.clone(), membership.role());
                let admission = ScheduleAdmission {
                    schedule: record.id,
                    nominal_at: time,
                    kind: ActionKind::Backfill,
                    revision: record.revision,
                    expected_state_version: record.state_version,
                    fence,
                    after: ScheduleAfter {
                        next_action_at: record.next_action_at,
                        next_actual_at: record.next_actual_at,
                        remaining_actions: record.remaining_actions,
                        extra: Vec::new(),
                    },
                    manual: None,
                    backfill: Some(progress(time, Some("STARTED"), false)),
                };
                match self
                    .start_run(&record, &principal, admission, &RequestContext::default())
                    .await
                {
                    Ok(_) => Ok(Fired::Acted),
                    // another process acted first, or the lease is gone: nothing was committed
                    Err(AppError::StateConflict(reason)) => Ok(Fired::Refused(reason)),
                    // the backfill waits and tries this time again
                    Err(
                        AppError::QuotaExceeded(_)
                        | AppError::Unavailable
                        | AppError::Busy { .. }
                        | AppError::NotAllowed(_),
                    ) => Ok(Fired::Refused(
                        "the admission was refused; the backfill waits",
                    )),
                    Err(other) => Err(other),
                }
            }
            // no room to wait in: the backfill waits instead of dropping times
            Decision::DropBufferFull => Ok(Fired::Idle),
            // cannot be: nothing limits a backfill
            Decision::Exhausted => Err(AppError::Internal),
            other @ (Decision::Skip { .. } | Decision::Buffer { .. }) => {
                let (outcome, reason) = match other {
                    Decision::Skip { reason } => {
                        (Outcome::SkippedOverlap, reason.map(Reason::as_str))
                    }
                    _ => (Outcome::Buffered, None),
                };
                let action = RecordedAction {
                    nominal_at: time,
                    kind: ActionKind::Backfill,
                    revision: record.revision,
                    outcome,
                    reason,
                    summary: None,
                };
                let step = progress(time, Some(outcome.as_str()), false);
                self.committed(
                    self.store
                        .record_decision(&decision(vec![action], step), None)
                        .await,
                )
            }
        }
    }

    /// The action a request committed, as it was stored: the first answer and every replay of it are
    /// the same.
    async fn committed_action(
        &self,
        caller: &Principal,
        idempotency: &Idempotency,
    ) -> Result<ActionView, AppError> {
        self.store
            .trigger_replay(caller.tenant_id(), idempotency)
            .await?
            .ok_or(AppError::Internal)
    }

    /// Takes the next step for a due schedule.
    ///
    /// # Errors
    /// [`AppError::Unavailable`] or [`AppError::Internal`] when the store fails; an error of the
    /// admission that is not a refusal to start (the caller logs it and moves on).
    pub async fn fire(
        &self,
        due: &DueSchedule,
        fence: Option<ShardFence>,
    ) -> Result<Fired, AppError> {
        let Some(record) = self.store.schedule(due.tenant_id, due.id).await? else {
            return Ok(Fired::Idle);
        };
        if record.state != ScheduleState::Active {
            return Ok(Fired::Idle);
        }
        let now = self.clock.now();
        let wake = [record.next_actual_at, record.retry_at, record.recheck_at]
            .into_iter()
            .flatten()
            .min();
        if wake.is_none_or(|w| w > now) {
            return Ok(Fired::Idle);
        }
        let system = Subject::schedule(record.id.as_uuid());
        let ctx = RequestContext::default();

        // 1. the runs it tracks that have ended
        let statuses = self
            .store
            .run_statuses(record.tenant_id, &record.tracked_runs)
            .await?;
        // a run that is gone is closed, and pauses nothing
        let mut ended: Vec<(Uuid, RunStatus)> = Vec::new();
        for id in &record.tracked_runs {
            match statuses.iter().find(|(s, _)| s == id) {
                Some((_, status)) if !status.is_terminal() => {}
                Some((_, status)) => ended.push((*id, *status)),
                None => ended.push((*id, RunStatus::Cancelled)),
            }
        }
        let open = record.tracked_runs.len() - ended.len();
        if !ended.is_empty() {
            let failed =
                record.policies.pause_on_failure() && ended.iter().any(|(_, s)| failure_pauses(*s));
            let reconcile = Reconcile {
                tenant_id: record.tenant_id,
                schedule: record.id,
                expected_state_version: record.state_version,
                fence: fence.clone(),
                forget: ended.iter().map(|(id, _)| *id).collect(),
                pause: failed.then(|| AutoPause {
                    note: "a run of the schedule failed".to_owned(),
                }),
                at: now,
            };
            let audit = failed.then(|| {
                self.audit(
                    &record,
                    &system,
                    AuditAction::ScheduleAutoPaused,
                    "a run failed",
                    now,
                )
            });
            return self.committed(
                self.store
                    .reconcile_schedule(&reconcile, audit.as_ref())
                    .await,
            );
        }

        // 2. whose authority every action is checked against, today
        let authorized = match Subject::parse(&record.authorized_by) {
            Ok(subject) => self
                .identity
                .find_membership(record.tenant_id, &subject)
                .await?
                .filter(|m| m.role().grants(Permission::RunStart)),
            Err(_) => None,
        };
        let Some(membership) = authorized else {
            let nominal = record.next_action_at.unwrap_or(now);
            let decision = ScheduleDecision {
                tenant_id: record.tenant_id,
                schedule: record.id,
                expected_state_version: record.state_version,
                fence,
                actions: vec![RecordedAction {
                    nominal_at: nominal,
                    kind: ActionKind::Scheduled,
                    revision: record.revision,
                    outcome: Outcome::AuthorityRevoked,
                    reason: Some("AUTHORITY_REVOKED"),
                    summary: None,
                }],
                after: ScheduleAfter {
                    next_action_at: None,
                    next_actual_at: None,
                    remaining_actions: record.remaining_actions,
                    extra: Vec::new(),
                },
                pause: Some(AutoPause {
                    note: "the authorizing subject can no longer start runs".to_owned(),
                }),
                at: now,
                dedupe: None,
                backfill: None,
            };
            let audit = self.audit(
                &record,
                &system,
                AuditAction::ScheduleAutoPaused,
                "authority revoked",
                now,
            );
            return self.committed(self.store.record_decision(&decision, Some(&audit)).await);
        };
        let principal = Principal::new(record.tenant_id, system.clone(), membership.role());

        // 3. an action that was waiting goes first once nothing is open
        let standing = Standing {
            open_runs: u32::try_from(open).unwrap_or(u32::MAX),
            buffered: record.buffered,
            remaining: record.remaining_actions,
        };
        if release(&standing) == Release::StartOldest
            && let Some(waiting) = self
                .store
                .oldest_buffered(record.tenant_id, record.id)
                .await?
        {
            let admission = ScheduleAdmission {
                schedule: record.id,
                nominal_at: waiting.nominal_at,
                kind: waiting.kind,
                revision: waiting.revision,
                expected_state_version: record.state_version,
                fence,
                after: ScheduleAfter {
                    next_action_at: record.next_action_at,
                    next_actual_at: record.next_actual_at,
                    // only a scheduled action uses one up; a waiting trigger-now does not
                    remaining_actions: record.remaining_actions.map(|n| {
                        n.saturating_sub(u32::from(waiting.kind == ActionKind::Scheduled))
                    }),
                    extra: Vec::new(),
                },
                manual: None,
                backfill: None,
            };
            return self.start(&record, &principal, admission, now, &ctx).await;
        }

        // a waiting action that an open run still holds back is looked at again soon; with none
        // waiting, a recheck that is still set is cleared
        if record.recheck_at.is_some_and(|t| t <= now) {
            let again = (record.buffered > 0).then(|| now + Duration::seconds(5));
            return self.committed(
                self.store
                    .recheck_schedule(record.tenant_id, record.id, record.state_version, again)
                    .await,
            );
        }

        // 4. the times that are due, planned as one recovery
        let time_due = [record.next_actual_at, record.retry_at]
            .into_iter()
            .flatten()
            .min()
            .is_some_and(|t| t <= now);
        let Some(first) = record.next_action_at.filter(|_| time_due) else {
            return Ok(Fired::Idle);
        };
        let ceiling = usize::try_from(self.ceilings.catchup_actions).unwrap_or(usize::MAX);
        let due_times = self
            .calendar
            .occurrences(
                &record.spec,
                first - Duration::milliseconds(1),
                Some(now),
                ceiling
                    .saturating_add(2)
                    .min(crate::schedules::MAX_OCCURRENCES_PER_CALL),
            )
            .map_err(|_| AppError::Internal)?;
        let plan = plan_catch_up(&due_times, now, &record.policies, &self.ceilings);
        if let Some(missed) = plan.missed {
            let next = next_times(&self.calendar, record.id, &record.spec, missed.last)
                .map_err(|_| AppError::Internal)?;
            let decision = ScheduleDecision {
                tenant_id: record.tenant_id,
                schedule: record.id,
                expected_state_version: record.state_version,
                fence,
                actions: vec![RecordedAction {
                    nominal_at: missed.first,
                    kind: ActionKind::Scheduled,
                    revision: record.revision,
                    outcome: Outcome::MissedCatchupWindow,
                    reason: None,
                    summary: Some((missed.count, missed.last)),
                }],
                after: ScheduleAfter {
                    next_action_at: next.0,
                    next_actual_at: next.1,
                    remaining_actions: record.remaining_actions,
                    extra: Vec::new(),
                },
                pause: None,
                at: now,
                dedupe: None,
                backfill: None,
            };
            return self.committed(self.store.record_decision(&decision, None).await);
        }
        let Some(time) = plan.act.first().copied() else {
            return Ok(Fired::Idle);
        };
        let last_act = plan.act.len() == 1;
        let coalesced = if last_act { plan.coalesced } else { None };
        let covers = coalesced.map_or(time, |c| c.last);
        let (next_action_at, next_actual_at) =
            next_times(&self.calendar, record.id, &record.spec, covers)
                .map_err(|_| AppError::Internal)?;
        let extra: Vec<RecordedAction> = coalesced
            .map(|c| RecordedAction {
                nominal_at: c.first,
                kind: ActionKind::Scheduled,
                revision: record.revision,
                outcome: Outcome::Coalesced,
                reason: None,
                summary: Some((c.count, c.last)),
            })
            .into_iter()
            .collect();
        let mut after = ScheduleAfter {
            next_action_at,
            next_actual_at,
            remaining_actions: record.remaining_actions,
            extra,
        };
        match decide(&record.policies, &standing, &self.ceilings) {
            Decision::Start => {
                after.remaining_actions = record.remaining_actions.map(|n| n.saturating_sub(1));
                let admission = ScheduleAdmission {
                    schedule: record.id,
                    nominal_at: time,
                    kind: ActionKind::Scheduled,
                    revision: record.revision,
                    expected_state_version: record.state_version,
                    fence,
                    after,
                    manual: None,
                    backfill: None,
                };
                self.start(&record, &principal, admission, now, &ctx).await
            }
            Decision::Exhausted => {
                // nothing is left to give: the schedule acts as paused, with no next time
                after.next_action_at = None;
                after.next_actual_at = None;
                self.record(&record, fence, Vec::new(), after, now).await
            }
            other
            @ (Decision::Skip { .. } | Decision::Buffer { .. } | Decision::DropBufferFull) => {
                let (outcome, reason) = match other {
                    Decision::Skip { reason } => {
                        (Outcome::SkippedOverlap, reason.map(Reason::as_str))
                    }
                    Decision::Buffer { .. } => (Outcome::Buffered, None),
                    _ => (Outcome::DroppedBufferFull, None),
                };
                let action = RecordedAction {
                    nominal_at: time,
                    kind: ActionKind::Scheduled,
                    revision: record.revision,
                    outcome,
                    reason,
                    summary: None,
                };
                self.record(&record, fence, vec![action], after, now).await
            }
        }
    }

    async fn record(
        &self,
        record: &ScheduleRecord,
        fence: Option<ShardFence>,
        actions: Vec<RecordedAction>,
        after: ScheduleAfter,
        now: OffsetDateTime,
    ) -> Result<Fired, AppError> {
        let decision = ScheduleDecision {
            tenant_id: record.tenant_id,
            schedule: record.id,
            expected_state_version: record.state_version,
            fence,
            actions,
            after,
            pause: None,
            at: now,
            dedupe: None,
            backfill: None,
        };
        self.committed(self.store.record_decision(&decision, None).await)
    }

    /// Starts the run of an action. A refusal to admit it keeps the time owed.
    async fn start(
        &self,
        record: &ScheduleRecord,
        principal: &Principal,
        admission: ScheduleAdmission,
        now: OffsetDateTime,
        ctx: &RequestContext,
    ) -> Result<Fired, AppError> {
        let nominal = admission.nominal_at;
        match self.start_run(record, principal, admission, ctx).await {
            Ok(_) => Ok(Fired::Acted),
            // another process acted first, or the lease is gone: nothing was committed
            Err(AppError::StateConflict(reason)) => Ok(Fired::Refused(reason)),
            // no room, a store that is down, a workflow with no published version: the time stays
            // owed and is tried again under the same key
            Err(
                AppError::QuotaExceeded(_)
                | AppError::Unavailable
                | AppError::Busy { .. }
                | AppError::NotAllowed(_),
            ) => {
                let retry_at = now + retry_delay(now - nominal);
                self.store
                    .defer_schedule(record.tenant_id, record.id, record.state_version, retry_at)
                    .await?;
                Ok(Fired::Refused(
                    "the admission was refused; the time stays owed",
                ))
            }
            Err(other) => Err(other),
        }
    }

    /// Builds the request of the run of an action and sends it to the run service, which admits the
    /// run and commits the action with it.
    async fn start_run(
        &self,
        record: &ScheduleRecord,
        principal: &Principal,
        admission: ScheduleAdmission,
        ctx: &RequestContext,
    ) -> Result<RunId, AppError> {
        let nominal = admission.nominal_at;
        let version = match record.target.version() {
            VersionPolicy::Active => None,
            VersionPolicy::Pinned(id) => Some(id),
        };
        let namespace = self
            .store
            .namespace_slug(record.tenant_id, record.namespace_id)
            .await?
            .ok_or(AppError::Internal)?;
        let input = match record.input_digest {
            Some(digest) => self
                .store
                .input(record.tenant_id, &digest)
                .await?
                .ok_or(AppError::Internal)?,
            None => b"{}".to_vec(),
        };
        let input = with_nominal_time(input, record.target.nominal_time_field(), nominal)?;
        let request = ScheduledStart {
            namespace: &namespace,
            workflow: record.target.workflow(),
            version,
            input: &input,
        };
        self.starts
            .start_scheduled(principal, &request, admission, ctx)
            .await
    }

    fn committed(&self, result: Result<(), StoreError>) -> Result<Fired, AppError> {
        match result {
            Ok(()) => Ok(Fired::Acted),
            Err(StoreError::StateConflict(reason)) => Ok(Fired::Refused(reason)),
            Err(other) => Err(other.into()),
        }
    }

    fn audit(
        &self,
        record: &ScheduleRecord,
        system: &Subject,
        action: AuditAction,
        reason: &str,
        now: OffsetDateTime,
    ) -> AuditRecord {
        let principal = Principal::new(
            record.tenant_id,
            system.clone(),
            sagat_domain::identity::Role::Operator,
        );
        AuditRecord::by(
            &principal,
            action,
            ResourceType::Schedule,
            record.id.to_string(),
            None,
            now,
        )
        .with_reason(reason)
    }
}

/// The input of the run, with the nominal time set in the declared field (RFC 3339, UTC, seconds).
fn with_nominal_time(
    input: Vec<u8>,
    field: Option<&str>,
    nominal: OffsetDateTime,
) -> Result<Vec<u8>, AppError> {
    let Some(field) = field else {
        return Ok(input);
    };
    let mut document: serde_json::Value =
        serde_json::from_slice(&input).map_err(|_| AppError::Internal)?;
    let text = nominal
        .to_offset(time::UtcOffset::UTC)
        .replace_nanosecond(0)
        .map_err(|_| AppError::Internal)?
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| AppError::Internal)?;
    match document.as_object_mut() {
        Some(map) => {
            map.insert(field.to_owned(), serde_json::Value::String(text));
        }
        None => return Err(AppError::Internal),
    }
    serde_json::to_vec(&document).map_err(|_| AppError::Internal)
}

//! Trigger-owned schedules (feature 018, US07, `FR-018-017`, `T018-060`, `T018-061`): the cron triggers of
//! feature 004 made visible and controllable as `TRIGGER` schedules, without changing how they fire.
//!
//! A cron trigger of the version in force has exactly one schedule. The schedule fires the tick the way the legacy cron
//! pass does ([`tick_identity`]: scope `trigger:{id}`, key `cron-{epoch}`, the `cron.fired` input document, run started
//! by `system:engine`), so that while both passes run (`LEGACY`) one tick starts one run and the other records
//! `ALREADY_ACTED`. Publication keeps the schedules equal to the triggers in the publishing transaction; the
//! reconciliation here ([`TriggerReconciler`]) covers what publication did not: an older control plane that published
//! during the upgrade, and the triggers that existed before this feature (it replaces a data migration, which could not
//! compute a next time without the calendar).
//!
//! The pure parts are in this file (the schedule a trigger becomes, the identity of a tick, the document of a firing),
//! so that the adapter that writes them and the pass that fires them cannot disagree about them.

use std::future::Future;

use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::schedules::{
    CatchupMode, Ceilings, MAX_CATCHUP_WINDOW_MS, Overlap, Policies, ScheduleError, ScheduleId,
    ScheduleName, ScheduleSpec, ScheduleTarget, SpecParts, TargetParts, VersionPolicy,
};
use sagat_domain::triggers::CronSpec;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::clock::Clock;
use crate::dispatch::{ClaimedEvent, EventView, event_document};
use crate::error::{AppError, StoreError};
use crate::schedules::{ScheduleCalendar, ShardFence};

/// Where the cron triggers fire from.
///
/// `Legacy` and `HandedOver` are the two values the operator command moves between; a value written by a newer build is
/// read as [`Self::Other`] and never fails a read: the legacy cron pass keeps running (two passes on one tick start one
/// run, and none would lose it) and the control of trigger-owned schedules stays refused (it needs the explicit
/// handover).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoverState {
    Legacy,
    HandedOver,
    /// A state this build does not know, as it was stored.
    Other(String),
}

impl HandoverState {
    /// The state a stored string stands for.
    #[must_use]
    pub fn from_stored(stored: &str) -> Self {
        match stored {
            "LEGACY" => Self::Legacy,
            "HANDED_OVER" => Self::HandedOver,
            other => Self::Other(other.chars().take(64).collect()),
        }
    }

    /// Whether the cron triggers fire from the schedule pass alone: the legacy cron pass stops, and
    /// trigger-owned schedules accept pause, resume, trigger and backfill.
    #[must_use]
    pub const fn is_handed_over(&self) -> bool {
        matches!(self, Self::HandedOver)
    }
}

/// The identity of the run a tick starts: the one feature 004 gave it (`D87`), shared by the legacy cron pass and the
/// schedule pass so that a tick reached by both starts one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickIdentity {
    /// `trigger:{id}`.
    pub scope: String,
    /// `cron-{epoch}`: the nominal time in whole seconds since the epoch.
    pub key: String,
    /// The bytes the key's request hash is made of: the epoch, big-endian.
    pub fingerprint: Vec<u8>,
}

/// The identity of the run of the tick `nominal` of `trigger`.
#[must_use]
pub fn tick_identity(trigger: Uuid, nominal: OffsetDateTime) -> TickIdentity {
    let epoch = nominal.unix_timestamp();
    TickIdentity {
        scope: format!("trigger:{trigger}"),
        key: format!("cron-{epoch}"),
        fingerprint: epoch.to_be_bytes().to_vec(),
    }
}

/// The reason the audit record of a run started by a tick gives, as the legacy pass writes it.
#[must_use]
pub fn tick_reason(trigger: Uuid, nominal: OffsetDateTime) -> String {
    format!(
        "cron trigger {trigger} scheduled for {}",
        ClaimedEvent::rfc3339(nominal)
    )
}

/// The instant as the database keeps it: whole microseconds. A clock reads nanoseconds on one machine and microseconds on
/// another, and an instant that went through the database (the nominal time of a manual trigger) must read the same as
/// one that did not (the instant it was fired), so the document writes both to the microsecond.
fn to_the_microsecond(instant: OffsetDateTime) -> OffsetDateTime {
    instant
        .replace_nanosecond(instant.nanosecond() / 1_000 * 1_000)
        .unwrap_or(instant)
}

/// The input document of the run of a tick: the `cron.fired` event of feature 004, built from the nominal time and the
/// instant it was acted on.
#[must_use]
pub fn cron_document(trigger: Uuid, nominal: OffsetDateTime, fired: OffsetDateTime) -> Vec<u8> {
    let epoch = nominal.unix_timestamp();
    let at = ClaimedEvent::rfc3339(to_the_microsecond(nominal));
    let fired = ClaimedEvent::rfc3339(to_the_microsecond(fired));
    let data = format!(r#"{{"scheduledAt":"{at}","firedAt":"{fired}"}}"#);
    let id = format!("cron:{trigger}:{epoch}");
    event_document(&EventView {
        id: &id,
        source: "cron",
        event_type: "cron.fired",
        subject: None,
        correlation_key: None,
        occurred_at: Some(&at),
        received_at: &fired,
        data: data.as_bytes(),
    })
}

/// What a firing of a trigger-owned schedule needs of its trigger: the version in force, which is the version the legacy
/// pass starts (`workflow_triggers.version_id`, moved by every publication and activation that keeps the trigger).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerSource {
    pub trigger: Uuid,
    pub workflow: WorkflowId,
    pub version: WorkflowVersionId,
}

/// That an admission is the action of a trigger-owned schedule, and what follows from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerTick {
    pub trigger: Uuid,
    /// The trigger's workflow and the version in force: what the action starts, whatever the schedule's own target says.
    pub workflow: WorkflowId,
    pub version: WorkflowVersionId,
    /// The run is the legacy pass's own: started by `system:engine`, audited as the engine and journalled by the
    /// dispatcher. Only a time of the schedule's own (a `SCHEDULED` action) is, because nobody asked for it; a backfill
    /// and a trigger-now are an operator's.
    pub engine: bool,
}

/// Whether `name` is one a trigger-owned schedule has or may fall back to: `trigger-` and 32 hex digits, and optionally `-` and
/// digits. An API schedule may not take it (D2049): a tenant that took the names of a trigger in advance would leave it with
/// no schedule.
#[must_use]
pub fn is_reserved_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("trigger-") else {
        return false;
    };
    let (hex, tail) = rest.split_at(rest.len().min(32));
    hex.len() == 32
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && (tail.is_empty()
            || tail
                .strip_prefix('-')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
}

/// The name of the schedule of `trigger`: derived from its id, so that it is unique and stable and says nothing a tenant
/// wrote.
///
/// # Panics
/// Never: the name is 40 characters of `a-z`, `0-9` and `-`.
#[must_use]
pub fn trigger_schedule_name(trigger: Uuid) -> ScheduleName {
    #[allow(clippy::expect_used)]
    ScheduleName::new(&format!("trigger-{}", trigger.simple()))
        .expect("trigger- and 32 hex digits are a legal schedule name")
}

/// The spec of the schedule of a cron trigger: its expression in its zone, no jitter, and the daylight-saving behaviour
/// of feature 004 (a time that does not exist is skipped, one that happens twice fires once), which the calendar keeps
/// for cron strings whatever the spec says (G1013).
///
/// # Errors
/// [`ScheduleError`] when the zone or the expression is not one a spec accepts.
pub fn trigger_spec(cron: &CronSpec) -> Result<ScheduleSpec, ScheduleError> {
    ScheduleSpec::new(
        SpecParts {
            cron: vec![cron.expression().to_owned()],
            timezone: Some(cron.timezone().to_owned()),
            ..SpecParts::default()
        },
        &Ceilings::default(),
    )
}

/// The policies of a trigger-owned schedule (`FR-018-017`): every tick starts, whatever is open (`ALLOW_ALL`), a missed
/// tick fires once up to 366 days late (`ONE`), and a failed run pauses nothing.
///
/// # Errors
/// Never in practice; the constants are inside the bounds.
pub fn trigger_policies() -> Result<Policies, ScheduleError> {
    Policies::new(
        Overlap::AllowAll,
        MAX_CATCHUP_WINDOW_MS,
        CatchupMode::One,
        false,
        1,
        &Ceilings::default(),
    )
}

/// The target of the schedule of a cron trigger: its workflow, at the version in force. No nominal-time field, no
/// business id, no run timeout: the legacy pass sets none.
///
/// # Errors
/// Never in practice.
pub fn trigger_target(workflow: WorkflowId) -> Result<ScheduleTarget, ScheduleError> {
    ScheduleTarget::new(TargetParts {
        workflow,
        version: VersionPolicy::Active,
        nominal_time_field: None,
        run_timeout_ms: None,
        business_id_prefix: None,
        business_id_conflict: sagat_domain::business_ids::ConflictPolicy::Fail,
        business_id_reuse: sagat_domain::business_ids::ReusePolicy::AllowDuplicate,
    })
}

/// What a process remembers of [`HandoverState`] between rounds: the state is moved by an operator command, once, and
/// never back (the guard of the table refuses it), so `HANDED_OVER` is remembered for good, and `LEGACY` for at most
/// `bound`, after which it is read again. Between the commit of the handover and the next read both cron passes may fire
/// a tick, which starts one run (the shared key); nothing is lost and nothing is started twice.
#[derive(Debug)]
pub struct HandoverCache {
    bound: std::time::Duration,
    seen: std::sync::Mutex<Option<(std::time::Instant, bool)>>,
}

impl HandoverCache {
    /// How long `LEGACY` is believed (D2037).
    pub const LEGACY_BOUND: std::time::Duration = std::time::Duration::from_secs(2);

    /// How long after the commit of the handover the control of a trigger-owned schedule stays refused: the bound above, for
    /// an engine that still believes `LEGACY`, and three seconds of margin for a clock and a round (D2046).
    pub const CONTROL_WAIT_SECONDS: f64 = 5.0;

    #[must_use]
    pub const fn new(bound: std::time::Duration) -> Self {
        Self {
            bound,
            seen: std::sync::Mutex::new(None),
        }
    }

    /// What is remembered at `now`: `Some(handed_over)`, or `None` when it must be read again.
    #[must_use]
    pub fn recall(&self, now: std::time::Instant) -> Option<bool> {
        let seen = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *seen {
            Some((_, true)) => Some(true),
            Some((at, false)) if now.saturating_duration_since(at) < self.bound => Some(false),
            _ => None,
        }
    }

    /// Remembers what was read at `now`.
    pub fn remember(&self, now: std::time::Instant, handed_over: bool) {
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // a handover is never forgotten, whatever a later read says
        if !matches!(*seen, Some((_, true))) {
            *seen = Some((now, handed_over));
        }
    }
}

// --- reconciliation -------------------------------------------------------------------------------------------------

/// A cron trigger of the version in force, as the reconciliation reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerEntry {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub workflow: WorkflowId,
    pub version: WorkflowVersionId,
    pub trigger: Uuid,
    /// The expression and the zone as stored: not parsed until the entry is worked on, so that one row this build cannot read
    /// fails its own entry (which backs off) and not the query that found it (D2044).
    pub cron: String,
    pub timezone: String,
    /// When the legacy pass will fire it next; `None` until it has looked at it.
    pub next_fire_at: Option<OffsetDateTime>,
}

impl TriggerEntry {
    /// The expression and zone of the trigger, parsed.
    ///
    /// # Errors
    /// [`sagat_domain::workflow::WorkflowError`] for a value this build cannot read (one another build wrote).
    pub fn cron_spec(&self) -> Result<CronSpec, sagat_domain::workflow::WorkflowError> {
        CronSpec::parse(&self.cron, &self.timezone)
    }
}

/// One way the schedules differ from the cron triggers in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// A trigger with no schedule: an older control plane published it, or it is older than this feature.
    Missing(TriggerEntry),
    /// A schedule that was removed with its trigger, and the trigger is back (the same id is the same trigger).
    Revived {
        entry: TriggerEntry,
        schedule: ScheduleId,
    },
    /// A schedule whose trigger is gone (an older control plane dropped it).
    Orphan {
        tenant_id: TenantId,
        schedule: ScheduleId,
        trigger: Uuid,
        state_version: u64,
    },
    /// A schedule whose spec is not its trigger's expression and zone.
    Respec {
        entry: TriggerEntry,
        schedule: ScheduleId,
        state_version: u64,
        revision: u32,
    },
    /// An active schedule that is not planned: it lacks the nominal or the actual time the schedule pass acts by, and it is not
    /// exhausted (publication cannot compute a next time: it has no calendar).
    NoNextTime {
        entry: TriggerEntry,
        schedule: ScheduleId,
        state_version: u64,
    },
    /// A paused schedule while the cron triggers still fire from the legacy pass: the API refuses every pause before the
    /// handover, so this is an accident, and a trigger the legacy pass fires must not be left paused when that pass stops
    /// (D2069). After the handover a pause is the operator's and is not drift.
    Paused {
        entry: TriggerEntry,
        schedule: ScheduleId,
        state_version: u64,
    },
}

impl Drift {
    /// The tenant it is in, for the fairness of the pass and for every log line about it.
    #[must_use]
    pub const fn tenant(&self) -> TenantId {
        match self {
            Self::Orphan { tenant_id, .. } => *tenant_id,
            Self::Missing(entry)
            | Self::Revived { entry, .. }
            | Self::Respec { entry, .. }
            | Self::NoNextTime { entry, .. }
            | Self::Paused { entry, .. } => entry.tenant_id,
        }
    }

    /// The trigger it is about: what the work is divided by (the shard of its id) and what a failing entry is known by.
    #[must_use]
    pub const fn trigger(&self) -> Uuid {
        match self {
            Self::Orphan { trigger, .. } => *trigger,
            Self::Missing(entry)
            | Self::Revived { entry, .. }
            | Self::Respec { entry, .. }
            | Self::NoNextTime { entry, .. }
            | Self::Paused { entry, .. } => entry.trigger,
        }
    }
}

/// What the reconciliation writes for one drift, the next time already worked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Create {
        entry: TriggerEntry,
        next: Option<OffsetDateTime>,
    },
    Revive {
        entry: TriggerEntry,
        schedule: ScheduleId,
        next: Option<OffsetDateTime>,
    },
    Remove {
        tenant_id: TenantId,
        schedule: ScheduleId,
        trigger: Uuid,
        expected_state_version: u64,
    },
    Respec {
        entry: TriggerEntry,
        schedule: ScheduleId,
        expected_state_version: u64,
        expected_revision: u32,
        next: Option<OffsetDateTime>,
    },
    SetNext {
        entry: TriggerEntry,
        schedule: ScheduleId,
        expected_state_version: u64,
        next: OffsetDateTime,
    },
    /// The calendar of the trigger has no occurrence at all: the schedule is marked exhausted (no next time, no actions left),
    /// which is a converged state and not one the handover waits for (D2069).
    Exhaust {
        entry: TriggerEntry,
        schedule: ScheduleId,
        expected_state_version: u64,
    },
    /// Resumes a schedule that was paused while the cron triggers still fire from the legacy pass.
    Resume {
        entry: TriggerEntry,
        schedule: ScheduleId,
        expected_state_version: u64,
        next: Option<OffsetDateTime>,
    },
}

/// Where a sweep over the triggers of a replica's shards has got to: the key `(shard of the id, id)` of the last trigger a window
/// covered. The sweep goes in that order, so a window is a range of an index and costs what it covers (D2072).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanCursor {
    pub shard: i16,
    pub trigger: Uuid,
}

/// What a round of the reconciliation looks at: the next `scan` cron triggers after the cursor, among those whose id falls in the
/// shards the replica holds (the work is divided by trigger, because a schedule that does not exist yet has no id), without
/// the entries that are backing off, at most `per_tenant` of each tenant's and `limit` in all, the tenants interleaved. A round
/// costs the window it reads and the writes it makes, not the number of triggers there are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftWindow {
    pub shards: Vec<sagat_domain::sharding::Shard>,
    pub exclude: Vec<Uuid>,
    pub per_tenant: u32,
    pub limit: u32,
    /// Triggers one window covers.
    pub scan: u32,
    /// Where the window starts; `None` at the start of the sweep.
    pub after: Option<ScanCursor>,
}

/// What a window found and where the next one starts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DriftPage {
    pub drift: Vec<Drift>,
    /// The cursor for the next round: the end of this window when everything it found fits the round; this window's own start
    /// when the round's `limit` cut it short (what is written is no longer drift, so the next round finds the rest); `None` when
    /// the window reached the last trigger and the sweep starts over.
    pub next: Option<ScanCursor>,
    /// Whether there are triggers beyond this window: the loop goes on at once instead of waiting for the next poll.
    pub more: bool,
}

/// The storage port of the reconciliation. Implemented by `sagat-infra-postgres`.
pub trait TriggerScheduleStore: Send + Sync {
    /// Where the cron triggers fire from. One row.
    fn handover(&self) -> impl Future<Output = Result<HandoverState, StoreError>> + Send;

    /// How the schedules differ from the cron triggers in force, in one query for the whole window.
    fn drift(
        &self,
        window: &DriftWindow,
    ) -> impl Future<Output = Result<DriftPage, StoreError>> + Send;

    /// How many cron triggers in force have no schedule equal to them over **every** shard, bounded: what the handover requires
    /// to be zero and what the gauge `trigger_schedule_unconverged` reads. It counts exactly what [`Self::drift`] lists for the
    /// triggers (the database has one definition of both, the view `schedule_trigger_drift`, D2069).
    fn convergence(&self) -> impl Future<Output = Result<Convergence, StoreError>> + Send;

    /// The cron triggers a legacy pass fired after the handover settled and after `since` (trigger id and when), at most
    /// twenty: an old engine binary that does not read the handover keeps firing (G2005).
    fn legacy_fires(
        &self,
        since: Option<OffsetDateTime>,
    ) -> impl Future<Output = Result<Vec<(Uuid, OffsetDateTime)>, StoreError>> + Send;

    /// Writes one change in one transaction, under the lease of the shard of the trigger's id and a compare-and-swap on
    /// what the drift was read from. `StateConflict` when the lease is gone or the row changed since (nothing is
    /// written; the next round reads it again).
    fn apply(
        &self,
        change: &Change,
        fence: Option<&ShardFence>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// How far the schedules are from the cron triggers in force.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Convergence {
    pub unconverged: u32,
    /// Up to sixteen of their ids.
    pub examples: Vec<Uuid>,
}

/// What one change did, for the metric that counts it (`created`, `removed`, `repaired`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconciled {
    Created,
    Removed,
    Repaired,
    /// Another process or the lease got there first: nothing was written.
    Refused(&'static str),
}

impl Reconciled {
    /// The closed label of the counter; `None` for what wrote nothing.
    #[must_use]
    pub const fn label(self) -> Option<&'static str> {
        match self {
            Self::Created => Some("created"),
            Self::Removed => Some("removed"),
            Self::Repaired => Some("repaired"),
            Self::Refused(_) => None,
        }
    }
}

/// Turns a drift into a change and writes it.
pub struct TriggerReconciler<S, K, C> {
    store: S,
    calendar: K,
    clock: C,
}

impl<S, K, C> TriggerReconciler<S, K, C>
where
    S: TriggerScheduleStore,
    K: ScheduleCalendar,
    C: Clock,
{
    pub const fn new(store: S, calendar: K, clock: C) -> Self {
        Self {
            store,
            calendar,
            clock,
        }
    }

    pub const fn store(&self) -> &S {
        &self.store
    }

    /// The next time of a trigger's schedule: the time the legacy pass would fire next while it still runs and has
    /// planned one, so that the two passes keep one tick between them; otherwise the first after now, as the legacy pass
    /// plans a trigger it has not looked at.
    fn next(
        &self,
        entry: &TriggerEntry,
        handover: &HandoverState,
    ) -> Result<Option<OffsetDateTime>, AppError> {
        if !handover.is_handed_over()
            && let Some(planned) = entry.next_fire_at
        {
            return Ok(Some(planned));
        }
        let cron = entry.cron_spec().map_err(|_| AppError::Internal)?;
        let spec = trigger_spec(&cron).map_err(|_| AppError::Internal)?;
        self.calendar
            .next_after(&spec, self.clock.now())
            .map_err(|_| AppError::Internal)
    }

    /// Reconciles one drift.
    ///
    /// # Errors
    /// [`AppError::Internal`] for a trigger whose zone or expression this build cannot evaluate (the caller backs the
    /// entry off and says which), and the store's errors.
    pub async fn reconcile(
        &self,
        drift: &Drift,
        handover: &HandoverState,
        fence: Option<&ShardFence>,
    ) -> Result<Reconciled, AppError> {
        let (change, done) = match drift {
            Drift::Missing(entry) => (
                Change::Create {
                    entry: entry.clone(),
                    next: self.next(entry, handover)?,
                },
                Reconciled::Created,
            ),
            Drift::Revived { entry, schedule } => (
                Change::Revive {
                    entry: entry.clone(),
                    schedule: *schedule,
                    next: self.next(entry, handover)?,
                },
                Reconciled::Created,
            ),
            Drift::Orphan {
                tenant_id,
                schedule,
                trigger,
                state_version,
            } => (
                Change::Remove {
                    tenant_id: *tenant_id,
                    schedule: *schedule,
                    trigger: *trigger,
                    expected_state_version: *state_version,
                },
                Reconciled::Removed,
            ),
            Drift::Respec {
                entry,
                schedule,
                state_version,
                revision,
            } => (
                Change::Respec {
                    entry: entry.clone(),
                    schedule: *schedule,
                    expected_state_version: *state_version,
                    expected_revision: *revision,
                    next: self.next(entry, handover)?,
                },
                Reconciled::Repaired,
            ),
            Drift::NoNextTime {
                entry,
                schedule,
                state_version,
            } => {
                // a calendar with no occurrence is not a failure to plan: the trigger never fires, and its schedule says so (D2069)
                let change = match self.next(entry, handover)? {
                    Some(next) => Change::SetNext {
                        entry: entry.clone(),
                        schedule: *schedule,
                        expected_state_version: *state_version,
                        next,
                    },
                    None => Change::Exhaust {
                        entry: entry.clone(),
                        schedule: *schedule,
                        expected_state_version: *state_version,
                    },
                };
                (change, Reconciled::Repaired)
            }
            Drift::Paused {
                entry,
                schedule,
                state_version,
            } => (
                Change::Resume {
                    entry: entry.clone(),
                    schedule: *schedule,
                    expected_state_version: *state_version,
                    next: self.next(entry, handover)?,
                },
                Reconciled::Repaired,
            ),
        };
        match self.store.apply(&change, fence).await {
            Ok(()) => Ok(done),
            Err(StoreError::StateConflict(reason)) => Ok(Reconciled::Refused(reason)),
            Err(other) => Err(other.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(seconds).expect("instant")
    }

    #[test]
    fn the_window_of_the_control_is_longer_than_the_time_an_engine_may_believe_legacy_with_a_stated_margin()
     {
        // an engine reads the state at most every LEGACY_BOUND, so a legacy pass can fire for LEGACY_BOUND after the commit; a
        // round of the pass, a clock and a network need a margin on top of that, and the control must open after both (D2072)
        const MARGIN_SECONDS: f64 = 2.0;
        assert!(
            HandoverCache::LEGACY_BOUND.as_secs_f64() + MARGIN_SECONDS
                <= HandoverCache::CONTROL_WAIT_SECONDS,
            "the control opens {}s after the commit; an engine may fire for {:?} and the margin is {MARGIN_SECONDS}s",
            HandoverCache::CONTROL_WAIT_SECONDS,
            HandoverCache::LEGACY_BOUND
        );
    }

    #[test]
    fn a_tick_is_identified_as_feature_004_identifies_it() {
        let trigger = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let identity = tick_identity(trigger, at(1_793_188_800));
        assert_eq!(identity.scope, format!("trigger:{trigger}"));
        assert_eq!(identity.key, "cron-1793188800");
        assert_eq!(identity.fingerprint, 1_793_188_800_i64.to_be_bytes());
        // sub-second parts of the nominal time never change the tick
        let later = at(1_793_188_800) + time::Duration::milliseconds(999);
        assert_eq!(tick_identity(trigger, later), identity);
        assert_ne!(tick_identity(trigger, at(1_793_188_801)).key, identity.key);
    }

    #[test]
    fn the_document_of_a_tick_is_the_cron_fired_event() {
        let trigger = Uuid::from_u128(7);
        let document: serde_json::Value = serde_json::from_slice(&cron_document(
            trigger,
            at(1_793_188_800),
            at(1_793_188_805),
        ))
        .expect("json");
        assert_eq!(document["event"]["source"], "cron");
        assert_eq!(document["event"]["type"], "cron.fired");
        assert_eq!(
            document["event"]["id"],
            format!("cron:{trigger}:1793188800")
        );
        assert_eq!(document["data"]["scheduledAt"], "2026-10-28T12:00:00Z");
        assert_eq!(document["data"]["firedAt"], "2026-10-28T12:00:05Z");
        assert_eq!(document["event"]["occurredAt"], "2026-10-28T12:00:00Z");
        assert_eq!(document["event"]["receivedAt"], "2026-10-28T12:00:05Z");
    }

    #[test]
    fn the_instants_of_the_document_are_written_to_the_microsecond_the_database_keeps() {
        // The database keeps microseconds. A clock reads nanoseconds on one machine and microseconds on another, so the
        // same instant must be written the same way on both: `scheduledAt` went through the database and `firedAt` did not
        // (CI failed an equality of the two for a manual trigger on Linux, which no macOS run could show).
        let nominal = at(1_793_188_800) + time::Duration::microseconds(835_955);
        let fired = nominal + time::Duration::nanoseconds(300);
        let document: serde_json::Value =
            serde_json::from_slice(&cron_document(Uuid::from_u128(7), nominal, fired))
                .expect("json");
        assert_eq!(
            document["data"]["scheduledAt"],
            "2026-10-28T12:00:00.835955Z"
        );
        assert_eq!(document["data"]["firedAt"], document["data"]["scheduledAt"]);
        assert_eq!(document["event"]["receivedAt"], document["data"]["firedAt"]);
        // and nothing below a microsecond reaches the document from either instant
        let both = cron_document(Uuid::from_u128(7), fired, fired);
        assert_eq!(both, cron_document(Uuid::from_u128(7), nominal, nominal));
    }

    #[test]
    fn the_document_of_a_tick_is_these_exact_bytes() {
        // a literal, so that the legacy pass and the schedule pass (which call the same function) cannot drift together
        let trigger = Uuid::from_u128(0x0194_f5e2_7b1c_7000_8000_0000_0000_00e1);
        let document = cron_document(trigger, at(1_793_188_800), at(1_793_188_803));
        let expected = serde_json::json!({
            "event": {
                "id": format!("cron:{trigger}:1793188800"),
                "source": "cron",
                "type": "cron.fired",
                "subject": null,
                "correlationKey": null,
                "occurredAt": "2026-10-28T12:00:00Z",
                "receivedAt": "2026-10-28T12:00:03Z"
            },
            "data": {"scheduledAt": "2026-10-28T12:00:00Z", "firedAt": "2026-10-28T12:00:03Z"}
        });
        assert_eq!(document, serde_json::to_vec(&expected).expect("json"));
        assert_eq!(
            tick_reason(trigger, at(1_793_188_800)),
            format!("cron trigger {trigger} scheduled for 2026-10-28T12:00:00Z")
        );
        // another input moves every field that depends on it
        let other = cron_document(Uuid::from_u128(9), at(1_793_192_400), at(1_793_192_401));
        let other: serde_json::Value = serde_json::from_slice(&other).expect("json");
        assert_eq!(other["data"]["scheduledAt"], "2026-10-28T13:00:00Z");
        assert_eq!(other["data"]["firedAt"], "2026-10-28T13:00:01Z");
        assert_eq!(
            other["event"]["id"],
            format!("cron:{}:1793192400", Uuid::from_u128(9))
        );
    }

    #[test]
    fn the_schedule_of_a_trigger_has_the_policies_feature_004_fixed() {
        let policies = trigger_policies().expect("policies");
        assert_eq!(policies.overlap(), Overlap::AllowAll);
        assert_eq!(policies.catchup_mode(), CatchupMode::One);
        assert_eq!(policies.catchup_window_ms(), 366 * 24 * 3_600_000);
        assert!(!policies.pause_on_failure());
        let cron = CronSpec::parse("*/5 * * * *", "America/Sao_Paulo").expect("cron");
        let spec = trigger_spec(&cron).expect("spec");
        assert_eq!(spec.timezone(), "America/Sao_Paulo");
        assert_eq!(spec.jitter_ms(), 0);
        assert_eq!(spec.cron().len(), 1);
        assert_eq!(spec.cron()[0].expression(), "*/5 * * * *");
        assert_eq!(
            spec.dst(),
            sagat_domain::schedules::Dst::new(
                sagat_domain::schedules::DstGap::Skip,
                sagat_domain::schedules::DstAmbiguous::OnceEarlier
            )
        );
        let target = trigger_target(WorkflowId::generate()).expect("target");
        assert_eq!(target.nominal_time_field(), None);
        assert!(target.business_id().is_none());
        assert_eq!(target.run_timeout_ms(), None);
    }

    #[test]
    fn the_name_of_a_trigger_schedule_is_derived_from_the_trigger() {
        let a = trigger_schedule_name(Uuid::from_u128(1));
        assert_eq!(a, trigger_schedule_name(Uuid::from_u128(1)));
        assert_ne!(a, trigger_schedule_name(Uuid::from_u128(2)));
        assert_eq!(a.as_str().len(), 40);
        assert!(a.as_str().starts_with("trigger-"));
    }

    #[test]
    fn a_handover_state_this_build_does_not_know_is_read_as_written_and_is_not_handed_over() {
        assert_eq!(HandoverState::from_stored("LEGACY"), HandoverState::Legacy);
        assert!(HandoverState::from_stored("HANDED_OVER").is_handed_over());
        let newer = HandoverState::from_stored("ROLLED_BACK_BY_A_NEWER_BUILD");
        assert!(!newer.is_handed_over());
        assert!(
            matches!(newer, HandoverState::Other(text) if text == "ROLLED_BACK_BY_A_NEWER_BUILD")
        );
        let long = HandoverState::from_stored(&"X".repeat(500));
        assert!(matches!(long, HandoverState::Other(text) if text.len() == 64));
    }

    #[test]
    fn a_handover_is_remembered_for_good_and_legacy_only_for_its_bound() {
        use std::time::{Duration, Instant};
        let cache = HandoverCache::new(Duration::from_secs(2));
        let t0 = Instant::now();
        assert_eq!(cache.recall(t0), None, "nothing read yet");
        cache.remember(t0, false);
        assert_eq!(cache.recall(t0 + Duration::from_millis(1_999)), Some(false));
        assert_eq!(
            cache.recall(t0 + Duration::from_secs(2)),
            None,
            "legacy is read again once its bound has passed"
        );
        cache.remember(t0 + Duration::from_secs(3), true);
        assert_eq!(cache.recall(t0 + Duration::from_secs(3)), Some(true));
        assert_eq!(
            cache.recall(t0 + Duration::from_secs(3_600)),
            Some(true),
            "a handover is never read again"
        );
        cache.remember(t0 + Duration::from_secs(4_000), false);
        assert_eq!(
            cache.recall(t0 + Duration::from_secs(4_000)),
            Some(true),
            "and a later read cannot undo it"
        );
    }

    #[test]
    fn the_names_of_trigger_schedules_are_reserved_and_nothing_else_is() {
        let hex = "0123456789abcdef0123456789abcdef";
        for reserved in [
            format!("trigger-{hex}"),
            format!("trigger-{hex}-1"),
            format!("trigger-{hex}-30"),
        ] {
            assert!(is_reserved_name(&reserved), "{reserved}");
        }
        for free in [
            "trigger-".to_owned(),
            "trigger-happy".to_owned(),
            format!("trigger-{}", &hex[..31]),
            format!("trigger-{hex}0"),
            format!("trigger-{hex}-"),
            format!("trigger-{hex}-x"),
            format!("trigger-{}-1", hex.to_uppercase()),
            format!("my-trigger-{hex}"),
        ] {
            assert!(!is_reserved_name(&free), "{free}");
        }
        assert!(is_reserved_name(
            trigger_schedule_name(Uuid::from_u128(5)).as_str()
        ));
    }

    #[test]
    fn what_a_reconciliation_counts_is_a_closed_set() {
        assert_eq!(Reconciled::Created.label(), Some("created"));
        assert_eq!(Reconciled::Removed.label(), Some("removed"));
        assert_eq!(Reconciled::Repaired.label(), Some("repaired"));
        assert_eq!(Reconciled::Refused("x").label(), None);
    }
}

//! The reconciliation of trigger-owned schedules (feature 018, `FR-018-017`, `T018-060`): keeps the `TRIGGER` schedules
//! equal to the cron triggers in force.
//!
//! Publication does it in the publishing transaction; this pass covers what publication did not: an older control plane
//! that published during the upgrade (it knows nothing of schedules), the triggers that existed before this feature, a
//! trigger whose schedule could not be made, and a schedule that drifted. It runs whether the cron triggers still fire
//! from the legacy pass (`LEGACY`) or from the schedule pass (`HANDED_OVER`).
//!
//! The work is divided by the shard of the trigger's id (a schedule that does not exist yet has no id of its own), one
//! query per round for all the shards the replica holds, every write under the lease of the shard of the trigger's id.
//! It is bounded per round and per tenant, so that a tenant with a great many triggers to repair does not hold the
//! others back, and an entry that fails is left out of the next rounds for a time that doubles, so that one stuck entry
//! (an expression this build cannot evaluate) does not take the place of the others. Every line it logs names the
//! tenant and the trigger.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sagat_application::clock::Clock;
use sagat_application::schedule_triggers::{
    DriftWindow, Reconciled, ScanCursor, TriggerReconciler, TriggerScheduleStore,
};
use sagat_application::schedules::{ScheduleCalendar, ShardFence};
use sagat_domain::sharding::Shard;
use uuid::Uuid;

use crate::resets::delay_after;
use crate::shards::Placement;

/// How a round is bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerSettings {
    /// Entries one round looks at, in all.
    pub batch: u32,
    /// Entries one round takes of one tenant.
    pub per_tenant: u32,
    /// Triggers one round reads, whatever the number of triggers there are (D2072).
    pub scan: u32,
}

impl TriggerSettings {
    /// The hard ceilings: the query is bounded whatever the settings say. A batch of `MAX_BATCH` writes is followed by a pause of
    /// `MAX_BATCH / WRITES_PER_SECOND` seconds, which is the poll of the loop at the most (the loop does not shorten it), so the
    /// rate stays at or under `WRITES_PER_SECOND` for any setting (D2072).
    pub const MAX_BATCH: u32 = 500;
    pub const MAX_PER_TENANT: u32 = 200;
    pub const MAX_SCAN: u32 = 10_000;

    /// The settings held inside their ceilings (at least one entry each).
    #[must_use]
    pub fn effective(self) -> Self {
        Self {
            batch: self.batch.clamp(1, Self::MAX_BATCH),
            per_tenant: self.per_tenant.clamp(1, Self::MAX_PER_TENANT),
            scan: self.scan.clamp(1, Self::MAX_SCAN),
        }
    }
}

impl Default for TriggerSettings {
    fn default() -> Self {
        Self {
            batch: 200,
            per_tenant: 20,
            scan: 2_000,
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TriggerRound {
    /// Entries that differed and were looked at.
    pub drift: usize,
    pub created: u32,
    pub removed: u32,
    pub repaired: u32,
    /// Entries another process or a lost lease got to first, or that have nothing to plan.
    pub refused: u32,
    /// Entries that failed for a reason of the store or the trigger.
    pub failed: u32,
    /// Entries left out of this round because they are backing off after a failure.
    pub backing_off: usize,
    /// The cron triggers in force that have no schedule equal to them, over every shard, when this round measured it (every
    /// sixth round: it is one bounded query and the same for every replica).
    pub unconverged: Option<u32>,
    /// Triggers a legacy cron pass fired after the handover settled (G2005): an engine binary older than the registry release.
    pub legacy_fires: u32,
    /// Whether the window did not reach the last trigger: the sweep goes on at once, without waiting for the poll.
    pub sweeping: bool,
}

impl TriggerRound {
    /// The pause between two windows of a sweep that found nothing to write: long enough that reading a hundred thousand
    /// triggers is about a hundred reads of a range of an index spread over half a minute, not a burst.
    pub const SWEEP_PAUSE: Duration = Duration::from_millis(250);

    /// The most writes a productive round is allowed per second (D2047): an upgrade that leaves a hundred thousand triggers to
    /// reconcile is worked through at this pace at the most, and not as fast as the database lets it. It is a ceiling and not the
    /// pace: the pause that keeps to it is added to the time of the round, so the pace is lower. Measured on one machine
    /// (G2048, two tenants, the engine's own loop, a window of 60 s): 100,000 triggers drained at 62 writes a second under
    /// the default settings (27 minutes) and at 77 with the per-tenant cap of a round raised to 100 (21 minutes, not the
    /// 17 estimated); 10,000 triggers at 30 and at 69 a second (6 and 2 minutes). The smaller backlog is the slower under
    /// the defaults because its sweep reaches the end of the triggers more often, and a round that finds nothing to write
    /// is followed by the poll of 5 s (7 or 8 of them in the 60 s): read from the counts of the rounds, not tested alone.
    pub const WRITES_PER_SECOND: u64 = 100;

    /// What the round wrote.
    #[must_use]
    pub const fn writes(&self) -> u32 {
        self.created + self.removed + self.repaired
    }

    /// How long a loop rests after this round: the time its writes cost at [`Self::WRITES_PER_SECOND`], at least 200 ms for a
    /// round that wrote anything.
    #[must_use]
    pub fn pause_after(&self) -> Duration {
        let cost =
            Duration::from_millis(u64::from(self.writes()) * 1_000 / Self::WRITES_PER_SECOND);
        cost.max(Duration::from_millis(200))
    }
}

impl TriggerRound {
    /// Whether the round wrote something, so the next one should follow soon.
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.created + self.removed + self.repaired > 0
    }
}

#[derive(Debug, Clone, Copy)]
struct Failing {
    attempts: u32,
    next_at: Instant,
}

/// The entries that failed lately, by trigger, and when each may be tried again; bounded in time and in size.
#[derive(Debug, Default)]
struct Failures {
    failing: HashMap<Uuid, Failing>,
}

impl Failures {
    /// The most entries remembered; past it the one whose time comes first is forgotten.
    const MAX_TRACKED: usize = 1_024;

    /// How long an entry that was refused (not failed) is left out.
    const REFUSED_PAUSE: Duration = Duration::from_secs(1);

    fn failed(&mut self, trigger: Uuid, now: Instant) -> u32 {
        let attempts = self
            .failing
            .get(&trigger)
            .map_or(1, |failing| failing.attempts.saturating_add(1));
        if !self.failing.contains_key(&trigger)
            && self.failing.len() >= Self::MAX_TRACKED
            && let Some(soonest) = self
                .failing
                .iter()
                .min_by_key(|(_, failing)| failing.next_at)
                .map(|(id, _)| *id)
        {
            self.failing.remove(&soonest);
        }
        self.failing.insert(
            trigger,
            Failing {
                attempts,
                next_at: now + delay_after(attempts),
            },
        );
        attempts
    }

    /// An entry another process or a lost lease got to first: tried again after a short constant pause. It is not a failure, so its
    /// attempts do not grow and the pause does not double (D2073).
    fn refused(&mut self, trigger: Uuid, now: Instant) {
        let attempts = self.failing.get(&trigger).map_or(0, |f| f.attempts);
        if !self.failing.contains_key(&trigger) && self.failing.len() >= Self::MAX_TRACKED {
            return;
        }
        self.failing.insert(
            trigger,
            Failing {
                attempts,
                next_at: now + Self::REFUSED_PAUSE,
            },
        );
    }

    fn forget(&mut self, trigger: Uuid) {
        self.failing.remove(&trigger);
    }

    fn waiting(&self, now: Instant) -> Vec<Uuid> {
        self.failing
            .iter()
            .filter(|(_, failing)| failing.next_at > now)
            .map(|(id, _)| *id)
            .collect()
    }
}

/// Books what one reconciled entry came to. A write forgets whatever was remembered of the entry. **A refusal is not a failure**:
/// another process or a lost lease got there first, so the entry is left out for a short constant time and its attempts do not
/// grow; only an error doubles the delay (D2073).
fn book(
    failures: &mut Failures,
    round: &mut TriggerRound,
    trigger: Uuid,
    done: Reconciled,
    now: Instant,
) {
    match done {
        Reconciled::Created => round.created += 1,
        Reconciled::Removed => round.removed += 1,
        Reconciled::Repaired => round.repaired += 1,
        Reconciled::Refused(_) => {
            round.refused += 1;
            failures.refused(trigger, now);
            return;
        }
    }
    failures.forget(trigger);
}

/// The pass over the triggers of the shards a replica owns.
pub struct TriggerPass<S, K, C> {
    reconciler: TriggerReconciler<S, K, C>,
    placement: Placement,
    settings: TriggerSettings,
    failures: Mutex<Failures>,
    /// Rounds that ended a sweep so far, to measure the convergence every sixth.
    rounds: AtomicU64,
    /// The last legacy fire this replica has seen: `None` before it has looked after the handover; then the baseline (a fire
    /// before it is history and is not told again after a restart) and, after, the latest it counted (D2074).
    legacy_seen: Mutex<Option<Option<time::OffsetDateTime>>>,
    /// Where the sweep over the triggers has got to.
    cursor: Mutex<Option<ScanCursor>>,
}

impl<S, K, C> TriggerPass<S, K, C>
where
    S: TriggerScheduleStore,
    K: ScheduleCalendar,
    C: Clock,
{
    pub fn new(
        reconciler: TriggerReconciler<S, K, C>,
        placement: Placement,
        settings: TriggerSettings,
    ) -> Self {
        Self {
            reconciler,
            placement,
            settings: settings.effective(),
            failures: Mutex::new(Failures::default()),
            rounds: AtomicU64::new(0),
            legacy_seen: Mutex::new(None),
            cursor: Mutex::new(None),
        }
    }

    /// What the round cannot see through the query that excludes the entries that are backing off: how far the schedules are from
    /// the triggers over every shard (the number the handover requires to be zero), and whether an old engine binary goes on
    /// firing triggers after the handover. Both are bounded queries; a failure is logged and the round goes on. The convergence is
    /// measured every sixth round that ends a sweep (a window in the middle of one does not count), the same number for every
    /// replica.
    async fn observe(&self, round: &mut TriggerRound) {
        let store = self.reconciler.store();
        if !round.sweeping
            && self
                .rounds
                .fetch_add(1, Ordering::Relaxed)
                .is_multiple_of(6)
        {
            match store.convergence().await {
                Ok(convergence) => {
                    round.unconverged = Some(convergence.unconverged);
                    metrics::gauge!("trigger_schedule_unconverged")
                        .set(f64::from(convergence.unconverged));
                    if convergence.unconverged > 0 {
                        tracing::warn!(
                            unconverged = convergence.unconverged,
                            examples = ?convergence.examples,
                            "cron triggers have no schedule equal to them"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "could not measure how far the schedules are from the cron triggers")
                }
            }
        }
        self.watch_legacy(round).await;
    }

    /// Looks for a legacy cron pass that fires after the handover (an engine binary older than the registry release). The first
    /// look after this process started is the baseline: what is there then is history, so a restart does not tell it again and
    /// several replicas do not each replay it. After it, the fires since the latest one seen are counted and told once, with
    /// the trigger ids of the latest few (D2074). The time it reads is the nominal time of the tick (the table keeps no other):
    /// a late tick of a time before the handover is not seen (G2019).
    async fn watch_legacy(&self, round: &mut TriggerRound) {
        let store = self.reconciler.store();
        let seen = *self
            .legacy_seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let since = seen.flatten();
        let fires = match store.legacy_fires(since).await {
            Ok(fires) => fires,
            Err(error) => {
                tracing::warn!(%error, "could not look for legacy cron fires after the handover");
                return;
            }
        };
        let latest = fires.iter().map(|(_, at)| *at).max().or(since);
        match seen {
            None => {
                // the first look: the baseline
                *self
                    .legacy_seen
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(latest);
            }
            Some(_) if !fires.is_empty() => {
                let ids: Vec<Uuid> = fires.iter().take(5).map(|(id, _)| *id).collect();
                tracing::warn!(
                    fired = fires.len(),
                    triggers = ?ids,
                    "a legacy cron pass fired triggers after the handover: an engine binary older than the registry release is running"
                );
                metrics::counter!("cron_legacy_fires_after_handover_total", "source" => "legacy_pass")
                    .increment(u64::try_from(fires.len()).unwrap_or(u64::MAX));
                round.legacy_fires = u32::try_from(fires.len()).unwrap_or(u32::MAX);
                *self
                    .legacy_seen
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(latest);
            }
            Some(_) => {}
        }
    }

    /// Says the loop is alive, now.
    fn stamp() {
        metrics::gauge!("trigger_schedule_last_round_timestamp_seconds").set(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |since| since.as_secs_f64()),
        );
    }

    /// Runs one round. A failure is logged and counted; the next round starts over from what is stored.
    pub async fn round(&self) -> TriggerRound {
        let snapshot = self.placement.snapshot();
        let now = Instant::now();
        let owned = snapshot.shards(now);
        let shards: Vec<Shard> = owned.iter().collect();
        let mut round = TriggerRound::default();
        if shards.is_empty() {
            // a replica that holds no shard has a loop that is alive: the alert on a loop that died reads this, and a replica
            // that has nothing to do is not one (D2075)
            Self::stamp();
            return round;
        }
        let store = self.reconciler.store();
        let handover = match store.handover().await {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(%error, "could not read whether the cron triggers were handed over; the reconciliation of trigger schedules waits");
                round.failed += 1;
                metrics::counter!("trigger_schedule_reconcile_failures_total").increment(1);
                return round;
            }
        };
        metrics::gauge!("trigger_schedule_handed_over").set(if handover.is_handed_over() {
            1.0
        } else {
            0.0
        });
        let exclude = self
            .failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .waiting(now);
        let exclude_len = exclude.len();
        let after = *self
            .cursor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let page = match store
            .drift(&DriftWindow {
                shards,
                exclude,
                per_tenant: self.settings.per_tenant,
                limit: self.settings.batch,
                scan: self.settings.scan,
                after,
            })
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(%error, "could not look for cron triggers and schedules that differ");
                round.failed += 1;
                metrics::counter!("trigger_schedule_reconcile_failures_total").increment(1);
                return round;
            }
        };
        *self
            .cursor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = page.next;
        round.sweeping = page.more;
        let drift = page.drift;
        round.drift = drift.len();
        round.backing_off = exclude_len;
        #[allow(clippy::cast_precision_loss)] // gauges
        {
            metrics::gauge!("trigger_schedule_drift").set(drift.len() as f64);
            metrics::gauge!("trigger_schedule_backing_off").set(exclude_len as f64);
        }
        // when a replica last asked how the schedules differ and was answered: the alert on a loop that died reads it
        Self::stamp();
        self.observe(&mut round).await;
        for drift in &drift {
            let trigger = drift.trigger();
            let Some(lease) = snapshot.lease_of(Shard::of(trigger), now) else {
                continue;
            };
            let fence = ShardFence {
                owner: self.placement.owner().to_owned(),
                lease,
            };
            match self
                .reconciler
                .reconcile(drift, &handover, Some(&fence))
                .await
            {
                Ok(done) => {
                    if let Reconciled::Refused(reason) = done {
                        tracing::debug!(tenant = %drift.tenant(), trigger = %trigger, reason, "a trigger schedule was not reconciled");
                    }
                    book(
                        &mut self
                            .failures
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                        &mut round,
                        trigger,
                        done,
                        now,
                    );
                    if let Some(label) = done.label() {
                        metrics::counter!("trigger_schedules_reconciled_total", "action" => label)
                            .increment(1);
                    }
                }
                Err(error) => {
                    let attempts = self
                        .failures
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .failed(trigger, now);
                    tracing::warn!(
                        tenant = %drift.tenant(),
                        trigger = %trigger,
                        attempts,
                        %error,
                        "a trigger schedule could not be reconciled; the entry is left out for a while"
                    );
                    metrics::counter!("trigger_schedule_reconcile_failures_total").increment(1);
                    round.failed += 1;
                }
            }
        }
        round
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn an_entry_that_fails_is_left_out_for_a_time_that_doubles_and_is_forgotten_when_it_is_done() {
        let mut failures = Failures::default();
        let now = Instant::now();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(failures.failed(a, now), 1);
        assert_eq!(failures.waiting(now), vec![a]);
        assert!(
            failures.waiting(now + Duration::from_secs(2)).is_empty(),
            "the first pause is two seconds"
        );
        assert_eq!(failures.failed(a, now), 2);
        assert_eq!(failures.waiting(now + Duration::from_secs(3)), vec![a]);
        assert_eq!(
            failures.failed(b, now),
            1,
            "another entry has its own count"
        );
        failures.forget(a);
        assert_eq!(failures.waiting(now), vec![b]);
    }

    #[test]
    fn the_failures_remembered_are_bounded() {
        let mut failures = Failures::default();
        let now = Instant::now();
        for n in 0..1_074_u64 {
            failures.failed(
                Uuid::from_u128(u128::from(n)),
                now + Duration::from_millis(n),
            );
        }
        assert_eq!(
            failures.failing.len(),
            1_024,
            "at most 1,024 entries are remembered"
        );
    }

    #[test]
    fn a_productive_round_is_followed_by_a_pause_that_keeps_the_writes_to_a_hundred_a_second() {
        let round = |writes: u32| TriggerRound {
            created: writes,
            ..TriggerRound::default()
        };
        assert_eq!(
            round(0).pause_after(),
            Duration::from_millis(200),
            "never a tight loop"
        );
        assert_eq!(round(1).pause_after(), Duration::from_millis(200));
        assert_eq!(round(20).pause_after(), Duration::from_millis(200));
        assert_eq!(
            round(200).pause_after(),
            Duration::from_secs(2),
            "two hundred writes cost two seconds"
        );
        assert_eq!(round(50).writes(), 50);
        let mixed = TriggerRound {
            created: 100,
            removed: 50,
            repaired: 50,
            ..TriggerRound::default()
        };
        assert_eq!(mixed.pause_after(), Duration::from_secs(2));
    }

    #[test]
    fn the_settings_stay_inside_their_ceilings() {
        let s = TriggerSettings {
            batch: 0,
            per_tenant: 0,
            scan: 0,
        }
        .effective();
        assert_eq!((s.batch, s.per_tenant, s.scan), (1, 1, 1));
        let s = TriggerSettings {
            batch: u32::MAX,
            per_tenant: u32::MAX,
            scan: u32::MAX,
        }
        .effective();
        assert_eq!(
            (s.batch, s.per_tenant, s.scan),
            (
                TriggerSettings::MAX_BATCH,
                TriggerSettings::MAX_PER_TENANT,
                TriggerSettings::MAX_SCAN
            )
        );
        let d = TriggerSettings::default().effective();
        assert_eq!((d.batch, d.per_tenant, d.scan), (200, 20, 2_000));
    }

    #[test]
    fn the_largest_batch_is_followed_by_a_pause_no_longer_than_the_poll_of_the_loop_so_the_rate_holds()
     {
        // the loop waits `pause_after()` and not less (D2072): the pause of the largest batch is what keeps a hundred writes a
        // second, and it must not be so long that the pass seems dead (the poll of the engine is five seconds)
        let largest = TriggerRound {
            created: TriggerSettings::MAX_BATCH,
            ..TriggerRound::default()
        };
        assert!(
            largest.pause_after() <= Duration::from_secs(5),
            "{:?}",
            largest.pause_after()
        );
        assert_eq!(
            largest.pause_after(),
            Duration::from_millis(
                u64::from(TriggerSettings::MAX_BATCH) * 1_000 / TriggerRound::WRITES_PER_SECOND
            ),
            "and it is the pause of a hundred writes a second, whatever the batch"
        );
    }

    /// The two pieces the round uses: `Failures` for the delays and `book` for what an outcome does to them.
    #[test]
    fn a_refused_outcome_is_booked_as_a_refusal_and_never_as_a_failure() {
        let mut failures = Failures::default();
        let mut round = TriggerRound::default();
        let now = Instant::now();
        let a = Uuid::from_u128(7);
        for _ in 0..5 {
            book(
                &mut failures,
                &mut round,
                a,
                Reconciled::Refused("a lost lease"),
                now,
            );
        }
        assert_eq!(round.refused, 5);
        assert_eq!(
            failures.failing.get(&a).map(|f| f.attempts),
            Some(0),
            "five refusals are not five failures: the delay does not grow"
        );
        assert!(failures.waiting(now + Duration::from_secs(1)).is_empty());
        // a write forgets the entry; an error counts
        book(&mut failures, &mut round, a, Reconciled::Repaired, now);
        assert_eq!(round.repaired, 1);
        assert!(failures.failing.is_empty());
        assert_eq!(failures.failed(a, now), 1);
    }

    #[test]
    fn an_entry_that_was_refused_is_left_out_for_a_short_constant_time_and_one_that_failed_for_a_time_that_doubles()
     {
        let mut failures = Failures::default();
        let now = Instant::now();
        let a = Uuid::from_u128(1);
        for _ in 0..6 {
            failures.refused(a, now);
        }
        assert_eq!(failures.waiting(now), vec![a]);
        assert!(
            failures.waiting(now + Duration::from_secs(1)).is_empty(),
            "a refusal is a pause of one second, however often it repeats"
        );
        assert_eq!(
            failures.failed(a, now),
            1,
            "and it is not counted as a failure: the first failure is the first"
        );
        assert!(
            !failures.waiting(now + Duration::from_secs(1)).is_empty(),
            "a failure waits longer"
        );
    }
}

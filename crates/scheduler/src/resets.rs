//! The pass that completes the resets that wait for their source run to end (feature 018, `FR-018-038`,
//! `T018-057`): finds the pending resets of the shards this replica owns whose source has ended and lets
//! [`ResetCompleter`] create the run of each, or fail it.
//!
//! The pass holds no durable state of its own. What it owns is a lease, taken from the keeper's placement, and every
//! entry it completes is committed under that lease by the completer: a replica that was slow or cut off finds, in the
//! transaction that would create the run, that the generation moved (or that another replica completed the entry
//! first) and commits nothing. An entry whose source is still open is not looked at.
//!
//! **An entry that keeps failing does not hold the head of the scan** (D2006). The scan is oldest first, so an entry
//! that errors in every round (a defect, an outage of a dependency) would be first in the next scan, and with a small
//! batch, or with a handful of such entries, nothing behind it would ever be completed. Three things bound that: the scan
//! takes at most `per_tenant` entries of any one tenant; the pass remembers, in memory, the entries that failed and
//! leaves them out of the scan until their time has come, a time that doubles up to a ceiling ([`Backoff`]); and what
//! it remembers is itself bounded. The memory is lost with the process, which then tries every entry again, once.
//! Every line the pass logs names the tenant and the reset (ids are not content).
//!
//! **An entry whose source does not end expires** (D2059, OD-2026-11-08-1). After the entries that are due, the round
//! looks at the entries that have waited a whole window ([`ResetSettings::expiry`], a day by default) for a source that is
//! still open and lets [`ResetCompleter::expire`] fail them with `SOURCE_DID_NOT_END`. The same bounds hold as for the
//! completion: the fence of the shard, at most `batch` entries a round and `per_tenant` of any one tenant, the backoff of
//! an entry that errors, ids in every line. The window is read on the clock of the completer (the database's, corrected
//! by the engine), never on the engine's own, and it runs from the request, so a restart of the engine does not
//! start it again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::identity::IdentityStore;
use sagat_application::reset_completion::{
    Completed, PendingResetStore, ResetCompleter, ResetCompletions, SOURCE_DID_NOT_END,
};
use sagat_application::schedules::{SHARD_LEASE_LOST, ShardFence};
use sagat_domain::execution::ResetId;
use sagat_domain::identity::TenantId;
use sagat_domain::sharding::Shard;

use crate::shards::{Placement, Snapshot};

/// How a pass is tuned.
#[derive(Debug, Clone, Copy)]
pub struct ResetSettings {
    /// Entries completed in one round, across the shards the replica owns (default 100, hard ceiling 1,000), and
    /// entries expired in one round (the same number, counted apart: a round does at most this many of each). The
    /// round stops there and the next one carries on, so a burst is spread over rounds.
    pub batch: u32,
    /// Entries of any one tenant that one round takes (default 10, at least 1 and at most [`Self::MAX_BATCH`]), so
    /// that the entries of one tenant that fail cannot fill the round and starve the others. It bounds the expiry the
    /// same way.
    pub per_tenant: u32,
    /// How long an entry waits for its source to end, counted from its request, before it expires (default 24 hours,
    /// from [`Self::MIN_EXPIRY`] to [`Self::MAX_EXPIRY`], D2059).
    pub expiry: Duration,
}

impl ResetSettings {
    /// The hard ceiling of [`Self::batch`] and of [`Self::per_tenant`].
    pub const MAX_BATCH: u32 = 1_000;
    /// The shortest window of [`Self::expiry`]: an hour.
    pub const MIN_EXPIRY: Duration = Duration::from_secs(60 * 60);
    /// The longest window of [`Self::expiry`]: seven days.
    pub const MAX_EXPIRY: Duration = Duration::from_secs(7 * 24 * 60 * 60);
    /// The window when the deployment does not set one: a day.
    pub const DEFAULT_EXPIRY: Duration = Duration::from_secs(24 * 60 * 60);

    /// The settings held inside their ceilings: above the hard one is lowered, and at least one entry is allowed (a
    /// round that may do nothing would never complete one); a window outside its bounds is brought to the nearest one,
    /// so that an entry is never expired at once by a window that is too short nor held for ever by one that is too
    /// long.
    #[must_use]
    pub fn effective(self) -> Self {
        Self {
            batch: clamp(self.batch),
            per_tenant: clamp(self.per_tenant),
            expiry: self.expiry.clamp(Self::MIN_EXPIRY, Self::MAX_EXPIRY),
        }
    }
}

const fn clamp(value: u32) -> u32 {
    if value == 0 {
        1
    } else if value > ResetSettings::MAX_BATCH {
        ResetSettings::MAX_BATCH
    } else {
        value
    }
}

impl Default for ResetSettings {
    fn default() -> Self {
        Self {
            batch: 100,
            per_tenant: 10,
            expiry: Self::DEFAULT_EXPIRY,
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResetRound {
    /// Entries that were due: pending, with a source that has ended, and not being backed off from.
    pub due: usize,
    /// Entries past their window whose source was still open when the round looked, and not being backed off from
    /// (D2061).
    pub lapsed: usize,
    /// Runs created.
    pub created: u32,
    /// Entries failed with a closed reason.
    pub failed_entries: u32,
    /// Entries that waited a whole window for a source that did not end and were failed for it (D2059).
    pub expired: u32,
    /// Entries past their window whose source was in the hands of another transaction, left out for a short time (D2081).
    pub busy: u32,
    /// Entries another replica or a lost lease took away.
    pub refused: u32,
    /// Entries that could not be completed for a reason of the store or the platform, to be tried again later.
    pub errors: u32,
    /// Entries left out of the scan because they failed lately and their time has not come.
    pub backing_off: usize,
}

impl ResetRound {
    /// How many entries a round may end by expiry per second, on average (D2081).
    pub const EXPIRIES_PER_SECOND: u64 = 100;

    /// Whether the round did something, so the next one should follow at once.
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.created > 0 || self.failed_entries > 0 || self.expired > 0
    }

    /// How long a loop rests after this round before the next one (D2081): nothing for a round that only completed or
    /// failed entries (a backlog of runs to make drains at full speed, as it always did), and for one that expired entries
    /// the time they cost at [`Self::EXPIRIES_PER_SECOND`], at least 200 ms. The caps of a round bound the round and not a
    /// rate, and the first rounds of an engine with this version expire every entry that is older than the window: this is
    /// what keeps those writes to a hundred a second **at the most**, as the other passes keep theirs (D2047, D2072): the
    /// pause is added to the time of the round, so the pace is lower, and it was not measured for this pass (the
    /// reconciliation of trigger schedules, which has the same structure, drained at 30 to 77 a second: G2048).
    #[must_use]
    pub fn pause_after(&self) -> Duration {
        if self.expired == 0 {
            return Duration::ZERO;
        }
        Duration::from_millis(u64::from(self.expired) * 1_000 / Self::EXPIRIES_PER_SECOND)
            .max(Duration::from_millis(200))
    }
}

/// How long an entry that has failed `attempts` times in a row is left out of the scan: the base, doubled for each
/// failure after the first, never more than the ceiling.
#[must_use]
pub fn delay_after(attempts: u32) -> Duration {
    let doublings = attempts.saturating_sub(1).min(16);
    Backoff::BASE
        .saturating_mul(1u32 << doublings)
        .min(Backoff::CEILING)
}

#[derive(Debug, Clone, Copy)]
struct Failing {
    attempts: u32,
    next_at: Instant,
}

/// The entries that failed lately, in memory, and when each may be tried again. Bounded in time (the delay has a
/// ceiling) and in size ([`Self::MAX_TRACKED`]); forgotten when the entry is completed, found completed, or leaves the
/// shards of the replica.
#[derive(Debug, Default)]
pub struct Backoff {
    failing: HashMap<ResetId, Failing>,
}

impl Backoff {
    /// The first delay.
    pub const BASE: Duration = Duration::from_secs(2);
    /// The longest delay: an entry that has failed for a long time is tried every five minutes.
    pub const CEILING: Duration = Duration::from_secs(300);
    /// The most entries remembered. Past it the entry whose time comes first is forgotten (it is tried again soonest
    /// anyway), so the memory and the list of entries left out of a scan are bounded whatever fails.
    pub const MAX_TRACKED: usize = 1_024;
    /// How long an entry whose source another transaction holds is left out of the scan (D2081): short and constant,
    /// and it does not count as a failure.
    pub const BUSY: Duration = Duration::from_secs(1);

    /// The entry may not be tried again before `now + delay`, and its failures do not grow: it was not an error but a
    /// source another transaction holds (D2081). A deferral never shortens what a failure asked for, and it does not
    /// double when repeated.
    pub fn defer(&mut self, id: ResetId, now: Instant, delay: Duration) {
        if !self.failing.contains_key(&id) {
            self.make_room();
        }
        let next_at = now + delay;
        self.failing
            .entry(id)
            .and_modify(|failing| failing.next_at = failing.next_at.max(next_at))
            .or_insert(Failing {
                attempts: 0,
                next_at,
            });
    }

    /// Past [`Self::MAX_TRACKED`] the entry whose time comes first is forgotten, as [`Self::failed`] does.
    fn make_room(&mut self) {
        if self.failing.len() >= Self::MAX_TRACKED
            && let Some(soonest) = self
                .failing
                .iter()
                .min_by_key(|(_, failing)| failing.next_at)
                .map(|(id, _)| *id)
        {
            self.failing.remove(&soonest);
        }
    }

    /// An entry failed at `now`: when it may be tried again, and after how many failures in a row.
    pub fn failed(&mut self, id: ResetId, now: Instant) -> (Duration, u32) {
        let attempts = self
            .failing
            .get(&id)
            .map_or(1, |failing| failing.attempts.saturating_add(1));
        let delay = delay_after(attempts);
        if !self.failing.contains_key(&id) {
            self.make_room();
        }
        self.failing.insert(
            id,
            Failing {
                attempts,
                next_at: now + delay,
            },
        );
        (delay, attempts)
    }

    /// The entry is not failing any more (it was completed, or found completed, or is not ours): forget it.
    pub fn forget(&mut self, id: ResetId) {
        self.failing.remove(&id);
    }

    /// The entries whose time has not come at `now`: the ones a scan leaves out.
    #[must_use]
    pub fn waiting(&self, now: Instant) -> Vec<ResetId> {
        self.failing
            .iter()
            .filter(|(_, failing)| failing.next_at > now)
            .map(|(id, _)| *id)
            .collect()
    }

    /// How many entries are remembered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.failing.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.failing.is_empty()
    }
}

/// The pass over the pending resets of the shards a replica owns.
pub struct ResetPass<S, A, I, C> {
    completer: ResetCompleter<S, A, I, C>,
    placement: Placement,
    settings: ResetSettings,
    backoff: Mutex<Backoff>,
}

impl<S, A, I, C> ResetPass<S, A, I, C>
where
    S: PendingResetStore,
    A: ResetCompletions,
    I: IdentityStore,
    C: Clock,
{
    pub fn new(
        completer: ResetCompleter<S, A, I, C>,
        placement: Placement,
        settings: ResetSettings,
    ) -> Self {
        Self {
            completer,
            placement,
            settings: settings.effective(),
            backoff: Mutex::new(Backoff::default()),
        }
    }

    /// Runs one round. A failure is logged and counted; the next round starts over from what is stored, leaving out
    /// the entries that failed lately.
    pub async fn round(&self) -> ResetRound {
        self.round_at(Instant::now()).await
    }

    fn backoff(&self) -> std::sync::MutexGuard<'_, Backoff> {
        // a panic while the memory was held leaves it as it was: it is only a list of times
        self.backoff
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// As [`Self::round`], at the instant given (the instants of leases and of the backoff are this one).
    pub async fn round_at(&self, now: Instant) -> ResetRound {
        let snapshot = self.placement.snapshot();
        let shards: Vec<Shard> = snapshot.shards(now).iter().collect();
        let mut round = ResetRound::default();
        if shards.is_empty() {
            return round;
        }
        let waiting = self.backoff().waiting(now);
        round.backing_off = waiting.len();
        let due = match self
            .completer
            .store()
            .due_resets(
                &shards,
                self.settings.batch,
                self.settings.per_tenant,
                &waiting,
            )
            .await
        {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(%error, "could not look for pending resets that are due");
                round.errors += 1;
                return round;
            }
        };
        round.due = due.len();
        for entry in due {
            let (tenant_id, id) = (entry.tenant_id, entry.id);
            let Some(fence) = self.fence_of(&snapshot, id, now) else {
                continue;
            };
            let result = self.completer.step(&entry, Some(fence)).await;
            self.settle(tenant_id, id, result, now, &mut round);
        }
        self.expire(&snapshot, &shards, now, &mut round).await;
        round
    }

    /// The entries whose source did not end within the window (D2059): after the entries that are due, so that an
    /// ended source always wins, and with the same fence, the same per-tenant share, the same backoff and the same
    /// bounded round as the completion. The window is read on the clock of the completer, which an engine gives the
    /// database's.
    async fn expire(
        &self,
        snapshot: &Snapshot,
        shards: &[Shard],
        now: Instant,
        round: &mut ResetRound,
    ) {
        let window = time::Duration::seconds(
            i64::try_from(self.settings.expiry.as_secs()).unwrap_or(i64::MAX),
        );
        let requested_by = self.completer.now().saturating_sub(window);
        // read again, so that an entry that failed in the completion stage of this round is left out of this scan too
        let waiting = self.backoff().waiting(now);
        let lapsed = match self
            .completer
            .store()
            .expired_resets(
                shards,
                self.settings.batch,
                self.settings.per_tenant,
                &waiting,
                requested_by,
            )
            .await
        {
            Ok(lapsed) => lapsed,
            Err(error) => {
                tracing::warn!(%error, "could not look for pending resets whose source did not end in time");
                round.errors += 1;
                return;
            }
        };
        round.lapsed = lapsed.len();
        for entry in lapsed {
            let (tenant_id, id) = (entry.tenant_id, entry.id);
            let Some(fence) = self.fence_of(snapshot, id, now) else {
                continue;
            };
            let result = self
                .completer
                .expire(&entry, requested_by, Some(fence))
                .await;
            self.settle(tenant_id, id, result, now, round);
        }
    }

    /// The fence of the lease the replica holds on the shard that owns the entry, if it holds it at `now`.
    fn fence_of(&self, snapshot: &Snapshot, id: ResetId, now: Instant) -> Option<ShardFence> {
        let lease = snapshot.lease_of(Shard::of(id.as_uuid()), now)?;
        Some(ShardFence {
            owner: self.placement.owner().to_owned(),
            lease,
        })
    }

    /// What a step on one entry came to: counted in the round, logged with the ids (ids are not content) and, for an
    /// error that is not an answer, left to the backoff.
    fn settle(
        &self,
        tenant_id: TenantId,
        id: ResetId,
        result: Result<Completed, AppError>,
        now: Instant,
        round: &mut ResetRound,
    ) {
        match result {
            Ok(Completed::Created) => {
                self.backoff().forget(id);
                round.created += 1;
            }
            Ok(Completed::Failed(reason)) => {
                self.backoff().forget(id);
                tracing::info!(%tenant_id, reset_id = %id, reason, "a pending reset failed");
                round.failed_entries += 1;
            }
            Ok(Completed::Expired) => {
                self.backoff().forget(id);
                tracing::info!(
                    %tenant_id,
                    reset_id = %id,
                    window_secs = self.settings.expiry.as_secs(),
                    reason = SOURCE_DID_NOT_END,
                    "a pending reset expired: its source did not end within the window"
                );
                round.expired += 1;
            }
            Ok(Completed::Idle) => self.backoff().forget(id),
            Ok(Completed::Busy) => {
                // Another transaction holds the row of the source, which may be ending the run (an ended source wins, so
                // the entry is not ended now). Not an error and not work: the entry is left out for a short constant time,
                // so that it does not keep its slot in the share of its tenant every round and hide the newer entries
                // behind it (D2081).
                self.backoff().defer(id, now, Backoff::BUSY);
                tracing::debug!(
                    %tenant_id,
                    reset_id = %id,
                    "a pending reset past its window has a source that another transaction holds: left for a moment"
                );
                round.busy += 1;
            }
            Ok(Completed::Refused(reason)) => {
                self.backoff().forget(id);
                tracing::debug!(%tenant_id, reset_id = %id, reason, "the completion of a reset was refused");
                // an entry that another step completed is an ordinary race; only a lost lease is what the
                // fencing alert counts
                if reason == SHARD_LEASE_LOST {
                    metrics::counter!("scheduler_fenced_total", "kind" => "reset").increment(1);
                }
                round.refused += 1;
            }
            Err(error) => {
                let (delay, attempts) = self.backoff().failed(id, now);
                tracing::warn!(
                    %tenant_id,
                    reset_id = %id,
                    %error,
                    attempts,
                    retry_in_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                    "a pending reset could not be completed: it stays pending"
                );
                round.errors += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> ResetId {
        ResetId::generate()
    }

    #[test]
    fn the_batch_and_the_share_of_a_tenant_stay_between_one_and_the_hard_ceiling() {
        let defaults = ResetSettings::default();
        assert_eq!((defaults.batch, defaults.per_tenant), (100, 10));
        for (given, held) in [
            (0, 1),
            (250, 250),
            (1_000, 1_000),
            (1_001, 1_000),
            (u32::MAX, 1_000),
        ] {
            let settings = ResetSettings {
                batch: given,
                per_tenant: given,
                ..ResetSettings::default()
            }
            .effective();
            assert_eq!(
                (settings.batch, settings.per_tenant),
                (held, held),
                "{given}"
            );
        }
    }

    #[test]
    fn the_window_of_an_entry_is_a_day_and_stays_between_an_hour_and_seven_days() {
        assert_eq!(
            ResetSettings::default().expiry,
            Duration::from_secs(24 * 3_600)
        );
        let hour = Duration::from_secs(3_600);
        let week = Duration::from_secs(7 * 24 * 3_600);
        for (given, held) in [
            (Duration::ZERO, hour),
            (hour - Duration::from_nanos(1), hour),
            (hour, hour),
            (hour + Duration::from_secs(1), hour + Duration::from_secs(1)),
            (week, week),
            (week + Duration::from_secs(1), week),
            (Duration::MAX, week),
        ] {
            let settings = ResetSettings {
                expiry: given,
                ..ResetSettings::default()
            }
            .effective();
            assert_eq!(settings.expiry, held, "{given:?}");
        }
    }

    #[test]
    fn an_entry_whose_source_is_busy_is_left_out_for_a_short_constant_time_and_its_failures_do_not_grow()
     {
        let (mut backoff, t0, entry) = (Backoff::default(), Instant::now(), id());
        assert_eq!(Backoff::BUSY, Duration::from_secs(1));
        backoff.defer(entry, t0, Backoff::BUSY);
        assert_eq!(backoff.waiting(t0), [entry]);
        assert_eq!(
            backoff.waiting(t0 + Backoff::BUSY - Duration::from_millis(1)),
            [entry]
        );
        assert!(
            backoff.waiting(t0 + Backoff::BUSY).is_empty(),
            "at its time the entry is tried again"
        );
        // busy again: the same second, not a doubled one
        for round in 1..=5u32 {
            let at = t0 + Backoff::BUSY * round;
            backoff.defer(entry, at, Backoff::BUSY);
            assert_eq!(
                backoff.waiting(at + Backoff::BUSY - Duration::from_millis(1)),
                [entry],
                "round {round}"
            );
            assert!(
                backoff.waiting(at + Backoff::BUSY).is_empty(),
                "round {round}"
            );
        }
        // a source that was busy is not a failure: the first error that follows is the first
        let (delay, attempts) = backoff.failed(entry, t0);
        assert_eq!((delay, attempts), (Backoff::BASE, 1));
        // and a deferral never shortens what a failure asked for
        backoff.failed(entry, t0);
        backoff.failed(entry, t0);
        assert_eq!(delay_after(3), Duration::from_secs(8));
        backoff.defer(entry, t0, Backoff::BUSY);
        assert_eq!(
            backoff.waiting(t0 + Duration::from_secs(8) - Duration::from_millis(1)),
            [entry]
        );
        // what is remembered is forgotten with a success, as for a failure
        backoff.forget(entry);
        assert!(backoff.is_empty());
        // and the memory stays bounded whatever is deferred
        for n in 0..(Backoff::MAX_TRACKED + 10) {
            backoff.defer(id(), t0 + Duration::from_millis(n as u64), Backoff::BUSY);
        }
        assert_eq!(backoff.len(), Backoff::MAX_TRACKED);
    }

    #[test]
    fn a_round_that_expired_entries_rests_for_what_they_cost_at_a_hundred_a_second_and_one_that_did_not_does_not()
     {
        let round = |created, failed_entries, expired| ResetRound {
            created,
            failed_entries,
            expired,
            ..ResetRound::default()
        };
        // completions and failures are not paced: a backlog of runs to make drains at full speed, as before
        assert_eq!(round(0, 0, 0).pause_after(), Duration::ZERO);
        assert_eq!(round(100, 0, 0).pause_after(), Duration::ZERO);
        assert_eq!(round(0, 30, 0).pause_after(), Duration::ZERO);
        // an expiry is: at least 200 ms for any, and the time the entries cost at the rate for many
        assert_eq!(ResetRound::EXPIRIES_PER_SECOND, 100);
        assert_eq!(round(0, 0, 1).pause_after(), Duration::from_millis(200));
        assert_eq!(round(0, 0, 20).pause_after(), Duration::from_millis(200));
        assert_eq!(round(0, 0, 50).pause_after(), Duration::from_millis(500));
        assert_eq!(round(0, 0, 100).pause_after(), Duration::from_secs(1));
        assert_eq!(round(0, 0, 1_000).pause_after(), Duration::from_secs(10));
        // the completions of the same round do not shorten it
        assert_eq!(round(60, 5, 100).pause_after(), Duration::from_secs(1));
        // the rate holds for every size of round the settings allow
        for expired in [1, 7, 99, 100, 250, 999, 1_000] {
            let pause = round(0, 0, expired).pause_after();
            assert!(
                pause.as_millis() * u128::from(ResetRound::EXPIRIES_PER_SECOND)
                    >= u128::from(expired) * 1_000,
                "{expired} entries in {pause:?}"
            );
        }
    }

    #[test]
    fn a_round_that_only_expired_entries_did_work_and_one_that_found_nothing_did_not() {
        assert!(!ResetRound::default().did_work());
        for round in [
            ResetRound {
                created: 1,
                ..ResetRound::default()
            },
            ResetRound {
                failed_entries: 1,
                ..ResetRound::default()
            },
            ResetRound {
                expired: 1,
                ..ResetRound::default()
            },
        ] {
            assert!(round.did_work(), "{round:?}");
        }
        // finding entries, being refused or erroring is not work: the next round waits its pause
        assert!(
            !ResetRound {
                due: 3,
                lapsed: 2,
                busy: 2,
                refused: 1,
                errors: 1,
                backing_off: 1,
                ..ResetRound::default()
            }
            .did_work()
        );
    }

    #[test]
    fn the_delay_doubles_from_the_base_and_never_passes_the_ceiling() {
        let delays: Vec<u64> = (1..=10).map(|n| delay_after(n).as_secs()).collect();
        assert_eq!(delays, [2, 4, 8, 16, 32, 64, 128, 256, 300, 300]);
        // however many times it has failed, even a count that would overflow the shift
        for attempts in [100, 1_000, u32::MAX] {
            assert_eq!(delay_after(attempts), Backoff::CEILING, "{attempts}");
        }
        // an entry that has not failed (0) waits the base at most
        assert_eq!(delay_after(0), Backoff::BASE);
    }

    #[test]
    fn an_entry_that_failed_is_left_out_until_its_time_and_comes_back_when_it_has_come() {
        let (mut backoff, t0, entry) = (Backoff::default(), Instant::now(), id());
        assert!(backoff.waiting(t0).is_empty());
        let (delay, attempts) = backoff.failed(entry, t0);
        assert_eq!((delay, attempts), (Backoff::BASE, 1));
        assert_eq!(backoff.waiting(t0), [entry]);
        assert_eq!(
            backoff.waiting(t0 + Backoff::BASE - Duration::from_millis(1)),
            [entry]
        );
        assert!(
            backoff.waiting(t0 + Backoff::BASE).is_empty(),
            "at its time the entry is tried again"
        );
        // and it failed again: the delay doubles from the time it was tried
        let (delay, attempts) = backoff.failed(entry, t0 + Backoff::BASE);
        assert_eq!((delay, attempts), (Duration::from_secs(4), 2));
    }

    #[test]
    fn what_is_remembered_is_forgotten_with_a_success_and_the_delay_starts_again() {
        let (mut backoff, t0, entry) = (Backoff::default(), Instant::now(), id());
        for _ in 0..5 {
            backoff.failed(entry, t0);
        }
        assert_eq!(delay_after(5), Duration::from_secs(32));
        backoff.forget(entry);
        assert!(backoff.is_empty());
        assert!(backoff.waiting(t0).is_empty());
        let (delay, attempts) = backoff.failed(entry, t0);
        assert_eq!(
            (delay, attempts),
            (Backoff::BASE, 1),
            "a new failure starts again"
        );
        // forgetting what is not remembered is nothing
        backoff.forget(id());
        assert_eq!(backoff.len(), 1);
    }

    #[test]
    fn the_memory_is_bounded_and_forgets_the_entry_whose_time_comes_first() {
        let (mut backoff, t0) = (Backoff::default(), Instant::now());
        let first = id();
        backoff.failed(first, t0);
        for n in 1..Backoff::MAX_TRACKED {
            backoff.failed(
                id(),
                t0 + Duration::from_secs(u64::try_from(n).expect("small")),
            );
        }
        assert_eq!(backoff.len(), Backoff::MAX_TRACKED);
        // one more: the oldest (the one tried again soonest) goes, the size does not grow
        let last = id();
        backoff.failed(last, t0 + Duration::from_secs(10_000));
        assert_eq!(backoff.len(), Backoff::MAX_TRACKED);
        let waiting = backoff.waiting(t0);
        assert!(!waiting.contains(&first), "the soonest was forgotten");
        assert!(
            backoff
                .waiting(t0 + Duration::from_secs(10_000))
                .contains(&last)
        );
        // an entry already remembered is updated in place, however full the memory is
        backoff.failed(last, t0 + Duration::from_secs(10_001));
        assert_eq!(backoff.len(), Backoff::MAX_TRACKED);
    }
}

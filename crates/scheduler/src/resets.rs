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

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sagat_application::clock::Clock;
use sagat_application::identity::IdentityStore;
use sagat_application::reset_completion::{
    Completed, PendingResetStore, ResetCompleter, ResetCompletions,
};
use sagat_application::schedules::{SHARD_LEASE_LOST, ShardFence};
use sagat_domain::execution::ResetId;
use sagat_domain::sharding::Shard;

use crate::shards::Placement;

/// How a pass is tuned.
#[derive(Debug, Clone, Copy)]
pub struct ResetSettings {
    /// Entries completed in one round, across the shards the replica owns (default 100, hard ceiling 1,000). The
    /// round stops there and the next one carries on, so a burst is spread over rounds.
    pub batch: u32,
    /// Entries of any one tenant that one round takes (default 10, at least 1 and at most [`Self::MAX_BATCH`]), so
    /// that the entries of one tenant that fail cannot fill the round and starve the others.
    pub per_tenant: u32,
}

impl ResetSettings {
    /// The hard ceiling of [`Self::batch`] and of [`Self::per_tenant`].
    pub const MAX_BATCH: u32 = 1_000;

    /// The settings held inside their ceilings: above the hard one is lowered, and at least one entry is allowed (a
    /// round that may do nothing would never complete one).
    #[must_use]
    pub const fn effective(self) -> Self {
        Self {
            batch: clamp(self.batch),
            per_tenant: clamp(self.per_tenant),
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
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResetRound {
    /// Entries that were due: pending, with a source that has ended, and not being backed off from.
    pub due: usize,
    /// Runs created.
    pub created: u32,
    /// Entries failed with a closed reason.
    pub failed_entries: u32,
    /// Entries another replica or a lost lease took away.
    pub refused: u32,
    /// Entries that could not be completed for a reason of the store or the platform, to be tried again later.
    pub errors: u32,
    /// Entries left out of the scan because they failed lately and their time has not come.
    pub backing_off: usize,
}

impl ResetRound {
    /// Whether the round did something, so the next one should follow at once.
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.created > 0 || self.failed_entries > 0
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

    /// An entry failed at `now`: when it may be tried again, and after how many failures in a row.
    pub fn failed(&mut self, id: ResetId, now: Instant) -> (Duration, u32) {
        let attempts = self
            .failing
            .get(&id)
            .map_or(1, |failing| failing.attempts.saturating_add(1));
        let delay = delay_after(attempts);
        if !self.failing.contains_key(&id)
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
            let shard = Shard::of(id.as_uuid());
            let Some(lease) = snapshot.lease_of(shard, now) else {
                continue;
            };
            let fence = ShardFence {
                owner: self.placement.owner().to_owned(),
                lease,
            };
            match self.completer.step(&entry, Some(fence)).await {
                Ok(Completed::Created) => {
                    self.backoff().forget(id);
                    round.created += 1;
                }
                Ok(Completed::Failed(reason)) => {
                    self.backoff().forget(id);
                    tracing::info!(%tenant_id, reset_id = %id, reason, "a pending reset failed");
                    round.failed_entries += 1;
                }
                Ok(Completed::Idle) => self.backoff().forget(id),
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
        round
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

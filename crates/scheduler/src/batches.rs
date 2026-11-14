//! The batch pass (feature 018, `FR-018-029..031`, `FR-018-033`): finds the batch jobs of the shards
//! this replica owns whose slot has come and lets [`BatchExecutor`] take a step on each.
//!
//! The pass holds no state of its own. What it owns is a lease, taken from the keeper's placement,
//! and every step it takes is committed under that lease by the executor: a replica that was slow or
//! cut off finds, in the transaction that would record the step, that the generation moved and
//! commits nothing. The effect of an item is keyed (`batch:{job}:{run}`), so the step that was
//! refused after its effect is repeated by the next owner without changing the run twice.

use std::time::{Duration, Instant};

use sagat_application::batch::BatchStore;
use sagat_application::batch_executor::{BatchExecutor, ItemActions, Stepped};
use sagat_application::clock::Clock;
use sagat_application::identity::IdentityStore;
use sagat_application::schedules::ShardFence;
use sagat_domain::sharding::Shard;

use crate::shards::Placement;

/// How a pass is tuned.
#[derive(Debug, Clone, Copy)]
pub struct BatchSettings {
    /// Jobs looked at in one round, across the shards the replica owns.
    pub batch: u32,
    /// Steps taken for one job in one round: a job at the top rate takes many items in a round, a
    /// slow one at most one, and no job holds the others back for the whole round.
    pub steps_per_job: u32,
    /// Steps one replica takes in one round, over all its jobs (default 100, hard ceiling 1,000).
    /// The round stops there and the next one carries on, so a burst of due jobs is spread over
    /// rounds instead of one long one.
    pub round_actions: u32,
}

impl BatchSettings {
    /// The hard ceiling of [`Self::round_actions`].
    pub const MAX_ROUND_ACTIONS: u32 = 1_000;

    /// The settings held inside their ceilings: above the hard one is lowered, and at least one
    /// step is allowed (a round that may do nothing would never advance a job).
    #[must_use]
    pub const fn effective(self) -> Self {
        let round_actions = if self.round_actions == 0 {
            1
        } else if self.round_actions > Self::MAX_ROUND_ACTIONS {
            Self::MAX_ROUND_ACTIONS
        } else {
            self.round_actions
        };
        let steps_per_job = if self.steps_per_job == 0 {
            1
        } else {
            self.steps_per_job
        };
        Self {
            round_actions,
            steps_per_job,
            ..self
        }
    }
}

impl Default for BatchSettings {
    fn default() -> Self {
        Self {
            batch: 100,
            steps_per_job: 20,
            round_actions: 100,
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchRound {
    /// Jobs that were due.
    pub due: usize,
    /// Steps taken: a page selected, an item applied, a cancelled job drained.
    pub steps: u32,
    /// Steps another replica or a lost lease took away.
    pub refused: u32,
    /// Steps that failed for a reason of the store or the platform. A selection round that failed and was recorded
    /// (`Stepped::Backoff`, G2093) counts too: the job waits its backoff and is not scanned until then, and the round, like
    /// any round that failed, leaves the loop to its poll (`next_due_in` is `None`).
    pub failed: u32,
    /// How long until the loop should look again, when that is sooner than its poll (G2043). Only a round that did nothing
    /// and met no refusal or failure says it: the time to the soonest slot of a running job of the shards (measured against
    /// the start of the round's own scan, so a slot that came during the round is a zero and not lost), or a few
    /// milliseconds when a due job found nothing to do (the executor's clock reads before the database's). `None` is
    /// "nothing is known", and the loop sleeps its poll.
    pub next_due_in: Option<Duration>,
}

impl BatchRound {
    /// Whether the round did something, so the next one should follow at once.
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.steps > 0
    }
}

/// The pass over the batch jobs of the shards a replica owns.
pub struct BatchPass<S, A, I, C> {
    executor: BatchExecutor<S, A, I, C>,
    placement: Placement,
    settings: BatchSettings,
}

impl<S, A, I, C> BatchPass<S, A, I, C>
where
    S: BatchStore,
    A: ItemActions,
    I: IdentityStore,
    C: Clock,
{
    pub const fn new(
        executor: BatchExecutor<S, A, I, C>,
        placement: Placement,
        settings: BatchSettings,
    ) -> Self {
        Self {
            executor,
            placement,
            settings: settings.effective(),
        }
    }

    /// Whether the round has taken the steps it may take: it stops there and the next one, which
    /// follows at once while there is work, carries on.
    const fn full(&self, round: &BatchRound) -> bool {
        round.steps >= self.settings.round_actions
    }

    /// Runs one round. A failure is logged and counted; the next round starts over from what is
    /// stored.
    pub async fn round(&self) -> BatchRound {
        let snapshot = self.placement.snapshot();
        let now = Instant::now();
        let shards: Vec<Shard> = snapshot.shards(now).iter().collect();
        let mut round = BatchRound::default();
        if shards.is_empty() {
            return round;
        }
        let due = match self
            .executor
            .store()
            .due_jobs(&shards, self.settings.batch)
            .await
        {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(%error, "could not look for due batch jobs");
                round.failed += 1;
                return round;
            }
        };
        round.due = due.jobs.len();
        let ahead = due.next_slot_in;
        // whether a step found a due job with nothing to do: the executor's clock reads before the slot the scan found come
        let mut found_idle = false;
        for job in due.jobs {
            if self.full(&round) {
                break;
            }
            let shard = Shard::of(job.id);
            let Some(lease) = snapshot.lease_of(shard, now) else {
                continue;
            };
            let fence = ShardFence {
                owner: self.placement.owner().to_owned(),
                lease,
            };
            for _ in 0..self.settings.steps_per_job {
                if self.full(&round) {
                    break;
                }
                match self.executor.step(&job, Some(fence.clone())).await {
                    Ok(Stepped::Acted) => round.steps += 1,
                    Ok(Stepped::Idle) => {
                        found_idle = true;
                        break;
                    }
                    Ok(Stepped::Refused(reason)) => {
                        tracing::debug!(reason, "a batch step was refused");
                        // an item that another step ended, or a job that was cancelled, is an
                        // ordinary race; only a lost lease is what the fencing alert counts
                        if reason.contains("lease") {
                            metrics::counter!("scheduler_fenced_total", "kind" => "batch")
                                .increment(1);
                        }
                        round.refused += 1;
                        break;
                    }
                    // a selection round failed and was recorded (the executor logged it, with the id of the job and the
                    // count only): the job leaves the scan until its backoff has passed, so the jobs behind it are reached
                    // and this one is not stepped again in this round
                    Ok(Stepped::Backoff) => {
                        round.failed += 1;
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "a batch step failed");
                        round.failed += 1;
                        break;
                    }
                }
            }
        }
        round.next_due_in = idle_hint(&round, ahead, found_idle);
        round
    }
}

/// How soon a round that found a job due, and could not step it, comes back: the scan said the job is due by the database's
/// clock and the executor decided by its own (`DatabaseClock`, whose reading is behind by up to half a round trip, and by
/// what it has drifted since its last), so the job is not yet due *for the executor*. The two agree within the next
/// milliseconds; waiting the whole poll for that brings the stall back whenever they differ by a hair. It cannot spin: the
/// job leaves the scan as soon as the engine's clock agrees, a round costs several statements, and the loop never rests less
/// than a millisecond, so this is a hundred rounds a second at the most and only while the clocks disagree.
const DUE_BUT_NOT_YET: Duration = Duration::from_millis(10);

/// What a round tells the loop about its next wake-up (G2043), given the time to the soonest slot ahead of the start of its
/// scan (`ahead`) and whether a step found a due job with nothing to do (`found_idle`). Only a round that did nothing, and
/// met neither a refusal nor a failure, says anything: one that worked is followed at once, and one that was refused or
/// failed is left to the poll.
///
/// * a step found a due job with nothing to do: the clocks disagree by a hair (see [`DUE_BUT_NOT_YET`]), so
///   [`DUE_BUT_NOT_YET`], or the slot ahead when that is sooner;
/// * otherwise the slot that is ahead, if any. It was measured against the start of the scan, in the scan's own statement,
///   so a slot that came while the round ran is a zero and not a lost wake-up. A slot that had already come at the start of
///   the scan and is not due is not here, so a job with nothing to do yet cannot make the loop spin; and a due job that no
///   step looked at (its shard's lease lapsed between the scan and the step) is the keeper's to renew, not a reason to
///   look again in ten milliseconds.
fn idle_hint(round: &BatchRound, ahead: Option<Duration>, found_idle: bool) -> Option<Duration> {
    if round.did_work() || round.refused > 0 || round.failed > 0 {
        return None;
    }
    if found_idle {
        return Some(ahead.map_or(DUE_BUT_NOT_YET, |slot| slot.min(DUE_BUT_NOT_YET)));
    }
    ahead
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{BatchRound, BatchSettings, DUE_BUT_NOT_YET, idle_hint};

    fn round(due: usize, steps: u32, refused: u32, failed: u32) -> BatchRound {
        BatchRound {
            due,
            steps,
            refused,
            failed,
            next_due_in: None,
        }
    }

    #[test]
    fn a_round_that_found_no_job_due_and_did_nothing_says_the_slot_that_is_ahead() {
        let ahead = Some(Duration::from_millis(50));
        assert_eq!(idle_hint(&round(0, 0, 0, 0), ahead, false), ahead);
        assert_eq!(
            idle_hint(&round(0, 0, 0, 0), None, false),
            None,
            "no job has a slot ahead: the poll"
        );
        assert_eq!(
            idle_hint(&round(0, 0, 0, 0), Some(Duration::ZERO), false),
            Some(Duration::ZERO),
            "a slot that came while the scan ran is a zero, which the loop floors at 1 ms: not a lost wake-up"
        );
    }

    /// A due job that the executor did not step is the clocks disagreeing by a hair (the database says due, the engine's own
    /// clock reads before the slot): it is looked at again in a few milliseconds and not after the poll. It cannot spin: the
    /// job leaves the scan as soon as the engine's clock agrees, a round costs several statements, and the wait is never
    /// less than the floor of the loop (1 ms) and at most ten here, so it is a hundred rounds a second at the most, and only
    /// while the clocks disagree.
    #[test]
    fn a_due_job_that_was_not_stepped_comes_back_in_a_few_milliseconds() {
        assert_eq!(
            idle_hint(&round(1, 0, 0, 0), None, true),
            Some(DUE_BUT_NOT_YET),
            "due, a step found nothing to do, nothing else ahead"
        );
        assert_eq!(
            idle_hint(&round(3, 0, 0, 0), Some(Duration::from_secs(60)), true),
            Some(DUE_BUT_NOT_YET),
            "a slot far ahead of another job does not stretch it"
        );
        assert_eq!(
            idle_hint(&round(1, 0, 0, 0), Some(Duration::from_millis(3)), true),
            Some(Duration::from_millis(3)),
            "and a slot sooner than that is the sooner one"
        );
        assert!(DUE_BUT_NOT_YET <= Duration::from_millis(10));
        assert_eq!(
            idle_hint(&round(1, 0, 0, 0), None, false),
            None,
            "a due job that no step looked at (its lease lapsed) is the keeper's, not a reason to look again at once"
        );
    }

    #[test]
    fn a_round_that_worked_or_was_refused_or_failed_says_nothing() {
        let ahead = Some(Duration::from_millis(50));
        for round in [
            round(1, 1, 0, 0),
            round(0, 1, 0, 0),
            round(1, 0, 1, 0),
            round(1, 0, 0, 1),
            round(0, 0, 0, 1),
        ] {
            for found_idle in [false, true] {
                assert_eq!(
                    idle_hint(&round, ahead, found_idle),
                    None,
                    "{round:?}: followed at once, or left to the poll"
                );
            }
        }
    }

    fn with(round_actions: u32, steps_per_job: u32) -> BatchSettings {
        BatchSettings {
            round_actions,
            steps_per_job,
            ..BatchSettings::default()
        }
        .effective()
    }

    #[test]
    fn the_steps_of_a_round_stay_between_one_and_the_hard_ceiling() {
        assert_eq!(BatchSettings::default().round_actions, 100);
        assert_eq!(
            with(0, 20).round_actions,
            1,
            "a round that may do nothing advances nothing"
        );
        assert_eq!(with(250, 20).round_actions, 250);
        assert_eq!(with(1_000, 20).round_actions, 1_000);
        assert_eq!(
            with(1_001, 20).round_actions,
            BatchSettings::MAX_ROUND_ACTIONS,
            "above the ceiling is lowered to it"
        );
        assert_eq!(with(u32::MAX, 20).round_actions, 1_000);
    }

    #[test]
    fn a_job_may_always_take_a_step() {
        assert_eq!(with(100, 0).steps_per_job, 1);
        assert_eq!(with(100, 7).steps_per_job, 7);
    }
}

//! The batch pass (feature 018, `FR-018-029..031`, `FR-018-033`): finds the batch jobs of the shards
//! this replica owns whose slot has come and lets [`BatchExecutor`] take a step on each.
//!
//! The pass holds no state of its own. What it owns is a lease, taken from the keeper's placement,
//! and every step it takes is committed under that lease by the executor: a replica that was slow or
//! cut off finds, in the transaction that would record the step, that the generation moved and
//! commits nothing. The effect of an item is keyed (`batch:{job}:{run}`), so the step that was
//! refused after its effect is repeated by the next owner without changing the run twice.

use std::time::Instant;

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
    /// Steps that failed for a reason of the store or the platform.
    pub failed: u32,
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
        round.due = due.len();
        for job in due {
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
                    Ok(Stepped::Idle) => break,
                    Ok(Stepped::Refused(reason)) => {
                        tracing::debug!(reason, "a batch step was refused");
                        metrics::counter!("scheduler_fenced_total", "kind" => "batch").increment(1);
                        round.refused += 1;
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
        round
    }
}

#[cfg(test)]
mod tests {
    use super::BatchSettings;

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

//! The schedule pass (feature 018, `FR-018-005`, `FR-018-044`): finds the schedules of the shards this
//! replica owns whose time has come and lets [`ScheduleFiring`] act on them.
//!
//! The pass holds no state of its own. What it owns is a lease, taken from the keeper's placement,
//! and every step it takes is committed under that lease by the firing: a replica that was slow or
//! cut off finds, in the transaction that would act, that the generation moved and commits nothing.

use std::time::Instant;

use sagat_application::clock::Clock;
use sagat_application::identity::IdentityStore;
use sagat_application::schedule_firing::{Fired, ScheduleFiring, ScheduledStarts};
use sagat_application::schedules::{ScheduleCalendar, ScheduleStore, ShardFence};
use sagat_domain::sharding::Shard;

use crate::shards::Placement;

/// How a pass is tuned.
#[derive(Debug, Clone, Copy)]
pub struct ScheduleSettings {
    /// Schedules looked at in one round, across the shards the replica owns.
    pub batch: u32,
    /// Steps taken for one schedule in one round: an outage is worked through over several rounds
    /// rather than by one schedule holding the others back.
    pub steps_per_schedule: u32,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            batch: 100,
            steps_per_schedule: 20,
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScheduleRound {
    /// Schedules that were due.
    pub due: usize,
    /// Steps taken: a run started, a decision recorded.
    pub steps: u32,
    /// Steps another replica or a lost lease took away.
    pub refused: u32,
    /// Steps that failed for a reason of the store or the platform.
    pub failed: u32,
}

impl ScheduleRound {
    /// Whether the round did something, so the next one should follow at once.
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.steps > 0
    }
}

/// The pass over the schedules of the shards a replica owns.
pub struct SchedulePass<S, K, C, I, R> {
    firing: ScheduleFiring<S, K, C, I, R>,
    placement: Placement,
    settings: ScheduleSettings,
}

impl<S, K, C, I, R> SchedulePass<S, K, C, I, R>
where
    S: ScheduleStore,
    K: ScheduleCalendar,
    C: Clock,
    I: IdentityStore,
    R: ScheduledStarts,
{
    pub const fn new(
        firing: ScheduleFiring<S, K, C, I, R>,
        placement: Placement,
        settings: ScheduleSettings,
    ) -> Self {
        Self {
            firing,
            placement,
            settings,
        }
    }

    /// Runs one round. A failure is logged and counted; the next round starts over from what is
    /// stored.
    pub async fn round(&self) -> ScheduleRound {
        let snapshot = self.placement.snapshot();
        let now = Instant::now();
        let shards: Vec<Shard> = snapshot.shards(now).iter().collect();
        let mut round = ScheduleRound::default();
        if shards.is_empty() {
            return round;
        }
        let due = match self
            .firing
            .store()
            .due_schedules(&shards, self.settings.batch)
            .await
        {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(%error, "could not look for due schedules");
                round.failed += 1;
                return round;
            }
        };
        round.due = due.len();
        for schedule in due {
            let shard = Shard::of(schedule.id.as_uuid());
            let Some(lease) = snapshot.lease_of(shard, now) else {
                continue;
            };
            let fence = ShardFence {
                owner: self.placement.owner().to_owned(),
                lease,
            };
            for _ in 0..self.settings.steps_per_schedule {
                match self.firing.fire(&schedule, Some(fence.clone())).await {
                    Ok(Fired::Acted) => round.steps += 1,
                    Ok(Fired::Idle) => break,
                    Ok(Fired::Refused(reason)) => {
                        tracing::debug!(reason, "a schedule step was refused");
                        metrics::counter!("scheduler_fenced_total", "kind" => "schedule")
                            .increment(1);
                        round.refused += 1;
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "a schedule step failed");
                        round.failed += 1;
                        break;
                    }
                }
            }
        }
        round
    }
}

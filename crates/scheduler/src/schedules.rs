//! The schedule pass (feature 018, `FR-018-005`, `FR-018-044`): finds the schedules of the shards this
//! replica owns whose time has come and lets [`ScheduleFiring`] act on them.
//!
//! The pass holds no state of its own. What it owns is a lease, taken from the keeper's placement,
//! and every step it takes is committed under that lease by the firing: a replica that was slow or
//! cut off finds, in the transaction that would act, that the generation moved and commits nothing.

use std::time::{Duration, Instant};

use sagat_application::clock::Clock;
use sagat_application::identity::IdentityStore;
use sagat_application::maintenance::Window;
use sagat_application::schedule_firing::{Fired, ScheduleFiring, ScheduledStarts};
use sagat_application::schedules::{ScheduleCalendar, ScheduleStore, ShardFence};
use sagat_domain::identity::TenantId;
use sagat_domain::sharding::Shard;

use crate::fairness;
use crate::shards::Placement;

/// How a pass is tuned.
#[derive(Debug, Clone, Copy)]
pub struct ScheduleSettings {
    /// Schedules looked at in one round, across the shards the replica owns.
    pub batch: u32,
    /// Steps taken for one schedule in one round: an outage is worked through over several rounds
    /// rather than by one schedule holding the others back.
    pub steps_per_schedule: u32,
    /// Actions one replica takes in one round, over all its schedules and backfills (default 100,
    /// hard ceiling 1,000). The round stops there and the next one carries on, so a burst of due
    /// schedules is spread over rounds instead of one long one.
    pub round_actions: u32,
    /// How long the search for other tenants rests after it found nobody new (see
    /// `SchedulerSettings::discovery_rest`); zero: never.
    pub discovery_rest: Duration,
}

impl ScheduleSettings {
    /// The hard ceiling of [`Self::round_actions`].
    pub const MAX_ROUND_ACTIONS: u32 = 1_000;

    /// The settings held inside their ceilings: above the hard one is lowered, and at least one
    /// action is allowed (a round that may do nothing would never advance a schedule).
    #[must_use]
    pub const fn effective(self) -> Self {
        let round_actions = if self.round_actions == 0 {
            1
        } else if self.round_actions > Self::MAX_ROUND_ACTIONS {
            Self::MAX_ROUND_ACTIONS
        } else {
            self.round_actions
        };
        Self {
            round_actions,
            ..self
        }
    }
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            batch: 100,
            steps_per_schedule: 20,
            round_actions: 100,
            discovery_rest: Duration::from_millis(100),
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
    /// Until when the search for other tenants rests (`fairness::rest_after`).
    rest_until: std::sync::Mutex<Option<Instant>>,
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
            settings: settings.effective(),
            rest_until: std::sync::Mutex::new(None),
        }
    }

    /// Whether the round has taken the actions it may take (`FR-018-045`): it stops there and the
    /// next one, which follows at once while there is work, carries on.
    const fn full(&self, round: &ScheduleRound) -> bool {
        round.steps >= self.settings.round_actions
    }

    /// The active backfills of the shards the replica owns, one step at a time under the lease of
    /// the shard of their schedule, like the schedules themselves.
    async fn backfills(
        &self,
        snapshot: &crate::shards::Snapshot,
        now: Instant,
        shards: &[Shard],
        round: &mut ScheduleRound,
    ) {
        let backfills = match self
            .firing
            .store()
            .due_backfills(shards, self.settings.batch)
            .await
        {
            Ok(backfills) => backfills,
            Err(error) => {
                tracing::warn!(%error, "could not look for backfills");
                round.failed += 1;
                return;
            }
        };
        for backfill in backfills {
            if self.full(round) {
                return;
            }
            let shard = Shard::of(backfill.schedule.as_uuid());
            let Some(lease) = snapshot.lease_of(shard, now) else {
                continue;
            };
            let fence = ShardFence {
                owner: self.placement.owner().to_owned(),
                lease,
            };
            for _ in 0..self.settings.steps_per_schedule {
                if self.full(round) {
                    return;
                }
                match self
                    .firing
                    .backfill_step(&backfill, Some(fence.clone()))
                    .await
                {
                    Ok(Fired::Acted) => round.steps += 1,
                    Ok(Fired::Idle) => break,
                    Ok(Fired::Refused(reason)) => {
                        tracing::debug!(reason, "a backfill step was refused");
                        metrics::counter!("scheduler_fenced_total", "kind" => "backfill")
                            .increment(1);
                        round.refused += 1;
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "a backfill step failed");
                        round.failed += 1;
                        break;
                    }
                }
            }
        }
    }

    /// The schedules this round looks at: the oldest of the shards the replica owns when they all
    /// fit in the batch, and otherwise a share for every tenant that has any due (`FR-010-002`),
    /// so that a tenant with a backlog of older schedules does not hold back the others.
    async fn due(
        &self,
        owned: sagat_domain::sharding::ShardSet,
        snapshot: &crate::shards::Snapshot,
    ) -> Result<Vec<sagat_application::schedules::DueSchedule>, sagat_application::error::StoreError>
    {
        let limit = self.settings.batch;
        let store = self.firing.store();
        let discover = self
            .rest_until
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none_or(|until| until <= Instant::now());
        let (chosen, selection) = fairness::select(
            limit,
            discover,
            snapshot.weights(),
            |due: &sagat_application::schedules::DueSchedule| due.tenant_id,
            |due| due.id,
            |exclude: Vec<TenantId>| async move {
                store
                    .due_schedules(&Window {
                        shards: owned,
                        exclude,
                        limit,
                    })
                    .await
            },
        )
        .await?;
        let rest = fairness::rest_after(self.settings.discovery_rest, &selection);
        if discover && !rest.is_zero() {
            *self
                .rest_until
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now() + rest);
        }
        if selection.limited > 0 {
            metrics::counter!("scheduler_fairness_limited_total", "pass" => "schedules")
                .increment(selection.limited as u64);
        }
        if selection.exhausted {
            metrics::counter!("scheduler_fairness_exhausted_total", "pass" => "schedules")
                .increment(1);
        }
        Ok(chosen)
    }

    /// Runs one round. A failure is logged and counted; the next round starts over from what is
    /// stored.
    pub async fn round(&self) -> ScheduleRound {
        let snapshot = self.placement.snapshot();
        let now = Instant::now();
        let owned = snapshot.shards(now);
        let shards: Vec<Shard> = owned.iter().collect();
        let mut round = ScheduleRound::default();
        if shards.is_empty() {
            return round;
        }
        let due = match self.due(owned, &snapshot).await {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(%error, "could not look for due schedules");
                round.failed += 1;
                return round;
            }
        };
        round.due = due.len();
        self.backfills(&snapshot, now, &shards, &mut round).await;
        for schedule in due {
            if self.full(&round) {
                break;
            }
            let shard = Shard::of(schedule.id.as_uuid());
            let Some(lease) = snapshot.lease_of(shard, now) else {
                continue;
            };
            let fence = ShardFence {
                owner: self.placement.owner().to_owned(),
                lease,
            };
            for _ in 0..self.settings.steps_per_schedule {
                if self.full(&round) {
                    break;
                }
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

#[cfg(test)]
mod tests {
    use super::ScheduleSettings;

    #[test]
    fn the_actions_of_a_round_stay_between_one_and_the_hard_ceiling() {
        let with = |round_actions| {
            ScheduleSettings {
                round_actions,
                ..ScheduleSettings::default()
            }
            .effective()
            .round_actions
        };
        assert_eq!(ScheduleSettings::default().round_actions, 100);
        assert_eq!(
            with(0),
            1,
            "a round that may do nothing would never advance"
        );
        assert_eq!(with(1_000), 1_000);
        assert_eq!(
            with(1_001),
            1_000,
            "above the hard ceiling is lowered to it"
        );
        assert_eq!(with(u32::MAX), 1_000);
    }
}

//! Chaos against the sharded engine: the database's connections killed again and again, the
//! database cut off from a replica, a replica that dies in the middle of a pass or is blocked past
//! its lease, the broker gone while a dispatcher changes hands, and workers lost with their leases
//! (T010-030; FR-010-005; edge cases "shard split-brain", "failover with stale outbox", "slow NATS
//! consumer"). What each one has to show is the same: nothing a run committed is lost, nothing that
//! should happen once happens twice, and the platform goes on by itself once the fault ends.
//!
//! Each test ends in the same two checks: a run's journal holds one `timer.fired` (or the single
//! outcome it should), and `verify_run` finds the projections equal to a replay of the journal.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod broker;
mod database;
mod replicas;
mod workers;

use sagat_domain::execution::RunId;
use support::engine::World;

/// Runs of a timer workflow, each `timer.fired` exactly once, each consistent with its journal.
pub async fn assert_each_fired_once(w: &World, runs: &[RunId]) {
    let counts: Vec<(uuid::Uuid, i64)> = sqlx::query_as(
        "SELECT run_id, count(*) FROM run_events WHERE event_type = 'timer.fired' AND run_id = ANY($1) GROUP BY run_id",
    )
    .bind(runs.iter().map(|r| r.as_uuid()).collect::<Vec<_>>())
    .fetch_all(&w.db.pool)
    .await
    .expect("journal");
    assert_eq!(counts.len(), runs.len(), "a timer was never fired");
    for (run, fired) in counts {
        assert_eq!(fired, 1, "the timer of run {run} fired {fired} times");
    }
    for run in runs {
        w.assert_consistent(*run).await;
    }
}

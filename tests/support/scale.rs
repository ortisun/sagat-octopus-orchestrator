//! Helpers shared by the suites of feature 010: schedulers and keepers over a world, and populations
//! of runs whose timers are due.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_domain::execution::RunId;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::json;

use super::engine::{Sched, World, activity, document, with};

pub const TTL: Duration = Duration::from_secs(30);

pub fn scheduler(w: &World, settings: SchedulerSettings, placement: Option<Placement>) -> Sched {
    let scheduler = Scheduler::new(
        w.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        settings,
    )
    .with_children(w.cache.clone());
    match placement {
        Some(placement) => scheduler.with_placement(placement),
        None => scheduler,
    }
}

pub fn keeper(w: &World, name: &str, ttl: Duration) -> Keeper<PgRunStore> {
    Keeper::new(
        w.store.clone(),
        Placement::new(name),
        KeeperSettings { ttl },
    )
}

/// `n` runs of a one-timer workflow, each with its timer due now.
pub async fn sleepers(w: &World, name: &str, n: usize) -> Vec<RunId> {
    let doc = document(
        name,
        "pause",
        vec![
            with(
                json!({"id": "pause", "type": "timer", "durationMs": 3_600_000}),
                "next",
                json!("after"),
            ),
            activity("after"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    let started = futures_util::future::join_all(
        (0..n).map(|i| async move { w.start(wf, &json!({"i": i})).await }),
    )
    .await;
    for run in &started {
        w.age_due(*run, "pause").await;
    }
    started
}

/// Whether the timer of `run` has fired (of any tenant: it reads the journal as the owner).
pub async fn fired(w: &World, run: RunId) -> bool {
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = 'timer.fired'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    events > 0
}

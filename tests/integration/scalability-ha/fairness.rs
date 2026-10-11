//! Fairness between tenants in a pass of the scheduler: a burst of one tenant does not hold another
//! back, and the weights of the tenants decide the split when both have more than a pass takes
//! (FR-010-002; US04; T010-007).

use sagat_application::runs::RunStore;
use sagat_domain::execution::RunId;
use sagat_scheduler::SchedulerSettings;
use serde_json::json;

use crate::passes::{fired, keeper, scheduler, sleepers};
use crate::support::engine::{World, activity, document, with, world};

/// `n` runs of the other tenant, each with its timer due now.
async fn sleepers_of_other_tenant(w: &World, name: &str, n: usize) -> Vec<RunId> {
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
    let wf = w.publish_as_other_tenant(doc).await;
    let mut runs = Vec::new();
    for i in 0..n {
        let run = w
            .start_as_other_tenant(wf, &json!({ "i": i }))
            .await
            .expect("start");
        w.age_due(run, "pause").await;
        runs.push(run);
    }
    runs
}

async fn count_fired(w: &World, runs: &[RunId]) -> usize {
    let mut n = 0;
    for run in runs {
        if fired(w, *run).await {
            n += 1;
        }
    }
    n
}

fn settings(batch: u32) -> SchedulerSettings {
    SchedulerSettings {
        batch,
        ..SchedulerSettings::default()
    }
}

#[tokio::test]
async fn a_burst_of_one_tenant_does_not_hold_another_back() {
    let w = world().await;
    // The burst is older than the other tenant's work, so oldest-first would serve all of it first.
    let burst = sleepers(&w, "burst", 60).await;
    let small = sleepers_of_other_tenant(&w, "small", 5).await;
    let alone = scheduler(&w, settings(20), None);

    let report = alone.tick().await;
    assert_eq!(
        report.timers_fired, 20,
        "the batch is used in full: {report:?}"
    );
    assert_eq!(
        count_fired(&w, &small).await,
        5,
        "the small tenant was served in the first pass, behind a burst of sixty"
    );
    assert_eq!(count_fired(&w, &burst).await, 15, "the burst gets the rest");

    // And the burst is not starved either: it drains.
    while alone.tick().await.timers_fired > 0 {}
    assert_eq!(count_fired(&w, &burst).await, 60);
}

#[tokio::test]
async fn weights_decide_the_split_when_both_tenants_have_more_than_a_pass_takes() {
    let w = world().await;
    let heavy_tenant = w.other_admin.tenant_id();
    sqlx::query(
        "INSERT INTO tenant_scheduling (tenant_id, weight, updated_at) VALUES ($1, 3, now())",
    )
    .bind(heavy_tenant.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("a weight");
    let light = sleepers(&w, "light", 80).await;
    let heavy = sleepers_of_other_tenant(&w, "heavy", 80).await;

    // The weights reach the scheduler through the keeper, as they do in the engine.
    let keeper = keeper(&w, "alpha", crate::passes::TTL);
    keeper.round().await.expect("round");
    let scheduler = scheduler(&w, settings(40), Some(keeper.placement().clone()));

    let report = scheduler.tick().await;
    assert_eq!(report.timers_fired, 40, "{report:?}");
    assert_eq!(count_fired(&w, &light).await, 10, "weight 1 of 4");
    assert_eq!(count_fired(&w, &heavy).await, 30, "weight 3 of 4");
}

#[tokio::test]
async fn equal_weights_split_a_pass_equally_and_a_lone_tenant_still_gets_all_of_it() {
    let w = world().await;
    let a = sleepers(&w, "even-a", 50).await;
    let b = sleepers_of_other_tenant(&w, "even-b", 50).await;
    let scheduler = scheduler(&w, settings(40), None);

    assert_eq!(scheduler.tick().await.timers_fired, 40);
    assert_eq!(count_fired(&w, &a).await, 20);
    assert_eq!(count_fired(&w, &b).await, 20);

    // Once one of them is done the other is not held to half.
    while count_fired(&w, &b).await < 50 {
        scheduler.tick().await;
    }
    let before = count_fired(&w, &a).await;
    let report = scheduler.tick().await;
    assert_eq!(
        report.timers_fired as usize,
        50 - before,
        "the rest of a single tenant's work goes in one pass: {report:?}"
    );
}

#[tokio::test]
async fn a_tenant_never_reaches_another_tenants_runs() {
    // Fairness is about order, not access: what a pass takes for one tenant is applied under that
    // tenant's id, and a run of the other is never touched by it.
    let w = world().await;
    let mine = sleepers(&w, "isolated-a", 5).await;
    let theirs = sleepers_of_other_tenant(&w, "isolated-b", 5).await;
    let scheduler = scheduler(&w, settings(200), None);
    scheduler.tick().await;
    assert_eq!(count_fired(&w, &mine).await, 5);
    assert_eq!(count_fired(&w, &theirs).await, 5);
    for run in &mine {
        assert_eq!(
            w.store
                .get(w.other_admin.tenant_id(), *run)
                .await
                .expect("get")
                .map(|_| ()),
            None,
            "the other tenant cannot read this run"
        );
    }
}

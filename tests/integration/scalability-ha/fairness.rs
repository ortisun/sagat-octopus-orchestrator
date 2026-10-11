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
        // These tests tick again and again and expect every tick to look for the other tenants.
        discovery_rest: std::time::Duration::ZERO,
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
    let a = sleepers(&w, "even-a", 90).await;
    let b = sleepers_of_other_tenant(&w, "even-b", 20).await;
    let scheduler = scheduler(&w, settings(40), None);

    // Both have more than half the pass: each gets half.
    assert_eq!(scheduler.tick().await.timers_fired, 40);
    assert_eq!(count_fired(&w, &a).await, 20);
    assert_eq!(count_fired(&w, &b).await, 20);

    // B is done; A has 70 left and nobody else: the whole pass is A's, not half of it.
    let report = scheduler.tick().await;
    assert_eq!(report.timers_fired, 40, "{report:?}");
    assert_eq!(count_fired(&w, &a).await, 60);
    assert_eq!(count_fired(&w, &b).await, 20);
    assert_eq!(scheduler.tick().await.timers_fired, 30);
    assert_eq!(count_fired(&w, &a).await, 90);
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

#[tokio::test]
async fn the_operator_sets_and_withdraws_a_weight_and_each_change_is_audited() {
    use sagat_application::error::AppError;
    use sagat_domain::identity::TenantId;

    let w = world().await;
    let tenant = w.admin.tenant_id();
    let stored = || async {
        sqlx::query_scalar::<_, i16>("SELECT weight FROM tenant_scheduling WHERE tenant_id = $1")
            .bind(tenant.as_uuid())
            .fetch_optional(&w.db.pool)
            .await
            .expect("weight")
    };
    assert_eq!(stored().await, None);

    w.quotas
        .set_scheduling_weight(tenant, Some(7))
        .await
        .expect("set");
    assert_eq!(stored().await, Some(7));
    w.quotas
        .set_scheduling_weight(tenant, Some(2))
        .await
        .expect("change");
    assert_eq!(stored().await, Some(2), "a second call changes it");
    w.quotas
        .set_scheduling_weight(tenant, None)
        .await
        .expect("withdraw");
    assert_eq!(stored().await, None);

    // Every change left a record of who (the operator) did what to which tenant.
    let reasons: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE tenant_id = $1 AND resource_id = 'scheduling-weight' ORDER BY created_at, id",
    )
    .bind(tenant.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        reasons,
        [
            "scheduling weight = 7",
            "scheduling weight = 2",
            "scheduling weight withdrawn"
        ]
    );

    // Out of range is refused and leaves nothing behind; so is a tenant that does not exist.
    for bad in [0_u16, 17, 1000] {
        assert!(matches!(
            w.quotas.set_scheduling_weight(tenant, Some(bad)).await,
            Err(AppError::NotAllowed(_))
        ));
    }
    assert!(matches!(
        w.quotas
            .set_scheduling_weight(TenantId::generate(), Some(2))
            .await,
        Err(AppError::NotFound)
    ));
    assert_eq!(stored().await, None);
    let after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND resource_id = 'scheduling-weight'",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(after, 3, "a refused change is not recorded as a change");
}

//! What a caller without the reveal right reads does not tell which runs share a key (feature 022; decisions D022-151 to
//! D022-155, D022-131, D022-133; FR-022-022 and FR-022-057 as amended 2026-11-26). The same runs are built twice: two
//! waiting runs on one key, and the same two runs on two keys, beside the same holders. For each surface the sweep of
//! D022-151 gates (a run's rate-limit slot in `getRun`, the alerts, the quota usages, the group view), a Viewer reads the
//! same answer in both worlds, at a key limit of 1 and of 2; a member with the reveal grant reads the true values, which
//! differ.

use sagat_application::clock::SystemClock;
use sagat_application::flow_control_keys::RunGateState;
use sagat_application::operations::OperationsService;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Principal, Role};
use sagat_domain::quota::QuotaResource;
use sagat_scheduler::{Monitor, MonitorSettings};
use serde_json::json;

use crate::run_admission::set;
use crate::support::engine::{World, activity, ctx, document};
use crate::support::flow_control::{gated_world, publish};

/// One world: `limit` holders on each of two keys (`acct-1`, `acct-2`) of group `accounts`, a rate limit of `limit` an
/// hour, then two waiting runs, both on `acct-1` when `shared`, else one on each key. Returns the world and the two
/// waiting runs.
async fn world(shared: bool, limit: u32) -> (World, [RunId; 2]) {
    let w = gated_world().await;
    // room for two waiters a key, so that both worlds admit the same runs; a key with two waiters is above 80% of it
    set(&w, QuotaResource::FlowControlKeyWaiters, 2).await;
    let mut doc = document("disclosure-rated", "g", vec![activity("g")]);
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": limit, "per": "hour"});
    let workflow = publish(&w, doc).await;
    for account in ["acct-1", "acct-2"] {
        for _ in 0..limit {
            w.start(workflow, &json!({"accountId": account})).await;
        }
    }
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second_key = if shared { "acct-1" } else { "acct-2" };
    let second = w.start(workflow, &json!({"accountId": second_key})).await;
    (w, [first, second])
}

/// A store of the runtime role's pool, the engine's.
async fn runtime_store(w: &World) -> sagat_infra_postgres::PgRunStore {
    crate::support::flow_control::gate(sagat_infra_postgres::PgRunStore::new(
        w.db.runtime_pool_with(4).await,
        w.cache.clone(),
    ))
}

/// Everything `who` reads of the gated surfaces, without instants (they differ between two worlds): the two waiting
/// runs' rate-limit slots (state, position, estimate shown or not, label shown or not), the alerts (kind, items), the
/// three quota lines (used, withheld) and the group view (grants in window, unleased, waiters, spans pages).
#[derive(Debug, PartialEq, Eq)]
struct Reading {
    slots: Vec<(String, Option<u32>, bool, bool)>,
    alerts: Vec<(String, u64)>,
    quotas: Vec<(String, Option<u64>, bool)>,
    groups: Vec<(u64, u64, u64)>,
}

async fn read(w: &World, who: &Principal, runs: [RunId; 2]) -> Reading {
    let mut slots = Vec::new();
    for run in runs {
        let record = w.runs.get(who, run, &ctx()).await.expect("the run");
        let slot = record.rate_limit.expect("a rate-limit slot");
        slots.push((
            // The variant only: the deadline is the run's own wait-timeout instant (G022-043), the same class
            // whether or not the runs share a key, and two worlds started apart never agree on it.
            match slot.state {
                RunGateState::Held => "Held",
                RunGateState::Waiting { .. } => "Waiting",
            }
            .to_owned(),
            slot.position,
            slot.estimated_grant_at.is_some(),
            slot.label().is_some(),
        ));
    }
    let service = OperationsService::new(
        runtime_store(w).await,
        SystemClock,
        "docs/runbooks/alerts.md",
    );
    let mut alerts: Vec<(String, u64)> = service
        .alerts(who, w.namespace.id(), None, None, None, &ctx())
        .await
        .expect("the alerts")
        .into_iter()
        .map(|alert| (alert.kind.as_str().to_owned(), alert.items))
        .collect();
    alerts.sort();
    let flow = [
        QuotaResource::ConcurrencyLiveKeys,
        QuotaResource::RateLimitLiveKeys,
        QuotaResource::FlowControlKeyWaiters,
    ];
    let mut quotas: Vec<(String, Option<u64>, bool)> = w
        .quotas
        .list(who, Some(w.namespace.id()), &ctx())
        .await
        .expect("the quota view")
        .into_iter()
        .filter(|view| flow.contains(&view.resource))
        .map(|view| {
            (
                view.resource.as_str().to_owned(),
                view.used,
                view.usage_withheld,
            )
        })
        .collect();
    quotas.sort();
    let page = service
        .rate_limit_keys(who, w.namespace.id(), None, None, Some(51), &ctx())
        .await
        .expect("the listing")
        .expect("a page");
    let groups = page
        .groups
        .iter()
        .map(|group| (group.grants_in_window, group.unleased_grants, group.waiters))
        .collect();
    Reading {
        slots,
        alerts,
        quotas,
        groups,
    }
}

/// The world, after the monitor's look, read by a Viewer and by a Developer given the namespace's reveal grant.
async fn readings(shared: bool, limit: u32) -> (Reading, Reading) {
    let (w, runs) = world(shared, limit).await;
    Monitor::new(runtime_store(&w).await, MonitorSettings::default())
        .run_once()
        .await;
    let viewer = w.member("vera", Role::Viewer).await;
    let dave = revealer(&w).await;
    (read(&w, &viewer, runs).await, read(&w, &dave, runs).await)
}

/// `dave`, a Developer given the namespace's reveal grant.
async fn revealer(w: &World) -> Principal {
    let dave = w.member("dave", Role::Developer).await;
    sqlx::query(
        "INSERT INTO payload_reveal_grants (tenant_id, namespace_id, subject, granted_by, granted_at) \
         VALUES ($1, $2, 'dave', 'admin', clock_timestamp())",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("the reveal grant");
    dave
}

/// Decision D022-155 (security check of the sweep, item 2): `getRun` does not read the run's place for a caller to whom
/// the key's label is hidden, since that read takes time that grows with the key's grants and waiters. Of what `getRun`
/// reads, only the place read reads `flow_control_grant_failures` (the eligibility of the waiters ahead), so another
/// transaction holds that table locked: the revealer's read waits out the place read's statement timeout and shows the
/// run without a place, and the Viewer's read, which does not try it, returns at once with the slot's limit. No sleep:
/// the lock is held, and the bound is below the place read's 2 s timeout.
#[tokio::test]
async fn a_caller_without_the_label_does_not_wait_for_the_runs_place_in_the_queue() {
    let (w, [first, _]) = world(true, 1).await;
    let viewer = w.member("vera", Role::Viewer).await;
    let dave = revealer(&w).await;
    let open = w.runs.get(&dave, first, &ctx()).await.expect("the run");
    assert_eq!(
        open.rate_limit.expect("a slot").position,
        Some(1),
        "the revealer reads the place while nothing holds it"
    );
    let mut lock = w.db.pool.begin().await.expect("a transaction");
    sqlx::query("LOCK TABLE flow_control_grant_failures IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .expect("the place read's table, held");
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        w.runs.get(&viewer, first, &ctx()),
    )
    .await
    .expect("a Viewer's read does not wait for the place read (D022-155)")
    .expect("the run");
    let slot = read.rate_limit.expect("a slot");
    assert_eq!(
        (slot.position, slot.estimated_grant_at, slot.limit),
        (None, None, 1),
        "no place, the effective limit"
    );
    let blocked = w.runs.get(&dave, first, &ctx()).await.expect("the run");
    assert_eq!(
        blocked.rate_limit.expect("a slot").position,
        None,
        "the lock holds the place read: it ran out of time and the run is shown without a place"
    );
    lock.rollback().await.expect("the lock released");
}

/// The matrix: at a key limit of 1 and of 2, a Viewer reads the same whether the two waiting runs share a key or not;
/// the member with the reveal grant reads the second waiter's true position (2 on a shared key, 1 alone), the saturated
/// key's alert only when the waiters share it, and the busiest key's waiters (2 or 1).
#[tokio::test]
async fn a_caller_without_the_reveal_right_reads_the_same_whether_runs_share_a_key() {
    for limit in [1, 2] {
        let (viewer_shared, revealer_shared) = readings(true, limit).await;
        let (viewer_apart, revealer_apart) = readings(false, limit).await;
        assert_eq!(
            viewer_shared, viewer_apart,
            "limit {limit}: a Viewer's answers depend on key sharing"
        );
        assert!(
            viewer_shared
                .slots
                .iter()
                .all(|(_, position, estimate, label)| position.is_none() && !estimate && !label),
            "limit {limit}: no position, estimate or label for a Viewer: {viewer_shared:?}"
        );
        assert!(
            viewer_shared
                .quotas
                .iter()
                .all(|(_, used, withheld)| used.is_none() && *withheld),
            "limit {limit}: {viewer_shared:?}"
        );
        assert_eq!(
            (
                revealer_shared
                    .slots
                    .iter()
                    .map(|slot| slot.1)
                    .collect::<Vec<_>>(),
                revealer_apart
                    .slots
                    .iter()
                    .map(|slot| slot.1)
                    .collect::<Vec<_>>()
            ),
            (vec![Some(1), Some(2)], vec![Some(1), Some(1)]),
            "limit {limit}: the revealer's positions"
        );
        assert_eq!(
            (
                revealer_shared
                    .alerts
                    .iter()
                    .any(|(kind, _)| kind == "RATE_LIMIT_QUEUE_SATURATED"),
                revealer_apart
                    .alerts
                    .iter()
                    .any(|(kind, _)| kind == "RATE_LIMIT_QUEUE_SATURATED"),
            ),
            (true, false),
            "limit {limit}: a key with two of its two waiters is saturated: {revealer_shared:?} {revealer_apart:?}"
        );
        let busiest = |reading: &Reading| {
            reading
                .quotas
                .iter()
                .find(|(resource, _, _)| resource == "flow_control_key_waiters")
                .and_then(|line| line.1)
        };
        assert_eq!(
            (busiest(&revealer_shared), busiest(&revealer_apart)),
            (Some(2), Some(1)),
            "limit {limit}"
        );
    }
}

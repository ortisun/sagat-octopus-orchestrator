//! Cross-run tenant concurrency: one tenant bursts several collection runs against its slot limit
//! while a second tenant runs one. The slot limit of each tenant is asserted after every scheduler
//! round against both the usage counter and the number of iterations that really hold a slot, and
//! the quiet tenant's progress is compared with the same run alone.

use std::time::{Duration, Instant};

use sagat_domain::execution::RunId;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use super::collections::{ACTIVITY, collection_document};
use super::common::{Platform, emit, machine, platform, postgres_description};

const SLOT_LIMIT: i32 = 8;

/// Rounds a worker keeps a leased task before it reports it: the activity's own duration. Without
/// it the worker finishes every task before the next round and no run ever holds more than one slot.
const ACTIVITY_ROUNDS: u32 = 8;

struct Tenant {
    held: Vec<(u32, sagat_application::worker::ClaimedTask)>,
    uuid: Uuid,
    runs: Vec<RunId>,
    session: crate::support::engine::Session,
    name: &'static str,
}

#[derive(Debug, Default, Clone)]
struct Round {
    ticks: u32,
    peak_counter: i32,
    peak_holding: i64,
    completed_round: Option<u32>,
    skipped: u64,
    tick_ms: Vec<f64>,
}

async fn usage(p: &Platform, tenant: Uuid) -> (i32, i64) {
    let row = sqlx::query(
        "SELECT u.active_iterations, \
                (SELECT count(*) FROM collection_iterations i \
                  WHERE i.tenant_id=u.tenant_id AND i.slot_held) AS holding \
         FROM tenant_iteration_usage u WHERE u.tenant_id=$1",
    )
    .bind(tenant)
    .fetch_one(&p.world.db.pool)
    .await
    .expect("tenant usage");
    (row.get("active_iterations"), row.get("holding"))
}

async fn finished(p: &Platform, tenant: &Tenant) -> bool {
    let done: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs WHERE tenant_id=$1 AND id = ANY($2) AND status='SUCCEEDED'",
    )
    .bind(tenant.uuid)
    .bind(tenant.runs.iter().map(|r| r.as_uuid()).collect::<Vec<_>>())
    .fetch_one(&p.world.db.pool)
    .await
    .expect("finished runs");
    usize::try_from(done).expect("count") == tenant.runs.len()
}

async fn work(p: &Platform, tenant: &mut Tenant, round: u32) {
    loop {
        let tasks = p.poll(&tenant.session, 64).await;
        if tasks.is_empty() {
            break;
        }
        tenant
            .held
            .extend(tasks.into_iter().map(|task| (round, task)));
    }
    let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut tenant.held)
        .into_iter()
        .partition(|(leased, _)| round >= leased + ACTIVITY_ROUNDS);
    tenant.held = later;
    for (_, task) in due {
        let index = task.instance.path().segments()[0].iteration_index();
        p.complete(&tenant.session, &task, &json!({ "v": index }))
            .await;
    }
}

/// Runs every tenant's collections to completion with the production scheduler tick, a worker per
/// tenant, and returns one `Round` per tenant.
async fn drive(p: &Platform, tenants: &mut [Tenant]) -> Vec<Round> {
    let mut rounds = vec![Round::default(); tenants.len()];
    let mut skipped = 0_u64;
    let started = Instant::now();
    let budget = super::common::budget();
    while started.elapsed() < budget {
        let at = Instant::now();
        let report = p.scheduler.tick().await;
        let tick = at.elapsed();
        assert_eq!(report.failed, 0, "scheduler failure: {report:?}");
        skipped += u64::from(report.skipped);
        for (index, tenant) in tenants.iter_mut().enumerate() {
            work(p, tenant, rounds[index].ticks).await;
            let (counter, holding) = usage(p, tenant.uuid).await;
            assert!(
                counter <= SLOT_LIMIT && holding <= i64::from(SLOT_LIMIT),
                "{} exceeded its slot limit: counter {counter}, holding {holding}, limit {SLOT_LIMIT}",
                tenant.name
            );
            assert_eq!(
                i64::from(counter),
                holding,
                "{}: the usage counter and the real slot holders disagree",
                tenant.name
            );
            let round = &mut rounds[index];
            round.ticks += 1;
            round.peak_counter = round.peak_counter.max(counter);
            round.peak_holding = round.peak_holding.max(holding);
            round.tick_ms.push(super::duration_ms(tick));
            if round.completed_round.is_none() && finished(p, tenant).await {
                round.completed_round = Some(round.ticks);
            }
        }
        if rounds.iter().all(|r| r.completed_round.is_some()) {
            break;
        }
    }
    for round in &mut rounds {
        round.skipped = skipped;
    }
    rounds
}

async fn tenant_a(p: &Platform, runs: usize, items: usize) -> Tenant {
    let mut ids = Vec::new();
    for index in 0..runs {
        let (workflow, _) = p
            .world
            .publish(collection_document(
                &format!("perf-burst-{index}-{}", Uuid::now_v7()),
                items,
                usize::try_from(SLOT_LIMIT).expect("limit"),
            ))
            .await;
        ids.push(
            p.world
                .start(
                    workflow,
                    &json!({ "items": (0..items).collect::<Vec<_>>() }),
                )
                .await,
        );
    }
    Tenant {
        held: Vec::new(),
        uuid: p.world.admin.tenant_id().as_uuid(),
        runs: ids,
        session: p.session(&[ACTIVITY]).await,
        name: "bursting tenant",
    }
}

async fn tenant_b(p: &Platform, items: usize) -> Tenant {
    let workflow = p
        .world
        .publish_as_other_tenant(collection_document(
            &format!("perf-quiet-{}", Uuid::now_v7()),
            items,
            usize::try_from(SLOT_LIMIT).expect("limit"),
        ))
        .await;
    let run = p
        .world
        .start_as_other_tenant(
            workflow,
            &json!({ "items": (0..items).collect::<Vec<_>>() }),
        )
        .await
        .expect("start the quiet tenant's run");
    let principal = p.other_tenant_worker().await;
    Tenant {
        held: Vec::new(),
        uuid: p.world.other_admin.tenant_id().as_uuid(),
        runs: vec![run],
        session: p.session_for(&principal, &[ACTIVITY]).await,
        name: "quiet tenant",
    }
}

async fn set_limits(p: &Platform) {
    for tenant in [p.world.admin.tenant_id(), p.world.other_admin.tenant_id()] {
        sqlx::query(
            "INSERT INTO tenant_collection_policies \
             (tenant_id,max_active_iterations,updated_by) VALUES ($1,$2,'capacity-benchmark') \
             ON CONFLICT (tenant_id) DO UPDATE SET max_active_iterations=EXCLUDED.max_active_iterations",
        )
        .bind(tenant.as_uuid())
        .bind(SLOT_LIMIT)
        .execute(&p.world.db.pool)
        .await
        .expect("tenant slot policy");
    }
}

fn percentile_row(label: &str, samples: &[f64]) -> String {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    format!(
        "| {label} | {} | {:.1} ms | {:.1} ms | {:.1} ms | {:.1} ms |\n",
        sorted.len(),
        super::percentile(&sorted, 0.50),
        super::percentile(&sorted, 0.95),
        super::percentile(&sorted, 0.99),
        sorted.last().copied().unwrap_or(0.0)
    )
}

#[tokio::test]
#[ignore = "capacity benchmark: run explicitly with --ignored"]
async fn measure_cross_run_tenant_slot_bounds_with_a_bursting_neighbour() {
    let burst_runs = super::common::list_env("SAGAT_PERF_BURST_RUNS", &[6])[0];
    let items = super::common::list_env("SAGAT_PERF_TENANT_ITEMS", &[40])[0];

    // Control: the quiet tenant alone.
    let alone = {
        let p = platform().await;
        set_limits(&p).await;
        let mut b = tenant_b(&p, items).await;
        let rounds = drive(&p, std::slice::from_mut(&mut b)).await;
        rounds[0].clone()
    };

    // Contention: the bursting tenant has `burst_runs` runs, each allowed the whole slot limit.
    let p = platform().await;
    set_limits(&p).await;
    let header = format!("{}\n{}\n", machine(), postgres_description(&p.world).await);
    let a = tenant_a(&p, burst_runs, items).await;
    let b = tenant_b(&p, items).await;
    let started = Instant::now();
    let rounds = drive(&p, &mut [a, b]).await;
    let wall = started.elapsed();
    let (burst, quiet) = (&rounds[0], &rounds[1]);

    let alone_rounds = alone.completed_round.expect("the control run completes");
    let quiet_rounds = quiet
        .completed_round
        .expect("the quiet tenant must finish while its neighbour bursts");
    let burst_rounds = burst
        .completed_round
        .expect("the bursting tenant must finish");
    // The test's own starvation bound, not a product promise: sharing the scheduler with a tenant
    // that has six times the runs must not cost the quiet tenant more than 4x its solo rounds.
    assert!(
        quiet_rounds <= alone_rounds * 4,
        "the quiet tenant needed {quiet_rounds} rounds beside a burst against {alone_rounds} alone"
    );
    assert_eq!(
        burst.peak_counter, SLOT_LIMIT,
        "the burst saturated its slots"
    );
    assert!(quiet.peak_counter <= SLOT_LIMIT);

    let mut table = String::from(
        "| Operation | Samples | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|---:|\n",
    );
    table.push_str(&percentile_row(
        "scheduler tick while both tenants are active",
        &burst.tick_ms,
    ));
    table.push_str(&percentile_row(
        "scheduler tick, quiet tenant alone (control)",
        &alone.tick_ms,
    ));
    emit(
        "tenant-bounds",
        &format!(
            "{header}### Two tenants, one bursting\n\n\
             - Configuration: slot limit {SLOT_LIMIT} per tenant; bursting tenant {burst_runs} runs x {items} items, each run `maxConcurrency` {SLOT_LIMIT} (demand {demand} against {SLOT_LIMIT} slots); quiet tenant 1 run x {items} items; one worker per tenant that reports each leased task {ACTIVITY_ROUNDS} rounds after leasing it; scheduler `tick()` rounds (batch 200, fair, concurrency 8).\n\
             - Slot bound asserted after every round for both tenants against the usage counter and against the count of iterations that really hold a slot (`slot_held`): peak counter {a_counter} / holding {a_holding} (bursting), {b_counter} / {b_holding} (quiet), limit {SLOT_LIMIT}. The two figures agreed in every round.\n\
             - Quiet tenant: finished in {quiet_rounds} rounds beside the burst, {alone_rounds} alone (ratio {ratio:.2}); bound asserted: at most 4x.\n\
             - Bursting tenant: finished in {burst_rounds} rounds; total wall {wall:.1} s. Scheduler actions skipped for a typed reason (tenant at capacity or revision conflict, no client retry involved): {skipped}.\n\n\
             {table}\n",
            demand = burst_runs * usize::try_from(SLOT_LIMIT).expect("limit"),
            a_counter = burst.peak_counter,
            a_holding = burst.peak_holding,
            b_counter = quiet.peak_counter,
            b_holding = quiet.peak_holding,
            ratio = f64::from(quiet_rounds) / f64::from(alone_rounds),
            wall = wall.as_secs_f64(),
            skipped = burst.skipped,
        ),
    );
    let _ = Duration::ZERO;
}

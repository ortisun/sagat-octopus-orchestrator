//! Bounded collections: N items at a configured `maxConcurrency`, driven to completion through the
//! public publish/start path, the production scheduler transition (`advance_collection`) and a
//! protocol 1.5 worker. Every durable transition is timed; the tenant slot bound is asserted at
//! every sample, not only reported.

use std::time::{Duration, Instant};

use sagat_domain::execution::{ExecutionInstanceId, RunStatus};
use sagat_domain::workflow::NodeId;
use sagat_scheduler::CollectionAdvance;
use serde_json::{Value, json};
use uuid::Uuid;

use super::common::{
    Platform, Probe, Samples, TABLE_HEAD, analyze, budget, early_late_rows, emit, list_env,
    machine, platform, postgres_description, probe_table, resources, statements,
};

/// Planner statistics are refreshed every this many transitions (what autovacuum does).
pub const ANALYZE_EVERY: usize = 50;

pub const ACTIVITY: &str = "perf.item";

pub(super) fn probe_transition(sequence: usize, stride: usize) -> bool {
    // Two consecutive transitions cover both alternating admission/checkpoint families even
    // when the reporting interval is even. A stride of one probes every transition.
    sequence >= 2 && sequence % stride < 2
}

pub fn collection_document(name: &str, items: usize, concurrency: usize) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "items",
        "bodies": {
            "line": {
                "entrypoint": "work",
                "nodes": [
                    { "id": "work", "type": "activity", "activity": ACTIVITY, "next": "returned" },
                    { "id": "returned", "type": "return", "result": "steps.work.output" }
                ]
            }
        },
        "nodes": [
            {
                "id": "items",
                "type": "forEach",
                "items": "input.items",
                "body": "line",
                "result": "bodyResult",
                "maxItems": items,
                "maxConcurrency": concurrency,
                "next": "done"
            },
            { "id": "done", "type": "end" }
        ]
    })
}

/// Raises every per-tenant ceiling that would otherwise measure the default quota instead of the
/// engine; each is a configured limit and is recorded in the report.
pub async fn open_tenant_capacity(p: &Platform, slots: usize) {
    let tenant = p.world.admin.tenant_id().as_uuid();
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
         (tenant_id,max_active_iterations,updated_by) VALUES ($1,$2,'capacity-benchmark') \
         ON CONFLICT (tenant_id) DO UPDATE SET max_active_iterations=EXCLUDED.max_active_iterations, \
           updated_by=EXCLUDED.updated_by,updated_at=clock_timestamp()",
    )
    .bind(tenant)
    .bind(i32::try_from(slots).expect("slot limit"))
    .execute(&p.world.db.pool)
    .await
    .expect("tenant slot policy");
}

#[derive(Debug)]
pub struct CollectionResult {
    pub items: usize,
    pub concurrency: usize,
    pub complete: bool,
    pub wall: Duration,
    pub transitions: usize,
    pub samples: Samples,
    pub decile_p50_ms: Vec<f64>,
    pub peak_active: i32,
    pub errors: usize,
    pub before: String,
    pub after: String,
    pub peak_threads: u64,
    pub peak_connections: i64,
    pub peak_rss_bytes: u64,
    pub journal_bytes_per_item: f64,
    pub step_rows: i64,
    pub checkpoints: i64,
    pub status: RunStatus,
    pub header: String,
    pub probes: Vec<Probe>,
}

pub async fn run_case(items: usize, concurrency: usize) -> CollectionResult {
    let p = platform().await;
    open_tenant_capacity(&p, concurrency.max(16)).await;
    let tenant = p.world.admin.tenant_id();
    let name = format!("perf-collection-{}", Uuid::now_v7());
    let (workflow, _) = p
        .world
        .publish(collection_document(&name, items, concurrency))
        .await;
    let input = json!({ "items": (0..items).collect::<Vec<_>>() });
    let run = p.world.start(workflow, &input).await;
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
    let session = p.session(&[ACTIVITY]).await;

    let header = format!("{}\n{}\n", machine(), postgres_description(&p.world).await);
    let before = resources(&p.world).await;
    let mut peak_threads = before.threads;
    let mut peak_connections = before.pg_connections;
    let mut peak_rss_bytes = before.rss_bytes;
    let mut samples = Samples::default();
    let mut sequence: Vec<Duration> = Vec::new();
    let mut peak_active = 0_i32;
    let mut errors = 0_usize;
    let mut transitions = 0_usize;
    let budget = budget();
    let started = Instant::now();
    let mut idle = 0_u32;
    let mut complete = false;
    let mut probes = Vec::new();
    let probe_every = (items * 2 / 12).max(1);
    loop {
        if started.elapsed() > budget {
            break;
        }
        if transitions % ANALYZE_EVERY == 1 {
            analyze(&p.world).await;
        }
        let probing = probe_transition(transitions, probe_every);
        let cost_before = if probing {
            statements(&p.world).await
        } else {
            None
        };
        let at = Instant::now();
        let outcome = p.scheduler.advance_collection(tenant, run, &owner).await;
        let elapsed = at.elapsed();
        if probing
            && matches!(
                outcome,
                Ok(CollectionAdvance::Admitted { .. } | CollectionAdvance::Checkpointed { .. })
            )
            && let (Some(cost_before), Some(cost_after)) = (cost_before, statements(&p.world).await)
        {
            probes.push(Probe {
                label: match outcome {
                    Ok(CollectionAdvance::Admitted { .. }) => "admit".to_owned(),
                    _ => "checkpoint".to_owned(),
                },
                sequence: transitions,
                elapsed,
                cost: cost_after - cost_before,
            });
        }
        let admitted = matches!(&outcome, Ok(CollectionAdvance::Admitted { .. }));
        match outcome {
            Ok(CollectionAdvance::Waiting) => {
                samples.record(
                    "scheduler: nothing to advance (waiting for a worker)",
                    elapsed,
                );
                let mut worked = 0;
                loop {
                    let polled = Instant::now();
                    let tasks = p.poll(&session, 64).await;
                    if tasks.is_empty() {
                        samples.record("worker: poll with nothing ready", polled.elapsed());
                        break;
                    }
                    samples.record("worker: poll that leased tasks", polled.elapsed());
                    for delay in p.ready_queue_delays(&session, &tasks).await {
                        samples.record("worker: ready-to-lease queue delay", delay);
                    }
                    for task in tasks {
                        let index = task.instance.path().segments()[0].iteration_index();
                        let completed = Instant::now();
                        p.complete(&session, &task, &json!({ "v": index })).await;
                        samples.record("worker: complete one scoped task", completed.elapsed());
                        worked += 1;
                    }
                }
                if worked == 0 {
                    idle += 1;
                    assert!(idle < 200, "the collection stalled with no ready work");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                } else {
                    idle = 0;
                }
            }
            Ok(CollectionAdvance::Finished | CollectionAdvance::AlreadyFinished) => {
                samples.record("scheduler: finish the collection", elapsed);
                transitions += 1;
                complete = true;
                break;
            }
            Ok(CollectionAdvance::Entered) => {
                samples.record("scheduler: enter (snapshot the items)", elapsed);
                transitions += 1;
            }
            Ok(CollectionAdvance::Admitted { .. }) => {
                samples.record("scheduler: admit one iteration", elapsed);
                sequence.push(elapsed);
                transitions += 1;
            }
            Ok(CollectionAdvance::Checkpointed { .. }) => {
                samples.record("scheduler: checkpoint one iteration", elapsed);
                sequence.push(elapsed);
                transitions += 1;
            }
            Ok(other) => panic!("unexpected collection transition {other:?}"),
            Err(error) => {
                errors += 1;
                samples.record("scheduler: transition returned an error", elapsed);
                assert!(errors < 25, "too many scheduler errors, last: {error}");
            }
        }
        if admitted || transitions.is_multiple_of(8) {
            let (active, limit): (i32, i32) = sqlx::query_as(
                "SELECT u.active_iterations, p.max_active_iterations \
                 FROM tenant_iteration_usage u JOIN tenant_collection_policies p USING (tenant_id) \
                 WHERE u.tenant_id = $1",
            )
            .bind(tenant.as_uuid())
            .fetch_one(&p.world.db.pool)
            .await
            .expect("tenant usage");
            assert!(active <= limit, "tenant slots exceeded: {active} > {limit}");
            let scope_active: i32 = sqlx::query_scalar(
                "SELECT active_count FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&p.world.db.pool)
            .await
            .expect("scope active count");
            peak_active = peak_active.max(scope_active);
            assert!(
                usize::try_from(scope_active).unwrap_or(usize::MAX) <= concurrency,
                "maxConcurrency exceeded: {scope_active} > {concurrency}"
            );
        }
        if transitions.is_multiple_of(256) {
            let now = resources(&p.world).await;
            peak_threads = peak_threads.max(now.threads);
            peak_connections = peak_connections.max(now.pg_connections);
            peak_rss_bytes = peak_rss_bytes.max(now.rss_bytes);
        }
    }
    let wall = started.elapsed();
    let after = resources(&p.world).await;
    peak_threads = peak_threads.max(after.threads);
    peak_connections = peak_connections.max(after.pg_connections);
    peak_rss_bytes = peak_rss_bytes.max(after.rss_bytes);
    let status = p.world.run_status(run).await;
    if complete {
        assert_eq!(status, RunStatus::Succeeded);
        p.world.assert_consistent(run).await;
    }
    let journal_bytes: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(pg_column_size(e.*)),0)::bigint FROM run_events e \
         WHERE e.tenant_id=$1 AND e.run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.world.db.pool)
    .await
    .expect("journal bytes");
    let step_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2")
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&p.world.db.pool)
            .await
            .expect("step rows");
    let checkpoints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
           AND event_type='collection.iteration_checkpointed'",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.world.db.pool)
    .await
    .expect("checkpoints");
    if complete {
        assert_eq!(checkpoints, i64::try_from(items).expect("items"));
    }
    CollectionResult {
        items,
        concurrency,
        complete,
        wall,
        transitions,
        decile_p50_ms: deciles(&sequence),
        samples,
        peak_active,
        errors,
        before: before.to_string(),
        after: after.to_string(),
        peak_threads,
        peak_connections,
        peak_rss_bytes,
        journal_bytes_per_item: journal_bytes as f64
            / if complete {
                items as f64
            } else {
                checkpoints.max(1) as f64
            },
        step_rows,
        checkpoints,
        status,
        header,
        probes,
    }
}

/// p50 latency of the admit/checkpoint transitions in each tenth of the run, in order. A flat
/// series means constant cost per transition; a rising one means the cost grows with progress.
pub fn deciles(sequence: &[Duration]) -> Vec<f64> {
    if sequence.len() < 10 {
        return Vec::new();
    }
    let size = sequence.len() / 10;
    (0..10)
        .map(|bucket| {
            let slice = &sequence[bucket * size..if bucket == 9 {
                sequence.len()
            } else {
                (bucket + 1) * size
            }];
            let mut sorted: Vec<f64> = slice.iter().map(|d| super::duration_ms(*d)).collect();
            sorted.sort_by(f64::total_cmp);
            super::percentile(&sorted, 0.50)
        })
        .collect()
}

pub fn fragment(result: &CollectionResult) -> String {
    let wall = result.wall.as_secs_f64();
    let work_summary = ["admit", "checkpoint"]
        .map(|kind| {
            let probes: Vec<_> = result
                .probes
                .iter()
                .filter(|probe| probe.label == kind)
                .cloned()
                .collect();
            match early_late_rows(&probes, 2) {
                Some((early, late)) => {
                    format!("- {kind}: first/last two-probe median rows {early}/{late}.")
                }
                None => format!("- {kind}: insufficient probes for an early/late row comparison."),
            }
        })
        .join("\n");
    format!(
        "### {items} items, maxConcurrency {c}{partial}\n\n\
         - Outcome: run status {status:?}, {complete}; wall {wall:.1} s, {transitions} scheduler transitions, {tput:.1} items/s (items checkpointed per second), {tps:.1} transitions/s.\n\
         - Peak active iterations observed (after every admission and every 8 transitions): {peak}; configured maxConcurrency {c}; tenant policy slots {slots}. Both bounds were asserted at every sample.\n\
         - Scheduler errors returned by a transition: {errors}. No client retries.\n\
         - Resources before: {before}.\n\
         - Resources after: {after}. Peak threads {threads}, peak PostgreSQL connections {conns} (sampled every 256 transitions).\n\
         - Sampled peak RSS: {rss:.1} MiB (before/after and every 256 transitions; not the OS process peak).\n\
         - Durable size: {journal:.0} journal bytes per item (`pg_column_size`, no index/toast), {steps} step rows, {cps} checkpoints.\n\
         - p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: {deciles}\n\n\
         {head}{rows}\nWork by transition family (families are never mixed in these medians):\n\n{work_summary}\n\nWhat the database did for single sampled transitions:\n\n{probes}\n",
        items = result.items,
        c = result.concurrency,
        partial = if result.complete {
            ""
        } else {
            " (BUDGET EXHAUSTED, NOT COMPLETED)"
        },
        status = result.status,
        complete = if result.complete {
            "completed"
        } else {
            "NOT completed within the time budget"
        },
        transitions = result.transitions,
        tput = (if result.complete {
            i64::try_from(result.items).unwrap_or(0)
        } else {
            result.checkpoints
        }) as f64
            / wall.max(f64::EPSILON),
        tps = result.transitions as f64 / wall.max(f64::EPSILON),
        peak = result.peak_active,
        slots = result.concurrency.max(16),
        errors = result.errors,
        before = result.before,
        after = result.after,
        threads = result.peak_threads,
        conns = result.peak_connections,
        rss = result.peak_rss_bytes as f64 / 1_048_576.0,
        journal = result.journal_bytes_per_item,
        steps = result.step_rows,
        cps = result.checkpoints,
        deciles = if result.decile_p50_ms.is_empty() {
            "n/a".to_owned()
        } else {
            result
                .decile_p50_ms
                .iter()
                .map(|v| format!("{v:.1}"))
                .collect::<Vec<_>>()
                .join(", ")
        },
        head = TABLE_HEAD,
        rows = result.samples.rows(),
        probes = probe_table(&result.probes),
    )
}

#[tokio::test]
#[ignore = "capacity benchmark: run explicitly with --ignored"]
async fn measure_collection_fanout() {
    let items = list_env("SAGAT_PERF_COLLECTION_ITEMS", &[100, 1_000]);
    let concurrency = list_env("SAGAT_PERF_COLLECTION_CONCURRENCY", &[1, 16, 256]);
    for count in items {
        for width in &concurrency {
            let width = (*width).min(count);
            let result = run_case(count, width).await;
            emit(
                &format!("collection-{count}-c{width}"),
                &format!("{}\n{}", result.header, fragment(&result)),
            );
        }
    }
}

/// `SAGAT_PERF_REFERENCE_PATH=1` drives the transitions through the whole-run path instead of the
/// frontier: the control that shows these measurements can see the growth the frontier removed.
pub fn reference_path() -> bool {
    std::env::var("SAGAT_PERF_REFERENCE_PATH").is_ok_and(|value| value == "1")
}

/// Drives a collection of `items` to its end through the production transition and returns what
/// the database did for every admission and checkpoint (rows and buffer accesses, which do not
/// depend on how loaded the host is). Needs a server that preloads `pg_stat_statements`; see the
/// report for the container command.
pub async fn probe_collection(items: usize, concurrency: usize) -> Vec<Probe> {
    let p = platform().await;
    open_tenant_capacity(&p, 16).await;
    let tenant = p.world.admin.tenant_id();
    let (workflow, _) = p
        .world
        .publish(collection_document(
            &format!("perf-flat-{}", Uuid::now_v7()),
            items,
            concurrency,
        ))
        .await;
    let run = p
        .world
        .start(
            workflow,
            &json!({ "items": (0..items).collect::<Vec<_>>() }),
        )
        .await;
    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
    let session = p.session(&[ACTIVITY]).await;
    assert!(
        statements(&p.world).await.is_some(),
        "this test needs a server with pg_stat_statements preloaded"
    );
    let mut probes = Vec::new();
    let mut transitions = 0_usize;
    loop {
        if transitions % ANALYZE_EVERY == 1 {
            analyze(&p.world).await;
        }
        let before = statements(&p.world).await.expect("statements");
        let at = Instant::now();
        let outcome = if reference_path() {
            p.scheduler
                .advance_collection_from_run(tenant, run, &owner)
                .await
        } else {
            p.scheduler.advance_collection(tenant, run, &owner).await
        }
        .expect("transition");
        let elapsed = at.elapsed();
        let cost = statements(&p.world).await.expect("statements") - before;
        match outcome {
            CollectionAdvance::Admitted { .. } | CollectionAdvance::Checkpointed { .. } => {
                transitions += 1;
                probes.push(Probe {
                    label: "iteration".into(),
                    sequence: transitions,
                    elapsed,
                    cost,
                });
            }
            CollectionAdvance::Entered => transitions += 1,
            CollectionAdvance::Waiting => {
                for task in p.poll(&session, 64).await {
                    p.complete(&session, &task, &json!({ "v": 1 })).await;
                }
            }
            CollectionAdvance::Finished | CollectionAdvance::AlreadyFinished => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    p.world.assert_consistent(run).await;
    probes
}

/// Median `rows` and `buffers` of the transitions between two thirds and 95 percent of the run, so
/// that the two transitions that are proportional to the scope by nature (the last checkpoint and
/// the finish) are not part of the figure.
pub fn late_cost(probes: &[Probe]) -> (i64, i64) {
    let from = probes.len() * 2 / 3;
    let to = probes.len() * 95 / 100;
    let window = &probes[from..to];
    let median = |pick: fn(&Probe) -> i64| {
        let mut values: Vec<i64> = window.iter().map(pick).collect();
        values.sort_unstable();
        values[values.len() / 2]
    };
    (median(|p| p.cost.rows), median(|p| p.cost.buffers))
}

/// G461. The work of one collection transition must not depend on how many iterations the
/// collection already has, otherwise a 10,000-item collection costs quadratic database work. Before
/// the fix every transition restored the whole run (every step, every item payload, every iteration
/// and every checkpoint), so the rows returned by the median late transition were several times the
/// early one (57 -> 829 over 160 items).
#[tokio::test]
async fn red_collection_transition_rows_do_not_grow_with_progress() {
    let items = 160;
    let probes = probe_collection(items, 2).await;
    let (early, late) = early_late_rows(&probes, 10).expect("enough probes");
    assert!(
        late <= early * 2,
        "rows per transition grew from {early} (first ten) to {late} (last ten) over {items} items"
    );
}

/// G461, across sizes: quadrupling the collection must not move the cost of a late transition
/// (rows returned and buffer accesses of the median transition in the last third), within a small
/// constant. A growth proportional to the size would multiply both by four.
#[tokio::test]
async fn collection_transition_cost_is_flat_across_sizes() {
    let mut late = Vec::new();
    for items in [160, 320, 640] {
        let probes = probe_collection(items, 2).await;
        let (rows, buffers) = late_cost(&probes);
        eprintln!(
            "collection of {items}: median late transition reads {rows} rows, {buffers} buffers"
        );
        late.push((items, rows, buffers));
    }
    let (_, base_rows, base_buffers) = late[0];
    for (items, rows, buffers) in &late[1..] {
        assert!(
            *rows <= base_rows + base_rows / 5 + 4,
            "rows per transition grew with the size: {base_rows} at 160, {rows} at {items}"
        );
        assert!(
            *buffers <= base_buffers + base_buffers / 3 + 40,
            "buffer accesses per transition grew with the size: {base_buffers} at 160, {buffers} at {items}"
        );
    }
}

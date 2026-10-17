//! Sequential `while` loops: N iterations driven through the production scheduler transition
//! (`advance_loop`) and a protocol 1.5 worker, and a population of loops parked in a durable delay
//! that must hold no worker, thread or connection.

use std::time::{Duration, Instant};

use sagat_domain::execution::{ExecutionInstanceId, RunStatus};
use sagat_domain::workflow::NodeId;
use sagat_scheduler::LoopAdvance;
use serde_json::{Value, json};
use uuid::Uuid;

use super::common::{
    Probe, Samples, TABLE_HEAD, analyze, budget, early_late_rows, emit, list_env, machine,
    platform, postgres_description, probe_table, resources, statements,
};

const ACTIVITY: &str = "perf.step";

pub fn loop_document(name: &str, iterations: usize, delay_ms: Option<u64>) -> Value {
    let mut node = json!({
        "id": "poll",
        "type": "while",
        "condition": format!("carry < {iterations}"),
        "body": "iteration",
        "initial": "0",
        "carry": "bodyResult",
        "maxIterations": iterations,
        "timeoutMs": 86_400_000,
        "next": "done"
    });
    if let Some(delay) = delay_ms {
        node["delayMs"] = json!(delay);
    }
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "poll",
        "bodies": {
            "iteration": {
                "entrypoint": "step",
                "nodes": [
                    { "id": "step", "type": "activity", "activity": ACTIVITY, "next": "returned" },
                    { "id": "returned", "type": "return", "result": "steps.step.output" }
                ]
            }
        },
        "nodes": [node, { "id": "done", "type": "end" }]
    })
}

#[derive(Debug)]
pub struct LoopResult {
    pub header: String,
    pub iterations: usize,
    pub complete: bool,
    pub wall: Duration,
    pub transitions: usize,
    pub samples: Samples,
    pub decile_p50_ms: Vec<f64>,
    pub before: String,
    pub after: String,
    pub peak_threads: u64,
    pub peak_connections: i64,
    pub journal_bytes_per_iteration: f64,
    pub step_rows: i64,
    pub status: RunStatus,
    pub probes: Vec<Probe>,
}

pub async fn run_case(iterations: usize) -> LoopResult {
    let p = platform().await;
    super::collections::open_tenant_capacity(&p, 16).await;
    let tenant = p.world.admin.tenant_id();
    let name = format!("perf-loop-{}", Uuid::now_v7());
    let (workflow, _) = p
        .world
        .publish(loop_document(&name, iterations, None))
        .await;
    let run = p.world.start(workflow, &json!({})).await;
    let owner = ExecutionInstanceId::root(NodeId::parse("poll").expect("owner"));
    let session = p.session(&[ACTIVITY]).await;
    let header = format!("{}\n{}\n", machine(), postgres_description(&p.world).await);
    let before = resources(&p.world).await;
    let (mut peak_threads, mut peak_connections) = (before.threads, before.pg_connections);
    let mut samples = Samples::default();
    let mut sequence = Vec::new();
    let mut transitions = 0_usize;
    let budget = budget();
    let started = Instant::now();
    let mut idle = 0_u32;
    let mut complete = false;
    let mut errors = 0_u32;
    let mut probes = Vec::new();
    let probe_every = (iterations / 10).max(1);
    loop {
        if started.elapsed() > budget {
            break;
        }
        if transitions % super::collections::ANALYZE_EVERY == 1 {
            analyze(&p.world).await;
        }
        let probing = transitions >= 2 && transitions.is_multiple_of(probe_every);
        let cost_before = if probing {
            statements(&p.world).await
        } else {
            None
        };
        let at = Instant::now();
        let outcome = p.scheduler.advance_loop(tenant, run, &owner, None).await;
        let elapsed = at.elapsed();
        if probing
            && matches!(outcome, Ok(LoopAdvance::Checkpointed { .. }))
            && let (Some(cost_before), Some(cost_after)) = (cost_before, statements(&p.world).await)
        {
            probes.push(Probe {
                label: "checkpoint and admit".to_owned(),
                sequence: transitions,
                elapsed,
                cost: cost_after - cost_before,
            });
        }
        match outcome {
            Ok(LoopAdvance::Waiting) => {
                samples.record(
                    "scheduler: nothing to advance (waiting for a worker)",
                    elapsed,
                );
                let mut worked = 0;
                loop {
                    let polled = Instant::now();
                    let tasks = p.poll(&session, 8).await;
                    if tasks.is_empty() {
                        samples.record("worker: poll with nothing ready", polled.elapsed());
                        break;
                    }
                    samples.record("worker: poll that leased a task", polled.elapsed());
                    for task in tasks {
                        let index = task.instance.path().segments()[0].iteration_index();
                        let completed = Instant::now();
                        p.complete(&session, &task, &json!(index + 1)).await;
                        samples.record("worker: complete one scoped task", completed.elapsed());
                        worked += 1;
                    }
                }
                if worked == 0 {
                    idle += 1;
                    assert!(idle < 200, "the loop stalled with no ready work");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                } else {
                    idle = 0;
                }
            }
            Ok(LoopAdvance::AlreadyFinished) => {
                complete = true;
                break;
            }
            Ok(LoopAdvance::Entered) => {
                samples.record("scheduler: enter and admit the first body", elapsed);
                transitions += 1;
            }
            Ok(LoopAdvance::Checkpointed { .. }) => {
                samples.record(
                    "scheduler: checkpoint an iteration and admit the next",
                    elapsed,
                );
                sequence.push(elapsed);
                transitions += 1;
                if p.world.run_status(run).await.is_terminal() {
                    complete = true;
                    break;
                }
            }
            Ok(other) => panic!("unexpected loop transition {other:?}"),
            Err(error) => {
                errors += 1;
                assert!(errors < 25, "too many scheduler errors, last: {error}");
            }
        }
        if transitions.is_multiple_of(256) {
            let now = resources(&p.world).await;
            peak_threads = peak_threads.max(now.threads);
            peak_connections = peak_connections.max(now.pg_connections);
        }
    }
    let wall = started.elapsed();
    let after = resources(&p.world).await;
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
    LoopResult {
        header,
        iterations,
        complete,
        wall,
        transitions,
        decile_p50_ms: super::collections::deciles(&sequence),
        samples,
        before: before.to_string(),
        after: after.to_string(),
        peak_threads: peak_threads.max(after.threads),
        peak_connections: peak_connections.max(after.pg_connections),
        journal_bytes_per_iteration: journal_bytes as f64
            / if complete {
                iterations as f64
            } else {
                transitions.saturating_sub(1).max(1) as f64
            },
        step_rows,
        status,
        probes,
    }
}

pub fn fragment(r: &LoopResult) -> String {
    let wall = r.wall.as_secs_f64();
    format!(
        "### {n} sequential iterations{partial}\n\n\
         - Outcome: run status {status:?}, {done}; wall {wall:.1} s, {t} scheduler transitions, {ips:.1} iterations/s (iterations checkpointed per second).\n\
         - Resources before: {before}.\n\
         - Resources after: {after}. Peak threads {threads}, peak PostgreSQL connections {conns}.\n\
         - Durable size: {journal:.0} journal bytes per iteration (`pg_column_size`, no index/toast), {steps} step rows.\n\
         - p50 (ms) of checkpoint/admit transitions by tenth of the run, first to last: {deciles}\n\n\
         {head}{rows}\nWhat the database did for single sampled transitions (load-independent):\n\n{probes}\n",
        n = r.iterations,
        partial = if r.complete {
            ""
        } else {
            " (BUDGET EXHAUSTED, NOT COMPLETED)"
        },
        status = r.status,
        done = if r.complete {
            "completed"
        } else {
            "NOT completed within the time budget"
        },
        t = r.transitions,
        ips = (if r.complete {
            r.iterations
        } else {
            r.transitions.saturating_sub(1)
        }) as f64
            / wall.max(f64::EPSILON),
        before = r.before,
        after = r.after,
        threads = r.peak_threads,
        conns = r.peak_connections,
        journal = r.journal_bytes_per_iteration,
        steps = r.step_rows,
        deciles = if r.decile_p50_ms.is_empty() {
            "n/a".to_owned()
        } else {
            r.decile_p50_ms
                .iter()
                .map(|v| format!("{v:.1}"))
                .collect::<Vec<_>>()
                .join(", ")
        },
        head = TABLE_HEAD,
        rows = r.samples.rows(),
        probes = probe_table(&r.probes),
    )
}

#[tokio::test]
#[ignore = "capacity benchmark: run explicitly with --ignored"]
async fn measure_sequential_loops() {
    for iterations in list_env("SAGAT_PERF_LOOP_ITERATIONS", &[100, 1_000]) {
        let result = run_case(iterations).await;
        emit(
            &format!("loop-{iterations}"),
            &format!("{}\n{}", result.header, fragment(&result)),
        );
    }
}

/// Drives a sequential loop of `iterations` to its end through the production transition and
/// returns what the database did for every checkpoint.
pub async fn probe_loop(iterations: usize) -> Vec<Probe> {
    let p = platform().await;
    super::collections::open_tenant_capacity(&p, 16).await;
    let tenant = p.world.admin.tenant_id();
    let (workflow, _) = p
        .world
        .publish(loop_document(
            &format!("perf-flat-loop-{}", Uuid::now_v7()),
            iterations,
            None,
        ))
        .await;
    let run = p.world.start(workflow, &json!({})).await;
    let owner = ExecutionInstanceId::root(NodeId::parse("poll").expect("owner"));
    let session = p.session(&[ACTIVITY]).await;
    assert!(
        statements(&p.world).await.is_some(),
        "this test needs a server with pg_stat_statements preloaded"
    );
    let mut probes = Vec::new();
    let mut transitions = 0_usize;
    loop {
        if transitions % super::collections::ANALYZE_EVERY == 1 {
            analyze(&p.world).await;
        }
        let before = statements(&p.world).await.expect("statements");
        let at = Instant::now();
        let outcome = if super::collections::reference_path() {
            p.scheduler
                .advance_loop_from_run(tenant, run, &owner, None)
                .await
        } else {
            p.scheduler.advance_loop(tenant, run, &owner, None).await
        }
        .expect("transition");
        let elapsed = at.elapsed();
        let cost = statements(&p.world).await.expect("statements") - before;
        match outcome {
            LoopAdvance::Checkpointed { .. } => {
                transitions += 1;
                probes.push(Probe {
                    label: "iteration".into(),
                    sequence: transitions,
                    elapsed,
                    cost,
                });
                if p.world.run_status(run).await.is_terminal() {
                    break;
                }
            }
            LoopAdvance::Entered => transitions += 1,
            LoopAdvance::Waiting => {
                for task in p.poll(&session, 8).await {
                    let index = task.instance.path().segments()[0].iteration_index();
                    p.complete(&session, &task, &json!(index + 1)).await;
                }
            }
            LoopAdvance::AlreadyFinished => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    p.world.assert_consistent(run).await;
    probes
}

/// G461. The same property for sequential loops: the work of one iteration transition must not
/// depend on the number of iterations already done. Before the fix it grew by about five rows per
/// completed iteration (91 -> 841 over 160 iterations).
#[tokio::test]
async fn red_loop_transition_rows_do_not_grow_with_progress() {
    let iterations = 160;
    let probes = probe_loop(iterations).await;
    let (early, late) = early_late_rows(&probes, 10).expect("enough probes");
    assert!(
        late <= early * 2,
        "rows per transition grew from {early} (first ten) to {late} (last ten) over {iterations} iterations"
    );
}

/// G461, across sizes (see `collection_transition_cost_is_flat_across_sizes`).
#[tokio::test]
async fn loop_transition_cost_is_flat_across_sizes() {
    let mut late = Vec::new();
    for iterations in [160, 320, 640] {
        let probes = probe_loop(iterations).await;
        let (rows, buffers) = super::collections::late_cost(&probes);
        eprintln!(
            "loop of {iterations}: median late transition reads {rows} rows, {buffers} buffers"
        );
        late.push((iterations, rows, buffers));
    }
    let (_, base_rows, base_buffers) = late[0];
    for (iterations, rows, buffers) in &late[1..] {
        assert!(
            *rows <= base_rows + base_rows / 5 + 4,
            "rows per transition grew with the size: {base_rows} at 160, {rows} at {iterations}"
        );
        assert!(
            *buffers <= base_buffers + base_buffers / 3 + 40,
            "buffer accesses per transition grew with the size: {base_buffers} at 160, {buffers} at {iterations}"
        );
    }
}

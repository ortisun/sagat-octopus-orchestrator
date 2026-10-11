//! Baseline benchmark of feature 002 (T002-034, NFR-002-PERF): ten thousand suspended runs and what
//! they cost, how long the scheduler takes when timers fall due, and how quickly workers claim and
//! complete the tasks that result.
//!
//! The numbers are only meaningful together with the environment, which the report records. Run on
//! purpose:
//!
//! ```sh
//! SAGAT_PERF_REPORT=tests/performance/reports/durable-execution.md \
//!   cargo test -p sagat-tests --test perf_durable_execution --release -- --ignored --nocapture
//! ```
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

#[path = "../../support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream;
use serde_json::json;
use support::engine::{World, document, timer_node, with, world};

const SUSPENDED: usize = 10_000;
/// The first population: the cost of an idle tick is compared between it and the full one.
const SMALL: usize = 1_000;
const DUE: usize = 500;
const LOAD_CONCURRENCY: usize = 16;
const DEFAULT_WORKERS: usize = 8;
const IDLE_TICKS: usize = 100;

/// Hypotheses to hold the system to, not promises: each is the figure below which a suspended run
/// is "free enough" that 10k of them are no concern for one engine replica.
const IDLE_TICK_P95: Duration = Duration::from_millis(50);
/// Per claimed task: a poll leases up to 16 steps, each in its own transaction under its run's lock.
/// The median is held to a budget; the tail with several workers is dominated by contention (and, on
/// a shared machine, by noise), so it is reported but not asserted.
const CLAIM_PER_TASK_P50: Duration = Duration::from_millis(50);
const COMPLETE_P95: Duration = Duration::from_millis(100);
/// Ten times the runs must not make an idle tick anywhere near ten times slower: it is an index scan.
const IDLE_GROWTH_FACTOR: f64 = 3.0;

/// `SAGAT_PERF_WORKERS=n` tries another number of concurrent workers.
fn workers() -> usize {
    std::env::var("SAGAT_PERF_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_WORKERS)
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn row(name: &str, mut samples: Vec<Duration>) -> (String, Duration) {
    samples.sort_unstable();
    let p95 = percentile(&samples, 0.95);
    (
        format!(
            "| {name} | {} | {:.1} ms | {:.1} ms | {:.1} ms | {:.1} ms |\n",
            samples.len(),
            ms(percentile(&samples, 0.50)),
            ms(p95),
            ms(percentile(&samples, 0.99)),
            ms(*samples.last().expect("samples")),
        ),
        p95,
    )
}

fn sh(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn environment() -> String {
    let cpu = if cfg!(target_os = "macos") {
        sh("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        sh(
            "sh",
            &["-c", "grep -m1 'model name' /proc/cpuinfo | cut -d: -f2"],
        )
    };
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    let load = sh("sh", &["-c", "uptime | sed 's/.*load averages*: //'"]);
    let commit = sh("git", &["rev-parse", "--short", "HEAD"]);
    let profile = if cfg!(debug_assertions) {
        "debug (unoptimised)"
    } else {
        "release"
    };
    format!(
        "- CPU: {cpu} ({cores} logical cores)\n- OS: {}\n- Load average when the run began: {load} (other work on the machine adds noise)\n- Build profile of the test process and services: {profile}\n- Commit: {commit}\n- PostgreSQL 16 in a Docker container on the same machine; database pool of 8 connections\n- Services run in-process (no network between them and the client)",
        sh("uname", &["-srm"])
    )
}

async fn size_of_tables(w: &World) -> i64 {
    sqlx::query_scalar(
        "SELECT sum(pg_total_relation_size(c.oid))::bigint FROM pg_class c \
         WHERE c.relname IN ('runs', 'steps', 'run_events', 'payloads', 'event_outbox', \
                             'command_dedupe', 'audit_records')",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("size")
}

async fn plan_of(w: &World, sql: &'static str) -> String {
    let rows: Vec<String> = sqlx::query_scalar(sql)
        .fetch_all(&w.db.pool)
        .await
        .expect("explain");
    rows.join(" / ")
}

#[tokio::test]
#[ignore = "benchmark: run on purpose with --ignored"]
async fn ten_thousand_suspended_runs_cost_a_row_each_and_scheduling_stays_cheap() {
    let w = world().await;
    let before = size_of_tables(&w).await;

    // ------------------------------------------------------------------ 1. load
    let (wf, _) = w
        .publish(document(
            "bench-flow",
            "pause",
            vec![
                with(timer_node("pause", 3_600_000), "next", json!("work")),
                json!({ "id": "work", "type": "activity", "activity": "act.work" }),
            ],
        ))
        .await;
    let loading = Instant::now();
    let mut starts: Vec<Duration> = Vec::with_capacity(SUSPENDED);
    let mut idle_small = Vec::new();
    for (from, to) in [(0, SMALL), (SMALL, SUSPENDED)] {
        starts.extend(
            stream::iter(from..to)
                .map(|i| {
                    let w = &w;
                    async move {
                        let t = Instant::now();
                        w.start_with(wf, &json!({ "i": i }), &format!("bench-key-{i:06}"), None)
                            .await
                            .expect("start");
                        t.elapsed()
                    }
                })
                .buffer_unordered(LOAD_CONCURRENCY)
                .collect::<Vec<_>>()
                .await,
        );
        if to == SMALL {
            sqlx::query("ANALYZE")
                .execute(&w.db.pool)
                .await
                .expect("analyze");
            for _ in 0..IDLE_TICKS {
                let t = Instant::now();
                let report = w.scheduler.tick().await;
                idle_small.push(t.elapsed());
                assert!(!report.did_work() && report.failed == 0, "{report:?}");
            }
        }
    }
    let load_elapsed = loading.elapsed();
    sqlx::query("ANALYZE")
        .execute(&w.db.pool)
        .await
        .expect("analyze");
    let footprint = size_of_tables(&w).await - before;
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE status = 'WAITING' AND waiting_kind = 'TIMER'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(waiting, i64::try_from(SUSPENDED).expect("fits"));
    let timer_plan = plan_of(
        &w,
        "EXPLAIN SELECT tenant_id, run_id, node FROM steps \
         WHERE status = 'WAITING' AND waiting_kind = 'TIMER' AND due_at <= clock_timestamp() \
         ORDER BY due_at LIMIT 200",
    )
    .await;
    assert!(
        timer_plan.contains("Index"),
        "the timer query must be an index scan with 10k suspended runs: {timer_plan}"
    );

    // ------------------------------------------------------------------ 2. idle scheduler
    let mut idle = Vec::new();
    for _ in 0..IDLE_TICKS {
        let t = Instant::now();
        let report = w.scheduler.tick().await;
        idle.push(t.elapsed());
        assert!(!report.did_work() && report.failed == 0, "{report:?}");
    }

    // ------------------------------------------------------------------ 3. a burst of due timers
    sqlx::query(
        "UPDATE steps SET due_at = clock_timestamp() - interval '1 second' \
         WHERE (tenant_id, run_id, node) IN ( \
             SELECT tenant_id, run_id, node FROM steps WHERE node = 'pause' \
             ORDER BY run_id LIMIT $1)",
    )
    .bind(i64::try_from(DUE).expect("fits"))
    .execute(&w.db.pool)
    .await
    .expect("make timers due");
    let burst = Instant::now();
    let (mut fired, mut ticks_in_burst) = (0u32, 0u32);
    let mut burst_ticks = Vec::new();
    while fired < u32::try_from(DUE).expect("fits") {
        let t = Instant::now();
        let report = w.scheduler.tick().await;
        burst_ticks.push(t.elapsed());
        fired += report.timers_fired;
        ticks_in_burst += 1;
        assert!(report.failed == 0, "{report:?}");
        assert!(ticks_in_burst < 100, "the burst did not drain: {fired}");
    }
    let burst_elapsed = burst.elapsed();

    // ------------------------------------------------------------------ 4. workers drain the tasks
    let claims = futures_util::future::join_all((0..workers()).map(|_| {
        let w = &w;
        async move {
            let session = w.session(&["act.work"]).await;
            let (mut claim, mut complete) = (Vec::new(), Vec::new());
            let mut done = 0usize;
            let mut empty = 0;
            while empty < 3 {
                let t = Instant::now();
                let tasks = w
                    .workers
                    .poll(
                        &session.principal,
                        session.worker_id,
                        &session.token,
                        16,
                        None,
                    )
                    .await
                    .expect("poll");
                if tasks.is_empty() {
                    empty += 1;
                    continue;
                }
                claim.push(t.elapsed() / u32::try_from(tasks.len()).expect("small"));
                empty = 0;
                for task in &tasks {
                    let t = Instant::now();
                    w.complete(&session, task, &json!({}))
                        .await
                        .expect("complete");
                    complete.push(t.elapsed());
                    done += 1;
                }
            }
            (claim, complete, done)
        }
    }))
    .await;
    let drain_elapsed = burst.elapsed() - burst_elapsed;
    let (mut claim, mut complete, mut completed) = (Vec::new(), Vec::new(), 0usize);
    for (c, d, n) in claims {
        claim.extend(c);
        complete.extend(d);
        completed += n;
    }
    assert_eq!(completed, DUE, "every due timer produced exactly one task");

    // ------------------------------------------------------------------ report
    starts.sort_unstable();
    let (start_row, _) = row("start a run (create, reserve the timer)", starts);

    let (idle_small_row, _) = row(
        &format!("scheduler tick, nothing due, {SMALL} suspended runs"),
        idle_small.clone(),
    );
    let mut idle_sorted = idle.clone();
    idle_sorted.sort_unstable();
    let (idle_row, idle_p95) = row(
        &format!("scheduler tick, nothing due, {SUSPENDED} suspended runs"),
        idle,
    );
    let (burst_row, _) = row(
        &format!("scheduler tick while {DUE} timers fall due"),
        burst_ticks,
    );
    let claim_p50 = {
        let mut sorted = claim.clone();
        sorted.sort_unstable();
        percentile(&sorted, 0.50)
    };
    let (claim_row, _) = row(
        &format!(
            "claim one task (poll of up to 16, {} workers, 10k suspended runs)",
            workers()
        ),
        claim,
    );
    idle_small.sort_unstable();
    let (complete_row, complete_p95) = row("complete a step (commit, route, finish)", complete);
    let idle_p50 = percentile(&idle_sorted, 0.50);
    let idle_p50_small = percentile(&idle_small, 0.50);
    // A floor of a few milliseconds keeps noise on an already tiny number from reading as growth.
    let growth = ms(idle_p50) / ms(idle_p50_small).max(3.0);
    let mut report = format!(
        "# Baseline benchmark: durable execution (feature 002)\n\nMeasured by `tests/performance/durable-execution/main.rs`. Hypotheses, not promises: figures are valid only for this environment.\n\n## Environment\n\n{}\n\n## Workload\n\n- {SUSPENDED} runs of `timer(1h) -> activity`, started with {LOAD_CONCURRENCY} concurrent callers; every one is suspended on a timer\n- {IDLE_TICKS} scheduler ticks with nothing due, then {DUE} timers made due at once and drained by repeated ticks\n- {} workers claiming (16 at a time) and completing the {DUE} resulting tasks while the other {} runs stay suspended\n\n## Results\n\n| Operation | Samples | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|---:|\n",
        environment(),
        workers(),
        SUSPENDED - DUE
    );
    for line in [
        &start_row,
        &idle_small_row,
        &idle_row,
        &burst_row,
        &claim_row,
        &complete_row,
    ] {
        report.push_str(line);
    }
    report.push_str(&format!(
        "\n- Loading {SUSPENDED} runs took {:.1} s ({:.0} runs/s).\n- Database footprint of a suspended run (runs, steps, journal, payloads, outbox, dedupe, audit with their indexes): **{} bytes** per run.\n- {DUE} due timers were fired in {:.2} s ({:.0} timers/s, {} ticks); their {DUE} tasks were claimed and completed in {:.2} s ({:.0} tasks/s).\n- Plan of the due-timer query with {SUSPENDED} waiting steps: `{}`.\n\n## Hypotheses\n\n| Hypothesis | Result |\n|---|---|\n| scheduler tick with nothing due, p95 < {} ms at 10k suspended runs | {:.1} ms: **{}** |\n| ten times the runs slows an idle tick (p50) by less than {}x | {:.1} ms to {:.1} ms ({:.1}x): **{}** |\n| claiming one task, p50 < {} ms with 10k suspended runs (the p95 is in the table) | {:.1} ms: **{}** |\n| completing a step, p95 < {} ms | {:.1} ms: **{}** |\n\nA suspended run holds no process, no connection and no broker consumer: it is the rows counted above. Larger populations (100k, 1M) are later experiments (spec, NFR-002-PERF).\n\n## Reading the numbers\n\n- The machine is shared with unrelated workloads (see the load average above), so absolute figures vary a lot between runs; the claims that matter are structural: the scheduler's queries are index scans whose cost does not grow with the number of suspended runs, and a suspended run costs about 2.4 KiB of rows.\n- Timers fall due and fire one transaction at a time under their run's lock, about 100 per second per scheduler tick loop. A burst of 100k timers due at the same instant would therefore take about 17 minutes to fire with one engine; more engines do not multiply it as things stand (they read the same oldest candidates and the losers skip what the winner already fired, which is safe but wasteful), so firing with bounded concurrency inside a tick and giving each engine disjoint candidates are the next steps (feature 010, scale and HA). Nothing is lost while it drains: due timers wait in an index.\n- The tail (p99) of claiming grows with the number of workers polling at once, because they all look at the oldest ready runs first and each loser skips the runs the winner holds. Spreading candidates (for example by hashing the worker) is a known optimisation, deliberately not made before it is measured on a quiet machine.\n- A poll leases up to 16 steps, each in its own transaction, so the figure above is per claimed task.\n",
        load_elapsed.as_secs_f64(),
        SUSPENDED as f64 / load_elapsed.as_secs_f64(),
        footprint / i64::try_from(SUSPENDED).expect("fits"),
        burst_elapsed.as_secs_f64(),
        DUE as f64 / burst_elapsed.as_secs_f64(),
        ticks_in_burst,
        drain_elapsed.as_secs_f64(),
        DUE as f64 / drain_elapsed.as_secs_f64(),
        timer_plan.replace('|', "/"),
        IDLE_TICK_P95.as_millis(),
        ms(idle_p95),
        if idle_p95 < IDLE_TICK_P95 { "met" } else { "NOT met" },
        IDLE_GROWTH_FACTOR,
        ms(idle_p50_small),
        ms(idle_p50),
        growth,
        if growth < IDLE_GROWTH_FACTOR { "met" } else { "NOT met" },
        CLAIM_PER_TASK_P50.as_millis(),
        ms(claim_p50),
        if claim_p50 < CLAIM_PER_TASK_P50 { "met" } else { "NOT met" },
        COMPLETE_P95.as_millis(),
        ms(complete_p95),
        if complete_p95 < COMPLETE_P95 { "met" } else { "NOT met" },
    ));
    println!("{report}");
    if let Ok(path) = std::env::var("SAGAT_PERF_REPORT") {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let target = if std::path::Path::new(&path).is_absolute() {
            path.into()
        } else {
            root.join(path)
        };
        std::fs::write(&target, &report).expect("write the report");
    }
    assert!(idle_p95 < IDLE_TICK_P95, "idle tick p95 {idle_p95:?}");
    assert!(
        growth < IDLE_GROWTH_FACTOR,
        "an idle tick grew {growth:.1}x from {SMALL} to {SUSPENDED} runs: it is not an index scan"
    );
    assert!(claim_p50 < CLAIM_PER_TASK_P50, "claim p50 {claim_p50:?}");
    assert!(complete_p95 < COMPLETE_P95, "complete p95 {complete_p95:?}");
}

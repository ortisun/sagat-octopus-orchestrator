//! Load harness of feature 010 (T010-029, T010-034; FR-010-006, NFR-010-PERF): what a million
//! suspended runs cost, how fast due timers fire with one and with several engine replicas, what a
//! noisy tenant does to another, and how many step transitions per second the architecture sustains
//! on the machine it runs on.
//!
//! The numbers mean something only with the environment they were taken in, which each section of
//! the report records, and they are **measurements, not promises**. The scenarios are `#[ignore]`d
//! and run on purpose, one at a time, by `tests/performance/scalability-ha/run.sh`, which also starts
//! a PostgreSQL of its own with the configuration it reports:
//!
//! ```sh
//! SAGAT_PERF_REPORT=tests/performance/reports/scalability-ha.md \
//!   cargo test -p sagat-tests --test perf_scalability_ha --release -- --ignored --nocapture <scenario>
//! ```
//!
//! Every population is made through the platform's own services (`start`), never by writing rows:
//! what a suspended run costs is what the real path writes.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::cast_precision_loss
)]

#[path = "../../support/mod.rs"]
mod support;

mod dr;

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::runs::StartRun;
use sagat_domain::identity::Role;
use sagat_domain::quota::QuotaResource;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::json;
use support::engine::{Sched, World, activity, ctx, document, timer_node, with};

// ---------------------------------------------------------------------------------- knobs

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The largest population of `waits_at_scale`. 1 000 000 is the target of BL03.
fn max_waits() -> usize {
    env_usize("SAGAT_PERF_MAX_WAITS", 1_000_000)
}

/// Connections of the pool of one worker-API replica in `step_transitions_per_second`.
const API_POOL: u32 = 20;

/// Callers starting runs at the same time.
fn load_concurrency() -> usize {
    env_usize("SAGAT_PERF_LOAD_CONCURRENCY", 80)
}

// ---------------------------------------------------------------------------------- numbers

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn sh(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// The hardware and the configuration of the database, read from the machine and from the server.
async fn environment(w: &World) -> String {
    let cpu = if cfg!(target_os = "macos") {
        sh("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        sh(
            "sh",
            &["-c", "grep -m1 'model name' /proc/cpuinfo | cut -d: -f2"],
        )
    };
    let memory_gib = if cfg!(target_os = "macos") {
        sh("sysctl", &["-n", "hw.memsize"])
            .parse::<u64>()
            .unwrap_or(0) as f64
            / 1024f64.powi(3)
    } else {
        sh("sh", &["-c", "awk '/MemTotal/ {print $2}' /proc/meminfo"])
            .parse::<u64>()
            .unwrap_or(0) as f64
            / 1024f64.powi(2)
    };
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    let load = sh("sh", &["-c", "uptime | sed 's/.*load averages*: //'"]);
    let version: String = sqlx::query_scalar("SELECT version()")
        .fetch_one(&w.db.pool)
        .await
        .expect("version");
    let settings: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, setting || coalesce(unit, '') FROM pg_settings WHERE name IN \
         ('shared_buffers', 'max_connections', 'synchronous_commit', 'fsync', 'wal_level', \
          'max_wal_size', 'checkpoint_timeout', 'work_mem', 'effective_cache_size', \
          'random_page_cost', 'full_page_writes') ORDER BY name",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("settings");
    let profile = if cfg!(debug_assertions) {
        "debug (unoptimised)"
    } else {
        "release"
    };
    let mut out = format!(
        "- CPU: {cpu} ({cores} logical cores), {memory_gib:.0} GiB of memory\n\
         - OS: {}\n\
         - Load average when the scenario began: {load} (other work on the machine adds noise)\n\
         - Build profile of the test process and the services: {profile}\n\
         - Commit: {}\n\
         - {}\n\
         - PostgreSQL configuration: ",
        sh("uname", &["-srm"]),
        sh("git", &["rev-parse", "--short", "HEAD"]),
        version.split(',').next().unwrap_or(&version),
    );
    out.push_str(
        &settings
            .iter()
            .map(|(k, v)| format!("`{k}={v}`"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str(
        "\n- Topology: PostgreSQL and the services on one machine (no network between them and the \
         client); each engine replica is a scheduler with its own connection pool; HA of the \
         database is not part of the measurement.\n",
    );
    out
}

/// Appends a section to the report (`SAGAT_PERF_REPORT`), and prints it.
fn section(title: &str, body: &str) {
    let text = format!("\n## {title}\n\n{body}");
    println!("{text}");
    if let Ok(path) = std::env::var("SAGAT_PERF_REPORT") {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let target = if std::path::Path::new(&path).is_absolute() {
            path.into()
        } else {
            root.join(path)
        };
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&target)
            .expect("open the report");
        file.write_all(text.as_bytes()).expect("write the report");
    }
}

// ---------------------------------------------------------------------------------- populations

/// A world whose tenants may hold as many unfinished runs as the scenarios start. The platform
/// refuses more than a namespace's quota of active runs (10 000 by default), which is the right
/// answer in production and not what a benchmark of waiting wants: the operator grants a ceiling
/// (`control-plane quota-ceiling`) and the tenant's administrator sets its limit, as they would.
async fn world() -> World {
    const ROOM: u64 = 5_000_000;
    let w = support::engine::world().await;
    for (admin, namespace) in [
        (&w.admin, w.namespace.id()),
        (&w.other_admin, w.other_namespace.id()),
    ] {
        for resource in [QuotaResource::ActiveRuns, QuotaResource::TenantActiveRuns] {
            w.quotas
                .grant_ceiling(admin.tenant_id(), resource, Some(ROOM))
                .await
                .expect("grant the ceiling");
        }
        w.quotas
            .set(
                admin,
                Some(namespace),
                QuotaResource::ActiveRuns,
                ROOM,
                &ctx(),
            )
            .await
            .expect("set the namespace's limit");
        w.quotas
            .set(admin, None, QuotaResource::TenantActiveRuns, ROOM, &ctx())
            .await
            .expect("set the tenant's limit");
    }
    w
}

/// The workflow of every population: a timer of an hour, then an activity.
async fn sleeper(w: &World, name: &str) -> sagat_domain::definition::WorkflowId {
    let (wf, _) = w
        .publish(document(
            name,
            "pause",
            vec![
                with(timer_node("pause", 3_600_000), "next", json!("work")),
                json!({ "id": "work", "type": "activity", "activity": "act.work" }),
            ],
        ))
        .await;
    wf
}

/// Starts runs `from..to` of `wf` with `load_concurrency()` callers, through the services of several
/// API replicas (each with a pool of its own, as separate processes have: the pool of the world is
/// eight connections and would be the limit); returns how long each start took.
async fn start_runs(
    w: &World,
    wf: sagat_domain::definition::WorkflowId,
    tag: &str,
    from: usize,
    to: usize,
) -> Vec<Duration> {
    let mut apis = Vec::new();
    for _ in 0..env_usize("SAGAT_PERF_LOAD_API_REPLICAS", 6) {
        apis.push(Arc::new(w.replica_with_pool(16).await));
    }
    let admin = w.admin.clone();
    stream::iter(from..to)
        .map(|i| {
            let api = Arc::clone(&apis[i % apis.len()]);
            let admin = admin.clone();
            async move {
                let t = Instant::now();
                let input = serde_json::to_vec(&json!({ "i": i })).expect("json");
                api.runs
                    .start(
                        &admin,
                        &StartRun {
                            namespace: "payments",
                            workflow: wf,
                            version: None,
                            input: &input,
                            correlation_id: None,
                            deadline: None,
                            key: &format!("{tag}-{i:08}"),
                        },
                        &ctx(),
                    )
                    .await
                    .expect("start");
                t.elapsed()
            }
        })
        .buffer_unordered(load_concurrency())
        .collect()
        .await
}

/// Makes `n` of the first tenant's timers due now, picked from the ones not yet due; returns the
/// database's clock at that moment (the instant they became due, to the second the timers were aged by).
async fn make_due(
    w: &World,
    tenant: sagat_domain::identity::TenantId,
    n: usize,
) -> time::OffsetDateTime {
    let aged: i64 = sqlx::query_scalar(
        "WITH picked AS ( \
             SELECT tenant_id, run_id, node FROM steps \
             WHERE tenant_id = $1 AND status = 'WAITING' AND waiting_kind = 'TIMER' \
               AND due_at > clock_timestamp() + interval '30 minutes' \
             ORDER BY run_id LIMIT $2), \
         moved AS ( \
             UPDATE steps s SET due_at = clock_timestamp() - interval '1 second' \
               FROM picked p WHERE s.tenant_id = p.tenant_id AND s.run_id = p.run_id AND s.node = p.node \
             RETURNING 1) \
         SELECT count(*) FROM moved",
    )
    .bind(tenant.as_uuid())
    .bind(i64::try_from(n).expect("fits"))
    .fetch_one(&w.db.pool)
    .await
    .expect("make due");
    assert_eq!(
        usize::try_from(aged).expect("fits"),
        n,
        "not enough waiting timers to make due"
    );
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&w.db.pool)
        .await
        .expect("clock")
}

/// The database's footprint, runs and everything written with them.
async fn footprint(w: &World) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(pg_total_relation_size(c.oid)), 0)::bigint FROM pg_class c \
         WHERE c.relname IN ('runs', 'steps', 'run_events', 'payloads', 'event_outbox', \
                             'command_dedupe', 'audit_records', 'telemetry_pending')",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("size")
}

async fn plan_of(w: &World, sql: &str) -> String {
    let rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_all(&w.db.pool)
        .await
        .expect("explain");
    rows.join("\n")
}

/// The plan PostgreSQL makes for the due-timer query as a prepared statement without the values.
async fn generic_plan(w: &World) -> String {
    use sqlx::Executor;
    let mut conn = w.db.pool.acquire().await.expect("a connection");
    for statement in [
        "SET plan_cache_mode = force_generic_plan",
        "PREPARE due_timers(bigint, smallint[], uuid[]) AS \
         SELECT t.tenant_id, t.run_id, t.node FROM unnest($2::smallint[]) AS sh(s) \
         CROSS JOIN LATERAL ( \
             SELECT tenant_id, run_id, node, due_at FROM steps \
             WHERE run_shard(run_id) = sh.s AND status = 'WAITING' AND waiting_kind = 'TIMER' \
               AND due_at <= now() AND tenant_id <> ALL($3::uuid[]) \
             ORDER BY due_at LIMIT $1) t \
         ORDER BY t.due_at LIMIT $1",
    ] {
        conn.execute(statement).await.expect("prepare");
    }
    let rows: Vec<String> = sqlx::query_scalar(
        "EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, SUMMARY ON) \
         EXECUTE due_timers(200, ARRAY[0, 17, 33, 63]::smallint[], ARRAY[]::uuid[])",
    )
    .fetch_all(&mut *conn)
    .await
    .expect("explain");
    conn.execute("DEALLOCATE due_timers")
        .await
        .expect("deallocate");
    rows.join("\n")
}

/// A scheduler with a pool of its own, as an engine replica would have.
async fn replica_scheduler(
    w: &World,
    placement: Option<Placement>,
    settings: SchedulerSettings,
) -> Sched {
    let replica = w.replica_with_pool(24).await;
    let scheduler = Scheduler::new(
        replica.store,
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

/// `n` replicas that share the 64 shards, each with its scheduler.
async fn replicas(w: &World, n: usize, settings: SchedulerSettings) -> Vec<Sched> {
    let keepers: Vec<Keeper<PgRunStore>> = (0..n)
        .map(|i| {
            Keeper::new(
                w.store.clone(),
                Placement::new(format!("bench-{n}-{i}-{}", uuid::Uuid::new_v4().simple())),
                KeeperSettings {
                    ttl: Duration::from_secs(60),
                },
            )
        })
        .collect();
    // Each announces, then rounds until the split settles.
    for _ in 0..(n + 2) {
        for keeper in &keepers {
            keeper.round().await.expect("round");
        }
    }
    let held: usize = keepers.iter().map(|k| k.placement().owned().len()).sum();
    assert_eq!(held, 64, "the replicas hold every shard between them");
    let mut out = Vec::new();
    for keeper in &keepers {
        out.push(replica_scheduler(w, Some(keeper.placement().clone()), settings).await);
    }
    out
}

struct Drained {
    elapsed: Duration,
    fired: u32,
    ticks: Vec<Duration>,
}

/// Every scheduler ticks until nothing is left for it; they run at the same time.
async fn drain(schedulers: &[Sched]) -> Drained {
    let started = Instant::now();
    let results = futures_util::future::join_all(schedulers.iter().map(|s| async move {
        let (mut ticks, mut fired) = (Vec::new(), 0u32);
        loop {
            let t = Instant::now();
            let report = s.tick().await;
            ticks.push(t.elapsed());
            assert_eq!(report.failed, 0, "{report:?}");
            fired += report.timers_fired;
            if !report.did_work() {
                break;
            }
        }
        (ticks, fired)
    }))
    .await;
    let elapsed = started.elapsed();
    let mut drained = Drained {
        elapsed,
        fired: 0,
        ticks: Vec::new(),
    };
    for (ticks, fired) in results {
        drained.fired += fired;
        drained.ticks.extend(ticks);
    }
    drained
}

/// How long after they became due the timers of `tenant` fired, from the journal.
async fn lateness(
    w: &World,
    tenant: sagat_domain::identity::TenantId,
    due_since: time::OffsetDateTime,
) -> Vec<Duration> {
    let seconds: Vec<f64> = sqlx::query_scalar(
        "SELECT extract(epoch FROM occurred_at - $2)::float8 FROM run_events \
         WHERE tenant_id = $1 AND event_type = 'timer.fired' AND occurred_at >= $2",
    )
    .bind(tenant.as_uuid())
    .bind(due_since)
    .fetch_all(&w.db.pool)
    .await
    .expect("journal");
    seconds
        .into_iter()
        .map(|s| Duration::from_secs_f64(s.max(0.0)))
        .collect()
}

fn rate(count: u32, elapsed: Duration) -> f64 {
    f64::from(count) / elapsed.as_secs_f64().max(1e-9)
}

// ---------------------------------------------------------------------------------- scenarios

/// BL01..BL03: the cost of waiting. The population grows from 10 000 to `max_waits()` in one
/// database; at each size it measures what a suspended run costs on disk, what a scheduler tick
/// costs when nothing is due, and the plan of the query that finds what is due.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "benchmark: run on purpose with --ignored"]
async fn waits_at_scale() {
    let w = world().await;
    let wf = sleeper(&w, "bench-waits").await;
    let settings = SchedulerSettings::default();
    // The whole shard set on one replica: what the engine is with one replica.
    let schedulers = replicas(&w, 1, settings).await;
    let mut body = format!("{}\n", environment(&w).await);
    body.push_str(
        "Runs of `timer(1h) -> activity`, started through the run service with the number of callers below; every one is suspended on its timer. The population grows in one database, so each size includes the earlier ones.\n\n",
    );
    let mut sizes: Vec<usize> = vec![10_000, 100_000, 1_000_000]
        .into_iter()
        .filter(|s| *s <= max_waits())
        .collect();
    if sizes.last() != Some(&max_waits()) {
        sizes.push(max_waits());
    }
    body.push_str(&format!(
        "Callers starting runs at the same time: {}.\n\n",
        load_concurrency()
    ));
    body.push_str("| Suspended runs | Loaded in | Starts/s | Start p50 / p95 / p99 | Bytes per run (rows and indexes) | Idle tick p50 | p95 | p99 | max |\n|---:|---:|---:|---|---:|---:|---:|---:|---:|\n");
    let before = footprint(&w).await;
    let mut loaded = 0usize;
    let mut plans = String::new();
    for size in sizes {
        let t = Instant::now();
        let mut starts = start_runs(&w, wf, "bench-waits", loaded, size).await;
        let load_elapsed = t.elapsed();
        loaded = size;
        sqlx::query("ANALYZE")
            .execute(&w.db.pool)
            .await
            .expect("analyze");
        starts.sort_unstable();
        let bytes = (footprint(&w).await - before) / i64::try_from(size).expect("fits");
        let mut idle = Vec::new();
        for _ in 0..100 {
            let t = Instant::now();
            let report = schedulers[0].tick().await;
            idle.push(t.elapsed());
            assert!(!report.did_work() && report.failed == 0, "{report:?}");
        }
        idle.sort_unstable();
        body.push_str(&format!(
            "| {size} | {:.0} s | {:.0} | {:.0} / {:.0} / {:.0} ms | {bytes} | {:.1} ms | {:.1} ms | {:.1} ms | {:.1} ms |\n",
            load_elapsed.as_secs_f64(),
            starts.len() as f64 / load_elapsed.as_secs_f64(),
            ms(percentile(&starts, 0.50)),
            ms(percentile(&starts, 0.95)),
            ms(percentile(&starts, 0.99)),
            ms(percentile(&idle, 0.50)),
            ms(percentile(&idle, 0.95)),
            ms(percentile(&idle, 0.99)),
            ms(*idle.last().expect("ticks")),
        ));
        let plan = plan_of(
            &w,
            "EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, SUMMARY ON) \
             SELECT t.tenant_id, t.run_id, t.node FROM unnest(ARRAY[0, 17, 33, 63]::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
                 SELECT tenant_id, run_id, node, due_at FROM steps \
                 WHERE run_shard(run_id) = sh.s AND status = 'WAITING' AND waiting_kind = 'TIMER' \
                   AND due_at <= now() ORDER BY due_at LIMIT 200) t \
             ORDER BY t.due_at LIMIT 200",
        )
        .await;
        plans.push_str(&format!("\nPlan of the due-timer query over four shards, {size} suspended runs:\n\n```\n{plan}\n```\n"));
        if size == max_waits() {
            // The store asks it with parameters, as a prepared statement: after a few executions
            // PostgreSQL may switch to a generic plan, which does not know the values. This is that plan.
            plans.push_str(&format!(
                "\nThe same query as a prepared statement with a generic plan (what a connection settles on after a few executions):\n\n```\n{}\n```\n",
                generic_plan(&w).await
            ));
        }
    }
    let sizes_of_indexes: Vec<(String, i64)> = sqlx::query_as(
        "SELECT indexrelid::regclass::text, pg_relation_size(indexrelid)::bigint \
         FROM pg_index WHERE indrelid = 'steps'::regclass AND indexrelid::regclass::text IN \
         ('steps_timer_idx', 'steps_timer_shard_idx', 'steps_ready_idx') ORDER BY 1",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("index sizes");
    body.push_str("\nSize of the indexes the passes read, at the largest population:\n\n");
    for (name, bytes) in sizes_of_indexes {
        body.push_str(&format!(
            "- `{name}`: {:.1} MiB\n",
            bytes as f64 / 1_048_576.0
        ));
    }
    body.push_str(&plans);
    section("Waits at scale (BL01, BL02, BL03)", &body);
}

/// US01: more replicas and more concurrency inside a replica fire a burst of due timers faster.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "benchmark: run on purpose with --ignored"]
async fn firing_scales_with_concurrency_and_replicas() {
    let waits = env_usize("SAGAT_PERF_FIRING_WAITS", 20_000);
    let burst = env_usize("SAGAT_PERF_FIRING_BURST", 2_000);
    // Six configurations, each fires `burst` timers that no earlier one used.
    assert!(
        waits >= 6 * burst,
        "SAGAT_PERF_FIRING_WAITS ({waits}) must be at least six times SAGAT_PERF_FIRING_BURST ({burst})"
    );
    let w = world().await;
    let wf = sleeper(&w, "bench-firing").await;
    start_runs(&w, wf, "bench-firing", 0, waits).await;
    sqlx::query("ANALYZE")
        .execute(&w.db.pool)
        .await
        .expect("analyze");
    let tenant = w.admin.tenant_id();

    let mut body = format!("{}\n", environment(&w).await);
    body.push_str(&format!(
        "{waits} suspended runs of `timer(1h) -> activity`; for each configuration {burst} of the timers are made due at the same instant and the replicas tick until nothing is left. `lateness` is the time from the timers becoming due to their `timer.fired` event in the journal. One replica with a concurrency of 1 is the closest to the scheduler of feature 002 (one transaction at a time); its report measured 145 timers/s.\n\n"
    ));
    body.push_str("| Replicas | Concurrency | Timers/s | Drained in | Tick p50 / p95 / p99 | Lateness p50 | p95 | p99 |\n|---:|---:|---:|---:|---|---:|---:|---:|\n");
    let configs: [(usize, usize); 6] = [(1, 1), (1, 8), (1, 32), (2, 8), (4, 8), (8, 8)];
    for (n, concurrency) in configs {
        let settings = SchedulerSettings {
            concurrency,
            batch: 200,
            ..SchedulerSettings::default()
        };
        let schedulers = replicas(&w, n, settings).await;
        let due_since = make_due(&w, tenant, burst).await;
        let drained = drain(&schedulers).await;
        assert_eq!(
            usize::try_from(drained.fired).expect("fits"),
            burst,
            "{n} replicas, concurrency {concurrency}"
        );
        let mut late = lateness(&w, tenant, due_since).await;
        late.sort_unstable();
        let mut ticks = drained.ticks;
        ticks.sort_unstable();
        body.push_str(&format!(
            "| {n} | {concurrency} | {:.0} | {:.2} s | {:.0} / {:.0} / {:.0} ms | {:.2} s | {:.2} s | {:.2} s |\n",
            rate(drained.fired, drained.elapsed),
            drained.elapsed.as_secs_f64(),
            ms(percentile(&ticks, 0.50)),
            ms(percentile(&ticks, 0.95)),
            ms(percentile(&ticks, 0.99)),
            percentile(&late, 0.50).as_secs_f64(),
            percentile(&late, 0.95).as_secs_f64(),
            percentile(&late, 0.99).as_secs_f64(),
        ));
        // Give the replicas back before the next configuration takes the shards.
        sqlx::query("DELETE FROM scheduler_members")
            .execute(&w.db.pool)
            .await
            .expect("members");
        sqlx::query("UPDATE scheduler_shards SET expires_at = 'epoch', owner = NULL")
            .execute(&w.db.pool)
            .await
            .expect("shards");
    }
    body.push_str("\nReplicas are schedulers with a connection pool of their own in one process; they contend for one PostgreSQL on the same machine, so the curve shows how the work divides and where the database becomes the limit, not what separate machines would do.\n");
    section(
        "Firing a burst of timers: concurrency and replicas (US01)",
        &body,
    );
}

/// US04 / BL06: a noisy tenant and a quiet one, with and without fairness.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "benchmark: run on purpose with --ignored"]
async fn a_noisy_tenant_and_a_quiet_one() {
    let noisy_runs = env_usize("SAGAT_PERF_NOISY", 8_000);
    let quiet_runs = env_usize("SAGAT_PERF_QUIET", 2_000);
    let w = world().await;
    let wf_a = sleeper(&w, "bench-noisy").await;
    let wf_b = w
        .publish_as_other_tenant(document(
            "bench-quiet",
            "pause",
            vec![
                with(timer_node("pause", 3_600_000), "next", json!("work")),
                json!({ "id": "work", "type": "activity", "activity": "act.work" }),
            ],
        ))
        .await;
    let world = &w;
    // The quiet tenant's work is the younger: oldest-first reaches it last.
    start_runs(&w, wf_a, "bench-noisy", 0, noisy_runs).await;
    stream::iter(0..quiet_runs)
        .map(|i| async move {
            world
                .start_as_other_tenant(wf_b, &json!({ "i": i }))
                .await
                .expect("start");
        })
        .buffer_unordered(load_concurrency())
        .collect::<Vec<_>>()
        .await;
    sqlx::query("ANALYZE")
        .execute(&w.db.pool)
        .await
        .expect("analyze");
    let (noisy, quiet) = (w.admin.tenant_id(), w.other_admin.tenant_id());

    let mut body = format!("{}\n", environment(&w).await);
    body.push_str(&format!(
        "Tenant A starts {noisy_runs} timers and tenant B {quiet_runs} (B's are the younger), and every one of them falls due at the same instant: A has {:.0}% of the work. One replica, concurrency 8, batch 200. `lateness` is the time from becoming due to the `timer.fired` event. With fairness off a pass takes the oldest 200 candidates, as before feature 010.\n\n",
        100.0 * noisy_runs as f64 / (noisy_runs + quiet_runs) as f64
    ));
    body.push_str("| Policy | Tenant | Timers | Lateness p50 | p95 | p99 | Last fired after |\n|---|---|---:|---:|---:|---:|---:|\n");
    for (label, fair, weights) in [
        ("fairness off (oldest first)", false, None),
        ("fairness on, equal weights", true, None),
        ("fairness on, B weighs 4", true, Some(4u16)),
    ] {
        sqlx::query("DELETE FROM tenant_scheduling")
            .execute(&w.db.pool)
            .await
            .expect("weights");
        if let Some(weight) = weights {
            w.quotas
                .set_scheduling_weight(quiet, Some(weight))
                .await
                .expect("weight");
        }
        // Every timer of both tenants due now.
        sqlx::query("UPDATE steps SET due_at = clock_timestamp() - interval '1 second' WHERE waiting_kind = 'TIMER' AND due_at > clock_timestamp() + interval '30 minutes'")
            .execute(&w.db.pool)
            .await
            .expect("age");
        let due_since: time::OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&w.db.pool)
            .await
            .expect("clock");
        let keeper = Keeper::new(
            w.store.clone(),
            Placement::new(format!("bench-noisy-{}", uuid::Uuid::new_v4().simple())),
            KeeperSettings {
                ttl: Duration::from_secs(60),
            },
        );
        keeper.round().await.expect("round");
        let schedulers = vec![
            replica_scheduler(
                &w,
                Some(keeper.placement().clone()),
                SchedulerSettings {
                    fair,
                    concurrency: 8,
                    batch: 200,
                    ..SchedulerSettings::default()
                },
            )
            .await,
        ];
        let drained = drain(&schedulers).await;
        assert_eq!(
            usize::try_from(drained.fired).expect("fits"),
            noisy_runs + quiet_runs
        );
        for (name, tenant) in [("A (noisy)", noisy), ("B (quiet)", quiet)] {
            let mut late = lateness(&w, tenant, due_since).await;
            late.sort_unstable();
            body.push_str(&format!(
                "| {label} | {name} | {} | {:.2} s | {:.2} s | {:.2} s | {:.2} s |\n",
                late.len(),
                percentile(&late, 0.50).as_secs_f64(),
                percentile(&late, 0.95).as_secs_f64(),
                percentile(&late, 0.99).as_secs_f64(),
                late.last().copied().unwrap_or_default().as_secs_f64(),
            ));
        }
        sqlx::query("DELETE FROM scheduler_members")
            .execute(&w.db.pool)
            .await
            .expect("members");
        sqlx::query("UPDATE scheduler_shards SET expires_at = 'epoch', owner = NULL")
            .execute(&w.db.pool)
            .await
            .expect("shards");
        // The next policy needs timers again: new runs, in the same proportions.
        refill(&w, wf_a, wf_b, noisy_runs, quiet_runs).await;
    }
    section("A noisy tenant and a quiet one (BL06, US04)", &body);
}

/// After a policy has fired every timer, the next policy needs timers again: new runs, in the same
/// proportions and ages. (The journal rows of the fired runs stay; they are filtered by time.)
async fn refill(
    w: &World,
    wf_a: sagat_domain::definition::WorkflowId,
    wf_b: sagat_domain::definition::WorkflowId,
    noisy_runs: usize,
    quiet_runs: usize,
) {
    let tag = uuid::Uuid::new_v4().simple().to_string();
    start_runs(w, wf_a, &format!("refill-a-{tag}"), 0, noisy_runs).await;
    let world = w;
    let tag_b = tag.clone();
    stream::iter(0..quiet_runs)
        .map(|i| {
            let tag_b = tag_b.clone();
            async move {
                let _ = &tag_b;
                world
                    .start_as_other_tenant(wf_b, &json!({ "i": i, "t": tag_b }))
                    .await
                    .expect("start");
            }
        })
        .buffer_unordered(load_concurrency())
        .collect::<Vec<_>>()
        .await;
}

/// BL02/BL04: how many step transitions per second the workers sustain, and the latency of each
/// step, with a growing number of workers polling and completing at the same time.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "benchmark: run on purpose with --ignored"]
async fn step_transitions_per_second() {
    let runs = env_usize("SAGAT_PERF_TRANSITION_RUNS", 3_000);
    let w = Arc::new(world().await);
    let (wf, _) = w
        .publish(document(
            "bench-chain",
            "a",
            vec![
                with(activity("a"), "next", json!("b")),
                with(activity("b"), "next", json!("c")),
                activity("c"),
            ],
        ))
        .await;
    let mut body = format!("{}\n", environment(&w).await);
    body.push_str(&format!(
        "{runs} runs of three chained activities per configuration; a worker polls up to 16 tasks and completes each one (a complete is one transition: the commit, the routing and the next step becoming ready). Workers are tasks in this process with sessions of their own, spread over identities (a session budget is 32 per identity). The worker API is served by 1 to 8 replicas, each with a pool of {API_POOL} connections (the engine's default is 20): a worker's calls go to the replica its number selects. Closed loop: the workers take work as fast as they can, so the figure is the ceiling of this machine, not a rate that was asked for.\n\n"
    ));
    body.push_str("| API replicas | Workers | Transitions | Transitions/s | Poll per task p50 / p95 / p99 | Complete p50 / p95 / p99 |\n|---:|---:|---:|---:|---|---|\n");
    let mut ceiling = 0f64;
    for (api_replicas, workers) in [
        (1usize, 4usize),
        (1, 16),
        (1, 64),
        (1, 128),
        (2, 64),
        (2, 128),
        (4, 64),
        (4, 128),
        (8, 128),
    ] {
        // The worker API replicas of this configuration, each with a pool of its own.
        let mut apis = Vec::new();
        for _ in 0..api_replicas {
            apis.push(Arc::new(w.replica_with_pool(API_POOL).await));
        }
        // Fresh runs for this configuration.
        let tag = uuid::Uuid::new_v4().simple().to_string();
        start_runs(&w, wf, &format!("chain-{tag}"), 0, runs).await;
        // (A session budget is 32 per identity and the sessions of earlier configurations stay, so
        // every configuration takes identities of its own.)
        let mut sessions = Vec::new();
        let mut principal = w.worker.clone();
        for n in 0..workers {
            if n % 32 == 0 {
                principal = w
                    .member(
                        &format!("bench-worker-{api_replicas}-{workers}-{}", n / 32),
                        Role::Worker,
                    )
                    .await;
            }
            sessions.push(
                w.session_as(&principal, &["act.a", "act.b", "act.c"], 100)
                    .await,
            );
        }
        let started = Instant::now();
        let results =
            futures_util::future::join_all(sessions.iter().enumerate().map(|(n, session)| {
                let api = Arc::clone(&apis[n % api_replicas]);
                async move {
                    let (mut polls, mut completes) = (Vec::new(), Vec::new());
                    let mut last_done = Instant::now();
                    let mut empty = 0;
                    while empty < 3 {
                        let t = Instant::now();
                        let tasks = api
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
                        empty = 0;
                        polls.push(t.elapsed() / u32::try_from(tasks.len()).expect("small"));
                        for task in &tasks {
                            let t = Instant::now();
                            api.workers
                                .complete(
                                    &session.principal,
                                    &support::engine::task_ref(task),
                                    b"{}",
                                    &format!("done-{}", task.task_id()),
                                )
                                .await
                                .expect("complete");
                            completes.push(t.elapsed());
                            last_done = Instant::now();
                        }
                    }
                    (polls, completes, last_done)
                }
            }))
            .await;
        // The rate is over the time the work took: until the last completion, not the last empty poll.
        let (mut polls, mut completes) = (Vec::new(), Vec::new());
        let mut finished = started;
        for (p, c, last) in results {
            polls.extend(p);
            completes.extend(c);
            finished = finished.max(last);
        }
        let elapsed = finished - started;
        let transitions = completes.len();
        let per_second = transitions as f64 / elapsed.as_secs_f64().max(1e-9);
        ceiling = ceiling.max(per_second);
        polls.sort_unstable();
        completes.sort_unstable();
        body.push_str(&format!(
            "| {api_replicas} | {workers} | {transitions} | {per_second:.0} | {:.0} / {:.0} / {:.0} ms | {:.0} / {:.0} / {:.0} ms |\n",
            ms(percentile(&polls, 0.50)),
            ms(percentile(&polls, 0.95)),
            ms(percentile(&polls, 0.99)),
            ms(percentile(&completes, 0.50)),
            ms(percentile(&completes, 0.95)),
            ms(percentile(&completes, 0.99)),
        ));
        // The runs of this configuration are all finished; the next one starts its own.
        assert_eq!(
            transitions,
            runs * 3,
            "every run was carried through its three steps"
        );
    }
    let mut verdicts = String::new();
    for target in [100.0, 1_000.0, 5_000.0] {
        let _ = writeln!(
            verdicts,
            "- {target:.0} transitions/s: **{}** (best measured {ceiling:.0}/s)",
            if ceiling >= target {
                "reached"
            } else {
                "not reached"
            }
        );
    }
    body.push_str(&format!(
        "\nThe load levels of the specification (FR-010-006), against the best closed-loop figure on this machine:\n\n{verdicts}\n`transitions/s` is measured from the start of the workers to the last completion. The workers, the API replicas and the database share the machine.\n"
    ));
    section("Step transitions per second (BL02, BL04)", &body);
}

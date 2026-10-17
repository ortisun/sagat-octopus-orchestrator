//! Shared machinery of the workflow-primitive workloads: a platform whose store negotiates worker
//! protocol 1.5, resource probes (threads, PostgreSQL connections, resident memory) and the report
//! fragments every workload writes.

use std::collections::BTreeMap;
use std::time::Duration;

use sagat_application::clock::SystemClock;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::worker::{Capabilities, ClaimedTask, WorkerService, WorkerSettings};
use sagat_domain::identity::{Principal, Role};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use uuid::Uuid;

use crate::support::engine::{Session, World, subject_of, task_ref, world};

pub const TRACE: &str = "0af7651916cd43dd8448eb211c80319c";

pub type ScopedWorkers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;
pub type ScopedScheduler = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;

/// A world plus the production wiring that executes scoped work: a store that negotiates worker
/// protocol 1.5, a worker service over it and a scheduler that holds every shard.
pub struct Platform {
    pub world: World,
    pub workers: ScopedWorkers,
    pub scheduler: ScopedScheduler,
}

pub async fn platform() -> Platform {
    let world = world().await;
    // Statement accounting needs the extension in this database; a server that does not preload
    // it refuses, and `statements` then reports it as unavailable.
    let _ = sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&world.db.pool)
        .await;
    let store =
        PgRunStore::new(world.db.pool.clone(), world.cache.clone()).with_scoped_worker_protocol();
    let workers = WorkerService::new(
        store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    )
    .with_vault(world.keys.cipher());
    let placement = Placement::new(format!("perf-scheduler-{}", Uuid::now_v7()));
    Keeper::new(
        store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(6 * 3600),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    let scheduler = Scheduler::new(
        store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_children(world.cache.clone())
    .with_placement(placement);
    Platform {
        world,
        workers,
        scheduler,
    }
}

impl Platform {
    /// A protocol 1.5 worker session of the first tenant.
    pub async fn session(&self, activities: &[&str]) -> Session {
        self.session_for(&self.world.worker.clone(), activities)
            .await
    }

    /// A worker of the second tenant, bound to its namespace the way an operator would.
    pub async fn other_tenant_worker(&self) -> Principal {
        let principal = Principal::new(
            self.world.other_admin.tenant_id(),
            subject_of(&format!("perf-worker-b-{}", Uuid::now_v7())),
            Role::Worker,
        );
        self.world.bind_everything(&principal).await;
        principal
    }

    pub async fn session_for(&self, principal: &Principal, activities: &[&str]) -> Session {
        let registered = self
            .workers
            .register(
                principal,
                "payments",
                &format!("perf-worker-{}", Uuid::now_v7()),
                &Capabilities {
                    activities: activities.iter().map(|a| (*a).to_owned()).collect(),
                    languages: vec!["rust".into()],
                    pools: Vec::new(),
                    max_inflight: 1000,
                    protocol_version: "1.5".into(),
                },
            )
            .await
            .expect("register the protocol 1.5 worker");
        assert_eq!(registered.accepted_protocol_version, "1.5");
        Session {
            principal: principal.clone(),
            worker_id: registered.worker_id,
            token: registered.session_token.expose().to_owned(),
        }
    }

    pub async fn poll(&self, session: &Session, max: u32) -> Vec<ClaimedTask> {
        self.workers
            .poll(
                &session.principal,
                session.worker_id,
                &session.token,
                max,
                Some(TRACE),
            )
            .await
            .expect("poll")
    }

    pub async fn complete(
        &self,
        session: &Session,
        task: &ClaimedTask,
        output: &serde_json::Value,
    ) {
        self.workers
            .complete(
                &session.principal,
                &task_ref(task),
                &serde_json::to_vec(output).expect("output"),
                &format!("perf-complete-{}", Uuid::now_v7()),
            )
            .await
            .expect("complete");
    }
}

// ------------------------------------------------------------------------------------------ stats

/// Latency samples by label, in insertion order of first use.
#[derive(Debug, Default)]
pub struct Samples(pub BTreeMap<String, Vec<Duration>>);

impl Samples {
    pub fn record(&mut self, label: &str, elapsed: Duration) {
        self.0.entry(label.to_owned()).or_default().push(elapsed);
    }

    pub fn rows(&self) -> String {
        let mut out = String::new();
        for (label, samples) in &self.0 {
            out.push_str(&super::duration_row(label, samples.clone()));
        }
        out
    }
}

pub const TABLE_HEAD: &str =
    "| Operation | Samples | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|---:|\n";

// -------------------------------------------------------------------------------------- resources

#[derive(Debug, Clone, Copy, Default)]
pub struct Resources {
    pub threads: u64,
    pub rss_bytes: u64,
    pub pg_connections: i64,
    pub pg_active: i64,
    pub pg_idle_in_transaction: i64,
}

/// The test process runs the engine, scheduler and worker service in-process, so its thread count
/// is the engine's. PostgreSQL connections are every backend of the benchmark database.
pub async fn resources(world: &World) -> Resources {
    let pid = std::process::id().to_string();
    let threads = if cfg!(target_os = "macos") {
        super::command(
            "sh",
            &["-c", &format!("ps -M -p {pid} | tail -n +2 | wc -l")],
        )
    } else {
        super::command("sh", &["-c", &format!("ls /proc/{pid}/task | wc -l")])
    }
    .trim()
    .parse()
    .unwrap_or(0);
    let row: (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), \
                count(*) FILTER (WHERE state = 'active'), \
                count(*) FILTER (WHERE state = 'idle in transaction') \
         FROM pg_stat_activity WHERE datname = current_database()",
    )
    .fetch_one(&world.db.pool)
    .await
    .expect("connection counts");
    Resources {
        threads,
        rss_bytes: super::process_rss_bytes(),
        pg_connections: row.0,
        pg_active: row.1,
        pg_idle_in_transaction: row.2,
    }
}

impl std::fmt::Display for Resources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} threads, {:.0} MiB RSS, {} PostgreSQL connections ({} active, {} idle in transaction)",
            self.threads,
            self.rss_bytes as f64 / 1_048_576.0,
            self.pg_connections,
            self.pg_active,
            self.pg_idle_in_transaction
        )
    }
}

/// PostgreSQL version and the settings that matter for the numbers.
pub async fn postgres_description(world: &World) -> String {
    let version: String = sqlx::query_scalar("SELECT version()")
        .fetch_one(&world.db.pool)
        .await
        .expect("version");
    let mut lines = vec![format!("- PostgreSQL: {version}")];
    for setting in [
        "shared_buffers",
        "max_connections",
        "work_mem",
        "effective_cache_size",
        "synchronous_commit",
        "fsync",
        "max_wal_size",
        "wal_level",
        "random_page_cost",
    ] {
        let value: String = sqlx::query_scalar("SELECT current_setting($1)")
            .bind(setting)
            .fetch_one(&world.db.pool)
            .await
            .unwrap_or_else(|_| "?".into());
        lines.push(format!("  - `{setting}` = `{value}`"));
    }
    lines.join("\n")
}

pub fn machine() -> String {
    let (cpu, memory) = if cfg!(target_os = "macos") {
        (
            super::command("sysctl", &["-n", "machdep.cpu.brand_string"]),
            super::command("sysctl", &["-n", "hw.memsize"]),
        )
    } else {
        (
            super::command(
                "sh",
                &["-c", "grep -m1 'model name' /proc/cpuinfo | cut -d: -f2"],
            ),
            super::command(
                "sh",
                &["-c", "grep MemTotal /proc/meminfo | awk '{print $2*1024}'"],
            ),
        )
    };
    let memory_gib = memory.trim().parse::<f64>().unwrap_or(0.0) / 1_073_741_824.0;
    format!(
        "- CPU: {cpu} ({} logical cores), {memory_gib:.0} GiB RAM\n- OS: {}\n- Load average at start: {}\n- Docker VM: {}\n- Build profile: {}\n- Commit: {}",
        std::thread::available_parallelism()
            .map(std::num::NonZero::get)
            .unwrap_or(0),
        super::command("uname", &["-srm"]),
        super::command("sh", &["-c", "uptime | sed 's/.*load averages*: //'"]),
        super::command(
            "sh",
            &[
                "-c",
                "docker info --format '{{.NCPU}} CPUs, {{.MemTotal}} bytes' 2>/dev/null"
            ]
        ),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        super::command("git", &["rev-parse", "--short", "HEAD"]),
    )
}

// --------------------------------------------------------------------------------------- fragments

/// Prints a report fragment and writes it to `SAGAT_PERF_FRAGMENTS/<name>.md` when that is set.
pub fn emit(name: &str, body: &str) {
    println!("\n===== {name} =====\n{body}");
    if let Ok(dir) = std::env::var("SAGAT_PERF_FRAGMENTS") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("fragment directory");
        std::fs::write(dir.join(format!("{name}.md")), body).expect("write fragment");
    }
}

pub fn budget() -> Duration {
    Duration::from_secs(
        std::env::var("SAGAT_PERF_BUDGET_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3_600),
    )
}

pub fn list_env(name: &str, default: &[usize]) -> Vec<usize> {
    std::env::var(name)
        .ok()
        .map(|v| v.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .filter(|v: &Vec<usize>| !v.is_empty())
        .unwrap_or_else(|| default.to_vec())
}

// -------------------------------------------------------------------------- statement accounting

/// Work PostgreSQL reports for the statements of this database. `rows` is rows returned or affected
/// and `buffers` is shared buffer hits plus reads: both are independent of machine load, which is
/// what makes a growth in them evidence of algorithmic cost rather than of a noisy host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Statements {
    pub calls: i64,
    pub rows: i64,
    pub buffers: i64,
}

impl std::ops::Sub for Statements {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self {
            calls: self.calls - other.calls,
            rows: self.rows - other.rows,
            buffers: self.buffers - other.buffers,
        }
    }
}

/// `None` when `pg_stat_statements` is not loaded on the server.
pub async fn statements(world: &World) -> Option<Statements> {
    let row: Option<(i64, i64, i64)> = sqlx::query_as(
        "SELECT coalesce(sum(calls),0)::bigint, coalesce(sum(rows),0)::bigint, \
                coalesce(sum(shared_blks_hit+shared_blks_read),0)::bigint \
         FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
           AND query NOT LIKE '%pg_stat_statements%'",
    )
    .fetch_optional(&world.db.pool)
    .await
    .ok()
    .flatten();
    row.map(|(calls, rows, buffers)| Statements {
        calls,
        rows,
        buffers,
    })
}

/// One sampled transition: its position, its latency and what it cost the database.
#[derive(Debug, Clone)]
pub struct Probe {
    pub label: String,
    pub sequence: usize,
    pub elapsed: Duration,
    pub cost: Statements,
}

pub fn probe_table(probes: &[Probe]) -> String {
    if probes.is_empty() {
        return "Statement accounting unavailable (`pg_stat_statements` not loaded).\n".to_owned();
    }
    let mut out = String::from(
        "| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |\n|---|---:|---:|---:|---:|---:|\n",
    );
    for probe in probes {
        out.push_str(&format!(
            "| {} | {} | {:.1} ms | {} | {} | {} |\n",
            probe.label,
            probe.sequence,
            super::duration_ms(probe.elapsed),
            probe.cost.calls,
            probe.cost.rows,
            probe.cost.buffers
        ));
    }
    out
}

/// Refreshes planner statistics the way autovacuum would on a long-lived database. The benchmark
/// databases are created empty a moment before the run, so without it the planner would still be
/// choosing plans for empty tables long after they hold thousands of rows.
pub async fn analyze(world: &World) {
    sqlx::query("ANALYZE")
        .execute(&world.db.pool)
        .await
        .expect("analyze");
}

/// Median `rows` of the first and the last `window` probes.
pub fn early_late_rows(probes: &[Probe], window: usize) -> Option<(i64, i64)> {
    if probes.len() < window * 2 {
        return None;
    }
    let median = |slice: &[Probe]| {
        let mut rows: Vec<i64> = slice.iter().map(|p| p.cost.rows).collect();
        rows.sort_unstable();
        rows[rows.len() / 2]
    };
    Some((
        median(&probes[..window]),
        median(&probes[probes.len() - window..]),
    ))
}

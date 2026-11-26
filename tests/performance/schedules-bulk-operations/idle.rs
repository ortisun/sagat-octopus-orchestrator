//! What the schedule passes cost a deployment that has no schedules (OD-2026-11-13-8: `SAGAT_SCHEDULES_ENABLED` is on by
//! default since 0.3.0, so every engine runs them whether anyone made a schedule or not; G2050 left it unmeasured).
//!
//! The passes are the ones `serve` starts when `settings::optional_passes` names `PASS_SCHEDULES`
//! (`apps/execution-engine/src/lib.rs`): the reconciliation of the trigger schedules (`loops::run_triggers` at `TRIGGER_POLL`,
//! with the convergence count it makes every sixth round that ends a sweep) and the schedule pass (`loops::run_schedules` at
//! `schedule_poll`), every round after the database clock is read again, over the pool and the run store the engine's other
//! services share. The configuration is the engine's own (`EngineConfig::from_lookup` over this process's environment), so
//! `SAGAT_SCHEDULES_POLL_MS` and `SAGAT_SCHEDULER_DISCOVERY_REST_MS` act here as they act in `serve`; "on" is the variable
//! absent (the default), "off" is `SAGAT_SCHEDULES_ENABLED=false`, and whether the passes run is decided by `optional_passes`.
//!
//! Two scenarios:
//!
//! * **idle**: a world with no schedule and no cron trigger (two tenants, or more with `SAGAT_PERF_IDLE_TENANTS`), five windows
//!   of `SAGAT_PERF_IDLE_SECONDS` each: nothing running, the schedule pass alone, the reconciliation alone, both (as the engine
//!   runs them), nothing running again. Each window reads, at its start and its end, the statements of the database
//!   (`pg_stat_statements`, when the server preloads it: calls and `total_exec_time`), the transactions and the active time of
//!   the database (`pg_stat_database`), the CPU time of the PostgreSQL container (its cgroup's `cpu.stat`, when `run.sh` names the
//!   container) and the CPU time of this process, which is the engine (`/proc/self/stat`, or `ps -o time=` on macOS). The cost is
//!   a window with passes less the mean of the two windows with none.
//! * **throughput**: runs of `a -> b` started through the run service and completed by workers through the worker service (a
//!   poll and a completion for each step), in segments of `SAGAT_PERF_THROUGHPUT_RUNS` runs, the passes on and off in pairs whose
//!   order alternates (off-on, on-off, ...) so that a drift of the machine's load does not fall on one side; the ratio on / off of
//!   each pair is the result.
//!
//! Bounds asserted (the spec promises nothing, NFR-018-005; these are what a reader would hold an idle feature to): the passes,
//! together, cost at most 10 statements a second, 10 ms of database execution time a second (1% of a core), 10 ms of the
//! engine's CPU a second, 20 ms of the database container's CPU a second, and no round fails; with them on, the median pair keeps
//! at least 95% of the run throughput it has with them off.
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use sagat_application::clock::SystemClock;
use sagat_application::execution::Engine;
use sagat_application::runs::RunService;
use sagat_application::schedule_firing::ScheduleFiring;
use sagat_application::schedule_triggers::TriggerReconciler;
use sagat_control_plane::config::Environment;
use sagat_domain::execution::RunStatus;
use sagat_domain::schedules::Ceilings;
use sagat_execution_engine::EngineConfig;
use sagat_execution_engine::{loops, settings};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{DatabaseClock, PgIdentityStore, PgScheduleStore};
use sagat_scheduler::{
    Keeper, KeeperSettings, Placement, SchedulePass, TriggerPass, TriggerSettings,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::harness::{Bench, RoundStats, bench, start_runs};
use crate::measure::{Summary, Verdict, conclude, environment, load_now, summary_json};
use crate::support::engine::{Runs, Session, World};

/// The three variables the engine refuses to start without; their values are never used here (no request is authenticated).
const OIDC: [(&str, &str); 3] = [
    ("SAGAT_OIDC_ISSUER", "https://idp.example/"),
    ("SAGAT_OIDC_AUDIENCE", "sagat"),
    ("SAGAT_OIDC_JWKS_URI", "https://idp.example/jwks.json"),
];

/// Callers that start runs, and workers that poll and complete them, at once. Twelve of the world's sixteen connections: the passes
/// share the pool with the load, as they share the engine's, and must not be the ones that wait for a connection.
const STARTERS: usize = 6;
const WORKERS: usize = 6;

// --- the engine's configuration ----------------------------------------------------------------------------------------

/// The engine's configuration as `serve` reads it from `get`, with `SAGAT_SCHEDULES_ENABLED` set to `schedules` (`None`: absent,
/// which is the default) and placeholders for the OIDC variables when `get` has none.
pub fn engine_config_from(
    get: &dyn Fn(&str) -> Option<String>,
    schedules: Option<&str>,
) -> EngineConfig {
    let lookup = |name: &str| -> Option<String> {
        if name == "SAGAT_SCHEDULES_ENABLED" {
            return schedules.map(str::to_owned);
        }
        get(name).or_else(|| {
            OIDC.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    };
    EngineConfig::from_lookup(&lookup, Environment::Prod).expect("the engine's configuration")
}

/// [`engine_config_from`] over this process's environment.
pub fn engine_config(schedules: Option<&str>) -> EngineConfig {
    engine_config_from(&|name| std::env::var(name).ok(), schedules)
}

/// Whether `serve` would start the schedule passes with this configuration.
pub fn runs_the_schedule_passes(config: &EngineConfig) -> bool {
    settings::optional_passes(config).contains(&settings::PASS_SCHEDULES)
}

// --- the passes, as `serve` wires them ---------------------------------------------------------------------------------

type Schedules =
    SchedulePass<PgScheduleStore, SpecCalendar, DatabaseClock<SystemClock>, PgIdentityStore, Runs>;
type Triggers = TriggerPass<PgScheduleStore, SpecCalendar, DatabaseClock<SystemClock>>;

/// The two loops of `PASS_SCHEDULES`, built once over the world's execution pool and run store (the ones its run and worker
/// services use), as `serve` builds them over the engine's.
pub struct Passes {
    clock: DatabaseClock<SystemClock>,
    schedules: Arc<Schedules>,
    triggers: Arc<Triggers>,
    poll: Duration,
}

/// Which of the two loops a window runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loops {
    Schedules,
    Triggers,
    Both,
}

impl Loops {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Schedules => "the schedule pass alone",
            Self::Triggers => "the reconciliation of trigger schedules alone",
            Self::Both => "both, as the engine runs them",
        }
    }
}

/// The loops of a window, until stopped, and what their rounds did.
pub struct Running {
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    pub schedules: Arc<RoundStats>,
    pub triggers: Arc<RoundStats>,
    /// Rounds of the reconciliation that measured the convergence (`TriggerRound::unconverged` is set).
    pub convergence: Arc<AtomicU64>,
}

/// What the rounds of a window came to, read at the end of the window (before the loops are stopped).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rounds {
    pub schedules: u64,
    pub triggers: u64,
    pub convergence: u64,
    pub failed: u64,
    pub refused: u64,
    pub steps: u64,
}

impl Running {
    pub fn rounds(&self) -> Rounds {
        Rounds {
            schedules: self.schedules.rounds.load(Ordering::Relaxed),
            triggers: self.triggers.rounds.load(Ordering::Relaxed),
            convergence: self.convergence.load(Ordering::Relaxed),
            failed: self.schedules.failed.load(Ordering::Relaxed)
                + self.triggers.failed.load(Ordering::Relaxed),
            refused: self.schedules.refused.load(Ordering::Relaxed)
                + self.triggers.refused.load(Ordering::Relaxed),
            steps: self.schedules.steps.load(Ordering::Relaxed)
                + self.triggers.steps.load(Ordering::Relaxed),
        }
    }

    pub async fn stop(self) {
        let _ = self.stop.send(true);
        for task in self.tasks {
            task.await.expect("the loop ends");
        }
    }
}

/// A placement that holds every shard for hours: the one engine of the deployment.
pub async fn own_the_shards(w: &World) -> Placement {
    let placement = Placement::new(format!("idle-engine-{}", Uuid::now_v7()));
    Keeper::new(
        w.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(6 * 3_600),
        },
    )
    .round()
    .await
    .expect("keeper round");
    assert_eq!(
        placement.owned().len(),
        64,
        "the one engine holds every shard"
    );
    placement
}

/// The passes `serve` would start with `config`: `None` when it starts none (`SAGAT_SCHEDULES_ENABLED=false`).
pub async fn engine_passes(
    w: &World,
    config: &EngineConfig,
    placement: &Placement,
) -> Option<Passes> {
    if !runs_the_schedule_passes(config) {
        return None;
    }
    let pool = w.execution_pool.clone();
    let clock = DatabaseClock::new(pool.clone(), SystemClock);
    clock.sync().await.expect("the database clock");
    let schedules = Arc::new(SchedulePass::new(
        ScheduleFiring::new(
            PgScheduleStore::new(pool.clone()),
            SpecCalendar,
            clock.clone(),
            PgIdentityStore::new(pool.clone()),
            RunService::new(
                w.store.clone(),
                SystemClock,
                Engine::new(JsonEvaluator),
                w.cache.clone(),
            ),
            Ceilings::default(),
        ),
        placement.clone(),
        config.schedules,
    ));
    let triggers = Arc::new(TriggerPass::new(
        TriggerReconciler::new(PgScheduleStore::new(pool), SpecCalendar, clock.clone()),
        placement.clone(),
        TriggerSettings::default(),
    ));
    Some(Passes {
        clock,
        schedules,
        triggers,
        poll: config.schedule_poll,
    })
}

impl Passes {
    pub fn spawn(&self, which: Loops) -> Running {
        let (stop, receiver) = watch::channel(false);
        let (schedule_stats, trigger_stats) = (
            Arc::new(RoundStats::default()),
            Arc::new(RoundStats::default()),
        );
        let convergence = Arc::new(AtomicU64::new(0));
        let mut tasks = Vec::new();
        if which != Loops::Schedules {
            let (clock, pass, stats, measured) = (
                self.clock.clone(),
                Arc::clone(&self.triggers),
                Arc::clone(&trigger_stats),
                Arc::clone(&convergence),
            );
            let receiver = receiver.clone();
            tasks.push(tokio::spawn(async move {
                loops::run_triggers(
                    || {
                        let (clock, pass, stats, measured) = (
                            clock.clone(),
                            Arc::clone(&pass),
                            Arc::clone(&stats),
                            Arc::clone(&measured),
                        );
                        async move {
                            let started = Instant::now();
                            let round = loops::trigger_round(&clock, &pass).await;
                            stats.record(
                                round.writes(),
                                round.refused,
                                round.failed,
                                started.elapsed(),
                            );
                            if round.unconverged.is_some() {
                                measured.fetch_add(1, Ordering::Relaxed);
                            }
                            round
                        }
                    },
                    loops::TRIGGER_POLL,
                    receiver,
                )
                .await;
            }));
        }
        if which != Loops::Triggers {
            let (clock, pass, stats, poll) = (
                self.clock.clone(),
                Arc::clone(&self.schedules),
                Arc::clone(&schedule_stats),
                self.poll,
            );
            tasks.push(tokio::spawn(async move {
                loops::run_schedules(
                    || {
                        let (clock, pass, stats) =
                            (clock.clone(), Arc::clone(&pass), Arc::clone(&stats));
                        async move {
                            let started = Instant::now();
                            let round = loops::schedule_round(&clock, &pass).await;
                            stats.record(
                                round.steps,
                                round.refused,
                                round.failed,
                                started.elapsed(),
                            );
                            round
                        }
                    },
                    poll,
                    receiver,
                )
                .await;
            }));
        }
        Running {
            stop,
            tasks,
            schedules: schedule_stats,
            triggers: trigger_stats,
            convergence,
        }
    }
}

// --- what a window reads ------------------------------------------------------------------------------------------------

/// The CPU seconds of a process from the text of its `/proc/<pid>/stat`: `utime` and `stime` (fields 14 and 15), in clock ticks.
pub fn parse_proc_stat(text: &str, ticks_per_second: f64) -> Option<f64> {
    // the command name (field 2) may hold spaces and parentheses: the fields that follow start after the last ')'
    let rest = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // `rest` starts at field 3 (the state), so field 14 is at index 11
    let utime: f64 = fields.get(11)?.parse().ok()?;
    let stime: f64 = fields.get(12)?.parse().ok()?;
    Some((utime + stime) / ticks_per_second)
}

/// The CPU seconds `ps -o time=` prints: `[dd-][hh:]mm:ss[.cc]` (macOS prints `m:ss.cc`, procps `[dd-]hh:mm:ss`).
pub fn parse_ps_time(text: &str) -> Option<f64> {
    let text = text.trim();
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<f64>().ok()?, clock),
        None => (0.0, text),
    };
    let mut seconds = 0.0;
    for part in clock.split(':') {
        seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
    }
    (!clock.is_empty()).then_some(days * 86_400.0 + seconds)
}

/// The CPU seconds of a cgroup from its `cpu.stat` (v2: `usage_usec`).
pub fn parse_cgroup_cpu(text: &str) -> Option<f64> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(' ')?;
        (key == "usage_usec").then(|| value.trim().parse::<f64>().ok().map(|usec| usec / 1e6))?
    })
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The CPU seconds this process (the engine) has used so far, user and system.
pub fn process_cpu_seconds() -> Option<f64> {
    if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
        let ticks = output("getconf", &["CLK_TCK"])
            .and_then(|value| value.parse().ok())
            .unwrap_or(100.0);
        return parse_proc_stat(&stat, ticks);
    }
    parse_ps_time(&output(
        "ps",
        &["-o", "time=", "-p", &std::process::id().to_string()],
    )?)
}

/// The CPU seconds the PostgreSQL container has used so far, every process of it (backends, checkpointer, WAL writer,
/// autovacuum), when `run.sh` names it in `SAGAT_PERF_PG_CONTAINER`.
pub fn database_cpu_seconds() -> Option<f64> {
    let container = std::env::var("SAGAT_PERF_PG_CONTAINER").ok()?;
    parse_cgroup_cpu(&output(
        "docker",
        &["exec", &container, "cat", "/sys/fs/cgroup/cpu.stat"],
    )?)
}

/// What `pg_stat_statements` says of the statements of this database, by statement: `(text, calls, execution ms)`.
pub type Statements = BTreeMap<String, (String, u64, f64)>;

/// The statements that ran between two readings, the most called first: `(text, calls, execution ms)`.
pub fn statement_delta(before: &Statements, after: &Statements) -> Vec<(String, u64, f64)> {
    let mut delta: Vec<(String, u64, f64)> = after
        .iter()
        .filter_map(|(key, (text, calls, ms))| {
            let (calls_before, ms_before) = before.get(key).map_or((0, 0.0), |(_, c, m)| (*c, *m));
            let calls = calls.saturating_sub(calls_before);
            (calls > 0).then(|| (text.clone(), calls, (ms - ms_before).max(0.0)))
        })
        .collect();
    delta.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    delta
}

/// One reading, at the start or the end of a window.
#[derive(Debug, Clone)]
pub struct Probe {
    pub at: Instant,
    pub process_cpu_s: Option<f64>,
    pub database_cpu_s: Option<f64>,
    pub statements: Option<Statements>,
    /// `pg_stat_database` of this database: transactions (committed and rolled back) and `active_time` in ms.
    pub transactions: Option<i64>,
    pub active_ms: Option<f64>,
}

/// Creates `pg_stat_statements` in this database when the server preloads it. Whether it does.
pub async fn statements_available(pool: &PgPool) -> bool {
    if sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(pool)
        .await
        .is_err()
    {
        return false;
    }
    read_statements(pool).await.is_some()
}

async fn read_statements(pool: &PgPool) -> Option<Statements> {
    // the readings of this module are left out (they run on the owner's pool and name the statistics views)
    let rows: Vec<(String, String, i64, f64)> = sqlx::query_as(
        "SELECT userid::text || ':' || coalesce(queryid::text, md5(query)) || ':' || toplevel::text, \
                left(regexp_replace(query, '\\s+', ' ', 'g'), 200), calls, total_exec_time \
           FROM pg_stat_statements \
          WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
            AND query NOT LIKE '%pg_stat%' AND query NOT LIKE '%pg_database%'",
    )
    .fetch_all(pool)
    .await
    .ok()?;
    Some(
        rows.into_iter()
            .map(|(key, text, calls, ms)| (key, (text, u64::try_from(calls).unwrap_or(0), ms)))
            .collect(),
    )
}

pub async fn probe(pool: &PgPool, statements: bool) -> Probe {
    let database: Option<(i64, f64)> = sqlx::query_as(
        "SELECT xact_commit + xact_rollback, active_time FROM pg_stat_database WHERE datname = current_database()",
    )
    .fetch_one(pool)
    .await
    .ok();
    Probe {
        at: Instant::now(),
        process_cpu_s: process_cpu_seconds(),
        database_cpu_s: database_cpu_seconds(),
        statements: if statements {
            read_statements(pool).await
        } else {
            None
        },
        transactions: database.map(|(transactions, _)| transactions),
        active_ms: database.map(|(_, active)| active),
    }
}

/// What a window cost, a second.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rates {
    pub statements: Option<f64>,
    pub exec_ms: Option<f64>,
    pub process_cpu_ms: Option<f64>,
    pub database_cpu_ms: Option<f64>,
    pub transactions: Option<f64>,
    pub active_ms: Option<f64>,
}

fn per_second(before: Option<f64>, after: Option<f64>, seconds: f64, scale: f64) -> Option<f64> {
    Some((after? - before?) * scale / seconds)
}

fn minus(on: Option<f64>, off: &[Option<f64>]) -> Option<f64> {
    let off: Option<Vec<f64>> = off.iter().copied().collect();
    let off = off?;
    if off.is_empty() {
        return on;
    }
    Some(on? - off.iter().sum::<f64>() / off.len() as f64)
}

impl Rates {
    pub fn between(before: &Probe, after: &Probe) -> (Self, Vec<(String, u64, f64)>) {
        let seconds = after.at.duration_since(before.at).as_secs_f64();
        let delta = match (&before.statements, &after.statements) {
            (Some(b), Some(a)) => Some(statement_delta(b, a)),
            _ => None,
        };
        let rates = Self {
            statements: delta
                .as_ref()
                .map(|d| d.iter().map(|(_, calls, _)| *calls as f64).sum::<f64>() / seconds),
            exec_ms: delta
                .as_ref()
                .map(|d| d.iter().map(|(_, _, ms)| *ms).sum::<f64>() / seconds),
            process_cpu_ms: per_second(before.process_cpu_s, after.process_cpu_s, seconds, 1_000.0),
            database_cpu_ms: per_second(
                before.database_cpu_s,
                after.database_cpu_s,
                seconds,
                1_000.0,
            ),
            transactions: per_second(
                before.transactions.map(|t| t as f64),
                after.transactions.map(|t| t as f64),
                seconds,
                1.0,
            ),
            active_ms: per_second(before.active_ms, after.active_ms, seconds, 1.0),
        };
        (rates, delta.unwrap_or_default())
    }

    /// This window less the mean of the windows in which nothing ran: what the passes added.
    pub fn excess(&self, baselines: &[Self]) -> Self {
        let field = |get: fn(&Self) -> Option<f64>| -> Option<f64> {
            minus(get(self), &baselines.iter().map(get).collect::<Vec<_>>())
        };
        Self {
            statements: field(|r| r.statements),
            exec_ms: field(|r| r.exec_ms),
            process_cpu_ms: field(|r| r.process_cpu_ms),
            database_cpu_ms: field(|r| r.database_cpu_ms),
            transactions: field(|r| r.transactions),
            active_ms: field(|r| r.active_ms),
        }
    }

    pub fn json(&self) -> Value {
        json!({
            "statements_per_s": self.statements, "exec_ms_per_s": self.exec_ms,
            "engine_cpu_ms_per_s": self.process_cpu_ms, "database_cpu_ms_per_s": self.database_cpu_ms,
            "transactions_per_s": self.transactions, "active_ms_per_s": self.active_ms,
        })
    }
}

/// Statements a round, from statements a second and rounds in a window of `seconds`.
pub fn per_round(per_second: Option<f64>, rounds: u64, seconds: f64) -> Option<f64> {
    (rounds > 0).then(|| per_second.map(|rate| rate * seconds / rounds as f64))?
}

// --- the idle scenario --------------------------------------------------------------------------------------------------

async fn quiet(pool: &PgPool, statements: bool, window: Duration) -> (Rates, Value) {
    let before = probe(pool, statements).await;
    tokio::time::sleep(window).await;
    let after = probe(pool, statements).await;
    let (rates, delta) = Rates::between(&before, &after);
    (
        rates,
        json!({"seconds": after.at.duration_since(before.at).as_secs_f64(), "rates": rates.json(),
                   "statements": delta.iter().take(10).map(|(t, c, m)| json!({"query": t, "calls": c, "exec_ms": m})).collect::<Vec<_>>()}),
    )
}

struct Measured {
    loops: Loops,
    rates: Rates,
    rounds: Rounds,
    seconds: f64,
    statements: Vec<(String, u64, f64)>,
}

async fn with_passes(
    pool: &PgPool,
    statements: bool,
    passes: &Passes,
    loops: Loops,
    window: Duration,
) -> Measured {
    let before = probe(pool, statements).await;
    let running = passes.spawn(loops);
    tokio::time::sleep(window).await;
    let rounds = running.rounds();
    let after = probe(pool, statements).await;
    running.stop().await;
    let (rates, delta) = Rates::between(&before, &after);
    Measured {
        loops,
        rates,
        rounds,
        seconds: after.at.duration_since(before.at).as_secs_f64(),
        statements: delta,
    }
}

/// Provisions `n` more tenants through the identity store (each with its administrator), as the platform would.
async fn add_tenants(w: &World, n: usize) {
    let store = PgIdentityStore::new(w.db.pool.clone());
    for i in 0..n {
        crate::support::provision(
            &store,
            &format!("idle-tenant-{i}"),
            &format!("idle-admin-{i}"),
        )
        .await;
    }
}

pub async fn idle(scenario: &str, tenants: usize, window: Duration) {
    let before = load_now();
    let b = bench().await;
    let environment = environment(&b.w.db.pool).await;
    add_tenants(&b.w, tenants.saturating_sub(2)).await;
    let counted: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM tenants), (SELECT count(*) FROM schedules), \
                (SELECT count(*) FROM workflow_triggers WHERE kind = 'cron')",
    )
    .fetch_one(&b.w.db.pool)
    .await
    .expect("count");
    assert_eq!(
        counted.0,
        i64::try_from(tenants.max(2)).expect("small"),
        "the tenants"
    );
    assert_eq!(
        (counted.1, counted.2),
        (0, 0),
        "no schedule and no cron trigger"
    );
    let statements = statements_available(&b.w.db.pool).await;

    let on = engine_config(None);
    let off = engine_config(Some("false"));
    assert!(
        on.schedules_enabled,
        "the schedule passes run by default (OD-2026-11-13-8)"
    );
    let placement = own_the_shards(&b.w).await;
    assert!(
        engine_passes(&b.w, &off, &placement).await.is_none(),
        "false starts no pass"
    );
    let passes = engine_passes(&b.w, &on, &placement)
        .await
        .expect("the default starts the passes");

    // a warm-up of both loops: the statements are prepared and the first convergence count (round 0) is made
    let warm = passes.spawn(Loops::Both);
    tokio::time::sleep(Duration::from_secs(12)).await;
    warm.stop().await;

    let pool = &b.w.db.pool;
    let (quiet_before, quiet_before_json) = quiet(pool, statements, window).await;
    let mut measured = Vec::new();
    for loops in [Loops::Schedules, Loops::Triggers, Loops::Both] {
        measured.push(with_passes(pool, statements, &passes, loops, window).await);
    }
    let (quiet_after, quiet_after_json) = quiet(pool, statements, window).await;
    let baselines = [quiet_before, quiet_after];

    let mut windows = Vec::new();
    for m in &measured {
        let excess = m.rates.excess(&baselines);
        let rounds = m.rounds.schedules + m.rounds.triggers;
        windows.push(json!({
            "loops": m.loops.label(), "seconds": m.seconds,
            "rounds": {"schedules": m.rounds.schedules, "triggers": m.rounds.triggers,
                       "convergence_counts": m.rounds.convergence, "failed": m.rounds.failed,
                       "refused": m.rounds.refused, "steps": m.rounds.steps},
            "rates": m.rates.json(), "excess": excess.json(),
            "excess_statements_per_round": per_round(excess.statements, rounds, m.seconds),
            "excess_exec_ms_per_round": per_round(excess.exec_ms, rounds, m.seconds),
            "statements": m.statements.iter().take(15)
                .map(|(t, c, ms)| json!({"query": t, "calls": c, "exec_ms": ms})).collect::<Vec<_>>(),
        }));
    }
    let both = measured.last().expect("both");
    let excess = both.rates.excess(&baselines);
    let fragment = json!({
        "tenants": tenants.max(2), "schedules": 0, "cron_triggers": 0, "window_s": window.as_secs_f64(),
        "pg_stat_statements": statements,
        "database_container": std::env::var("SAGAT_PERF_PG_CONTAINER").ok(),
        "settings": {
            "schedule_poll_ms": on.schedule_poll.as_millis() as u64,
            "trigger_poll_ms": loops::TRIGGER_POLL.as_millis() as u64,
            "discovery_rest_ms": on.schedules.discovery_rest.as_millis() as u64,
            "schedule_batch": on.schedules.batch, "round_actions": on.schedules.round_actions,
        },
        "baseline": [quiet_before_json, quiet_after_json],
        "windows": windows,
        "both": {"excess": excess.json(),
                 "statements_per_round": per_round(excess.statements, both.rounds.schedules + both.rounds.triggers, both.seconds)},
    });
    // a figure that could not be read (no pg_stat_statements, no container) is not a bound that held: it is reported as such
    let unread = |value: Option<f64>| value.unwrap_or(f64::INFINITY);
    let verdicts = [
        Verdict::at_most(
            "the_idle_passes_issue_at_most_ten_statements_a_second",
            "statements a second the two loops add on a deployment with no schedule (pg_stat_statements)",
            unread(excess.statements),
            10.0,
            "statements/s",
        ),
        Verdict::at_most(
            "the_idle_passes_cost_the_database_at_most_one_percent_of_a_core_in_execution",
            "execution time a second the two loops add (pg_stat_statements total_exec_time)",
            unread(excess.exec_ms),
            10.0,
            "ms/s",
        ),
        Verdict::at_most(
            "the_idle_passes_cost_the_engine_at_most_one_percent_of_a_core",
            "CPU time a second the two loops add to the engine process",
            unread(excess.process_cpu_ms),
            10.0,
            "ms/s",
        ),
        Verdict::at_most(
            "the_idle_passes_cost_the_database_container_at_most_two_percent_of_a_core",
            "CPU time a second the two loops add to the PostgreSQL container (cgroup cpu.stat)",
            unread(excess.database_cpu_ms),
            20.0,
            "ms/s",
        ),
        Verdict::at_most(
            "no_idle_round_fails",
            "rounds that failed or were refused, over the three windows with passes",
            measured
                .iter()
                .map(|m| (m.rounds.failed + m.rounds.refused) as f64)
                .sum(),
            0.0,
            "rounds",
        ),
    ];
    println!(
        "{scenario}: {} tenants, both loops: {:?} statements/s, {:?} exec ms/s, engine {:?} ms/s, database {:?} ms/s, {} + {} rounds",
        tenants.max(2),
        excess.statements,
        excess.exec_ms,
        excess.process_cpu_ms,
        excess.database_cpu_ms,
        both.rounds.schedules,
        both.rounds.triggers
    );
    conclude(scenario, environment, before, fragment, &verdicts);
}

// --- the throughput scenario --------------------------------------------------------------------------------------------

/// Whether each segment of `pairs` pairs runs the passes: off-on, on-off, off-on, ... (a drift of the load falls on both sides).
pub fn alternating(pairs: usize) -> Vec<bool> {
    (0..pairs)
        .flat_map(|pair| {
            if pair % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            }
        })
        .collect()
}

/// The ratio on / off of each pair of segments, from the order of [`alternating`] and the rate of each segment.
pub fn paired_ratios(order: &[bool], rates: &[f64]) -> Vec<f64> {
    order
        .chunks(2)
        .zip(rates.chunks(2))
        .filter(|(modes, rates)| modes.len() == 2 && rates.len() == 2)
        .map(|(modes, rates)| {
            let (on, off) = if modes[0] {
                (rates[0], rates[1])
            } else {
                (rates[1], rates[0])
            };
            on / off
        })
        .collect()
}

/// Starts `runs` runs of the bench's `a -> b` and completes both steps of each through the worker service. Runs a second, from
/// the first start to the completion of the last.
async fn segment(b: &Arc<Bench>, sessions: &[Arc<Session>], runs: usize) -> f64 {
    let done = Arc::new(AtomicUsize::new(0));
    let began = Instant::now();
    let starter = {
        let b = Arc::clone(b);
        tokio::spawn(async move { start_runs(&b, b.workflow, runs, STARTERS).await })
    };
    let workers: Vec<_> = sessions
        .iter()
        .map(|session| {
            let (b, session, done) = (Arc::clone(b), Arc::clone(session), Arc::clone(&done));
            tokio::spawn(async move {
                while done.load(Ordering::Relaxed) < runs {
                    let tasks = b.w.poll(&session, 16).await;
                    if tasks.is_empty() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        continue;
                    }
                    for task in &tasks {
                        let ack =
                            b.w.complete(&session, task, &json!({"ok": true}))
                                .await
                                .expect("the step completes");
                        if ack.run_status == RunStatus::Succeeded {
                            done.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            })
        })
        .collect();
    crate::support::bounded::within("a segment of runs", Duration::from_secs(1_800), async {
        starter.await.expect("the starts");
        for worker in workers {
            worker.await.expect("a worker");
        }
    })
    .await;
    runs as f64 / began.elapsed().as_secs_f64()
}

pub async fn throughput(scenario: &str, runs: usize, pairs: usize) {
    let before = load_now();
    let b = Arc::new(bench().await);
    let environment = environment(&b.w.db.pool).await;
    let placement = own_the_shards(&b.w).await;
    let on = engine_config(None);
    let off = engine_config(Some("false"));
    let on_passes = engine_passes(&b.w, &on, &placement).await;
    let off_passes = engine_passes(&b.w, &off, &placement).await;
    assert!(
        on_passes.is_some() && off_passes.is_none(),
        "the default runs the passes, false none"
    );
    let mut sessions = Vec::new();
    for _ in 0..WORKERS {
        sessions.push(Arc::new(b.w.session(&["act.a", "act.b"]).await));
    }
    // a warm-up segment of each mode, not measured
    for passes in [&off_passes, &on_passes] {
        let running = passes.as_ref().map(|p| p.spawn(Loops::Both));
        segment(&b, &sessions, (runs / 4).max(10)).await;
        if let Some(running) = running {
            running.stop().await;
        }
    }
    let order = alternating(pairs);
    let mut rates = Vec::new();
    let mut rounds = Vec::new();
    for &passes_on in &order {
        let passes = if passes_on { &on_passes } else { &off_passes };
        let running = passes.as_ref().map(|p| p.spawn(Loops::Both));
        rates.push(segment(&b, &sessions, runs).await);
        if let Some(running) = running {
            let r = running.rounds();
            rounds.push(json!({"schedules": r.schedules, "triggers": r.triggers, "failed": r.failed, "steps": r.steps}));
            running.stop().await;
        }
    }
    let ratios = paired_ratios(&order, &rates);
    let of = |on: bool| -> Vec<f64> {
        order
            .iter()
            .zip(&rates)
            .filter(|(m, _)| **m == on)
            .map(|(_, r)| *r)
            .collect()
    };
    let ratio = Summary::of(&ratios).expect("pairs");
    let fragment = json!({
        "runs_per_segment": runs, "pairs": pairs, "workflow": "a -> b (two activities)",
        "starters": STARTERS, "workers": WORKERS, "poll_max": 16,
        "order": order.iter().map(|on| if *on { "on" } else { "off" }).collect::<Vec<_>>(),
        "runs_per_s": rates,
        "off_runs_per_s": summary_json(&of(false)), "on_runs_per_s": summary_json(&of(true)),
        "pair_ratio_on_over_off": ratios, "ratio": ratio.json(),
        "rounds_while_on": rounds,
    });
    let verdicts = [Verdict::at_least(
        "the_passes_keep_ninety_five_percent_of_the_run_throughput",
        "median of the pairs' ratio, runs a second with the passes on over off",
        ratio.p50,
        0.95,
        "",
    )];
    println!(
        "{scenario}: off {:?} on {:?} ratio p50 {:.3} [{:.3} - {:.3}]",
        of(false),
        of(true),
        ratio.p50,
        ratio.min,
        ratio.max
    );
    conclude(scenario, environment, before, fragment, &verdicts);
}

#[cfg(test)]
mod tests {
    use super::{
        Probe, Rates, Statements, alternating, engine_config_from, paired_ratios, parse_cgroup_cpu,
        parse_proc_stat, parse_ps_time, per_round, runs_the_schedule_passes, statement_delta,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn the_schedule_passes_run_by_default_and_false_starts_none() {
        let nothing = |_: &str| -> Option<String> { None };
        let on = engine_config_from(&nothing, None);
        assert!(on.schedules_enabled && runs_the_schedule_passes(&on));
        assert_eq!(on.schedule_poll, Duration::from_millis(1_000));
        let off = engine_config_from(&nothing, Some("false"));
        assert!(!runs_the_schedule_passes(&off));
        let poll = |name: &str| (name == "SAGAT_SCHEDULES_POLL_MS").then(|| "250".to_owned());
        assert_eq!(
            engine_config_from(&poll, None).schedule_poll,
            Duration::from_millis(250)
        );
    }

    #[test]
    fn ps_time_is_read_in_the_formats_of_macos_and_procps() {
        assert_eq!(parse_ps_time("  0:01.25 "), Some(1.25));
        assert_eq!(parse_ps_time("429:26.39"), Some(429.0 * 60.0 + 26.39));
        assert_eq!(parse_ps_time("01:02:03"), Some(3_723.0));
        assert_eq!(parse_ps_time("2-00:00:01"), Some(2.0 * 86_400.0 + 1.0));
        assert_eq!(parse_ps_time(""), None);
        assert_eq!(parse_ps_time("soon"), None);
    }

    #[test]
    fn proc_stat_is_read_after_the_last_parenthesis() {
        // pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime stime ...
        let stat = "42 (a (b) c) S 1 42 42 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 8 0";
        assert_eq!(parse_proc_stat(stat, 100.0), Some(3.0));
        assert_eq!(parse_proc_stat("no parenthesis", 100.0), None);
    }

    #[test]
    fn the_cgroup_usage_is_read_in_seconds() {
        assert_eq!(
            parse_cgroup_cpu("usage_usec 13134\nuser_usec 6567\n"),
            Some(0.013_134)
        );
        assert_eq!(parse_cgroup_cpu("user_usec 6567\n"), None);
    }

    #[test]
    fn the_delta_of_the_statements_keeps_only_what_ran_the_most_called_first() {
        let mut before = Statements::new();
        before.insert("1".into(), ("SELECT a".into(), 10, 5.0));
        before.insert("2".into(), ("SELECT b".into(), 3, 1.0));
        let mut after = before.clone();
        after.insert("1".into(), ("SELECT a".into(), 12, 6.0));
        after.insert("3".into(), ("SELECT c".into(), 7, 2.0));
        let delta = statement_delta(&before, &after);
        assert_eq!(
            delta,
            vec![("SELECT c".into(), 7, 2.0), ("SELECT a".into(), 2, 1.0)]
        );
    }

    fn probe(at: Instant, cpu: f64, statements: u64) -> Probe {
        let mut map = Statements::new();
        map.insert(
            "1".into(),
            ("SELECT 1".into(), statements, statements as f64 * 0.5),
        );
        Probe {
            at,
            process_cpu_s: Some(cpu),
            database_cpu_s: None,
            statements: Some(map),
            transactions: Some(i64::try_from(statements).expect("small")),
            active_ms: Some(0.0),
        }
    }

    #[test]
    fn a_window_is_read_a_second_and_its_cost_is_what_the_quiet_windows_did_not_have() {
        let start = Instant::now();
        let (rates, _) = Rates::between(
            &probe(start, 1.0, 0),
            &probe(start + Duration::from_secs(10), 1.1, 40),
        );
        assert!((rates.statements.expect("read") - 4.0).abs() < 1e-9);
        assert!((rates.exec_ms.expect("read") - 2.0).abs() < 1e-9);
        assert!((rates.process_cpu_ms.expect("read") - 10.0).abs() < 1e-6);
        assert_eq!(rates.database_cpu_ms, None, "an unread figure stays unread");
        let quiet = Rates {
            statements: Some(1.0),
            process_cpu_ms: Some(2.0),
            ..Rates::default()
        };
        let louder = Rates {
            statements: Some(3.0),
            process_cpu_ms: Some(4.0),
            ..Rates::default()
        };
        let excess = rates.excess(&[quiet, louder]);
        assert!((excess.statements.expect("read") - 2.0).abs() < 1e-9);
        assert!((excess.process_cpu_ms.expect("read") - 7.0).abs() < 1e-6);
        assert_eq!(
            excess.exec_ms, None,
            "a baseline that was not read leaves the excess unread"
        );
        assert_eq!(per_round(Some(4.0), 20, 10.0), Some(2.0));
        assert_eq!(per_round(Some(4.0), 0, 10.0), None);
    }

    #[test]
    fn the_pairs_alternate_and_each_ratio_is_on_over_off() {
        assert_eq!(alternating(3), [false, true, true, false, false, true]);
        let ratios = paired_ratios(&alternating(2), &[100.0, 90.0, 80.0, 100.0]);
        assert_eq!(ratios, [0.9, 0.8]);
        assert!(paired_ratios(&alternating(1), &[100.0]).is_empty());
    }
}

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
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::cast_precision_loss
)]

#[path = "../../support/mod.rs"]
mod support;

mod diagnostics;
mod dr;
mod poll_client;
#[cfg(test)]
#[path = "transition_oracle_tests.rs"]
mod transition_oracle_tests;

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream;
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::AppError;
use sagat_application::execution::Engine;
use sagat_application::runs::{RunStore, StartRun};
use sagat_domain::execution::{RunId, RunStatus};
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
const INTEGRITY_ORACLE_LIMIT: Duration = Duration::from_secs(120);

/// Callers starting runs at the same time.
fn load_concurrency() -> usize {
    env_usize("SAGAT_PERF_LOAD_CONCURRENCY", 80)
}

fn app_error_variant(error: &AppError) -> &'static str {
    match error {
        AppError::Unauthenticated => "Unauthenticated",
        AppError::Forbidden { .. } => "Forbidden",
        AppError::NotFound => "NotFound",
        AppError::Conflict(_) => "Conflict",
        AppError::RevisionConflict { .. } => "RevisionConflict",
        AppError::IdempotencyKeyReused => "IdempotencyKeyReused",
        AppError::InvalidDefinition(_) => "InvalidDefinition",
        AppError::NotAllowed(_) => "NotAllowed",
        AppError::NotBound => "NotBound",
        AppError::SecretUnavailable { .. } => "SecretUnavailable",
        AppError::StaleAttempt => "StaleAttempt",
        AppError::StateConflict(_) => "StateConflict",
        AppError::InvalidSchedule { .. } => "InvalidSchedule",
        AppError::ScheduleConflict { .. } => "ScheduleConflict",
        AppError::Invalid(_) => "Invalid",
        AppError::InvalidScript(_) => "InvalidScript",
        AppError::QuotaExceeded(_) => "QuotaExceeded",
        AppError::Versioning(_) => "Versioning",
        AppError::Unavailable => "Unavailable",
        AppError::PayloadTooLarge(_) => "PayloadTooLarge",
        AppError::Busy { .. } => "Busy",
        AppError::Internal => "Internal",
    }
}

struct WorkerFailure {
    worker: usize,
    operation: &'static str,
    variant: &'static str,
    retryable: bool,
    attempts: u32,
    retryable_failures: u32,
    exhausted: bool,
    logical_call_elapsed: Duration,
}

struct FailedCall {
    attempts: u32,
    retryable_failures: u32,
    exhausted: bool,
    elapsed: Duration,
}

struct WorkerProgress {
    polls: Vec<Duration>,
    logical_polls: Vec<Duration>,
    completes: Vec<Duration>,
    poll_counters: poll_client::PollCounters,
    completions: Vec<CompletedTask>,
    last_done: Instant,
}

struct CompletedTask {
    task_id: String,
    run_id: RunId,
    node: String,
}

impl WorkerProgress {
    fn new() -> Self {
        Self {
            polls: Vec::new(),
            logical_polls: Vec::new(),
            completes: Vec::new(),
            poll_counters: poll_client::PollCounters::default(),
            completions: Vec::new(),
            last_done: Instant::now(),
        }
    }

    fn record_poll_success(&mut self, outcome: &poll_client::PollSuccess) {
        self.logical_polls.push(outcome.logical_call_elapsed);
        if !outcome.tasks.is_empty() {
            self.polls.push(
                outcome.success_attempt_elapsed
                    / u32::try_from(outcome.tasks.len()).expect("small poll batch"),
            );
        }
    }
}

#[derive(Clone, Copy)]
struct TransitionConfig {
    api_replicas: usize,
    workers: usize,
    runs: usize,
}

struct FailureDiagnostics<'a> {
    started: &'a AtomicBool,
    observer: &'a World,
    apis: &'a [Arc<support::engine::Replica>],
    storage: &'a diagnostics::SafeStorageDiagnostics,
    config: TransitionConfig,
}

async fn report_worker_failure(
    w: &World,
    apis: &[Arc<support::engine::Replica>],
    storage_diagnostics: &diagnostics::SafeStorageDiagnostics,
    config: TransitionConfig,
    failure: &WorkerFailure,
) {
    let TransitionConfig {
        api_replicas,
        workers,
        runs,
    } = config;
    eprintln!(
        "transition configuration failure: api_replicas={api_replicas} workers={workers} runs={runs} api_pool_connections={API_POOL} worker={} operation={} error_variant={} retryable={} attempts={} retryable_failures={} exhausted={} logical_call_ms={:.1}",
        failure.worker,
        failure.operation,
        failure.variant,
        failure.retryable,
        failure.attempts,
        failure.retryable_failures,
        failure.exhausted,
        ms(failure.logical_call_elapsed),
    );
    for (replica, api) in apis.iter().enumerate() {
        let size = api.execution_pool.size();
        let idle = u32::try_from(api.execution_pool.num_idle()).unwrap_or(u32::MAX);
        eprintln!(
            "transition replica pool: replica={replica} max={API_POOL} size={size} idle={idle} in_use={} closed={}",
            size.saturating_sub(idle),
            api.execution_pool.is_closed()
        );
    }
    let mut cause_seen = false;
    for (classification, observations) in storage_diagnostics.counts() {
        cause_seen = true;
        eprintln!(
            "transition storage cause: classification={classification} observations={observations}"
        );
    }
    if !cause_seen {
        eprintln!("transition storage cause: classification=other observations=0");
    }
    let activity = tokio::time::timeout(
        Duration::from_millis(500),
        sqlx::query_as::<_, (String, String, String, bool, i64, i64)>(
            "SELECT coalesce(state, 'unknown'), coalesce(wait_event_type, 'CPU'), \
                coalesce(wait_event, 'running'), cardinality(pg_blocking_pids(pid)) > 0, \
                count(*)::bigint, \
                coalesce(max(extract(epoch FROM (clock_timestamp() - \
                    coalesce(xact_start, query_start, state_change))) * 1000)::bigint, 0) \
           FROM pg_stat_activity \
          WHERE datname = current_database() AND backend_type = 'client backend' \
            AND pid <> pg_backend_pid() \
          GROUP BY 1, 2, 3, 4 ORDER BY 1, 2, 3, 4",
        )
        .fetch_all(&w.db.pool),
    )
    .await;
    match activity {
        Ok(Ok(rows)) => {
            for (state, wait_type, wait, blocked, connections, max_age_ms) in rows {
                eprintln!(
                    "transition database state: state={state} wait_type={wait_type} wait={wait} blocked={blocked} connections={connections} max_age_ms={max_age_ms}"
                );
            }
        }
        Ok(Err(_)) => eprintln!("transition database state: unavailable"),
        Err(_) => eprintln!("transition database state: timed_out"),
    }
}

async fn capture_worker_failure(
    error: AppError,
    worker: usize,
    operation: &'static str,
    call: FailedCall,
    diagnostics: &FailureDiagnostics<'_>,
) -> WorkerFailure {
    let failure = WorkerFailure {
        worker,
        operation,
        variant: app_error_variant(&error),
        retryable: error.is_retryable(),
        attempts: call.attempts,
        retryable_failures: call.retryable_failures,
        exhausted: call.exhausted,
        logical_call_elapsed: call.elapsed,
    };
    if diagnostics
        .started
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return std::future::pending().await;
    }
    report_worker_failure(
        diagnostics.observer,
        diagnostics.apis,
        diagnostics.storage,
        diagnostics.config,
        &failure,
    )
    .await;
    failure
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

struct StartedRun {
    run_id: RunId,
    elapsed: Duration,
}

/// Starts runs `from..to` through the same public service path used by every load scenario.
async fn start_runs_with_ids(
    w: &World,
    wf: sagat_domain::definition::WorkflowId,
    tag: &str,
    from: usize,
    to: usize,
) -> Vec<StartedRun> {
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
                let run_id = api
                    .runs
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
                    .expect("start")
                    .info
                    .run_id;
                StartedRun {
                    run_id,
                    elapsed: t.elapsed(),
                }
            }
        })
        .buffer_unordered(load_concurrency())
        .collect()
        .await
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
    start_runs_with_ids(w, wf, tag, from, to)
        .await
        .into_iter()
        .map(|run| run.elapsed)
        .collect()
}

#[derive(Clone, Copy, Debug)]
enum IntegrityFailure {
    DeadlockObserved,
    DuplicateStartedRun,
    CompletionCount,
    DuplicateTask,
    DuplicateRunNode,
    ForeignRun,
    NodeSet,
    ProjectionUnavailable,
    ProjectionMissing,
    RunNotTerminal,
    StepProjectionUnavailable,
    StepProjectionMismatch,
    JournalUnavailable,
    JournalMissing,
    JournalMismatch,
    ReplayMismatch,
    LeaseObservationUnavailable,
    LeaseRemaining,
    Pool(poll_client::PoolOracleFailure),
    TimedOut,
}

impl IntegrityFailure {
    const fn label(self) -> &'static str {
        match self {
            Self::DeadlockObserved => "deadlock_observed",
            Self::DuplicateStartedRun => "duplicate_started_run",
            Self::CompletionCount => "completion_count_mismatch",
            Self::DuplicateTask => "duplicate_task_id",
            Self::DuplicateRunNode => "duplicate_run_node",
            Self::ForeignRun => "foreign_run_completion",
            Self::NodeSet => "run_node_set_mismatch",
            Self::ProjectionUnavailable => "run_projection_unavailable",
            Self::ProjectionMissing => "run_projection_missing",
            Self::RunNotTerminal => "run_not_terminal",
            Self::StepProjectionUnavailable => "step_projection_unavailable",
            Self::StepProjectionMismatch => "step_projection_mismatch",
            Self::JournalUnavailable => "journal_unavailable",
            Self::JournalMissing => "journal_missing",
            Self::JournalMismatch => "journal_mismatch",
            Self::ReplayMismatch => "journal_replay_mismatch",
            Self::LeaseObservationUnavailable => "lease_observation_unavailable",
            Self::LeaseRemaining => "lease_remaining",
            Self::Pool(failure) => failure.label(),
            Self::TimedOut => "integrity_oracle_timed_out",
        }
    }
}

fn verify_completion_identities(
    started_runs: &[RunId],
    completions: &[CompletedTask],
) -> Result<(), IntegrityFailure> {
    let expected_runs = started_runs
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    if expected_runs.len() != started_runs.len() {
        return Err(IntegrityFailure::DuplicateStartedRun);
    }
    if completions.len() != started_runs.len() * 3 {
        return Err(IntegrityFailure::CompletionCount);
    }
    if completions
        .iter()
        .map(|completion| completion.task_id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len()
        != completions.len()
    {
        return Err(IntegrityFailure::DuplicateTask);
    }
    if completions
        .iter()
        .map(|completion| (completion.run_id, completion.node.as_str()))
        .collect::<std::collections::HashSet<_>>()
        .len()
        != completions.len()
    {
        return Err(IntegrityFailure::DuplicateRunNode);
    }
    let mut nodes_by_run: std::collections::HashMap<RunId, std::collections::HashSet<&str>> =
        std::collections::HashMap::new();
    for completion in completions {
        if !expected_runs.contains(&completion.run_id) {
            return Err(IntegrityFailure::ForeignRun);
        }
        nodes_by_run
            .entry(completion.run_id)
            .or_default()
            .insert(&completion.node);
    }
    let expected_nodes = ["a", "b", "c"].into_iter().collect();
    for run in &expected_runs {
        if nodes_by_run.get(run) != Some(&expected_nodes) {
            return Err(IntegrityFailure::NodeSet);
        }
    }
    Ok(())
}

async fn bounded_integrity_oracle<F>(limit: Duration, oracle: F) -> Result<(), IntegrityFailure>
where
    F: std::future::Future<Output = Result<(), IntegrityFailure>>,
{
    match tokio::time::timeout(limit, oracle).await {
        Ok(result) => result,
        Err(_) => Err(IntegrityFailure::TimedOut),
    }
}

async fn verify_transition_integrity(
    w: &World,
    apis: &[Arc<support::engine::Replica>],
    started_runs: &[RunId],
    completions: &[CompletedTask],
    deadlocks: u64,
) -> Result<(), IntegrityFailure> {
    if deadlocks != 0 {
        return Err(IntegrityFailure::DeadlockObserved);
    }
    verify_completion_identities(started_runs, completions)?;

    let verified = stream::iter(started_runs.iter().copied())
        .map(|run| async move {
            let projected = w
                .store
                .get(w.admin.tenant_id(), run)
                .await
                .map_err(|_| IntegrityFailure::ProjectionUnavailable)?
                .ok_or(IntegrityFailure::ProjectionMissing)?;
            if projected.status != RunStatus::Succeeded {
                return Err(IntegrityFailure::RunNotTerminal);
            }
            let steps: Vec<(String, String)> = sqlx::query_as(
                "SELECT node, status FROM steps WHERE tenant_id = $1 AND run_id = $2",
            )
            .bind(w.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .fetch_all(&w.db.pool)
            .await
            .map_err(|_| IntegrityFailure::StepProjectionUnavailable)?;
            let projected_steps = steps.into_iter().collect::<std::collections::HashSet<_>>();
            let expected_steps = [
                ("a".to_owned(), "SUCCEEDED".to_owned()),
                ("b".to_owned(), "SUCCEEDED".to_owned()),
                ("c".to_owned(), "SUCCEEDED".to_owned()),
            ]
            .into_iter()
            .collect();
            if projected_steps != expected_steps {
                return Err(IntegrityFailure::StepProjectionMismatch);
            }
            let journal = w
                .store
                .journal(w.admin.tenant_id(), run, None, 500)
                .await
                .map_err(|_| IntegrityFailure::JournalUnavailable)?
                .ok_or(IntegrityFailure::JournalMissing)?;
            if journal
                .iter()
                .filter(|event| event.event_type == "step.completed")
                .count()
                != 3
                || journal
                    .iter()
                    .filter(|event| event.event_type == "run.finished")
                    .count()
                    != 1
            {
                return Err(IntegrityFailure::JournalMismatch);
            }
            w.store
                .verify_run(w.admin.tenant_id(), run)
                .await
                .map_err(|_| IntegrityFailure::ReplayMismatch)
        })
        .buffer_unordered(load_concurrency())
        .collect::<Vec<Result<(), IntegrityFailure>>>()
        .await;
    for result in verified {
        result?;
    }

    let run_ids = started_runs
        .iter()
        .map(|run| run.as_uuid())
        .collect::<Vec<_>>();
    let leased: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE run_id = ANY($1) AND status = 'LEASED'",
    )
    .bind(&run_ids)
    .fetch_one(&w.db.pool)
    .await
    .map_err(|_| IntegrityFailure::LeaseObservationUnavailable)?;
    if leased != 0 {
        return Err(IntegrityFailure::LeaseRemaining);
    }
    poll_client::verify_replica_transactions(&w.db.pool, apis)
        .await
        .map_err(IntegrityFailure::Pool)?;
    Ok(())
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

/// The lease of the harness's keepers. The harness rounds once and then measures for as long as the
/// scenario takes, so the lease must outlast the scenario or the snapshot of what a scheduler holds
/// expires on its own clock and the passes measured would be empty ones.
const HARNESS_LEASE: Duration = Duration::from_secs(3_600);

/// `n` replicas that share the 64 shards, each with its scheduler.
async fn replicas(w: &World, n: usize, settings: SchedulerSettings) -> Vec<Sched> {
    let keepers: Vec<Keeper<PgRunStore>> = (0..n)
        .map(|i| {
            Keeper::new(
                w.store.clone(),
                Placement::new(format!("bench-{n}-{i}-{}", uuid::Uuid::new_v4().simple())),
                KeeperSettings { ttl: HARNESS_LEASE },
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
        "Tenant A starts {noisy_runs} timers and tenant B {quiet_runs} (B's are the younger), and every one of them falls due at the same instant: A has {:.0}% of the work. One replica, concurrency 8, batch 200. `lateness` is the time from becoming due to the `timer.fired` event. With fairness off a pass takes the oldest 200 candidates, as before feature 010. The table is vacuumed before each policy, so that the dead index entries of the timers the previous policy fired do not weigh on the next one.\n\n",
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
        // Each policy starts from a table the previous one left clean. The timers it fired leave
        // dead entries in the partial indexes that a pass reads through until vacuum removes them
        // (autovacuum does, in production); left there they make every later policy slower than
        // the one before whatever its weights, which a first run of this scenario showed.
        sqlx::query("VACUUM (ANALYZE) steps, runs")
            .execute(&w.db.pool)
            .await
            .expect("vacuum");
        // Every timer of both tenants due now, in the order the scenario says: all of A's first, and
        // B's after them, a microsecond apart. (Left to the clock, the order follows the rows' places
        // on disk, and after a vacuum the places of the new rows are the old ones' free space, so B's
        // timers would be mixed in with A's and the policies would not be given the same work.)
        let first_due: time::OffsetDateTime =
            sqlx::query_scalar("SELECT clock_timestamp() - interval '1 second'")
                .fetch_one(&w.db.pool)
                .await
                .expect("clock");
        sqlx::query(
            "WITH ordered AS ( \
                 SELECT run_id, node, row_number() OVER (ORDER BY (tenant_id = $1) DESC, run_id) AS n \
                   FROM steps WHERE waiting_kind = 'TIMER' AND due_at > clock_timestamp() + interval '30 minutes') \
             UPDATE steps s SET due_at = $2 + make_interval(secs => ordered.n / 1000000.0) \
               FROM ordered WHERE s.run_id = ordered.run_id AND s.node = ordered.node",
        )
        .bind(noisy.as_uuid())
        .bind(first_due)
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
            KeeperSettings { ttl: HARNESS_LEASE },
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
    let storage_diagnostics = diagnostics::SafeStorageDiagnostics::install();
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
        "{runs} runs of three chained activities per configuration; a worker polls up to 16 tasks and completes each one (a complete is one transition: the commit, the routing and the next step becoming ready). Workers are tasks in this process with sessions of their own, spread over identities (a session budget is 32 per identity). The worker API is served by 1 to 8 replicas, each with a pool of {API_POOL} connections (the engine's default is 20): a worker's calls go to the replica its number selects. Closed loop: the workers take work as fast as they can, so the figure is the ceiling of this machine, not a rate that was asked for. PollTasks retries only typed Unavailable, with four total attempts, 200 ms initial exponential delay, random factor [0.5, 1.0] and a 5 s cap; completion and every other application error remain strict. `Final successful poll attempt per task` retains the existing final-attempt elapsed/tasks normalization for successful nonempty polls. `Logical poll` is one successful call from its first attempt through success, including failed attempts and backoff; failed attempts are counters, not separate successful latency samples. Throughput is worker start through the last acknowledged completion and excludes the final empty-poll drain; configuration wall time includes that drain but excludes run/session setup and integrity verification.\n\n"
    ));
    body.push_str("| API replicas | Workers | Transitions | Transitions/s | Poll attempts / retryable failures / exhausted calls | Final successful poll attempt per task p50 / p95 / p99 | Successful logical poll p50 / p95 / p99 | Complete p50 / p95 / p99 | Config wall incl. drain | What the active backends of the database were doing (share of 25 ms samples) |\n|---:|---:|---:|---:|---:|---|---|---|---:|---|\n");
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
        let config = TransitionConfig {
            api_replicas,
            workers,
            runs,
        };
        eprintln!(
            "transition configuration start: api_replicas={api_replicas} workers={workers} runs={runs} api_pool_connections={API_POOL}"
        );
        storage_diagnostics.reset();
        // The worker API replicas of this configuration, each with a pool of its own.
        let mut apis = Vec::new();
        for _ in 0..api_replicas {
            apis.push(Arc::new(w.replica_with_pool(API_POOL).await));
        }
        let apis = Arc::new(apis);
        // Fresh runs for this configuration.
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let started_runs = start_runs_with_ids(&w, wf, &format!("chain-{tag}"), 0, runs)
            .await
            .into_iter()
            .map(|run| run.run_id)
            .collect::<Vec<_>>();
        assert_eq!(started_runs.len(), runs, "all configured runs started");
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
        // While the workers run, the database is asked every 25 ms what its active backends are
        // doing: running on a CPU, or waiting for a lock, the WAL, a buffer... The figure of the
        // table says how much; this says what the time was spent on.
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sampler = tokio::spawn({
            let (w, stop) = (Arc::clone(&w), Arc::clone(&stop));
            async move {
                let mut seen: std::collections::BTreeMap<String, u64> = Default::default();
                let mut total = 0_u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let rows: Result<Vec<(String, String, i64)>, _> = sqlx::query_as(
                        "SELECT coalesce(wait_event_type, 'CPU'), coalesce(wait_event, 'running'), count(*)::bigint \
                           FROM pg_stat_activity \
                          WHERE state = 'active' AND backend_type = 'client backend' AND pid <> pg_backend_pid() \
                          GROUP BY 1, 2",
                    )
                    .fetch_all(&w.db.pool)
                    .await;
                    for (kind, event, n) in rows.unwrap_or_default() {
                        let n = u64::try_from(n).unwrap_or(0);
                        *seen.entry(format!("{kind}:{event}")).or_default() += n;
                        total += n;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                (seen, total)
            }
        });
        let started = Instant::now();
        let diagnostic_started = Arc::new(AtomicBool::new(false));
        let mut progress = (0..workers)
            .map(|_| WorkerProgress::new())
            .collect::<Vec<_>>();
        let failure =
            diagnostics::stop_on_first_error(sessions.iter().zip(&mut progress).enumerate().map(
                |(n, (session, progress))| {
                    let api = Arc::clone(&apis[n % api_replicas]);
                    let diagnostic_apis = Arc::clone(&apis);
                    let diagnostic_started = Arc::clone(&diagnostic_started);
                    let storage_diagnostics = storage_diagnostics.clone();
                    let observer = Arc::clone(&w);
                    async move {
                        let failure_diagnostics = FailureDiagnostics {
                            started: &diagnostic_started,
                            observer: &observer,
                            apis: &diagnostic_apis,
                            storage: &storage_diagnostics,
                            config,
                        };
                        let mut empty = 0;
                        while empty < 3 {
                            let outcome = match poll_client::poll(
                                &api,
                                session,
                                16,
                                &mut progress.poll_counters,
                            )
                            .await
                            {
                                Ok(outcome) => outcome,
                                Err(failed_poll) => {
                                    let failure = capture_worker_failure(
                                        failed_poll.error,
                                        n,
                                        "poll",
                                        FailedCall {
                                            attempts: failed_poll.attempts,
                                            retryable_failures: failed_poll.retryable_failures,
                                            exhausted: failed_poll.exhausted,
                                            elapsed: failed_poll.logical_call_elapsed,
                                        },
                                        &failure_diagnostics,
                                    )
                                    .await;
                                    return Err(failure);
                                }
                            };
                            progress.record_poll_success(&outcome);
                            let tasks = outcome.tasks;
                            if tasks.is_empty() {
                                empty += 1;
                                continue;
                            }
                            empty = 0;
                            for task in &tasks {
                                let t = Instant::now();
                                if let Err(error) = api
                                    .workers
                                    .complete(
                                        &session.principal,
                                        &support::engine::task_ref(task),
                                        b"{}",
                                        &format!("done-{}", task.task_id()),
                                    )
                                    .await
                                {
                                    let failure = capture_worker_failure(
                                        error,
                                        n,
                                        "complete",
                                        FailedCall {
                                            attempts: 1,
                                            retryable_failures: 0,
                                            exhausted: false,
                                            elapsed: t.elapsed(),
                                        },
                                        &failure_diagnostics,
                                    )
                                    .await;
                                    return Err(failure);
                                }
                                progress.completes.push(t.elapsed());
                                progress.completions.push(CompletedTask {
                                    task_id: task.task_id(),
                                    run_id: task.run_id,
                                    node: task.instance.node().as_str().to_owned(),
                                });
                                progress.last_done = Instant::now();
                            }
                        }
                        Ok(())
                    }
                },
            ))
            .await
            .err();
        let failed = failure.is_some();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut waiting_on = match diagnostics::finish_sampler(sampler, failed)
            .await
            .expect("the sampler")
        {
            Some((seen, samples)) => {
                let mut by_share: Vec<_> = seen.into_iter().collect();
                by_share.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
                by_share
                    .iter()
                    .take(4)
                    .map(|(event, n)| {
                        format!("{event} {:.0}%", 100.0 * *n as f64 / samples.max(1) as f64)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
            None => "sampling unavailable after worker failure".to_owned(),
        };
        let config_wall = started.elapsed();
        // The rate is over the time the work took: until the last completion, not the last empty poll.
        let (mut polls, mut logical_polls, mut completes) = (Vec::new(), Vec::new(), Vec::new());
        let (mut poll_attempts, mut retryable_failures, mut exhausted_calls) = (0, 0, 0);
        let mut completed_tasks = Vec::new();
        let mut finished = started;
        for worker in progress {
            polls.extend(worker.polls);
            logical_polls.extend(worker.logical_polls);
            completes.extend(worker.completes);
            poll_attempts += worker.poll_counters.attempts;
            retryable_failures += worker.poll_counters.retryable_failures;
            exhausted_calls += worker.poll_counters.exhausted_calls;
            completed_tasks.extend(worker.completions);
            finished = finished.max(worker.last_done);
        }
        let transitions = completes.len();
        polls.sort_unstable();
        logical_polls.sort_unstable();
        completes.sort_unstable();
        let storage_causes = storage_diagnostics.counts().collect::<Vec<_>>();
        if retryable_failures > 0 {
            let safe_summary = if storage_causes.is_empty() {
                "other=0".to_owned()
            } else {
                storage_causes
                    .iter()
                    .map(|(classification, observations)| {
                        format!("{classification}={observations}")
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            };
            eprintln!(
                "transition recovered retries: api_replicas={api_replicas} workers={workers} runs={runs} retryable_failures={retryable_failures} safe_storage_causes={safe_summary}"
            );
            waiting_on.push_str(&format!("; retry storage causes {safe_summary}"));
        }
        if let Some(failure) = failure {
            body.push_str(&format!(
                "| {api_replicas} | {workers} | {transitions} (incomplete) | n/a | {poll_attempts} / {retryable_failures} / {exhausted_calls} | {:.0} / {:.0} / {:.0} ms | {:.0} / {:.0} / {:.0} ms | {:.0} / {:.0} / {:.0} ms | {:.2} s | {waiting_on} |\n",
                ms(percentile(&polls, 0.50)),
                ms(percentile(&polls, 0.95)),
                ms(percentile(&polls, 0.99)),
                ms(percentile(&logical_polls, 0.50)),
                ms(percentile(&logical_polls, 0.95)),
                ms(percentile(&logical_polls, 0.99)),
                ms(percentile(&completes, 0.50)),
                ms(percentile(&completes, 0.95)),
                ms(percentile(&completes, 0.99)),
                config_wall.as_secs_f64(),
            ));
            body.push_str(&format!(
                "\nConfiguration `{api_replicas}` API replicas / `{workers}` workers / `{runs}` runs is **incomplete**: worker {} failed during `{}` with safe application error variant `{}` (`retryable={}`) after {} attempt(s), {} retryable failure(s), exhausted={}, and {:.1} ms logical-call time. The failed logical call is not a successful latency sample. The transition count contains acknowledged completions only; no throughput result, integrity verdict or load-level verdict is reported for this configuration. Earlier rows remain complete.\n",
                failure.worker,
                failure.operation,
                failure.variant,
                failure.retryable,
                failure.attempts,
                failure.retryable_failures,
                failure.exhausted,
                ms(failure.logical_call_elapsed),
            ));
            section(
                "Step transitions per second (BL02, BL04) — incomplete",
                &body,
            );
            panic!(
                "transition configuration failed: api_replicas={api_replicas} workers={workers} runs={runs} operation={} error_variant={} retryable={}",
                failure.operation, failure.variant, failure.retryable
            );
        }
        let deadlocks = storage_causes
            .iter()
            .find_map(|(classification, observations)| {
                (*classification == "deadlock").then_some(*observations)
            })
            .unwrap_or(0);
        let integrity = bounded_integrity_oracle(
            INTEGRITY_ORACLE_LIMIT,
            verify_transition_integrity(&w, &apis, &started_runs, &completed_tasks, deadlocks),
        )
        .await;
        if let Err(failure) = integrity {
            let label = failure.label();
            body.push_str(&format!(
                "| {api_replicas} | {workers} | {transitions} (incomplete) | n/a | {poll_attempts} / {retryable_failures} / {exhausted_calls} | {:.0} / {:.0} / {:.0} ms | {:.0} / {:.0} / {:.0} ms | {:.0} / {:.0} / {:.0} ms | {:.2} s | {waiting_on} |\n",
                ms(percentile(&polls, 0.50)),
                ms(percentile(&polls, 0.95)),
                ms(percentile(&polls, 0.99)),
                ms(percentile(&logical_polls, 0.50)),
                ms(percentile(&logical_polls, 0.95)),
                ms(percentile(&logical_polls, 0.99)),
                ms(percentile(&completes, 0.50)),
                ms(percentile(&completes, 0.95)),
                ms(percentile(&completes, 0.99)),
                config_wall.as_secs_f64(),
            ));
            body.push_str(&format!(
                "\nConfiguration `{api_replicas}` API replicas / `{workers}` workers / `{runs}` runs is **incomplete** after all {transitions} acknowledged completions: the bounded integrity oracle reported fixed safe classification `{label}`. Poll counters and acknowledged latency samples are retained, but no throughput result, integrity verdict or load-level verdict is reported for this configuration. Earlier rows remain complete.\n"
            ));
            section(
                "Step transitions per second (BL02, BL04) — incomplete",
                &body,
            );
            panic!("transition integrity oracle failed: {label}");
        }
        let elapsed = finished - started;
        let per_second = transitions as f64 / elapsed.as_secs_f64().max(1e-9);
        ceiling = ceiling.max(per_second);
        body.push_str(&format!(
            "| {api_replicas} | {workers} | {transitions} | {per_second:.0} | {poll_attempts} / {retryable_failures} / {exhausted_calls} | {:.0} / {:.0} / {:.0} ms | {:.0} / {:.0} / {:.0} ms | {:.0} / {:.0} / {:.0} ms | {:.2} s | {waiting_on} |\n",
            ms(percentile(&polls, 0.50)),
            ms(percentile(&polls, 0.95)),
            ms(percentile(&polls, 0.99)),
            ms(percentile(&logical_polls, 0.50)),
            ms(percentile(&logical_polls, 0.95)),
            ms(percentile(&logical_polls, 0.99)),
            ms(percentile(&completes, 0.50)),
            ms(percentile(&completes, 0.95)),
            ms(percentile(&completes, 0.99)),
            config_wall.as_secs_f64(),
        ));
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

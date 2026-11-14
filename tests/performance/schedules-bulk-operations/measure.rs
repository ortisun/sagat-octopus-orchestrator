//! What every scenario of the benchmark shares: summaries (a median and a spread), the machine and its load, the
//! bound a scenario asserts and the fragment it leaves behind.
//!
//! A scenario states its bounds before it runs. When the data exceeds one, the scenario writes its fragment (the numbers
//! are the point) and then fails, naming the bound: a bound is never loosened to make a run green, and a run that fails is
//! reported as a failure (`tests/performance/reports/schedules-bulk-operations.md`).
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::io::Write as _;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use sqlx::PgPool;

/// The nearest-rank percentile (`p` in 0..=100) of values that are already sorted.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    assert!(!sorted.is_empty(), "no samples");
    let rank = ((p / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

/// What a set of samples looks like.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub min: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub mean: f64,
}

impl Summary {
    /// `None` when there is nothing to summarise.
    pub fn of(values: &[f64]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        Some(Self {
            n: sorted.len(),
            min: sorted[0],
            p50: percentile(&sorted, 50.0),
            p95: percentile(&sorted, 95.0),
            p99: percentile(&sorted, 99.0),
            max: sorted[sorted.len() - 1],
            mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
        })
    }

    /// The same numbers as JSON, in the unit of the samples.
    pub fn json(&self) -> Value {
        json!({
            "n": self.n, "min": self.min, "p50": self.p50, "p95": self.p95,
            "p99": self.p99, "max": self.max, "mean": self.mean,
        })
    }
}

/// The summary of `values` as JSON, or `null` for none.
pub fn summary_json(values: &[f64]) -> Value {
    Summary::of(values).map_or(Value::Null, |summary| summary.json())
}

/// Milliseconds of a duration.
pub fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

/// One bound a scenario asserts, with what was observed and whether it held.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub name: &'static str,
    pub bound: String,
    pub observed: String,
    pub held: bool,
}

impl Verdict {
    /// `observed <= limit`.
    pub fn at_most(name: &'static str, what: &str, observed: f64, limit: f64, unit: &str) -> Self {
        Self {
            name,
            bound: format!("{what} <= {limit} {unit}"),
            observed: format!("{observed:.3} {unit}"),
            held: observed <= limit,
        }
    }

    /// `observed >= limit`.
    pub fn at_least(name: &'static str, what: &str, observed: f64, limit: f64, unit: &str) -> Self {
        Self {
            name,
            bound: format!("{what} >= {limit} {unit}"),
            observed: format!("{observed:.3} {unit}"),
            held: observed >= limit,
        }
    }

    pub fn json(&self) -> Value {
        json!({"name": self.name, "bound": self.bound, "observed": self.observed, "held": self.held})
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64())
}

/// The load of the machine now: the one-minute load average and what `uptime` says, with the time it was read.
pub fn load_now() -> Value {
    json!({
        "at_unix": unix_seconds(),
        "uptime": run("uptime", &[]),
    })
}

/// The machine, the build and the database a fragment was measured on (declared once, repeated in every fragment so a
/// fragment can be read alone).
pub async fn environment(pool: &PgPool) -> Value {
    let settings: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, setting || coalesce(unit, '') FROM pg_settings \
         WHERE name IN ('shared_buffers', 'max_connections', 'work_mem', 'effective_cache_size', \
                        'synchronous_commit', 'fsync', 'full_page_writes', 'max_wal_size', 'wal_level', \
                        'random_page_cost', 'checkpoint_timeout', 'wal_buffers', 'max_worker_processes', \
                        'default_statistics_target', 'shared_preload_libraries') \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("read the settings");
    let version: String = sqlx::query_scalar("SELECT version()")
        .fetch_one(pool)
        .await
        .expect("read the version");
    let postgres: Map<String, Value> = settings
        .into_iter()
        .map(|(name, value)| (name, Value::String(value)))
        .collect();
    json!({
        "commit": run("git", &["rev-parse", "HEAD"]),
        "dirty": run("git", &["status", "--short"]).map(|status| !status.is_empty()),
        "cpus": run("sysctl", &["-n", "hw.ncpu"]).or_else(|| run("nproc", &[])),
        "memory_bytes": run("sysctl", &["-n", "hw.memsize"]),
        "os": run("uname", &["-srm"]),
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "postgres_version": version,
        "postgres_image": std::env::var("SAGAT_PERF_PG_IMAGE").ok(),
        "postgres_settings": postgres,
        "role": if crate::support::as_runtime_role() { "sagat_app (runtime role)" } else { "owner" },
        "docker_vm": run("docker", &["info", "--format", "{{.NCPU}} CPUs, {{.MemTotal}} bytes, {{.OperatingSystem}}, server {{.ServerVersion}}"]),
        "commit_latency_ms": commit_latency(pool).await,
    })
}

/// What one commit costs on this database, with `synchronous_commit` and `fsync` as set: a hundred inserts into a table of one
/// column, each its own transaction, one after the other. The cost of every transaction the scenarios measure includes one of these.
async fn commit_latency(pool: &PgPool) -> Value {
    sqlx::query("CREATE TABLE IF NOT EXISTS perf_commit_probe (n integer)")
        .execute(pool)
        .await
        .expect("create the probe table");
    let mut samples = Vec::new();
    for n in 0..100_i32 {
        let started = std::time::Instant::now();
        sqlx::query("INSERT INTO perf_commit_probe (n) VALUES ($1)")
            .bind(n)
            .execute(pool)
            .await
            .expect("insert a probe row");
        samples.push(ms(started.elapsed()));
    }
    summary_json(&samples)
}

/// Where fragments go: `SAGAT_PERF_FRAGMENTS` (a directory, one `<scenario>.jsonl` file each) or only the log.
fn fragment_file(scenario: &str) -> Option<std::path::PathBuf> {
    let directory = std::path::PathBuf::from(std::env::var_os("SAGAT_PERF_FRAGMENTS")?);
    std::fs::create_dir_all(&directory).expect("create the fragment directory");
    Some(directory.join(format!("{scenario}.jsonl")))
}

/// Writes the fragment of one run of a scenario and fails the run when a bound did not hold.
///
/// `fragment` carries the numbers; the verdicts, the load before and after and the environment are added here. The
/// fragment is written first, so that a run that failed still leaves its numbers.
pub fn conclude(
    scenario: &str,
    environment: Value,
    before: Value,
    fragment: Value,
    verdicts: &[Verdict],
) {
    let record = json!({
        "scenario": scenario,
        "environment": environment,
        "load_before": before,
        "load_after": load_now(),
        "numbers": fragment,
        "verdicts": verdicts.iter().map(Verdict::json).collect::<Vec<_>>(),
    });
    let line = serde_json::to_string(&record).expect("serialise the fragment");
    println!("FRAGMENT {line}");
    if let Some(path) = fragment_file(scenario) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open the fragment file");
        writeln!(file, "{line}").expect("write the fragment");
    }
    for verdict in verdicts {
        println!(
            "{} {}: {} (observed {})",
            if verdict.held { "HELD  " } else { "BROKEN" },
            verdict.name,
            verdict.bound,
            verdict.observed
        );
    }
    let broken: Vec<String> = verdicts
        .iter()
        .filter(|verdict| !verdict.held)
        .map(|verdict| {
            format!(
                "{}: {} (observed {})",
                verdict.name, verdict.bound, verdict.observed
            )
        })
        .collect();
    assert!(
        broken.is_empty(),
        "{scenario}: the data exceeds a declared bound:\n  {}",
        broken.join("\n  ")
    );
}

/// What a scenario started with settings other than the engine's defaults adds to its name, so that its fragments are not taken
/// for the default ones: `-poll50` for a batch loop that polls every 50 ms, `-round1000` and `-batch500` for a schedule pass with
/// those ceilings. Empty for the defaults.
pub fn variant() -> String {
    let mut name = String::new();
    for (variable, tag) in [
        ("SAGAT_PERF_BATCH_POLL_MS", "poll"),
        ("SAGAT_PERF_SCHEDULES_ROUND_ACTIONS", "round"),
        ("SAGAT_PERF_SCHEDULES_BATCH", "batch"),
    ] {
        if let Ok(value) = std::env::var(variable) {
            name.push_str(&format!("-{tag}{value}"));
        }
    }
    name
}

/// A size from the environment: a list like `1000,10000`, or `default` when the variable is not set. The defaults are
/// small on purpose: an ignored scenario that is started by hand without the sizes of the report is a smoke run, not a
/// half-hour benchmark.
pub fn sizes(variable: &str, default: &[usize]) -> Vec<usize> {
    std::env::var(variable).map_or_else(
        |_| default.to_vec(),
        |value| {
            value
                .split(',')
                .map(|part| part.trim().parse().expect("a size is a whole number"))
                .collect()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{Summary, Verdict, percentile, sizes};

    #[test]
    fn a_percentile_is_the_nearest_rank_of_sorted_samples() {
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&values, 50.0), 50.0);
        assert_eq!(percentile(&values, 95.0), 95.0);
        assert_eq!(percentile(&values, 99.0), 99.0);
        assert_eq!(percentile(&values, 100.0), 100.0);
        assert_eq!(percentile(&values, 0.0), 1.0);
        assert_eq!(percentile(&[7.0], 99.0), 7.0);
    }

    #[test]
    fn a_summary_does_not_depend_on_the_order_of_the_samples() {
        let forward = Summary::of(&[3.0, 1.0, 2.0, 10.0, 4.0]).expect("summary");
        let backward = Summary::of(&[4.0, 10.0, 2.0, 1.0, 3.0]).expect("summary");
        assert_eq!(forward, backward);
        assert_eq!((forward.min, forward.p50, forward.max), (1.0, 3.0, 10.0));
        assert!(Summary::of(&[]).is_none());
    }

    #[test]
    fn a_bound_that_the_data_exceeds_is_broken_and_the_limit_itself_holds() {
        assert!(Verdict::at_most("a", "x", 60.0, 60.0, "s").held);
        assert!(!Verdict::at_most("a", "x", 60.001, 60.0, "s").held);
        assert!(Verdict::at_least("a", "x", 16.0, 16.0, "/s").held);
        assert!(!Verdict::at_least("a", "x", 15.9, 16.0, "/s").held);
    }

    #[test]
    fn a_size_falls_back_to_the_small_default_when_the_variable_is_not_set() {
        assert_eq!(
            sizes("SAGAT_PERF_THIS_VARIABLE_IS_NOT_SET", &[5, 7]),
            [5, 7]
        );
    }
}

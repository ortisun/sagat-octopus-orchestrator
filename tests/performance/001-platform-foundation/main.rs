//! Baseline benchmark of feature 001 (T001-033): latency of namespace reads.
//!
//! Plan target: p95 < 250 ms for a namespace read (`NFR-001-PERF`). The numbers are only meaningful
//! together with the environment, which the report records. Run on purpose:
//!
//! ```sh
//! SAGAT_PERF_REPORT=tests/performance/reports/001-platform-foundation.md \
//!   cargo test -p sagat-tests --test perf_001 --release -- --ignored --nocapture
//! ```
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

#[path = "../../support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use sagat_api_http::{HttpLimits, RateLimitConfig};
use support::api::{Options, start_with};

const NAMESPACES: usize = 1_000;
const CONCURRENCY: usize = 32;
const WARMUP: Duration = Duration::from_secs(2);
const MEASURE: Duration = Duration::from_secs(10);
const P95_BUDGET: Duration = Duration::from_millis(250);

struct Run {
    name: &'static str,
    requests: usize,
    errors: usize,
    elapsed: Duration,
    latencies: Vec<Duration>,
}

impl Run {
    fn percentile(&self, p: f64) -> Duration {
        let idx = ((self.latencies.len() as f64 * p).ceil() as usize).saturating_sub(1);
        self.latencies[idx.min(self.latencies.len() - 1)]
    }
    fn throughput(&self) -> f64 {
        self.requests as f64 / self.elapsed.as_secs_f64()
    }
}

async fn drive(
    name: &'static str,
    client: Arc<reqwest::Client>,
    urls: Arc<Vec<String>>,
    token: Arc<String>,
    duration: Duration,
) -> Run {
    let started = Instant::now();
    let mut tasks = Vec::new();
    for worker in 0..CONCURRENCY {
        let (client, urls, token) = (Arc::clone(&client), Arc::clone(&urls), Arc::clone(&token));
        tasks.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            let mut errors = 0usize;
            let mut i = worker;
            while started.elapsed() < duration {
                let url = &urls[i % urls.len()];
                i += CONCURRENCY;
                let t = Instant::now();
                let ok = matches!(client.get(url).bearer_auth(token.as_str()).send().await, Ok(r) if r.status() == 200);
                latencies.push(t.elapsed());
                if !ok {
                    errors += 1;
                }
            }
            (latencies, errors)
        }));
    }
    let (mut latencies, mut errors) = (Vec::new(), 0);
    for t in tasks {
        let (l, e) = t.await.expect("worker");
        latencies.extend(l);
        errors += e;
    }
    latencies.sort_unstable();
    Run {
        name,
        requests: latencies.len(),
        errors,
        elapsed: started.elapsed(),
        latencies,
    }
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
    let mem = if cfg!(target_os = "macos") {
        sh("sysctl", &["-n", "hw.memsize"])
            .parse::<u64>()
            .map_or_else(|_| "?".into(), |b| format!("{} GiB", b / (1 << 30)))
    } else {
        sh(
            "sh",
            &[
                "-c",
                "awk '/MemTotal/ {printf \"%d GiB\", $2/1048576}' /proc/meminfo",
            ],
        )
    };
    let commit = sh("git", &["rev-parse", "--short", "HEAD"]);
    let profile = if cfg!(debug_assertions) {
        "debug (unoptimised)"
    } else {
        "release"
    };
    format!(
        "- CPU: {cpu} ({cores} logical cores)\n- Memory: {mem}\n- OS: {}\n- Build profile of the test process and API: {profile}\n- Commit: {commit}\n- PostgreSQL 16 in a Docker container on the same machine (Docker Desktop VM on macOS)\n- API, client and database share this machine, so the numbers exclude any real network",
        sh("uname", &["-srm"])
    )
}

#[tokio::test]
#[ignore = "benchmark: run on purpose with --ignored"]
async fn namespace_read_latency_meets_the_baseline_budget() {
    let api = start_with(Options {
        rate_limit: RateLimitConfig {
            per_second: 1e9,
            burst: 1e9,
        },
        limits: HttpLimits {
            max_in_flight: 10_000,
            ..HttpLimits::default()
        },
        ..Options::default()
    })
    .await;
    api.tenant("acme", "alice").await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    // Bulk-load the data set directly; creating 1000 namespaces through the API is not what is measured.
    sqlx::query(
        "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
         SELECT gen_random_uuid(), w.tenant_id, w.id, 'ns-' || g, 'dev', now() FROM workspaces w, generate_series(1, $1) g WHERE w.id = $2::uuid",
    )
    .bind(i32::try_from(NAMESPACES).expect("fits"))
    .bind(&ws)
    .execute(&api.db.pool)
    .await
    .expect("bulk insert");
    let ids: Vec<sqlx::types::Uuid> = sqlx::query_scalar("SELECT id FROM namespaces ORDER BY id")
        .fetch_all(&api.db.pool)
        .await
        .expect("ids");
    assert_eq!(ids.len(), NAMESPACES);

    let client = Arc::new(api.client.clone());
    let token = Arc::new(api.idp.token("alice", "acme").issued(0, 3000).build());
    let reads: Arc<Vec<String>> = Arc::new(
        ids.iter()
            .map(|id| format!("{}/namespaces/{id}", api.base))
            .collect(),
    );
    let lists: Arc<Vec<String>> = Arc::new(vec![format!("{}/namespaces?limit=50", api.base)]);

    drive(
        "namespace read (GET /namespaces/{id})",
        Arc::clone(&client),
        Arc::clone(&reads),
        Arc::clone(&token),
        WARMUP,
    )
    .await;
    let read = drive(
        "namespace read (GET /namespaces/{id})",
        Arc::clone(&client),
        reads,
        Arc::clone(&token),
        MEASURE,
    )
    .await;
    let list = drive(
        "namespace list (GET /namespaces?limit=50)",
        client,
        lists,
        token,
        MEASURE,
    )
    .await;

    let mut report = format!(
        "# Baseline benchmark: platform foundation (feature 001)\n\nMeasured by `tests/performance/001-platform-foundation/main.rs`. Hypotheses, not promises: figures are valid only for this environment.\n\n## Environment\n\n{}\n\n## Workload\n\n- {NAMESPACES} namespaces in one tenant; authenticated requests (JWT verified on every request) with {CONCURRENCY} concurrent clients\n- {} s warm-up, then {} s measured per scenario\n- Rate limit and in-flight cap raised so they do not mask the system; database pool of 8 connections\n\n## Results\n\n| Scenario | Requests | Errors | Throughput | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|---:|---:|---:|\n",
        environment(),
        WARMUP.as_secs(),
        MEASURE.as_secs()
    );
    for run in [&read, &list] {
        report.push_str(&format!(
            "| {} | {} | {} | {:.0} req/s | {:.1} ms | {:.1} ms | {:.1} ms | {:.1} ms |\n",
            run.name,
            run.requests,
            run.errors,
            run.throughput(),
            run.percentile(0.50).as_secs_f64() * 1e3,
            run.percentile(0.95).as_secs_f64() * 1e3,
            run.percentile(0.99).as_secs_f64() * 1e3,
            run.latencies.last().expect("some").as_secs_f64() * 1e3
        ));
    }
    report.push_str(&format!(
        "\nTarget (plan): namespace read p95 < {} ms. Result: **{}**.\n",
        P95_BUDGET.as_millis(),
        if read.percentile(0.95) < P95_BUDGET {
            "met"
        } else {
            "NOT met"
        }
    ));
    println!("{report}");
    if let Ok(path) = std::env::var("SAGAT_PERF_REPORT") {
        // Relative paths are relative to the repository root, not to this package.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let target = if std::path::Path::new(&path).is_absolute() {
            path.into()
        } else {
            root.join(path)
        };
        std::fs::write(&target, &report).expect("write the report");
    }

    assert_eq!(
        read.errors + list.errors,
        0,
        "no request may fail during the benchmark"
    );
    assert!(
        read.percentile(0.95) < P95_BUDGET,
        "p95 {:?} exceeds the {:?} budget",
        read.percentile(0.95),
        P95_BUDGET
    );
}

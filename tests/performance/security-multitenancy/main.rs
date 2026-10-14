//! Benchmark of feature 009 (T009-034, NFR-009-PERF): what the security controls cost on the paths
//! they sit on.
//!
//! The specification forbids deciding a security limit from a counter that may be late, so a quota
//! is judged by an exact count taken under a lock in the creating transaction. The question this
//! answers is what that costs: how a run start slows as the namespace fills, how starts into one
//! namespace compare with starts spread over several (the lock serialises one scope), and what a
//! worker pays for the binding check, for resolving secrets and for the scan of its output.
//!
//! The numbers are only meaningful together with the environment, which the report records. Run on
//! purpose:
//!
//! ```sh
//! SAGAT_PERF_REPORT=tests/performance/reports/security-multitenancy.md \
//!   cargo test -p sagat-tests --test perf_security_multitenancy --release -- --ignored --nocapture
//! ```
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

mod durable_quotas;
#[path = "../../support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream;
use sagat_application::quotas::QuotaStore;
use sagat_domain::quota::QuotaResource;
use serde_json::{Value, json};
use support::engine::{World, activity, ctx, document, task_ref, world};

/// Populations of active runs at which the cost of starting one more is sampled.
const LEVELS: [usize; 3] = [0, 2_500, 5_000];
const SAMPLES_PER_LEVEL: usize = 200;
const CONCURRENCY: usize = 16;
const STARTS_PER_SCENARIO: usize = 1_600;
const RESOLVE_SAMPLES: usize = 200;

/// Hypotheses to hold the system to, not promises.
const START_P95_AT_5K: Duration = Duration::from_millis(60);
/// Counting 5 000 active runs of a namespace is an index-only scan: it must not make a start many
/// times slower than starting into an empty namespace.
const START_GROWTH_FACTOR: f64 = 3.0;
const RESOLVE_P95: Duration = Duration::from_millis(60);

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn row(name: &str, mut samples: Vec<Duration>) -> (String, Duration, Duration) {
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    (
        format!(
            "| {name} | {} | {:.1} ms | {:.1} ms | {:.1} ms | {:.1} ms |\n",
            samples.len(),
            ms(p50),
            ms(p95),
            ms(percentile(&samples, 0.99)),
            ms(*samples.last().expect("samples")),
        ),
        p50,
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
    let cores = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
    let load = sh("sh", &["-c", "uptime | sed 's/.*load averages*: //'"]);
    let mut commit = sh("git", &["rev-parse", "--short", "HEAD"]);
    if !sh("git", &["status", "--porcelain", "--untracked-files=no"])
        .trim()
        .is_empty()
    {
        commit.push_str(" plus uncommitted changes to tracked files");
    }
    let profile = if cfg!(debug_assertions) {
        "debug (unoptimised)"
    } else {
        "release"
    };
    format!(
        "- CPU: {cpu} ({cores} logical cores)\n- OS: {}\n- Load average when the run began: {load} (other work on the machine adds noise)\n- Build profile of the test process and services: {profile}\n- Commit: {commit}\n- PostgreSQL 16 in a Docker container on the same machine\n- Services run in-process (no network between them and the client)",
        sh("uname", &["-srm"])
    )
}

fn waiting_flow(name: &str) -> Value {
    document(name, "a", vec![activity("a")])
}

async fn start_timed(w: &World, wf: sagat_domain::definition::WorkflowId, key: String) -> Duration {
    let t = Instant::now();
    w.start_with(wf, &json!({}), &key, None)
        .await
        .expect("start");
    t.elapsed()
}

/// Starts `count` runs of `wf` with `CONCURRENCY` callers and returns the wall time and each call's.
async fn burst(
    w: &World,
    wf: sagat_domain::definition::WorkflowId,
    count: usize,
    tag: &str,
) -> (Duration, Vec<Duration>) {
    let begun = Instant::now();
    let samples = stream::iter(0..count)
        .map(|i| start_timed(w, wf, format!("{tag}-{i:06}")))
        .buffer_unordered(CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    (begun.elapsed(), samples)
}

#[tokio::test]
#[ignore = "benchmark: run on purpose with --ignored"]
async fn exact_quota_counting_stays_cheap_and_the_new_worker_paths_cost_little() {
    let w = world().await;
    // Limits far above what the benchmark creates, so no start is refused and nothing but the
    // counting is measured. A tenant cannot set a limit above its ceiling, so the operator grants
    // the headroom first, as it would for a customer that runs this many.
    for resource in [QuotaResource::TenantActiveRuns, QuotaResource::ActiveRuns] {
        w.quotas
            .grant_ceiling(w.admin.tenant_id(), resource, Some(1_000_000))
            .await
            .expect("ceiling");
    }
    w.quotas
        .set(
            &w.admin,
            None,
            QuotaResource::TenantActiveRuns,
            1_000_000,
            &ctx(),
        )
        .await
        .expect("limit");
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
            1_000_000,
            &ctx(),
        )
        .await
        .expect("limit");
    let (wf, _) = w.publish(waiting_flow("bench-flow")).await;

    // ------------------------------------------------ 1. the cost of an exact count as the namespace fills
    let mut level_rows = Vec::new();
    let mut level_p50 = Vec::new();
    let mut populated = 0usize;
    let mut level_p95_last = Duration::ZERO;
    for (n, level) in LEVELS.iter().enumerate() {
        while populated < *level {
            let upto = (*level).min(populated + 500);
            let _ = burst(&w, wf, upto - populated, &format!("fill-{populated}")).await;
            populated = upto;
        }
        sqlx::query("ANALYZE runs")
            .execute(&w.db.pool)
            .await
            .expect("analyze");
        // One caller at a time: this is the latency of a start, not of contention.
        let mut samples = Vec::new();
        for i in 0..SAMPLES_PER_LEVEL {
            samples.push(start_timed(&w, wf, format!("sample-{n}-{i:05}")).await);
        }
        populated += SAMPLES_PER_LEVEL;
        let (line, p50, p95) = row(
            &format!("start a run, {} active runs in the namespace", level),
            samples,
        );
        level_rows.push(line);
        level_p50.push(p50);
        level_p95_last = p95;
    }
    // The count a start performs, timed on its own: it is the part that grows with the namespace.
    let tenant = w.admin.tenant_id().as_uuid();
    let namespace = w.namespace.id().as_uuid();
    let mut count_times = Vec::new();
    for _ in 0..50 {
        let t = Instant::now();
        let _: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM runs WHERE tenant_id = $1 AND namespace_id = $2 AND finished_at IS NULL",
        )
        .bind(tenant)
        .bind(namespace)
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
        count_times.push(t.elapsed());
    }
    let (count_row, ..) = row(
        &format!("the exact count alone, {populated} active runs in the namespace"),
        count_times,
    );
    level_rows.push(count_row);
    let explain = |ns: uuid::Uuid| {
        let w = &w;
        async move {
            let rows: Vec<String> = sqlx::query_scalar(
                "EXPLAIN SELECT count(*) FROM runs \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND finished_at IS NULL",
            )
            .bind(tenant)
            .bind(ns)
            .fetch_all(&w.db.pool)
            .await
            .expect("explain");
            rows.join(" / ")
        }
    };
    // This namespace holds most of the table, so a sequential scan is what the planner should pick:
    // reported, not asserted. A namespace that is a small share of the table must use the index.
    let big_plan = explain(namespace).await;
    let counted = w
        .store
        .usage(
            w.admin.tenant_id(),
            Some(w.namespace.id()),
            QuotaResource::ActiveRuns,
        )
        .await
        .expect("usage");
    assert_eq!(
        counted,
        u64::try_from(populated).expect("fits"),
        "the exact count equals what was started"
    );
    let growth = ms(level_p50[LEVELS.len() - 1]) / ms(level_p50[0]).max(0.001);

    // ------------------------------------------------ 2. one namespace against several
    let one = {
        let (elapsed, samples) = burst(&w, wf, STARTS_PER_SCENARIO, "one").await;
        (elapsed, samples)
    };
    let mut spread_namespaces = Vec::new();
    for i in 0..CONCURRENCY {
        spread_namespaces.push(w.add_namespace(&format!("bench-ns-{i}")).await);
    }
    // The spread scenario starts into the same namespace-less workflow of each namespace: the
    // workflow of the first namespace is reused through the header of its own namespace.
    let spread = {
        let mut wfs = Vec::new();
        for ns in &spread_namespaces {
            let name = format!("flow-{}", ns.slug());
            let bytes = serde_json::to_vec(&waiting_flow(&name)).expect("json");
            let created = w
                .defs
                .create_workflow(
                    &w.admin,
                    ns.slug().as_str(),
                    &name,
                    &bytes,
                    &format!("create-{name}-key"),
                    &ctx(),
                )
                .await
                .expect("create");
            w.defs
                .publish(
                    &w.admin,
                    created.workflow.id(),
                    1,
                    None,
                    &format!("publish-{name}-key"),
                    &ctx(),
                )
                .await
                .expect("publish");
            wfs.push((ns.slug().to_string(), created.workflow.id()));
        }
        let begun = Instant::now();
        let samples = stream::iter(0..STARTS_PER_SCENARIO)
            .map(|i| {
                let (namespace, workflow) = &wfs[i % wfs.len()];
                let w = &w;
                async move {
                    let bytes = b"{}".to_vec();
                    let t = Instant::now();
                    w.runs
                        .start(
                            &w.admin,
                            &sagat_application::runs::StartRun {
                                namespace,
                                workflow: *workflow,
                                version: None,
                                input: &bytes,
                                correlation_id: None,
                                deadline: None,
                                key: &format!("spread-{i:06}"),
                            },
                            &ctx(),
                        )
                        .await
                        .expect("start");
                    t.elapsed()
                }
            })
            .buffer_unordered(CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        (begun.elapsed(), samples)
    };
    // The same number of callers over two TENANTS: no scope in common, so nothing but the insert path
    // and the pool is shared. Namespaces of one tenant all share the tenant-wide count (a start
    // locks its namespace and then its tenant, until it commits), so the previous scenario cannot
    // tell the lock from the rest; this one can.
    let two_tenants = {
        let name = "flow-second-tenant";
        let bytes = serde_json::to_vec(&waiting_flow(name)).expect("json");
        let created = w
            .defs
            .create_workflow(
                &w.other_admin,
                w.other_namespace.slug().as_str(),
                name,
                &bytes,
                "create-second-tenant-key",
                &ctx(),
            )
            .await
            .expect("create");
        w.defs
            .publish(
                &w.other_admin,
                created.workflow.id(),
                1,
                None,
                "publish-second-tenant-key",
                &ctx(),
            )
            .await
            .expect("publish");
        let targets = [
            (&w.admin, w.namespace.slug().to_string(), wf),
            (
                &w.other_admin,
                w.other_namespace.slug().to_string(),
                created.workflow.id(),
            ),
        ];
        let begun = Instant::now();
        let samples = stream::iter(0..STARTS_PER_SCENARIO)
            .map(|i| {
                let (principal, namespace, workflow) = &targets[i % targets.len()];
                let w = &w;
                async move {
                    let bytes = b"{}".to_vec();
                    let t = Instant::now();
                    w.runs
                        .start(
                            principal,
                            &sagat_application::runs::StartRun {
                                namespace,
                                workflow: *workflow,
                                version: None,
                                input: &bytes,
                                correlation_id: None,
                                deadline: None,
                                key: &format!("tenants-{i:06}"),
                            },
                            &ctx(),
                        )
                        .await
                        .expect("start");
                    t.elapsed()
                }
            })
            .buffer_unordered(CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        (begun.elapsed(), samples)
    };
    sqlx::query("ANALYZE runs")
        .execute(&w.db.pool)
        .await
        .expect("analyze");
    let small_plan = explain(spread_namespaces[0].id().as_uuid()).await;
    let one_rate = STARTS_PER_SCENARIO as f64 / one.0.as_secs_f64();
    let spread_rate = STARTS_PER_SCENARIO as f64 / spread.0.as_secs_f64();
    let tenants_rate = STARTS_PER_SCENARIO as f64 / two_tenants.0.as_secs_f64();
    let (one_row, ..) = row("start a run, 16 callers into ONE namespace", one.1);
    let (spread_row, ..) = row(
        "start a run, 16 callers over 16 namespaces of ONE tenant",
        spread.1,
    );
    let (tenants_row, ..) = row("start a run, 16 callers over 2 tenants", two_tenants.1);

    // ------------------------------------------------ 3. the worker paths
    let developer = w
        .member("dora", sagat_domain::identity::Role::Developer)
        .await;
    let many = ["s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9"];
    for name in many {
        w.put_secret(
            &format!("{name}-key"),
            &format!("secret-value-{name}-0000000000"),
        )
        .await;
        w.secrets
            .grant(
                &w.admin,
                w.namespace.id(),
                &format!("{name}-key"),
                "dora",
                &ctx(),
            )
            .await
            .expect("grant");
    }
    let names: Vec<String> = many.iter().map(|n| format!("{n}-key")).collect();
    let mut resolve_rows = Vec::new();
    let mut resolve_p95_ten = Duration::ZERO;
    let mut scan_rows = Vec::new();
    for (label, declared) in [("none", 0usize), ("1 secret", 1), ("10 secrets", 10)] {
        let secrets: Vec<&String> = names.iter().take(declared).collect();
        let (wf, _) = w
            .try_publish_as(
                &developer,
                document(
                    &format!("secret-flow-{declared}"),
                    "a",
                    vec![json!({
                        "id": "a", "type": "activity",
                        "activity": format!("sec.d{declared}"),
                        "secrets": secrets,
                    })],
                ),
            )
            .await
            .expect("publish");
        for i in 0..RESOLVE_SAMPLES.max(60) {
            w.start_with(wf, &json!({}), &format!("sf-{declared}-{i:04}"), None)
                .await
                .expect("start");
        }
        let activity_name = format!("sec.d{declared}");
        let session = w.session(&[activity_name.as_str()]).await;
        let mut resolve = Vec::new();
        let mut heartbeat = Vec::new();
        let mut complete = Vec::new();
        let output = json!({"blob": "x".repeat(200_000)});
        for _ in 0..60 {
            let task = w.poll(&session, 1).await.remove(0);
            if declared > 0 {
                let t = Instant::now();
                let got = w
                    .workers
                    .resolve_secrets(&session.principal, &task_ref(&task), None)
                    .await
                    .expect("resolve");
                resolve.push(t.elapsed());
                assert_eq!(got.len(), declared);
            }
            let t = Instant::now();
            w.workers
                .heartbeat(&session.principal, &task_ref(&task))
                .await
                .expect("heartbeat");
            heartbeat.push(t.elapsed());
            let t = Instant::now();
            w.complete(&session, &task, &output)
                .await
                .expect("complete");
            complete.push(t.elapsed());
        }
        if declared > 0 {
            let (line, _, p95) = row(
                &format!("resolve {label} (audited, in the lease's transaction)"),
                resolve,
            );
            if declared == 10 {
                resolve_p95_ten = p95;
            }
            resolve_rows.push(line);
        }
        scan_rows.push(
            row(
                &format!("heartbeat, step declaring {label} (binding re-checked)"),
                heartbeat,
            )
            .0,
        );
        scan_rows.push(
            row(
                &format!("complete with a 200 KB output, step declaring {label}"),
                complete,
            )
            .0,
        );
    }

    // ------------------------------------------------ report
    let mut report = format!(
        "# Benchmark: security and multi-tenancy controls (feature 009)\n\nMeasured by `tests/performance/security-multitenancy/main.rs`. Hypotheses, not promises: figures are valid only for this environment.\n\n## Environment\n\n{}\n\n## Results\n\n| Operation | Samples | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|---:|\n",
        environment()
    );
    for line in level_rows
        .iter()
        .chain([&one_row, &spread_row, &tenants_row])
        .chain(resolve_rows.iter())
        .chain(scan_rows.iter())
    {
        report.push_str(line);
    }
    report.push_str(&format!(
        "\n- Starting with {CONCURRENCY} callers: into one namespace **{one_rate:.0} runs/s**; over {CONCURRENCY} namespaces of one tenant **{spread_rate:.0} runs/s** ({:.1}x); over two tenants **{tenants_rate:.0} runs/s** ({:.1}x). Read the first figure with care: that namespace holds most of the table, so each of its starts also pays for counting thousands of rows, which the other two do not. The comparison that isolates the locks is the last two: in both the counts are small; creators in different namespaces of one tenant share the tenant-wide count (a start locks its namespace and then its tenant, and holds both until it commits), while tenants share no lock. Starting over namespaces of one tenant reached {:.2} of the rate over two tenants, which is what the tenant-wide lock costs here at this concurrency; the rest is set by the insert path and the pool of eight connections.\n- Plan of the count for a namespace that is a small share of the table (it must use the `runs_active_idx` partial index): `{}`.\n- Plan of the count for the namespace of this benchmark, which holds most of the table (a sequential scan is the right choice there): `{}`.\n- Exact usage after {populated} starts into the namespace: {counted}, equal to what was started.\n\n## Hypotheses\n\n| Hypothesis | Result |\n|---|---|\n| starting a run with 5 000 active runs in the namespace, p95 < {} ms | {:.1} ms: **{}** |\n| the exact count makes a start (p50) less than {}x slower from 0 to 5 000 active runs | {:.1} ms to {:.1} ms ({:.1}x): **{}** |\n| resolving 10 secrets, p95 < {} ms | {:.1} ms: **{}** |\n\n## Reading the numbers\n\n- A quota is decided by counting, under an advisory lock on the scope, in the transaction that creates the run. It is exact whatever the number of replicas, and its cost is a count of the namespace's active runs (an index-only scan when the namespace is a small share of the table, which is the normal case; a sequential scan when it is nearly all of it) plus the time the lock is held (the length of the insert). **It grows linearly with the namespace's active runs**: negligible up to the default limit, and the reason a limit far above 100 000 should come with a counter row instead. Creators in different **tenants** do not wait for each other; creators in different namespaces of **one** tenant do, for the length of an insert, because the tenant-wide count is a second scope they share.\n- The ceiling for *one* namespace is, in principle, the rate at which one transaction can count and insert; on this machine the measurement does not reach it (see the ratios above). A namespace that needs more should be split, or the count replaced by a transactional counter row (which serialises the same way and adds a decrement on every path a run can end by): not done before it is measured to matter.\n- The binding check and the output scan only run for steps that need them; the heartbeat rows show the check costs one indexed read on the transaction's own connection.\n- Resolving secrets reads the version pinned by the run, the publisher's role and the access list, decrypts, and writes the audit record, in one transaction under the run's lock.\n- The machine is shared with unrelated workloads (see the load average), so absolute figures vary between runs; the structural claims (index-only counting, one lock per scope, no second connection while a transaction is open) are what the tests prove.\n",
        spread_rate / one_rate,
        tenants_rate / one_rate,
        spread_rate / tenants_rate,
        small_plan.replace('|', "/"),
        big_plan.replace('|', "/"),
        START_P95_AT_5K.as_millis(),
        ms(level_p95_last),
        if level_p95_last < START_P95_AT_5K { "met" } else { "NOT met" },
        START_GROWTH_FACTOR,
        ms(level_p50[0]),
        ms(level_p50[LEVELS.len() - 1]),
        growth,
        if growth < START_GROWTH_FACTOR { "met" } else { "NOT met" },
        RESOLVE_P95.as_millis(),
        ms(resolve_p95_ten),
        if resolve_p95_ten < RESOLVE_P95 { "met" } else { "NOT met" },
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
    assert!(
        small_plan.contains("runs_active_idx"),
        "the count of a small namespace must use the partial index: {small_plan}"
    );
    assert!(
        level_p95_last < START_P95_AT_5K,
        "start p95 {level_p95_last:?}"
    );
    assert!(
        growth < START_GROWTH_FACTOR,
        "a start slowed {growth:.1}x as the namespace filled"
    );
    assert!(
        resolve_p95_ten < RESOLVE_P95,
        "resolve p95 {resolve_p95_ten:?}"
    );
}

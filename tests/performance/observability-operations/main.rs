//! Benchmark of feature 008 (T008-033, NFR-008-PERF): what observability costs the paths it watches.
//!
//! The specification says projection must not block the write path and that its lag is budgeted and
//! flagged. The design (ADR-0017) answers with a note per run written in the transaction that
//! changes the journal and a projector that works from the notes, off that path. This asks four
//! questions of it:
//!
//! 1. what the note costs a writer, and whether a projector that is busy slows the writers;
//! 2. how fast a backlog is projected, and how far behind the projection falls under sustained
//!    writing (the figure the lag budget has to be set against);
//! 3. what a monitor round costs as the platform fills (it looks at the whole platform with
//!    aggregate queries);
//! 4. what the new reads cost: the telemetry state and attempts of a run, the audit trail, the alerts.
//!
//! The spans go to a fake collector in the same process, which parses them: the figures include that
//! work, which a real collector would do elsewhere. The numbers are only meaningful together with the
//! environment, which the report records. Run on purpose:
//!
//! ```sh
//! SAGAT_PERF_REPORT=tests/performance/reports/observability-operations.md \
//!   cargo test -p sagat-tests --test perf_observability_operations --release -- --ignored --nocapture
//! ```
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

#[path = "../../support/mod.rs"]
mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream;
use sagat_application::operations::{AlertReader, AuditFilter, AuditStore, MonitorStore};
use sagat_application::telemetry::TelemetryStore;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::quota::QuotaResource;
use sagat_scheduler::{Monitor, MonitorSettings, ProjectorSettings, TelemetryProjector};
use serde_json::json;
use support::collector::Collector;
use support::engine::{World, activity, chain, ctx, document, world};

/// Writers that measure the write path one at a time.
const WRITE_SAMPLES: usize = 300;
/// Runs left unprojected to time how fast a backlog is taken.
const BACKLOG_RUNS: usize = 600;
/// Callers writing at once while the lag is watched, and for how long.
const WRITERS: usize = 8;
const SUSTAINED: Duration = Duration::from_secs(10);
/// Live runs at which a monitor round is timed.
const LEVELS: [usize; 3] = [0, 2_500, 5_000];
const ROUNDS_PER_LEVEL: usize = 20;
const READ_SAMPLES: usize = 200;

/// Hypotheses to hold the system to, not promises.
/// A busy projector slows the writer's median by less than this factor.
const WRITER_SLOWDOWN: f64 = 1.5;
/// The note, on its own, is a small part of the median write.
const NOTE_SHARE: f64 = 0.20;
/// A backlog is taken at no less than this many runs a second.
const PROJECT_RATE: f64 = 50.0;
/// Under sustained writing the oldest unprojected change stays under the default lag budget (30 s).
const LAG_BUDGET: Duration = Duration::from_secs(30);
/// A monitor round at 5 000 live runs.
const MONITOR_P95: Duration = Duration::from_millis(500);
const READ_P95: Duration = Duration::from_millis(100);

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// Milliseconds with the precision the figure deserves: a hundredth below a millisecond.
fn shown(d: Duration) -> String {
    if ms(d) < 1.0 {
        format!("{:.2} ms", ms(d))
    } else {
        format!("{:.1} ms", ms(d))
    }
}

struct Row {
    line: String,
    p50: Duration,
    p95: Duration,
}

fn row(name: &str, mut samples: Vec<Duration>) -> Row {
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    Row {
        line: format!(
            "| {name} | {} | {} | {} | {} | {} |
",
            samples.len(),
            shown(p50),
            shown(p95),
            shown(percentile(&samples, 0.99)),
            shown(*samples.last().expect("samples")),
        ),
        p50,
        p95,
    }
}

fn verdict(met: bool) -> &'static str {
    if met { "met" } else { "NOT met" }
}

fn sh(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// What the database under the benchmark is, asked of it: the figures depend on it, above all on
/// whether commits wait for the disk.
async fn database(pool: &sqlx::PgPool) -> String {
    let version: String = sqlx::query_scalar("SHOW server_version")
        .fetch_one(pool)
        .await
        .expect("version");
    let fsync: String = sqlx::query_scalar("SHOW fsync")
        .fetch_one(pool)
        .await
        .expect("fsync");
    let synchronous: String = sqlx::query_scalar("SHOW synchronous_commit")
        .fetch_one(pool)
        .await
        .expect("synchronous_commit");
    let sync_method: String = sqlx::query_scalar("SHOW wal_sync_method")
        .fetch_one(pool)
        .await
        .expect("wal_sync_method");
    let origin = if std::env::var_os("SAGAT_TEST_DATABASE_URL").is_some() {
        "a server named by SAGAT_TEST_DATABASE_URL on the same machine"
    } else {
        "a Docker container on the same machine"
    };
    format!(
        "- PostgreSQL {version} in {origin}; `fsync = {fsync}`, `synchronous_commit = {synchronous}`, `wal_sync_method = {sync_method}` (with either of the first two off a commit does not wait for the disk; on macOS the default method does not ask the drive to flush its own cache, so commits here are faster than on the storage of a server: read every write figure below as optimistic)"
    )
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
        "- CPU: {cpu} ({cores} logical cores)\n- OS: {}\n- Load average when the run began: {load} (other work on the machine adds noise)\n- Build profile of the test process and services: {profile}\n- Commit: {commit}\n{{DATABASE}}\n- Services, projector and a fake collector run in-process (no network between them and the client)",
        sh("uname", &["-srm"])
    )
}

fn projector(
    w: &World,
    collector: &Collector,
) -> TelemetryProjector<sagat_infra_postgres::PgRunStore> {
    TelemetryProjector::new(
        w.store.clone(),
        Some(collector.client()),
        ProjectorSettings::default(),
    )
}

/// A caller who starts a one-step run, takes the task and completes it; returns how long the
/// completion took (the transaction that moves the journal and leaves the note).
async fn one_run(
    w: &World,
    wf: sagat_domain::definition::WorkflowId,
    session: &support::engine::Session,
    key: &str,
) -> Duration {
    w.start_with(wf, &json!({}), key, None)
        .await
        .expect("start");
    let task = w.one_task(session).await;
    let t = Instant::now();
    w.complete(session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
    t.elapsed()
}

/// Runs the projector until nothing is left to take or `stop` is set; returns what it did.
async fn project_until_idle(
    projector: &TelemetryProjector<sagat_infra_postgres::PgRunStore>,
    stop: &AtomicBool,
) -> (u64, u64, u64) {
    let (mut runs, mut spans, mut rounds) = (0u64, 0u64, 0u64);
    loop {
        let round = projector.run_once().await;
        rounds += 1;
        runs += u64::from(round.runs);
        spans += u64::from(round.spans);
        if !round.did_work() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    (runs, spans, rounds)
}

#[tokio::test]
#[ignore = "benchmark: run on purpose with --ignored"]
async fn the_note_costs_a_writer_little_the_projector_keeps_up_and_the_monitor_scales() {
    let w = world().await;
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
    let collector = Collector::start().await;
    let (single, _) = w
        .publish(document("bench-single", "a", vec![activity("a")]))
        .await;
    let (pair, _) = w.publish(chain("bench-pair")).await;
    let session = w.session(&["act.a", "act.b"]).await;

    // ------------------------------------------------ 1. the cost of the note to a writer
    // The projector is idle: the completion is the write path as it is.
    let mut idle = Vec::new();
    for i in 0..WRITE_SAMPLES {
        idle.push(one_run(&w, single, &session, &format!("idle-{i:05}")).await);
    }
    // The statement that leaves the note, on its own, against a run that exists: the same upsert the
    // write path runs, inside a transaction that is rolled back.
    let (tenant, run): (uuid::Uuid, uuid::Uuid) =
        sqlx::query_as("SELECT tenant_id, run_id FROM telemetry_pending LIMIT 1")
            .fetch_one(&w.db.pool)
            .await
            .expect("a note");
    let mut note = Vec::new();
    for _ in 0..WRITE_SAMPLES {
        let mut tx = w.db.pool.begin().await.expect("begin");
        let t = Instant::now();
        sqlx::query(
            "INSERT INTO telemetry_pending (tenant_id, run_id, head_seq, applied_seq, behind_since) \
             VALUES ($1, $2, $3, 0, clock_timestamp()) \
             ON CONFLICT (tenant_id, run_id) DO UPDATE \
               SET behind_since = CASE WHEN telemetry_pending.applied_seq >= telemetry_pending.head_seq \
                                       THEN clock_timestamp() ELSE telemetry_pending.behind_since END, \
                   head_seq = GREATEST(telemetry_pending.head_seq, EXCLUDED.head_seq)",
        )
        .bind(tenant)
        .bind(run)
        .bind(1_000_000_i64)
        .execute(&mut *tx)
        .await
        .expect("upsert");
        note.push(t.elapsed());
        tx.rollback().await.expect("rollback");
    }
    // The same completions while the projector works flat out on the backlog and then on the new
    // runs as they appear.
    let busy_projector = projector(&w, &collector);
    let stop = AtomicBool::new(false);
    let busy = {
        let writing = async {
            let mut samples = Vec::new();
            for i in 0..WRITE_SAMPLES {
                samples.push(one_run(&w, single, &session, &format!("busy-{i:05}")).await);
            }
            stop.store(true, Ordering::SeqCst);
            samples
        };
        let (samples, _) = tokio::join!(writing, project_until_idle(&busy_projector, &stop));
        samples
    };
    let idle_row = row("complete a step, projector idle", idle);
    let busy_row = row("complete a step, projector working", busy);
    let note_row = row("the note's upsert alone (rolled back)", note);
    let slowdown = ms(busy_row.p50) / ms(idle_row.p50).max(0.001);
    let note_share = ms(note_row.p50) / ms(idle_row.p50).max(0.001);

    // ------------------------------------------------ 2. taking a backlog
    for i in 0..BACKLOG_RUNS {
        w.start_with(pair, &json!({}), &format!("backlog-{i:05}"), None)
            .await
            .expect("start");
        for _ in 0..2 {
            let task = w.one_task(&session).await;
            w.complete(&session, &task, &json!({"ok": true}))
                .await
                .expect("complete");
        }
    }
    let before = w.store.backlog().await.expect("backlog");
    let spans_before = collector.spans().len();
    let take = projector(&w, &collector);
    let taking = Instant::now();
    let (runs, spans, rounds) = project_until_idle(&take, &AtomicBool::new(true)).await;
    let taken_in = taking.elapsed();
    let rate = runs as f64 / taken_in.as_secs_f64();
    let received = collector.spans().len() - spans_before;
    assert_eq!(w.store.backlog().await.expect("backlog").runs, 0);
    assert_eq!(
        received as u64, spans,
        "every span the projector counted reached the collector"
    );

    // ------------------------------------------------ 3. lag under sustained writing
    let sustained_projector = projector(&w, &collector);
    let stop = AtomicBool::new(false);
    let finished = AtomicBool::new(false);
    let started = Instant::now();
    let (completed, lags) = {
        let writing = async {
            let mut completed = 0usize;
            let mut i = 0usize;
            while started.elapsed() < SUSTAINED {
                let batch = stream::iter(i..i + WRITERS * 4)
                    .map(|n| {
                        let w = &w;
                        let session = &session;
                        async move {
                            w.start_with(pair, &json!({}), &format!("sustained-{n:06}"), None)
                                .await
                                .expect("start");
                            let mut done = 0usize;
                            // Whatever is ready, the caller's own steps or another's: all the same.
                            for _ in 0..2 {
                                for task in w.poll(session, 4).await {
                                    w.complete(session, &task, &json!({"ok": true}))
                                        .await
                                        .expect("complete");
                                    done += 1;
                                }
                            }
                            done
                        }
                    })
                    .buffer_unordered(WRITERS)
                    .collect::<Vec<_>>()
                    .await;
                completed += batch.iter().sum::<usize>();
                i += WRITERS * 4;
            }
            // The rest of the steps that were left ready.
            loop {
                let tasks = w.poll(&session, 8).await;
                if tasks.is_empty() {
                    break;
                }
                for task in tasks {
                    w.complete(&session, &task, &json!({"ok": true}))
                        .await
                        .expect("complete");
                    completed += 1;
                }
            }
            stop.store(true, Ordering::SeqCst);
            completed
        };
        let watching = async {
            let mut lags = Vec::new();
            while !finished.load(Ordering::SeqCst) {
                let b = w.store.backlog().await.expect("backlog");
                lags.push((b.runs, b.oldest_seconds));
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            lags
        };
        let ((completed, _), lags) = tokio::join!(
            async {
                let r = tokio::join!(writing, project_until_idle(&sustained_projector, &stop));
                finished.store(true, Ordering::SeqCst);
                r
            },
            watching
        );
        (completed, lags)
    };
    let wrote_for = started.elapsed();
    let write_rate = completed as f64 / wrote_for.as_secs_f64();
    let max_lag = lags.iter().map(|(_, s)| *s).fold(0.0_f64, f64::max);
    let max_backlog = lags.iter().map(|(r, _)| *r).max().unwrap_or(0);
    let mut sorted_lag: Vec<f64> = lags.iter().map(|(_, s)| *s).collect();
    sorted_lag.sort_by(f64::total_cmp);
    let p95_lag = sorted_lag[((sorted_lag.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(sorted_lag.len() - 1)];

    // ------------------------------------------------ 4. a monitor round as the platform fills
    let (waiting, _) = w
        .publish(document("bench-waiting", "w", vec![activity("w")]))
        .await;
    let worst_case = MonitorSettings {
        thresholds: sagat_application::operations::Thresholds {
            outbox_age: Duration::ZERO,
            lease_grace: Duration::ZERO,
            unclaimed: Duration::ZERO,
            review: Duration::ZERO,
        },
        ..MonitorSettings::default()
    };
    let mut monitor_rows = Vec::new();
    let mut monitor_p95_last = Duration::ZERO;
    let mut live = 0usize;
    for level in LEVELS {
        while live < level {
            let upto = level.min(live + 500);
            stream::iter(live..upto)
                .map(|n| {
                    let w = &w;
                    async move {
                        w.start_with(waiting, &json!({}), &format!("live-{n:06}"), None)
                            .await
                            .expect("start");
                    }
                })
                .buffer_unordered(16)
                .collect::<Vec<_>>()
                .await;
            live = upto;
        }
        sqlx::query("ANALYZE")
            .execute(&w.db.pool)
            .await
            .expect("analyze");
        for (label, settings) in [
            (
                "default thresholds (nothing fires)",
                MonitorSettings::default(),
            ),
            (
                "every threshold zero (everything fires)",
                worst_case.clone(),
            ),
        ] {
            let monitor = Monitor::new(w.store.clone(), settings);
            let mut samples = Vec::new();
            for _ in 0..ROUNDS_PER_LEVEL {
                let t = Instant::now();
                let round = monitor.run_once().await;
                samples.push(t.elapsed());
                assert!(!round.passive, "the benchmark's monitor holds the lease");
            }
            let r = row(
                &format!("monitor round, {live} live runs, {label}"),
                samples,
            );
            if label.starts_with("every") {
                monitor_p95_last = r.p95;
            }
            monitor_rows.push(r.line);
        }
    }

    // What each detection query costs by itself at the largest population, and what it was looking at.
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM event_outbox WHERE delivered_at IS NULL), \
                (SELECT count(*) FROM steps WHERE status = 'READY'), \
                (SELECT count(*) FROM runs)",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("counts");
    let (outbox_rows, ready_rows, all_runs) = counts;
    let mut detect_rows = Vec::new();
    for kind in sagat_domain::operations::AlertKind::ALL {
        let mut samples = Vec::new();
        for _ in 0..ROUNDS_PER_LEVEL {
            let t = Instant::now();
            w.store
                .detect(kind, &worst_case.thresholds)
                .await
                .expect("detect");
            samples.push(t.elapsed());
        }
        detect_rows.push(
            row(
                &format!("detect {}, every threshold zero", kind.as_str()),
                samples,
            )
            .line,
        );
    }

    // ------------------------------------------------ 5. the new reads
    let finished_run = w
        .start_with(single, &json!({}), "read-run", None)
        .await
        .expect("start");
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
    let mut telemetry = Vec::new();
    let mut attempts = Vec::new();
    for _ in 0..READ_SAMPLES {
        let t = Instant::now();
        w.runs
            .telemetry(&w.admin, finished_run, &ctx())
            .await
            .expect("telemetry");
        telemetry.push(t.elapsed());
        let t = Instant::now();
        w.runs
            .attempts(&w.admin, finished_run, &ctx())
            .await
            .expect("attempts");
        attempts.push(t.elapsed());
    }
    let audit_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_records")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    let reading = AuditRecord::by(
        &w.admin,
        AuditAction::AuditRead,
        ResourceType::Tenant,
        w.admin.tenant_id().to_string(),
        None,
        time::OffsetDateTime::now_utc(),
    );
    let mut audit_all = Vec::new();
    let mut audit_one = Vec::new();
    for _ in 0..READ_SAMPLES {
        let t = Instant::now();
        w.store
            .read_audit(
                w.admin.tenant_id(),
                &AuditFilter::default(),
                None,
                50,
                &reading,
            )
            .await
            .expect("audit");
        audit_all.push(t.elapsed());
        let filter = AuditFilter {
            action: Some("run.started".to_owned()),
            ..AuditFilter::default()
        };
        let t = Instant::now();
        w.store
            .read_audit(w.admin.tenant_id(), &filter, None, 50, &reading)
            .await
            .expect("audit");
        audit_one.push(t.elapsed());
    }
    let mut alerts = Vec::new();
    for _ in 0..READ_SAMPLES {
        let t = Instant::now();
        w.store
            .list_alerts(w.admin.tenant_id(), w.namespace.id(), None, None, 50)
            .await
            .expect("alerts");
        alerts.push(t.elapsed());
    }
    let telemetry_row = row("GET /runs/{id}/telemetry (service call)", telemetry);
    let attempts_row = row("GET /runs/{id}/attempts (service call)", attempts);
    let audit_all_row = row(
        &format!("GET /audit, newest 50 of {audit_rows} records"),
        audit_all,
    );
    let audit_one_row = row(
        &format!("GET /audit?action=run.started, 50 of {audit_rows} records"),
        audit_one,
    );
    let alerts_row = row("GET /alerts, newest 50", alerts);
    let reads_p95 = [
        telemetry_row.p95,
        attempts_row.p95,
        audit_all_row.p95,
        audit_one_row.p95,
        alerts_row.p95,
    ]
    .into_iter()
    .max()
    .expect("rows");

    // ------------------------------------------------ report
    let mut report = format!(
        "# Benchmark: observability and operations (feature 008)\n\nMeasured by `tests/performance/observability-operations/main.rs`. Hypotheses, not promises: figures are valid only for this environment.\n\n## Environment\n\n{}\n\n## Latency\n\n| Operation | Samples | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|---:|\n",
        environment().replace("{DATABASE}", &database(&w.db.pool).await)
    );
    for line in [
        &idle_row.line,
        &busy_row.line,
        &note_row.line,
        &telemetry_row.line,
        &attempts_row.line,
        &audit_all_row.line,
        &audit_one_row.line,
        &alerts_row.line,
    ] {
        report.push_str(line);
    }
    for line in monitor_rows.iter().chain(&detect_rows) {
        report.push_str(line);
    }
    report.push_str(&format!(
        "\n## Throughput and lag\n\n| Measurement | Result |\n|---|---|\n| backlog taken ({BACKLOG_RUNS} runs of two steps, none projected before) | {runs} runs and {spans} spans in {:.1} s over {rounds} rounds: **{rate:.0} runs/s** (backlog before: {} runs, oldest {:.1} s) |\n| sustained writing, {WRITERS} callers for {} s with the projector working | {completed} steps completed in {:.1} s: **{write_rate:.0} steps/s** |\n| lag during that, sampled every 100 ms | oldest unprojected change: p95 **{p95_lag:.1} s**, max **{max_lag:.1} s**; most runs waiting at once: {max_backlog} |\n\n## Hypotheses\n\n| Hypothesis | Result |\n|---|---|\n| a busy projector slows the median completion by less than {WRITER_SLOWDOWN}x | {:.1} ms to {:.1} ms ({slowdown:.2}x): **{}** |\n| the note's upsert is less than {:.0}% of the median completion | {:.2} ms of {:.1} ms ({:.0}%): **{}** |\n| a backlog is projected at no less than {PROJECT_RATE:.0} runs/s | {rate:.0} runs/s: **{}** |\n| under sustained writing the oldest unprojected change stays under the {} s lag budget | max {max_lag:.1} s: **{}** |\n| a monitor round at {} live runs, every condition firing, p95 < {} ms | {:.1} ms: **{}** |\n| each new read, p95 < {} ms | {:.1} ms (the slowest): **{}** |\n\n## Reading the numbers\n\n- The write path pays one upsert of a row of its own per transaction that moves the journal. The row is per run, so writers of different runs do not meet on it; writers of the same run already meet on the run's row. The projector reads the notes and the journal in transactions of its own and takes no lock a writer waits for: that is what the second row of the table compares with the first.\n- The statement alone was timed in a transaction that was rolled back, so it is a bound on what the note adds and not an A/B of the engine with and without it: the engine has no switch for that and one would not be honest to add only for this.\n- The projector's rate is set by the database round trips of a batch of runs (50) and by the collector; here the collector is in the same process and parses the spans, which a real one does elsewhere. A backlog it cannot take in the lag budget is what `TELEMETRY_BEHIND` and the run's `BEHIND` state report; one projector at a time is a design limit (G177, feature 010).\n- Lag is measured as the database sees it (`oldest_seconds` of the backlog), at 100 ms, so a figure below that is below the resolution. It includes the time a change waits for the projector's next round.\n- A monitor round is a handful of aggregate queries and one reconciliation per kind. With every threshold at zero each ready step, each undelivered outbox row and each lease counts as a condition: the worst case for the detection queries, plus the writes for the few alerts that result (at most 25 per kind per namespace). **This benchmark runs no outbox dispatcher, so every message it ever created is still undelivered** ({outbox_rows} rows when the detection rows were measured, with {ready_rows} ready steps and {all_runs} runs in all): the outbox query reads those rows, and it is the figure to look at in the detection rows. A healthy deployment has a few seconds of outbox and not this, but this is the one query whose cost grows with the very failure it detects (a dispatcher that is down): about 9 µs per undelivered message here, so some 10 s of a round at a million. Bounding the scan is the remedy and is not done (G186). The round with default thresholds (nothing old enough to fire) is the cost of looking when all is well.\n- The audit reads are measured at {audit_rows} records, with and without a filter, and the reading itself is written in the same transaction (included). The plan was not examined and the trail has no retention (G182): measure again when it holds millions.\n- The machine is shared with unrelated workloads (see the load average), so absolute figures vary between runs; the structural claims (a note per run, no lock shared with the projector, aggregate detection) are what the tests prove.\n",
        taken_in.as_secs_f64(),
        before.runs,
        before.oldest_seconds,
        SUSTAINED.as_secs(),
        wrote_for.as_secs_f64(),
        ms(idle_row.p50),
        ms(busy_row.p50),
        verdict(slowdown < WRITER_SLOWDOWN),
        NOTE_SHARE * 100.0,
        ms(note_row.p50),
        ms(idle_row.p50),
        note_share * 100.0,
        verdict(note_share < NOTE_SHARE),
        verdict(rate >= PROJECT_RATE),
        LAG_BUDGET.as_secs(),
        verdict(max_lag < LAG_BUDGET.as_secs_f64()),
        LEVELS[LEVELS.len() - 1],
        MONITOR_P95.as_millis(),
        ms(monitor_p95_last),
        verdict(monitor_p95_last < MONITOR_P95),
        READ_P95.as_millis(),
        ms(reads_p95),
        verdict(reads_p95 < READ_P95),
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
        slowdown < WRITER_SLOWDOWN,
        "a busy projector slowed the median completion {slowdown:.2}x"
    );
    assert!(
        note_share < NOTE_SHARE,
        "the note is {note_share:.2} of a completion"
    );
    assert!(
        rate >= PROJECT_RATE,
        "the backlog was taken at {rate:.0} runs/s"
    );
    assert!(
        max_lag < LAG_BUDGET.as_secs_f64(),
        "the projection fell {max_lag:.1} s behind"
    );
    assert!(
        monitor_p95_last < MONITOR_P95,
        "monitor round p95 {monitor_p95_last:?}"
    );
    assert!(reads_p95 < READ_P95, "a read p95 {reads_p95:?}");
}

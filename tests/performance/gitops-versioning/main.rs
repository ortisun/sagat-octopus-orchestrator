//! Benchmark of feature 011 (T011-033, NFR-011-PERF): what publishing, moving the active pointer and
//! the command line cost.
//!
//! The specification asks that publishing be asynchronous in builds (it is a call a pipeline makes
//! and waits for, once per workflow that changed) and that the switch of the active pointer be
//! transactional. This asks six questions of the design (ADR-0018):
//!
//! 1. what publishing a version costs as the document grows (5, 100 and 1000 nodes);
//! 2. what moving the pointer costs, with and without event triggers to register again;
//! 3. whether moving the pointer slows the runs that are being started meanwhile;
//! 4. what happens when callers contend: several operators on one workflow, publishers on
//!    different ones;
//! 5. what reading the history costs when it is long, and what the approval path costs;
//! 6. what a pipeline waits for: `sagat publish` of a manifest of 50 workflows, and `sagat diff`
//!    when nothing changed.
//!
//! Everything runs in one process, against a PostgreSQL on the same machine: the figures leave out
//! the network between a pipeline and the platform. The numbers are only meaningful together with
//! the environment, which the report records. Run on purpose:
//!
//! ```sh
//! SAGAT_PERF_REPORT=tests/performance/reports/gitops-versioning.md \
//!   cargo test -p sagat-tests --test perf_gitops_versioning --release -- --ignored --nocapture
//! ```
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

#[path = "../../support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use futures_util::future::join_all;
use sagat_application::identity::PageRequest;
use sagat_domain::definition::WorkflowId;
use sagat_domain::identity::{Environment, Role};
use sagat_domain::versioning::Verdict;
use serde_json::{Value, json};
use support::engine::{World, activity, ctx, document, with, world};
use support::worker::{platform, single_ir};

/// Hypotheses to hold the system to, not promises.
const PUBLISH_SMALL_P95: Duration = Duration::from_millis(50);
const PUBLISH_LARGE_P95: Duration = Duration::from_millis(500);
const SWITCH_P95: Duration = Duration::from_millis(25);
const START_SLOWDOWN: f64 = 1.5;
const HISTORY_P95: Duration = Duration::from_millis(20);
const REQUEST_P95: Duration = Duration::from_millis(100);
const CLI_PUBLISH_50: Duration = Duration::from_secs(15);
const CLI_DIFF_50: Duration = Duration::from_secs(3);

const LARGE_NODES: usize = 1_000;
const HISTORY_ROWS: i64 = 10_000;
const MANIFEST: usize = 50;

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn shown(d: Duration) -> String {
    if ms(d) < 1.0 {
        format!("{:.2} ms", ms(d))
    } else if ms(d) < 1000.0 {
        format!("{:.1} ms", ms(d))
    } else {
        format!("{:.2} s", d.as_secs_f64())
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
            "| {name} | {} | {} | {} | {} | {} |\n",
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

fn environment(database: &str) -> String {
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
        "- CPU: {cpu} ({cores} logical cores)\n- OS: {}\n- Load average when the run began: {load} (other work on the machine adds noise)\n- Build profile of the test process and services: {profile}\n- Commit: {commit}\n{database}\n- Services and the command line run in-process (no network between them and the client; the command line speaks HTTP to the API on the loopback)",
        sh("uname", &["-srm"])
    )
}

async fn timed<T>(f: impl std::future::Future<Output = T>) -> (Duration, T) {
    let start = Instant::now();
    let out = f.await;
    (start.elapsed(), out)
}

/// A chain of `n` activities, `marker` making the digest differ between versions.
fn chain_of(name: &str, n: usize, marker: &str) -> Value {
    let nodes: Vec<Value> = (0..n)
        .map(|i| {
            let node = activity(&format!("n{i}"));
            if i + 1 < n {
                with(node, "next", json!(format!("n{}", i + 1)))
            } else {
                node
            }
        })
        .collect();
    let mut doc = document(name, "n0", nodes);
    doc["metadata"]["description"] = json!(marker);
    doc
}

/// A one-activity workflow started by `triggers` event triggers.
fn triggered(name: &str, triggers: usize, marker: &str) -> Value {
    let mut doc = document(name, "work", vec![activity("work")]);
    doc["triggers"] = Value::Array(
        (0..triggers)
            .map(|i| json!({"kind": "event", "eventType": format!("order.created-{i}")}))
            .collect(),
    );
    doc["metadata"]["description"] = json!(marker);
    doc
}

/// Publishes a new version of `id`, timing the publication alone (the edit of the draft is not
/// part of what a pipeline waits for at the end).
async fn publish_timed(w: &World, id: WorkflowId, doc: &Value, n: usize) -> Duration {
    let record = w
        .defs
        .get_workflow(&w.admin, id, &ctx())
        .await
        .expect("workflow");
    let updated = w
        .defs
        .update_draft(
            &w.admin,
            id,
            record.workflow.draft_revision(),
            &serde_json::to_vec(doc).expect("json"),
            &ctx(),
        )
        .await
        .expect("draft");
    let (took, published) = timed(w.defs.publish(
        &w.admin,
        id,
        updated.workflow.draft_revision(),
        None,
        &format!("bench-publish-{n}-{}", uuid::Uuid::now_v7()),
        &ctx(),
    ))
    .await;
    published.expect("published");
    took
}

#[tokio::test]
#[ignore = "benchmark: run on purpose, see the module documentation"]
async fn the_cost_of_publishing_moving_the_pointer_and_the_command_line() {
    let w = world().await;
    let mut table = String::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut push = |r: Row, table: &mut String| {
        table.push_str(&r.line);
        rows.push(r);
    };

    // 1. publishing as the document grows
    let mut small = Vec::new();
    let (id_small, _) = w.publish(chain_of("small", 5, "v0")).await;
    for n in 0..100 {
        small.push(
            publish_timed(
                &w,
                id_small,
                &chain_of("small", 5, &format!("v{}", n + 1)),
                n,
            )
            .await,
        );
    }
    let small = row("publish a version, 5 nodes", small);
    let small_p95 = small.p95;
    push(small, &mut table);
    let (id_mid, _) = w.publish(chain_of("medium", 100, "v0")).await;
    let mut mid = Vec::new();
    for n in 0..50 {
        mid.push(
            publish_timed(
                &w,
                id_mid,
                &chain_of("medium", 100, &format!("v{}", n + 1)),
                n,
            )
            .await,
        );
    }
    push(row("publish a version, 100 nodes", mid), &mut table);
    let (id_large, _) = w.publish(chain_of("large", LARGE_NODES, "v0")).await;
    let mut large = Vec::new();
    for n in 0..20 {
        large.push(
            publish_timed(
                &w,
                id_large,
                &chain_of("large", LARGE_NODES, &format!("v{}", n + 1)),
                n,
            )
            .await,
        );
    }
    let large = row(&format!("publish a version, {LARGE_NODES} nodes"), large);
    let large_p95 = large.p95;
    push(large, &mut table);

    // 2. moving the pointer
    let ops = w.member("olga", Role::Operator).await;
    let (flip_plain, _) = w.publish(triggered("plain", 0, "one")).await;
    w.publish_new_version(flip_plain, triggered("plain", 0, "two"))
        .await;
    let mut plain = Vec::new();
    for round in 0..200u32 {
        let (took, done) = timed(w.publishing.activate(
            &ops,
            flip_plain,
            1 + round % 2,
            "benchmark: moving the pointer",
            None,
            true,
            &ctx(),
        ))
        .await;
        done.expect("moved");
        plain.push(took);
    }
    let plain = row("move the pointer, no triggers", plain);
    let mut switch_p95 = plain.p95;
    push(plain, &mut table);
    let (flip_triggers, _) = w.publish(triggered("with-triggers", 16, "one")).await;
    w.publish_new_version(flip_triggers, triggered("with-triggers", 16, "two"))
        .await;
    let mut with_triggers = Vec::new();
    for round in 0..200u32 {
        let (took, done) = timed(w.publishing.activate(
            &ops,
            flip_triggers,
            1 + round % 2,
            "benchmark: moving the pointer",
            None,
            true,
            &ctx(),
        ))
        .await;
        done.expect("moved");
        with_triggers.push(took);
    }
    let with_triggers = row(
        "move the pointer, 16 event triggers registered again",
        with_triggers,
    );
    switch_p95 = switch_p95.max(with_triggers.p95);
    push(with_triggers, &mut table);

    // 3. starting runs while the pointer moves
    let (started, _) = w.publish(triggered("started", 0, "one")).await;
    w.publish_new_version(started, triggered("started", 0, "two"))
        .await;
    let mut idle = Vec::new();
    for _ in 0..300 {
        let (took, run) = timed(w.start_with(
            started,
            &json!({}),
            &format!("bench-start-{}", uuid::Uuid::now_v7()),
            None,
        ))
        .await;
        run.expect("started");
        idle.push(took);
    }
    let idle = row("start a run, the pointer still", idle);
    let idle_p50 = idle.p50;
    push(idle, &mut table);
    let stop = std::sync::atomic::AtomicBool::new(false);
    let flipper = async {
        let mut moved = 0u32;
        while !stop.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = w
                .publishing
                .activate(
                    &ops,
                    started,
                    1 + moved % 2,
                    "benchmark: moving the pointer",
                    None,
                    true,
                    &ctx(),
                )
                .await;
            moved += 1;
        }
        moved
    };
    let starter = async {
        let mut samples = Vec::new();
        for _ in 0..300 {
            let (took, run) = timed(w.start_with(
                started,
                &json!({}),
                &format!("bench-start-{}", uuid::Uuid::now_v7()),
                None,
            ))
            .await;
            run.expect("started");
            samples.push(took);
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        samples
    };
    let (moved, moving) = {
        let (moved, samples) = futures_util::future::join(flipper, starter).await;
        (moved, samples)
    };
    let moving = row(
        "start a run, an operator moving the pointer all the while",
        moving,
    );
    let moving_p50 = moving.p50;
    push(moving, &mut table);
    let slowdown = ms(moving_p50) / ms(idle_p50).max(0.001);

    // 4. contention
    let (contended, _) = w.publish(triggered("contended", 0, "one")).await;
    w.publish_new_version(contended, triggered("contended", 0, "two"))
        .await;
    let t = Instant::now();
    let mut done = 0u64;
    let mut refused = 0u64;
    while t.elapsed() < Duration::from_secs(5) {
        let attempts = join_all((0..8u32).map(|i| {
            let (w, ops) = (&w, &ops);
            async move {
                w.publishing
                    .activate(
                        ops,
                        contended,
                        1 + i % 2,
                        "benchmark: eight operators",
                        Some(1 + (i + 1) % 2),
                        true,
                        &ctx(),
                    )
                    .await
            }
        }))
        .await;
        done += attempts.iter().filter(|a| a.is_ok()).count() as u64;
        refused += attempts.iter().filter(|a| a.is_err()).count() as u64;
    }
    let contended_for = t.elapsed();
    let contended_rate = done as f64 / contended_for.as_secs_f64();
    let mut ids = Vec::new();
    for n in 0..8 {
        ids.push(w.publish(chain_of(&format!("own-{n}"), 5, "v0")).await.0);
    }
    // each of eight publishers has a workflow of its own: names must differ, so they edit and publish theirs.
    let t = Instant::now();
    let mut published = 0u64;
    while t.elapsed() < Duration::from_secs(5) {
        let round = published / 8;
        join_all(ids.iter().enumerate().map(|(n, id)| {
            let w = &w;
            let id = *id;
            async move {
                publish_timed(
                    w,
                    id,
                    &chain_of(&format!("own-{n}"), 5, &format!("round-{round}")),
                    0,
                )
                .await
            }
        }))
        .await;
        published += 8;
    }
    let apart_rate = published as f64 / t.elapsed().as_secs_f64();

    // 5. a long history, and the approval path
    let (long, _) = w.publish(triggered("long", 0, "one")).await;
    w.publish_new_version(long, triggered("long", 0, "two"))
        .await;
    sqlx::query(
        "INSERT INTO workflow_activations (id, tenant_id, workflow_id, version_id, previous_version_id, kind, reason, actor, created_at) \
         SELECT gen_random_uuid(), a.tenant_id, a.workflow_id, \
                CASE WHEN g % 2 = 0 THEN a.version_id ELSE a.previous_version_id END, \
                CASE WHEN g % 2 = 0 THEN a.previous_version_id ELSE a.version_id END, \
                'ROLLBACK', 'benchmark history', 'olga', a.created_at + g * interval '1 second' \
         FROM workflow_activations a, generate_series(1, $2) g \
         WHERE a.workflow_id = $1 AND a.previous_version_id IS NOT NULL ORDER BY a.created_at DESC LIMIT $2",
    )
    .bind(long.as_uuid())
    .bind(HISTORY_ROWS)
    .execute(&w.db.pool)
    .await
    .expect("history");
    let rows_in_history: i64 =
        sqlx::query_scalar("SELECT count(*) FROM workflow_activations WHERE workflow_id = $1")
            .bind(long.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    let mut history = Vec::new();
    for _ in 0..200 {
        let (took, page) = timed(w.publishing.activations(
            &w.admin,
            long,
            &PageRequest {
                limit: Some(50),
                cursor: None,
            },
            &ctx(),
        ))
        .await;
        assert_eq!(page.expect("page").items.len(), 50);
        history.push(took);
    }
    let history = row(
        &format!("list the history, newest 50 of {rows_in_history}"),
        history,
    );
    let history_p95 = history.p95;
    push(history, &mut table);

    w.add_namespace_in("billing-prod", Environment::Prod).await;
    let author = w.member("author", Role::Developer).await;
    let reviewer = w.member("reviewer", Role::Developer).await;
    let mut path = Vec::new();
    for n in 0..50 {
        let draft = w
            .defs
            .create_workflow(
                &w.admin,
                "billing-prod",
                &format!("flow-{n}"),
                &serde_json::to_vec(&single_ir(&format!("flow-{n}"), None)).expect("json"),
                &format!("bench-create-{n}-{}", uuid::Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("draft");
        let (took, ()) = timed(async {
            let request = w
                .publishing
                .open_request(
                    &author,
                    draft.workflow.id(),
                    draft.workflow.draft_revision(),
                    None,
                    &ctx(),
                )
                .await
                .expect("request");
            w.publishing
                .decide(
                    &reviewer,
                    request.id(),
                    Verdict::Approve,
                    "benchmark: looks right",
                    &ctx(),
                )
                .await
                .expect("approved");
            w.publishing
                .publish_request(
                    &author,
                    request.id(),
                    &format!("bench-request-{n}-{}", uuid::Uuid::now_v7()),
                    &ctx(),
                )
                .await
                .expect("published");
        })
        .await;
        path.push(took);
    }
    let path = row("open, approve and publish a request (three calls)", path);
    let request_p95 = path.p95;
    push(path, &mut table);

    // 6. the command line, end to end over HTTP
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    let dir = std::env::temp_dir().join(format!("sagat-bench-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("workflows")).expect("dirs");
    let mut manifest = String::from(
        "apiVersion: sagat-octopus-orchestrator.io/v1\nkind: Manifest\nnamespace: payments\nworkflows:\n",
    );
    for n in 0..MANIFEST {
        let name = format!("flow-{n}");
        manifest.push_str(&format!(
            "  - name: {name}\n    file: workflows/{name}.json\n"
        ));
        std::fs::write(
            dir.join(format!("workflows/{name}.json")),
            serde_json::to_string(&single_ir(&name, None)).expect("json"),
        )
        .expect("write");
    }
    std::fs::write(dir.join("sagat.yaml"), manifest).expect("manifest");
    let run = |args: &'static [&'static str]| {
        let (p, dir) = (&p, dir.clone());
        async move {
            let env = sagat_cli::Env {
                server: Some(p.api.base.clone()),
                token: Some(p.api.idp.token("dev1", "acme").build()),
                namespace: None,
                cwd: dir,
            };
            let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = sagat_cli::run(&args, &env, &mut out, &mut err).await;
            (code, String::from_utf8_lossy(&err).into_owned())
        }
    };
    let (first_diff, (code, said)) = timed(run(&["diff"])).await;
    assert_eq!(code, 0, "{said}");
    let (publish_all, (code, said)) = timed(run(&["publish"])).await;
    assert_eq!(code, 0, "{said}");
    let (diff_none, (code, said)) = timed(run(&["diff", "--check"])).await;
    assert_eq!(code, 0, "{said}");
    for n in 0..5 {
        let name = format!("flow-{n}");
        let mut doc = single_ir(&name, None);
        doc["metadata"]["description"] = json!("changed");
        std::fs::write(
            dir.join(format!("workflows/{name}.json")),
            serde_json::to_string(&doc).expect("json"),
        )
        .expect("write");
    }
    let (publish_five, (code, said)) = timed(run(&["publish"])).await;
    assert_eq!(code, 0, "{said}");
    let _ = std::fs::remove_dir_all(&dir);

    let database = database(&w.db.pool).await;
    let mut report = format!(
        "# Benchmark: GitOps and versioning (feature 011)\n\nMeasured by `tests/performance/gitops-versioning/main.rs`. Hypotheses, not promises: figures are valid only for this environment.\n\n## Environment\n\n{}\n\n## Latency\n\n| Operation | Samples | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|---:|\n",
        environment(&database)
    );
    report.push_str(&table);
    report.push_str(&format!(
        "\n## Contention and the command line\n\n| Measurement | Result |\n|---|---|\n| eight operators moving the pointer of one workflow, each expecting the version the others will have changed (5 s) | {done} moved and {refused} refused as stale in {:.1} s: **{contended_rate:.0} moves/s** |\n| eight publishers, each on a workflow of its own (5 s) | {published} versions: **{apart_rate:.0} versions/s** |\n| start a run while the pointer moves: median slowdown | {:.2} ms to {:.2} ms (**{slowdown:.2}x**), the pointer moved {moved} times meanwhile |\n| `sagat diff`, {MANIFEST} workflows none of which exist yet | {} |\n| `sagat publish`, {MANIFEST} new workflows (create, then publish) | {} (**{:.0} ms** a workflow) |\n| `sagat diff --check`, {MANIFEST} workflows, nothing changed | {} |\n| `sagat publish`, {MANIFEST} workflows of which 5 changed | {} |\n\n## Hypotheses\n\n| Hypothesis | Result |\n|---|---|\n| publishing a 5-node version, p95 < {} ms | {}: **{}** |\n| publishing a {LARGE_NODES}-node version, p95 < {} ms | {}: **{}** |\n| moving the pointer (with or without triggers), p95 < {} ms | {}: **{}** |\n| moving the pointer slows the median start of a run by less than {START_SLOWDOWN}x | {slowdown:.2}x: **{}** |\n| the history, 50 of {rows_in_history} rows, p95 < {} ms | {}: **{}** |\n| the approval path (open, approve, publish), p95 < {} ms | {}: **{}** |\n| `sagat publish` of {MANIFEST} new workflows < {} s | {}: **{}** |\n| `sagat diff` of {MANIFEST} unchanged workflows < {} s | {}: **{}** |\n\n## Reading the numbers\n\n- Publishing parses and validates the document (schema, graph rules, expressions), canonicalises it, hashes it and stores the version, the compensation plans, the change of the pointer and the dependencies in one transaction: the cost grows with the document, as the 5, 100 and {LARGE_NODES} node rows show. The edit of the draft before it is not part of the figure.\n- Moving the pointer takes the workflow row lock, so changes of one workflow are serialised and changes of different workflows are not: that is what the two rates under contention compare. The stale refusals in the first are the compare-and-swap doing its job (`expectedVersion`); they are cheap and are not retried by the benchmark.\n- A run is started on the version the pointer holds when it is read; starting does not take the lock the switch takes, so the slowdown is the cost of the database being busy, not of waiting for the switch.\n- The history is read through an index on `(tenant_id, workflow_id, created_at DESC, id DESC)`; the rows were inserted by a statement that copies real ones, which is not how they grow. The history has no retention (G194).\n- The command line figures include everything a pipeline waits for: reading the manifest, validating every document with the platform's codec, listing the namespace, reading the active version of each changed workflow and, for `publish`, a create and a publish per workflow, each an HTTP call. There is no network between the command line and the API here: add the round trips of a real one (about 4 calls a changed workflow).\n- The machine is shared with unrelated workloads (see the load average), so absolute figures vary between runs; the structural claims (one transaction, one lock per workflow, a compare-and-swap) are what the tests prove.\n",
        contended_for.as_secs_f64(),
        ms(idle_p50),
        ms(moving_p50),
        shown(first_diff),
        shown(publish_all),
        ms(publish_all) / MANIFEST as f64,
        shown(diff_none),
        shown(publish_five),
        PUBLISH_SMALL_P95.as_millis(),
        shown(small_p95),
        verdict(small_p95 < PUBLISH_SMALL_P95),
        PUBLISH_LARGE_P95.as_millis(),
        shown(large_p95),
        verdict(large_p95 < PUBLISH_LARGE_P95),
        SWITCH_P95.as_millis(),
        shown(switch_p95),
        verdict(switch_p95 < SWITCH_P95),
        verdict(slowdown < START_SLOWDOWN),
        HISTORY_P95.as_millis(),
        shown(history_p95),
        verdict(history_p95 < HISTORY_P95),
        REQUEST_P95.as_millis(),
        shown(request_p95),
        verdict(request_p95 < REQUEST_P95),
        CLI_PUBLISH_50.as_secs(),
        shown(publish_all),
        verdict(publish_all < CLI_PUBLISH_50),
        CLI_DIFF_50.as_secs(),
        shown(diff_none),
        verdict(diff_none < CLI_DIFF_50),
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
        small_p95 < PUBLISH_SMALL_P95,
        "publishing 5 nodes p95 {small_p95:?}"
    );
    assert!(
        large_p95 < PUBLISH_LARGE_P95,
        "publishing {LARGE_NODES} nodes p95 {large_p95:?}"
    );
    assert!(
        switch_p95 < SWITCH_P95,
        "moving the pointer p95 {switch_p95:?}"
    );
    assert!(
        slowdown < START_SLOWDOWN,
        "starting a run slowed {slowdown:.2}x"
    );
    assert!(history_p95 < HISTORY_P95, "history p95 {history_p95:?}");
    assert!(
        request_p95 < REQUEST_P95,
        "the approval path p95 {request_p95:?}"
    );
    assert!(
        publish_all < CLI_PUBLISH_50,
        "publish of {MANIFEST} took {publish_all:?}"
    );
    assert!(
        diff_none < CLI_DIFF_50,
        "diff of {MANIFEST} took {diff_none:?}"
    );
}

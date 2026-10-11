//! Benchmark of feature 012 (T012-033, NFR-012-PERF): the machine's part of the first day.
//!
//! The specification asks for an onboarding under fifteen minutes with the dependencies installed,
//! "in UX evaluation, without promising a universal time". A benchmark cannot evaluate a person's
//! reading, typing and thinking, and it does not pretend to: it measures what the *machine* takes
//! on the way, so that the part of the fifteen minutes that is not the person's is known and small.
//!
//! 1. the commands of the quickstart in order (`doctor`, `new`, `workflow validate`, `workflow
//!    publish`, `worker bind`, and a run served by `worker dev` from its start to `SUCCEEDED`),
//!    repeated on a fresh platform each time;
//! 2. a token from the development identity provider;
//! 3. how long a two-step run takes end to end when `worker dev` serves it (what a developer
//!    waits for each time they try a change);
//! 4. each SDK's hello-world: its `prepare.sh` from a clean build (the package caches of the machine
//!    are warm: a first run downloads) and its `run.sh` from the start of the process to `SUCCEEDED`.
//!
//! It does not measure the time to build the container images of the stack (this machine could not
//! run containers), the time to read the documentation, or anything about a person. Run on purpose:
//!
//! ```sh
//! SAGAT_PERF_REPORT=tests/performance/reports/developer-experience.md \
//!   cargo test -p sagat-tests --test perf_developer_experience --release -- --ignored --nocapture
//! ```
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

#[path = "../../support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use support::dx::{Dir, cli, engine, run_with};
use support::worker::platform;

/// Hypotheses to hold the system to, not promises.
const MACHINE_FIRST_DAY_P95: Duration = Duration::from_secs(10);
const RUN_P95: Duration = Duration::from_secs(3);
const TOKEN_P95: Duration = Duration::from_millis(50);
const EXAMPLE_RUN: Duration = Duration::from_secs(30);
const EXAMPLE_PREPARE: Duration = Duration::from_secs(300);

const ROUNDS: usize = 5;
const RUNS: usize = 30;
const LANGUAGES: [&str; 5] = ["typescript", "go", "python", "dotnet", "java"];

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

fn verdict(ok: bool) -> &'static str {
    if ok { "held" } else { "NOT held" }
}

fn row(name: &str, mut samples: Vec<Duration>) -> (String, Duration, Duration) {
    samples.sort();
    let (p50, p95) = (percentile(&samples, 0.50), percentile(&samples, 0.95));
    let line = format!(
        "| {name} | {} | {} | {} | {} |",
        samples.len(),
        shown(p50),
        shown(p95),
        shown(*samples.last().expect("samples"))
    );
    (line, p50, p95)
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
    let toolchains = [
        ("node", "--version"),
        ("go", "version"),
        ("python3", "--version"),
        ("dotnet", "--version"),
        ("java", "-version"),
    ]
    .iter()
    .map(|(c, a)| {
        let out = std::process::Command::new(c).arg(a).output();
        let text = out
            .map(|o| {
                let both = [o.stdout, o.stderr].concat();
                String::from_utf8_lossy(&both)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned()
            })
            .unwrap_or_else(|_| "not found".to_owned());
        format!("{c}: {text}")
    })
    .collect::<Vec<_>>()
    .join("; ");
    format!(
        "- CPU: {cpu} ({cores} logical cores)\n- OS: {}\n- Load average when the run began: {load} (other work on the machine adds noise)\n- Build profile of the test process and the tool: {profile}\n- Commit: {commit}\n- Toolchains: {toolchains}\n- PostgreSQL in a server named by SAGAT_TEST_DATABASE_URL (or a container) on the same machine; the platform and the tool run in-process, so there is no network between them (the tool speaks HTTP to the API on the loopback)",
        sh("uname", &["-srm"])
    )
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Runs an example's script and says how long it took and whether it worked. The platform runs in
/// this process, so the script is waited for without blocking the thread that serves it.
async fn timed(
    example: &str,
    language: &str,
    script: &str,
    setting: &[(&str, String)],
) -> (Duration, bool, String) {
    let path = repo()
        .join("examples")
        .join(example)
        .join(language)
        .join(script);
    let started = Instant::now();
    let mut command = tokio::process::Command::new(&path);
    command
        .current_dir(path.parent().expect("dir"))
        .kill_on_drop(true);
    for (k, v) in setting {
        command.env(k, v);
    }
    let output = command.output().await.expect("run the script");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (started.elapsed(), output.status.success(), text)
}

/// What a clean build of an example removes first: the outputs, not the machine's package caches.
fn clean(language: &str) {
    let dir = repo().join("examples/hello-world").join(language);
    for leftover in ["node_modules", "dist", ".venv", "bin", "obj", "target"] {
        let _ = std::fs::remove_dir_all(dir.join(leftover));
    }
}

#[tokio::test]
#[ignore = "a benchmark: run on purpose, in release mode (see the head of this file)"]
async fn the_machines_part_of_the_first_day() {
    // 1. The quickstart's commands, in order, on a fresh platform each time.
    let (mut doctor, mut new, mut validate, mut publish, mut bind, mut served_run, mut total) =
        (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
    for _ in 0..ROUNDS {
        let p = platform().await;
        let engine = engine(&p).await;
        let dir = Dir::new();
        let all = Instant::now();
        let step = |label: &'static str| (label, Instant::now());
        let (_, t) = step("doctor");
        cli(&p, &dir, &["doctor", "--grpc", &engine.grpc_url])
            .await
            .expect(0);
        doctor.push(t.elapsed());
        let (_, t) = step("new");
        cli(&p, &dir, &["new", "hello-world"]).await.expect(0);
        new.push(t.elapsed());
        let (_, t) = step("validate");
        cli(
            &p,
            &dir,
            &["workflow", "validate", "hello-world/workflow.json"],
        )
        .await
        .expect(0);
        validate.push(t.elapsed());
        let (_, t) = step("publish");
        cli(
            &p,
            &dir,
            &["workflow", "publish", "hello-world/workflow.json"],
        )
        .await
        .expect(0);
        publish.push(t.elapsed());
        let (_, t) = step("bind");
        cli(
            &p,
            &dir,
            &["worker", "bind", "--workflow", "hello-world/workflow.json"],
        )
        .await
        .expect(0);
        bind.push(t.elapsed());
        let (_, t) = step("run");
        let serve = [
            "worker",
            "dev",
            "--mocks",
            "hello-world/mocks.yaml",
            "--workflow",
            "hello-world/workflow.json",
            "--grpc",
            &engine.grpc_url,
            "--max-tasks",
            "2",
            "--idle-exit",
            "30",
        ];
        let start = [
            "workflow",
            "run",
            "hello-world",
            "--input",
            r#"{"name":"Ada"}"#,
            "--wait",
            "--timeout",
            "60",
        ];
        let (served, ran) = tokio::join!(cli(&p, &dir, &serve), cli(&p, &dir, &start));
        served.expect(0);
        ran.expect(0).mentions("SUCCEEDED");
        served_run.push(t.elapsed());
        total.push(all.elapsed());
    }

    // 2. A token from the development identity provider.
    let idp = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let pairs = [
            ("SAGAT_ENV", "dev".to_owned()),
            ("SAGAT_DEV_IDP_ADDR", addr.to_string()),
            ("SAGAT_DEV_IDP_ISSUER", format!("http://{addr}/")),
        ];
        let config = sagat_dev_idp::Config::from_lookup(&|n| {
            pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| v.clone())
        })
        .expect("config");
        let app = sagat_dev_idp::router(config, sagat_dev_idp::DevKey::generate().expect("key"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    };
    let dir = Dir::new();
    let mut tokens = Vec::new();
    for _ in 0..100 {
        let t = Instant::now();
        let env = sagat_cli::Env {
            cwd: dir.0.clone(),
            ..Default::default()
        };
        run_with(env, &["dev", "token", "--idp", &idp])
            .await
            .expect(0);
        tokens.push(t.elapsed());
    }

    // 3. How long a two-step run takes when `worker dev` serves it.
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    cli(&p, &dir, &["new", "hello-world"]).await.expect(0);
    cli(
        &p,
        &dir,
        &["workflow", "publish", "hello-world/workflow.json"],
    )
    .await
    .expect(0);
    let tasks = (RUNS * 2).to_string();
    let serve = [
        "worker",
        "dev",
        "--mocks",
        "hello-world/mocks.yaml",
        "--workflow",
        "hello-world/workflow.json",
        "--bind",
        "--grpc",
        &engine.grpc_url,
        "--max-tasks",
        &tasks,
        "--idle-exit",
        "60",
    ];
    let mut runs = Vec::new();
    let driver = async {
        for _ in 0..RUNS {
            let t = Instant::now();
            cli(
                &p,
                &dir,
                &[
                    "workflow",
                    "run",
                    "hello-world",
                    "--input",
                    r#"{"name":"Ada"}"#,
                    "--wait",
                    "--timeout",
                    "60",
                ],
            )
            .await
            .expect(0);
            runs.push(t.elapsed());
        }
    };
    let (served, ()) = tokio::join!(cli(&p, &dir, &serve), driver);
    served.expect(0);

    // 4. Each SDK's hello-world, from a clean build.
    let token = p.api.idp.token("alice", "acme").build();
    let setting = vec![
        ("SAGAT_SERVER", p.api.base.clone()),
        ("SAGAT_GRPC", engine.grpc_url.clone()),
        ("SAGAT_NAMESPACE", "payments".to_owned()),
        ("SAGAT_TOKEN", token),
    ];
    cli(
        &p,
        &dir,
        &["worker", "bind", "--workflow", "hello-world/workflow.json"],
    )
    .await
    .expect(0);
    let mut examples: Vec<(String, Duration, Duration)> = Vec::new();
    for language in LANGUAGES {
        clean(language);
        let (prepare, ok, text) = timed("hello-world", language, "prepare.sh", &setting).await;
        assert!(ok, "{language} prepare.sh:\n{text}");
        let (first, ok, text) = timed("hello-world", language, "run.sh", &setting).await;
        assert!(
            ok && text.contains("SUCCEEDED"),
            "{language} run.sh:\n{text}"
        );
        let (second, ok, text) = timed("hello-world", language, "run.sh", &setting).await;
        assert!(ok, "{language} run.sh:\n{text}");
        examples.push((language.to_owned(), prepare, first.min(second)));
    }

    let rows = [
        row("`sagat doctor`", doctor),
        row("`sagat new hello-world`", new),
        row("`sagat workflow validate`", validate),
        row("`sagat workflow publish`", publish),
        row("`sagat worker bind`", bind),
        row(
            "`worker dev` + `workflow run --wait`, start to SUCCEEDED",
            served_run,
        ),
    ];
    let (_, _, total_p95) = row("total", total.clone());
    let (token_line, _, token_p95) = row("`sagat dev token`", tokens);
    let (run_line, _, run_p95) = row(
        "one two-step run served by `worker dev`, `workflow run --wait`",
        runs,
    );
    let (total_line, ..) = row(
        "the commands above, together (what the machine takes)",
        total,
    );
    let example_rows: Vec<String> = examples
        .iter()
        .map(|(language, prepare, run)| {
            format!(
                "| {language} | {} | {} | {} | {} |",
                shown(*prepare),
                shown(*run),
                verdict(*prepare < EXAMPLE_PREPARE),
                verdict(*run < EXAMPLE_RUN)
            )
        })
        .collect();
    let slowest_run = examples.iter().map(|e| e.2).max().expect("examples");
    let slowest_prepare = examples.iter().map(|e| e.1).max().expect("examples");

    let report = format!(
        "# Benchmark: developer experience (feature 012)\n\nMeasured by `tests/performance/developer-experience/main.rs`. Hypotheses, not promises: figures are valid only for this environment.\n\n## What this is, and what it is not\n\nThe specification asks for an onboarding under fifteen minutes with the dependencies installed, \"in UX evaluation, without promising a universal time\" (`NFR-012-PERF`, `SC-012-001`). **This benchmark does not evaluate that**: it measures only what the *machine* takes on the way, so that the part of the fifteen minutes that is not the person's reading, typing and thinking is known. Nobody was timed. It also does not include building the container images of the stack (the machine this was run on could not run containers; the `local-environment` job of CI builds them) or downloading dependencies (the package caches were warm).\n\n## Environment\n\n{}\n\n## Latency\n\n| Operation | Samples | p50 | p95 | max |\n|---|---:|---:|---:|---:|\n{}\n{}\n{}\n{}\n\nThe six commands together, on a fresh platform: {}. Their rows have only five samples, so their p95 is the slowest one.\n\n## Each SDK's hello-world\n\n`prepare.sh` from a clean build (the example's `node_modules`, `.venv`, `bin`, `obj`, `target` removed first), and `run.sh` (the better of two runs): serve the two activities, start a run, wait for `SUCCEEDED`.\n\n| Language | prepare.sh | run.sh | prepare < {} s | run < {} s |\n|---|---:|---:|---|---|\n{}\n\n## Hypotheses\n\n| Hypothesis | Measured | |\n|---|---|---|\n| the quickstart's commands take the machine under {} s together (p95) | {} | {} |\n| one run served by `worker dev` takes under {} s (p95) | {} | {} |\n| a token from the development provider under {} ms (p95) | {} | {} |\n| each example's `run.sh` under {} s | slowest {} | {} |\n| each example's `prepare.sh` under {} s | slowest {} | {} |\n\n## What the figures say, and do not\n\n- A run served by `worker dev` is dominated by the worker's polling and the engine's scheduling interval, not by the work; it is a few hundred milliseconds to a few seconds, which is what a developer waits each time they try a change.\n- The JVM and the .NET host start slower than Node, Go or Python; the examples are one process that is the worker and the client, so that start is in every `run.sh` figure.\n- `prepare.sh` is the cost of building the SDK from this checkout; a published package would replace most of it with a download.\n- The commands ran in-process against a PostgreSQL on the same machine: over a real network each call costs its round trip more. The same holds for the compose stack, which is a set of containers.\n",
        environment(),
        rows.iter()
            .map(|r| r.0.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        token_line,
        run_line,
        total_line,
        shown(total_p95),
        EXAMPLE_PREPARE.as_secs(),
        EXAMPLE_RUN.as_secs(),
        example_rows.join("\n"),
        MACHINE_FIRST_DAY_P95.as_secs(),
        shown(total_p95),
        verdict(total_p95 < MACHINE_FIRST_DAY_P95),
        RUN_P95.as_secs(),
        shown(run_p95),
        verdict(run_p95 < RUN_P95),
        TOKEN_P95.as_millis(),
        shown(token_p95),
        verdict(token_p95 < TOKEN_P95),
        EXAMPLE_RUN.as_secs(),
        shown(slowest_run),
        verdict(slowest_run < EXAMPLE_RUN),
        EXAMPLE_PREPARE.as_secs(),
        shown(slowest_prepare),
        verdict(slowest_prepare < EXAMPLE_PREPARE),
    );
    println!("{report}");
    if let Ok(path) = std::env::var("SAGAT_PERF_REPORT") {
        let target = if Path::new(&path).is_absolute() {
            path.into()
        } else {
            repo().join(path)
        };
        std::fs::write(&target, &report).expect("write the report");
    }
    assert!(
        total_p95 < MACHINE_FIRST_DAY_P95,
        "the commands took {total_p95:?}"
    );
    assert!(run_p95 < RUN_P95, "a run took {run_p95:?}");
    assert!(token_p95 < TOKEN_P95, "a token took {token_p95:?}");
    assert!(
        slowest_run < EXAMPLE_RUN,
        "an example run took {slowest_run:?}"
    );
    assert!(
        slowest_prepare < EXAMPLE_PREPARE,
        "an example build took {slowest_prepare:?}"
    );
}

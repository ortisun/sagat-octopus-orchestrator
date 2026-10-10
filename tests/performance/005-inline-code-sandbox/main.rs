//! Benchmark of feature 005 (NFR-005-PERF): what it costs to build and run a script in a container
//! of its own.
//!
//! The plan sets no number, only a method: cold and warm start per language, caches only when the
//! policy and the digest are identical, builds apart from execution. Six questions follow. How long
//! from asking for a container to the first instruction of the script, for the first run of a
//! language (cold) and for the ones after it (warm)? How long does a build take, and what does a
//! build that is not made cost? How long does an author wait for a test run? How many script steps
//! per second does a runner with room for one container and one with room for four get through, end
//! to end through the engine? How long after its limit is a script that will not stop actually gone?
//! And what does the isolation cost, against running the same code as a plain container?
//!
//! The numbers are only meaningful together with the environment, which the report records; the
//! containers run in the VM of Docker Desktop on this machine, which is not what a production host
//! looks like. Run on purpose:
//!
//! ```sh
//! SAGAT_PERF_REPORT=tests/performance/reports/005-inline-code-sandbox.md \
//!   cargo test -p sagat-tests --test perf_005 --release -- --ignored --nocapture
//! ```
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

#[path = "../../support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use reqwest::Method;
use sagat_application::sandbox::{BuildRequest, RunRequest, Sandbox};
use sagat_domain::scripts::{Language, Limits, Policy, Source};
use sagat_infra_sandbox::{DockerSandbox, Images};
use sagat_sandbox_runner::host::ScriptHost;
use serde_json::json;
use support::scripts::{lab, lab_configured};

const SANDBOX_SAMPLES: usize = 30;
const BUILD_SAMPLES: usize = 15;
const TEST_RUN_SAMPLES: usize = 20;
const STEP_RUNS: usize = 40;
const SCALING_REPEATS: usize = 3;
const ENFORCEMENT_SAMPLES: usize = 3;

/// Hypotheses to hold the system to, written before the first run; not promises.
/// A script is running in a warm container's namespace within this of asking for it.
const WARM_START: Duration = Duration::from_millis(1500);
/// The first run of a language after the process started, with the image already on the machine.
const COLD_START: Duration = Duration::from_secs(5);
/// A build, from asking to a ready version.
const BUILD: Duration = Duration::from_secs(3);
/// Publishing what was built before makes no container: it is a database write and a lookup.
const CACHED_BUILD: Duration = Duration::from_millis(250);
/// What an author waits for a test run (build and run), at the 95th percentile.
const TEST_RUN_P95: Duration = Duration::from_secs(5);
/// A runner with room for four containers gets this much more done than one with room for one. Not
/// four: the engine, the VM and the database are shared.
const SCALING: f64 = 1.5;
/// A script that will not stop is gone this long after its limit.
const KILL_OVERSHOOT: Duration = Duration::from_secs(2);

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
            "| {name} | {} | {:.0} ms | {:.0} ms | {:.0} ms | {:.0} ms |\n",
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
    let engine = sh(
        "docker",
        &[
            "version",
            "--format",
            "{{.Server.Version}} {{.Server.Os}}/{{.Server.Arch}}",
        ],
    );
    let runtime = sh(
        "docker",
        &[
            "info",
            "--format",
            "{{.DefaultRuntime}} (cpus {{.NCPU}}, memory {{.MemTotal}} bytes)",
        ],
    );
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
        "- CPU: {cpu} ({cores} logical cores)\n- OS: {}\n- Container engine: Docker {engine}, runtime {runtime}\n- Images: node:24-alpine, python:3.13-alpine (already on the machine)\n- Load average when the run began: {load} (other work on the machine adds noise)\n- Build profile of the test process and services: {profile}\n- Commit: {commit}\n- PostgreSQL 16 in a Docker container on the same machine; the API, the engine's worker protocol and the runner run in-process, over loopback",
        sh("uname", &["-srm"])
    )
}

fn hello(language: Language) -> &'static str {
    match language {
        Language::TypeScript => {
            "export default function (input: unknown) { return { ok: true }; }\n"
        }
        _ => "def run(input, octopus):\n    return {'ok': True}\n",
    }
}

/// Builds a script and runs it `SANDBOX_SAMPLES` times straight on the sandbox (no control plane, no
/// database): what the isolation costs by itself.
async fn sandbox_starts(
    sandbox: &DockerSandbox,
    language: Language,
) -> (Vec<Duration>, Vec<Duration>) {
    let built = sandbox
        .build(BuildRequest {
            language,
            source: Source::parse(hello(language)).expect("source"),
            lockfile: None,
            policy: Policy::default(),
            input_schema: None,
            output_schema: None,
        })
        .await
        .expect("builds");
    let (mut starts, mut walls) = (Vec::new(), Vec::new());
    for _ in 0..SANDBOX_SAMPLES {
        let outcome = sandbox
            .run(
                RunRequest {
                    language,
                    artifact: built.artifact.clone(),
                    input: b"{}".to_vec(),
                    input_schema: None,
                    output_schema: None,
                    policy: Policy::default(),
                    secrets_to_hide: Vec::new(),
                    idempotency_key: None,
                },
                &ScriptHost::closed(),
            )
            .await;
        assert!(
            outcome.result.is_ok(),
            "{:?} {}",
            outcome.result,
            outcome.log
        );
        starts.push(outcome.start);
        walls.push(outcome.wall);
    }
    (starts, walls)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: run on purpose, in release mode"]
async fn script_sandbox_benchmark() {
    let mut report = String::new();

    // ------------------------------------------------------------ start, by language, cold and warm
    let sandbox = DockerSandbox::connect(Images::default()).expect("the container engine");
    let mut starts_table = String::new();
    let (mut cold, mut warm_p50) = (Vec::new(), Vec::new());
    for language in [Language::TypeScript, Language::Python] {
        let (starts, walls) = sandbox_starts(&sandbox, language).await;
        cold.push((language, starts[0]));
        let rest = starts[1..].to_vec();
        let (line, p50, _) = row(
            &format!("{language}: start (first instruction), warm"),
            rest,
        );
        warm_p50.push((language, p50));
        starts_table.push_str(&line);
        starts_table.push_str(
            &row(
                &format!("{language}: whole run (create, start, run, remove), warm"),
                walls[1..].to_vec(),
            )
            .0,
        );
    }
    report.push_str("## Start\n\nStraight on the sandbox (no control plane, no database): a build, then the same artifact run thirty times, each in a container of its own. *Start* is from asking for the container to the first instruction of the script (the shim's `ready`); the first run of a language after the process began is the cold one, the rest are warm. The images are already on the machine: pulling one is the network's time and is not measured.\n\n| What | Samples | p50 | p95 | p99 | max |\n|---|---|---|---|---|---|\n");
    report.push_str(&starts_table);
    report.push_str("\n| Language | Cold start (first run) |\n|---|---|\n");
    for (language, d) in &cold {
        report.push_str(&format!("| {language} | {:.0} ms |\n", ms(*d)));
    }

    // ----------------------------------------------------------------------------- builds, by REST
    let lab = lab().await;
    let mut builds = Vec::new();
    let mut python_builds = Vec::new();
    for i in 0..BUILD_SAMPLES {
        for (language, name, source, sink) in [
            (
                "typescript",
                format!("ts-build-{i}"),
                format!("export default () => ({i});\n"),
                &mut builds,
            ),
            (
                "python",
                format!("py-build-{i}"),
                format!("def run(input, octopus):\n    return {i}\n"),
                &mut python_builds,
            ),
        ] {
            let script = lab.script(&name, language, &source).await;
            let started = Instant::now();
            let version = lab.publish(&script).await;
            sink.push(started.elapsed());
            assert_eq!(version["status"], "READY", "{version}");
        }
    }
    let mut cached = Vec::new();
    for i in 0..BUILD_SAMPLES {
        // The same source as build `i`, in another script: the artifact exists.
        let script = lab
            .script(
                &format!("twin-{i}"),
                "typescript",
                &format!("export default () => ({i});\n"),
            )
            .await;
        let started = Instant::now();
        let (status, published) = lab
            .call(
                Method::POST,
                &format!("/namespaces/{}/scripts/{script}/versions", lab.ns),
                None,
            )
            .await;
        assert_eq!(status, 202, "{published}");
        let id = published["version"]["number"].as_u64().expect("number");
        let (_, version) = lab
            .call(
                Method::GET,
                &format!("/namespaces/{}/scripts/{script}/versions/{id}", lab.ns),
                None,
            )
            .await;
        // The runner is asked, as it would be: it is handed nothing, and the version is ready.
        assert!(
            !lab.runner.build_once().await.expect("claim"),
            "nothing to build"
        );
        let (_, version_after) = lab
            .call(
                Method::GET,
                &format!("/namespaces/{}/scripts/{script}/versions/{id}", lab.ns),
                None,
            )
            .await;
        assert_eq!(
            version_after["status"], "READY",
            "{version} / {version_after}"
        );
        cached.push(started.elapsed());
    }
    let (ts_line, ts_build_p50, _) = row("typescript: publish to ready (a build)", builds);
    let (py_line, py_build_p50, _) = row("python: publish to ready (a build)", python_builds);
    let (cached_line, cached_p50, _) = row(
        "typescript: publish of an identical build (no container)",
        cached,
    );
    report.push_str("\n## Builds\n\nFrom `POST .../versions` to a ready version, as an author sees it: queue, the runner's claim, a container that checks the source, the report, the signature. The last row publishes the same source again in another script: the control plane finds the artifact by its build key and the runner is never handed a build.\n\n| What | Samples | p50 | p95 | p99 | max |\n|---|---|---|---|---|---|\n");
    report.push_str(&ts_line);
    report.push_str(&py_line);
    report.push_str(&cached_line);

    // ---------------------------------------------------------------------------------- test runs
    let script = lab
        .script(
            "tried",
            "typescript",
            "export default (i: any) => ({ got: i });\n",
        )
        .await;
    let mut turnaround = Vec::new();
    for i in 0..TEST_RUN_SAMPLES {
        let started = Instant::now();
        let run = lab.test(&script, &json!({"n": i})).await;
        turnaround.push(started.elapsed());
        assert_eq!(run["status"], "SUCCEEDED", "{run}");
    }
    let (test_line, _, test_p95) = row("typescript: test run (build, run, report)", turnaround);
    report.push_str("\n## What an author waits for\n\nA test run through the API: the draft is built and run with an input, and the author reads the result. Each includes a build.\n\n| What | Samples | p50 | p95 | p99 | max |\n|---|---|---|---|---|---|\n");
    report.push_str(&test_line);

    // --------------------------------------------------------------------- steps, one and four wide
    let digest = lab
        .ready(
            &lab.script(
                "steps",
                "typescript",
                "export default () => ({ done: true });\n",
            )
            .await,
        )
        .await;
    let workflow = lab.workflow("wf-steps", &digest, json!({})).await;
    let mut single = Vec::new();
    for _ in 0..STEP_RUNS.min(10) {
        let run = lab.p.start(&workflow, &json!({})).await;
        let started = Instant::now();
        assert_eq!(lab.drive(&run).await, "SUCCEEDED");
        single.push(started.elapsed());
    }
    let (single_line, _, _) = row(
        "typescript step, one run at a time (start to SUCCEEDED)",
        single,
    );
    drop(lab);

    let mut rates: Vec<(usize, Vec<f64>)> = Vec::new();
    for width in [1usize, 4] {
        let mut samples = Vec::new();
        for _ in 0..SCALING_REPEATS {
            let lab = lab_configured(support::api::Options::default(), |config| {
                config.max_concurrent = width
            })
            .await;
            let script = lab
                .script(
                    "steps",
                    "typescript",
                    "export default () => ({ done: true });\n",
                )
                .await;
            let digest = lab.ready(&script).await;
            let workflow = lab.workflow("wf-steps", &digest, json!({})).await;
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let serving = tokio::spawn(std::sync::Arc::clone(&lab.runner).serve(async move {
                let _ = stopped.await;
            }));
            let mut runs = Vec::new();
            for _ in 0..STEP_RUNS {
                runs.push(lab.p.start(&workflow, &json!({})).await);
            }
            let began = Instant::now();
            loop {
                lab.p.tick().await;
                let done: i64 =
                    sqlx::query_scalar("SELECT count(*) FROM runs WHERE status = 'SUCCEEDED'")
                        .fetch_one(&lab.p.api.db.pool)
                        .await
                        .expect("count");
                if usize::try_from(done).unwrap_or(0) >= STEP_RUNS {
                    break;
                }
                assert!(
                    began.elapsed() < Duration::from_secs(300),
                    "the runs did not finish"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            samples.push(STEP_RUNS as f64 / began.elapsed().as_secs_f64());
            let _ = stop.send(());
            let _ = serving.await;
        }
        samples.sort_by(f64::total_cmp);
        rates.push((width, samples));
    }
    let median = |v: &[f64]| v[v.len() / 2];
    let (one, four) = (median(&rates[0].1), median(&rates[1].1));
    report.push_str("\n## Script steps through the engine\n\nA workflow of one script node, started by the API, leased by the runner through the worker protocol, run in a container and completed; the runner's serving loop and the scheduler run as they do in production. Each cell is the median of three runs of forty, with the lowest and highest in brackets.\n\n");
    report.push_str("| What | Samples | p50 | p95 | p99 | max |\n|---|---|---|---|---|---|\n");
    report.push_str(&single_line);
    report.push_str("\n| Containers at once (`SAGAT_RUNNER_MAX_CONCURRENT`) | Script steps per second |\n|---|---|\n");
    for (width, samples) in &rates {
        report.push_str(&format!(
            "| {width} | {:.1} ({:.1} to {:.1}) |\n",
            median(samples),
            samples[0],
            samples[samples.len() - 1]
        ));
    }
    report.push_str(&format!(
        "\nFour at once got **{:.1}x** the throughput of one.\n",
        four / one
    ));

    // --------------------------------------------------------------------------------- enforcement
    let tight = Policy {
        limits: Limits::new(
            64 * 1024 * 1024,
            1000,
            Duration::from_secs(3),
            64,
            64 * 1024,
            16 * 1024 * 1024,
        )
        .expect("limits"),
        ..Policy::default()
    };
    // The flood is about the log and not about memory, so it runs with 128 MiB (the first version of
    // this benchmark gave it 64 MiB, and the kernel ended the process before the supervisor cut it
    // off: `SCRIPT_OUT_OF_MEMORY`; the cause was the shim, see below).
    let roomy = Policy {
        limits: Limits::new(
            128 * 1024 * 1024,
            1000,
            Duration::from_secs(3),
            64,
            64 * 1024,
            16 * 1024 * 1024,
        )
        .expect("limits"),
        ..Policy::default()
    };
    let mut overshoots = Vec::new();
    let mut bomb_times = Vec::new();
    let mut flood_times = Vec::new();
    for _ in 0..ENFORCEMENT_SAMPLES {
        for (language, source, sink, expect, policy) in [
            (
                Language::TypeScript,
                "export default function () { for (;;) {} }\n",
                &mut overshoots,
                "SCRIPT_TIMEOUT",
                tight,
            ),
            (
                Language::Python,
                "def run(input, octopus):\n    hog = []\n    while True:\n        hog.append(bytearray(8 * 1024 * 1024))\n",
                &mut bomb_times,
                "SCRIPT_OUT_OF_MEMORY",
                tight,
            ),
            (
                Language::TypeScript,
                "export default function () { const l = 'x'.repeat(1000); for (let i = 0; i < 5_000_000; i++) console.log(l); }\n",
                &mut flood_times,
                "SCRIPT_OUTPUT_LIMIT",
                roomy,
            ),
        ] {
            let built = sandbox
                .build(BuildRequest {
                    language,
                    source: Source::parse(source).expect("source"),
                    lockfile: None,
                    policy,
                    input_schema: None,
                    output_schema: None,
                })
                .await
                .expect("builds");
            let started = Instant::now();
            let outcome = sandbox
                .run(
                    RunRequest {
                        language,
                        artifact: built.artifact,
                        input: b"{}".to_vec(),
                        input_schema: None,
                        output_schema: None,
                        policy,
                        secrets_to_hide: Vec::new(),
                        idempotency_key: None,
                    },
                    &ScriptHost::closed(),
                )
                .await;
            let took = started.elapsed();
            assert_eq!(
                outcome.result.as_ref().err().map(|f| f.code()),
                Some(expect),
                "{}",
                outcome.log
            );
            sink.push(if expect == "SCRIPT_TIMEOUT" {
                took.saturating_sub(Duration::from_secs(3))
            } else {
                took
            });
        }
    }
    let (kill_line, _, kill_p95) = row(
        "CPU burner: time past its 3 s limit until it is gone",
        overshoots,
    );
    report.push_str("\n## Holding a script to its limits\n\nA hostile script run on the sandbox with a 3 second limit and 64 MiB of memory (128 MiB for the log flood): how long until it is gone. *The first runs of this benchmark ended the flood with `SCRIPT_OUT_OF_MEMORY` (64 and 128 MiB) and, once the cause was looked for, sometimes with a normal `null` result. The cause was the Node shim, which wrote the log through `process.stderr`: that stream queues what the pipe will not take and drops what is still queued when the process exits. A script that wrote faster than it was read filled its memory with the queue or lost almost all of its output, and in the second case the supervisor never saw the megabyte it needs to cut the script off (a plain `docker run` of the same script delivers 0.4 to 0.9 MB of 2 GB to a fast reader). The shim now writes the descriptor itself, so a flood meets the pipe's backpressure; a run the supervisor stopped for its output is reported as that even if the kernel ended it a moment later.*\n\n| What | Samples | p50 | p95 | p99 | max |\n|---|---|---|---|---|---|\n");
    report.push_str(&kill_line);
    report.push_str(
        &row(
            "memory bomb: until the kernel killed it (64 MiB)",
            bomb_times,
        )
        .0,
    );
    report.push_str(&row("log flood: until it was cut off (128 MiB)", flood_times).0);
    assert_eq!(sandbox.leftovers().await, 0, "no container was left behind");

    // -------------------------------------------------------------------------------------- verdicts
    let held = |ok: bool| if ok { "held" } else { "NOT MET" };
    let ts_warm = warm_p50
        .iter()
        .find(|(l, _)| *l == Language::TypeScript)
        .expect("ts")
        .1;
    let py_warm = warm_p50
        .iter()
        .find(|(l, _)| *l == Language::Python)
        .expect("py")
        .1;
    let colds_ok = cold.iter().all(|(_, d)| *d < COLD_START);
    report.push_str(
        "\n## Hypotheses\n\nWritten before the first run.\n\n| Hypothesis | Result |\n|---|---|\n",
    );
    report.push_str(&format!("| Warm start p50 under {:.1} s, both languages | {} (typescript {:.0} ms, python {:.0} ms) |\n", WARM_START.as_secs_f64(), held(ts_warm < WARM_START && py_warm < WARM_START), ms(ts_warm), ms(py_warm)));
    report.push_str(&format!(
        "| Cold start under {} s | {} |\n",
        COLD_START.as_secs(),
        held(colds_ok)
    ));
    report.push_str(&format!(
        "| A build under {} s (p50) | {} (typescript {:.0} ms, python {:.0} ms) |\n",
        BUILD.as_secs(),
        held(ts_build_p50 < BUILD && py_build_p50 < BUILD),
        ms(ts_build_p50),
        ms(py_build_p50)
    ));
    report.push_str(&format!(
        "| An identical build costs no container, under {} ms | {} ({:.0} ms) |\n",
        CACHED_BUILD.as_millis(),
        held(cached_p50 < CACHED_BUILD),
        ms(cached_p50)
    ));
    report.push_str(&format!(
        "| A test run under {} s (p95) | {} ({:.0} ms) |\n",
        TEST_RUN_P95.as_secs(),
        held(test_p95 < TEST_RUN_P95),
        ms(test_p95)
    ));
    report.push_str(&format!(
        "| Four containers at once get {SCALING}x the throughput of one | {} ({:.1}x) |\n",
        held(four / one >= SCALING),
        four / one
    ));
    report.push_str(&format!(
        "| A script that will not stop is gone within {} s of its limit | {} (p95 {:.0} ms) |\n",
        KILL_OVERSHOOT.as_secs(),
        held(kill_p95 < KILL_OVERSHOOT),
        ms(kill_p95)
    ));
    report.push_str("| A bounded environment (`environment()` above) | recorded |\n");

    let report = format!(
        "# Benchmark: inline code sandbox (feature 005)\n\nProduced by `tests/performance/005-inline-code-sandbox` (T005-034, NFR-005-PERF). Hypotheses are written down before the run and the result says whether each held; a hypothesis that fails is restated openly, not dropped.\n\n## Environment\n\n{}\n\n{report}",
        environment()
    );
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
        ts_warm < WARM_START && py_warm < WARM_START,
        "warm start {ts_warm:?} / {py_warm:?}"
    );
    assert!(colds_ok, "cold start {cold:?}");
    assert!(
        ts_build_p50 < BUILD && py_build_p50 < BUILD,
        "build {ts_build_p50:?} / {py_build_p50:?}"
    );
    assert!(
        cached_p50 < CACHED_BUILD,
        "an identical build took {cached_p50:?}"
    );
    assert!(test_p95 < TEST_RUN_P95, "test run p95 {test_p95:?}");
    assert!(
        four / one >= SCALING,
        "four containers got {:.2}x",
        four / one
    );
    assert!(kill_p95 < KILL_OVERSHOOT, "kill overshoot {kill_p95:?}");
}

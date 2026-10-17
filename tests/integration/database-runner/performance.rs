//! Phase timings from real HTTP/gRPC/runner/PostgreSQL/OpenBao, without failure injection.
use super::*;
use std::time::Instant;

const WARMUP: usize = 10;
const SAMPLES: usize = 200;

fn row(name: &str, mut values: Vec<f64>) -> String {
    assert_eq!(values.len(), SAMPLES);
    values.sort_by(f64::total_cmp);
    let p = |percent: usize| values[(values.len() * percent).div_ceil(100) - 1];
    format!(
        "| {name} | {SAMPLES} | {:.3} | {:.3} | {:.3} |\n",
        p(50),
        p(95),
        p(99)
    )
}

fn command(program: &str, args: &[&str]) -> String {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

pub async fn run(
    p: &Platform,
    ns: &str,
    proxy: &proxy::Running,
    collector: &support::collector::Collector,
) {
    let body = json!({"mode":"step","input":{},"definition":document("database-phase-benchmark", "query", vec![json!({
        "id":"query","type":"database","connectionRef":{"name":"target"},"operation":"query",
        "statement":"SELECT $1::text AS value","parameters":[{"literal":{"type":"text","value":"x".repeat(1024)}}]
    })])});
    let path = format!("/namespaces/{ns}/database-executions");
    let mut admission = Vec::new();
    for i in 0..WARMUP + SAMPLES {
        if i == WARMUP {
            // All warmups have completed. Measure only subsequent successful RPCs.
            proxy.faults.measure.store(true, Ordering::SeqCst);
        }
        let started = Instant::now();
        let (status, admitted) = p.rest(Method::POST, &path, Some(&body)).await;
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        assert_eq!(status, 201);
        let run = admitted["runId"].as_str().unwrap();
        finished(p, run, "SUCCEEDED").await;
        let (status, result) = p
            .rest(
                Method::GET,
                &format!("/namespaces/{ns}/runs/{run}/database-results/query"),
                None,
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(result["rows"][0][0]["value"].as_str().unwrap().len(), 1024);
        if i >= WARMUP {
            admission.push(elapsed);
        }
    }
    let spans = collector
        .wait_for(WARMUP + SAMPLES, |s| s.name == "database.connect")
        .await;
    assert_eq!(spans.len(), WARMUP + SAMPLES);
    let connect = spans
        .into_iter()
        .skip(WARMUP)
        .map(|s| (s.end - s.start) as f64 / 1_000_000.)
        .collect();
    let millis = |values: &std::sync::Mutex<Vec<Duration>>| {
        values
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.as_secs_f64() * 1000.)
            .collect()
    };
    let mut report = format!(
        "# Managed database phase latency\n\n- Date: {}\n- Revision: {} plus the measured working tree\n- OS: {}\n- Host: {}\n- Docker: {}\n- Profiles: API/engine test harness and native runner debug; PostgreSQL 16 and OpenBao 2.7.1 containers\n- Transport: numeric loopback development plaintext, persistent gRPC; fresh target connection per attempt\n- Workload: one tenant/namespace/connection, one concurrent run, {WARMUP} warmups then {SAMPLES} sequential successful reads; 1 text parameter and 1 row of 1,024 bytes\n- Bounds: operation/connection defaults; runner maxConcurrent=2, per-connection poolSize=1, result ceiling=1 MiB\n- No throughput or production TLS/HA latency claim; polling wait and result disclosure are excluded from phase timings.\n\n| Phase (milliseconds) | Samples | p50 | p95 | p99 |\n|---|---:|---:|---:|---:|\n",
        command("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"]),
        command("git", &["rev-parse", "--short", "HEAD"]),
        command("uname", &["-srm"]),
        if cfg!(target_os = "macos") {
            format!(
                "{}; {} bytes RAM; {} logical cores",
                command("sysctl", &["-n", "machdep.cpu.brand_string"]),
                command("sysctl", &["-n", "hw.memsize"]),
                command("sysctl", &["-n", "hw.logicalcpu"]),
            )
        } else {
            command("uname", &["-m"])
        },
        command(
            "docker",
            &[
                "info",
                "--format",
                "{{.NCPU}} CPUs; {{.MemTotal}} bytes memory; {{.ServerVersion}}"
            ]
        )
    );
    report.push_str(&row(
        "Admission (HTTP request through durable receipt)",
        admission,
    ));
    report.push_str(&row(
        "Secret capability (gRPC, both authorization transactions + OpenBao)",
        millis(&proxy.faults.resolutions),
    ));
    report.push_str(&row(
        "Connection acquisition (DNS policy + socket/auth; OTLP span)",
        connect,
    ));
    report.push_str(&row(
        "Result retention (CompleteStep RPC, provider scan + durable completion)",
        millis(&proxy.faults.completions),
    ));
    report.push_str("\nEvery sample asserted successful completion and the retained typed result. Failures: 0. Nearest-rank percentiles; timings include instrumentation overhead. Connection spans have no SQL, credentials or result fields. Result retention measures inline content only; blob and slow-reader limits have separate integration proofs.\n\nReproduce after building the runner: `cargo test -p sagat-tests --test integration_database_runner measure_managed_database_phase_latency --locked -- --ignored`. Set `SAGAT_DATABASE_PERF_REPORT` to choose the report path.\n\nResponse resource probe: [native slow-reader measurement](database-result-memory.md).\n");
    let destination = std::env::var_os("SAGAT_DATABASE_PERF_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("performance/reports/database-connections.md")
        });
    std::fs::write(destination, report).unwrap();
}

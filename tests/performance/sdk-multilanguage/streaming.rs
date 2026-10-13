//! Comparative transport evidence. Delivery latency joins a durable ready event to the public
//! handler's wall-clock entry timestamp; durable lease timing is reported separately.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::{harness, scenario, shell};

fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1)]
}

fn percentiles(values: &[f64]) -> String {
    if values.is_empty() {
        return "—".to_owned();
    }
    format!(
        "{:.2} / {:.2} / {:.2}",
        percentile(values, 0.5),
        percentile(values, 0.95),
        percentile(values, 0.99)
    )
}

fn percentile_or_dash(values: &[f64], fraction: f64) -> String {
    if values.is_empty() {
        "—".to_owned()
    } else {
        format!("{:.2}", percentile(values, fraction))
    }
}

fn fixture(workload: &str, transport: &str, dir: &Path) -> PathBuf {
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(scenario(workload)).expect("fixture")).expect("JSON");
    value["name"] = format!("stream-compare-{workload}-{transport}").into();
    let workers = value["workers"].as_object_mut().expect("workers");
    // The per-subject stream cap is two. Saturation is covered by the server admission tests;
    // this measurement compares equal admitted idle populations for the two transports.
    if workload == "idle" {
        workers.remove("w3");
    }
    for worker in workers.values_mut() {
        worker["transport"] = transport.into();
    }
    let path = dir.join(format!("{workload}-{transport}.json"));
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&value).expect("fixture serialization"),
    )
    .expect("write fixture");
    path
}

#[test]
#[ignore = "comparative five-SDK streaming benchmark needs runtimes and PostgreSQL"]
fn compare_polling_and_streaming() {
    harness::block_on(async {
        let _database = crate::support::hold_server();
        let languages = harness::available_languages();
        assert!(!languages.is_empty());
        let failures = harness::ensure_prepared(&languages).await;
        assert!(failures.is_empty(), "{failures:?}");
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/sdk-stream-comparison");
        std::fs::create_dir_all(&dir).expect("fixture directory");
        let mut report = String::from("# SDK task streaming comparison\n\n");
        let _ = writeln!(
            report,
            "Source: `{}` plus the working-tree changes; profile: {}; host: `{}`; CPU: `{}`.\n",
            shell("git", &["rev-parse", "--short", "HEAD"]),
            if cfg!(debug_assertions) {
                "debug (not a release throughput result)"
            } else {
                "release"
            },
            shell("uname", &["-sm"]),
            shell("sysctl", &["-n", "machdep.cpu.brand_string"])
        );
        report.push_str("One round per language/workload/transport on a shared machine. Idle: two workers for eight seconds. Burst: 300 real runs. Backpressure: 32 tasks of two seconds, maxInflight eight. RPC/s and throughput include runner startup/shutdown. Delivery latency is durable step.released/retry_due→public handler entry; it includes database discovery, admission, network delivery, and SDK dispatch. It subtracts the local PostgreSQL container's epoch timestamp from the runner's epoch timestamp, so every percentile carries the clocks' unknown offset. Results stay signed and unclamped; negative samples are reported as count/minimum and are measurement uncertainty, not a transport failure. Ready→lease is the durable server-side portion and is not relabeled as client delivery. Poll claim attempts equal PollTasks calls (one claim operation per request); stream claim attempts come from worker_task_stream_claims_total. Peak RSS samples the runner process tree every 50 ms and excludes the server. Peak local in-flight is measured at handler entry/exit. Slow rows show heartbeat progress under handler-capacity backpressure; admission-saturation progress is covered separately by the held-stream server test.\n\n");
        report.push_str("| SDK | Workload | Transport | Wall s | Receive RPCs | RPC/s | Claim attempts | Completions/s | Delivery p50/p95/p99 ms | Negative count/min ms | Ready→lease p95 ms | Peak local | Peak leases | Peak RSS MiB | Heartbeats OK/refused |\n|---|---|---|---:|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|\n");
        for language in languages {
            for workload in ["idle", "burst", "slow"] {
                for transport in ["poll", "stream"] {
                    println!("--- streaming comparison {language}/{workload}/{transport}");
                    let measured = harness::measure(&fixture(workload, transport, &dir), language)
                        .await
                        .expect("measurement");
                    let observations = measured.out["observations"]
                        .as_array()
                        .expect("observations");
                    assert!(
                        observations.iter().all(|o| o["ok"] == true),
                        "{language}/{workload}/{transport}: failed observation"
                    );
                    let completed = observations
                        .iter()
                        .filter(|o| {
                            o["op"] == "waitForCompletion" && o["result"]["status"] == "SUCCEEDED"
                        })
                        .count();
                    if workload != "idle" {
                        assert!(completed > 0);
                    }
                    let rpc = measured
                        .calls
                        .get(if transport == "poll" {
                            "PollTasks"
                        } else {
                            "StreamTasks"
                        })
                        .copied()
                        .unwrap_or(0);
                    assert!(rpc > 0, "requested transport was not exercised");
                    let other = measured
                        .calls
                        .get(if transport == "poll" {
                            "StreamTasks"
                        } else {
                            "PollTasks"
                        })
                        .copied()
                        .unwrap_or(0);
                    assert_eq!(other, 0, "explicit transport silently changed");
                    assert!(
                        measured.peak_leases <= if workload == "slow" { 8 } else { 64 },
                        "lease bound"
                    );
                    assert!(measured.peak_rss_kib > 0, "runner RSS was not sampled");
                    if workload != "idle" {
                        assert_eq!(
                            measured.delivery_latency_ms.len(),
                            completed,
                            "{language}/{workload}/{transport}: every completed task needs a delivery timestamp"
                        );
                        assert!(
                            measured.peak_local_inflight > 0,
                            "no handler entry measured"
                        );
                    }
                    let heartbeats = measured.calls.get("Heartbeat").copied().unwrap_or(0);
                    let heartbeat_refused: u64 = measured
                        .refused
                        .iter()
                        .filter(|(key, _)| key.starts_with("Heartbeat:"))
                        .map(|(_, count)| *count)
                        .sum();
                    let heartbeat_ok = heartbeats.saturating_sub(heartbeat_refused);
                    if workload == "slow" {
                        assert!(
                            heartbeat_ok > 0,
                            "saturated workload completed no successful heartbeat: {:?}",
                            measured.refused
                        );
                    }
                    let duration = measured.wall.as_secs_f64();
                    let negative_samples: Vec<f64> = measured
                        .delivery_latency_ms
                        .iter()
                        .copied()
                        .filter(|sample| *sample < 0.0)
                        .collect();
                    let negative_summary = negative_samples
                        .iter()
                        .copied()
                        .reduce(f64::min)
                        .map_or_else(
                            || "—".to_owned(),
                            |minimum| format!("{}/{minimum:.2}", negative_samples.len()),
                        );
                    let claim_attempts = if transport == "poll" {
                        rpc as f64
                    } else {
                        measured.stream_claim_attempts
                    };
                    let _ = writeln!(
                        report,
                        "| {language} | {workload} | {transport} | {duration:.3} | {rpc} | {:.2} | {claim_attempts:.0} | {:.2} | {} | {negative_summary} | {} | {} | {} | {:.2} | {heartbeat_ok}/{heartbeat_refused} |",
                        rpc as f64 / duration,
                        completed as f64 / duration,
                        percentiles(&measured.delivery_latency_ms),
                        percentile_or_dash(&measured.ready_to_lease_ms, 0.95),
                        measured.peak_local_inflight,
                        measured.peak_leases,
                        measured.peak_rss_kib as f64 / 1024.0,
                    );
                    // Preserve completed rows if a later runtime or environment fails.
                    let output = std::env::var_os("SAGAT_STREAM_PERF_REPORT")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| dir.join("report.md"));
                    std::fs::write(output, &report).expect("write report");
                }
            }
        }
        println!("{report}");
    });
}

//! US04 — a hostile script is held to its limits, its network, its secrets, and what it tried is
//! audited (`FR-005-004`, `NFR-005-SEC`). Every scenario runs as a step of a real workflow, through
//! the worker protocol, in a real container.

use reqwest::Method;
use sagat_domain::scripts::{Egress, Policy};
use serde_json::{Value, json};

use crate::support::api::Options;
use crate::support::scripts::{Lab, lab, lab_with};

/// A platform whose scripts may ask for the network (the namespace's egress policy still decides).
async fn lab_with_egress() -> Lab {
    let lab = lab_with(Options {
        script_ceiling: Policy {
            egress: Egress::Namespace,
            ..Policy::default()
        },
        ..Options::default()
    })
    .await;
    let (status, body) = lab
        .call(
            Method::PUT,
            &format!("/namespaces/{}/egress-policy", lab.ns),
            Some(&json!({"rules": ["api.example.test"], "ports": [443], "allowHttp": false})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    lab
}

async fn step(
    lab: &Lab,
    name: &str,
    language: &str,
    source: &str,
    policy: Value,
) -> (String, String) {
    let script = lab
        .script_with(name, language, source, json!({"policy": policy}))
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab
        .workflow(&format!("wf-{name}"), &digest, json!({}))
        .await;
    let run = lab.p.start(&workflow, &json!({})).await;
    (run, digest)
}

#[tokio::test]
async fn a_script_that_reaches_for_the_cloud_metadata_service_is_refused_and_it_is_audited() {
    let lab = lab_with_egress().await;
    let (run, digest) = step(
        &lab,
        "metadata-thief",
        "typescript",
        "export default async function (_i: unknown, octopus: any) {\n  const out: Record<string, string> = {};\n  for (const url of ['http://169.254.169.254/latest/meta-data/?token=SECRET123', 'http://metadata.google.internal/computeMetadata/v1/']) {\n    try { await octopus.fetch(url); out[url] = 'reached'; } catch (e) { out[url] = String((e as Error).message); }\n  }\n  try { await fetch('http://169.254.169.254/', { signal: AbortSignal.timeout(2000) }); out.direct = 'reached'; } catch { out.direct = 'unreachable'; }\n  return out;\n}\n",
        json!({"egress": "namespace"}),
    )
    .await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    let out = lab.output(&run, "s").await.expect("output");
    assert_eq!(
        out["direct"], "unreachable",
        "no route from the container: {out}"
    );
    for (url, verdict) in out.as_object().expect("object") {
        if url != "direct" {
            assert_eq!(verdict, "denied", "{url}: {out}");
        }
    }
    let executions = lab.executions(&digest).await;
    assert_eq!(
        executions[0]["denied"],
        json!([
            "fetch http://169.254.169.254",
            "fetch http://metadata.google.internal"
        ]),
        "the destination and nothing after it: {executions:?}"
    );
    assert!(!executions[0].to_string().contains("SECRET123"));
    let audited = lab.audited("script.violation").await;
    assert_eq!(
        audited,
        vec![
            "fetch http://169.254.169.254".to_owned(),
            "fetch http://metadata.google.internal".to_owned()
        ]
    );
}

#[tokio::test]
async fn a_script_that_does_not_catch_the_refusal_fails_its_step_without_a_retry() {
    let lab = lab_with_egress().await;
    let (run, _) = step(
        &lab,
        "uncaught",
        "python",
        "def run(input, octopus):\n    octopus.fetch('http://169.254.169.254/latest/meta-data/')\n    return {'reached': True}\n",
        json!({"egress": "namespace"}),
    )
    .await;
    assert_eq!(lab.drive(&run).await, "FAILED");
    assert_eq!(
        lab.failure(&run, "s").await.as_deref(),
        Some("SCRIPT_DENIED")
    );
}

#[tokio::test]
async fn a_script_without_network_policy_has_no_way_out_even_when_the_namespace_allows_one() {
    // The namespace allows api.example.test, but the script did not ask for the network.
    let lab = lab_with_egress().await;
    let (run, digest) = step(
        &lab,
        "no-network",
        "typescript",
        "export default async function (_i: unknown, octopus: any) {\n  try { await octopus.fetch('https://api.example.test/'); return { reached: true }; }\n  catch (e) { return { reached: false, why: String((e as Error).message) }; }\n}\n",
        json!({}),
    )
    .await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    assert_eq!(
        lab.output(&run, "s").await,
        Some(json!({"reached": false, "why": "denied"}))
    );
    assert_eq!(
        lab.executions(&digest).await[0]["denied"],
        json!(["fetch https://api.example.test"])
    );
}

#[tokio::test]
async fn a_cpu_burner_is_stopped_at_its_time_limit_and_the_step_fails_with_the_code() {
    let lab = lab().await;
    let began = std::time::Instant::now();
    let (run, digest) = step(
        &lab,
        "burner",
        "typescript",
        "export default function () { for (;;) {} }\n",
        json!({"timeoutMs": 3000}),
    )
    .await;
    assert_eq!(lab.drive(&run).await, "FAILED");
    assert_eq!(
        lab.failure(&run, "s").await.as_deref(),
        Some("SCRIPT_TIMEOUT")
    );
    assert!(began.elapsed() < std::time::Duration::from_secs(60));
    let executions = lab.executions(&digest).await;
    assert_eq!(executions[0]["failure"], "SCRIPT_TIMEOUT");
    assert_eq!(executions[0]["status"], "FAILED");
}

#[tokio::test]
async fn a_memory_bomb_is_killed_and_nothing_is_left_running() {
    let lab = lab().await;
    let (run, _) = step(
        &lab,
        "bomb",
        "python",
        "def run(input, octopus):\n    hog = []\n    while True:\n        hog.append(bytearray(8 * 1024 * 1024))\n",
        json!({"memoryMib": 64, "timeoutMs": 30000}),
    )
    .await;
    assert_eq!(lab.drive(&run).await, "FAILED");
    assert_eq!(
        lab.failure(&run, "s").await.as_deref(),
        Some("SCRIPT_OUT_OF_MEMORY")
    );
    assert_eq!(
        lab.runner.sandbox().sweep(std::time::Duration::ZERO).await,
        0,
        "no container is left behind"
    );
}

#[tokio::test]
async fn a_script_that_floods_its_output_is_cut_off() {
    let lab = lab().await;
    let (run, digest) = step(
        &lab,
        "flood",
        "typescript",
        "export default function () { const l = 'x'.repeat(1000); for (let i = 0; i < 3_000_000; i++) console.log(l); }\n",
        json!({"timeoutMs": 20000}),
    )
    .await;
    assert_eq!(lab.drive(&run).await, "FAILED");
    let executions = lab.executions(&digest).await;
    assert_eq!(executions[0]["logTruncated"], true);
    assert!(executions[0]["log"].as_str().expect("log").len() <= 64 * 1024);
}

#[tokio::test]
async fn a_secret_the_step_declared_reaches_the_script_and_nothing_else_does_and_it_cannot_leave_through_the_log_or_the_result()
 {
    let lab = lab().await;
    lab.secret("payments-api-key", "pk_live_0123456789abcdefghij")
        .await;
    lab.secret("unrelated-secret", "other-0123456789abcdefghij")
        .await;
    let script = lab
        .script(
            "uses-secret",
            "typescript",
            "export default async function (_i: unknown, octopus: any) {\n  const key = await octopus.secret('payments-api-key');\n  console.log('calling with', key);\n  let other = 'none';\n  try { other = await octopus.secret('unrelated-secret'); } catch (e) { other = String((e as Error).message); }\n  return { echo: key, other, length: key.length };\n}\n",
        )
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab
        .workflow(
            "wf-secret",
            &digest,
            json!({"secrets": ["payments-api-key"]}),
        )
        .await;
    let run = lab.p.start(&workflow, &json!({})).await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    let out = lab.output(&run, "s").await.expect("output");
    assert_eq!(
        out["other"], "denied",
        "an undeclared secret is refused: {out}"
    );
    assert_eq!(out["length"], 28, "the script really had it: {out}");
    let everything = format!("{out} {:?}", lab.executions(&digest).await);
    assert!(
        !everything.contains("pk_live_"),
        "the secret left the sandbox: {everything}"
    );
    assert!(!everything.contains("other-0123456789"), "{everything}");
    assert!(everything.contains("[REDACTED]"));
    let audited = lab.audited("script.violation").await;
    assert_eq!(audited, vec!["secret unrelated-secret".to_owned()]);
}

#[tokio::test]
async fn a_script_sees_nothing_of_the_host_and_runs_as_nobody() {
    let lab = lab().await;
    let (run, _) = step(
        &lab,
        "snoop",
        "python",
        "import os\ndef run(input, octopus):\n    return {\n        'docker_sock': os.path.exists('/var/run/docker.sock'),\n        'host_users': os.path.exists('/Users'),\n        'uid': os.getuid(),\n        'env': sorted(k for k in os.environ if k.startswith(('SAGAT', 'DATABASE', 'AWS'))),\n        'cwd_writable': os.access('/', os.W_OK),\n    }\n",
        json!({}),
    )
    .await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    assert_eq!(
        lab.output(&run, "s").await,
        Some(
            json!({"docker_sock": false, "host_users": false, "uid": 65534, "env": [], "cwd_writable": false})
        )
    );
}

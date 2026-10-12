//! Failure and repeat cases for US03. These scenarios compose the 009 egress policy with the real
//! 005 runner and Docker sandbox (`T009-022`).

use std::time::Duration;

use reqwest::Method;
use sagat_application::worker::WorkerSettings;
use sagat_domain::scripts::{Egress, Policy};
use sagat_sandbox_runner::StepOutcome;
use serde_json::{Value, json};

use crate::support::api::Options;
use crate::support::scripts::{Lab, lab_with};

const FAST_RETRY: &str = r#"{"maxAttempts": 3, "initialDelayMs": 1, "maxDelayMs": 1, "strategy": "fixed", "jitter": false}"#;

async fn egress_lab(options: Options, rules: &[&str]) -> Lab {
    let lab = lab_with(Options {
        script_ceiling: Policy {
            egress: Egress::Namespace,
            ..options.script_ceiling
        },
        ..options
    })
    .await;
    let (status, body) = lab
        .call(
            Method::PUT,
            &format!("/namespaces/{}/egress-policy", lab.ns),
            Some(&json!({"rules": rules, "ports": [443], "allowHttp": false})),
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
    retry: Option<Value>,
) -> (String, String) {
    let script = lab
        .script_with(name, language, source, json!({"policy": policy}))
        .await;
    let digest = lab.ready(&script).await;
    let mut node = json!({});
    if let Some(retry) = retry {
        node["retry"] = retry;
    }
    let workflow = lab.workflow(&format!("wf-{name}"), &digest, node).await;
    (lab.p.start(&workflow, &json!({})).await, digest)
}

#[tokio::test]
async fn duplicate_refusals_are_answered_each_time_but_kept_and_audited_once() {
    let lab = egress_lab(Options::default(), &["api.example.test"]).await;
    let (run, digest) = step(
        &lab,
        "repeat-denial",
        "typescript",
        "export default async function (_i: unknown, octopus: any) {\n\
         const answers: string[] = [];\n\
         for (let i = 0; i < 3; i++) {\n\
           try { await octopus.fetch('https://blocked.example.test/private'); }\n\
           catch (e) { answers.push(String((e as Error).message)); }\n\
         }\n\
         return answers;\n\
         }\n",
        json!({"egress": "namespace"}),
        None,
    )
    .await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    assert_eq!(
        lab.output(&run, "s").await,
        Some(json!(["denied", "denied", "denied"]))
    );
    assert_eq!(
        lab.executions(&digest).await[0]["denied"],
        json!(["fetch https://blocked.example.test"])
    );
    assert_eq!(
        lab.audited("script.violation").await,
        vec!["NOT_ALLOWLISTED".to_owned()]
    );
}

#[tokio::test]
async fn a_denial_before_the_script_times_out_is_still_recorded_and_the_container_is_removed() {
    let lab = egress_lab(Options::default(), &["api.example.test"]).await;
    let (run, digest) = step(
        &lab,
        "denial-then-timeout",
        "python",
        r#"def run(input, octopus):
    try:
        octopus.fetch('http://169.254.169.254/latest/meta-data/')
    except Exception:
        pass
    while True:
        pass
"#,
        json!({"egress": "namespace", "timeoutMs": 2000}),
        None,
    )
    .await;
    assert_eq!(lab.drive(&run).await, "FAILED");
    assert_eq!(
        lab.failure(&run, "s").await.as_deref(),
        Some("SCRIPT_TIMEOUT")
    );
    let execution = &lab.executions(&digest).await[0];
    assert_eq!(execution["failure"], "SCRIPT_TIMEOUT");
    assert_eq!(execution["denied"], json!(["fetch http://169.254.169.254"]));
    assert_eq!(
        lab.audited("script.violation").await,
        vec!["UNSUPPORTED_SCHEME".to_owned()]
    );
    assert_eq!(lab.runner.sandbox().leftovers().await, 0);
}

#[tokio::test]
async fn a_runner_lost_before_start_is_replaced_and_only_the_fenced_attempt_records_the_denial() {
    let options = Options {
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(5),
            ..WorkerSettings::default()
        },
        ..Options::default()
    };
    let mut lab = egress_lab(options, &["api.example.test"]).await;
    let retry = serde_json::from_str(FAST_RETRY).expect("retry json");
    let (run, digest) = step(
        &lab,
        "denial-after-takeover",
        "typescript",
        "export default async function (_i: unknown, octopus: any) {\n\
           await octopus.fetch('https://blocked.example.test/');\n\
         }\n",
        json!({"egress": "namespace"}),
        Some(retry),
    )
    .await;
    lab.p.tick().await;
    let namespace = lab.runner.config().namespaces[0].clone();
    let lost = lab.runner.poll_steps(&namespace).await.expect("poll");
    assert_eq!(lost.len(), 1, "the first runner leased the step");
    drop(lost);
    lab.restart().await;
    lab.p.expire_lease(&run, "s").await;
    lab.p.tick().await;
    lab.p.elapse_retry_delay(&run, "s").await;

    assert_eq!(lab.drive(&run).await, "FAILED");
    assert_eq!(
        lab.failure(&run, "s").await.as_deref(),
        Some("SCRIPT_DENIED")
    );
    let executions = lab.executions(&digest).await;
    assert_eq!(executions.len(), 1, "only the attempt that ran is recorded");
    assert_eq!(
        executions[0]["denied"],
        json!(["fetch https://blocked.example.test"])
    );
    assert_eq!(
        lab.audited("script.violation").await,
        vec!["NOT_ALLOWLISTED".to_owned()]
    );
}

#[tokio::test]
async fn another_tenants_allowlist_never_authorizes_the_callers_script() {
    let lab = egress_lab(Options::default(), &["acme.example.test"]).await;
    let other_workspace = lab
        .p
        .api
        .create_workspace("bob", "globex", "other-platform")
        .await;
    let other_namespace = lab
        .p
        .api
        .create_namespace("bob", "globex", &other_workspace, "other", "dev")
        .await;
    let response = lab
        .p
        .api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{other_namespace}/egress-policy"),
            "bob",
            "globex",
        )
        .header("x-octopus-namespace", "other")
        .json(&json!({
            "rules": ["globex.example.test"],
            "ports": [443],
            "allowHttp": false
        }))
        .send()
        .await
        .expect("store the other tenant's policy");
    assert_eq!(response.status(), 200);

    let (run, digest) = step(
        &lab,
        "cross-tenant-policy",
        "python",
        r#"def run(input, octopus):
    try:
        octopus.fetch('https://globex.example.test/private')
        return {'reached': True}
    except Exception as error:
        return {'reached': False, 'why': str(error)}
"#,
        json!({"egress": "namespace"}),
        None,
    )
    .await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    assert_eq!(
        lab.output(&run, "s").await,
        Some(json!({"reached": false, "why": "denied"}))
    );
    assert_eq!(
        lab.executions(&digest).await[0]["denied"],
        json!(["fetch https://globex.example.test"])
    );
    let acme_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = 'script.violation'",
    )
    .bind(lab.p.tenant.as_uuid())
    .fetch_one(&lab.p.api.db.pool)
    .await
    .expect("acme audits");
    let globex_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = 'script.violation'",
    )
    .bind(lab.p.other_tenant.as_uuid())
    .fetch_one(&lab.p.api.db.pool)
    .await
    .expect("globex audits");
    assert_eq!((acme_audits, globex_audits), (1, 0));
}

#[tokio::test]
async fn revoking_egress_while_a_script_is_running_applies_to_its_next_host_call() {
    let options = Options {
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(10),
            ..WorkerSettings::default()
        },
        script_ceiling: Policy {
            limits: sagat_domain::scripts::Limits::new(
                256 * 1024 * 1024,
                1000,
                Duration::from_secs(30),
                64,
                64 * 1024,
                32 * 1024 * 1024,
            )
            .expect("limits"),
            ..Policy::default()
        },
        ..Options::default()
    };
    let lab = egress_lab(options, &["revoked.example.invalid"]).await;
    let (run, digest) = step(
        &lab,
        "live-policy",
        "typescript",
        r#"export default async function (_i: unknown, octopus: any) {
  await new Promise(resolve => setTimeout(resolve, 3000));
  try {
    await octopus.fetch('https://revoked.example.invalid/private');
    return { verdict: 'reached' };
  } catch (error) {
    return { verdict: String((error as Error).message) };
  }
}
"#,
        json!({"egress": "namespace", "timeoutMs": 10000}),
        None,
    )
    .await;
    lab.p.tick().await;
    let namespace = lab.runner.config().namespaces[0].clone();
    let step = lab
        .runner
        .poll_steps(&namespace)
        .await
        .expect("poll")
        .remove(0);
    let runner = std::sync::Arc::clone(&lab.runner);
    let running = tokio::spawn(async move { runner.run_step(step).await });
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (status, body) = lab
        .call(
            Method::PUT,
            &format!("/namespaces/{}/egress-policy", lab.ns),
            Some(&json!({
                "rules": ["still-allowed.example.invalid"],
                "ports": [443],
                "allowHttp": false
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(20), running)
            .await
            .expect("script ends")
            .expect("no panic")
            .expect("runner reports"),
        StepOutcome::Completed
    );
    lab.p.tick().await;
    assert_eq!(lab.p.status(&run).await, "SUCCEEDED");
    assert_eq!(
        lab.output(&run, "s").await,
        Some(json!({"verdict": "denied"})),
        "the policy is read at the host call, after the revocation"
    );
    assert_eq!(
        lab.executions(&digest).await[0]["denied"],
        json!(["fetch https://revoked.example.invalid"])
    );
    assert_eq!(
        lab.audited("script.violation").await,
        vec!["NOT_ALLOWLISTED".to_owned()]
    );
}

#[tokio::test]
async fn a_denial_is_audited_before_a_worker_can_lose_its_lease() {
    let options = Options {
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(3),
            ..WorkerSettings::default()
        },
        script_ceiling: Policy {
            limits: sagat_domain::scripts::Limits::new(
                256 * 1024 * 1024,
                1000,
                Duration::from_secs(60),
                64,
                64 * 1024,
                32 * 1024 * 1024,
            )
            .expect("limits"),
            ..Policy::default()
        },
        ..Options::default()
    };
    let lab = egress_lab(options, &["api.example.test"]).await;
    let (run, _) = step(
        &lab,
        "audit-before-loss",
        "typescript",
        r#"export default async function (_i: unknown, octopus: any) {
  try { await octopus.fetch('https://blocked.example.test/private'); } catch {}
  await new Promise(resolve => setTimeout(resolve, 30000));
  return { late: true };
}
"#,
        json!({"egress": "namespace", "timeoutMs": 45000}),
        None,
    )
    .await;
    lab.p.tick().await;
    let namespace = lab.runner.config().namespaces[0].clone();
    let step = lab
        .runner
        .poll_steps(&namespace)
        .await
        .expect("poll")
        .remove(0);
    let runner = std::sync::Arc::clone(&lab.runner);
    let running = tokio::spawn(async move { runner.run_step(step).await });
    let incident_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let incidents = lab.audited("script.violation").await;
        if incidents.len() == 1 && incidents[0] == "NOT_ALLOWLISTED" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < incident_deadline,
            "the denial was not durably audited before the lease-loss checkpoint"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    lab.p.expire_lease(&run, "s").await;
    lab.p.tick().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), running)
            .await
            .expect("the runner notices the loss")
            .expect("no panic")
            .expect("runner result"),
        StepOutcome::Abandoned
    );
    assert_eq!(
        lab.audited("script.violation").await,
        vec!["NOT_ALLOWLISTED".to_owned()],
        "the incident must survive even though no execution record can be completed"
    );
    for _ in 0..50 {
        if lab.runner.sandbox().leftovers().await == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(lab.runner.sandbox().leftovers().await, 0);
}

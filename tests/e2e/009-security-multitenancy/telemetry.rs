//! The security events of feature 009 are visible to an operator as metrics, and the metrics name
//! nobody (NFR-009-OBS): what was refused is a label from a closed set; who asked, which tenant and
//! which secret are never labels.

use reqwest::Method;
use sagat_domain::identity::Role;
use sagat_telemetry::Metrics;
use serde_json::json;

use crate::support::worker::{Worker, platform};

const CANARY: &str = "sk-live-metrics-canary-77d2e1b0";

/// The sum of every series of `name` whose line carries all of `labels` (a counter with no labels is
/// matched by an empty list).
fn total(page: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    page.lines()
        .filter(|line| {
            line.starts_with(name)
                && matches!(line[name.len()..].chars().next(), Some('{' | ' '))
                && labels
                    .iter()
                    .all(|(k, v)| line.contains(&format!("{k}=\"{v}\"")))
        })
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

#[tokio::test]
async fn refusals_and_secret_handling_are_counted_without_naming_anyone() {
    // The recorder is process-wide: install it before anything counts. Other tests of this binary
    // run in the same process and count too, so every check below is "at least one more than
    // before", and the series are read from the same page.
    let metrics = Metrics::global();
    let p = platform().await;
    let ns = p.namespace_id().await;
    let (status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/metered-key"),
            Some(&json!({"value": CANARY})),
        )
        .await;
    assert_eq!(status, 200);
    let wf = p
        .publish(
            "metered-flow",
            &json!({
                "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
                "metadata": {"name": "metered-flow"}, "entrypoint": "a",
                "nodes": [{"id": "a", "type": "activity", "activity": "act.a",
                           "secrets": ["metered-key"]}]
            }),
        )
        .await;
    let before = metrics.render();

    // A full namespace: the refusal names the limit.
    let (status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/quotas/active_runs"),
            Some(&json!({"limit": 1})),
        )
        .await;
    assert_eq!(status, 200);
    let run = p.start(&wf, &json!({})).await;
    let (status, refused) = p
        .rest(
            Method::POST,
            "/runs",
            Some(&json!({"workflowId": wf, "input": {}})),
        )
        .await;
    assert_eq!(status, 429, "{refused}");

    // Another tenant reaches for the run.
    let globex_ws = p.api.create_workspace("bob", "globex", "platform").await;
    p.api
        .create_namespace("bob", "globex", &globex_ws, "payments", "dev")
        .await;
    let foreign = p
        .api
        .as_user(Method::GET, &format!("/runs/{run}"), "bob", "globex")
        .header("x-octopus-namespace", "payments")
        .send()
        .await
        .expect("send");
    assert_eq!(foreign.status(), 404);

    // A worker nobody bound.
    p.grant(p.tenant, "stray", Role::Worker).await;
    let mut stray = p.connect("stray", crate::support::worker::TENANT).await;
    let request = stray.req(Worker::register_message(&["act.a"], 10));
    stray
        .client
        .register_worker(request)
        .await
        .expect_err("not bound");

    // A secret handed out, then found in an output.
    let mut worker = p.worker("wendy").await;
    worker.register(&["act.a"]).await;
    let lease = worker.one_task().await;
    let secrets = worker.resolve_secrets(&lease).await.expect("resolve");
    assert_eq!(secrets.len(), 1);
    worker
        .complete(&lease, &json!({"debug": CANARY}))
        .await
        .expect("accepted as a failure");

    // A secret that is gone before the next step asks for it.
    let (status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/quotas/active_runs"),
            Some(&json!({"limit": 5})),
        )
        .await;
    assert_eq!(status, 200);
    let second = p.start(&wf, &json!({})).await;
    let lease = worker.one_task().await;
    assert_eq!(lease.run_id, second);
    let (status, _) = p
        .rest_as(
            "alice",
            Method::DELETE,
            &format!("/namespaces/{ns}/secrets/metered-key"),
            None,
        )
        .await;
    assert_eq!(status, 204);
    assert!(worker.resolve_secrets(&lease).await.is_err());

    let after = metrics.render();
    for (name, labels) in [
        (
            "quota_exceeded_total",
            vec![("limit", "active runs in the namespace")],
        ),
        (
            "security_denials_total",
            vec![("action", "access.denied.cross_tenant")],
        ),
        ("security_denials_total", vec![("action", "worker.denied")]),
        ("secrets_handed_out_total", vec![]),
        ("secret_output_detected_total", vec![]),
        ("secret_resolution_denied_total", vec![]),
    ] {
        assert!(
            total(&after, name, &labels) >= total(&before, name, &labels) + 1.0,
            "{name} {labels:?} did not move:\n{after}"
        );
    }

    // Nobody is named: no tenant, subject, run, namespace or secret in any label or name.
    for forbidden in [
        CANARY,
        "metered-key",
        "bob",
        "alice",
        "wendy",
        "stray",
        "globex",
        &ns.to_string(),
        &run,
        &p.tenant.to_string(),
    ] {
        assert!(
            !after.contains(forbidden),
            "the metrics page names {forbidden:?}"
        );
    }
}

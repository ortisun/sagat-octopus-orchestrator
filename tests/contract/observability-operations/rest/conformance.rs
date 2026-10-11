use std::collections::BTreeSet;

use sagat_domain::identity::Role;
use sagat_scheduler::{Monitor, MonitorSettings};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::contract::{As, OpenApi, Scenario, check, check_body, scenario};
use crate::support::worker::{Platform, platform, single_ir};

fn key() -> String {
    format!("key-{}", Uuid::now_v7())
}

fn with_key(s: Scenario, namespace: &str) -> Scenario {
    s.with_header("x-octopus-namespace", namespace)
        .with_header("idempotency-key", key())
}

fn retrying() -> Value {
    single_ir(
        "payment",
        Some(
            json!({"maxAttempts": 3, "strategy": "fixed", "initialDelayMs": 3_600_000, "maxDelayMs": 3_600_000, "jitter": false}),
        ),
    )
}

/// A platform with an alert firing, a run with an attempt that failed and waits for its retry, a
/// worker that holds a lease, and a run that ended.
async fn world() -> (Platform, String, String, String) {
    let p = platform().await;
    p.bind_everything("worker-1").await;
    p.grant(p.tenant, "vera", Role::Viewer).await;
    p.grant(p.tenant, "olga", Role::Operator).await;
    let workflow = p.publish("payment", &retrying()).await;
    let waiting = p.start(&workflow, &json!({"n": 1})).await;
    let mut worker = p.worker("worker-1").await;
    worker.register(&["act.a"]).await;
    let lease = worker.one_task().await;
    worker.fail(&lease, true).await.expect("fails");
    // An alert: a step nobody takes.
    let other = p.publish("unserved", &single_ir("unserved", None)).await;
    p.start(&other, &json!({})).await;
    sqlx::query(
        "UPDATE steps SET ready_at = clock_timestamp() - interval '1 hour' WHERE status = 'READY'",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("time passes");
    Monitor::new(p.api.executions.clone(), MonitorSettings::default())
        .run_once()
        .await;
    // A run that ended.
    let ended_workflow = p.publish("short", &single_ir("short", None)).await;
    let ended = p.start(&ended_workflow, &json!({"n": 2})).await;
    let task = worker.poll(5).await;
    for lease in task.iter().filter(|l| l.run_id == ended) {
        worker.complete(lease, &json!({})).await.expect("complete");
    }
    let ns = p.namespace_id().await;
    (p, waiting, ended, ns)
}

#[tokio::test]
async fn the_operations_of_feature_008_conform_to_the_contract_on_their_normal_and_error_paths() {
    let spec = OpenApi::load();
    let (p, waiting, ended, ns) = world().await;
    let alice = || As::User("alice", "acme");
    let vera = || As::User("vera", "acme");
    let olga = || As::User("olga", "acme");
    let bob = || As::User("bob", "globex");
    let alerts = format!("/namespaces/{ns}/alerts");
    let workers = format!("/namespaces/{ns}/workers");
    let drain = format!("/namespaces/{ns}/workers/worker-1/drain");
    let undrain = format!("/namespaces/{ns}/workers/worker-1/undrain");
    let ops = |run: &str| format!("/runs/{run}/operations");
    let get = |template: &'static str, path: &str, caller: As, expect: u16| {
        scenario("GET", template, path, caller, None, expect)
    };

    let scenarios = vec![
        // Alerts.
        get("/namespaces/{namespaceId}/alerts", &alerts, vera(), 200),
        get(
            "/namespaces/{namespaceId}/alerts",
            &format!("{alerts}?state=FIRING&limit=1"),
            vera(),
            200,
        ),
        get(
            "/namespaces/{namespaceId}/alerts",
            &format!("{alerts}?state=SLEEPING"),
            vera(),
            422,
        ),
        get(
            "/namespaces/{namespaceId}/alerts",
            &format!("{alerts}?cursor=nope"),
            vera(),
            400,
        ),
        get("/namespaces/{namespaceId}/alerts", &alerts, bob(), 404),
        get(
            "/namespaces/{namespaceId}/alerts",
            &alerts,
            As::Anonymous,
            401,
        ),
        // The state of a run's telemetry, and its attempts.
        get(
            "/runs/{runId}/telemetry",
            &format!("/runs/{waiting}/telemetry"),
            vera(),
            200,
        ),
        get(
            "/runs/{runId}/telemetry",
            &format!("/runs/{}/telemetry", Uuid::now_v7()),
            vera(),
            404,
        ),
        get(
            "/runs/{runId}/telemetry",
            &format!("/runs/{waiting}/telemetry"),
            bob(),
            404,
        ),
        get(
            "/runs/{runId}/attempts",
            &format!("/runs/{waiting}/attempts"),
            vera(),
            200,
        ),
        get(
            "/runs/{runId}/attempts",
            &format!("/runs/{}/attempts", Uuid::now_v7()),
            vera(),
            404,
        ),
        get(
            "/runs/{runId}/events",
            &format!("/runs/{waiting}/events"),
            vera(),
            200,
        ),
        // Workers.
        get("/namespaces/{namespaceId}/workers", &workers, vera(), 200),
        get("/namespaces/{namespaceId}/workers", &workers, bob(), 404),
        scenario(
            "POST",
            "/namespaces/{namespaceId}/workers/{subject}/drain",
            &drain,
            vera(),
            Some(json!({"reason": "deploying a new version"})),
            403,
        ),
        scenario(
            "POST",
            "/namespaces/{namespaceId}/workers/{subject}/drain",
            &drain,
            olga(),
            Some(json!({"reason": "no"})),
            422,
        ),
        scenario(
            "POST",
            "/namespaces/{namespaceId}/workers/{subject}/drain",
            &drain,
            olga(),
            Some(json!({"why": "unknown field of the request"})),
            422,
        ),
        scenario(
            "POST",
            "/namespaces/{namespaceId}/workers/{subject}/drain",
            &drain,
            bob(),
            Some(json!({"reason": "deploying a new version"})),
            404,
        ),
        scenario(
            "POST",
            "/namespaces/{namespaceId}/workers/{subject}/drain",
            &drain,
            olga(),
            Some(json!({"reason": "deploying a new version"})),
            200,
        ),
        get("/namespaces/{namespaceId}/workers", &workers, vera(), 200),
        scenario(
            "POST",
            "/namespaces/{namespaceId}/workers/{subject}/undrain",
            &undrain,
            vera(),
            Some(json!({"reason": "the new version is out"})),
            403,
        ),
        scenario(
            "POST",
            "/namespaces/{namespaceId}/workers/{subject}/undrain",
            &undrain,
            olga(),
            Some(json!({"reason": "the new version is out"})),
            200,
        ),
        // The audit trail.
        get("/audit", "/audit", alice(), 200),
        get(
            "/audit",
            "/audit?action=worker.drained&limit=1",
            alice(),
            200,
        ),
        get("/audit", "/audit?from=yesterday", alice(), 422),
        get("/audit", "/audit?cursor=nope", alice(), 400),
        get("/audit", "/audit", olga(), 403),
        get("/audit", "/audit", As::Garbage, 401),
        // What operateRun gained.
        with_key(
            scenario(
                "POST",
                "/runs/{runId}/operations",
                &ops(&waiting),
                olga(),
                Some(
                    json!({"action": "MARK_MANUAL_REVIEW", "stepId": "a", "reason": "the provider asked us to wait"}),
                ),
                202,
            ),
            "payments",
        ),
        with_key(
            scenario(
                "POST",
                "/runs/{runId}/operations",
                &ops(&waiting),
                olga(),
                Some(
                    json!({"action": "MARK_MANUAL_REVIEW", "stepId": "a", "reason": "holding it a second time"}),
                ),
                409,
            ),
            "payments",
        ),
        with_key(
            scenario(
                "POST",
                "/runs/{runId}/operations",
                &ops(&waiting),
                olga(),
                Some(json!({"action": "MARK_MANUAL_REVIEW", "reason": "no step is named here"})),
                422,
            ),
            "payments",
        ),
        with_key(
            scenario(
                "POST",
                "/runs/{runId}/operations",
                &ops(&waiting),
                olga(),
                Some(json!({"action": "RERUN_AS_NEW", "reason": "this run has not ended"})),
                409,
            ),
            "payments",
        ),
        with_key(
            scenario(
                "POST",
                "/runs/{runId}/operations",
                &ops(&ended),
                vera(),
                Some(json!({"action": "RERUN_AS_NEW", "reason": "a viewer may not do this"})),
                403,
            ),
            "payments",
        ),
        with_key(
            scenario(
                "POST",
                "/runs/{runId}/operations",
                &ops(&ended),
                olga(),
                Some(json!({"action": "RERUN_AS_NEW", "reason": "start it again please"})),
                202,
            ),
            "payments",
        ),
        with_key(
            scenario(
                "POST",
                "/runs/{runId}/operations",
                &ops(&waiting),
                olga(),
                Some(
                    json!({"action": "RETRY_STEP", "stepId": "a", "reason": "the provider is back up"}),
                ),
                202,
            ),
            "payments",
        ),
    ];
    let mut exercised: BTreeSet<(String, String, u16)> = BTreeSet::new();
    for s in &scenarios {
        // `check` needs the idempotency headers some scenarios carry; the others carry none.
        exercised.insert(check(&p.api, &spec, s).await);
    }

    // Everything feature 008 added succeeds somewhere.
    let added = spec.operations_marked("x-implemented", "008");
    assert_eq!(added.len(), 7, "{added:?}");
    for (method, template) in &added {
        assert!(
            exercised
                .iter()
                .any(|(m, t, st)| m == method && t == template && matches!(*st, 200 | 202)),
            "{method} {template} has no successful conformance scenario"
        );
    }
    // And what it extended says so.
    for (method, template) in spec.operations_marked("x-extended-again", "008") {
        assert!(
            exercised
                .iter()
                .any(|(m, t, st)| *m == method && *t == template && *st == 202),
            "{method} {template}"
        );
    }
}

#[tokio::test]
async fn every_error_says_what_to_do_about_it_and_never_names_data() {
    let spec = OpenApi::load();
    let (p, waiting, _, ns) = world().await;
    let bad: Vec<(Scenario, u16)> = vec![
        (
            scenario(
                "GET",
                "/namespaces/{namespaceId}/alerts",
                &format!("/namespaces/{ns}/alerts"),
                As::User("bob", "globex"),
                None,
                404,
            ),
            404,
        ),
        (
            scenario(
                "GET",
                "/namespaces/{namespaceId}/alerts",
                &format!("/namespaces/{ns}/alerts?state=SLEEPING"),
                As::User("alice", "acme"),
                None,
                422,
            ),
            422,
        ),
        (
            scenario("GET", "/audit", "/audit", As::Anonymous, None, 401),
            401,
        ),
        (
            scenario(
                "GET",
                "/audit",
                "/audit",
                As::User("vera", "acme"),
                None,
                403,
            ),
            403,
        ),
        (
            with_key(
                scenario(
                    "POST",
                    "/runs/{runId}/operations",
                    &format!("/runs/{waiting}/operations"),
                    As::User("alice", "acme"),
                    Some(json!({"action": "RERUN_AS_NEW", "reason": "this run has not ended"})),
                    409,
                ),
                "payments",
            ),
            409,
        ),
    ];
    for (scenario, status) in &bad {
        let (_, _, got, body) = check_body(&p.api, &spec, scenario).await;
        assert_eq!(got, *status);
        let remediation = body["remediation"].as_str().unwrap_or_default();
        assert!(remediation.len() > 20, "{status}: {body}");
        // A fixed sentence from a closed table: nothing of the request in it.
        let text = body.to_string();
        assert!(
            !text.contains(&waiting) && !text.contains(&ns.to_string()),
            "{text}"
        );
    }
    // The same code with another reason is told something else.
    let reused = {
        let first = with_key(
            scenario(
                "POST",
                "/runs",
                "/runs",
                As::User("alice", "acme"),
                Some(json!({"workflowId": Uuid::now_v7().to_string(), "input": {}})),
                404,
            ),
            "payments",
        );
        check_body(&p.api, &spec, &first).await.3
    };
    assert!(reused["remediation"].as_str().is_some());
}

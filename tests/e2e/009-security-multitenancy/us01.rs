//! US01: tenant A, with a valid token, tries to reach tenant B's runs: reading, manipulating and
//! discovering that they exist are all refused (FR-009-001, NFR-009-SEC). The REST side of this is
//! attacked operation by operation in `tests/contract/009-security-multitenancy/rest/battery.rs`;
//! this is the same story through the worker protocol, with a lease token that leaked.

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::worker::{Worker, machine_code, platform};

const CANARY: &str = "sk-live-globex-only-8a1c5e9f";

fn flow(name: &str, secrets: &[&str]) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "a",
        "nodes": [{"id": "a", "type": "activity", "activity": "act.a", "secrets": secrets}]
    })
}

#[tokio::test]
async fn a_leaked_lease_token_of_another_tenant_opens_nothing() {
    let p = platform().await;

    // Globex has a secret, a workflow that uses it, a worker and a leased step.
    let globex_ws = p.api.create_workspace("bob", "globex", "platform").await;
    let globex_ns = p
        .api
        .create_namespace("bob", "globex", &globex_ws, "payments", "dev")
        .await;
    let as_bob = |method: Method, path: &str, body: Option<&Value>| {
        let mut request = p
            .api
            .as_user(method, path, "bob", "globex")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", format!("key-{}", uuid::Uuid::now_v7()));
        if let Some(body) = body {
            request = request.json(body);
        }
        request
    };
    let put = as_bob(
        Method::PUT,
        &format!("/namespaces/{globex_ns}/secrets/payments-key"),
        Some(&json!({"value": CANARY})),
    )
    .send()
    .await
    .expect("send");
    assert_eq!(put.status(), 200);
    let created: Value = as_bob(
        Method::POST,
        "/workflows",
        Some(&json!({"name": "globex-flow", "ir": flow("globex-flow", &["payments-key"])})),
    )
    .send()
    .await
    .expect("send")
    .json()
    .await
    .expect("json");
    let wf = created["id"].as_str().expect("id").to_owned();
    assert_eq!(
        as_bob(
            Method::POST,
            &format!("/workflows/{wf}/publish"),
            Some(&json!({"draftRevision": 1}))
        )
        .send()
        .await
        .expect("send")
        .status(),
        201
    );
    let run: Value = as_bob(
        Method::POST,
        "/runs",
        Some(&json!({"workflowId": wf, "input": {}})),
    )
    .send()
    .await
    .expect("send")
    .json()
    .await
    .expect("json");
    let run = run["runId"].as_str().expect("run").to_owned();
    // Globex's own worker, bound to its own namespace.
    p.grant(p.other_tenant, "gina", Role::Worker).await;
    sqlx::query(
        "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by) \
         SELECT n.tenant_id, n.id, 'gina', '*', 'test' FROM namespaces n \
         WHERE n.tenant_id = $1 AND n.slug = 'payments'",
    )
    .bind(p.other_tenant.as_uuid())
    .execute(&p.api.db.pool)
    .await
    .expect("bind gina");
    let mut gina: Worker<'_> = p.connect("gina", "globex").await;
    gina.register(&["act.a"]).await;
    let lease = gina.one_task().await;
    let own = gina
        .resolve_secrets(&lease)
        .await
        .expect("globex's own worker");
    assert_eq!(own[0].value, CANARY.as_bytes());

    // Acme's worker is bound and valid in acme. It has globex's run id, step id and the REAL lease
    // token. Every call is refused exactly as a stale attempt would be.
    let mut wendy = p.worker("wendy").await;
    wendy.register(&["act.a"]).await;
    let stolen = lease.clone();
    let resolved = wendy.resolve_secrets(&stolen).await.expect_err("refused");
    assert_eq!(resolved.code(), tonic::Code::Aborted);
    assert_eq!(machine_code(&resolved), "STALE_ATTEMPT");
    assert!(!resolved.message().contains(CANARY));

    let mut beat = Worker::heartbeat_message(&wendy, &stolen);
    beat.worker_id = wendy.worker_id.clone();
    let heartbeat = {
        let request = wendy.req(beat);
        wendy.client.heartbeat(request).await.expect_err("refused")
    };
    assert_eq!(machine_code(&heartbeat), "STALE_ATTEMPT");
    let completed = wendy
        .complete(&stolen, &json!({"hijacked": true}))
        .await
        .expect_err("refused");
    assert_eq!(machine_code(&completed), "STALE_ATTEMPT");
    let failed = wendy.fail(&stolen, false).await.expect_err("refused");
    assert_eq!(machine_code(&failed), "STALE_ATTEMPT");

    // The answer is the same as for a run that does not exist at all.
    let mut nowhere = stolen.clone();
    nowhere.run_id = uuid::Uuid::now_v7().to_string();
    // A task id is `run/node/generation`: keep the lease consistent so that the only difference is
    // that this run exists nowhere.
    nowhere.task_id = format!("{}/a/{}", nowhere.run_id, nowhere.generation);
    let missing = wendy.resolve_secrets(&nowhere).await.expect_err("refused");
    assert_eq!(
        (missing.code(), machine_code(&missing), missing.message()),
        (resolved.code(), machine_code(&resolved), resolved.message())
    );

    // Globex's step is untouched: its worker finishes it normally.
    gina.complete(&lease, &json!({"charged": true}))
        .await
        .expect("globex's worker is unaffected");
    let status: Value = as_bob(Method::GET, &format!("/runs/{run}"), None)
        .send()
        .await
        .expect("send")
        .json()
        .await
        .expect("json");
    assert_eq!(status["status"], "SUCCEEDED");

    // The attempts are on record against acme, and the value reached nothing of acme's.
    crate::support::eventually("the attempts are audited", || async {
        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records a JOIN tenants t ON t.id = a.tenant_id \
             WHERE t.slug = 'acme' AND a.action = 'access.denied.cross_tenant'",
        )
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count");
        (rows >= 1).then_some(())
    })
    .await;
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records a JOIN tenants t ON t.id = a.tenant_id \
         WHERE t.slug = 'acme' AND strpos(a::text, $1) > 0",
    )
    .bind(CANARY)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("count");
    assert_eq!(leaked, 0);
}

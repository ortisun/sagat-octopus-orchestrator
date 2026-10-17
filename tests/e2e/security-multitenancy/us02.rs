//! US02: an author with access to a staging secret publishes a workflow; the worker receives only a
//! temporary capability and the authorized secrets (FR-009-002, FR-009-003, NFR-009-SEC).

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::worker::{machine_code, platform};

const CANARY: &str = "sk-live-4eC39HqLyjWDarjtT1zdp7dc";

async fn publish_as_dora(p: &crate::support::worker::Platform, id: &str) -> (u16, Value) {
    p.rest_as(
        "dora",
        Method::POST,
        &format!("/workflows/{id}/publish"),
        Some(&json!({"draftRevision": 1})),
    )
    .await
}

fn charge_ir(name: &str, secrets: &[&str]) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "a",
        "nodes": [{"id": "a", "type": "activity", "activity": "act.a", "secrets": secrets}]
    })
}

#[tokio::test]
async fn an_author_with_access_publishes_and_the_worker_receives_only_the_authorized_secret() {
    let p = platform().await;
    let ns = p.namespace_id().await;
    p.grant(p.tenant, "dora", Role::Developer).await;

    // The administrator stores two secrets and lets dora use only one of them.
    for (name, value) in [
        ("stripe-key", CANARY),
        ("other-key", "not-for-this-workflow-0001"),
    ] {
        let (status, body) = p
            .rest(
                Method::PUT,
                &format!("/namespaces/{ns}/secrets/{name}"),
                Some(&json!({"value": value})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert!(body.get("value").is_none(), "{body}");
    }

    // Without being on the list, dora cannot publish a workflow that names the secret.
    let (status, created) = p
        .rest_as(
            "dora",
            Method::POST,
            "/workflows",
            Some(&json!({"name": "charge-flow", "ir": charge_ir("charge-flow", &["stripe-key"])})),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().expect("id").to_owned();
    let (status, refused) = publish_as_dora(&p, &id).await;
    assert_eq!(status, 403, "{refused}");
    assert_eq!(refused["code"], "FORBIDDEN");
    assert_eq!(refused["details"]["reason"], "SECRET_NOT_USABLE");

    // Put on the list, she can.
    let (status, _) = p
        .rest_as(
            "alice",
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/stripe-key/access/dora"),
            None,
        )
        .await;
    assert_eq!(status, 204);
    let (status, published) = publish_as_dora(&p, &id).await;
    assert_eq!(status, 201, "{published}");

    // A worker takes the task and asks for the secrets of its step.
    let run = p.start(&id, &json!({})).await;
    let mut worker = p.worker("wendy").await;
    worker.register(&["act.a"]).await;
    let lease = worker.one_task().await;
    let secrets = worker.resolve_secrets(&lease).await.expect("resolve");
    assert_eq!(secrets.len(), 1, "only what the step declares");
    assert_eq!(secrets[0].name, "stripe-key");
    assert_eq!(secrets[0].value, CANARY.as_bytes());
    assert_eq!(secrets[0].version, 1);
    assert!(
        !format!("{secrets:?}").contains(CANARY),
        "the generated message never prints a value"
    );
    assert!(
        !lease.input_json.contains(CANARY),
        "the lease itself carries no secret: {}",
        lease.input_json
    );

    // The capability is the lease: another token gets nothing.
    let mut forged = lease.clone();
    forged.lease_token = "not-the-token".into();
    let denied = worker.resolve_secrets(&forged).await.expect_err("stale");
    assert_eq!(denied.code(), tonic::Code::Aborted);
    assert_eq!(machine_code(&denied), "STALE_ATTEMPT");

    // The step finishes; the capability ends with the attempt.
    worker
        .complete(&lease, &json!({"charged": true}))
        .await
        .expect("complete");
    let after = worker.resolve_secrets(&lease).await.expect_err("over");
    assert_eq!(machine_code(&after), "STALE_ATTEMPT");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
}

#[tokio::test]
async fn the_secret_reaches_no_response_log_row_or_error_on_the_way() {
    let p = platform().await;
    // Every log line of the whole flow, at every level, from the API and the worker service alike
    // (they run in this test's runtime).
    let (_logging, logs) = crate::support::logs::capture_at(tracing::Level::TRACE);
    let ns = p.namespace_id().await;
    let (status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/stripe-key"),
            Some(&json!({"value": CANARY})),
        )
        .await;
    assert_eq!(status, 200);
    let id = p
        .publish("charge-flow", &charge_ir("charge-flow", &["stripe-key"]))
        .await;
    let run = p.start(&id, &json!({})).await;
    let mut worker = p.worker("wendy").await;
    worker.register(&["act.a"]).await;
    let lease = worker.one_task().await;
    let secrets = worker.resolve_secrets(&lease).await.expect("resolve");
    assert_eq!(secrets[0].value, CANARY.as_bytes());

    // A careless activity echoes its secret back: the platform fails the step instead of storing it.
    let done = worker
        .complete(&lease, &json!({"debug": format!("using {CANARY}")}))
        .await
        .expect("accepted as a failure");
    assert_eq!(done.step_status, "FAILED");
    assert_eq!(p.status(&run).await, "FAILED");

    // Everything the platform logged: first prove that the capture sees this flow at all.
    tracing::warn!("capture check");
    let logged = logs.text();
    assert!(
        logged.contains("capture check"),
        "the capture is not working"
    );
    assert!(
        !logged.contains(CANARY),
        "a log line carried the value: {}",
        logged
            .lines()
            .filter(|l| l.contains(CANARY))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Everything the platform keeps about the run, and everything it said about it.
    let journal = p
        .rest(Method::GET, &format!("/runs/{run}/events"), None)
        .await
        .1;
    let graph = p
        .rest(Method::GET, &format!("/runs/{run}/graph"), None)
        .await
        .1;
    let run_body = p.run(&run).await;
    for (what, body) in [("journal", journal), ("graph", graph), ("run", run_body)] {
        assert!(!body.to_string().contains(CANARY), "{what}: {body}");
    }
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname = 'public'")
            .fetch_all(&p.api.db.pool)
            .await
            .expect("tables");
    for table in tables {
        let sql = format!("SELECT count(*) FROM \"{table}\" x WHERE strpos(x::text, $1) > 0");
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(CANARY)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("count");
        assert_eq!(count, 0, "the value is in {table}");
    }
}

#[tokio::test]
async fn without_a_binding_a_worker_cannot_even_register_and_the_status_says_why() {
    let p = platform().await;
    // A worker identity that holds the role but whose administrator never bound it.
    p.grant(p.tenant, "stray", Role::Worker).await;
    let mut stray = p.connect("stray", crate::support::worker::TENANT).await;
    let request = stray.req(crate::support::worker::Worker::register_message(
        &["act.a"],
        10,
    ));
    let status = stray
        .client
        .register_worker(request)
        .await
        .expect_err("not bound");
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
    assert_eq!(machine_code(&status), "NOT_BOUND");
}

#[tokio::test]
async fn a_secret_restricted_to_a_worker_pool_is_leased_and_resolved_only_by_workers_of_that_pool()
{
    let p = platform().await;
    let ns = p.namespace_id().await;
    let (status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/stripe-key"),
            Some(&json!({"value": CANARY})),
        )
        .await;
    assert_eq!(status, 200);
    let (status, set) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/stripe-key/pools"),
            Some(&json!({"pools": ["trusted"]})),
        )
        .await;
    assert_eq!((status, set), (200, json!({"pools": ["trusted"]})));
    let (_, listed) = p
        .rest(Method::GET, &format!("/namespaces/{ns}/secrets"), None)
        .await;
    assert_eq!(listed["items"][0]["pools"], json!(["trusted"]));
    // Someone who may merely use the secret is not told which fleets are trusted with it.
    p.grant(p.tenant, "dora", Role::Developer).await;
    let (status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/stripe-key/access/dora"),
            None,
        )
        .await;
    assert_eq!(status, 204);
    let (status, seen) = p
        .rest_as(
            "dora",
            Method::GET,
            &format!("/namespaces/{ns}/secrets"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{seen}");
    assert_eq!(seen["items"][0]["name"], "stripe-key");
    assert!(seen["items"][0].get("pools").is_none(), "{seen}");
    let id = p
        .publish("charge-flow", &charge_ir("charge-flow", &["stripe-key"]))
        .await;
    let run = p.start(&id, &json!({})).await;

    // A worker bound to the activity but granted no pool is handed nothing, however often it asks.
    let mut outsider = p.worker("oscar").await;
    outsider.register(&["act.a"]).await;
    for _ in 0..3 {
        assert!(outsider.poll(8).await.is_empty());
    }
    assert_eq!(p.status(&run).await, "RUNNING");

    // An administrator puts another identity in the pool; it takes the same step and the value.
    let (status, bound) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/worker-bindings/wendy"),
            Some(&json!({"activities": ["*"], "pools": ["trusted"]})),
        )
        .await;
    assert_eq!(status, 200, "{bound}");
    assert_eq!(bound["pools"], json!(["trusted"]));
    let mut insider = p.worker("wendy").await;
    insider.register(&["act.a"]).await;
    let lease = insider.one_task().await;
    let secrets = insider.resolve_secrets(&lease).await.expect("resolve");
    assert_eq!(secrets[0].value, CANARY.as_bytes());

    // Taking the pool away stops the lease it holds from yielding the value again.
    let (status, _) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/worker-bindings/wendy"),
            Some(&json!({"activities": ["*"], "pools": []})),
        )
        .await;
    assert_eq!(status, 200);
    let refused = insider
        .resolve_secrets(&lease)
        .await
        .expect_err("left the pool");
    assert_eq!(refused.code(), tonic::Code::PermissionDenied);
    assert_eq!(machine_code(&refused), "SECRET_NOT_USABLE");
    assert!(!refused.message().contains(CANARY));

    // The refusals are on the trail, once per worker identity and stage, without the value.
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT actor, reason FROM audit_records WHERE action = 'secret.delivery_refused' \
         ORDER BY actor, reason",
    )
    .fetch_all(&p.api.db.pool)
    .await
    .expect("audit");
    let actors: Vec<_> = rows.iter().map(|(a, _)| a.as_str()).collect();
    assert_eq!(actors, ["oscar", "wendy"], "{rows:?}");
    assert!(rows.iter().all(|(_, r)| !r.contains(CANARY)));
}

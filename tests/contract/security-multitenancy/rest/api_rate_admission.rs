//! Live HTTP proof for durable namespace API admission (FR-009-006, T009-040).

use reqwest::{Method, StatusCode};
use sagat_domain::identity::{Membership, NamespaceId, Role, TenantId};
use serde_json::{Value, json};

use crate::support::api::{Options, TestApi, start, start_with};
use crate::support::{now, subject};

const MICROTOKENS: i64 = 1_000_000;

async fn world() -> (TestApi, TenantId, String) {
    let api = start().await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "platform").await;
    let namespace = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    (api, tenant, namespace)
}

async fn set_rate(api: &TestApi, namespace: &str, rate: u64) {
    let response = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/quotas/api_requests_per_second"),
            "alice",
            "acme",
        )
        .json(&json!({ "limit": rate }))
        .send()
        .await
        .expect("set API rate");
    assert_eq!(response.status(), StatusCode::OK);
    sqlx::query(
        "DELETE FROM quota_rate_buckets WHERE namespace_id = $1 \
         AND resource = 'api_requests_per_second'",
    )
    .bind(
        namespace
            .parse::<NamespaceId>()
            .expect("namespace id")
            .as_uuid(),
    )
    .execute(&api.db.pool)
    .await
    .expect("reset bucket");
}

async fn seed_bucket(api: &TestApi, tenant: TenantId, namespace: &str, rate: u64) {
    let tokens = i64::try_from(rate).expect("bounded test rate") * 2 * MICROTOKENS;
    sqlx::query(
        "INSERT INTO quota_rate_buckets \
             (tenant_id, namespace_id, resource, source_kind, source_id, microtokens, updated_at) \
         VALUES ($1, $2, 'api_requests_per_second', 'NAMESPACE', $3, $4, \
                 clock_timestamp() + interval '1 hour') \
         ON CONFLICT (tenant_id, namespace_id, resource, source_kind, source_id) \
         DO UPDATE SET microtokens = EXCLUDED.microtokens, updated_at = EXCLUDED.updated_at",
    )
    .bind(tenant.as_uuid())
    .bind(
        namespace
            .parse::<NamespaceId>()
            .expect("namespace id")
            .as_uuid(),
    )
    .bind(uuid::Uuid::nil())
    .bind(tokens)
    .execute(&api.db.pool)
    .await
    .expect("seed bucket");
}

async fn tokens(api: &TestApi, tenant: TenantId, namespace: &str) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT microtokens FROM quota_rate_buckets \
         WHERE tenant_id = $1 AND namespace_id = $2 \
           AND resource = 'api_requests_per_second' \
           AND source_kind = 'NAMESPACE' AND source_id = $3",
    )
    .bind(tenant.as_uuid())
    .bind(
        namespace
            .parse::<NamespaceId>()
            .expect("namespace id")
            .as_uuid(),
    )
    .bind(uuid::Uuid::nil())
    .fetch_optional(&api.db.pool)
    .await
    .expect("read bucket")
}

async fn get_at(api: &TestApi, base: &str, path: &str) -> reqwest::Response {
    api.client
        .get(format!("{base}{path}"))
        .bearer_auth(api.idp.token("alice", "acme").build())
        .send()
        .await
        .expect("GET")
}

fn workflow_named(name: &str) -> Value {
    json!({
        "name": name,
        "ir": {
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": { "name": name },
            "entrypoint": "a",
            "nodes": [{ "id": "a", "type": "activity", "activity": "test.echo" }]
        }
    })
}

fn workflow() -> Value {
    workflow_named("rated-flow")
}

async fn create_and_publish_workflow(api: &TestApi, name: &str) -> String {
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", uuid::Uuid::now_v7().to_string())
        .json(&workflow_named(name))
        .send()
        .await
        .expect("create workflow");
    assert_eq!(created.status(), StatusCode::CREATED);
    let body = created.json::<Value>().await.expect("workflow");
    let workflow = body["id"].as_str().expect("workflow id").to_owned();
    let revision = body["draftRevision"].as_u64().expect("draft revision");
    let published = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            "alice",
            "acme",
        )
        .header("idempotency-key", uuid::Uuid::now_v7().to_string())
        .json(&json!({ "draftRevision": revision }))
        .send()
        .await
        .expect("publish workflow");
    assert_eq!(published.status(), StatusCode::CREATED);
    workflow
}

async fn start_run(api: &TestApi, workflow: &str) -> String {
    let response = api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", uuid::Uuid::now_v7().to_string())
        .json(&json!({ "workflowId": workflow, "input": {} }))
        .send()
        .await
        .expect("start run");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    response.json::<Value>().await.expect("run")["runId"]
        .as_str()
        .expect("run id")
        .to_owned()
}

#[tokio::test]
async fn two_http_servers_share_one_durable_namespace_bucket() {
    let (mut api, _tenant, namespace) = world().await;
    set_rate(&api, &namespace, 1).await;
    let replica = api.start_http_replica().await;

    assert_eq!(
        api.get_as(&format!("/namespaces/{namespace}"), "alice", "acme")
            .await
            .status(),
        StatusCode::OK
    );
    // Tenant-wide inventory is deliberately exempt and cannot spend the namespace's last token.
    assert_eq!(
        get_at(&api, &replica, "/namespaces").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        get_at(&api, &replica, &format!("/namespaces/{namespace}"))
            .await
            .status(),
        StatusCode::OK
    );
    let refused = api
        .get_as(&format!("/namespaces/{namespace}"), "alice", "acme")
        .await;
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(refused.headers()["retry-after"], "1");
}

#[tokio::test]
async fn quota_policy_routes_can_recover_a_namespace_from_zero_rate() {
    let (api, _tenant, namespace) = world().await;
    set_rate(&api, &namespace, 0).await;
    assert_eq!(
        api.get_as(&format!("/namespaces/{namespace}"), "alice", "acme")
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );

    // The policy route keeps its quota:manage authorization and canonical namespace lookup, but
    // cannot debit the bucket it is the recovery control for.
    set_rate(&api, &namespace, 1).await;
    assert_eq!(
        api.get_as(&format!("/namespaces/{namespace}"), "alice", "acme")
            .await
            .status(),
        StatusCode::OK
    );
    set_rate(&api, &namespace, 0).await;
    let reset = api
        .as_user(
            Method::DELETE,
            &format!("/namespaces/{namespace}/quotas/api_requests_per_second"),
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("reset API rate");
    assert_eq!(reset.status(), StatusCode::OK);
    assert_eq!(
        api.get_as(&format!("/namespaces/{namespace}"), "alice", "acme")
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn malformed_denied_and_foreign_requests_do_not_debit_a_namespace() {
    let (api, tenant, namespace) = world().await;
    set_rate(&api, &namespace, 1).await;
    api.owner_store
        .insert_membership(&Membership::restore(
            tenant,
            subject("vera"),
            Role::Viewer,
            now(),
        ))
        .await
        .expect("viewer");

    let malformed = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("content-type", "application/json")
        .body("{")
        .send()
        .await
        .expect("malformed request");
    assert!(malformed.status().is_client_error());
    let denied = api
        .as_user(Method::POST, "/workflows", "vera", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", uuid::Uuid::now_v7().to_string())
        .json(&workflow())
        .send()
        .await
        .expect("denied request");
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(tokens(&api, tenant, &namespace).await, None);

    let foreign_tenant = api.tenant("globex", "bob").await;
    let foreign_workspace = api.create_workspace("bob", "globex", "platform").await;
    let foreign_namespace = api
        .create_namespace("bob", "globex", &foreign_workspace, "billing", "dev")
        .await;
    let foreign = api
        .get_as(&format!("/namespaces/{foreign_namespace}"), "alice", "acme")
        .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    assert_eq!(tokens(&api, tenant, &namespace).await, None);
    assert_eq!(tokens(&api, foreign_tenant, &foreign_namespace).await, None);

    // The two-token burst is intact, proving every refusal above was non-debiting.
    for _ in 0..2 {
        assert_eq!(
            api.get_as(&format!("/namespaces/{namespace}"), "alice", "acme")
                .await
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        api.get_as(&format!("/namespaces/{namespace}"), "alice", "acme")
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn a_success_and_a_later_conflict_each_consume_exactly_one_token() {
    let (api, tenant, namespace) = world().await;
    set_rate(&api, &namespace, 10).await;
    seed_bucket(&api, tenant, &namespace, 10).await;

    let send = || {
        api.as_user(Method::POST, "/workflows", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", uuid::Uuid::now_v7().to_string())
            .json(&workflow())
            .send()
    };
    assert_eq!(send().await.expect("create").status(), StatusCode::CREATED);
    assert_eq!(
        send().await.expect("conflict").status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        tokens(&api, tenant, &namespace).await,
        Some(18 * MICROTOKENS)
    );
}

#[tokio::test]
async fn refused_upload_returns_before_a_streaming_body_is_consumed() {
    let api = start_with(Options {
        blobs: Some(sagat_application::blobs::BlobLimits::default()),
        ..Options::default()
    })
    .await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "platform").await;
    let namespace = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    set_rate(&api, &namespace, 0).await;

    let malformed = api
        .as_user(Method::POST, "/run-input-blobs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("x-octopus-blob-sha256", "not-a-sha256-digest")
        .body("unused")
        .send()
        .await
        .expect("malformed upload header");
    assert_eq!(malformed.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(tokens(&api, tenant, &namespace).await, None);

    // The source never yields EOF. A handler that starts consuming it before admission hangs;
    // admission-first returns the durable 429 from headers alone.
    let never = futures_util::stream::pending::<Result<bytes::Bytes, std::io::Error>>();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        api.as_user(Method::POST, "/run-input-blobs", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .header("content-type", "application/octet-stream")
            .body(reqwest::Body::wrap_stream(never))
            .send(),
    )
    .await
    .expect("admission answered without waiting for the body")
    .expect("upload response");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn nested_entity_url_namespace_mismatch_does_not_debit_either_namespace() {
    let (api, tenant, namespace) = world().await;
    let workspaces = api.get_as("/workspaces", "alice", "acme").await;
    assert_eq!(workspaces.status(), StatusCode::OK);
    let workspace = workspaces.json::<Value>().await.expect("workspaces")["items"][0]["id"]
        .as_str()
        .expect("workspace id")
        .to_owned();
    let other = api
        .create_namespace("alice", "acme", &workspace, "billing", "dev")
        .await;
    let created = api
        .as_user(
            Method::POST,
            &format!("/namespaces/{namespace}/scripts"),
            "alice",
            "acme",
        )
        .json(&json!({
            "name": "owned-script",
            "language": "typescript",
            "source": "export default () => 1"
        }))
        .send()
        .await
        .expect("create script");
    assert_eq!(created.status(), StatusCode::CREATED);
    let script = created.json::<Value>().await.expect("script")["id"]
        .as_str()
        .expect("script id")
        .to_owned();

    for ns in [&namespace, &other] {
        set_rate(&api, ns, 10).await;
        seed_bucket(&api, tenant, ns, 10).await;
    }
    let mismatch = api
        .get_as(
            &format!("/namespaces/{other}/scripts/{script}"),
            "alice",
            "acme",
        )
        .await;
    assert_eq!(mismatch.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        tokens(&api, tenant, &namespace).await,
        Some(20 * MICROTOKENS)
    );
    assert_eq!(tokens(&api, tenant, &other).await, Some(20 * MICROTOKENS));
}

#[tokio::test]
async fn an_open_sse_stream_debits_once_and_a_reconnect_debits_once_more() {
    let (api, tenant, namespace) = world().await;
    let workflow = create_and_publish_workflow(&api, "stream-rate").await;
    let run = start_run(&api, &workflow).await;
    set_rate(&api, &namespace, 10).await;
    seed_bucket(&api, tenant, &namespace, 10).await;

    let stream = api
        .as_user(Method::GET, &format!("/runs/{run}/stream"), "alice", "acme")
        .header("accept", "text/event-stream")
        .send()
        .await
        .expect("open stream");
    assert_eq!(stream.status(), StatusCode::OK);
    assert_eq!(
        tokens(&api, tenant, &namespace).await,
        Some(19 * MICROTOKENS)
    );
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    assert_eq!(
        tokens(&api, tenant, &namespace).await,
        Some(19 * MICROTOKENS),
        "stream polls must not debit the API bucket"
    );
    drop(stream);

    let reconnect = api
        .as_user(Method::GET, &format!("/runs/{run}/stream"), "alice", "acme")
        .header("accept", "text/event-stream")
        .header("last-event-id", "0")
        .send()
        .await
        .expect("reconnect stream");
    assert_eq!(reconnect.status(), StatusCode::OK);
    assert_eq!(
        tokens(&api, tenant, &namespace).await,
        Some(18 * MICROTOKENS)
    );
}

#[tokio::test]
async fn promotion_debits_only_the_destination_namespace() {
    let (api, tenant, source) = world().await;
    let workspaces = api
        .get_as("/workspaces", "alice", "acme")
        .await
        .json::<Value>()
        .await
        .expect("workspaces");
    let workspace = workspaces["items"][0]["id"].as_str().expect("workspace id");
    let destination = api
        .create_namespace("alice", "acme", workspace, "staging", "staging")
        .await;
    let workflow = create_and_publish_workflow(&api, "promoted-flow").await;
    for namespace in [&source, &destination] {
        set_rate(&api, namespace, 10).await;
        seed_bucket(&api, tenant, namespace, 10).await;
    }

    let response = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/versions/1/promote"),
            "alice",
            "acme",
        )
        .header("idempotency-key", uuid::Uuid::now_v7().to_string())
        .json(&json!({
            "namespaceId": destination,
            "reason": "verified in the source environment"
        }))
        .send()
        .await
        .expect("promote");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(tokens(&api, tenant, &source).await, Some(20 * MICROTOKENS));
    assert_eq!(
        tokens(&api, tenant, &destination).await,
        Some(19 * MICROTOKENS)
    );
}

#[tokio::test]
async fn script_runner_completion_is_exempt_when_namespace_api_rate_is_zero() {
    let (api, tenant, namespace) = world().await;
    api.owner_store
        .insert_membership(&Membership::restore(
            tenant,
            subject("runner-one"),
            Role::Runner,
            now(),
        ))
        .await
        .expect("runner membership");
    let created = api
        .as_user(
            Method::POST,
            &format!("/namespaces/{namespace}/scripts"),
            "alice",
            "acme",
        )
        .json(&json!({
            "name": "quota-runner",
            "language": "typescript",
            "source": "export default () => 1"
        }))
        .send()
        .await
        .expect("create script");
    assert_eq!(created.status(), StatusCode::CREATED);
    let script = created.json::<Value>().await.expect("script")["id"]
        .as_str()
        .expect("script id")
        .to_owned();
    let publish = api
        .as_user(
            Method::POST,
            &format!("/namespaces/{namespace}/scripts/{script}/versions"),
            "alice",
            "acme",
        )
        .send()
        .await
        .expect("publish script");
    assert_eq!(publish.status(), StatusCode::ACCEPTED);
    let claim = api
        .as_user(
            Method::POST,
            "/script-runner/builds/claim",
            "runner-one",
            "acme",
        )
        .json(&json!({
            "toolchains": { "typescript": "node@sha256:aa;shim=1" }
        }))
        .send()
        .await
        .expect("claim build");
    assert_eq!(claim.status(), StatusCode::OK);
    let job = claim.json::<Value>().await.expect("job");
    set_rate(&api, &namespace, 0).await;

    let completed = api
        .as_user(
            Method::POST,
            &format!(
                "/script-runner/builds/{}/complete",
                job["jobId"].as_str().expect("job id")
            ),
            "runner-one",
            "acme",
        )
        .json(&json!({
            "leaseToken": job["leaseToken"],
            "outcome": "failed",
            "kind": "rejected",
            "message": "source did not expose the required entrypoint"
        }))
        .send()
        .await
        .expect("complete build");
    assert_eq!(completed.status(), StatusCode::NO_CONTENT);
    assert_eq!(tokens(&api, tenant, &namespace).await, None);
}

#[tokio::test]
async fn admission_database_failure_fails_closed_before_the_handler_runs() {
    let api = start_with(Options {
        statement_timeout: std::time::Duration::from_millis(100),
        ..Options::default()
    })
    .await;
    let _tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "platform").await;
    let namespace = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    // Warm authentication before inducing an admission-only database timeout.
    assert_eq!(
        api.get_as("/namespaces", "alice", "acme").await.status(),
        StatusCode::OK
    );
    let mut blocker = api.db.pool.begin().await.expect("blocking transaction");
    sqlx::query("LOCK TABLE quota_rate_buckets IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .expect("lock admission buckets");

    let response = api
        .get_as(&format!("/namespaces/{namespace}"), "alice", "acme")
        .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    blocker.rollback().await.expect("release admission table");
}

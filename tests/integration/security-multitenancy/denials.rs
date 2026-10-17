//! Gap G9, decided on 2026-10-16: every permission-denied response (`403` over REST,
//! `PERMISSION_DENIED` on the worker API) leaves one audit record, bounded by the per-subject rate
//! limit, without changing what the caller is told.

use reqwest::Method;
use sagat_api_http::RateLimitConfig;
use sagat_domain::identity::Role;
use sagat_telemetry::Metrics;
use serde_json::{Value, json};
use tonic::Code;

use crate::support::api::Options;
use crate::support::worker::{HUMAN, NAMESPACE, Platform, TENANT, platform, platform_with};

const DENIED: &str = "access.denied.permission";

type Row = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

fn workflow(name: &str) -> Value {
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

/// A member of acme with the given role.
async fn member(p: &Platform, sub: &str, role: Role) {
    p.grant(p.tenant, sub, role).await;
}

/// The permission-denied records of acme: actor, action, resource type, resource id, reason, trace.
async fn denials(p: &Platform) -> Vec<Row> {
    sqlx::query_as(
        "SELECT actor, action, resource_type, resource_id, reason, trace_id FROM audit_records \
         WHERE tenant_id = $1 AND action = $2 ORDER BY created_at, id",
    )
    .bind(p.tenant.as_uuid())
    .bind(DENIED)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("denial records")
}

async fn count_all_denials(p: &Platform) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = $1")
        .bind(DENIED)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count")
}

/// A viewer asks to create a workflow, which needs `definition:write`.
async fn viewer_creates_workflow(p: &Platform, sub: &str) -> reqwest::Response {
    p.api
        .as_user(Method::POST, "/workflows", sub, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", uuid::Uuid::now_v7().to_string())
        .json(&workflow("denied-flow"))
        .send()
        .await
        .expect("denied request")
}

#[tokio::test]
async fn a_viewer_calling_a_developer_endpoint_leaves_exactly_one_record_with_its_context() {
    let p = platform().await;
    member(&p, "vera", Role::Viewer).await;

    let response = viewer_creates_workflow(&p, "vera").await;

    assert_eq!(response.status(), 403);
    let trace = response
        .headers()
        .get("x-trace-id")
        .expect("trace header")
        .to_str()
        .expect("ascii")
        .to_owned();
    let body: Value = response.json().await.expect("envelope");
    assert_eq!(body["code"], "FORBIDDEN");
    assert_eq!(
        body["details"],
        json!({ "requiredPermission": "definition:write" })
    );
    // Written before the answer is sent, so there is nothing to wait for.
    assert_eq!(
        denials(&p).await,
        vec![(
            "vera".to_owned(),
            DENIED.to_owned(),
            "endpoint".to_owned(),
            "POST /v1/workflows".to_owned(),
            Some("missing permission definition:write".to_owned()),
            Some(trace),
        )]
    );
}

#[tokio::test]
async fn the_record_names_the_route_template_and_never_the_path_the_caller_chose() {
    let p = platform().await;
    member(&p, "vera", Role::Viewer).await;
    let (status, created) = p
        .rest(Method::POST, "/workflows", Some(&workflow("target")))
        .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().expect("workflow id");

    let (status, body) = p
        .rest_as(
            "vera",
            Method::POST,
            &format!("/workflows/{id}/publish?token=sekret-query"),
            Some(&json!({ "draftRevision": 1 })),
        )
        .await;

    assert_eq!(status, 403, "{body}");
    let rows = denials(&p).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].3, "POST /v1/workflows/{workflow_id}/publish");
    let stored = format!("{rows:?}");
    assert!(
        !stored.contains(id) && !stored.contains("sekret-query"),
        "the record must not carry the path, the query or an identifier: {stored}"
    );
}

#[tokio::test]
async fn the_answer_is_the_same_whether_or_not_the_record_could_be_written() {
    // Installed first: a counter incremented before the recorder exists is lost.
    let metrics = Metrics::global();
    let p = platform().await;
    member(&p, "vera", Role::Viewer).await;
    let normalised = |mut body: Value| {
        body["traceId"] = Value::Null;
        body["requestId"] = Value::Null;
        body
    };

    let audited = viewer_creates_workflow(&p, "vera").await;
    let (status, headers) = (audited.status(), audited.headers().clone());
    let audited_body = normalised(audited.json().await.expect("envelope"));
    assert_eq!(denials(&p).await.len(), 1);

    sqlx::raw_sql(
        "CREATE FUNCTION reject_denial_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.action = 'access.denied.permission' THEN \
         RAISE EXCEPTION 'injected audit failure'; END IF; RETURN NEW; END $$; \
         CREATE TRIGGER reject_denial_audit BEFORE INSERT ON audit_records \
         FOR EACH ROW EXECUTE FUNCTION reject_denial_audit();",
    )
    .execute(&p.api.db.pool)
    .await
    .expect("failure trigger");

    let unaudited = viewer_creates_workflow(&p, "vera").await;

    assert_eq!(unaudited.status(), status);
    for name in ["content-type", "cache-control", "x-content-type-options"] {
        assert_eq!(unaudited.headers().get(name), headers.get(name), "{name}");
    }
    assert_eq!(
        normalised(unaudited.json().await.expect("envelope")),
        audited_body
    );
    assert_eq!(
        denials(&p).await.len(),
        1,
        "the failed write left nothing behind"
    );
    let page = metrics.render();
    assert!(
        page.lines()
            .any(|line| line.starts_with("access_denial_audits_total{")
                && line.contains("outcome=\"failed\"")),
        "the failure is counted:\n{page}"
    );
    assert!(
        page.lines()
            .any(|line| line.starts_with("access_denial_audits_total{")
                && line.contains("outcome=\"recorded\"")),
        "the success is counted:\n{page}"
    );
}

#[tokio::test]
async fn repeated_denials_are_bounded_by_the_per_subject_rate_limit() {
    let p = platform_with(Options {
        subject_rate_limit: RateLimitConfig {
            per_second: 0.001,
            burst: 3.0,
        },
        ..Options::default()
    })
    .await;
    member(&p, "vera", Role::Viewer).await;

    let mut statuses = Vec::new();
    for _ in 0..10 {
        statuses.push(viewer_creates_workflow(&p, "vera").await.status().as_u16());
    }

    assert_eq!(
        statuses.iter().filter(|s| **s == 403).count(),
        3,
        "{statuses:?}"
    );
    assert_eq!(
        statuses.iter().filter(|s| **s == 429).count(),
        7,
        "{statuses:?}"
    );
    assert_eq!(
        denials(&p).await.len(),
        3,
        "one record per denial, and none for a request the limiter refused"
    );
}

#[tokio::test]
async fn a_404_that_hides_a_foreign_resource_keeps_its_cross_tenant_record_and_is_not_a_403() {
    let p = platform().await;
    member(&p, "vera", Role::Viewer).await;
    let workspace = p.api.create_workspace("bob", "globex", "platform").await;
    let foreign = p
        .api
        .create_namespace("bob", "globex", &workspace, "billing", "dev")
        .await;

    let response = p
        .api
        .get_as(&format!("/namespaces/{foreign}"), "vera", TENANT)
        .await;

    assert_eq!(response.status(), 404);
    let cross_tenant = crate::support::eventually("the probe is recorded", || async {
        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE tenant_id = $1 \
             AND action = 'access.denied.cross_tenant' AND actor = 'vera'",
        )
        .bind(p.tenant.as_uuid())
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count");
        (rows > 0).then_some(rows)
    })
    .await;
    assert_eq!(cross_tenant, 1);
    assert_eq!(count_all_denials(&p).await, 0, "a 404 is not a denial");
}

#[tokio::test]
async fn a_permitted_request_leaves_no_denial_record() {
    let p = platform().await;
    member(&p, "vera", Role::Viewer).await;

    let (status, _) = p.rest_as("vera", Method::GET, "/workflows", None).await;

    assert_eq!(status, 200);
    assert_eq!(count_all_denials(&p).await, 0);
}

#[tokio::test]
async fn a_worker_identity_without_the_permission_leaves_one_record_and_the_same_status() {
    let p = platform().await;
    member(&p, "vera", Role::Viewer).await;
    let mut caller = p.connect("vera", TENANT).await;
    let request = caller.req(crate::support::worker::Worker::register_message(
        &["act.a"],
        10,
    ));

    let status = caller
        .client
        .register_worker(request)
        .await
        .expect_err("a viewer is not a worker");

    assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
    assert_eq!(
        status
            .metadata()
            .get("x-sagat-error")
            .and_then(|v| v.to_str().ok()),
        Some("FORBIDDEN")
    );
    assert_eq!(
        status.message(),
        "the caller's role does not allow this operation"
    );
    let rows = denials(&p).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let (actor, _, kind, call, reason, trace) = &rows[0];
    assert_eq!((actor.as_str(), kind.as_str()), ("vera", "endpoint"));
    assert_eq!(call, "grpc WorkerService/RegisterWorker");
    assert_eq!(reason.as_deref(), Some("missing permission worker:execute"));
    assert!(
        trace.as_deref().is_some_and(|t| t.len() == 32),
        "the call has a trace id: {trace:?}"
    );
}

#[tokio::test]
async fn a_worker_not_bound_to_the_activity_is_recorded_once_with_its_refusal_code() {
    let p = platform().await;
    member(&p, "stranger", Role::Worker).await;
    let mut caller = p.connect("stranger", TENANT).await;
    let request = caller.req(crate::support::worker::Worker::register_message(
        &["act.a"],
        10,
    ));

    let status = caller
        .client
        .register_worker(request)
        .await
        .expect_err("no binding");

    assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
    let rows = denials(&p).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].4.as_deref(), Some("refused: NOT_BOUND"));
    // The worker trail keeps its own record of the same refusal.
    let tenant = p.tenant.as_uuid();
    let worker_denied = crate::support::eventually("the worker trail", || async {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = 'worker.denied'",
        )
        .bind(tenant)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count");
        (n > 0).then_some(n)
    })
    .await;
    assert_eq!(worker_denied, 1);
}

#[tokio::test]
async fn the_trail_of_a_tenant_shows_its_own_denials_to_an_administrator() {
    let p = platform().await;
    member(&p, "vera", Role::Viewer).await;
    assert_eq!(viewer_creates_workflow(&p, "vera").await.status(), 403);

    let (status, trail) = p
        .rest_as(
            HUMAN,
            Method::GET,
            "/audit?action=access.denied.permission",
            None,
        )
        .await;

    assert_eq!(status, 200, "{trail}");
    let items = trail["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{trail}");
    assert_eq!(items[0]["actor"], "vera");
    assert_eq!(items[0]["resourceId"], "POST /v1/workflows");
}

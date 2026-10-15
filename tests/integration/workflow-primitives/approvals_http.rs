//! Public approval decisions over the real HTTP and PostgreSQL boundaries.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use sagat_application::approval_progress::{
    ApprovalAddress, ApprovalExpiryAttempt, ApprovalOpening, ApprovalProgressStore, ApprovalRoutes,
};
use sagat_application::execution::instance_step_id;
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{
    ApprovalPolicy, ApprovalRequest, ApprovalRequestId, ExecutionInstanceId, RunId,
};
use sagat_domain::identity::{Membership, NamespaceId, Principal, Role, Subject};
use sagat_domain::workflow::{Digest, NodeId};
use sagat_infra_postgres::PgApprovalProgressStore;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::support;
use crate::support::api::{TestApi, json_of};

struct Fixture {
    api: TestApi,
    tenant: sagat_domain::identity::TenantId,
    namespace: NamespaceId,
    request: ApprovalRequest,
    scheduler: SchedulerAuthority,
    opened_at: OffsetDateTime,
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

fn histogram_count(page: &str, name: &str, outcome: &str) -> f64 {
    let prefix = format!("{name}_count{{outcome=\"{outcome}\"}} ");
    page.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.0)
}

fn document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "approval",
        "nodes": [
            {
                "id": "approval",
                "type": "approval",
                "timeoutMs": 60000,
                "policy": {"roles": ["operator"], "excludeInitiator": true},
                "onApprove": "approved",
                "onReject": "rejected",
                "onExpire": "expired"
            },
            {
                "id": "approval-two",
                "type": "approval",
                "timeoutMs": 60000,
                "policy": {"roles": ["operator"], "excludeInitiator": true},
                "onApprove": "approved",
                "onReject": "rejected",
                "onExpire": "expired"
            },
            {"id": "approved", "type": "activity", "activity": "approval.approved"},
            {"id": "rejected", "type": "activity", "activity": "approval.rejected"},
            {"id": "expired", "type": "activity", "activity": "approval.expired"}
        ]
    })
}

async fn add_member(fixture: &Fixture, subject: &str, role: Role) {
    let admin = Principal::new(
        fixture.tenant,
        Subject::parse("alice").expect("admin subject"),
        Role::Admin,
    );
    let membership = Membership::grant(
        &admin,
        Subject::parse(subject).expect("member subject"),
        role,
        support::now(),
    )
    .expect("membership");
    fixture
        .api
        .owner_store
        .insert_membership(&membership)
        .await
        .expect("persist membership");
}

async fn scheduler(pool: &sqlx::PgPool, run: RunId) -> SchedulerAuthority {
    let owner = "approval-http-scheduler";
    let generation: i64 = sqlx::query_scalar(
        "UPDATE scheduler_shards SET generation=generation+1, owner=$2, \
         expires_at=clock_timestamp()+interval '1 hour', acquired_at=clock_timestamp() \
         WHERE shard=run_shard($1) RETURNING generation",
    )
    .bind(run.as_uuid())
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("scheduler fence");
    SchedulerAuthority::for_run(
        owner,
        run,
        u64::try_from(generation).expect("scheduler generation"),
    )
}

async fn fixture(name: &str) -> Fixture {
    let api = support::api::start().await;
    let tenant = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    let workspace = api.create_workspace("alice", "acme", "operations").await;
    let namespace_text = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let namespace: NamespaceId = namespace_text.parse().expect("namespace id");
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({
            "name": name,
            "ir": {
                "apiVersion": "sagat-octopus-orchestrator.io/v1",
                "kind": "Workflow",
                "metadata": {"name": name},
                "entrypoint": "start",
                "nodes": [{"id": "start", "type": "activity", "activity": "fixture.start"}]
            }
        }))
        .send()
        .await
        .expect("create workflow");
    assert_eq!(created.status(), StatusCode::CREATED);
    let workflow: WorkflowId = json_of(created).await["id"]
        .as_str()
        .expect("workflow id")
        .parse()
        .expect("workflow id");

    let document = serde_json::to_vec(&document(name)).expect("approval document");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    let opened_at = support::now();
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id, tenant_id, workflow_id, version_number, digest, document, published_by, published_at) \
         VALUES ($1,$2,$3,2,$4,$5,'alice',$6)",
    )
    .bind(version.as_uuid())
    .bind(tenant.as_uuid())
    .bind(workflow.as_uuid())
    .bind(digest(&document).to_hex())
    .bind(document)
    .bind(opened_at)
    .execute(&api.db.pool)
    .await
    .expect("approval version");
    let input = b"{}";
    let input_digest = digest(input);
    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .bind(input_digest.as_bytes().as_slice())
    .bind(input.as_slice())
    .execute(&api.db.pool)
    .await
    .expect("run input");
    let run = RunId::generate();
    sqlx::query(
        "INSERT INTO runs \
         (id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, run_version, \
          input_digest, started_by, created_at, updated_at, root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'RUNNING',1,$6,'alice',$7,$7,$1)",
    )
    .bind(run.as_uuid())
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(input_digest.as_bytes().as_slice())
    .bind(opened_at)
    .execute(&api.db.pool)
    .await
    .expect("approval run");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, detail, actor, occurred_at) \
         VALUES ($1,$2,1,'run.started',$3,'system:scheduler',$4)",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(json!({"entrypoint": "approval"}))
    .bind(opened_at)
    .execute(&api.db.pool)
    .await
    .expect("run journal");
    let owner = ExecutionInstanceId::root(NodeId::parse("approval").expect("approval node"));
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, kind, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1,$2,'approval',$3,$4,'PENDING',$5,'FORWARD','{}','{}')",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(instance_step_id(version, run, &owner))
    .bind(namespace.as_uuid())
    .bind(opened_at)
    .execute(&api.db.pool)
    .await
    .expect("approval step");

    let scheduler = scheduler(&api.db.pool, run).await;
    let request = ApprovalRequest::open(
        ApprovalRequestId::generate(),
        tenant,
        namespace,
        run,
        owner,
        ApprovalPolicy::new(Vec::new(), vec![Role::Admin, Role::Operator])
            .expect("approval policy"),
        Subject::parse("alice").expect("initiator"),
        2,
        opened_at + Duration::from_secs(60),
    );
    let audit = AuditRecord::engine(
        tenant,
        AuditAction::ApprovalOpened,
        ResourceType::ApprovalRequest,
        request.id().to_string(),
        "approval opened",
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        opened_at,
    );
    PgApprovalProgressStore::new(api.db.pool.clone())
        .open(&ApprovalOpening::new(
            request.clone(),
            ApprovalRoutes::new(
                NodeId::parse("approved").expect("approved route"),
                NodeId::parse("rejected").expect("rejected route"),
                NodeId::parse("expired").expect("expired route"),
            ),
            scheduler.clone(),
            audit,
            Uuid::now_v7(),
            opened_at,
        ))
        .await
        .expect("open approval");

    let fixture = Fixture {
        api,
        tenant,
        namespace,
        request,
        scheduler,
        opened_at,
    };
    add_member(&fixture, "olga", Role::Operator).await;
    fixture
}

fn path(fixture: &Fixture) -> String {
    format!("/approvals/{}/decisions", fixture.request.id())
}

async fn decision(fixture: &Fixture, subject: &str, body: &Value) -> reqwest::Response {
    fixture
        .api
        .post_as(&path(fixture), subject, "acme", body)
        .await
}

#[tokio::test]
async fn run_approvals_are_tenant_scoped_bounded_and_do_not_disclose_eligibility() {
    let fixture = fixture("http-approval-list").await;
    add_member(&fixture, "vera", Role::Viewer).await;
    add_member(&fixture, "wendy", Role::Worker).await;
    let version = WorkflowVersionId::new(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT workflow_version_id FROM runs WHERE tenant_id=$1 AND id=$2",
        )
        .bind(fixture.tenant.as_uuid())
        .bind(fixture.request.run_id().as_uuid())
        .fetch_one(&fixture.api.db.pool)
        .await
        .expect("workflow version"),
    );
    let second_owner =
        ExecutionInstanceId::root(NodeId::parse("approval-two").expect("second approval node"));
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, kind, \
          instance_path_nodes, instance_path_indices) \
         VALUES ($1,$2,'approval-two',$3,$4,'PENDING',$5,'FORWARD','{}','{}')",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.request.run_id().as_uuid())
    .bind(instance_step_id(
        version,
        fixture.request.run_id(),
        &second_owner,
    ))
    .bind(fixture.namespace.as_uuid())
    .bind(fixture.opened_at)
    .execute(&fixture.api.db.pool)
    .await
    .expect("second approval step");
    let second = ApprovalRequest::open(
        ApprovalRequestId::generate(),
        fixture.tenant,
        fixture.namespace,
        fixture.request.run_id(),
        second_owner,
        ApprovalPolicy::new(Vec::new(), vec![Role::Operator]).expect("approval policy"),
        Subject::parse("alice").expect("initiator"),
        fixture.request.opened_sequence() + 1,
        fixture.opened_at + Duration::from_secs(120),
    );
    PgApprovalProgressStore::new(fixture.api.db.pool.clone())
        .open(&ApprovalOpening::new(
            second.clone(),
            ApprovalRoutes::new(
                NodeId::parse("approved").expect("approved route"),
                NodeId::parse("rejected").expect("rejected route"),
                NodeId::parse("expired").expect("expired route"),
            ),
            fixture.scheduler.clone(),
            AuditRecord::engine(
                fixture.tenant,
                AuditAction::ApprovalOpened,
                ResourceType::ApprovalRequest,
                second.id().to_string(),
                "approval opened",
                Some("0af7651916cd43dd8448eb211c80319c".into()),
                fixture.opened_at,
            ),
            Uuid::now_v7(),
            fixture.opened_at,
        ))
        .await
        .expect("open second approval");
    let path = format!("/runs/{}/approvals?limit=1", fixture.request.run_id());

    let response = fixture.api.get_as(&path, "vera", "acme").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = json_of(response).await;
    let item = &body["items"][0];
    assert_eq!(item["approvalId"], fixture.request.id().to_string());
    assert_eq!(item["runId"], fixture.request.run_id().to_string());
    assert_eq!(item["nodeId"], "approval");
    assert_eq!(item["instancePath"], json!([]));
    assert_eq!(item["status"], "PENDING");
    let next_cursor = body["nextCursor"].as_str().expect("next cursor");
    let serialized = body.to_string();
    for hidden in [
        "policy",
        "subjects",
        "roles",
        "initiator",
        "actor",
        "winningReceiptId",
    ] {
        assert!(!serialized.contains(hidden), "disclosed {hidden}");
    }

    let next = fixture
        .api
        .get_as(
            &format!(
                "/runs/{}/approvals?limit=1&cursor={next_cursor}",
                fixture.request.run_id()
            ),
            "vera",
            "acme",
        )
        .await;
    assert_eq!(next.status(), StatusCode::OK);
    let next = json_of(next).await;
    assert_eq!(next["items"][0]["approvalId"], second.id().to_string());
    assert_eq!(next["nextCursor"], Value::Null);

    let invalid_cursor = fixture
        .api
        .get_as(
            &format!(
                "/runs/{}/approvals?cursor=not-a-cursor",
                fixture.request.run_id()
            ),
            "vera",
            "acme",
        )
        .await;
    assert_eq!(invalid_cursor.status(), StatusCode::BAD_REQUEST);

    let forbidden = fixture.api.get_as(&path, "wendy", "acme").await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let foreign = fixture.api.get_as(&path, "bob", "globex").await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    let foreign = json_of(foreign).await;
    assert_eq!(foreign["code"], "NOT_FOUND");
    assert!(!foreign.to_string().contains("acme"));
    assert!(!foreign.to_string().contains("payments"));
}

#[tokio::test]
async fn decision_commits_once_and_exact_replay_returns_the_receipt() {
    let fixture = fixture("http-approval-idempotency").await;
    let metrics = sagat_telemetry::Metrics::global();
    let approved_before = histogram_count(
        &metrics.render(),
        "workflow_approval_wait_seconds",
        "approved",
    );
    let body = json!({
        "decisionKey": "change-approval-7",
        "decision": "APPROVE",
        "reasonCode": "POLICY_OK"
    });
    let first = decision(&fixture, "olga", &body).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["cache-control"], "no-store");
    let first = json_of(first).await;
    assert_eq!(first["replayed"], false);
    assert!(
        histogram_count(
            &metrics.render(),
            "workflow_approval_wait_seconds",
            "approved"
        ) >= approved_before + 1.0
    );

    let replay = decision(&fixture, "olga", &body).await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = json_of(replay).await;
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["receiptId"], first["receiptId"]);
    assert_eq!(replay["outcomeSequence"], first["outcomeSequence"]);

    let changed = decision(
        &fixture,
        "olga",
        &json!({
            "decisionKey": "change-approval-7",
            "decision": "REJECT",
            "reasonCode": "POLICY_CHANGED"
        }),
    )
    .await;
    assert_eq!(changed.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_of(changed).await["details"]["reason"],
        "IDEMPOTENCY_KEY_REUSED"
    );

    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_decision_receipts WHERE tenant_id=$1 AND request_id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.request.id().as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("receipt count");
    assert_eq!(receipts, 1);
}

#[tokio::test]
async fn permission_eligibility_initiator_and_tenant_boundaries_are_opaque_and_audited() {
    let fixture = fixture("http-approval-authorization").await;
    add_member(&fixture, "dora", Role::Developer).await;
    add_member(&fixture, "vera", Role::Viewer).await;
    let body = json!({"decisionKey": "authorization-check", "decision": "APPROVE"});

    let viewer = decision(&fixture, "vera", &body).await;
    assert_eq!(viewer.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_of(viewer).await["details"]["reason"], "NOT_ELIGIBLE");

    let ineligible = decision(&fixture, "dora", &body).await;
    assert_eq!(ineligible.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        json_of(ineligible).await["details"]["reason"],
        "NOT_ELIGIBLE"
    );

    let initiator = decision(&fixture, "alice", &body).await;
    assert_eq!(initiator.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        json_of(initiator).await["details"]["reason"],
        "INITIATOR_CANNOT_DECIDE"
    );

    let denied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='approval.denied' \
         AND resource_id=$2 AND actor IN ('vera','dora','alice')",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.request.id().to_string())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("durable denial audit count");
    assert_eq!(denied, 3);

    let foreign = fixture
        .api
        .post_as(&path(&fixture), "bob", "globex", &body)
        .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    let foreign_body = json_of(foreign).await;
    assert_eq!(foreign_body["code"], "NOT_FOUND");
    assert!(!foreign_body.to_string().contains("acme"));
    assert!(!foreign_body.to_string().contains("payments"));

    fixture.api.execution_store.flush_denial_writes().await;
    let cross_tenant: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records a JOIN tenants t ON t.id=a.tenant_id \
         WHERE t.slug='globex' AND a.action='access.denied.cross_tenant' \
         AND a.resource_type='approval_request' AND a.resource_id=$1",
    )
    .bind(fixture.request.id().to_string())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("cross-tenant audit count");
    assert_eq!(cross_tenant, 1);
}

#[tokio::test]
async fn expired_approval_rejects_a_late_http_decision() {
    let fixture = fixture("http-approval-expired").await;
    let metrics = sagat_telemetry::Metrics::global();
    let expired_before = histogram_count(
        &metrics.render(),
        "workflow_approval_wait_seconds",
        "expired",
    );
    let expired_at = fixture.opened_at + Duration::from_secs(61);
    PgApprovalProgressStore::new(fixture.api.db.pool.clone())
        .expire(&ApprovalExpiryAttempt::new(
            ApprovalAddress::new(fixture.tenant, fixture.namespace, fixture.request.id()),
            fixture.scheduler.clone(),
            AuditRecord::engine(
                fixture.tenant,
                AuditAction::ApprovalExpired,
                ResourceType::ApprovalRequest,
                fixture.request.id().to_string(),
                "approval expired",
                Some("0af7651916cd43dd8448eb211c80319c".into()),
                expired_at,
            ),
            Uuid::now_v7(),
            expired_at,
        ))
        .await
        .expect("expire approval");
    assert!(
        histogram_count(
            &metrics.render(),
            "workflow_approval_wait_seconds",
            "expired"
        ) >= expired_before + 1.0
    );

    let late = decision(
        &fixture,
        "olga",
        &json!({"decisionKey": "late-decision", "decision": "APPROVE"}),
    )
    .await;
    assert_eq!(late.status(), StatusCode::CONFLICT);
    assert_eq!(json_of(late).await["details"]["reason"], "ALREADY_DECIDED");
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM approval_decision_receipts WHERE tenant_id=$1 AND request_id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.request.id().as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("receipt count");
    assert_eq!(receipts, 0);
}

#[tokio::test]
async fn opaque_approval_lookup_has_a_request_id_index() {
    let api = support::api::start().await;
    let definition: String = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes \
         WHERE schemaname='public' AND tablename='approval_requests' \
           AND indexname='approval_requests_request_id_idx'",
    )
    .fetch_one(&api.db.pool)
    .await
    .expect("approval request lookup index");

    assert!(definition.ends_with("USING btree (request_id)"));
}

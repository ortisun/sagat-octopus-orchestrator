//! Public declared operations over the real HTTP and PostgreSQL boundaries.

use reqwest::{Method, StatusCode};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Membership, NamespaceId, Principal, Role, Subject};
use sagat_domain::workflow::Digest;
use sagat_infra_ir::{JsonIrCodec, ParsedDeclaredState};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support;
use crate::support::api::{TestApi, json_of};

pub(crate) struct Fixture {
    pub(crate) api: TestApi,
    pub(crate) tenant: sagat_domain::identity::TenantId,
    pub(crate) namespace: NamespaceId,
    version: WorkflowVersionId,
    pub(crate) run: RunId,
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

fn document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "done",
        "nodes": [{"id": "done", "type": "end"}],
        "state": {
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"balance": {"type": "integer"}},
                "required": ["balance"]
            },
            "initial": "input.account",
            "queries": {
                "balance.read": {
                    "argsSchema": {"type": "integer"},
                    "resultSchema": {"type": "integer"},
                    "expression": "state.balance + args",
                    "disclosure": "declared-business-result-v1"
                }
            },
            "updates": {
                "balance.replace": {
                    "argsSchema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "allowed": {"type": "boolean"},
                            "nextState": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {"balance": {"type": "integer"}},
                                "required": ["balance"]
                            }
                        },
                        "required": ["allowed", "nextState"]
                    },
                    "validate": "args.allowed",
                    "state": "args.nextState",
                    "result": "newState.balance",
                    "resultSchema": {"type": "integer"},
                    "disclosure": "declared-business-result-v1"
                }
            }
        }
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

async fn seed_version(
    api: &TestApi,
    tenant: sagat_domain::identity::TenantId,
    namespace: NamespaceId,
    workflow: WorkflowId,
    declared: &ParsedDeclaredState,
) -> (WorkflowVersionId, RunId) {
    let version = WorkflowVersionId::generate();
    let now = support::now();
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id, tenant_id, workflow_id, version_number, digest, document, published_by, published_at) \
         VALUES ($1,$2,$3,2,$4,$5,'alice',$6)",
    )
    .bind(version.as_uuid())
    .bind(tenant.as_uuid())
    .bind(workflow.as_uuid())
    .bind(declared.parsed().digest.to_hex())
    .bind(&declared.parsed().canonical)
    .bind(now)
    .execute(&api.db.pool)
    .await
    .expect("declared version");

    for (kind, operation, declaration_digest, result_schema) in declared
        .state()
        .queries()
        .map(|operation| {
            (
                "QUERY",
                operation.name(),
                operation.digest(),
                operation.result_schema().digest(),
            )
        })
        .chain(declared.state().updates().map(|operation| {
            (
                "UPDATE",
                operation.name(),
                operation.digest(),
                operation.result_schema().digest(),
            )
        }))
    {
        sqlx::query(
            "INSERT INTO declared_operation_definitions \
             (tenant_id, namespace_id, workflow_version_id, operation_kind, operation_name, \
              declaration_digest, state_schema_digest, result_schema_digest, disclosure, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'declared-business-result-v1',$9)",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(version.as_uuid())
        .bind(kind)
        .bind(operation.as_str())
        .bind(declaration_digest.as_bytes().as_slice())
        .bind(declared.state().schema().digest().as_bytes().as_slice())
        .bind(result_schema.as_bytes().as_slice())
        .bind(now)
        .execute(&api.db.pool)
        .await
        .expect("declared operation");
    }

    let input = br#"{"account":{"balance":10}}"#;
    let state = br#"{"balance":10}"#;
    let input_digest = digest(input);
    let state_digest = digest(state);
    for (content_digest, content) in [
        (input_digest, input.as_slice()),
        (state_digest, state.as_slice()),
    ] {
        sqlx::query(
            "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
        )
        .bind(tenant.as_uuid())
        .bind(content_digest.as_bytes().as_slice())
        .bind(content)
        .execute(&api.db.pool)
        .await
        .expect("operation payload");
    }

    let run = RunId::generate();
    sqlx::query(
        "INSERT INTO runs \
         (id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, run_version, \
          input_digest, started_by, created_at, updated_at, root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'WAITING',1,$6,'alice',$7,$7,$1)",
    )
    .bind(run.as_uuid())
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(input_digest.as_bytes().as_slice())
    .bind(now)
    .execute(&api.db.pool)
    .await
    .expect("operation run");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, detail, actor, occurred_at) \
         VALUES ($1,$2,1,'run.started',$3,'system:scheduler',$4)",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(json!({"entrypoint": "done"}))
    .bind(now)
    .execute(&api.db.pool)
    .await
    .expect("run journal");
    sqlx::query(
        "INSERT INTO workflow_operation_states \
         (tenant_id, namespace_id, run_id, state_digest, state_schema_digest, state_size_bytes, \
          state_revision, last_changed_run_version, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,0,1,$7)",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(run.as_uuid())
    .bind(state_digest.as_bytes().as_slice())
    .bind(declared.state().schema().digest().as_bytes().as_slice())
    .bind(i32::try_from(state.len()).expect("state size"))
    .bind(now)
    .execute(&api.db.pool)
    .await
    .expect("operation state");
    (version, run)
}

pub(crate) async fn fixture(name: &str) -> Fixture {
    let api = support::api::start().await;
    let tenant = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    let workspace = api.create_workspace("alice", "acme", "operations").await;
    let namespace: NamespaceId = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await
        .parse()
        .expect("namespace id");
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
                "entrypoint": "done",
                "nodes": [{"id": "done", "type": "end"}]
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
    let declared = JsonIrCodec
        .parse_declared_state(document(name).to_string().as_bytes())
        .expect("declared document");
    let (version, run) = seed_version(&api, tenant, namespace, workflow, &declared).await;
    Fixture {
        api,
        tenant,
        namespace,
        version,
        run,
    }
}

pub(crate) fn query_path(fixture: &Fixture) -> String {
    format!("/runs/{}/queries/balance.read", fixture.run)
}

pub(crate) fn update_path(fixture: &Fixture) -> String {
    format!("/runs/{}/updates/balance.replace", fixture.run)
}

async fn attach_singleton_chain(fixture: &Fixture) {
    let mut transaction = fixture
        .api
        .db
        .pool
        .begin()
        .await
        .expect("chain transaction");
    let now = support::now();
    sqlx::query(
        "INSERT INTO execution_chains \
         (tenant_id,namespace_id,chain_id,current_run_id,definition_version_id,started_by, \
          max_segments,original_deadline_at,created_at,updated_at) \
         VALUES ($1,$2,$3,$3,$4,'alice',4,$5,$6,$6)",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.namespace.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(fixture.version.as_uuid())
    .bind(now + time::Duration::hours(1))
    .bind(now)
    .execute(&mut *transaction)
    .await
    .expect("chain");
    sqlx::query(
        "INSERT INTO chain_segments \
         (tenant_id,namespace_id,chain_id,segment_index,run_id,created_at) \
         VALUES ($1,$2,$3,0,$3,$4)",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.namespace.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(now)
    .execute(&mut *transaction)
    .await
    .expect("initial segment");
    transaction.commit().await.expect("commit chain");
}

fn chain_query_path(fixture: &Fixture) -> String {
    format!(
        "/namespaces/{}/chains/{}/queries/balance.read",
        fixture.namespace, fixture.run
    )
}

fn chain_update_path(fixture: &Fixture) -> String {
    format!(
        "/namespaces/{}/chains/{}/updates/balance.replace",
        fixture.namespace, fixture.run
    )
}

async fn seed_successor(fixture: &Fixture, balance: i64) -> RunId {
    let (workflow_id, input_digest, state_schema_digest): (Uuid, Vec<u8>, Vec<u8>) =
        sqlx::query_as(
            "SELECT r.workflow_id,r.input_digest,s.state_schema_digest \
             FROM runs r JOIN workflow_operation_states s \
               ON s.tenant_id=r.tenant_id AND s.run_id=r.id \
             WHERE r.tenant_id=$1 AND r.id=$2",
        )
        .bind(fixture.tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.api.db.pool)
        .await
        .expect("predecessor state");
    let state = serde_json::to_vec(&json!({"balance": balance})).expect("successor state");
    let state_digest = digest(&state);
    sqlx::query(
        "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(state_digest.as_bytes().as_slice())
    .bind(&state)
    .execute(&fixture.api.db.pool)
    .await
    .expect("successor state payload");
    let run = RunId::generate();
    let now = support::now();
    sqlx::query(
        "INSERT INTO runs \
         (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status,run_version, \
          input_digest,started_by,created_at,updated_at,root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'WAITING',1,$6,'alice',$7,$7,$8)",
    )
    .bind(run.as_uuid())
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.namespace.as_uuid())
    .bind(workflow_id)
    .bind(fixture.version.as_uuid())
    .bind(input_digest)
    .bind(now)
    .bind(run.as_uuid())
    .execute(&fixture.api.db.pool)
    .await
    .expect("successor run");
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id,run_id,seq,event_type,detail,actor,occurred_at) \
         VALUES ($1,$2,1,'run.started',$3,'system:scheduler',$4)",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(json!({"entrypoint": "done"}))
    .bind(now)
    .execute(&fixture.api.db.pool)
    .await
    .expect("successor journal");
    sqlx::query(
        "INSERT INTO workflow_operation_states \
         (tenant_id,namespace_id,run_id,state_digest,state_schema_digest,state_size_bytes, \
          state_revision,last_changed_run_version,updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,0,1,$7)",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.namespace.as_uuid())
    .bind(run.as_uuid())
    .bind(state_digest.as_bytes().as_slice())
    .bind(state_schema_digest)
    .bind(i32::try_from(state.len()).expect("state size"))
    .bind(now)
    .execute(&fixture.api.db.pool)
    .await
    .expect("successor operation state");
    run
}

async fn advance_chain(fixture: &Fixture, successor: RunId) {
    let mut transaction = fixture.api.db.pool.begin().await.expect("handoff tx");
    let now = support::now();
    let successor_input: Vec<u8> =
        sqlx::query_scalar("SELECT input_digest FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(fixture.tenant.as_uuid())
            .bind(successor.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .expect("successor input");
    sqlx::query(
        "UPDATE runs SET status='SUCCEEDED',finished_at=$3,updated_at=$3 \
         WHERE tenant_id=$1 AND id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(now)
    .execute(&mut *transaction)
    .await
    .expect("complete predecessor");
    sqlx::query(
        "UPDATE chain_segments SET completion_kind='CONTINUED',successor_run_id=$4, \
                transfer_input_digest=$5,completed_at=$6 \
         WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 AND segment_index=0",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.namespace.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(successor.as_uuid())
    .bind(successor_input)
    .bind(now)
    .execute(&mut *transaction)
    .await
    .expect("complete predecessor segment");
    sqlx::query(
        "INSERT INTO chain_segments \
         (tenant_id,namespace_id,chain_id,segment_index,run_id,predecessor_run_id,created_at) \
         VALUES ($1,$2,$3,1,$4,$3,$5)",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.namespace.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(successor.as_uuid())
    .bind(now)
    .execute(&mut *transaction)
    .await
    .expect("successor segment");
    sqlx::query(
        "UPDATE execution_chains SET current_run_id=$4,segment_count=2,revision=revision+1,updated_at=$5 \
         WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.namespace.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(successor.as_uuid())
    .bind(now)
    .execute(&mut *transaction)
    .await
    .expect("advance chain head");
    transaction.commit().await.expect("commit handoff");
}

#[tokio::test]
async fn query_returns_only_the_declared_result_without_mutating_the_run() {
    let fixture = fixture("http-declared-query").await;
    let before: (i64, i64, i64) = sqlx::query_as(
        "SELECT r.run_version, \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id), \
                (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id) \
         FROM runs r WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("run counters");

    let response = fixture
        .api
        .post_as(&query_path(&fixture), "alice", "acme", &json!({"args": 5}))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = json_of(response).await;
    assert_eq!(body["runId"], fixture.run.to_string());
    assert_eq!(body["runVersion"], 1);
    assert_eq!(body["stateRevision"], 0);
    assert_eq!(body["result"], 15);
    assert_eq!(body.as_object().expect("object").len(), 4);

    let after: (i64, i64, i64) = sqlx::query_as(
        "SELECT r.run_version, \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id), \
                (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id) \
         FROM runs r WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("run counters");
    assert_eq!(after, before);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.queried' \
         AND resource_id=$2 AND actor='alice' AND reason='operation=balance.read'",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.to_string())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("query audit");
    assert_eq!(audits, 1);

    let invalid = fixture
        .api
        .post_as(
            &query_path(&fixture),
            "alice",
            "acme",
            &json!({"args": {"unexpected": true}}),
        )
        .await;
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid.headers()["cache-control"], "no-store");
    assert_eq!(
        json_of(invalid).await["details"]["reason"],
        "INVALID_ARGUMENTS"
    );

    let missing = fixture
        .api
        .post_as(
            &format!("/runs/{}/queries/balance.missing", fixture.run),
            "alice",
            "acme",
            &json!({"args": 1}),
        )
        .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(missing.headers()["cache-control"], "no-store");
}

#[tokio::test]
async fn chain_query_resolves_one_head_and_enforces_tenant_and_expected_run() {
    let fixture = fixture("http-chain-declared-query").await;
    attach_singleton_chain(&fixture).await;
    add_member(&fixture, "vera", Role::Viewer).await;
    let path = chain_query_path(&fixture);

    let viewer = fixture
        .api
        .post_as(&path, "vera", "acme", &json!({"args": 5}))
        .await;
    assert_eq!(viewer.status(), StatusCode::FORBIDDEN);
    assert_eq!(viewer.headers()["cache-control"], "no-store");
    assert_eq!(json_of(viewer).await["code"], "FORBIDDEN");

    let response = fixture
        .api
        .post_as(
            &path,
            "alice",
            "acme",
            &json!({"args": 5, "expectedRunId": fixture.run.to_string()}),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = json_of(response).await;
    assert_eq!(body["runId"], fixture.run.to_string());
    assert_eq!(body["runVersion"], 1);
    assert_eq!(body["stateRevision"], 0);
    assert_eq!(body["result"], 15);

    let stale = fixture
        .api
        .post_as(
            &path,
            "alice",
            "acme",
            &json!({"args": 5, "expectedRunId": RunId::generate().to_string()}),
        )
        .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(stale.headers()["cache-control"], "no-store");
    let stale = json_of(stale).await;
    assert_eq!(stale["code"], "CONFLICT");
    assert_eq!(stale["details"]["reason"], "CHAIN_HEAD_CHANGED");

    let foreign = fixture
        .api
        .post_as(&path, "bob", "globex", &json!({"args": 5}))
        .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    assert_eq!(foreign.headers()["cache-control"], "no-store");

    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.queried' \
         AND resource_id=$2 AND actor='alice' AND reason='operation=balance.read'",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.to_string())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("chain query audit");
    assert_eq!(audits, 1);
    let viewer_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.queried' \
         AND resource_id=$2 AND actor='vera'",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.to_string())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("viewer query audit");
    assert_eq!(viewer_audits, 0);
}

#[tokio::test]
async fn chain_update_keeps_one_receipt_across_handoff_and_cas_protects_new_keys() {
    let fixture = fixture("http-chain-declared-update").await;
    attach_singleton_chain(&fixture).await;
    add_member(&fixture, "vera", Role::Viewer).await;
    let path = chain_update_path(&fixture);
    let body = json!({
        "updateKey": "chain-balance-27",
        "args": {"allowed": true, "nextState": {"balance": 27}},
        "expectedStateRevision": 0
    });

    let viewer = fixture.api.post_as(&path, "vera", "acme", &body).await;
    assert_eq!(viewer.status(), StatusCode::FORBIDDEN);
    assert_eq!(viewer.headers()["cache-control"], "no-store");

    let foreign = fixture.api.post_as(&path, "bob", "globex", &body).await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    assert_eq!(foreign.headers()["cache-control"], "no-store");

    let mut chain_lock = fixture.api.db.pool.begin().await.expect("chain lock tx");
    let locked: Uuid = sqlx::query_scalar(
        "SELECT current_run_id FROM execution_chains \
         WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 FOR UPDATE",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.namespace.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&mut *chain_lock)
    .await
    .expect("lock current head");
    assert_eq!(locked, fixture.run.as_uuid());
    let mut first_request = Box::pin(fixture.api.post_as(&path, "alice", "acme", &body));
    tokio::select! {
        response = &mut first_request => panic!("chain update bypassed the chain lock: {}", response.status()),
        () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
    }
    chain_lock.commit().await.expect("release chain lock");
    let first = first_request.await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["cache-control"], "no-store");
    let first = json_of(first).await;
    assert_eq!(first["runId"], fixture.run.to_string());
    assert_eq!(first["result"], 27);
    assert_eq!(first["replayed"], false);

    let explicit = json!({
        "updateKey": "chain-explicit-head",
        "args": {"allowed": true, "nextState": {"balance": 28}},
        "expectedRunId": fixture.run.to_string(),
        "expectedStateRevision": 1
    });
    let explicit_first = fixture.api.post_as(&path, "alice", "acme", &explicit).await;
    assert_eq!(explicit_first.status(), StatusCode::OK);
    let explicit_first = json_of(explicit_first).await;
    assert_eq!(explicit_first["runId"], fixture.run.to_string());
    assert_eq!(explicit_first["result"], 28);
    assert_eq!(explicit_first["replayed"], false);

    let denied = json!({
        "updateKey": "chain-validator-rejection",
        "args": {"allowed": false, "nextState": {"balance": 99}}
    });
    let denied_first = fixture.api.post_as(&path, "alice", "acme", &denied).await;
    assert_eq!(denied_first.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(denied_first.headers()["cache-control"], "no-store");
    assert_eq!(json_of(denied_first).await["details"]["replayed"], false);

    let successor = seed_successor(&fixture, 50).await;
    advance_chain(&fixture, successor).await;

    let replay = fixture.api.post_as(&path, "alice", "acme", &body).await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(replay.headers()["cache-control"], "no-store");
    let replay = json_of(replay).await;
    assert_eq!(replay["runId"], fixture.run.to_string());
    assert_eq!(replay["result"], 27);
    assert_eq!(replay["replayed"], true);

    let explicit_replay = fixture.api.post_as(&path, "alice", "acme", &explicit).await;
    assert_eq!(explicit_replay.status(), StatusCode::OK);
    let explicit_replay = json_of(explicit_replay).await;
    assert_eq!(explicit_replay["runId"], fixture.run.to_string());
    assert_eq!(explicit_replay["result"], 28);
    assert_eq!(explicit_replay["replayed"], true);

    let denied_replay = fixture.api.post_as(&path, "alice", "acme", &denied).await;
    assert_eq!(denied_replay.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let denied_replay = json_of(denied_replay).await;
    assert_eq!(denied_replay["details"]["runId"], fixture.run.to_string());
    assert_eq!(denied_replay["details"]["replayed"], true);

    let changed = fixture
        .api
        .post_as(
            &path,
            "alice",
            "acme",
            &json!({
                "updateKey": "chain-balance-27",
                "args": {"allowed": true, "nextState": {"balance": 28}},
                "expectedStateRevision": 0
            }),
        )
        .await;
    assert_eq!(changed.status(), StatusCode::CONFLICT);
    assert_eq!(changed.headers()["cache-control"], "no-store");
    assert_eq!(
        json_of(changed).await["details"]["reason"],
        "IDEMPOTENCY_KEY_REUSED"
    );

    let stale = fixture
        .api
        .post_as(
            &path,
            "alice",
            "acme",
            &json!({
                "updateKey": "chain-stale-head",
                "args": {"allowed": true, "nextState": {"balance": 60}},
                "expectedRunId": fixture.run.to_string()
            }),
        )
        .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(stale.headers()["cache-control"], "no-store");
    assert_eq!(
        json_of(stale).await["details"]["reason"],
        "CHAIN_HEAD_CHANGED"
    );

    let next = fixture
        .api
        .post_as(
            &path,
            "alice",
            "acme",
            &json!({
                "updateKey": "chain-successor",
                "args": {"allowed": true, "nextState": {"balance": 61}},
                "expectedRunId": successor.to_string(),
                "expectedStateRevision": 0
            }),
        )
        .await;
    assert_eq!(next.status(), StatusCode::OK);
    let next = json_of(next).await;
    assert_eq!(next["runId"], successor.to_string());
    assert_eq!(next["result"], 61);

    let receipts: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT target_id,resolved_run_id,update_key FROM operation_update_receipts \
         WHERE tenant_id=$1 AND namespace_id=$2 AND address_scope='CHAIN' \
         ORDER BY update_key",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.namespace.as_uuid())
    .fetch_all(&fixture.api.db.pool)
    .await
    .expect("chain update receipts");
    assert_eq!(receipts.len(), 4);
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt.0 == fixture.run.as_uuid())
    );
    assert!(
        receipts.iter().any(|receipt| {
            receipt.1 == fixture.run.as_uuid() && receipt.2 == "chain-balance-27"
        })
    );
    assert!(receipts.iter().any(|receipt| {
        receipt.1 == fixture.run.as_uuid() && receipt.2 == "chain-explicit-head"
    }));
    assert!(receipts.iter().any(|receipt| {
        receipt.1 == fixture.run.as_uuid() && receipt.2 == "chain-validator-rejection"
    }));
    assert!(
        receipts
            .iter()
            .any(|receipt| { receipt.1 == successor.as_uuid() && receipt.2 == "chain-successor" })
    );

    let audits: Vec<(String, i64)> = sqlx::query_as(
        "SELECT resource_id,count(*) FROM audit_records \
         WHERE tenant_id=$1 AND action='run.updated' AND actor='alice' \
           AND reason LIKE 'operation=balance.replace;outcome=%' \
           AND resource_id IN ($2,$3) GROUP BY resource_id ORDER BY resource_id",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.to_string())
    .bind(successor.to_string())
    .fetch_all(&fixture.api.db.pool)
    .await
    .expect("resolved update audits");
    assert!(audits.contains(&(fixture.run.to_string(), 6)));
    assert!(audits.contains(&(successor.to_string(), 1)));
    let viewer_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.updated' \
         AND actor='vera' AND resource_id IN ($2,$3)",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.to_string())
    .bind(successor.to_string())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("viewer update audit");
    assert_eq!(viewer_audits, 0);
}

#[tokio::test]
async fn update_commits_once_and_replays_the_durable_result() {
    let fixture = fixture("http-declared-update").await;
    let body = json!({
        "updateKey": "replace-balance-7",
        "args": {"allowed": true, "nextState": {"balance": 27}},
        "expectedStateRevision": 0
    });
    let first = fixture
        .api
        .post_as(&update_path(&fixture), "alice", "acme", &body)
        .await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["cache-control"], "no-store");
    let first = json_of(first).await;
    assert_eq!(first["status"], "APPLIED");
    assert_eq!(first["result"], 27);
    assert_eq!(first["beforeStateRevision"], 0);
    assert_eq!(first["afterStateRevision"], 1);
    assert_eq!(first["runVersion"], 2);
    assert_eq!(first["replayed"], false);

    let replay = fixture
        .api
        .post_as(&update_path(&fixture), "alice", "acme", &body)
        .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = json_of(replay).await;
    assert_eq!(replay["result"], 27);
    assert_eq!(replay["runVersion"], 2);
    assert_eq!(replay["replayed"], true);

    let changed = fixture
        .api
        .post_as(
            &update_path(&fixture),
            "alice",
            "acme",
            &json!({
                "updateKey": "replace-balance-7",
                "args": {"allowed": true, "nextState": {"balance": 28}},
                "expectedStateRevision": 0
            }),
        )
        .await;
    assert_eq!(changed.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_of(changed).await["details"]["reason"],
        "IDEMPOTENCY_KEY_REUSED"
    );
    let changed_operation = fixture
        .api
        .post_as(
            &format!("/runs/{}/updates/balance.missing", fixture.run),
            "alice",
            "acme",
            &json!({
                "updateKey": "replace-balance-7",
                "args": {"allowed": true, "nextState": {"balance": 27}},
                "expectedStateRevision": 0
            }),
        )
        .await;
    assert_eq!(changed_operation.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_of(changed_operation).await["details"]["reason"],
        "IDEMPOTENCY_KEY_REUSED"
    );

    let stored: (i64, i64, Vec<u8>, i64, i64, i64) = sqlx::query_as(
        "SELECT s.state_revision, s.last_changed_run_version, p.content, \
                (SELECT count(*) FROM operation_update_receipts x WHERE x.tenant_id=s.tenant_id AND x.resolved_run_id=s.run_id), \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=s.tenant_id AND e.run_id=s.run_id AND e.event_type='state.updated'), \
                (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=s.tenant_id AND o.aggregate_id=s.run_id AND o.event_type='run.state_updated') \
         FROM workflow_operation_states s \
         JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.state_digest \
         WHERE s.tenant_id=$1 AND s.run_id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("committed update");
    assert_eq!(stored.0, 1);
    assert_eq!(stored.1, 2);
    assert_eq!(stored.2, br#"{"balance":27}"#);
    assert_eq!((stored.3, stored.4, stored.5), (1, 1, 1));
    let published =
        crate::hint_contract::assert_outbox_matches_the_contract(&fixture.api.db.pool).await;
    crate::hint_contract::assert_published(&published, &["run.state_updated"]);
}

#[tokio::test]
async fn validator_and_revision_rejections_are_stable_and_do_not_schedule_work() {
    let fixture = fixture("http-declared-rejections").await;
    let validator = json!({
        "updateKey": "validator-denied",
        "args": {"allowed": false, "nextState": {"balance": 90}}
    });
    for replayed in [false, true] {
        let response = fixture
            .api
            .post_as(&update_path(&fixture), "alice", "acme", &validator)
            .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = json_of(response).await;
        assert_eq!(body["details"]["reason"], "UPDATE_REJECTED");
        assert_eq!(body["details"]["replayed"], replayed);
        assert_eq!(body["details"]["beforeStateRevision"], 0);
        assert_eq!(body["details"]["afterStateRevision"], 0);
        assert_eq!(body["details"]["runVersion"], 1);
    }

    let revision = fixture
        .api
        .post_as(
            &update_path(&fixture),
            "alice",
            "acme",
            &json!({
                "updateKey": "stale-revision",
                "args": {"allowed": true, "nextState": {"balance": 91}},
                "expectedStateRevision": 9
            }),
        )
        .await;
    assert_eq!(revision.status(), StatusCode::CONFLICT);
    let revision = json_of(revision).await;
    assert_eq!(revision["details"]["reason"], "REVISION_CONFLICT");
    assert_eq!(revision["details"]["expectedStateRevision"], 9);
    assert_eq!(revision["details"]["actualStateRevision"], 0);

    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT run_version, \
                (SELECT count(*) FROM operation_update_receipts x WHERE x.tenant_id=r.tenant_id AND x.resolved_run_id=r.id), \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id), \
                (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id) \
         FROM runs r WHERE tenant_id=$1 AND id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("rejection counts");
    assert_eq!(counts, (1, 2, 1, 0));
}

#[tokio::test]
async fn outbox_failure_rolls_back_the_http_update_and_the_same_key_can_retry() {
    let fixture = fixture("http-declared-rollback").await;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_declared_update_hint() RETURNS trigger LANGUAGE plpgsql AS $$ \
           BEGIN \
             IF NEW.event_type = 'run.state_updated' THEN \
               RAISE EXCEPTION 'injected declared update outbox failure'; \
             END IF; \
             RETURN NEW; \
           END $$; \
         CREATE TRIGGER reject_declared_update_hint BEFORE INSERT ON event_outbox \
           FOR EACH ROW EXECUTE FUNCTION reject_declared_update_hint();",
    )
    .execute(&fixture.api.db.pool)
    .await
    .expect("install transaction fault");
    let body = json!({
        "updateKey": "retry-after-rollback",
        "args": {"allowed": true, "nextState": {"balance": 777}}
    });
    let failed = fixture
        .api
        .post_as(&update_path(&fixture), "alice", "acme", &body)
        .await;
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(failed.headers()["cache-control"], "no-store");
    let failed = json_of(failed).await;
    assert_eq!(failed["code"], "INTERNAL");
    assert!(!failed.to_string().contains("777"));
    assert!(!failed.to_string().contains("injected"));

    let rolled_back: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT r.run_version, s.state_revision, \
                (SELECT count(*) FROM operation_update_receipts x WHERE x.tenant_id=r.tenant_id AND x.resolved_run_id=r.id), \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.event_type='state.updated'), \
                (SELECT count(*) FROM audit_records a WHERE a.tenant_id=r.tenant_id AND a.resource_id=r.id::text AND a.action='run.updated') \
         FROM runs r JOIN workflow_operation_states s \
           ON s.tenant_id=r.tenant_id AND s.run_id=r.id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("rolled back update");
    assert_eq!(rolled_back, (1, 0, 0, 0, 0));

    sqlx::raw_sql(
        "DROP TRIGGER reject_declared_update_hint ON event_outbox; \
         DROP FUNCTION reject_declared_update_hint();",
    )
    .execute(&fixture.api.db.pool)
    .await
    .expect("remove transaction fault");
    let retry = fixture
        .api
        .post_as(&update_path(&fixture), "alice", "acme", &body)
        .await;
    assert_eq!(retry.status(), StatusCode::OK);
    let retry = json_of(retry).await;
    assert_eq!(retry["result"], 777);
    assert_eq!(retry["replayed"], false);
}

#[tokio::test]
async fn current_permissions_and_tenant_boundary_guard_results_and_replays() {
    let fixture = fixture("http-declared-authorization").await;
    add_member(&fixture, "olga", Role::Operator).await;
    add_member(&fixture, "vera", Role::Viewer).await;

    let viewer = fixture
        .api
        .post_as(
            &update_path(&fixture),
            "vera",
            "acme",
            &json!({"updateKey": "invalid key", "args": null}),
        )
        .await;
    assert_eq!(viewer.status(), StatusCode::FORBIDDEN);
    assert_eq!(viewer.headers()["cache-control"], "no-store");

    let update = json!({
        "updateKey": "authorized-before-revocation",
        "args": {"allowed": true, "nextState": {"balance": 31}}
    });
    let applied = fixture
        .api
        .post_as(&update_path(&fixture), "olga", "acme", &update)
        .await;
    assert_eq!(applied.status(), StatusCode::OK);

    sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject='olga'")
        .bind(fixture.tenant.as_uuid())
        .execute(&fixture.api.db.pool)
        .await
        .expect("revoke operation grants");
    let replay = fixture
        .api
        .post_as(&update_path(&fixture), "olga", "acme", &update)
        .await;
    assert_eq!(replay.status(), StatusCode::FORBIDDEN);
    assert_eq!(replay.headers()["cache-control"], "no-store");

    let foreign = fixture
        .api
        .post_as(&query_path(&fixture), "bob", "globex", &json!({"args": 1}))
        .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    let foreign = json_of(foreign).await;
    assert_eq!(foreign["code"], "NOT_FOUND");
    assert!(!foreign.to_string().contains("payments"));
    assert!(!foreign.to_string().contains("acme"));
}

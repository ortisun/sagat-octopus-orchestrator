//! Public HTTP evidence for tenant-scoped execution-chain commands.
#![allow(clippy::expect_used)]

use reqwest::{Method, StatusCode};
use sagat_application::chain_progress::ChainInitialization;
use sagat_domain::execution::{ChainPolicy, ChainScope, RunId};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_infra_postgres::initialize_chain_in_transaction;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support;
use crate::support::api::{TestApi, json_of};

struct Fixture {
    api: TestApi,
    tenant: TenantId,
    namespace: NamespaceId,
    chain: RunId,
}

fn waiting_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "wait",
        "nodes": [
            {
                "id": "wait",
                "type": "waitEvent",
                "eventType": "chain.release",
                "correlationExpr": "input.key",
                "timeoutMs": 600000,
                "next": "done"
            },
            {"id": "done", "type": "end"}
        ]
    })
}

fn activity_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [{"id": "work", "type": "activity", "activity": "chain.work"}]
    })
}

fn continuation_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {
                "id": "work",
                "type": "activity",
                "activity": "chain.work",
                "next": "rollover"
            },
            {
                "id": "rollover",
                "type": "continueAsNew",
                "input": "input.next"
            }
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    })
}

#[tokio::test]
async fn public_start_creates_the_initial_chain_atomically() {
    let api = support::api::start().await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "chains").await;
    let namespace: NamespaceId = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await
        .parse()
        .expect("namespace id");
    let name = format!("public-chain-{}", Uuid::now_v7());
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": continuation_document(&name)}))
        .send()
        .await
        .expect("create workflow");
    assert_eq!(created.status(), StatusCode::CREATED);
    let workflow = json_of(created).await["id"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    let published = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("publish workflow");
    assert_eq!(published.status(), StatusCode::CREATED);
    let before = support::now();
    let started = api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({
            "workflowId": workflow,
            "input": {"segment": 1, "next": {"segment": 2}}
        }))
        .send()
        .await
        .expect("start run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let chain: RunId = json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("run id");

    let projection: (
        Uuid,
        i32,
        i32,
        time::OffsetDateTime,
        Option<time::OffsetDateTime>,
    ) = sqlx::query_as(
        "SELECT current_run_id,segment_count,max_segments,original_deadline_at,r.deadline_at \
             FROM execution_chains ec JOIN runs r ON r.tenant_id=ec.tenant_id AND r.id=ec.chain_id \
             WHERE ec.tenant_id=$1 AND ec.namespace_id=$2 AND ec.chain_id=$3",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("initial chain projection");
    assert_eq!(projection.0, chain.as_uuid());
    assert_eq!(projection.1, 1);
    assert_eq!(projection.2, 3);
    assert_eq!(projection.4, Some(projection.3));
    assert!(projection.3 >= before + time::Duration::minutes(59));
    assert!(projection.3 <= before + time::Duration::minutes(61));
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM run_events WHERE tenant_id=$1 AND run_id=$2 ORDER BY seq",
    )
    .bind(tenant.as_uuid())
    .bind(chain.as_uuid())
    .fetch_all(&api.db.pool)
    .await
    .expect("initial chain journal");
    assert_eq!(
        events,
        ["run.started", "step.released", "execution_chain.started"]
    );

    let inspection = api
        .get_as(
            &format!("/namespaces/{namespace}/chains/{chain}"),
            "alice",
            "acme",
        )
        .await;
    assert_eq!(inspection.status(), StatusCode::OK);
    let inspection = json_of(inspection).await;
    assert_eq!(inspection["chainId"], chain.to_string());
    assert_eq!(inspection["currentRunId"], chain.to_string());
    assert_eq!(
        inspection["segments"].as_array().expect("segments").len(),
        1
    );
}

async fn fixture(name: &str, document: Value) -> Fixture {
    let api = support::api::start().await;
    let tenant = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    let workspace = api.create_workspace("alice", "acme", "chains").await;
    let namespace: NamespaceId = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await
        .parse()
        .expect("namespace id");
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": document}))
        .send()
        .await
        .expect("create workflow");
    assert_eq!(created.status(), StatusCode::CREATED);
    let workflow = json_of(created).await["id"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    let published = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("publish workflow");
    assert_eq!(published.status(), StatusCode::CREATED);
    let deadline = (support::now() + time::Duration::hours(1))
        .format(&time::format_description::well_known::Rfc3339)
        .expect("deadline");
    let started = api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({
            "workflowId": workflow,
            "input": {"key": "order-1"},
            "deadline": deadline
        }))
        .send()
        .await
        .expect("start run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let chain: RunId = json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("run id");
    let mut tx = api.db.pool.begin().await.expect("chain transaction");
    initialize_chain_in_transaction(
        &mut tx,
        &ChainInitialization::new(
            ChainScope::new(tenant, namespace),
            chain,
            ChainPolicy::new(3).expect("chain policy"),
        ),
    )
    .await
    .expect("initialize chain");
    tx.commit().await.expect("chain commit");
    Fixture {
        api,
        tenant,
        namespace,
        chain,
    }
}

fn chain_path(fixture: &Fixture) -> String {
    format!("/namespaces/{}/chains/{}", fixture.namespace, fixture.chain)
}

#[tokio::test]
async fn inspect_and_signal_are_tenant_scoped_and_signal_replays_exactly() {
    let fixture = fixture("http-chain-signal", waiting_document("http-chain-signal")).await;
    let inspection = fixture
        .api
        .get_as(&chain_path(&fixture), "alice", "acme")
        .await;
    assert_eq!(inspection.status(), StatusCode::OK);
    assert_eq!(inspection.headers()["cache-control"], "no-store");
    let inspection = json_of(inspection).await;
    assert_eq!(inspection["chainId"], fixture.chain.to_string());
    assert_eq!(inspection["currentRunId"], fixture.chain.to_string());
    assert_eq!(inspection["revision"], 0);
    assert_eq!(
        inspection["segments"].as_array().expect("segments").len(),
        1
    );

    let foreign = fixture
        .api
        .get_as(&chain_path(&fixture), "bob", "globex")
        .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    fixture.api.executions.flush_denial_writes().await;
    let denied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM audit_records WHERE tenant_id<>$1 \
         AND action='access.denied.cross_tenant' AND resource_id=$2)",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.chain.to_string())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("denial audit");
    assert!(denied);

    let path = format!("{}/signals/chain.release", chain_path(&fixture));
    let key = "http-chain-signal-0001";
    let body = json!({
        "expectedRunId": fixture.chain.to_string(),
        "payload": {"released": true}
    });
    let first = fixture
        .api
        .as_user(Method::POST, &path, "alice", "acme")
        .header("idempotency-key", key)
        .json(&body)
        .send()
        .await
        .expect("signal chain");
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    assert_eq!(first.headers()["cache-control"], "no-store");
    let first = json_of(first).await;
    assert_eq!(first["deliveredTo"], json!(["wait"]));

    let replay = fixture
        .api
        .as_user(Method::POST, &path, "alice", "acme")
        .header("idempotency-key", key)
        .json(&body)
        .send()
        .await
        .expect("replay signal");
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(json_of(replay).await, first);
    let reused = fixture
        .api
        .as_user(Method::POST, &path, "alice", "acme")
        .header("idempotency-key", key)
        .json(&json!({
            "expectedRunId": fixture.chain.to_string(),
            "payload": {"released": false}
        }))
        .send()
        .await
        .expect("reuse signal key");
    assert_eq!(reused.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_of(reused).await["details"]["reason"],
        "IDEMPOTENCY_KEY_REUSED"
    );
    let journal_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
         AND event_type='wait.satisfied'",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("signal journal count");
    assert_eq!(journal_count, 1);
}

#[tokio::test]
async fn cancel_requires_the_current_head_and_replays_without_a_second_transition() {
    let fixture = fixture("http-chain-cancel", activity_document("http-chain-cancel")).await;
    let path = format!("{}/cancel", chain_path(&fixture));
    let stale = fixture
        .api
        .as_user(Method::POST, &path, "alice", "acme")
        .header("idempotency-key", "http-chain-stale-0001")
        .json(&json!({
            "expectedRunId": RunId::generate().to_string(),
            "reason": "operator requested cancellation"
        }))
        .send()
        .await
        .expect("stale cancel");
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_of(stale).await["details"]["reason"],
        "CHAIN_HEAD_CHANGED"
    );

    let body = json!({
        "expectedRunId": fixture.chain.to_string(),
        "reason": "operator requested cancellation"
    });
    let key = "http-chain-cancel-0001";
    for _ in 0..2 {
        let response = fixture
            .api
            .as_user(Method::POST, &path, "alice", "acme")
            .header("idempotency-key", key)
            .json(&body)
            .send()
            .await
            .expect("cancel chain");
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let projection: (bool, Option<String>, i64, String) = sqlx::query_as(
        "SELECT ec.cancel_requested,ec.final_outcome,ec.revision,r.status \
         FROM execution_chains ec JOIN runs r \
           ON r.tenant_id=ec.tenant_id AND r.id=ec.current_run_id \
         WHERE ec.tenant_id=$1 AND ec.chain_id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.chain.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("cancelled projection");
    assert_eq!(
        projection,
        (
            true,
            Some("CANCELLED".to_owned()),
            2,
            "CANCELLED".to_owned()
        )
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 \
         AND resource_id=$2 AND action='run.cancel_requested'",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.chain.to_string())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("cancel audit count");
    assert_eq!(audits, 1);
}

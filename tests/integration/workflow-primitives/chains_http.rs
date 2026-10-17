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
    fixture_with_segments(name, document, 3).await
}

async fn fixture_with_segments(name: &str, document: Value, max_segments: u16) -> Fixture {
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
            ChainPolicy::new(max_segments).expect("chain policy"),
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

/// Appends committed segments the way a hand-off leaves them: every earlier segment `CONTINUED`
/// and finished, the last one the open head.
async fn grow_chain_to(fixture: &Fixture, total: i32) {
    let pool = &fixture.api.db.pool;
    let tenant = fixture.tenant.as_uuid();
    let namespace = fixture.namespace.as_uuid();
    let chain = fixture.chain.as_uuid();
    let present: i32 = sqlx::query_scalar(
        "SELECT segment_count FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(tenant)
    .bind(chain)
    .fetch_one(pool)
    .await
    .expect("segment count");
    for index in present..total {
        let previous: Uuid = sqlx::query_scalar(
            "SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
        )
        .bind(tenant)
        .bind(chain)
        .fetch_one(pool)
        .await
        .expect("head");
        let next = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO runs (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status, \
                               run_version,input_digest,started_by,created_at,updated_at, \
                               deadline_at,root_run_id,depth) \
             SELECT $3,tenant_id,namespace_id,workflow_id,workflow_version_id,'RUNNING',3, \
                    input_digest,started_by,now(),now(),deadline_at,$3,0 \
             FROM runs WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant)
        .bind(chain)
        .bind(next)
        .execute(pool)
        .await
        .expect("successor run");
        sqlx::query(
            "UPDATE runs SET status='SUCCEEDED',finished_at=now(),updated_at=now() \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant)
        .bind(previous)
        .execute(pool)
        .await
        .expect("finish predecessor");
        sqlx::query(
            "INSERT INTO chain_segments (tenant_id,namespace_id,chain_id,segment_index,run_id, \
                                         predecessor_run_id,created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,now())",
        )
        .bind(tenant)
        .bind(namespace)
        .bind(chain)
        .bind(index)
        .bind(next)
        .bind(previous)
        .execute(pool)
        .await
        .expect("successor segment");
        sqlx::query(
            "UPDATE chain_segments SET completion_kind='CONTINUED',successor_run_id=$4, \
                    transfer_input_digest=(SELECT input_digest FROM runs WHERE tenant_id=$1 AND id=$3), \
                    completed_at=now() \
             WHERE tenant_id=$1 AND chain_id=$2 AND run_id=$3",
        )
        .bind(tenant)
        .bind(chain)
        .bind(previous)
        .bind(next)
        .execute(pool)
        .await
        .expect("complete predecessor segment");
        sqlx::query(
            "UPDATE execution_chains SET current_run_id=$3,segment_count=segment_count+1, \
                    revision=revision+1,updated_at=now() \
             WHERE tenant_id=$1 AND chain_id=$2",
        )
        .bind(tenant)
        .bind(chain)
        .bind(next)
        .execute(pool)
        .await
        .expect("move head");
    }
}

async fn runs_page(
    fixture: &Fixture,
    query: &str,
    subject: &str,
    tenant: &str,
) -> reqwest::Response {
    fixture
        .api
        .get_as(
            &format!("{}/runs{query}", chain_path(fixture)),
            subject,
            tenant,
        )
        .await
}

#[tokio::test]
async fn the_runs_of_a_chain_page_in_segment_order_with_a_stable_bounded_cursor() {
    let fixture =
        fixture_with_segments("http-chain-runs", waiting_document("http-chain-runs"), 10).await;
    grow_chain_to(&fixture, 5).await;

    let first = runs_page(&fixture, "?limit=2", "alice", "acme").await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["cache-control"], "no-store");
    let first = json_of(first).await;
    let items = first["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["index"], 0);
    assert_eq!(items[0]["runId"], fixture.chain.to_string());
    assert_eq!(items[0]["completionKind"], "CONTINUED");
    assert_eq!(items[0]["status"], "SUCCEEDED");
    assert_eq!(items[0]["predecessorRunId"], Value::Null);
    assert_eq!(items[1]["index"], 1);
    assert_eq!(items[1]["predecessorRunId"], items[0]["runId"]);
    assert_eq!(items[0]["successorRunId"], items[1]["runId"]);
    assert!(
        items[0].as_object().expect("item").keys().all(|key| [
            "index",
            "runId",
            "status",
            "completionKind",
            "predecessorRunId",
            "successorRunId",
            "createdAt",
            "finishedAt"
        ]
        .contains(&key.as_str())),
        "a page carries no payloads: {}",
        items[0]
    );
    let cursor = first["nextCursor"]
        .as_str()
        .expect("a next page")
        .to_owned();

    // A segment appended between pages neither repeats nor skips one.
    grow_chain_to(&fixture, 6).await;
    let second = json_of(
        runs_page(
            &fixture,
            &format!("?limit=2&cursor={cursor}"),
            "alice",
            "acme",
        )
        .await,
    )
    .await;
    let indexes = |page: &Value| {
        page["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["index"].as_i64().expect("index"))
            .collect::<Vec<_>>()
    };
    assert_eq!(indexes(&second), [2, 3]);
    let cursor = second["nextCursor"]
        .as_str()
        .expect("third page")
        .to_owned();
    let third = json_of(
        runs_page(
            &fixture,
            &format!("?limit=2&cursor={cursor}"),
            "alice",
            "acme",
        )
        .await,
    )
    .await;
    assert_eq!(indexes(&third), [4, 5]);
    assert_eq!(third["nextCursor"], Value::Null);
    let open = third["items"][1].clone();
    assert_eq!(
        open["completionKind"],
        Value::Null,
        "the head is still open"
    );
    assert_eq!(open["status"], "RUNNING");

    // Page size is bounded: the default and the clamps.
    let all = json_of(runs_page(&fixture, "", "alice", "acme").await).await;
    assert_eq!(indexes(&all), [0, 1, 2, 3, 4, 5]);
    let clamped = json_of(runs_page(&fixture, "?limit=0", "alice", "acme").await).await;
    assert_eq!(indexes(&clamped), [0], "a limit below one is one");
    let huge = json_of(runs_page(&fixture, "?limit=100000", "alice", "acme").await).await;
    assert!(huge["items"].as_array().expect("items").len() <= 200);

    // A cursor is opaque and checked.
    for bad in ["?cursor=not-a-cursor", "?cursor=", "?limit=abc"] {
        let response = runs_page(&fixture, bad, "alice", "acme").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn the_runs_of_a_chain_are_tenant_opaque_and_need_authentication() {
    let fixture = fixture_with_segments(
        "http-chain-runs-opacity",
        waiting_document("http-chain-runs-opacity"),
        10,
    )
    .await;
    grow_chain_to(&fixture, 2).await;
    let foreign = runs_page(&fixture, "", "bob", "globex").await;
    let missing = fixture
        .api
        .get_as(
            &format!(
                "/namespaces/{}/chains/{}/runs",
                fixture.namespace,
                RunId::generate()
            ),
            "bob",
            "globex",
        )
        .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let foreign_body = json_of(foreign).await;
    let missing_body = json_of(missing).await;
    assert_eq!(foreign_body["code"], missing_body["code"]);
    assert_eq!(foreign_body["message"], missing_body["message"]);
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
    assert!(denied, "a foreign probe is audited like a chain inspection");

    let anonymous = fixture
        .api
        .request(Method::GET, &format!("{}/runs", chain_path(&fixture)))
        .send()
        .await
        .expect("anonymous request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_run_read_names_its_segment_and_whether_it_continued() {
    let fixture = fixture_with_segments(
        "http-chain-run-read",
        waiting_document("http-chain-run-read"),
        10,
    )
    .await;
    grow_chain_to(&fixture, 3).await;
    let segments = json_of(runs_page(&fixture, "", "alice", "acme").await).await;
    let segment = |index: usize| segments["items"][index].clone();

    // The segment wait of the SDKs reads the run: SUCCEEDED alone must not read as the process end.
    let first = json_of(
        fixture
            .api
            .get_as(
                &format!("/runs/{}", segment(0)["runId"].as_str().expect("id")),
                "alice",
                "acme",
            )
            .await,
    )
    .await;
    assert_eq!(first["status"], "SUCCEEDED");
    assert_eq!(
        first["chain"],
        json!({
            "chainId": fixture.chain.to_string(),
            "segmentIndex": 0,
            "completionKind": "CONTINUED",
            "successorRunId": segment(1)["runId"]
        })
    );
    let head = json_of(
        fixture
            .api
            .get_as(
                &format!("/runs/{}", segment(2)["runId"].as_str().expect("id")),
                "alice",
                "acme",
            )
            .await,
    )
    .await;
    assert_eq!(head["chain"]["segmentIndex"], 2);
    assert_eq!(head["chain"]["completionKind"], Value::Null);
    assert_eq!(head["chain"]["successorRunId"], Value::Null);

    // The same metadata is on a listed run, and a run outside any chain has no such key.
    let listed = json_of(
        fixture
            .api
            .as_user(Method::GET, "/runs", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .send()
            .await
            .expect("list runs"),
    )
    .await;
    let in_list = listed["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|run| run["runId"] == first["runId"])
        .expect("the first segment is listed");
    assert_eq!(in_list["chain"], first["chain"]);
}

#[tokio::test]
async fn a_run_outside_any_chain_has_no_chain_key() {
    let fixture = fixture("http-chain-plain", activity_document("http-chain-plain")).await;
    // `fixture` made the run a chain on purpose; a second workflow run is not.
    let name = "http-chain-plain-run";
    let created = fixture
        .api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": activity_document(name)}))
        .send()
        .await
        .expect("create workflow");
    let workflow = json_of(created).await["id"]
        .as_str()
        .expect("id")
        .to_owned();
    fixture
        .api
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
        .expect("publish");
    let started = json_of(
        fixture
            .api
            .as_user(Method::POST, "/runs", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"workflowId": workflow, "input": {}}))
            .send()
            .await
            .expect("start"),
    )
    .await;
    let run = fixture
        .api
        .get_as(
            &format!("/runs/{}", started["runId"].as_str().expect("id")),
            "alice",
            "acme",
        )
        .await;
    let run = json_of(run).await;
    assert!(run.get("chain").is_none(), "{run}");
}

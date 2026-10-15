//! Declared-operation response loss, serialization and disclosure boundaries over public HTTP.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::{Method, StatusCode};
use sagat_domain::execution::RunId;
use sagat_domain::identity::TenantId;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support;
use crate::support::api::{TestApi, json_of};

const CANARY: &str = "sagat-secret-canary-operations-7c692e9b";

struct Fixture {
    api: TestApi,
    tenant: TenantId,
    run: RunId,
}

fn document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "wait",
        "nodes": [
            {
                "id": "wait",
                "type": "waitEvent",
                "eventType": "account.release",
                "correlationExpr": "input.key",
                "timeoutMs": 600_000,
                "next": "hold"
            },
            {"id": "hold", "type": "activity", "activity": "operations.unserved"}
        ],
        "state": {
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"balance": {"type": "integer", "minimum": 0}},
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
                                "properties": {
                                    "balance": {"type": "integer", "minimum": 0}
                                },
                                "required": ["balance"]
                            }
                        },
                        "required": ["allowed", "nextState"]
                    },
                    "validate": "args.allowed",
                    "state": "args.nextState",
                    "result": "newState.balance",
                    "resultSchema": {"type": "integer", "minimum": 0},
                    "disclosure": "declared-business-result-v1"
                }
            }
        }
    })
}

async fn fixture(name: &str) -> Fixture {
    let api = support::api::start().await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "operations").await;
    api.create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": document(name)}))
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
            "input": {
                "key": "account-7",
                "account": {"balance": 10},
                "privateCanary": CANARY
            },
            "deadline": deadline
        }))
        .send()
        .await
        .expect("start run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let run = json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("run id");
    Fixture { api, tenant, run }
}

fn query_path(fixture: &Fixture) -> String {
    format!("/runs/{}/queries/balance.read", fixture.run)
}

fn update_path(fixture: &Fixture) -> String {
    format!("/runs/{}/updates/balance.replace", fixture.run)
}

fn update_body(key: &str, balance: u64) -> Value {
    json!({
        "updateKey": key,
        "args": {"allowed": true, "nextState": {"balance": balance}},
        "expectedStateRevision": 0
    })
}

#[tokio::test]
async fn lost_update_response_replays_the_committed_receipt_without_a_second_mutation() {
    let fixture = fixture("operation-lost-response").await;
    let body = update_body("lost-response-key", 41);
    let response = fixture
        .api
        .as_user(Method::POST, &update_path(&fixture), "alice", "acme")
        .json(&body)
        .send()
        .await
        .expect("server commits before the client loses the response");
    drop(response);

    let replay = fixture
        .api
        .post_as(&update_path(&fixture), "alice", "acme", &body)
        .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = json_of(replay).await;
    assert_eq!(replay["status"], "APPLIED");
    assert_eq!(replay["result"], 41);
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["beforeStateRevision"], 0);
    assert_eq!(replay["afterStateRevision"], 1);

    let durable: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT s.state_revision, \
                (SELECT count(*) FROM operation_update_receipts x \
                 WHERE x.tenant_id=s.tenant_id AND x.resolved_run_id=s.run_id), \
                (SELECT count(*) FROM run_events e \
                 WHERE e.tenant_id=s.tenant_id AND e.run_id=s.run_id \
                   AND e.event_type='state.updated'), \
                (SELECT count(*) FROM event_outbox o \
                 WHERE o.tenant_id=s.tenant_id AND o.aggregate_id=s.run_id \
                   AND o.event_type='run.state_updated') \
         FROM workflow_operation_states s WHERE s.tenant_id=$1 AND s.run_id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("durable update evidence");
    assert_eq!(durable, (1, 1, 1, 1));
}

#[tokio::test]
async fn update_and_signal_race_serialize_without_losing_either_transition() {
    for iteration in 0..5 {
        let fixture = fixture(&format!("operation-signal-race-{iteration}")).await;
        let update_body = update_body("signal-race-update", 52);
        let update = fixture
            .api
            .as_user(Method::POST, &update_path(&fixture), "alice", "acme")
            .json(&update_body)
            .send();
        let signal = fixture
            .api
            .as_user(
                Method::POST,
                &format!("/runs/{}/signals/account.release", fixture.run),
                "alice",
                "acme",
            )
            .header("idempotency-key", "signal-race-delivery")
            .json(&json!({"released": true}))
            .send();
        let (updated, signalled) = tokio::join!(update, signal);
        let updated = updated.expect("update response");
        let signalled = signalled.expect("signal response");
        assert_eq!(updated.status(), StatusCode::OK);
        assert_eq!(signalled.status(), StatusCode::ACCEPTED);
        assert_eq!(json_of(updated).await["result"], 52);
        assert_eq!(json_of(signalled).await["deliveredTo"], json!(["wait"]));

        let durable: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT s.state_revision, \
                (SELECT count(*) FROM operation_update_receipts x \
                 WHERE x.tenant_id=s.tenant_id AND x.resolved_run_id=s.run_id), \
                (SELECT count(*) FROM run_events e \
                 WHERE e.tenant_id=s.tenant_id AND e.run_id=s.run_id \
                   AND e.event_type='state.updated'), \
                (SELECT count(*) FROM run_events e \
                 WHERE e.tenant_id=s.tenant_id AND e.run_id=s.run_id \
                   AND e.event_type='wait.satisfied') \
         FROM workflow_operation_states s WHERE s.tenant_id=$1 AND s.run_id=$2",
        )
        .bind(fixture.tenant.as_uuid())
        .bind(fixture.run.as_uuid())
        .fetch_one(&fixture.api.db.pool)
        .await
        .expect("serialized transitions");
        assert_eq!(durable, (1, 1, 1, 1));
    }
}

#[tokio::test]
async fn undeclared_input_canary_never_crosses_the_operation_response_or_evidence_boundary() {
    let fixture = fixture("operation-canary").await;
    let query = fixture
        .api
        .post_as(&query_path(&fixture), "alice", "acme", &json!({"args": 3}))
        .await;
    assert_eq!(query.status(), StatusCode::OK);
    let query = json_of(query).await;
    assert_eq!(query["result"], 13);
    assert!(!query.to_string().contains(CANARY));

    let update = fixture
        .api
        .post_as(
            &update_path(&fixture),
            "alice",
            "acme",
            &update_body("canary-boundary-update", 63),
        )
        .await;
    assert_eq!(update.status(), StatusCode::OK);
    let update = json_of(update).await;
    assert_eq!(update["result"], 63);
    assert!(!update.to_string().contains(CANARY));

    let leaked: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM audit_records a WHERE a.tenant_id=$1 \
             AND a.resource_id=$2 AND position($3 in coalesce(a.reason,'')) > 0), \
            (SELECT count(*) FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$4 \
             AND position($3 in e.detail::text) > 0), \
            (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=$1 AND o.aggregate_id=$4 \
             AND position($3 in o.payload::text) > 0), \
            (SELECT count(*) FROM operation_update_receipts r \
             WHERE r.tenant_id=$1 AND r.resolved_run_id=$4 \
             AND position($3 in concat_ws('|', r.operation_name, r.update_key, r.actor, \
                                           coalesce(r.rejection_code,''))) > 0), \
            (SELECT count(*) FROM operation_update_receipts r \
             JOIN payloads p ON p.tenant_id=r.tenant_id AND p.digest=r.result_digest \
             WHERE r.tenant_id=$1 AND r.resolved_run_id=$4 \
               AND position($3 in convert_from(p.content, 'UTF8')) > 0), \
            (SELECT count(*) FROM workflow_operation_states s \
             JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.state_digest \
             WHERE s.tenant_id=$1 AND s.run_id=$4 \
               AND position($3 in convert_from(p.content, 'UTF8')) > 0)",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.to_string())
    .bind(CANARY)
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("inspect non-payload evidence");
    assert_eq!(leaked, (0, 0, 0, 0, 0, 0));
}

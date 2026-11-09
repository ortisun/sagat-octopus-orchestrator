//! A lake step writes what a SQL step selected, on the wire (feature 035, D035-145, G035-067): a real control plane over real PostgreSQL,
//! the REST API that creates the connections and the secrets and publishes and starts the run, and the worker gRPC. Two gRPC clients stand
//! in for the two runners: the SQL runner leases the step that selects, and completes it with the typed result; the lake runner is then
//! leased the step that writes, with the rows of that result as its records.
//!
//! It runs inside the lifecycle scenario of `main.rs`, with the OpenBao of that scenario.
#![allow(clippy::expect_used, clippy::panic)]
use super::lake::{append, rows_schema};
use crate::support::{
    api::Options,
    engine::document,
    worker::{HUMAN, Platform, Worker, platform_with},
};
use reqwest::Method;
use sagat_application::secret_provider::SecretProvider;
use sagat_domain::{
    database::{ICEBERG_WRITE_ACTIVITY, MANAGED_DATABASE_ACTIVITY},
    identity::Role,
};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// The workflow: a statement that selects, and the write of its result.
fn selecting(name: &str) -> Value {
    document(
        name,
        "read",
        vec![
            json!({
                "id": "read", "type": "database", "connectionRef": {"name": "source-db"},
                "operation": "query", "statement": "SELECT id, total FROM orders", "next": "save"
            }),
            json!({
                "id": "save", "type": "icebergWrite", "connectionRef": {"name": "result-warehouse"},
                "table": {"namespace": ["sales"], "name": "orders"}, "mode": "append",
                "records": {"databaseResult": "read"},
                "mapping": [
                    {"field": "/id", "column": "id"},
                    {"field": "/total", "column": "total"}
                ],
                "next": "elsewhere"
            }),
            // A statement of its own, on a connection of its own, after the write: what the write is admitted against is its own
            // connection and the one its rows came from, and not every connection of the workflow.
            json!({
                "id": "elsewhere", "type": "database", "connectionRef": {"name": "other-db"},
                "operation": "query", "statement": "SELECT 2"
            }),
        ],
    )
}

/// The typed result of a statement that selected two rows.
fn two_rows() -> Value {
    json!({
        "columns": [{"name": "id", "type": "text"}, {"name": "total", "type": "integer"}],
        "rows": [
            [{"type": "text", "value": "r-1"}, {"type": "integer", "value": "12"}],
            [{"type": "text", "value": "r-2"}, {"type": "integer", "value": "9007199254740993"}]
        ],
        "affectedRows": "2",
        "truncated": false
    })
}

async fn failures(p: &Platform, run: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT detail->>'code' FROM run_events WHERE tenant_id = $1 AND run_id = $2::uuid AND event_type = 'step.failed'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("the failures")
}

async fn result_reads(p: &Platform, run: &str) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT resource_id, reason FROM audit_records WHERE tenant_id = $1 AND action = 'database.result_read' AND resource_id = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("the audit records")
}

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let p = platform_with(Options {
        secret_provider: Some(provider),
        lake: true,
        ..Default::default()
    })
    .await;
    let ns = p.namespace_id().await;
    for (name, value) in [
        (
            "result-catalog",
            r#"{"clientId":"wire-client","clientSecret":"WIRE_CATALOG_CANARY"}"#,
        ),
        (
            "result-storage",
            r#"{"accessKeyId":"WIRE_KEY","secretAccessKey":"WIRE_STORAGE_CANARY"}"#,
        ),
        (
            "source-credential",
            r#"{"username":"reader","password":"WIRE_SOURCE_CANARY"}"#,
        ),
    ] {
        let (status, body) = p
            .rest(
                Method::PUT,
                &format!("/namespaces/{ns}/secrets/{name}"),
                Some(&json!({
                    "value": value, "requestId": Uuid::new_v4().to_string(),
                    "expectedVersion": 0, "requiredProvider": "openbao"
                })),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }
    let source = |status: &str, expected_revision: u32| {
        json!({
            "requestId": Uuid::new_v4().to_string(), "expectedRevision": expected_revision, "status": status,
            "config": {
                "driver": "postgresql", "host": "db.example.com", "port": 5432, "database": "test",
                "tls": {"mode": "verify-full"}, "secretName": "source-credential"
            }
        })
    };
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/source-db"),
            Some(&source("ACTIVE", 0)),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let other = |status: &str, expected_revision: u32| {
        json!({
            "requestId": Uuid::new_v4().to_string(), "expectedRevision": expected_revision, "status": status,
            "config": {
                "driver": "postgresql", "host": "other.example.com", "port": 5432, "database": "test",
                "tls": {"mode": "verify-full"}, "secretName": "source-credential"
            }
        })
    };
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/other-db"),
            Some(&other("ACTIVE", 0)),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    // Every field of a result is `pii` for the egress check, so the connection that takes one has that ceiling, and the administrator
    // who writes it holds the reveal grant of the namespace.
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/payload-reveal-grants/{HUMAN}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/result-warehouse"),
            Some(&json!({
                "requestId": Uuid::new_v4().to_string(), "expectedRevision": 0, "status": "ACTIVE",
                "config": {
                    "driver": "iceberg",
                    "catalog": {"type": "rest", "rest": {
                        "uri": "https://catalog.example/api", "warehouse": "analytics",
                        "auth": {"type": "oauth2", "tokenUri": "https://catalog.example/oauth/token"},
                        "accessDelegation": "none", "remoteSigning": false
                    }},
                    "storage": {"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": false}, "credentials": "static"},
                    "warehouseLocations": ["s3://lake/warehouse/"],
                    "allowedModes": ["append"],
                    "egressCeiling": "pii",
                    "catalogSecret": "result-catalog",
                    "storageSecret": "result-storage",
                    "limits": {}
                }
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let workflow = p
        .publish("lake-result-wire", &selecting("lake-result-wire"))
        .await;

    // The two runners: an identity of the database profile for the statement, one of the lake profile for the write.
    p.grant(p.tenant, "sql-result-wire", Role::DatabaseRunner)
        .await;
    p.bind_everything("sql-result-wire").await;
    let mut sql = p.connect("sql-result-wire", "acme").await;
    sql.register_version(&[MANAGED_DATABASE_ACTIVITY], "1.6")
        .await;
    p.grant(p.tenant, "lake-result-wire", Role::DatabaseRunner)
        .await;
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/worker-bindings/lake-result-wire"),
            Some(&json!({"activities": [ICEBERG_WRITE_ACTIVITY], "profile": "lake"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut lake = p.connect("lake-result-wire", "acme").await;
    lake.register_version(&[ICEBERG_WRITE_ACTIVITY], "1.6")
        .await;

    // The statement is leased and completes with its result; nothing for the lake runner until it has.
    let run = p.start(&workflow, &json!({})).await;
    assert!(
        lake.poll(8).await.is_empty(),
        "the write waits for the statement"
    );
    let statement = sql.one_task().await;
    assert_eq!(statement.activity_name, MANAGED_DATABASE_ACTIVITY);
    sql.client
        .resolve_database(sql.req(Worker::resolve_secrets_message(&statement)))
        .await
        .expect("the credentials of the statement");
    sql.complete(&statement, &two_rows())
        .await
        .expect("the result of the statement");

    // The rows are already in the run, but the connection they came from is still asked when they are written: paused, it stops the
    // write, which waits (it is refused and the refusal is audited, as for the claim of any managed step) and is leased when the
    // connection is resumed. A revocation that has returned stops every subsequent admission.
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/source-db"),
            Some(&source("PAUSED", 1)),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        lake.poll(8).await.is_empty(),
        "the connection the rows came from is paused: the write waits"
    );
    let refusals: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = 'database.denied' AND resource_id = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(&run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the refusals");
    assert_eq!(
        refusals, 1,
        "the refused claim of the write is audited for the run"
    );
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/source-db"),
            Some(&source("ACTIVE", 2)),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    // A connection of the workflow that the write has nothing to do with, paused, does not stop it.
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/other-db"),
            Some(&other("PAUSED", 1)),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // The write is leased with the rows of the result as its records.
    let write = lake.one_task().await;
    assert_eq!(write.activity_name, ICEBERG_WRITE_ACTIVITY);
    assert_eq!(write.run_id, run);
    let input: Value = serde_json::from_str(&write.input_json).expect("the lease input");
    assert_eq!(
        input["records"]["inline"],
        json!([
            {"id": "r-1", "total": "12"},
            {"id": "r-2", "total": "9007199254740993"}
        ]),
        "the rows of the result, keyed by column, each integer as its digits"
    );
    assert!(
        !write.input_json.contains("CANARY"),
        "no credential travels in the lease"
    );

    // Reading the result for a write is audited once, for the run, as reading it through the API is.
    let reads = result_reads(&p, &run).await;
    assert_eq!(reads.len(), 1, "{reads:?}");
    let reason = reads[0].1.as_deref().expect("a reason");
    for (what, text) in [
        ("the node whose result was read", "node:read"),
        ("the write that read it", "lake write save"),
        ("where it was written", "result-warehouse"),
        ("who started the run", HUMAN),
    ] {
        assert!(reason.contains(text), "the record names {what}: {reason}");
    }
    assert!(
        !format!("{reads:?}").contains("r-1"),
        "the audit record carries no value of the result"
    );

    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/other-db"),
            Some(&other("ACTIVE", 2)),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    // A result that cannot be records fails the write for good, with the code that says why, and leases nothing.
    let second = p.start(&workflow, &json!({})).await;
    let statement = sql.one_task().await;
    assert_eq!(statement.run_id, second);
    sql.client
        .resolve_database(sql.req(Worker::resolve_secrets_message(&statement)))
        .await
        .expect("the credentials of the statement");
    sql.complete(
        &statement,
        &json!({
            "columns": [{"name": "id", "type": "text"}, {"name": "id", "type": "text"}],
            "rows": [[{"type": "text", "value": "a"}, {"type": "text", "value": "b"}]],
            "affectedRows": "1", "truncated": false
        }),
    )
    .await
    .expect("a result of two columns of one name");
    assert!(
        lake.poll(8).await.is_empty(),
        "a write whose records cannot be made is not leased"
    );
    assert_eq!(
        failures(&p, &second).await,
        vec!["DATABASE_CONFIGURATION_INVALID".to_owned()]
    );
    assert_eq!(p.status(&second).await, "FAILED");
    assert!(
        result_reads(&p, &second).await.is_empty(),
        "nothing was read for a write that was not leased"
    );

    // A write whose records are its own reads no result, and records no such read.
    let inline = p
        .publish(
            "lake-result-inline",
            &append(
                "lake-result-inline",
                "result-warehouse",
                &["/id", "/total"],
                Some(rows_schema()),
            ),
        )
        .await;
    let third = p
        .start(&inline, &json!({"rows": [{"id": "r-1", "total": 3}]}))
        .await;
    let own = lake.one_task().await;
    assert_eq!(own.run_id, third);
    assert!(
        result_reads(&p, &third).await.is_empty(),
        "inline records are not the result of a statement"
    );
}

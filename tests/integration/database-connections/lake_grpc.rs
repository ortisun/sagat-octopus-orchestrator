//! The server's half of a lake step on the wire (feature 035, D035-141, D035-142): a real control plane over real PostgreSQL, the
//! REST API that creates the connection and the secrets and publishes and starts the run, and the worker gRPC that leases the
//! step, resolves its connection and completes it. A gRPC client stands in for the lake runner, so what is exercised is everything on
//! the server's side of the wire that the runner's shell reaches (the shell is tested against a scripted service in the runner).
//!
//! It runs inside the lifecycle scenario of `main.rs`, with the OpenBao of that scenario.
#![allow(clippy::expect_used, clippy::panic)]
use super::lake::{append, rows_schema};
use crate::support::{
    api::Options,
    worker::{Worker, platform_with},
};
use reqwest::Method;
use sagat_api_grpc::pb;
use sagat_application::secret_provider::SecretProvider;
use sagat_domain::{database::ICEBERG_WRITE_ACTIVITY, identity::Role};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

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
            "lake-catalog",
            r#"{"clientId":"wire-client","clientSecret":"WIRE_CATALOG_CANARY"}"#,
        ),
        (
            "lake-storage",
            r#"{"accessKeyId":"WIRE_KEY","secretAccessKey":"WIRE_STORAGE_CANARY"}"#,
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
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/warehouse"),
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
                    "catalogSecret": "lake-catalog",
                    "storageSecret": "lake-storage",
                    "limits": {}
                }
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let workflow = p
        .publish(
            "lake-wire",
            &append(
                "lake-wire",
                "warehouse",
                &["/id", "/total"],
                Some(rows_schema()),
            ),
        )
        .await;
    let run = p
        .start(&workflow, &json!({"rows": [{"id": "r-1", "total": 3}]}))
        .await;

    // The runner is an identity of the lake profile, bound to the lake activity.
    p.grant(p.tenant, "lake-wire", Role::DatabaseRunner).await;
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/worker-bindings/lake-wire"),
            Some(&json!({"activities": [ICEBERG_WRITE_ACTIVITY], "profile": "lake"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut worker = p.connect("lake-wire", "acme").await;
    worker
        .register_version(&[ICEBERG_WRITE_ACTIVITY], "1.6")
        .await;
    let lease = worker.one_task().await;
    assert_eq!(lease.activity_name, ICEBERG_WRITE_ACTIVITY);
    assert_eq!(lease.run_id, run);

    // The connection and its two secrets arrive in the message of the lake, and nothing of SQL.
    let resolved = worker
        .client
        .resolve_database(worker.req(Worker::resolve_secrets_message(&lease)))
        .await
        .expect("the credentials of a lake step")
        .into_inner();
    assert_eq!(resolved.connection_name, "warehouse");
    assert_eq!(resolved.connection_revision, 1);
    assert_eq!(resolved.namespace_id, ns);
    assert!(resolved.postgres.is_none() && resolved.credential.is_none());
    assert!(resolved.statement.is_empty() && resolved.parameters.is_empty());
    let iceberg = resolved.iceberg.as_ref().expect("the Iceberg connection");
    let secrets: Vec<_> = {
        let mut names: Vec<_> = iceberg.credentials.keys().cloned().collect();
        names.sort();
        names
    };
    assert_eq!(secrets, ["catalog", "storage"]);
    assert!(
        String::from_utf8_lossy(&iceberg.credentials["catalog"]).contains("WIRE_CATALOG_CANARY")
    );
    assert!(
        String::from_utf8_lossy(&iceberg.credentials["storage"]).contains("WIRE_STORAGE_CANARY")
    );
    assert!(
        !format!("{resolved:?}").contains("CANARY"),
        "the message prints no secret"
    );

    // A receipt that is not the step's is refused on the wire and leaves the step as it was.
    let receipt = |effect_key: &str| {
        json!({
            "kind": "iceberg", "outcome": "COMMITTED", "effectKey": effect_key, "foundByLookup": false,
            "mode": "append", "ref": "main", "snapshotId": "8744736658442914487", "sequenceNumber": "12",
            "schemaId": 3, "formatVersion": 2, "rowLevelMode": "NONE", "rowsAdded": "1", "dataFilesAdded": "1",
            "bytesWritten": "512", "ignoredFields": "0", "droppedValues": "0"
        })
    };
    let complete = |output: &Value, key: &str| pb::CompleteStepRequest {
        instance: lease.instance.clone(),
        task_id: lease.task_id.clone(),
        run_id: lease.run_id.clone(),
        step_id: lease.step_id.clone(),
        attempt: lease.attempt,
        generation: lease.generation,
        lease_token: lease.lease_token.clone(),
        idempotency_key: key.to_owned(),
        output_json: serde_json::to_string(output).unwrap(),
        output_blob_ref: String::new(),
        output_sha256: sagat_domain::workflow::Digest::new(
            Sha256::digest(serde_json::to_vec(output).unwrap()).into(),
        )
        .to_hex(),
        traceparent: String::new(),
    };
    let refused = worker
        .client
        .complete_step(worker.req(complete(&receipt("another-run:save"), "lake-wire-refused")))
        .await
        .expect_err("a receipt of another effect");
    assert_eq!(refused.code(), tonic::Code::InvalidArgument, "{refused:?}");
    assert_eq!(p.status(&run).await, "RUNNING");
    let committed = || async {
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT resource_id, reason FROM audit_records WHERE tenant_id = $1 AND action = 'iceberg.write_committed' AND resource_id = $2",
        )
        .bind(p.tenant.as_uuid())
        .bind(&run)
        .fetch_all(&p.api.db.pool)
        .await
        .expect("the audit records")
    };
    assert!(
        committed().await.is_empty(),
        "a receipt that was refused is not a committed write"
    );

    // The receipt of the step completes it, through the real server.
    let good = receipt(&lease.idempotency_key);
    worker
        .client
        .complete_step(worker.req(complete(&good, "lake-wire-complete")))
        .await
        .expect("the receipt of the step completes it");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    // The committed write is audited once, for the run: where it was written and which snapshot it made, and nothing it held.
    let written = committed().await;
    assert_eq!(written.len(), 1, "{written:?}");
    let reason = written[0].1.as_deref().expect("a reason");
    for (what, text) in [
        ("the step", "node:save"),
        ("the connection", "connection:warehouse"),
        ("the table", "table:sales.orders"),
        ("the snapshot", "snapshot:8744736658442914487"),
        ("the rows", "rows:1"),
    ] {
        assert!(reason.contains(text), "the record names {what}: {reason}");
    }
    for forbidden in ["r-1", "CANARY", "s3://", &lease.idempotency_key] {
        assert!(
            !reason.contains(forbidden),
            "no value, credential or location, nor the effect key, in the record: {reason}"
        );
    }
    // a replay of the same report is the same completion, and the same write: one record
    let replay = worker
        .client
        .complete_step(worker.req(complete(&good, "lake-wire-complete")))
        .await
        .expect("a replay")
        .into_inner();
    assert!(replay.already_applied);
    assert_eq!(committed().await.len(), 1, "a replay is not a second write");
}

//! Canaries across the server's half of a lake write (feature 035, T035-061, NFR-035-SEC): a secret, a value, a file location and the
//! text of a host error are planted where a hostile or merely careless runner can put them, and then every place the server keeps or
//! shows anything is searched for them: every table of the database, the REST views of the run, the answers to the refused calls and
//! the log at its most verbose.
//!
//! What a runner can put in a call: the fields of a receipt (a field the receipt does not have, or text where a closed word or a number
//! belongs), the code of a failure (which is stored and shown to everyone who reads the run) and its message (which the transport drops).
//! The test never prints a canary: a failed assertion names the surface and the class (the harness of feature 016 does the same), so a
//! detected leak is not copied into the log of the CI that detected it.
//!
//! It runs inside the lifecycle scenario of `main.rs`, with the OpenBao of that scenario.
#![allow(clippy::expect_used, clippy::panic)]
use super::lake::{append, rows_schema};
use crate::support::{
    api::Options,
    logs,
    worker::{Platform, Worker, platform_with},
};
use reqwest::Method;
use sagat_api_grpc::pb;
use sagat_application::secret_provider::SecretProvider;
use sagat_domain::{database::ICEBERG_WRITE_ACTIVITY, identity::Role};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

/// A planted value, by the class of leak it stands for.
struct Canary {
    class: &'static str,
    value: String,
}

fn canaries() -> Vec<Canary> {
    let suffix = Uuid::now_v7().simple().to_string();
    let make = |class: &'static str, stem: &str| Canary {
        class,
        value: format!("{stem}-{suffix}"),
    };
    vec![
        make("the secret of the catalog", "CANARY-CATALOG-SECRET"),
        make("the secret of the storage", "CANARY-STORAGE-SECRET"),
        make("a value of the records", "CANARY-ROW-VALUE"),
        make("the location of a file", "CANARY-FILE-LOCATION"),
        make("the text of a host error", "CANARY-HOST-ERROR"),
    ]
}

fn value_of<'a>(all: &'a [Canary], class: &str) -> &'a str {
    &all.iter()
        .find(|canary| canary.class == class)
        .expect("the class")
        .value
}

/// Every base table of the schema: the places the server keeps anything. A table added by a later feature is searched without anyone
/// remembering to add it here.
async fn tables(p: &Platform) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY 1",
    )
    .fetch_all(&p.api.db.pool)
    .await
    .expect("the tables")
}

/// The tables that hold `needle`, as text or as the hex of bytes (how a row prints a `bytea`). The names come from the catalog of the
/// database and are quoted; the needle is a bind.
async fn holders(p: &Platform, tables: &[String], needle: &str) -> Vec<String> {
    let hex = needle
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut found = Vec::new();
    for table in tables {
        let statement = format!(
            "SELECT count(*) FROM \"{table}\" AS x WHERE x::text LIKE $1 OR x::text LIKE $2"
        );
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(statement))
            .bind(format!("%{needle}%"))
            .bind(format!("%{hex}%"))
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the search of a table");
        if count > 0 {
            found.push(table.clone());
        }
    }
    found
}

/// What the run shows to whoever reads it over REST.
async fn views(p: &Platform, run: &str) -> Vec<(String, String)> {
    let mut seen = Vec::new();
    for path in [
        format!("/runs/{run}"),
        format!("/runs/{run}/events"),
        format!("/runs/{run}/attempts"),
        format!("/runs/{run}/graph"),
        format!("/runs/{run}/telemetry"),
    ] {
        let (status, body) = p.rest(Method::GET, &path, None).await;
        assert_eq!(status, 200, "{path}: {body}");
        seen.push((path, body.to_string()));
    }
    seen
}

/// The body of a typed receipt, as the runner's host would report it.
fn receipt(effect_key: &str) -> Value {
    json!({
        "kind": "iceberg", "outcome": "COMMITTED", "effectKey": effect_key, "foundByLookup": false,
        "mode": "append", "ref": "main", "snapshotId": "8744736658442914487", "sequenceNumber": "12",
        "schemaId": 3, "formatVersion": 2, "rowLevelMode": "NONE", "rowsAdded": "1", "dataFilesAdded": "1",
        "bytesWritten": "512", "ignoredFields": "0", "droppedValues": "0"
    })
}

fn completion(lease: &pb::TaskLease, output: &Value, key: &str) -> pb::CompleteStepRequest {
    let text = serde_json::to_string(output).expect("the output");
    pb::CompleteStepRequest {
        idempotency_key: key.to_owned(),
        output_sha256: sagat_domain::workflow::Digest::new(Sha256::digest(text.as_bytes()).into())
            .to_hex(),
        ..Worker::complete_message(lease, &text)
    }
}

/// What a refused call says back: the message and every piece of metadata, as one text.
fn said(status: &tonic::Status) -> String {
    format!("{:?} {:?}", status.message(), status.metadata())
}

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let all = canaries();
    let (catalog, storage) = (
        value_of(&all, "the secret of the catalog"),
        value_of(&all, "the secret of the storage"),
    );
    let (row, location, host_error) = (
        value_of(&all, "a value of the records"),
        value_of(&all, "the location of a file"),
        value_of(&all, "the text of a host error"),
    );
    let p = platform_with(Options {
        secret_provider: Some(provider),
        lake: true,
        ..Default::default()
    })
    .await;
    let ns = p.namespace_id().await;
    for (name, value) in [
        (
            "canary-catalog",
            json!({"clientId": "canary-client", "clientSecret": catalog}).to_string(),
        ),
        (
            "canary-storage",
            json!({"accessKeyId": "canary-key", "secretAccessKey": storage}).to_string(),
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
        assert_eq!(status, 200, "the secret {name} is stored: {body}");
    }
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/canary-warehouse"),
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
                    "catalogSecret": "canary-catalog",
                    "storageSecret": "canary-storage",
                    "limits": {}
                }
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let workflow = p
        .publish(
            "lake-canaries",
            &append(
                "lake-canaries",
                "canary-warehouse",
                &["/id", "/total"],
                Some(rows_schema()),
            ),
        )
        .await;
    p.grant(p.tenant, "lake-canaries", Role::DatabaseRunner)
        .await;
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/worker-bindings/lake-canaries"),
            Some(&json!({"activities": [ICEBERG_WRITE_ACTIVITY], "profile": "lake"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let mut worker = p.connect("lake-canaries", "acme").await;
    worker
        .register_version(&[ICEBERG_WRITE_ACTIVITY], "1.6")
        .await;

    // Everything from here on is logged at its most verbose, and kept: the refusals and the failure below are where text would leak.
    let (_log_guard, log) = logs::capture_at(tracing::Level::TRACE);

    // --- A run whose runner is hostile: every call it can make with text in it.
    let hostile = p
        .start(&workflow, &json!({"rows": [{"id": row, "total": 3}]}))
        .await;
    let lease = worker.one_task().await;
    assert_eq!(lease.run_id, hostile);
    worker
        .client
        .resolve_database(worker.req(Worker::resolve_secrets_message(&lease)))
        .await
        .expect("the credentials of the step");
    let mut refusals: Vec<(String, String)> = Vec::new();

    // A receipt that has a field the receipt does not have, or text where a closed word or a number belongs, is refused whole.
    let with = |name: &str, edit: &dyn Fn(&mut Value)| {
        let mut body = receipt(&lease.idempotency_key);
        edit(&mut body);
        (name.to_owned(), body)
    };
    let hostile_receipts = vec![
        with("a field the receipt does not have (a location)", &|body| {
            body["location"] = json!(format!("s3://lake/{location}/data/00000.parquet"));
        }),
        with("a field the receipt does not have (an error)", &|body| {
            body["error"] = json!(format!("Traceback: {host_error}"));
        }),
        with("a field the receipt does not have (a value)", &|body| {
            body["partition"] = json!({"id": row});
        }),
        with("the effect key", &|body| {
            body["effectKey"] = json!(format!("{host_error}:save"));
        }),
        with("the branch", &|body| {
            body["ref"] = json!(host_error);
        }),
        with("the mode", &|body| {
            body["mode"] = json!(host_error);
        }),
        with("the outcome", &|body| {
            body["outcome"] = json!(host_error);
        }),
        with("the kind", &|body| {
            body["kind"] = json!(host_error);
        }),
        with("the row-level mode", &|body| {
            body["rowLevelMode"] = json!(location);
        }),
        with("a count", &|body| {
            body["rowsAdded"] = json!(row);
        }),
        with("the snapshot", &|body| {
            body["snapshotId"] = json!(row);
        }),
    ];
    for (index, (name, body)) in hostile_receipts.iter().enumerate() {
        let refused = worker
            .client
            .complete_step(worker.req(completion(&lease, body, &format!("canary-{index}"))))
            .await
            .expect_err(name);
        refusals.push((format!("the refusal of {name}"), said(&refused)));
        assert_eq!(
            p.status(&hostile).await,
            "RUNNING",
            "a receipt with {name} leaves the step as it was"
        );
    }

    // A failure whose code is the text of a secret is refused: only the closed codes of the managed failures enter the journal.
    let mut code = Worker::fail_message(&lease, catalog, false);
    code.safe_message = format!("{host_error} at {location}");
    let refused = worker
        .client
        .fail_step(worker.req(code))
        .await
        .expect_err("a failure with a code that is not one of the closed ones");
    refusals.push(("the refusal of a failure code".to_owned(), said(&refused)));
    assert_eq!(p.status(&hostile).await, "RUNNING");

    // A failure with a closed code and a message made of the host's text fails the step for good; the message is dropped.
    let mut message = Worker::fail_message(&lease, "ICEBERG_LIMIT_EXCEEDED", false);
    message.safe_message =
        format!("{host_error} at s3://lake/{location}/data: {storage} {catalog} {row}");
    worker
        .client
        .fail_step(worker.req(message))
        .await
        .expect("a closed code is a failure, whatever the message says");
    assert_eq!(p.status(&hostile).await, "FAILED");

    // --- A run whose write commits: the receipt is typed, and the output the server keeps is what it rebuilt from it.
    let committed = p
        .start(&workflow, &json!({"rows": [{"id": row, "total": 4}]}))
        .await;
    let lease = worker.one_task().await;
    assert_eq!(lease.run_id, committed);
    worker
        .client
        .resolve_database(worker.req(Worker::resolve_secrets_message(&lease)))
        .await
        .expect("the credentials of the step");
    worker
        .client
        .complete_step(worker.req(completion(
            &lease,
            &receipt(&lease.idempotency_key),
            "canary-complete",
        )))
        .await
        .expect("the receipt of the step completes it");
    assert_eq!(p.status(&committed).await, "SUCCEEDED");

    // --- The search.
    let tables = tables(&p).await;
    assert!(
        tables.iter().any(|table| table == "audit_records")
            && tables.iter().any(|table| table == "run_events"),
        "the catalog of the database lists the tables the server keeps its records in"
    );
    // The control: the value of the records is the run's own input, which the server keeps for the run; a search that found it nowhere
    // would be a search that cannot see.
    let keeps_the_input = holders(&p, &tables, row).await;
    assert!(
        !keeps_the_input.is_empty(),
        "the search finds the input of a run where it is kept"
    );
    for table in &keeps_the_input {
        assert!(
            !["audit_records", "run_events", "event_outbox"].contains(&table.as_str()),
            "a value of the records is in `{table}`, which holds no payload"
        );
    }
    for canary in all
        .iter()
        .filter(|canary| canary.class != "a value of the records")
    {
        let found = holders(&p, &tables, &canary.value).await;
        assert!(
            found.is_empty(),
            "{} is in the tables {found:?}",
            canary.class
        );
    }

    let mut surfaces: Vec<(String, String)> = refusals;
    for run in [&hostile, &committed] {
        surfaces.extend(views(&p, run).await);
    }
    surfaces.push(("the log".to_owned(), log.text()));
    assert!(
        surfaces
            .iter()
            .any(|(name, text)| name == "the log" && !text.is_empty()),
        "the log was captured"
    );
    for (surface, text) in &surfaces {
        for canary in &all {
            // The records are the run's own input, which a reader of the run sees; they are not in a refusal, a failure or the log.
            if canary.class == "a value of the records" && surface.starts_with("/runs/") {
                continue;
            }
            assert!(
                !text.contains(&canary.value),
                "{} is in {surface}",
                canary.class
            );
        }
    }
}

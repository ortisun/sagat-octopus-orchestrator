//! A lake runner is handed the credentials of its step (feature 035, T035-044, D035-141): the catalog's and the storage's, each
//! pinned under its role, read from the provider one after the other and judged again when both have been read.
//!
//! Through the real store, the real worker service, a real OpenBao and the runner's own reader of the lease. It runs inside the
//! lifecycle scenario of `main.rs`, before the lease scenario, whose budget check counts the attempts that are in flight: every
//! step leased here is failed for good before the scenario ends.
#![allow(clippy::expect_used, clippy::panic)]
use super::lake::{append, iceberg_in, iceberg_secrets, publish_on_lake, rows_schema, write};
use crate::{
    ReadBarrier,
    support::engine::{World, ctx, task_ref},
};
use sagat_application::{
    clock::SystemClock,
    database::{ConnectionService, ResolvedGrant},
    database::{ExpectedReceipt, LakeReceiptCodec},
    entropy::SystemEntropy,
    execution::Engine,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    runs::{RunService, StartRun},
    worker::{Capabilities, ClaimedTask, WorkerService, WorkerSettings},
};
use sagat_domain::{
    audit::AuditAction,
    binding::BindingProfile,
    classification::EgressCeiling,
    database::{ConnectionStatus, WriteMode},
    definition::WorkflowId,
    execution::RunId,
    identity::Role,
    secrets::SecretValue,
};
use sagat_infra_ir::{JsonEvaluator, JsonLakeReceiptCodec};
use sagat_infra_postgres::PgRunStore;
use sagat_infra_secrets::openbao::OpenBao;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

const CATALOG: &str = "lake-catalog-credentials";
const STORAGE: &str = "lake-storage-credentials";
const ACTIVITY: &str = "sagat.database.iceberg.write";

async fn put(
    w: &World,
    secrets: &ManagedSecretService<PgRunStore, SystemClock>,
    name: &str,
    expected_version: u32,
    value: &str,
) -> u32 {
    secrets
        .put(
            &w.admin,
            w.namespace.id(),
            ManagedSecretWrite {
                name,
                value: SecretValue::new(value.as_bytes().to_vec()).unwrap(),
                expected_version,
                request_id: Uuid::new_v4(),
            },
            &ctx(),
        )
        .await
        .expect("write the secret")
        .version
}

/// What is pinned for a run: the attempt, the role, the name and the version, in that order.
async fn pins(w: &World, run: RunId) -> Vec<(i64, String, String, i64)> {
    sqlx::query_as(
        "SELECT attempt::bigint, secret_role, secret_name, secret_version::bigint FROM step_database_credentials \
         WHERE tenant_id = $1 AND run_id = $2 ORDER BY attempt, secret_role",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .unwrap()
}

/// The reasons of the audit records that say the credentials of a run were handed over.
async fn handed(w: &World, run: RunId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE tenant_id = $1 AND action = $2 AND resource_id = $3 ORDER BY created_at, id",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::SecretResolved.as_str())
    .bind(format!("{}/run {run}", w.namespace.id()))
    .fetch_all(&w.db.pool)
    .await
    .unwrap()
}

async fn denials(w: &World, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = $2 AND resource_id = $3",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::DatabaseDenied.as_str())
    .bind(run.to_string())
    .fetch_one(&w.db.pool)
    .await
    .unwrap()
}

pub async fn run(
    w: &World,
    secrets: &ManagedSecretService<PgRunStore, SystemClock>,
    provider: Arc<OpenBao>,
) {
    let ns = w.namespace.id();
    assert_eq!(put(w, secrets, CATALOG, 0, "catalog-canary-0350").await, 1);
    assert_eq!(put(w, secrets, STORAGE, 0, "storage-canary-0350").await, 1);

    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    let connect = |name: &str, catalog_secret: &str, storage_secret: &str| {
        write(
            name,
            0,
            ConnectionStatus::Active,
            iceberg_secrets(
                "https://catalog.example/api",
                catalog_secret,
                storage_secret,
                EgressCeiling::Internal,
                vec![WriteMode::Append],
            ),
        )
    };
    catalog
        .put(&w.admin, ns, connect("lake-two", CATALOG, STORAGE), &ctx())
        .await
        .expect("a connection with a secret for the catalog and another for the storage");
    catalog
        .put(
            &w.admin,
            ns,
            write(
                "lake-one",
                0,
                ConnectionStatus::Active,
                iceberg_in(
                    "https://catalog.example/api",
                    "db-credentials",
                    EgressCeiling::Internal,
                    vec![WriteMode::Append],
                ),
            ),
            &ctx(),
        )
        .await
        .expect("a connection whose two secrets are one");
    let publish = |name: &'static str, connection: &'static str| async move {
        publish_on_lake(
            w,
            &w.admin,
            append(name, connection, &["/id", "/total"], Some(rows_schema())),
        )
        .await
        .expect("a workflow on the connection")
    };
    let two: WorkflowId = publish("lake-resolve-two", "lake-two").await;
    let one: WorkflowId = publish("lake-resolve-one", "lake-one").await;

    // The process that admits runs and the ones that lease and resolve have the lake on; one of them reads the provider through a
    // barrier, to hold a read while the secrets change under it.
    let lake_store = w.store.clone().with_lake_enabled(true);
    let settings = || WorkerSettings {
        lease_ttl: Duration::from_secs(3600),
        ..WorkerSettings::default()
    };
    let runs = RunService::new(
        lake_store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        w.cache.clone(),
    );
    let workers = WorkerService::new(
        lake_store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        settings(),
    )
    .with_secret_provider(provider.clone())
    .with_lake_receipts(Arc::new(JsonLakeReceiptCodec));
    let uncoded = WorkerService::new(
        lake_store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        settings(),
    )
    .with_secret_provider(provider.clone());
    let gate = Arc::new(ReadBarrier::new(provider.clone()));
    let gated = WorkerService::new(
        lake_store,
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        settings(),
    )
    .with_secret_provider(gate.clone());

    let runner = w.member("lake-resolve-runner", Role::DatabaseRunner).await;
    w.bindings
        .bind_with_profile(
            &w.admin,
            ns,
            runner.subject().as_str(),
            &[ACTIVITY.to_owned()],
            None,
            Some(BindingProfile::Lake),
            &ctx(),
        )
        .await
        .expect("bind the lake runner");
    let registered = workers
        .register(
            &runner,
            "payments",
            "lake-resolve-machine",
            &Capabilities {
                activities: vec![ACTIVITY.to_owned()],
                languages: vec![],
                pools: vec![],
                max_inflight: 32,
                protocol_version: "1.6".into(),
            },
        )
        .await
        .expect("register the lake runner");
    let token = registered.session_token.expose().to_owned();
    let start = |workflow: WorkflowId, key: &'static str| {
        let runs = &runs;
        async move {
            let input = serde_json::to_vec(&json!({"rows": [{"id": "r-1", "total": 3}]})).unwrap();
            runs.start(
                &w.admin,
                &StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: &input,
                    correlation_id: None,
                    deadline: None,
                    key,
                },
                &ctx(),
            )
            .await
            .expect("a run of an Iceberg write with the lake on")
            .info
            .run_id
        }
    };
    // Runs of Iceberg writes made earlier in the scenario are waiting too: poll until the one wanted is handed over.
    let lease = |run: RunId| {
        let (workers, runner, token) = (&workers, &runner, &token);
        async move {
            for _ in 0..20 {
                let tasks = workers
                    .poll(
                        runner,
                        registered.worker_id,
                        token,
                        8,
                        Some("0af7651916cd43dd8448eb211c80319c"),
                    )
                    .await
                    .expect("poll");
                if let Some(task) = tasks.into_iter().find(|task| task.run_id == run) {
                    return task;
                }
            }
            panic!("the lake runner is never handed the step of the run {run}");
        }
    };
    // Frees the budget of the namespace: the lease scenario that follows counts what is in flight.
    let release = |task: ClaimedTask| {
        let (workers, runner) = (&workers, &runner);
        async move {
            let key = format!("lake-resolve-release-{}", task.run_id);
            workers
                .fail(
                    runner,
                    &task_ref(&task),
                    "ICEBERG_LIMIT_EXCEEDED",
                    false,
                    &key,
                )
                .await
                .expect("fail the step for good");
        }
    };

    // The two secrets of a connection are handed over, each under its role, read and pinned.
    let first = start(two, "lake-resolve-start-00001").await;
    let task = lease(first).await;
    let resolved = workers
        .resolve_database(&runner, &task_ref(&task), None)
        .await
        .expect("the credentials of a lake step");
    assert_eq!(resolved.connection.revision, 1);
    assert!(resolved.connection.config.as_iceberg().is_some());
    let ResolvedGrant::Lake(grant) = &resolved.grant else {
        panic!("a lake step is handed a lake grant: {resolved:?}");
    };
    let catalog_secret = grant.catalog.as_ref().expect("the catalog's secret");
    let storage_secret = grant.storage.as_ref().expect("the storage's secret");
    assert_eq!(
        (catalog_secret.name.as_str(), catalog_secret.version),
        (CATALOG, 1)
    );
    assert_eq!(catalog_secret.value.expose(), b"catalog-canary-0350");
    assert_eq!(
        (storage_secret.name.as_str(), storage_secret.version),
        (STORAGE, 1)
    );
    assert_eq!(storage_secret.value.expose(), b"storage-canary-0350");
    assert!(
        !format!("{resolved:?}").contains("canary"),
        "no secret byte is printed"
    );
    assert_eq!(
        pins(w, first).await,
        vec![
            (1, "catalog".into(), CATALOG.into(), 1),
            (1, "storage".into(), STORAGE.into(), 1),
        ],
        "one pin per role, in the transaction that hands the secrets over"
    );
    // Each secret is audited on its own (contracts/worker-protocol.md): which role, which secret, which version, never what it holds.
    let mut audited = handed(w, first).await;
    audited.sort();
    assert_eq!(audited.len(), 2, "one audit record per secret: {audited:?}");
    assert!(
        audited[0].ends_with(&format!(": catalog {CATALOG} v1")),
        "{}",
        audited[0]
    );
    assert!(
        audited[1].ends_with(&format!(": storage {STORAGE} v1")),
        "{}",
        audited[1]
    );
    assert!(
        audited.iter().all(|reason| !reason.contains("canary")),
        "and never what it holds: {audited:?}"
    );
    assert_eq!(denials(w, first).await, 0);

    // The pin wins over a later rotation, for the attempt that holds it; a new attempt reads what is in force.
    assert_eq!(put(w, secrets, CATALOG, 1, "catalog-canary-0351").await, 2);
    let again = workers
        .resolve_database(&runner, &task_ref(&task), None)
        .await
        .expect("the same attempt resolves again");
    let ResolvedGrant::Lake(again) = &again.grant else {
        panic!("a lake grant");
    };
    let catalog_again = again.catalog.as_ref().expect("the catalog's secret");
    assert_eq!(
        catalog_again.version, 1,
        "the pinned version, not the new one"
    );
    assert_eq!(catalog_again.value.expose(), b"catalog-canary-0350");
    assert_eq!(pins(w, first).await.len(), 2, "nothing more is pinned");
    assert_eq!(
        handed(w, first).await.len(),
        2,
        "an identical resolve is the same two events, recorded once"
    );
    workers
        .fail(
            &runner,
            &task_ref(&task),
            "CATALOG_UNAVAILABLE",
            true,
            "lake-resolve-retry-1",
        )
        .await
        .expect("a retryable failure");
    w.age_retry(first, "save").await;
    w.scheduler.tick().await;
    let second = lease(first).await;
    assert_eq!(second.fence.attempt, 2);
    let next = workers
        .resolve_database(&runner, &task_ref(&second), None)
        .await
        .expect("the credentials of the second attempt");
    let ResolvedGrant::Lake(next) = &next.grant else {
        panic!("a lake grant");
    };
    let (catalog_next, storage_next) = (
        next.catalog.as_ref().expect("catalog"),
        next.storage.as_ref().expect("storage"),
    );
    assert_eq!(
        handed(w, first).await.len(),
        4,
        "the second attempt is audited on its own, secret by secret"
    );
    assert_eq!(catalog_next.version, 2);
    assert_eq!(catalog_next.value.expose(), b"catalog-canary-0351");
    assert_eq!(storage_next.version, 1);
    assert_eq!(
        pins(w, first).await,
        vec![
            (1, "catalog".into(), CATALOG.into(), 1),
            (1, "storage".into(), STORAGE.into(), 1),
            (2, "catalog".into(), CATALOG.into(), 2),
            (2, "storage".into(), STORAGE.into(), 1),
        ]
    );
    release(second).await;

    // The pins of an attempt are released with the retention of the terminal run, every role of every attempt together.
    assert_eq!(pins(w, first).await.len(), 4);
    sqlx::query(
        "UPDATE runs SET finished_at = clock_timestamp() - interval '25 hours' WHERE tenant_id = $1 AND id = $2 AND finished_at IS NOT NULL",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(first.as_uuid())
    .execute(&w.db.pool)
    .await
    .unwrap();
    assert!(
        sagat_application::secret_cleanup::SecretCleanupStore::release_terminal_credential_pins(
            &w.store, 128
        )
        .await
        .unwrap()
            >= 4
    );
    assert!(
        pins(w, first).await.is_empty(),
        "the catalog's and the storage's pins go together"
    );
    let released: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = $2 AND resource_id = $3",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::SecretAttemptPinsReleased.as_str())
    .bind(first.to_string())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(released, 1, "and the release is audited once for the run");

    // A connection whose catalog and storage name one secret: two roles, one secret, two pins.
    let shared = start(one, "lake-resolve-start-00002").await;
    let task = lease(shared).await;
    // The process that serves the worker gRPC needs the lake on too: with it off the step is not found, and nothing is pinned.
    assert!(
        matches!(
            w.workers
                .resolve_database(&runner, &task_ref(&task), None)
                .await,
            Err(sagat_application::error::AppError::NotFound)
        ),
        "a process that has the lake off hands nothing over"
    );
    assert!(pins(w, shared).await.is_empty());
    let resolved = workers
        .resolve_database(&runner, &task_ref(&task), None)
        .await
        .expect("a connection whose two secrets are one");
    let ResolvedGrant::Lake(grant) = &resolved.grant else {
        panic!("a lake grant");
    };
    let (catalog_secret, storage_secret) = (
        grant.catalog.as_ref().expect("catalog"),
        grant.storage.as_ref().expect("storage"),
    );
    assert_eq!(catalog_secret.name.as_str(), "db-credentials");
    assert_eq!(storage_secret.name.as_str(), "db-credentials");
    assert_eq!(catalog_secret.version, storage_secret.version);
    assert_eq!(catalog_secret.value.expose(), storage_secret.value.expose());
    let roles: Vec<String> = pins(w, shared)
        .await
        .into_iter()
        .map(|(_, role, ..)| role)
        .collect();
    assert_eq!(roles, vec!["catalog".to_owned(), "storage".to_owned()]);
    assert_eq!(
        handed(w, shared).await.len(),
        2,
        "one secret under two roles is two events, not one that repeats"
    );
    release(task).await;

    // Either secret changing between the two reads fails the resolve and pins nothing: the first read is held while the storage
    // secret is rotated, and the second while the catalog's is.
    for (held, rotated, key) in [
        (0, STORAGE, "lake-resolve-start-00003"),
        (1, CATALOG, "lake-resolve-start-00004"),
    ] {
        let run = start(two, key).await;
        let task = lease(run).await;
        let reference = task_ref(&task);
        let resolving = gated.resolve_database(&runner, &reference, None);
        // The driver holds each read of the provider, changes the secret while the chosen one is held, and lets it go. A resolve
        // that returns before it has read both leaves the driver waiting: the limit names that, instead of a hung suite.
        let driver = async {
            for read in 0..2 {
                gate.entered.wait().await;
                if read == held {
                    let version = if rotated == CATALOG { 2 } else { 1 };
                    put(w, secrets, rotated, version, "rotated-canary-0352").await;
                }
                gate.resume.wait().await;
            }
        };
        let (outcome, driven) = tokio::join!(
            resolving,
            tokio::time::timeout(Duration::from_secs(120), driver)
        );
        driven.expect("the resolve read the provider twice");
        assert!(
            outcome.is_err(),
            "a secret that changed between the reads is not handed over"
        );
        assert!(
            pins(w, run).await.is_empty(),
            "and nothing is pinned for it"
        );
        // The audit of a denial is written off the request path, so it is waited for, boundedly.
        crate::support::eventually("the audit of the refused resolve", || async {
            (denials(w, run).await == 1).then_some(())
        })
        .await;
        release(task).await;
    }

    // A secret limited to worker pools is not handed to a runner outside them, whichever role it plays: the storage's first, then
    // the catalog's. The refusal pins nothing, and the grant of the pool is all the runner needs.
    for (restricted, pool, granted, key) in [
        (
            STORAGE,
            "lake-pool-a",
            vec!["lake-pool-a"],
            "lake-resolve-start-00005",
        ),
        (
            CATALOG,
            "lake-pool-b",
            vec!["lake-pool-a", "lake-pool-b"],
            "lake-resolve-start-00006",
        ),
    ] {
        let run = start(two, key).await;
        let task = lease(run).await;
        w.secrets
            .set_pools(&w.admin, ns, restricted, &[pool.to_owned()], &ctx())
            .await
            .expect("limit the secret to a pool");
        assert!(
            workers
                .resolve_database(&runner, &task_ref(&task), None)
                .await
                .is_err(),
            "the {restricted} secret is limited to {pool} and the runner is not in it"
        );
        assert!(
            pins(w, run).await.is_empty(),
            "and nothing is pinned for it"
        );
        let granted: Vec<String> = granted.into_iter().map(str::to_owned).collect();
        w.bindings
            .bind_with_pools(
                &w.admin,
                ns,
                runner.subject().as_str(),
                &[ACTIVITY.to_owned()],
                Some(&granted),
                &ctx(),
            )
            .await
            .expect("grant the pool to the runner");
        workers
            .resolve_database(&runner, &task_ref(&task), None)
            .await
            .expect("with the pool granted the credentials are handed over");
        assert_eq!(pins(w, run).await.len(), 2);
        release(task).await;
    }

    // The step is completed with a receipt the server builds again, and with nothing else (D035-142).
    let run = start(two, "lake-resolve-start-00008").await;
    let task = lease(run).await;
    let reference = task_ref(&task);
    let receipt = json!({
        "kind": "iceberg", "outcome": "COMMITTED", "effectKey": task.idempotency_key, "foundByLookup": false,
        "mode": "append", "ref": "main", "snapshotId": "8744736658442914487", "sequenceNumber": "12", "schemaId": 3,
        "formatVersion": 2, "rowLevelMode": "NONE", "rowsAdded": "1", "dataFilesAdded": "1", "bytesWritten": "512",
        "ignoredFields": "0", "droppedValues": "0"
    });
    let compact = serde_json::to_vec(&receipt).unwrap();
    let state = || async {
        sqlx::query_as::<_, (String, Option<Vec<u8>>)>(
            "SELECT status, output_digest FROM steps WHERE run_id = $1 AND node = 'save'",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .unwrap()
    };
    // a completion that no resolve preceded is not one: the attempt never held the credentials of the write
    assert!(
        workers
            .complete(&runner, &reference, &compact, "lake-complete-0")
            .await
            .is_err(),
        "a receipt for a write whose credentials were never handed over"
    );
    assert_eq!(state().await.1, None);
    workers
        .resolve_database(&runner, &reference, None)
        .await
        .expect("the credentials");
    // a service with no codec cannot complete a lake step, and a receipt that is not the step's is not stored
    assert!(
        uncoded
            .complete(&runner, &reference, &compact, "lake-complete-1")
            .await
            .is_err()
    );
    let with = |member: &str, value: Value| {
        let mut other = receipt.clone();
        other[member] = value;
        serde_json::to_vec(&other).unwrap()
    };
    let mut extra = receipt.clone();
    extra["firstRow"] = json!({"email": "someone@example.com"});
    let refused = [
        with("effectKey", json!("another-run:save")),
        with("ref", json!("audit")),
        with("mode", json!("overwrite")),
        with("outcome", json!("REVERTED")),
        with("kind", json!("object_storage")),
        serde_json::to_vec(&extra).unwrap(),
        br#"{"not":"a receipt"}"#.to_vec(),
    ];
    for (n, bytes) in refused.iter().enumerate() {
        let key = format!("lake-complete-refused-{n}");
        assert!(
            matches!(
                workers.complete(&runner, &reference, bytes, &key).await,
                Err(sagat_application::error::AppError::NotAllowed(_))
            ),
            "refusal {n}: {}",
            String::from_utf8_lossy(bytes)
        );
    }
    let (status, digest) = state().await;
    assert_eq!(
        (status.as_str(), digest),
        ("LEASED", None),
        "nothing was stored and the step is still the attempt's"
    );
    // the receipt of the step, sent with other whitespace and the members the other way round, is stored as the platform writes it
    let mut sent = String::from("{ ");
    for (n, (name, value)) in receipt.as_object().unwrap().iter().rev().enumerate() {
        if n > 0 {
            sent.push_str(" ,\n ");
        }
        sent.push_str(&format!("{}: {value}", json!(name)));
    }
    sent.push_str(" }");
    let canonical = JsonLakeReceiptCodec
        .rebuild(
            sent.as_bytes(),
            &ExpectedReceipt {
                effect_key: &task.idempotency_key,
                reference: "main",
                mode: sagat_domain::database::WriteMode::Append,
            },
        )
        .expect("the receipt of the step");
    assert_ne!(
        canonical,
        sent.as_bytes(),
        "what is stored is not what was sent"
    );
    let acked = workers
        .complete(&runner, &reference, sent.as_bytes(), "lake-complete-2")
        .await
        .expect("the receipt of the step completes it");
    assert!(!acked.already_applied);
    assert_eq!(
        w.run_status(run).await,
        sagat_domain::execution::RunStatus::Succeeded
    );
    let (status, digest) = state().await;
    assert_eq!(status, "SUCCEEDED");
    assert_eq!(
        digest.as_deref(),
        Some(Sha256::digest(&canonical).as_slice()),
        "the output of the step is the receipt the server wrote"
    );
    assert_ne!(
        digest.as_deref(),
        Some(Sha256::digest(sent.as_bytes()).as_slice())
    );
    // the same report again is the same completion; the same key for other bytes is not, and a new report finds the step done
    assert!(
        workers
            .complete(&runner, &reference, sent.as_bytes(), "lake-complete-2")
            .await
            .expect("a replay")
            .already_applied
    );
    assert!(matches!(
        workers
            .complete(&runner, &reference, &compact, "lake-complete-2")
            .await,
        Err(sagat_application::error::AppError::IdempotencyKeyReused)
    ));
    assert!(matches!(
        workers
            .complete(&runner, &reference, &compact, "lake-complete-3")
            .await,
        Err(sagat_application::error::AppError::StaleAttempt)
    ));

    // A write whose outcome the runner cannot tell waits for an operator, with the code and nothing else; it is not retried or failed.
    let unknown = start(two, "lake-resolve-start-00009").await;
    let task = lease(unknown).await;
    workers
        .resolve_database(&runner, &task_ref(&task), None)
        .await
        .expect("the credentials");
    let ack = workers
        .fail(
            &runner,
            &task_ref(&task),
            "OUTCOME_UNKNOWN",
            false,
            "lake-resolve-unknown",
        )
        .await
        .expect("the report of an unknown outcome");
    assert_eq!(
        (ack.step_status.as_str(), ack.run_status.as_str()),
        ("WAITING", "MANUAL_REVIEW")
    );
    let waiting: (String, Option<String>, Option<String>, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT status, waiting_kind, failure_code, output_digest FROM steps WHERE run_id = $1 AND node = 'save'",
    )
    .bind(unknown.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(
        waiting,
        (
            "WAITING".into(),
            Some("OPERATOR".into()),
            Some("OUTCOME_UNKNOWN".into()),
            None
        )
    );

    // The receipt of a lake step is not a SQL result: the route that serves those does not find it (it would otherwise be read
    // through the validator of rows, fail, and answer without a trace).
    {
        use sagat_application::database_results::{
            DatabaseResultRequest, DatabaseResultService, DatabaseResultUseCase,
        };
        let results = DatabaseResultService::new(
            w.store.clone(),
            SystemClock,
            sagat_infra_ir::JsonDatabaseResultValidator,
            None,
        );
        let prepared = results
            .prepare(DatabaseResultRequest {
                principal: w.admin.clone(),
                namespace: ns,
                run,
                node: sagat_domain::workflow::NodeId::parse("save").unwrap(),
                trace_id: None,
            })
            .await;
        assert!(
            matches!(prepared, Err(sagat_application::error::AppError::NotFound)),
            "the result route of SQL steps does not serve a lake step"
        );
    }

    // A logical name that is deleted and written again is a new secret: an attempt that pinned the old one does not get it back.
    let run = start(two, "lake-resolve-start-00007").await;
    let task = lease(run).await;
    workers
        .resolve_database(&runner, &task_ref(&task), None)
        .await
        .expect("the credentials are handed over");
    w.secrets
        .delete(&w.admin, ns, STORAGE, &ctx())
        .await
        .expect("delete the storage's secret");
    assert!(
        workers
            .resolve_database(&runner, &task_ref(&task), None)
            .await
            .is_err(),
        "a deleted secret is not handed over"
    );
    put(w, secrets, STORAGE, 0, "storage-canary-recreated").await;
    assert!(
        matches!(
            workers
                .resolve_database(&runner, &task_ref(&task), None)
                .await,
            Err(sagat_application::error::AppError::NotFound)
        ),
        "recreating the name does not reauthorize the pin of the deleted secret"
    );
    assert_eq!(pins(w, run).await.len(), 2, "the pins are as they were");
    release(task).await;
}

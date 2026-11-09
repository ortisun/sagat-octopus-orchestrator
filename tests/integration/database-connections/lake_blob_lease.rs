//! A lake runner is leased a step whose records are blobs, and the lease pins them (feature 035, slice R, step R5, D035-144).
//!
//! The workflow value that names a blob proves nothing: the lease asks the catalog, takes only what this run's own steps committed, and
//! writes the pins of the attempt it grants in the same transaction. Whatever the catalog does not vouch for fails the step for good, with
//! a closed code, and leases nothing and pins nothing. The run's input is fixed when it starts and a blob is committed by a step that
//! runs after, so the blob rows are written, as the owner does, between the start of the run and its lease.
#![allow(clippy::expect_used, clippy::panic)]
use super::lake::{append_blobs, iceberg_in, publish_on_lake, write};
use super::lake_blobs::{NDJSON, PARQUET, committed_output_as};
use crate::support::engine::{World, ctx, task_ref};
use sagat_application::{
    clock::SystemClock,
    database::ConnectionService,
    entropy::SystemEntropy,
    error::AppError,
    execution::Engine,
    reveal_grants::RevealGrantService,
    runs::{RunService, StartRun},
    worker::{Capabilities, WorkerService, WorkerSettings},
};
use sagat_domain::{
    binding::BindingProfile,
    blobs::BlobId,
    classification::EgressCeiling,
    database::{ConnectionStatus, WriteMode},
    execution::RunId,
    identity::Role,
    workflow::Digest,
};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_lake::protocol::{AttemptInput, Records};
use serde_json::{Value, json};

const ACTIVITY: &str = "sagat.database.iceberg.write";
const TRACE: &str = "0af7651916cd43dd8448eb211c80319c";
const GIB: i64 = 1024 * 1024 * 1024;

/// The descriptor as a workflow value carries it.
fn descriptor(id: BlobId, size: i64, content_type: &str) -> Value {
    json!({"$blob": {
        "id": id.to_string(),
        "size": size,
        "sha256": Digest::new([1; 32]).to_hex(),
        "contentType": content_type,
    }})
}

pub async fn run(w: &World) {
    let ns = w.namespace.id();
    // Every field of a blob is `pii` for the egress check (the bytes are never read), so the connection that takes blobs has that ceiling,
    // which needs the reveal grant of the namespace for as long as it is written.
    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    let grants = RevealGrantService::new(w.store.clone(), SystemClock);
    let scope = grants
        .prepare(&w.admin, ns, &ctx())
        .await
        .expect("the reveal scope");
    grants
        .grant(&scope, w.admin.subject().as_str(), &ctx())
        .await
        .expect("the reveal grant");
    catalog
        .put(
            &w.admin,
            ns,
            write(
                "lake-blob-lease",
                0,
                ConnectionStatus::Active,
                iceberg_in(
                    "https://catalog.example/api",
                    "db-credentials",
                    EgressCeiling::Pii,
                    vec![WriteMode::Append],
                ),
            ),
            &ctx(),
        )
        .await
        .expect("the connection");
    grants
        .revoke(&scope, w.admin.subject().as_str(), &ctx())
        .await
        .expect("the grant is given back");
    let workflow = publish_on_lake(
        w,
        &w.admin,
        append_blobs(
            "lake-blob-lease-flow",
            "lake-blob-lease",
            &["/id", "/total"],
        ),
    )
    .await
    .expect("a workflow whose records are blobs");

    let lake_store = w.store.clone().with_lake_enabled(true);
    let runs = RunService::new(
        lake_store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        w.cache.clone(),
    );
    let workers = WorkerService::new(
        lake_store,
        SystemClock,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        WorkerSettings::default(),
    );
    let runner = w.member("lake-blob-runner", Role::DatabaseRunner).await;
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
            "lake-blob-machine",
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
    let start = |key: &'static str, parts: &Value| {
        let input = serde_json::to_vec(&json!({ "parts": parts })).expect("input");
        let runs = &runs;
        async move {
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
            .expect("a run of an Iceberg write with blobs")
            .info
            .run_id
        }
    };
    let poll = || async {
        workers
            .poll(&runner, registered.worker_id, &token, 8, Some(TRACE))
            .await
            .expect("poll")
    };
    let tenant = w.admin.tenant_id().as_uuid();
    let pins = |run: RunId| async move {
        sqlx::query_as::<_, (i32, String)>(
            "SELECT attempt, blob_id::text FROM step_lease_blobs WHERE tenant_id = $1 AND run_id = $2 ORDER BY attempt, blob_id",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("the pins")
    };
    let failures = |run: RunId| async move {
        sqlx::query_scalar::<_, String>(
            "SELECT detail->>'code' FROM run_events WHERE tenant_id = $1 AND run_id = $2 AND event_type = 'step.failed'",
        )
        .bind(tenant)
        .bind(run.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("the failures")
    };

    // ----- the runs: one the catalog vouches for, and the cases it does not
    let (a1, a2) = (BlobId::generate(), BlobId::generate());
    let good = start(
        "lake-blob-lease-00001",
        &json!([descriptor(a1, 10, NDJSON), descriptor(a2, 20, PARQUET)]),
    )
    .await;
    committed_output_as(w, good, a1, NDJSON, 10).await;
    committed_output_as(w, good, a2, PARQUET, 20).await;

    // A descriptor of another run's blob, which exists and is committed: forged into this run's input.
    let theirs = BlobId::generate();
    let forged = start(
        "lake-blob-lease-00002",
        &json!([descriptor(theirs, 10, NDJSON)]),
    )
    .await;
    committed_output_as(w, good, theirs, NDJSON, 10).await;
    // A descriptor that lies about the size of a blob that is the run's.
    let lie = BlobId::generate();
    let lying = start(
        "lake-blob-lease-00003",
        &json!([descriptor(lie, 11, NDJSON)]),
    )
    .await;
    committed_output_as(w, lying, lie, NDJSON, 10).await;
    // A blob that is not a file of records.
    let text = BlobId::generate();
    let wrong_type = start(
        "lake-blob-lease-00004",
        &json!([descriptor(text, 10, "application/json")]),
    )
    .await;
    committed_output_as(w, wrong_type, text, "application/json", 10).await;
    // Two gibibytes and a byte.
    let (big1, big2) = (BlobId::generate(), BlobId::generate());
    let too_big = start(
        "lake-blob-lease-00005",
        &json!([
            descriptor(big1, GIB, PARQUET),
            descriptor(big2, GIB + 1, PARQUET)
        ]),
    )
    .await;
    committed_output_as(w, too_big, big1, PARQUET, GIB).await;
    committed_output_as(w, too_big, big2, PARQUET, GIB + 1).await;
    // Thirty-three blobs: refused before the catalog is asked. Were it asked, the store would refuse a list of 33 itself
    // (`DatabaseLimit`) and the poll would fail, not the step: the poll below is what holds the order.
    let crowd: Vec<Value> = (0..33)
        .map(|_| descriptor(BlobId::generate(), 1, NDJSON))
        .collect();
    let too_many = start("lake-blob-lease-00006", &Value::Array(crowd)).await;

    // ----- the lease: the vouched run is handed over with its blobs named, and none of the others is handed over at all
    let mut handed = Vec::new();
    let mut task = None;
    for _ in 0..20 {
        for leased in poll().await {
            if leased.run_id == good {
                task = Some(leased);
            } else {
                handed.push(leased.run_id);
            }
        }
        if task.is_some() {
            break;
        }
    }
    let task =
        task.expect("the lake runner is handed the step whose blobs the catalog vouches for");
    // The others are judged in the same polls or the next ones: give the lease path a few rounds to reach each.
    for _ in 0..6 {
        for leased in poll().await {
            handed.push(leased.run_id);
        }
    }
    let input =
        AttemptInput::read(&task.input).expect("the runner's own reader accepts the document");
    let Records::Blobs(blobs) = input.records() else {
        panic!("blobs expected: {}", String::from_utf8_lossy(&task.input));
    };
    let named: Vec<(String, String)> = blobs
        .iter()
        .map(|blob| (blob.id.to_string(), format!("{:?}", blob.format)))
        .collect();
    assert_eq!(
        named,
        vec![
            (a1.to_string(), "Ndjson".to_owned()),
            (a2.to_string(), "Parquet".to_owned())
        ],
        "the blobs the step names, in its order, each with the format of its own media type"
    );
    assert_eq!(
        pins(good).await,
        {
            let mut expected = vec![
                (
                    i32::try_from(task.fence.attempt).expect("attempt"),
                    a1.to_string(),
                ),
                (
                    i32::try_from(task.fence.attempt).expect("attempt"),
                    a2.to_string(),
                ),
            ];
            expected.sort();
            expected
        },
        "the lease wrote the pins of the attempt it granted, and only of the blobs it names"
    );

    // The pin is what the read asks: the named blobs are read, the run's other blob is not.
    for blob in [a1, a2] {
        workers
            .check_blob_read(&runner, &task_ref(&task), blob, Some(TRACE))
            .await
            .expect("a blob the lease named");
    }
    let refused = workers
        .check_blob_read(&runner, &task_ref(&task), theirs, Some(TRACE))
        .await;
    assert!(
        matches!(refused, Err(AppError::NotFound)),
        "a blob of this tenant that the lease did not name: {refused:?}"
    );

    // ----- what the catalog does not vouch for fails the step for good, leases nothing and pins nothing
    for (name, run, code) in [
        (
            "a descriptor of another run's blob",
            forged,
            "DATABASE_RESULT_UNAVAILABLE",
        ),
        (
            "a descriptor that lies about the size",
            lying,
            "DATABASE_RESULT_UNAVAILABLE",
        ),
        (
            "a blob that is not a file of records",
            wrong_type,
            "DATABASE_CONFIGURATION_INVALID",
        ),
        (
            "more than two gibibytes together",
            too_big,
            "ICEBERG_LIMIT_EXCEEDED",
        ),
        (
            "thirty-three blobs",
            too_many,
            "DATABASE_CONFIGURATION_INVALID",
        ),
    ] {
        assert!(!handed.contains(&run), "{name}: nothing was leased");
        assert_eq!(failures(run).await, vec![code.to_owned()], "{name}");
        assert_eq!(
            w.run_status(run).await,
            sagat_domain::execution::RunStatus::Failed,
            "{name}"
        );
        assert!(
            pins(run).await.is_empty(),
            "{name}: a lease that was not granted pins nothing"
        );
    }

    // ----- a retry is a lease of its own, with pins of its own
    let failed = workers
        .fail(
            &runner,
            &task_ref(&task),
            "CATALOG_UNAVAILABLE",
            true,
            "lake-blob-lease-fail-1",
        )
        .await;
    assert!(failed.is_ok(), "{failed:?}");
    w.age_retry(good, "save").await;
    w.scheduler.tick().await;
    let mut second = None;
    for _ in 0..20 {
        second = poll()
            .await
            .into_iter()
            .find(|leased| leased.run_id == good);
        if second.is_some() {
            break;
        }
    }
    let second = second.expect("the second attempt is leased");
    assert_eq!(second.fence.attempt, 2);
    let again = AttemptInput::read(&second.input).expect("the second document is read too");
    assert!(matches!(again.records(), Records::Blobs(blobs) if blobs.len() == 2));
    let rows = pins(good).await;
    assert_eq!(
        rows.iter().map(|(attempt, _)| *attempt).collect::<Vec<_>>(),
        vec![1, 1, 2, 2],
        "each attempt holds the pins of its own lease: {rows:?}"
    );
    workers
        .check_blob_read(&runner, &task_ref(&second), a1, Some(TRACE))
        .await
        .expect("the second attempt reads what its own lease named");
    let stale = workers
        .check_blob_read(&runner, &task_ref(&task), a1, Some(TRACE))
        .await;
    assert!(
        matches!(stale, Err(AppError::StaleAttempt)),
        "and the first attempt's token reads nothing (the pin of the second attempt is keyed by its own fence, which lake_read.rs holds): {stale:?}"
    );

    // The namespace's budget of lake attempts is counted by the scenario that follows: the step ends for good.
    let _ = workers
        .fail(
            &runner,
            &task_ref(&second),
            "ICEBERG_LIMIT_EXCEEDED",
            false,
            "lake-blob-lease-release",
        )
        .await;
}

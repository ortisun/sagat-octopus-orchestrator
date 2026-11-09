//! The start of the probe of an Iceberg connection, at the service (feature 035, D035-150 slice S2a, ADR-0080): a real control plane over
//! real PostgreSQL, the REST API that creates the connections and the secrets, and a `RunService` over the same store.
//!
//! The probe is a **private run** whose single node the platform writes. What is held here, in the order a run lives it: it is admitted
//! (its connection is bound at the revision it was started on, as a write's is), it is stored as a private workflow, the loader restores it
//! through the private entry **because its workflow is internal and for no other reason**, and a start that is refused creates nothing: a
//! SQL connection, a connection that is not there, a paused one, a deployment with the lake off, and a caller who may not run it.
//!
//! The REST probe still refuses an Iceberg connection (slice S6): this is the service, which nothing public calls yet.
//!
//! It runs inside the lifecycle scenario of `main.rs`, with the OpenBao of that scenario.
#![allow(clippy::expect_used, clippy::panic)]
use crate::support::{
    api::Options,
    engine::{ctx, document},
    subject,
    worker::{HUMAN, Platform, platform_with},
};
use reqwest::Method;
use sagat_application::{
    clock::SystemClock,
    error::AppError,
    execution::{DefinitionCache, Engine, RunTx},
    runs::{Lock, RunOrigin, RunService, RunStore},
    secret_provider::SecretProvider,
};
use sagat_domain::{
    database::{ConnectionRef, ICEBERG_PROBE_ACTIVITY},
    execution::RunId,
    identity::{NamespaceId, Principal, Role, Slug},
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgRunStore;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub(super) type Runs = RunService<PgRunStore, SystemClock, JsonEvaluator>;

pub(super) fn runs_of(p: &Platform) -> Runs {
    RunService::new(
        p.api.executions.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        DefinitionCache::new(Arc::new(JsonIrCodec), 16),
    )
    .with_lake_probes(Arc::new(sagat_infra_ir::JsonLakeProbeCodec))
}

pub(super) fn reference(name: &str, revision: Option<u32>) -> ConnectionRef {
    ConnectionRef::new(Slug::parse(name).expect("a slug"), revision).expect("a reference")
}

async fn put_secret(p: &Platform, ns: &str, name: &str, value: &str) {
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

async fn put_connection(
    p: &Platform,
    ns: &str,
    name: &str,
    revision: u32,
    status: &str,
    config: &Value,
) {
    let (code, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/connections/{name}"),
            Some(&json!({
                "requestId": Uuid::new_v4().to_string(), "expectedRevision": revision,
                "status": status, "config": config
            })),
        )
        .await;
    assert_eq!(code, 200, "{name}: {body}");
}

fn lake_config() -> Value {
    json!({
        "driver": "iceberg",
        "catalog": {"type": "rest", "rest": {
            "uri": "https://catalog.example/api", "warehouse": "analytics",
            "auth": {"type": "oauth2", "tokenUri": "https://catalog.example/oauth/token"},
            "accessDelegation": "none", "remoteSigning": false
        }},
        "storage": {"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": false}, "credentials": "static"},
        "warehouseLocations": ["s3://lake/warehouse/"],
        "allowedModes": ["append"],
        "catalogSecret": "probe-catalog",
        "storageSecret": "probe-storage",
        "limits": {}
    })
}

fn sql_config() -> Value {
    json!({
        "driver": "postgresql", "host": "db.example.com", "port": 5432, "database": "test",
        "tls": {"mode": "verify-full"}, "secretName": "probe-sql-credential"
    })
}

/// A platform with the three kinds of connection a probe can be asked about, and its run service.
pub(super) async fn platform(
    provider: &Arc<dyn SecretProvider>,
    lake: bool,
) -> (Platform, String, Runs) {
    let p = platform_with(Options {
        secret_provider: Some(provider.clone()),
        lake,
        ..Default::default()
    })
    .await;
    let ns = p.namespace_id().await;
    put_secret(
        &p,
        &ns,
        "probe-catalog",
        r#"{"clientId":"c","clientSecret":"probe-catalog-secret"}"#,
    )
    .await;
    put_secret(
        &p,
        &ns,
        "probe-storage",
        r#"{"accessKeyId":"k","secretAccessKey":"probe-storage-secret"}"#,
    )
    .await;
    put_secret(
        &p,
        &ns,
        "probe-sql-credential",
        r#"{"username":"u","password":"probe-sql-secret"}"#,
    )
    .await;
    put_connection(&p, &ns, "probe-warehouse", 0, "ACTIVE", &lake_config()).await;
    put_connection(&p, &ns, "probe-sql", 0, "ACTIVE", &sql_config()).await;
    put_connection(&p, &ns, "probe-paused", 0, "ACTIVE", &lake_config()).await;
    put_connection(&p, &ns, "probe-paused", 1, "PAUSED", &lake_config()).await;
    let runs = runs_of(&p);
    (p, ns, runs)
}

async fn count(p: &Platform, table: &str) -> i64 {
    let statement = format!("SELECT count(*) FROM {table} WHERE tenant_id = $1");
    sqlx::query_scalar(sqlx::AssertSqlSafe(statement))
        .bind(p.tenant.as_uuid())
        .fetch_one(&p.api.db.pool)
        .await
        .expect("a count")
}

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let (p, ns, runs) = platform(&provider, true).await;
    let namespace: NamespaceId = ns.parse().expect("a namespace id");
    let admin = Principal::new(p.tenant, subject(HUMAN), Role::Admin);
    let key = |n: u32| format!("lake-probe-start-{n:04}");

    // --- A probe of an Iceberg connection is a run of its own origin, admitted and bound.
    let started = runs
        .start_lake_probe(
            &admin,
            namespace,
            &reference("probe-warehouse", None),
            &key(1),
            &ctx(),
        )
        .await
        .expect("the probe of an Iceberg connection starts");
    let run = started.info.run_id;
    assert_eq!(started.info.origin, Some(RunOrigin::ConnectionProbe));
    let bindings: Vec<(String, i32)> = sqlx::query_as(
        "SELECT node_id, revision FROM run_database_connections WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&p.api.db.pool)
    .await
    .expect("the bindings");
    assert_eq!(
        bindings,
        vec![("probe".to_owned(), 1)],
        "the connection is bound to the node at the revision the probe was started on, as a write's is"
    );
    let step: (String, Option<String>) = sqlx::query_as(
        "SELECT status, activity FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'probe'",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the step");
    assert_eq!(
        step,
        ("READY".to_owned(), Some(ICEBERG_PROBE_ACTIVITY.to_owned())),
        "the step waits for a runner that offers the probe"
    );

    // --- It is a private workflow, and the loader restores it through the private entry because the workflow is internal.
    let internal: bool = sqlx::query_scalar(
        "SELECT w.internal FROM workflows w JOIN runs r ON r.tenant_id = w.tenant_id AND r.workflow_id = w.id WHERE r.tenant_id = $1 AND r.id = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the workflow");
    assert!(internal, "a probe's workflow is private");
    let mut tx = p
        .api
        .executions
        .begin(p.tenant, run, Lock::Wait)
        .await
        .expect("the store")
        .expect("the run");
    let node = tx.parts().0.nodes().next().expect("the one node").clone();
    assert_eq!(
        node.lake_probe_target()
            .map(|target| target.connection().name.as_str().to_owned()),
        Some("probe-warehouse".to_owned()),
        "the stored definition restores as a probe"
    );
    drop(tx);

    // --- The same key is the same run; another key is another probe.
    let again = runs
        .start_lake_probe(
            &admin,
            namespace,
            &reference("probe-warehouse", None),
            &key(1),
            &ctx(),
        )
        .await
        .expect("a replay");
    assert_eq!(again.info.run_id, run, "an idempotent start");
    let second = runs
        .start_lake_probe(
            &admin,
            namespace,
            &reference("probe-warehouse", Some(1)),
            &key(2),
            &ctx(),
        )
        .await
        .expect("a probe of a named revision");
    assert_ne!(second.info.run_id, run);

    // --- A start that is refused creates nothing, and says no more than "not found" about a connection it cannot probe, whatever the reason.
    let runs_before = count(&p, "runs").await;
    for (what, name, revision) in [
        ("a SQL connection", "probe-sql", None),
        ("a connection that is not there", "probe-absent", None),
        ("a revision that is not there", "probe-warehouse", Some(9)),
    ] {
        let refused = runs
            .start_lake_probe(
                &admin,
                namespace,
                &reference(name, revision),
                &key(3),
                &ctx(),
            )
            .await
            .expect_err(what);
        assert!(
            matches!(refused, AppError::NotFound),
            "{what} is not a connection a lake probe can test, and nothing says which: {refused:?}"
        );
    }
    assert!(
        runs.start_lake_probe(
            &admin,
            namespace,
            &reference("probe-paused", None),
            &key(4),
            &ctx()
        )
        .await
        .is_err(),
        "a paused connection is not probed"
    );
    let nobody = Principal::new(p.tenant, subject("nobody-0001"), Role::Viewer);
    assert!(
        runs.start_lake_probe(
            &nobody,
            namespace,
            &reference("probe-warehouse", None),
            &key(5),
            &ctx()
        )
        .await
        .is_err(),
        "a caller who may not run an ad hoc database operation does not start one"
    );
    assert_eq!(
        count(&p, "runs").await,
        runs_before,
        "and none of them left a run behind"
    );

    // --- No public path starts a probe. The document of a probe, handed to the ad hoc endpoint as an ordinary execution, is refused as any
    // unknown node is (the public grammar does not know it), and the REST probe still refuses an Iceberg connection until a runner has a
    // probe to run: no run of this kind can be made by anyone who is not the platform.
    let probe_json: Value = serde_json::from_slice(&sagat_application::runs::lake_probe_document(
        &reference("probe-warehouse", None),
    ))
    .expect("the document is JSON");
    let runs_before = count(&p, "runs").await;
    let (status, refused) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/database-executions"),
            Some(&json!({"mode": "step", "definition": probe_json})),
        )
        .await;
    assert!(
        (400..500).contains(&status),
        "the probe document is not an ad hoc execution ({status}): {refused}"
    );
    let (status, refused) = p
        .rest(
            Method::POST,
            &format!("/namespaces/{ns}/connections/probe-warehouse/probe"),
            Some(&json!({})),
        )
        .await;
    assert!(
        status != 201,
        "the REST probe does not start a probe of an Iceberg connection yet ({status}): {refused}"
    );
    assert_eq!(
        count(&p, "runs").await,
        runs_before,
        "and neither made a run"
    );

    // --- With the lake off a probe is refused at the start, instead of waiting for a runner that is not there.
    let (off, off_ns, off_runs) = platform(&provider, false).await;
    let off_namespace: NamespaceId = off_ns.parse().expect("a namespace id");
    let off_admin = Principal::new(off.tenant, subject(HUMAN), Role::Admin);
    let before = count(&off, "runs").await;
    assert!(
        off_runs
            .start_lake_probe(
                &off_admin,
                off_namespace,
                &reference("probe-warehouse", None),
                &key(6),
                &ctx()
            )
            .await
            .is_err(),
        "the lake is off"
    );
    assert_eq!(count(&off, "runs").await, before);

    // --- The row decides, not the document: a public version is never restored through the private entry, whatever it holds.
    // An ordinary document parses the same under both grammars, so only a document that is private shows which entry restored it. A
    // version of a public workflow is made to hold one (as the owner, with the immutability trigger off for that statement and on again
    // after it: a test that has to change what the platform guarantees unchangeable says so), and a store with an empty cache, which has
    // nothing parsed before to serve, must refuse to load it, while it loads the private version of the probe through the same cache.
    let public = p
        .publish(
            "probe-public-flow",
            &document(
                "probe-public-flow",
                "done",
                vec![json!({"id": "done", "type": "end"})],
            ),
        )
        .await;
    let public_run: RunId =
        RunId::new(Uuid::parse_str(&p.start(&public, &json!({})).await).expect("a run id"));
    let fresh = || {
        PgRunStore::new(
            p.api.api_pool.clone(),
            DefinitionCache::new(Arc::new(JsonIrCodec), 4),
        )
    };
    assert!(
        fresh()
            .begin(p.tenant, public_run, Lock::Wait)
            .await
            .expect("a public version loads")
            .is_some(),
        "control: the public version loads while it holds an ordinary document"
    );
    assert!(
        fresh()
            .begin(p.tenant, run, Lock::Wait)
            .await
            .expect("a private version loads")
            .is_some(),
        "control: the private version of the probe loads through a store that has parsed nothing"
    );
    let probe_document: Vec<u8> = sqlx::query_scalar(
        "SELECT v.document FROM workflow_versions v JOIN runs r ON r.tenant_id = v.tenant_id AND r.workflow_version_id = v.id WHERE r.tenant_id = $1 AND r.id = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the document of the probe");
    sqlx::query("ALTER TABLE workflow_versions DISABLE TRIGGER workflow_versions_immutable")
        .execute(&p.api.db.pool)
        .await
        .expect("the trigger is off");
    let changed = sqlx::query(
        "UPDATE workflow_versions SET document = $3 WHERE tenant_id = $1 AND id = (SELECT workflow_version_id FROM runs WHERE tenant_id = $1 AND id = $2)",
    )
    .bind(p.tenant.as_uuid())
    .bind(public_run.as_uuid())
    .bind(&probe_document)
    .execute(&p.api.db.pool)
    .await;
    sqlx::query("ALTER TABLE workflow_versions ENABLE ALWAYS TRIGGER workflow_versions_immutable")
        .execute(&p.api.db.pool)
        .await
        .expect("the trigger is on again");
    assert_eq!(changed.expect("the document is put").rows_affected(), 1);
    assert!(
        fresh()
            .begin(p.tenant, public_run, Lock::Wait)
            .await
            .is_err(),
        "a public version that holds a private document is unreadable: the workflow says which entry restores it, and the document cannot"
    );
    // Run creation reads the stored document of the version as well, for its declared state, and a public version is read by the public
    // entry there too: it cannot start a run, whatever the cache holds of it.
    let (status, started) = p
        .rest(
            Method::POST,
            "/runs",
            Some(&json!({"workflowId": public, "input": {}})),
        )
        .await;
    assert!(
        status >= 500,
        "a run of a public version that holds a private document is not created ({status}): {started}"
    );
}

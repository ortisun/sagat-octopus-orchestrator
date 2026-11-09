//! The start of the probe of an Iceberg connection, at the service (feature 035, D035-150 slice S2a, ADR-0080): a real control plane over
//! real PostgreSQL, the REST API that creates the connections and the secrets, and a `RunService` over the same store.
//!
//! The probe is a **private run** whose single node the platform writes. What is held here, in the order a run lives it: it is admitted
//! (its connection is bound at the revision it was started on, as a write's is), it is stored as a private workflow, the loader restores it
//! through the private entry **because its workflow is internal and for no other reason**, and a start that is refused creates nothing: a
//! SQL connection, a connection that is not there, a paused one, a deployment with the lake off, and a caller who may not run it.
//!
//! The REST probe starts it as well (slice S6c), chosen by the kind of the connection, with the rules of the service.
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
    quota::QuotaResource,
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgRunStore;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
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
    // two more of the same kind: a probe in flight holds its own connection and revision, and not these
    put_connection(&p, &ns, "probe-second", 0, "ACTIVE", &lake_config()).await;
    put_connection(&p, &ns, "probe-rest", 0, "ACTIVE", &lake_config()).await;
    // and two for the starts that race
    put_connection(&p, &ns, "probe-race", 0, "ACTIVE", &lake_config()).await;
    put_connection(&p, &ns, "probe-race-b", 0, "ACTIVE", &lake_config()).await;
    put_connection(&p, &ns, "probe-sql", 0, "ACTIVE", &sql_config()).await;
    // a connection whose first revision is an Iceberg one and whose current revision is a SQL one: the kind is read at the revision asked for
    put_connection(&p, &ns, "probe-switch", 0, "ACTIVE", &lake_config()).await;
    put_connection(&p, &ns, "probe-switch", 1, "ACTIVE", &sql_config()).await;
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

/// The quota lock of the active runs of the namespace, which every start in it takes, held from a side transaction: the starts that are
/// made meanwhile have done their replay lookup and wait at it, and are let go together when it is released.
async fn hold_the_namespace(
    p: &Platform,
    namespace: NamespaceId,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let key = format!(
        "quota:{}:{}:{namespace}",
        QuotaResource::ActiveRuns,
        p.tenant
    );
    let mut held = p.api.db.pool.begin().await.expect("a side transaction");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(key)
        .execute(&mut *held)
        .await
        .expect("the lock");
    held
}

/// Waits until `n` sessions wait for an advisory lock, read from `pg_stat_activity`: the starts are at the lock, past everything that
/// comes before it.
async fn waiting_for_the_lock(p: &Platform, n: i64) {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND wait_event = 'advisory'",
            )
            .fetch_one(&p.api.db.pool)
            .await
            .expect("the sessions");
            if waiting >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the starts reach the lock");
}

/// How many probes were ever made of the connection `name`.
async fn probes_of(p: &Platform, name: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM run_database_connections b JOIN database_connections c ON c.tenant_id = b.tenant_id AND c.namespace_id = b.namespace_id AND c.id = b.connection_id WHERE b.tenant_id = $1 AND c.name = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(name)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("a count")
}

/// Starts that are made at the same time, held at the lock of the namespace until both have reached it (a held lock and
/// `pg_stat_activity`, and no sleep that a machine could outlast).
async fn races(p: &Platform, ns: &str, runs: &Runs) {
    let namespace: NamespaceId = ns.parse().expect("a namespace id");
    let admin = Principal::new(p.tenant, subject(HUMAN), Role::Admin);

    // --- The retry of a start that has not finished is that start, and not a conflict: a client that times out and sends the same key
    // again can reach the check while the first start holds the lock, and the first one's run is what the key stands for.
    let connection = reference("probe-race", None);
    let held = hold_the_namespace(p, namespace).await;
    let (first_ctx, retry_ctx) = (ctx(), ctx());
    let first = runs.start_lake_probe(
        &admin,
        namespace,
        &connection,
        "lake-probe-race-0001",
        &first_ctx,
    );
    let retry = runs.start_lake_probe(
        &admin,
        namespace,
        &connection,
        "lake-probe-race-0001",
        &retry_ctx,
    );
    let release = async {
        waiting_for_the_lock(p, 2).await;
        held.commit().await.expect("the lock is released");
    };
    let (first, retry, ()) = tokio::join!(first, retry, release);
    let first = first.expect("the first start");
    let retry = retry.expect("the retry of the same key is the same run, not a conflict");
    assert_eq!(first.info.run_id, retry.info.run_id);
    assert_eq!(probes_of(p, "probe-race").await, 1, "and one run was made");

    // --- Two starts of one connection and revision under different keys: one is made and the other is refused, whichever gets the lock
    // first, and the refused one leaves nothing behind.
    let connection = reference("probe-race-b", None);
    let held = hold_the_namespace(p, namespace).await;
    let (a_ctx, b_ctx) = (ctx(), ctx());
    let a = runs.start_lake_probe(
        &admin,
        namespace,
        &connection,
        "lake-probe-race-0002",
        &a_ctx,
    );
    let b = runs.start_lake_probe(
        &admin,
        namespace,
        &connection,
        "lake-probe-race-0003",
        &b_ctx,
    );
    let release = async {
        waiting_for_the_lock(p, 2).await;
        held.commit().await.expect("the lock is released");
    };
    let (a, b, ()) = tokio::join!(a, b, release);
    let (made, refused) = [&a, &b]
        .iter()
        .fold((0, 0), |(made, refused), result| match result {
            Ok(_) => (made + 1, refused),
            Err(AppError::StateConflict(_)) => (made, refused + 1),
            Err(_) => (made, refused),
        });
    assert_eq!(
        (made, refused),
        (1, 1),
        "one start is made and one is a conflict: {:?} and {:?}",
        a.as_ref().map(|run| run.info.run_id),
        b.as_ref().map(|run| run.info.run_id)
    );
    assert_eq!(
        probes_of(p, "probe-race-b").await,
        1,
        "and one run was made"
    );
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

    // --- The same key is the same run, also while it is in flight. Another key is another probe only for another connection: a probe costs a
    // container, two secret reads and a call to the customer's catalog, so at most one is in flight for a connection and a revision, whether
    // the revision is named or is the current one.
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
    let runs_in_flight = count(&p, "runs").await;
    for (what, revision) in [("the current revision", None), ("a named one", Some(1))] {
        let refused = runs
            .start_lake_probe(
                &admin,
                namespace,
                &reference("probe-warehouse", revision),
                &key(2),
                &ctx(),
            )
            .await
            .expect_err(what);
        assert!(
            matches!(refused, AppError::StateConflict(_)),
            "a probe of this connection and revision is in flight, {what}: {refused:?}"
        );
    }
    assert_eq!(
        count(&p, "runs").await,
        runs_in_flight,
        "and the refused starts left no run behind"
    );
    let second = runs
        .start_lake_probe(
            &admin,
            namespace,
            &reference("probe-second", None),
            &key(2),
            &ctx(),
        )
        .await
        .expect("a probe of another connection");
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
    // unknown node is (the public grammar does not know it): no run of this kind is made from a document of the caller's, and the
    // REST probe (below) writes the document itself.
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
    assert_eq!(count(&p, "runs").await, runs_before, "and it made no run");

    // --- The REST probe starts the probe of an Iceberg connection, chosen by the kind of the connection, with the same rules as the service:
    // one in flight for a connection and a revision (a conflict, and no run made), and the probe of a SQL connection is what it always was.
    let probe_path = |name: &str| format!("/namespaces/{ns}/connections/{name}/probe");
    let (status, refused) = p
        .rest(
            Method::POST,
            &probe_path("probe-warehouse"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(
        status, 409,
        "a probe of this connection is in flight: {refused}"
    );
    assert_eq!(count(&p, "runs").await, runs_before, "and it made no run");
    let (status, started) = p
        .rest(Method::POST, &probe_path("probe-rest"), Some(&json!({})))
        .await;
    assert_eq!(status, 201, "the probe of an Iceberg connection: {started}");
    let rest_run: Uuid = started["runId"]
        .as_str()
        .expect("the run")
        .parse()
        .expect("a run id");
    let step: (String, Option<String>) = sqlx::query_as(
        "SELECT status, activity FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'probe'",
    )
    .bind(p.tenant.as_uuid())
    .bind(rest_run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the step");
    assert_eq!(
        step,
        ("READY".to_owned(), Some(ICEBERG_PROBE_ACTIVITY.to_owned())),
        "the REST probe of an Iceberg connection waits for a runner that offers the probe"
    );
    let (status, refused) = p
        .rest(Method::POST, &probe_path("probe-rest"), Some(&json!({})))
        .await;
    assert_eq!(status, 409, "and holds its connection: {refused}");
    let (status, refused) = p
        .rest(
            Method::POST,
            &probe_path("probe-rest"),
            Some(&json!({"revision": 1})),
        )
        .await;
    assert_eq!(status, 409, "also when the revision is named: {refused}");
    // a revision is its own connection to a probe: another revision starts while this one is in flight, and each holds the one it was
    // started on, whether the caller named it or took the current one
    put_connection(&p, &ns, "probe-rest", 1, "ACTIVE", &lake_config()).await;
    let (status, other) = p
        .rest(
            Method::POST,
            &probe_path("probe-rest"),
            Some(&json!({"revision": 2})),
        )
        .await;
    assert_eq!(
        status, 201,
        "a probe of another revision is not held by this one: {other}"
    );
    for (what, body) in [
        ("the first revision", json!({"revision": 1})),
        ("the second", json!({"revision": 2})),
        ("the current one, which is the second", json!({})),
    ] {
        let (status, refused) = p
            .rest(Method::POST, &probe_path("probe-rest"), Some(&body))
            .await;
        assert_eq!(status, 409, "a probe of {what} is in flight: {refused}");
    }
    let (status, first) = p
        .rest(
            Method::POST,
            &probe_path("probe-switch"),
            Some(&json!({"revision": 1})),
        )
        .await;
    assert_eq!(
        status, 201,
        "the kind is the kind of the revision asked for, an Iceberg one: {first}"
    );
    let switched: Uuid = first["runId"]
        .as_str()
        .expect("the run")
        .parse()
        .expect("a run id");
    let activity: Option<String> = sqlx::query_scalar(
        "SELECT activity FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = 'probe'",
    )
    .bind(p.tenant.as_uuid())
    .bind(switched)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the step");
    assert_eq!(activity.as_deref(), Some(ICEBERG_PROBE_ACTIVITY));
    let (status, current) = p
        .rest(Method::POST, &probe_path("probe-switch"), Some(&json!({})))
        .await;
    assert_eq!(
        status, 201,
        "and the current revision, a SQL one, is the SQL probe: {current}"
    );
    let (status, sql) = p
        .rest(Method::POST, &probe_path("probe-sql"), Some(&json!({})))
        .await;
    assert_eq!(status, 201, "the probe of a SQL connection: {sql}");
    let (status, again) = p
        .rest(Method::POST, &probe_path("probe-sql"), Some(&json!({})))
        .await;
    assert_eq!(
        status, 201,
        "a SQL probe is not held to one in flight, as it never was: {again}"
    );
    let (status, refused) = p
        .rest(Method::POST, &probe_path("probe-paused"), Some(&json!({})))
        .await;
    assert_eq!(
        status, 404,
        "a paused connection is not probed, and is answered as one that is not there: {refused}"
    );
    let (status, refused) = p
        .rest(Method::POST, &probe_path("probe-absent"), Some(&json!({})))
        .await;
    assert_eq!(
        status, 404,
        "a connection that is not there is not found: {refused}"
    );

    races(&p, &ns, &runs).await;

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

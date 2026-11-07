//! The Iceberg connection through the real service and the real store (feature 035, T035-039, T035-042): written as the
//! document of the contract, read back, audited, refused where a rule says so, and unusable by a SQL step.
//!
//! It runs inside the lifecycle scenario of `main.rs`, after the managed secret `db-credentials` exists in OpenBao.
#![allow(clippy::panic)]
use crate::support::engine::{World, ctx, document};
use sagat_application::{
    clock::SystemClock,
    database::{ConnectionService, ConnectionWrite},
    definitions::IrError,
    error::AppError,
    managed_secrets::{ManagedSecretService, ManagedSecretWrite},
    publishing::Promotion,
    reveal_grants::RevealGrantService,
    worker::Capabilities,
};
use sagat_domain::{
    audit::AuditAction,
    classification::{EgressCeiling, Level},
    database::{
        AccessDelegation, ConnectionConfig, ConnectionStatus, DatabaseLimits, EgressRefusal,
        IcebergCatalog, IcebergConfig, LakeLimits, LakeRefusal, LakeStorage, RestAuth, RestCatalog,
        S3Storage, StorageCredentials, StorageKind, TlsPolicy, WriteMode,
    },
    definition::WorkflowId,
    identity::{Environment, Permission, Principal, Role, Slug},
    secrets::{SecretName, SecretValue},
};
use sagat_infra_postgres::PgRunStore;
use serde_json::{Value, json};
use uuid::Uuid;

fn iceberg(uri: &str, catalog_secret: &str, ceiling: EgressCeiling) -> ConnectionConfig {
    iceberg_in(
        uri,
        catalog_secret,
        ceiling,
        vec![WriteMode::Append, WriteMode::Overwrite],
    )
}

fn iceberg_in(
    uri: &str,
    catalog_secret: &str,
    ceiling: EgressCeiling,
    modes: Vec<WriteMode>,
) -> ConnectionConfig {
    ConnectionConfig::iceberg(IcebergConfig {
        catalog: IcebergCatalog::Rest(RestCatalog {
            uri: uri.to_owned(),
            warehouse: "analytics".to_owned(),
            prefix: None,
            auth: RestAuth::OAuth2 {
                token_uri: "https://catalog.example/oauth/token".to_owned(),
                scope: None,
            },
            access_delegation: AccessDelegation::None,
            remote_signing: false,
        }),
        storage: LakeStorage {
            kind: StorageKind::S3(S3Storage {
                region: "eu-west-1".to_owned(),
                endpoint: None,
                path_style: false,
                role_arn: None,
                external_id: None,
            }),
            credentials: StorageCredentials::Static,
        },
        warehouse_locations: vec!["s3://lake/warehouse/".to_owned()],
        allowed_modes: modes,
        catalog_secret: Some(SecretName::parse(catalog_secret).unwrap()),
        storage_secret: Some(SecretName::parse("db-credentials").unwrap()),
        egress_ceiling: ceiling,
        limits: LakeLimits::default(),
    })
    .unwrap()
}

fn write(
    name: &str,
    expected_revision: u32,
    status: ConnectionStatus,
    config: ConnectionConfig,
) -> ConnectionWrite {
    ConnectionWrite {
        name: Slug::parse(name).unwrap(),
        expected_revision,
        request_id: Uuid::new_v4(),
        status,
        config,
    }
}

/// A workflow whose `database` step names the connection: what a SQL step would do with it.
fn sql_step(workflow: &str, connection: &str) -> Value {
    document(
        workflow,
        "database",
        vec![json!({
            "id": "database", "type": "database", "connectionRef": {"name": connection},
            "operation": "query", "statement": "SELECT 1"
        })],
    )
}

async fn stored(
    w: &World,
    connection: sagat_domain::database::ConnectionId,
    revision: i64,
) -> Value {
    sqlx::query_scalar(
        "SELECT config FROM database_connection_revisions WHERE tenant_id=$1 AND connection_id=$2 AND revision=$3",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(connection.as_uuid())
    .bind(revision)
    .fetch_one(&w.db.pool)
    .await
    .unwrap()
}

pub async fn run(w: &World, secrets: &ManagedSecretService<PgRunStore, SystemClock>) {
    let catalog = ConnectionService::new(w.store.clone(), SystemClock);
    let ns = w.namespace.id();
    let name = Slug::parse("lake-main").unwrap();

    // Written, and read back as it was written.
    let first = write(
        "lake-main",
        0,
        ConnectionStatus::Active,
        iceberg(
            "https://catalog.example/api",
            "db-credentials",
            EgressCeiling::Internal,
        ),
    );
    let record = catalog
        .put(&w.admin, ns, first.clone(), &ctx())
        .await
        .unwrap();
    assert_eq!(record.revision, 1);
    assert_eq!(record.config, first.config);
    assert_eq!(
        catalog
            .get(&w.admin, ns, &name, None, &ctx())
            .await
            .unwrap()
            .config,
        first.config
    );
    // The retry of the same write is answered from its receipt.
    assert_eq!(
        catalog
            .put(&w.admin, ns, first.clone(), &ctx())
            .await
            .unwrap(),
        record
    );

    // The document in the database is the one of the contract, and it holds references to secrets and no value.
    let document_in_database = stored(w, record.id, 1).await;
    assert_eq!(
        document_in_database,
        json!({
            "driver": "iceberg",
            "catalog": {"type": "rest", "rest": {
                "uri": "https://catalog.example/api", "warehouse": "analytics",
                "auth": {"type": "oauth2", "tokenUri": "https://catalog.example/oauth/token"},
                "accessDelegation": "none", "remoteSigning": false
            }},
            "storage": {"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": false}, "credentials": "static"},
            "warehouseLocations": ["s3://lake/warehouse/"],
            "allowedModes": ["append", "overwrite"],
            "catalogSecret": "db-credentials",
            "storageSecret": "db-credentials",
            "limits": {}
        })
    );

    // A change is audited like any other revision of a connection.
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action=$2 AND resource_id LIKE '%lake-main'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(AuditAction::DatabaseConnectionChanged.as_str())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);

    // The listing holds connections of both kinds.
    let listed = catalog.list(&w.admin, ns, &ctx()).await.unwrap();
    assert!(
        listed
            .iter()
            .any(|connection| connection.name == name && connection.config.driver() == "iceberg")
    );

    // Another tenant does not see it.
    assert!(matches!(
        catalog.get(&w.other_admin, ns, &name, None, &ctx()).await,
        Err(AppError::NotFound)
    ));

    // A secret that does not exist is a missing reference, and nothing is written for it.
    let missing = write(
        "lake-missing-secret",
        0,
        ConnectionStatus::Active,
        iceberg(
            "https://catalog.example/api",
            "no-such-secret",
            EgressCeiling::Internal,
        ),
    );
    assert!(matches!(
        catalog.put(&w.admin, ns, missing, &ctx()).await,
        Err(AppError::NotFound)
    ));
    let heads: i64 = sqlx::query_scalar("SELECT count(*) FROM database_connections WHERE tenant_id=$1 AND name='lake-missing-secret'")
        .bind(w.admin.tenant_id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .unwrap();
    assert_eq!(heads, 0);

    // A SQL step cannot use a lake connection: it is as unusable to it as one that does not exist.
    assert!(matches!(
        w.try_publish_as(&w.admin, sql_step("sql-on-a-lake-connection", "lake-main"))
            .await,
        Err(AppError::NotFound)
    ));

    // A deployment without the lake refuses an Iceberg write at publication, and says why (`LAKE_DISABLED`, T035-036).
    let step = |name: &str| {
        document(
            name,
            "save",
            vec![json!({
                "id": "save", "type": "icebergWrite", "connectionRef": {"name": "lake-main"},
                "table": {"namespace": ["sales"], "name": "orders"}, "mode": "append",
                "records": {"inline": "input.rows"}
            })],
        )
    };
    match w
        .try_publish_as(&w.admin, step("iceberg-write-with-the-lake-off"))
        .await
    {
        Err(AppError::InvalidDefinition(error)) => assert!(
            matches!(*error, IrError::LakeDisabled { ref node } if node == "save"),
            "{error:?}"
        ),
        other => panic!("expected LAKE_DISABLED, got {other:?}"),
    }

    // With the lake on, the store looks at the connection: `lake-main` accepts `internal` at most, and a workflow with no
    // input schema gives every field the level of a field nobody classified.
    let published = publish_on_lake(
        w,
        &w.admin,
        append(
            "iceberg-write-with-the-lake-on",
            "lake-main",
            &["/id"],
            None,
        ),
    )
    .await;
    assert_eq!(
        egress_of(published),
        vec![field("/id", Level::Pii)],
        "unclassified is pii, and `lake-main` accepts internal"
    );

    // Plain `http` is for a development namespace, as a plaintext PostgreSQL connection is.
    let plain = iceberg(
        "http://catalog.local:8181",
        "db-credentials",
        EgressCeiling::Internal,
    );
    let environment: String =
        sqlx::query_scalar("SELECT environment FROM namespaces WHERE tenant_id=$1 AND id=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(ns.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .unwrap();
    let plaintext = catalog
        .put(
            &w.admin,
            ns,
            write("lake-plain", 0, ConnectionStatus::Active, plain),
            &ctx(),
        )
        .await;
    if environment == "dev" {
        assert!(
            plaintext.is_ok(),
            "plain http is for a development namespace: {plaintext:?}"
        );
    } else {
        assert!(
            matches!(plaintext, Err(AppError::StateConflict(_))),
            "plain http needs a development namespace: {plaintext:?}"
        );
    }

    // A `pii` ceiling needs the reveal grant of the namespace, as it does for a PostgreSQL connection.
    let pii = write(
        "lake-pii",
        0,
        ConnectionStatus::Active,
        iceberg(
            "https://catalog.example/api",
            "db-credentials",
            EgressCeiling::Pii,
        ),
    );
    let grants = RevealGrantService::new(w.store.clone(), SystemClock);
    let refused = catalog.put(&w.admin, ns, pii.clone(), &ctx()).await;
    assert!(
        matches!(
            refused,
            Err(AppError::Forbidden {
                permission: Permission::PayloadRevealPii
            })
        ),
        "{refused:?}"
    );
    let scope = grants.prepare(&w.admin, ns, &ctx()).await.unwrap();
    grants
        .grant(&scope, w.admin.subject().as_str(), &ctx())
        .await
        .unwrap();
    let granted = catalog.put(&w.admin, ns, pii, &ctx()).await.unwrap();
    assert_eq!(granted.config.egress_ceiling(), EgressCeiling::Pii);
    assert!(
        grants
            .revoke(&scope, w.admin.subject().as_str(), &ctx())
            .await
            .unwrap()
    );

    let published = publication(w, &catalog, ns).await;
    promotion(w, &catalog, secrets, published).await;

    // A revision can pause it, and the history keeps the first revision's configuration.
    let paused = catalog
        .put(
            &w.admin,
            ns,
            write(
                "lake-main",
                1,
                ConnectionStatus::Paused,
                iceberg(
                    "https://catalog.example/api",
                    "db-credentials",
                    EgressCeiling::Internal,
                ),
            ),
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(
        (paused.revision, paused.status),
        (2, ConnectionStatus::Paused)
    );
    let history = catalog
        .get(&w.admin, ns, &name, Some(1), &ctx())
        .await
        .unwrap();
    // a historical read returns the configuration of that revision and the connection's status now
    assert_eq!(
        (history.revision, history.status),
        (1, ConnectionStatus::Paused)
    );
    assert_eq!(history.config, first.config);

    // A paused connection is unusable to a step, as one that does not exist is.
    let on_a_paused_connection = publish_on_lake(
        w,
        &w.admin,
        append(
            "iceberg-write-on-a-paused-connection",
            "lake-main",
            &["/id"],
            Some(rows_schema()),
        ),
    )
    .await;
    assert!(
        matches!(on_a_paused_connection, Err(AppError::NotFound)),
        "{on_a_paused_connection:?}"
    );
}

// ------------------------------------------------------------------------------------------------ publication (T035-054)

/// The input of the workflows below: a list of records whose classification the schema states. The root is `public`,
/// and the unmarked `id` inherits it.
fn rows_schema() -> Value {
    json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {"rows": {"type": "array", "items": {"type": "object", "properties": {
            "id": {"type": "string"},
            "total": {"type": "integer", "x-sagat-classification": "internal"},
            "email": {"type": "string", "x-sagat-classification": "pii"},
            "token": {"type": "string", "x-sagat-classification": "secret"}
        }}}}
    })
}

/// A workflow with one append step reading `input.rows`, mapping the given fields.
fn append(name: &str, connection: &str, fields: &[&str], schema: Option<Value>) -> Value {
    let mapping: Vec<Value> = fields
        .iter()
        .enumerate()
        .map(|(n, field)| json!({"field": field, "column": format!("column_{n}")}))
        .collect();
    let mut document = document(
        name,
        "save",
        vec![json!({
            "id": "save", "type": "icebergWrite", "connectionRef": {"name": connection},
            "table": {"namespace": ["sales"], "name": "orders"}, "mode": "append",
            "records": {"inline": "input.rows"}, "mapping": mapping
        })],
    );
    if let Some(schema) = schema {
        document["inputSchema"] = schema;
    }
    document
}

/// Creates the draft as the administrator and publishes it as `who`, on a service that has the lake on.
async fn publish_on_lake(
    w: &World,
    who: &Principal,
    document: Value,
) -> Result<WorkflowId, AppError> {
    let lake_on = w.defs.clone().with_lake_enabled(true);
    let name = document["metadata"]["name"].as_str().unwrap().to_owned();
    let created = lake_on
        .create_workflow(
            &w.admin,
            "payments",
            &name,
            &serde_json::to_vec(&document).unwrap(),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("create the workflow");
    let id = created.workflow.id();
    lake_on
        .publish(
            who,
            id,
            created.workflow.draft_revision(),
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .map(|_| id)
}

/// The problems validation reports for the draft of `document`, on a service that has the lake on.
async fn problems_on_lake(
    w: &World,
    document: Value,
) -> Vec<(&'static str, Option<String>, String)> {
    let lake_on = w.defs.clone().with_lake_enabled(true);
    let name = document["metadata"]["name"].as_str().unwrap().to_owned();
    let created = lake_on
        .create_workflow(
            &w.admin,
            "payments",
            &name,
            &serde_json::to_vec(&document).unwrap(),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("create the workflow");
    lake_on
        .validate(&w.admin, created.workflow.id(), None, &ctx())
        .await
        .expect("validate")
        .problems
        .into_iter()
        .map(|problem| (problem.code, problem.node, problem.message))
        .collect()
}

fn field(path: &str, level: Level) -> EgressRefusal {
    EgressRefusal::Field {
        path: path.to_owned(),
        level,
    }
}

/// What a refused publication says about the step `save`, whichever way it is refused.
fn refusal_of<T: std::fmt::Debug>(result: Result<T, AppError>) -> LakeRefusal {
    match result {
        Err(AppError::InvalidDefinition(error)) => match *error {
            IrError::LakeWriteRefused { node, refusal } => {
                assert_eq!(node, "save");
                refusal
            }
            other => panic!("expected a refusal by the connection, got {other:?}"),
        },
        other => panic!("expected a refusal by the connection, got {other:?}"),
    }
}

/// The fields a publication was refused for on grounds of classification.
fn egress_of<T: std::fmt::Debug>(result: Result<T, AppError>) -> Vec<EgressRefusal> {
    match refusal_of(result) {
        LakeRefusal::Egress(fields) => fields,
        other => panic!("expected a refusal on grounds of classification, got {other:?}"),
    }
}

async fn publication(
    w: &World,
    catalog: &ConnectionService<PgRunStore, SystemClock>,
    ns: sagat_domain::identity::NamespaceId,
) -> WorkflowId {
    // A connection that allows overwriting only, and a SQL one: the two ways a connection can be the wrong one.
    catalog
        .put(
            &w.admin,
            ns,
            write(
                "lake-overwrite-only",
                0,
                ConnectionStatus::Active,
                iceberg_in(
                    "https://catalog.example/api",
                    "db-credentials",
                    EgressCeiling::Internal,
                    vec![WriteMode::Overwrite],
                ),
            ),
            &ctx(),
        )
        .await
        .unwrap();
    catalog
        .put(
            &w.admin,
            ns,
            write(
                "orders-sql",
                0,
                ConnectionStatus::Active,
                ConnectionConfig::postgres(
                    "db.example.com",
                    5432,
                    "orders",
                    TlsPolicy::VerifyFull { ca_pem: None },
                    SecretName::parse("db-credentials").unwrap(),
                    DatabaseLimits::default(),
                )
                .unwrap(),
            ),
            &ctx(),
        )
        .await
        .unwrap();

    // Fields the schema of the input marks `public` or `internal` leave through a connection that accepts `internal`.
    // Publication never contacts the catalog: `catalog.example` does not resolve, and this is published all the same.
    let published = publish_on_lake(
        w,
        &w.admin,
        append(
            "lake-within-ceiling",
            "lake-main",
            &["/id", "/total"],
            Some(rows_schema()),
        ),
    )
    .await
    .expect("fields within the ceiling are published");

    // A run of it is refused until the run admission of an Iceberg write exists (T035-055): nothing serves the step yet, and
    // a run that started would wait for a worker that is not there.
    let run = w
        .start_with(
            published,
            &json!({"rows": []}),
            "lake-run-start-0000001",
            None,
        )
        .await;
    assert!(matches!(run, Err(AppError::NotFound)), "{run:?}");

    // A `pii` field is refused by an `internal` ceiling, and named by its path and class.
    assert_eq!(
        egress_of(
            publish_on_lake(
                w,
                &w.admin,
                append(
                    "lake-pii-field",
                    "lake-main",
                    &["/id", "/email"],
                    Some(rows_schema())
                )
            )
            .await
        ),
        vec![field("/email", Level::Pii)]
    );

    // A `secret` field is refused by every ceiling, the `pii` one included.
    for connection in ["lake-main", "lake-pii"] {
        assert_eq!(
            egress_of(
                publish_on_lake(
                    w,
                    &w.admin,
                    append(
                        &format!("lake-secret-field-{connection}"),
                        connection,
                        &["/token"],
                        Some(rows_schema())
                    )
                )
                .await
            ),
            vec![field("/token", Level::Secret)],
            "{connection}"
        );
    }

    // A `pii` ceiling lets a `pii` field leave.
    publish_on_lake(
        w,
        &w.admin,
        append(
            "lake-pii-ceiling",
            "lake-pii",
            &["/id", "/email"],
            Some(rows_schema()),
        ),
    )
    .await
    .expect("a pii field leaves through a pii ceiling");

    // A source with no schema is unclassified, which counts as `pii`: refused by `internal`, accepted by `pii`.
    assert_eq!(
        egress_of(
            publish_on_lake(
                w,
                &w.admin,
                append("lake-unclassified", "lake-main", &["/id"], None)
            )
            .await
        ),
        vec![field("/id", Level::Pii)]
    );
    publish_on_lake(
        w,
        &w.admin,
        append("lake-unclassified-pii", "lake-pii", &["/id"], None),
    )
    .await
    .expect("unclassified fields leave through a pii ceiling");

    // A mode the connection does not allow is refused by name.
    assert_eq!(
        refusal_of(
            publish_on_lake(
                w,
                &w.admin,
                append(
                    "lake-wrong-mode",
                    "lake-overwrite-only",
                    &["/id"],
                    Some(rows_schema())
                )
            )
            .await
        ),
        LakeRefusal::ModeNotAllowed(WriteMode::Append)
    );

    // Validation says the same, as problems an author can act on.
    let problems = problems_on_lake(
        w,
        append(
            "lake-validate-egress",
            "lake-main",
            &["/id", "/email", "/token"],
            Some(rows_schema()),
        ),
    )
    .await;
    let refused: Vec<_> = problems
        .iter()
        .filter(|(code, _, _)| *code == "CLASSIFICATION_EGRESS_REFUSED")
        .collect();
    assert_eq!(refused.len(), 2, "{problems:?}");
    assert!(
        refused
            .iter()
            .all(|(_, node, _)| node.as_deref() == Some("save"))
    );
    assert!(refused[0].2.contains("/email") && refused[0].2.contains("pii"));
    assert!(refused[1].2.contains("/token") && refused[1].2.contains("secret"));
    let problems = problems_on_lake(
        w,
        append(
            "lake-validate-mode",
            "lake-overwrite-only",
            &["/id"],
            Some(rows_schema()),
        ),
    )
    .await;
    assert_eq!(
        problems.iter().map(|(code, ..)| *code).collect::<Vec<_>>(),
        ["MODE_NOT_ALLOWED"]
    );
    assert!(
        problems_on_lake(
            w,
            append(
                "lake-validate-clean",
                "lake-main",
                &["/id", "/total"],
                Some(rows_schema())
            )
        )
        .await
        .is_empty()
    );

    // A connection that does not exist, a SQL one, and a revision that does not exist are all unusable to the step, and
    // alike: not found.
    for (name, connection, revision) in [
        ("lake-ghost-connection", "lake-ghost", None),
        ("lake-sql-connection", "orders-sql", None),
        ("lake-ghost-revision", "lake-main", Some(99)),
    ] {
        let mut document = append(name, connection, &["/id"], Some(rows_schema()));
        if let Some(revision) = revision {
            document["nodes"][0]["connectionRef"]["revision"] = json!(revision);
        }
        let result = publish_on_lake(w, &w.admin, document).await;
        assert!(
            matches!(result, Err(AppError::NotFound)),
            "{name}: {result:?}"
        );
    }
    // The revision that does exist is usable.
    let mut pinned = append(
        "lake-pinned-revision",
        "lake-main",
        &["/id"],
        Some(rows_schema()),
    );
    pinned["nodes"][0]["connectionRef"]["revision"] = json!(1);
    publish_on_lake(w, &w.admin, pinned)
        .await
        .expect("revision 1 of lake-main is usable");

    // A step that pins a revision is judged by the current one too: tightening the modes applies to it, so pinning an older
    // revision cannot loosen what the connection says now.
    let tightened = |expected_revision: u32, modes: Vec<WriteMode>| {
        write(
            "lake-tightened",
            expected_revision,
            ConnectionStatus::Active,
            iceberg_in(
                "https://catalog.example/api",
                "db-credentials",
                EgressCeiling::Internal,
                modes,
            ),
        )
    };
    catalog
        .put(
            &w.admin,
            ns,
            tightened(0, vec![WriteMode::Append, WriteMode::Overwrite]),
            &ctx(),
        )
        .await
        .unwrap();
    let pinned_to_the_first = |name: &str| {
        let mut document = append(name, "lake-tightened", &["/id"], Some(rows_schema()));
        document["nodes"][0]["connectionRef"]["revision"] = json!(1);
        document
    };
    publish_on_lake(w, &w.admin, pinned_to_the_first("lake-pinned-before"))
        .await
        .expect("the first revision is the current one and allows append");
    catalog
        .put(
            &w.admin,
            ns,
            tightened(1, vec![WriteMode::Overwrite]),
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(
        refusal_of(publish_on_lake(w, &w.admin, pinned_to_the_first("lake-pinned-after")).await),
        LakeRefusal::ModeNotAllowed(WriteMode::Append),
        "the first revision allowed append, the current one does not"
    );

    // A developer holds `database:use` and may use the secret, but not `database:write`, which every Iceberg write needs:
    // the connection is as good as missing to them, and the refusal is audited.
    let developer = w.member("lake-publisher", Role::Developer).await;
    w.secrets
        .grant(
            &w.admin,
            ns,
            "db-credentials",
            developer.subject().as_str(),
            &ctx(),
        )
        .await
        .unwrap();
    let denials = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action=$2 AND actor=$3",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(AuditAction::DatabaseDenied.as_str())
        .bind(developer.subject().as_str())
        .fetch_one(&w.db.pool)
        .await
        .unwrap()
    };
    let before = denials().await;
    let refused = publish_on_lake(
        w,
        &developer,
        append(
            "lake-no-database-write",
            "lake-main",
            &["/id"],
            Some(rows_schema()),
        ),
    )
    .await;
    assert!(matches!(refused, Err(AppError::NotFound)), "{refused:?}");
    // The audit of a denial is written off the request path, so it is waited for, boundedly.
    crate::support::eventually("the audit of the refused publication", || async {
        (denials().await == before + 1).then_some(())
    })
    .await;
    published
}

/// The promotion of a version that holds an Iceberg write to another namespace. It goes through the publishing service,
/// which has its own copy of the switch: a service with the lake off refuses it, one with the lake on looks at the target
/// namespace's own connection and secret, as a publication there would.
async fn promotion(
    w: &World,
    catalog: &ConnectionService<PgRunStore, SystemClock>,
    secrets: &ManagedSecretService<PgRunStore, SystemClock>,
    source: WorkflowId,
) {
    let target = w
        .add_namespace_in("lake-staging", Environment::Staging)
        .await;
    let reason = "lake promotion validation";

    match w
        .publishing
        .promote(
            &w.admin,
            source,
            1,
            target.id(),
            reason,
            false,
            "lake-promotion-off",
            &ctx(),
        )
        .await
    {
        Err(AppError::InvalidDefinition(error)) => assert!(
            matches!(*error, IrError::LakeDisabled { ref node } if node == "save"),
            "{error:?}"
        ),
        other => panic!("expected LAKE_DISABLED from a service with the lake off, got {other:?}"),
    }

    // With the lake on, the target namespace has to have the connection and the secret of its own: a source connection
    // grants no authority in a different namespace.
    let lake_on = w.publishing.clone().with_lake_enabled(true);
    let missing = lake_on
        .promote(
            &w.admin,
            source,
            1,
            target.id(),
            reason,
            false,
            "lake-promotion-missing",
            &ctx(),
        )
        .await;
    assert!(matches!(missing, Err(AppError::NotFound)), "{missing:?}");

    secrets
        .put(
            &w.admin,
            target.id(),
            ManagedSecretWrite {
                name: "db-credentials",
                value: SecretValue::new(b"staging-canary".to_vec()).unwrap(),
                expected_version: 0,
                request_id: Uuid::new_v4(),
            },
            &ctx(),
        )
        .await
        .unwrap();
    catalog
        .put(
            &w.admin,
            target.id(),
            write(
                "lake-main",
                0,
                ConnectionStatus::Active,
                iceberg(
                    "https://catalog.example/api",
                    "db-credentials",
                    EgressCeiling::Internal,
                ),
            ),
            &ctx(),
        )
        .await
        .unwrap();
    let promoted = lake_on
        .promote(
            &w.admin,
            source,
            1,
            target.id(),
            reason,
            false,
            "lake-promotion-on",
            &ctx(),
        )
        .await;
    assert!(
        matches!(promoted, Ok(Promotion::Published(_))),
        "the promotion is published once the target namespace has what the step needs: {promoted:?}"
    );
}

/// A lake runner registers for the write activity alone, and only under a binding that covers it (feature 035,
/// D035-135): through the real store, the membership role, the binding patterns and the negotiated minor. No lake step
/// can start yet (the admission guard is closed), so this is the whole of what registering gives it.
pub async fn registration(w: &World) {
    let ns = w.namespace.id();
    let lake = w.member("lake-machine", Role::DatabaseRunner).await;
    let caps = |activities: &[&str], version: &str| Capabilities {
        activities: activities.iter().map(|name| (*name).to_owned()).collect(),
        languages: vec![],
        pools: vec![],
        max_inflight: 4,
        protocol_version: version.into(),
    };
    let write = "sagat.database.iceberg.write";
    let sql = "sagat.database.postgresql";
    let refused = |result: Result<_, AppError>| matches!(result, Err(AppError::NotBound));

    // Without a binding nothing registers; a binding for the SQL activity does not cover the lake one.
    assert!(refused(
        w.workers
            .register(&lake, "payments", "lake-unbound", &caps(&[write], "1.6"))
            .await
    ));
    w.bindings
        .bind(
            &w.admin,
            ns,
            lake.subject().as_str(),
            &[sql.to_owned()],
            &ctx(),
        )
        .await
        .unwrap();
    assert!(refused(
        w.workers
            .register(
                &lake,
                "payments",
                "lake-sql-binding",
                &caps(&[write], "1.6")
            )
            .await
    ));

    // A binding that names the write activity is the authority for it, and for nothing else.
    w.bindings
        .bind(
            &w.admin,
            ns,
            lake.subject().as_str(),
            &[write.to_owned()],
            &ctx(),
        )
        .await
        .unwrap();
    assert!(refused(
        w.workers
            .register(&lake, "payments", "lake-too-old", &caps(&[write], "1.5"))
            .await
    ));
    assert!(refused(
        w.workers
            .register(&lake, "payments", "lake-with-sql", &caps(&[sql], "1.6"))
            .await
    ));
    let registered = w
        .workers
        .register(&lake, "payments", "lake-machine", &caps(&[write], "1.6"))
        .await
        .unwrap();
    assert_eq!(registered.accepted_protocol_version, "1.6");

    // Everything bound: the profile rule still holds, because it does not come from the binding.
    w.bind_everything(&lake).await;
    for (name, activities) in [
        ("lake-mixed", vec![write, sql]),
        ("lake-ordinary", vec![write, "ordinary"]),
        ("lake-revert", vec!["sagat.database.iceberg.revert"]),
        ("lake-probe", vec!["sagat.database.lake.probe"]),
    ] {
        assert!(
            refused(
                w.workers
                    .register(&lake, "payments", name, &caps(&activities, "1.6"))
                    .await
            ),
            "{name}"
        );
    }

    // The reserved prefix stays closed to every other role, whatever it is bound to.
    for principal in [&w.worker, &w.runner_identity] {
        w.bind_everything(principal).await;
        assert!(refused(
            w.workers
                .register(principal, "payments", "forged-lake", &caps(&[write], "1.6"))
                .await
        ));
    }
}

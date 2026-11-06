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
    reveal_grants::RevealGrantService,
};
use sagat_domain::{
    audit::AuditAction,
    classification::EgressCeiling,
    database::{
        AccessDelegation, ConnectionConfig, ConnectionStatus, IcebergCatalog, IcebergConfig,
        LakeLimits, LakeStorage, RestAuth, RestCatalog, S3Storage, StorageCredentials, StorageKind,
        WriteMode,
    },
    identity::{Permission, Slug},
    secrets::SecretName,
};
use serde_json::{Value, json};
use uuid::Uuid;

fn iceberg(uri: &str, catalog_secret: &str, ceiling: EgressCeiling) -> ConnectionConfig {
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
        allowed_modes: vec![WriteMode::Append, WriteMode::Overwrite],
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

pub async fn run(w: &World) {
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

    // With the lake on, the store still fails closed until the admission of an Iceberg write exists (T035-054): the
    // codec parses the node, and the store refuses it as it refuses a connection the author may not use, rather than
    // publish a step that nobody checked for access, modes or egress.
    let lake_on = w.defs.clone().with_lake_enabled(true);
    let doc = step("iceberg-write-with-the-lake-on");
    let bytes = serde_json::to_vec(&doc).expect("json");
    let created = lake_on
        .create_workflow(
            &w.admin,
            "payments",
            "iceberg-write-with-the-lake-on",
            &bytes,
            &format!("create-{}", uuid::Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("create the workflow");
    let published = lake_on
        .publish(
            &w.admin,
            created.workflow.id(),
            created.workflow.draft_revision(),
            None,
            &format!("publish-{}", uuid::Uuid::now_v7()),
            &ctx(),
        )
        .await;
    assert!(
        matches!(published, Err(AppError::NotFound)),
        "{published:?}"
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
}

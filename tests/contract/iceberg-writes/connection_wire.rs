//! The Iceberg connection the server answers with is the connection the runner reads (feature 035, T035-044, D035-115, D035-132).
//!
//! The server writes the message (`sagat-api-grpc`), the runner reads it back into the documents the host reads
//! (`sagat-database-runner`); neither can link the other, so this suite, which depends on both, holds them to **the stored
//! documents of the wire fixture** (`fixtures/requests/iceberg-write-append.json`, which the Python host parses too). A connection
//! the server answers as it is stored and the runner cannot open is refused by the runner **by name**, never misread.

use std::collections::BTreeMap;

use sagat_api_grpc::lake_connection::{LakeSecrets, iceberg_connection};
use sagat_database_runner::lake::connection::{ConvertError, Secret, convert};
use sagat_domain::classification::EgressCeiling;
use sagat_domain::database::{
    AccessDelegation, AdlsStorage, GcsStorage, GlueCatalog, HiveCatalog, HiveTls, IcebergCatalog,
    IcebergConfig, JdbcCatalog, JdbcEngine, LakeLimits, LakeStorage, RestAuth, RestCatalog,
    S3Storage, StorageCredentials, StorageKind, TlsPolicy, WriteMode,
};
use serde_json::{Value, json};

use super::{fixtures, read_json};

fn wire() -> Value {
    read_json(&fixtures().join("requests/iceberg-write-append.json"))
}

fn rest(
    prefix: Option<&str>,
    auth: RestAuth,
    access_delegation: AccessDelegation,
) -> IcebergCatalog {
    IcebergCatalog::Rest(RestCatalog {
        uri: "https://catalog.example/api".to_owned(),
        warehouse: "s3://lake/warehouse/".to_owned(),
        prefix: prefix.map(str::to_owned),
        auth,
        access_delegation,
        remote_signing: false,
    })
}

fn s3(endpoint: Option<&str>, path_style: bool, region: &str) -> StorageKind {
    StorageKind::S3(S3Storage {
        region: region.to_owned(),
        endpoint: endpoint.map(str::to_owned),
        path_style,
        role_arn: None,
        external_id: None,
    })
}

fn config(
    catalog: IcebergCatalog,
    kind: StorageKind,
    credentials: StorageCredentials,
) -> IcebergConfig {
    IcebergConfig {
        catalog,
        storage: LakeStorage { kind, credentials },
        warehouse_locations: vec!["s3://lake/warehouse/".to_owned()],
        allowed_modes: vec![WriteMode::Append],
        catalog_secret: None,
        storage_secret: None,
        egress_ceiling: EgressCeiling::Internal,
        limits: LakeLimits::default(),
    }
}

fn plain(secret: &Secret) -> BTreeMap<String, String> {
    secret
        .iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

fn document(value: &Value) -> BTreeMap<String, String> {
    serde_json::from_value(value.clone()).expect("a document of text to text")
}

#[test]
fn the_connection_the_server_answers_is_read_by_the_runner_as_the_stored_documents_of_the_fixture()
{
    let wire = wire();
    // the connection of the fixture, as the platform stores it
    let config = config(
        rest(
            Some("tenant-a"),
            RestAuth::OAuth2 {
                token_uri: "https://auth.example/token".to_owned(),
                scope: Some("catalog".to_owned()),
            },
            AccessDelegation::None,
        ),
        s3(None, false, "eu-west-1"),
        StorageCredentials::Static,
    );
    let catalog_secret =
        serde_json::to_vec(&wire["connection"]["credentials"]["catalog"]).expect("JSON");
    let storage_secret =
        serde_json::to_vec(&wire["connection"]["credentials"]["storage"]).expect("JSON");

    let mut message = iceberg_connection(
        &config,
        &LakeSecrets {
            catalog: Some(&catalog_secret),
            storage: Some(&storage_secret),
        },
    );
    let converted = convert(&mut message).expect("the runner reads what the server answers");

    assert_eq!(converted.catalog, wire["connection"]["catalog"]);
    assert_eq!(converted.storage, wire["connection"]["storage"]);
    assert_eq!(converted.warehouse_locations, ["s3://lake/warehouse/"]);
    assert_eq!(
        plain(&converted.catalog_secret),
        document(&wire["connection"]["credentials"]["catalog"])
    );
    assert_eq!(
        plain(&converted.storage_secret),
        document(&wire["connection"]["credentials"]["storage"])
    );
    assert_eq!(converted.allowed_modes, ["append"]);
    assert_eq!(converted.egress_ceiling, EgressCeiling::Internal);
    assert_eq!(converted.limits.max_records, None);
}

#[test]
fn a_catalog_that_delegates_and_vended_storage_are_read_as_the_documents_the_platform_stores() {
    let config = config(
        rest(None, RestAuth::Bearer, AccessDelegation::VendedCredentials),
        s3(None, false, "eu-west-1"),
        StorageCredentials::Vended,
    );
    let mut message = iceberg_connection(
        &config,
        &LakeSecrets {
            catalog: Some(br#"{"token":"t"}"#),
            storage: None,
        },
    );
    let converted = convert(&mut message).expect("read");
    assert_eq!(
        converted.catalog,
        json!({"type": "rest", "rest": {
            "uri": "https://catalog.example/api", "warehouse": "s3://lake/warehouse/",
            "auth": {"type": "bearer"}, "accessDelegation": "vended-credentials", "remoteSigning": false
        }})
    );
    assert_eq!(
        converted.storage,
        json!({"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": false}, "credentials": "vended"})
    );
    assert!(converted.storage_secret.is_empty());
    assert_eq!(
        plain(&converted.catalog_secret),
        document(&json!({"token": "t"}))
    );
}

#[test]
fn a_compatible_store_keeps_its_endpoint_and_its_style_through_both_ends() {
    let config = config(
        rest(
            None,
            RestAuth::OAuth2 {
                token_uri: "https://auth.example/token".to_owned(),
                scope: None,
            },
            AccessDelegation::None,
        ),
        s3(Some("https://minio.internal:9000"), true, "us-east-1"),
        StorageCredentials::Static,
    );
    let mut message = iceberg_connection(
        &config,
        &LakeSecrets {
            catalog: Some(br#"{"clientId":"c","clientSecret":"s"}"#),
            storage: Some(br#"{"accessKeyId":"a","secretAccessKey":"k"}"#),
        },
    );
    let converted = convert(&mut message).expect("read");
    assert_eq!(
        converted.storage["s3"],
        json!({"region": "us-east-1", "endpoint": "https://minio.internal:9000", "pathStyle": true})
    );
}

#[test]
fn what_the_platform_stores_and_the_host_cannot_open_is_answered_and_refused_by_name_at_the_runner()
{
    let s3_static = || s3(None, false, "eu-west-1");
    let unsupported = [
        config(
            IcebergCatalog::Glue(GlueCatalog {
                region: "eu-west-1".to_owned(),
                catalog_id: "1".to_owned(),
                role_arn: None,
                external_id: None,
            }),
            s3_static(),
            StorageCredentials::Static,
        ),
        config(
            IcebergCatalog::Hive(HiveCatalog {
                uris: vec!["thrift://hms:9083".to_owned()],
                tls: HiveTls::Required,
            }),
            s3_static(),
            StorageCredentials::Static,
        ),
        config(
            IcebergCatalog::Jdbc(JdbcCatalog {
                host: "db.internal".to_owned(),
                port: 5432,
                database: "iceberg".to_owned(),
                tls: TlsPolicy::DevelopmentPlaintext,
                engine: JdbcEngine::Postgresql,
            }),
            s3_static(),
            StorageCredentials::Static,
        ),
        config(
            rest(None, RestAuth::Bearer, AccessDelegation::None),
            StorageKind::Gcs(GcsStorage {
                project: Some("p".to_owned()),
                endpoint: None,
            }),
            StorageCredentials::Static,
        ),
        config(
            rest(None, RestAuth::Bearer, AccessDelegation::None),
            StorageKind::Adls(AdlsStorage {
                account: "lake".to_owned(),
                endpoint_suffix: None,
            }),
            StorageCredentials::Static,
        ),
    ];
    for config in &unsupported {
        let mut message = iceberg_connection(
            config,
            &LakeSecrets {
                catalog: Some(br#"{"token":"t"}"#),
                storage: Some(br#"{"accessKeyId":"a","secretAccessKey":"k"}"#),
            },
        );
        assert_eq!(
            convert(&mut message).map(|_| ()),
            Err(ConvertError::Unsupported),
            "{:?}",
            config.catalog
        );
    }
}
/// A `ResolvedDatabase` for a lake step: the connection as stored, the two secrets as the server resolved them.
fn resolved_lake(
    iceberg: IcebergConfig,
    catalog: Option<&[u8]>,
    storage: Option<&[u8]>,
) -> sagat_application::database::ResolvedDatabase {
    use sagat_application::{
        database::{ConnectionRecord, ResolvedDatabase, ResolvedGrant, ResolvedLake},
        worker::ResolvedSecret,
    };
    use sagat_domain::{
        database::{ConnectionConfig, ConnectionId, ConnectionStatus},
        identity::{NamespaceId, Slug, TenantId},
        secrets::{SecretName, SecretValue},
    };
    let now = time::OffsetDateTime::now_utc();
    let secret = |name: &str, bytes: &[u8]| ResolvedSecret {
        name: SecretName::parse(name).expect("a name"),
        version: 1,
        value: SecretValue::new(bytes.to_vec()).expect("a value"),
    };
    ResolvedDatabase {
        connection: ConnectionRecord {
            id: ConnectionId::new(uuid::Uuid::from_u128(7)),
            tenant_id: TenantId::new(uuid::Uuid::from_u128(1)),
            namespace_id: NamespaceId::new(uuid::Uuid::from_u128(2)),
            name: Slug::parse("lake-main").expect("a slug"),
            revision: 3,
            status: ConnectionStatus::Active,
            config: ConnectionConfig::iceberg(iceberg).expect("a valid connection"),
            created_at: now,
            updated_at: now,
        },
        expires_at: now + time::Duration::minutes(5),
        grant: ResolvedGrant::Lake(ResolvedLake {
            catalog: catalog.map(|bytes| secret("lake-catalog", bytes)),
            storage: storage.map(|bytes| secret("lake-storage", bytes)),
        }),
    }
}

/// The configuration with the secrets it names, as a valid connection has them.
fn with_secrets(mut iceberg: IcebergConfig, catalog: bool, storage: bool) -> IcebergConfig {
    use sagat_domain::secrets::SecretName;
    iceberg.catalog_secret = catalog.then(|| SecretName::parse("lake-catalog").expect("name"));
    iceberg.storage_secret = storage.then(|| SecretName::parse("lake-storage").expect("name"));
    iceberg
}

#[test]
fn the_answer_to_a_lake_step_is_the_connection_the_runner_reads_with_its_secrets_and_nothing_of_sql()
 {
    let catalog_secret = br#"{"clientId":"c","clientSecret":"s"}"#;
    let storage_secret = br#"{"accessKeyId":"a","secretAccessKey":"k"}"#;
    let oauth = RestAuth::OAuth2 {
        token_uri: "https://auth.example/token".to_owned(),
        scope: None,
    };
    let stored = with_secrets(
        config(
            rest(None, oauth, AccessDelegation::None),
            s3(None, false, "eu-west-1"),
            StorageCredentials::Static,
        ),
        true,
        true,
    );
    let mut answer = sagat_api_grpc::database::response(resolved_lake(
        stored,
        Some(catalog_secret),
        Some(storage_secret),
    ))
    .expect("a lake step is answered");
    assert_eq!(answer.connection_name, "lake-main");
    assert_eq!(answer.connection_revision, 3);
    assert_eq!(answer.tenant_id, "00000000-0000-0000-0000-000000000001");
    assert_eq!(answer.namespace_id, "00000000-0000-0000-0000-000000000002");
    assert_eq!(answer.connection_id, "00000000-0000-0000-0000-000000000007");
    assert!(
        answer.lease_expires_at_epoch_ms > 0,
        "the lease the secrets are bound to"
    );
    // nothing of SQL travels with it
    assert!(answer.postgres.is_none() && answer.credential.is_none());
    assert!(answer.statement.is_empty() && answer.parameters.is_empty() && answer.limits.is_none());
    assert!(answer.object_storage.is_none() && answer.clickhouse.is_none());
    let mut message = answer.iceberg.take().expect("the Iceberg connection");
    let converted = convert(&mut message).expect("the runner reads what the server answers");
    assert_eq!(
        plain(&converted.catalog_secret),
        document(&json!({"clientId": "c", "clientSecret": "s"}))
    );
    assert_eq!(
        plain(&converted.storage_secret),
        document(&json!({"accessKeyId": "a", "secretAccessKey": "k"}))
    );

    // a catalog that delegates the storage credentials: the storage secret is not there, and is not answered
    let vended = with_secrets(
        config(
            rest(None, RestAuth::Bearer, AccessDelegation::VendedCredentials),
            s3(None, false, "eu-west-1"),
            StorageCredentials::Vended,
        ),
        true,
        false,
    );
    let mut answer =
        sagat_api_grpc::database::response(resolved_lake(vended, Some(br#"{"token":"t"}"#), None))
            .expect("answered");
    let mut message = answer.iceberg.take().expect("the Iceberg connection");
    assert_eq!(
        message.credentials.keys().collect::<Vec<_>>(),
        ["catalog"],
        "only the secrets that exist travel"
    );
    let converted = convert(&mut message).expect("read");
    assert!(converted.storage_secret.is_empty());
}

#[test]
fn a_lake_grant_is_not_answered_for_a_connection_that_is_not_a_lake() {
    use sagat_domain::database::{ConnectionConfig, DatabaseLimits};
    let mut resolved = resolved_lake(
        with_secrets(
            config(
                rest(None, RestAuth::Bearer, AccessDelegation::None),
                s3(None, false, "eu-west-1"),
                StorageCredentials::Static,
            ),
            true,
            true,
        ),
        Some(b"{}"),
        Some(b"{}"),
    );
    resolved.connection.config = ConnectionConfig::postgres(
        "db.example",
        5432,
        "app",
        TlsPolicy::DevelopmentPlaintext,
        sagat_domain::secrets::SecretName::parse("db-credentials").expect("name"),
        DatabaseLimits::default(),
    )
    .expect("a PostgreSQL connection");
    assert!(
        sagat_api_grpc::database::response(resolved).is_err(),
        "a lake grant over a PostgreSQL connection is refused, not answered"
    );
}
#[test]
fn a_lake_grant_that_does_not_match_the_secrets_the_connection_names_is_not_answered() {
    let statics = || {
        config(
            rest(None, RestAuth::Bearer, AccessDelegation::None),
            s3(None, false, "eu-west-1"),
            StorageCredentials::Static,
        )
    };
    let vended = || {
        config(
            rest(None, RestAuth::Bearer, AccessDelegation::VendedCredentials),
            s3(None, false, "eu-west-1"),
            StorageCredentials::Vended,
        )
    };
    let answered = |iceberg, catalog: Option<&[u8]>, storage: Option<&[u8]>| {
        sagat_api_grpc::database::response(resolved_lake(iceberg, catalog, storage)).is_ok()
    };
    // the shapes that match are answered
    assert!(answered(
        with_secrets(statics(), true, true),
        Some(b"{}"),
        Some(b"{}")
    ));
    assert!(answered(
        with_secrets(vended(), true, false),
        Some(b"{}"),
        None
    ));
    // a secret the connection does not name is never sent, and one it names is never missing
    assert!(!answered(
        with_secrets(vended(), true, false),
        Some(b"{}"),
        Some(b"{}")
    ));
    assert!(!answered(
        with_secrets(statics(), true, true),
        Some(b"{}"),
        None
    ));
    assert!(!answered(
        with_secrets(statics(), true, true),
        None,
        Some(b"{}")
    ));
}

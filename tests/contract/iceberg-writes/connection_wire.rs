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

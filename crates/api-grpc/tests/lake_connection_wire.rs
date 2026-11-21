//! The wire form of an Iceberg connection (feature 035, T035-044, D035-132): what `ResolveDatabase` answers a lake runner with,
//! field by field. The runner's end (`apps/database-runner/src/lake/connection.rs`) reads it back; the test that holds the two to
//! one stored document depends on both and lives in `tests/contract/iceberg-writes`.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_api_grpc::lake_connection::{LakeSecrets, iceberg_connection};
use sagat_api_grpc::pb::{self, iceberg_connection::Catalog, lake_storage::Kind};
use sagat_domain::classification::EgressCeiling;
use sagat_domain::database::{
    AccessDelegation, AdlsStorage, GcsStorage, GlueCatalog, HiveCatalog, HiveTls, IcebergCatalog,
    IcebergConfig, JdbcCatalog, JdbcEngine, LakeLimits, LakeStorage, RestAuth, RestCatalog,
    S3Storage, StorageCredentials, StorageKind, TlsPolicy, WriteMode,
};

/// The oneofs of the message have no `Debug` (a connection message holds credentials, and the crate keeps them undebuggable), so
/// they are compared with `PartialEq` alone.
macro_rules! same {
    ($left:expr, $right:expr $(,)?) => {
        assert!(
            $left == $right,
            "{} is not {}",
            stringify!($left),
            stringify!($right)
        )
    };
}

const NO_SECRETS: LakeSecrets<'static> = LakeSecrets {
    catalog: None,
    storage: None,
};

fn rest(
    auth: RestAuth,
    access_delegation: AccessDelegation,
    remote_signing: bool,
) -> IcebergCatalog {
    IcebergCatalog::Rest(RestCatalog {
        uri: "https://catalog.example/api".to_owned(),
        warehouse: "s3://lake/warehouse/".to_owned(),
        prefix: Some("tenant-a".to_owned()),
        auth,
        access_delegation,
        remote_signing,
    })
}

fn oauth2() -> RestAuth {
    RestAuth::OAuth2 {
        token_uri: "https://auth.example/token".to_owned(),
        scope: Some("catalog".to_owned()),
    }
}

fn s3(endpoint: Option<&str>, path_style: bool) -> StorageKind {
    StorageKind::S3(S3Storage {
        region: "eu-west-1".to_owned(),
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

fn base() -> IcebergConfig {
    config(
        rest(oauth2(), AccessDelegation::None, false),
        s3(None, false),
        StorageCredentials::Static,
    )
}

fn catalog_of(message: &pb::IcebergConnection) -> &Catalog {
    message.catalog.as_ref().expect("a catalog")
}

fn storage_of(message: &pb::IcebergConnection) -> &pb::LakeStorage {
    message.storage.as_ref().expect("a storage")
}

#[test]
fn a_rest_catalog_with_oauth2_and_s3_is_the_message_the_runner_reads() {
    let message = iceberg_connection(&base(), &NO_SECRETS);
    same!(
        catalog_of(&message),
        &Catalog::Rest(pb::IcebergRestCatalog {
            uri: "https://catalog.example/api".into(),
            warehouse: "s3://lake/warehouse/".into(),
            prefix: "tenant-a".into(),
            auth: pb::LakeRestAuth::Oauth2 as i32,
            token_uri: "https://auth.example/token".into(),
            scope: "catalog".into(),
            ..pb::IcebergRestCatalog::default()
        })
    );
    assert_eq!(
        storage_of(&message),
        &pb::LakeStorage {
            kind: Some(Kind::S3(pb::LakeS3Storage {
                region: "eu-west-1".into(),
                ..pb::LakeS3Storage::default()
            })),
            vended_credentials: false,
        }
    );
    assert_eq!(message.warehouse_locations, ["s3://lake/warehouse/"]);
    assert_eq!(message.allowed_modes, ["append"]);
    assert_eq!(message.egress_ceiling, pb::EgressCeiling::Internal as i32);
    assert_eq!(message.limits, None);
    assert!(message.credentials.is_empty());
}

#[test]
fn a_bearer_catalog_that_delegates_and_vended_storage_say_so_in_both_places() {
    let config = config(
        rest(RestAuth::Bearer, AccessDelegation::VendedCredentials, false),
        s3(None, false),
        StorageCredentials::Vended,
    );
    let message = iceberg_connection(&config, &NO_SECRETS);
    let Catalog::Rest(rest) = catalog_of(&message) else {
        panic!("not REST");
    };
    assert_eq!(rest.auth, pb::LakeRestAuth::Bearer as i32);
    assert!(rest.vended_credentials);
    assert!(rest.token_uri.is_empty() && rest.scope.is_empty());
    assert!(storage_of(&message).vended_credentials);
}

#[test]
fn sigv4_carries_its_region_and_signing_name_and_remote_signing_is_its_own_flag() {
    let config = config(
        rest(
            RestAuth::SigV4 {
                region: "us-east-1".to_owned(),
                signing_name: "glue".to_owned(),
            },
            AccessDelegation::None,
            true,
        ),
        s3(None, false),
        StorageCredentials::Static,
    );
    let message = iceberg_connection(&config, &NO_SECRETS);
    let Catalog::Rest(rest) = catalog_of(&message) else {
        panic!("not REST");
    };
    assert_eq!(rest.auth, pb::LakeRestAuth::Sigv4 as i32);
    assert_eq!(
        (rest.region.as_str(), rest.signing_name.as_str()),
        ("us-east-1", "glue")
    );
    assert!(rest.remote_signing);
    assert!(!rest.vended_credentials);
}

#[test]
fn an_absent_optional_setting_is_the_empty_string_the_runner_reads_as_absent() {
    let mut config = base();
    if let IcebergCatalog::Rest(rest) = &mut config.catalog {
        rest.prefix = None;
        rest.auth = RestAuth::OAuth2 {
            token_uri: "https://auth.example/token".to_owned(),
            scope: None,
        };
    }
    let message = iceberg_connection(&config, &NO_SECRETS);
    let Catalog::Rest(rest) = catalog_of(&message) else {
        panic!("not REST");
    };
    assert!(rest.prefix.is_empty());
    assert!(rest.scope.is_empty());
}

#[test]
fn the_other_catalogs_are_carried_whole() {
    let glue = config(
        IcebergCatalog::Glue(GlueCatalog {
            region: "eu-west-1".to_owned(),
            catalog_id: "123456789012".to_owned(),
            role_arn: Some("arn:aws:iam::123456789012:role/lake".to_owned()),
            external_id: Some("ext".to_owned()),
        }),
        s3(None, false),
        StorageCredentials::Static,
    );
    same!(
        catalog_of(&iceberg_connection(&glue, &NO_SECRETS)),
        &Catalog::Glue(pb::IcebergGlueCatalog {
            region: "eu-west-1".into(),
            catalog_id: "123456789012".into(),
            role_arn: "arn:aws:iam::123456789012:role/lake".into(),
            external_id: "ext".into(),
        })
    );

    for (tls, required) in [(HiveTls::Required, true), (HiveTls::None, false)] {
        let hive = config(
            IcebergCatalog::Hive(HiveCatalog {
                uris: vec![
                    "thrift://hms-1:9083".to_owned(),
                    "thrift://hms-2:9083".to_owned(),
                ],
                tls,
            }),
            s3(None, false),
            StorageCredentials::Static,
        );
        same!(
            catalog_of(&iceberg_connection(&hive, &NO_SECRETS)),
            &Catalog::Hive(pb::IcebergHiveCatalog {
                uris: vec!["thrift://hms-1:9083".into(), "thrift://hms-2:9083".into()],
                tls_required: required,
            })
        );
    }

    for (tls, mode, ca) in [
        (
            TlsPolicy::VerifyFull {
                ca_pem: Some("-----BEGIN CERTIFICATE-----".to_owned()),
            },
            pb::DatabaseTlsMode::DatabaseTlsVerifyFull,
            "-----BEGIN CERTIFICATE-----",
        ),
        (
            TlsPolicy::DevelopmentPlaintext,
            pb::DatabaseTlsMode::DatabaseTlsDevelopmentPlaintext,
            "",
        ),
    ] {
        let jdbc = config(
            IcebergCatalog::Jdbc(JdbcCatalog {
                host: "catalog-db.internal".to_owned(),
                port: 5432,
                database: "iceberg".to_owned(),
                tls,
                engine: JdbcEngine::Postgresql,
            }),
            s3(None, false),
            StorageCredentials::Static,
        );
        same!(
            catalog_of(&iceberg_connection(&jdbc, &NO_SECRETS)),
            &Catalog::Jdbc(pb::IcebergJdbcCatalog {
                host: "catalog-db.internal".into(),
                port: 5432,
                database: "iceberg".into(),
                tls: mode as i32,
                ca_pem: ca.into(),
                engine: "postgresql".into(),
            })
        );
    }
}

#[test]
fn every_storage_kind_is_carried_whole() {
    let compatible = StorageKind::S3(S3Storage {
        region: "us-east-1".to_owned(),
        endpoint: Some("https://minio.internal:9000".to_owned()),
        path_style: true,
        role_arn: Some("arn:aws:iam::1:role/r".to_owned()),
        external_id: Some("ext".to_owned()),
    });
    let gcs = StorageKind::Gcs(GcsStorage {
        project: Some("project-1".to_owned()),
        endpoint: Some("http://localhost:4443".to_owned()),
    });
    let adls = StorageKind::Adls(AdlsStorage {
        account: "lakeaccount".to_owned(),
        endpoint_suffix: Some("core.windows.net".to_owned()),
    });
    let expected = [
        (
            compatible,
            Kind::S3(pb::LakeS3Storage {
                region: "us-east-1".into(),
                endpoint: "https://minio.internal:9000".into(),
                path_style: true,
                role_arn: "arn:aws:iam::1:role/r".into(),
                external_id: "ext".into(),
            }),
        ),
        (
            gcs,
            Kind::Gcs(pb::LakeGcsStorage {
                project: "project-1".into(),
                endpoint: "http://localhost:4443".into(),
            }),
        ),
        (
            adls,
            Kind::Adls(pb::LakeAdlsStorage {
                account: "lakeaccount".into(),
                endpoint_suffix: "core.windows.net".into(),
            }),
        ),
    ];
    for (kind, wire) in expected {
        let message = iceberg_connection(
            &config(
                rest(oauth2(), AccessDelegation::None, false),
                kind,
                StorageCredentials::Static,
            ),
            &NO_SECRETS,
        );
        assert_eq!(storage_of(&message).kind, Some(wire));
    }
}

#[test]
fn the_modes_keep_their_order_and_the_ceiling_is_always_set() {
    let mut config = base();
    config.allowed_modes = vec![WriteMode::Upsert, WriteMode::Append, WriteMode::Overwrite];
    for (ceiling, wire) in [
        (EgressCeiling::Public, pb::EgressCeiling::Public),
        (EgressCeiling::Internal, pb::EgressCeiling::Internal),
        (EgressCeiling::Pii, pb::EgressCeiling::Pii),
    ] {
        config.egress_ceiling = ceiling;
        let message = iceberg_connection(&config, &NO_SECRETS);
        assert_eq!(message.allowed_modes, ["upsert", "append", "overwrite"]);
        assert_eq!(message.egress_ceiling, wire as i32);
        assert_ne!(
            message.egress_ceiling,
            pb::EgressCeiling::Unspecified as i32
        );
    }
}

#[test]
fn the_limits_are_absent_when_none_is_set_and_zero_means_not_set_for_the_others() {
    let mut config = base();
    assert_eq!(iceberg_connection(&config, &NO_SECRETS).limits, None);
    config.limits = LakeLimits {
        max_record_bytes: Some(4096),
        max_records: None,
        timeout_ms: Some(60_000),
    };
    assert_eq!(
        iceberg_connection(&config, &NO_SECRETS).limits,
        Some(pb::LakeLimits {
            max_record_bytes: 4096,
            max_records: 0,
            timeout_ms: 60_000,
        })
    );
}

#[test]
fn only_the_secrets_that_were_resolved_are_in_the_message_under_their_names() {
    let catalog = br#"{"clientId":"c","clientSecret":"s"}"#;
    let storage = br#"{"accessKeyId":"a","secretAccessKey":"k"}"#;

    let both = iceberg_connection(
        &base(),
        &LakeSecrets {
            catalog: Some(catalog),
            storage: Some(storage),
        },
    );
    assert_eq!(both.credentials.len(), 2);
    assert_eq!(both.credentials["catalog"], catalog.to_vec());
    assert_eq!(both.credentials["storage"], storage.to_vec());

    let only_catalog = iceberg_connection(
        &base(),
        &LakeSecrets {
            catalog: Some(catalog),
            storage: None,
        },
    );
    assert_eq!(
        only_catalog.credentials.keys().collect::<Vec<_>>(),
        ["catalog"]
    );

    let only_storage = iceberg_connection(
        &base(),
        &LakeSecrets {
            catalog: None,
            storage: Some(storage),
        },
    );
    assert_eq!(
        only_storage.credentials.keys().collect::<Vec<_>>(),
        ["storage"]
    );
}

#[test]
fn neither_the_message_nor_the_secrets_show_a_value() {
    const CANARY: &str = "secret-canary-0123456789";
    let secret = format!(r#"{{"clientSecret":"{CANARY}"}}"#);
    let secrets = LakeSecrets {
        catalog: Some(secret.as_bytes()),
        storage: None,
    };
    let message = iceberg_connection(&base(), &secrets);
    let shown = format!("{message:?} {secrets:?}");
    assert!(!shown.contains(CANARY), "{shown}");
    assert!(shown.contains("LakeSecrets"), "{shown}");
}

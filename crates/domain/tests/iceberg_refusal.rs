//! What the settings of an Apache Iceberg connection refuse about a step: the modes it allows, then the ceiling of what it
//! receives (feature 035: T035-054; FR-035-005, FR-035-025).
//!
//! Pure: the decision is a comparison of the step with the connection's own settings, so the store only has to find the
//! connection. A mode is refused before the fields are looked at, and a refusal names a mode or a field path and class,
//! never a value.
#![allow(clippy::expect_used, clippy::panic)]

mod common;

use common::expr;
use sagat_domain::{
    classification::{EgressCeiling, Level},
    database::{
        AccessDelegation, ColumnMapping, ConnectionConfig, ConnectionRef, DatabaseLimits,
        EgressField, EgressRefusal, IcebergCatalog, IcebergConfig, IcebergWrite, IcebergWriteParts,
        LakeLimits, LakeRefusal, LakeStorage, RecordSource, RestAuth, RestCatalog, S3Storage,
        StorageCredentials, StorageKind, TableIdentifier, TlsPolicy, WriteIntent, WriteMode,
    },
    identity::Slug,
    secrets::SecretName,
};

fn write() -> IcebergWrite {
    IcebergWrite::new(IcebergWriteParts {
        connection: ConnectionRef::new(Slug::parse("lake-main").expect("name"), None).expect("ref"),
        table: TableIdentifier::new(vec!["sales".to_owned()], "orders").expect("table"),
        reference: "main".to_owned(),
        intent: WriteIntent::Append,
        records: RecordSource::Inline(expr("input.orders")),
        mapping: vec![
            ColumnMapping::new("/id", "id").expect("mapping"),
            ColumnMapping::new("/email", "email").expect("mapping"),
        ],
        ignore_unknown_fields: false,
        limits: LakeLimits::default(),
        revert: false,
    })
    .expect("a valid write")
}

fn classified() -> IcebergWrite {
    write().with_egress(vec![
        EgressField::new("/id", Level::Public),
        EgressField::new("/email", Level::Pii),
    ])
}

fn connection(modes: Vec<WriteMode>, ceiling: EgressCeiling) -> IcebergConfig {
    IcebergConfig {
        catalog: IcebergCatalog::Rest(RestCatalog {
            uri: "https://catalog.example/api".to_owned(),
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
        catalog_secret: Some(SecretName::parse("catalog-credentials").expect("secret")),
        storage_secret: Some(SecretName::parse("storage-credentials").expect("secret")),
        egress_ceiling: ceiling,
        limits: LakeLimits::default(),
    }
}

#[test]
fn a_write_in_a_mode_the_connection_allows_and_within_its_ceiling_is_not_refused() {
    let config = connection(vec![WriteMode::Append], EgressCeiling::Pii);
    assert_eq!(classified().refusal_by(&config), None);
}

#[test]
fn a_mode_the_connection_does_not_allow_is_refused_by_name() {
    let config = connection(
        vec![WriteMode::Overwrite, WriteMode::Upsert],
        EgressCeiling::Pii,
    );
    assert_eq!(
        classified().refusal_by(&config),
        Some(LakeRefusal::ModeNotAllowed(WriteMode::Append))
    );
}

#[test]
fn the_mode_is_refused_before_the_fields_are_looked_at() {
    // Both are wrong: the author fixes the cheaper one first, and a mode refusal never lists fields.
    let config = connection(vec![WriteMode::Overwrite], EgressCeiling::Public);
    assert_eq!(
        classified().refusal_by(&config),
        Some(LakeRefusal::ModeNotAllowed(WriteMode::Append))
    );
}

#[test]
fn fields_above_the_ceiling_of_the_connection_are_refused_with_their_paths_and_classes() {
    let config = connection(vec![WriteMode::Append], EgressCeiling::Internal);
    assert_eq!(
        classified().refusal_by(&config),
        Some(LakeRefusal::Egress(vec![EgressRefusal::Field {
            path: "/email".to_owned(),
            level: Level::Pii,
        }]))
    );
}

#[test]
fn a_step_nobody_classified_is_refused_even_by_a_pii_ceiling() {
    let config = connection(vec![WriteMode::Append], EgressCeiling::Pii);
    assert_eq!(
        write().refusal_by(&config),
        Some(LakeRefusal::Egress(vec![EgressRefusal::NotComputed]))
    );
}

#[test]
fn the_intent_of_a_step_is_the_mode_it_is_checked_against() {
    assert_eq!(WriteIntent::Append.mode(), WriteMode::Append);
}

#[test]
fn a_connection_config_gives_back_its_lake_settings_only_when_it_is_one() {
    let lake =
        ConnectionConfig::iceberg(connection(vec![WriteMode::Append], EgressCeiling::Internal))
            .expect("a valid lake connection");
    assert_eq!(
        lake.as_iceberg().map(|config| config.egress_ceiling),
        Some(EgressCeiling::Internal)
    );
    let sql = ConnectionConfig::postgres(
        "db.example.com",
        5432,
        "orders",
        TlsPolicy::VerifyFull { ca_pem: None },
        SecretName::parse("db-credentials").expect("secret"),
        DatabaseLimits::default(),
    )
    .expect("a valid SQL connection");
    assert!(sql.as_iceberg().is_none());
}

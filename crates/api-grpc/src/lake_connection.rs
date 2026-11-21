//! The wire form of an Iceberg connection (feature 035, T035-044, D035-132).
//!
//! `ResolveDatabase` answers a lake runner with the typed message `IcebergConnection`: the pinned, non-secret settings of one
//! revision and `credentials`, one entry for each secret the connection has (`catalog`, `storage`), each the JSON document of the
//! data model (`{"clientId","clientSecret"}`, `{"token"}`, `{"accessKeyId","secretAccessKey","sessionToken"?}`). This is the
//! server's end of that message; the runner's end is `apps/database-runner/src/lake/connection.rs`, which reads it back into the
//! documents the host reads. The two are held to the same stored document by a test that depends on both.
//!
//! Every shape the proto can carry is written here (REST, Glue, Hive and JDBC catalogs; S3, GCS and ADLS storage); which of them the
//! lake host can open is the **runner's** to refuse, with a name, before a container exists (G035-048), and not the server's to
//! hide: a connection the platform stored is answered as it is.
//!
//! The secrets are given as the bytes the server resolved; nothing here reads, formats or checks them, and the message's `Debug`
//! shows none (`crate::pb`).

use sagat_domain::classification::EgressCeiling;
use sagat_domain::database::{
    AccessDelegation, HiveTls, IcebergCatalog, IcebergConfig, JdbcEngine, LakeLimits, RestAuth,
    StorageCredentials, StorageKind, TlsPolicy,
};

use crate::pb;

/// The two secrets of a connection, as the server resolved them. A connection that needs none of one has `None` for it.
#[derive(Clone, Copy)]
pub struct LakeSecrets<'a> {
    pub catalog: Option<&'a [u8]>,
    pub storage: Option<&'a [u8]>,
}

/// The values are secrets: only whether each is there is shown.
impl std::fmt::Debug for LakeSecrets<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LakeSecrets")
            .field("catalog", &self.catalog.is_some())
            .field("storage", &self.storage.is_some())
            .finish()
    }
}

fn text(value: Option<&String>) -> String {
    value.cloned().unwrap_or_default()
}

fn catalog(catalog: &IcebergCatalog) -> pb::iceberg_connection::Catalog {
    use pb::iceberg_connection::Catalog;
    match catalog {
        IcebergCatalog::Rest(rest) => {
            let (auth, token_uri, scope, region, signing_name) = match &rest.auth {
                RestAuth::OAuth2 { token_uri, scope } => (
                    pb::LakeRestAuth::Oauth2,
                    token_uri.clone(),
                    text(scope.as_ref()),
                    String::new(),
                    String::new(),
                ),
                RestAuth::Bearer => (
                    pb::LakeRestAuth::Bearer,
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                RestAuth::SigV4 {
                    region,
                    signing_name,
                } => (
                    pb::LakeRestAuth::Sigv4,
                    String::new(),
                    String::new(),
                    region.clone(),
                    signing_name.clone(),
                ),
            };
            Catalog::Rest(pb::IcebergRestCatalog {
                uri: rest.uri.clone(),
                warehouse: rest.warehouse.clone(),
                prefix: text(rest.prefix.as_ref()),
                auth: auth as i32,
                token_uri,
                scope,
                region,
                signing_name,
                vended_credentials: rest.access_delegation == AccessDelegation::VendedCredentials,
                remote_signing: rest.remote_signing,
            })
        }
        IcebergCatalog::Glue(glue) => Catalog::Glue(pb::IcebergGlueCatalog {
            region: glue.region.clone(),
            catalog_id: glue.catalog_id.clone(),
            role_arn: text(glue.role_arn.as_ref()),
            external_id: text(glue.external_id.as_ref()),
        }),
        IcebergCatalog::Hive(hive) => Catalog::Hive(pb::IcebergHiveCatalog {
            uris: hive.uris.clone(),
            tls_required: hive.tls == HiveTls::Required,
        }),
        IcebergCatalog::Jdbc(jdbc) => {
            let (tls, ca_pem) = match &jdbc.tls {
                TlsPolicy::VerifyFull { ca_pem } => (
                    pb::DatabaseTlsMode::DatabaseTlsVerifyFull,
                    text(ca_pem.as_ref()),
                ),
                TlsPolicy::DevelopmentPlaintext => (
                    pb::DatabaseTlsMode::DatabaseTlsDevelopmentPlaintext,
                    String::new(),
                ),
            };
            Catalog::Jdbc(pb::IcebergJdbcCatalog {
                host: jdbc.host.clone(),
                port: u32::from(jdbc.port),
                database: jdbc.database.clone(),
                tls: tls as i32,
                ca_pem,
                engine: match jdbc.engine {
                    JdbcEngine::Postgresql => "postgresql".to_owned(),
                },
            })
        }
    }
}

fn storage(config: &IcebergConfig) -> pb::LakeStorage {
    use pb::lake_storage::Kind;
    let kind = match &config.storage.kind {
        StorageKind::S3(s3) => Kind::S3(pb::LakeS3Storage {
            region: s3.region.clone(),
            endpoint: text(s3.endpoint.as_ref()),
            path_style: s3.path_style,
            role_arn: text(s3.role_arn.as_ref()),
            external_id: text(s3.external_id.as_ref()),
        }),
        StorageKind::Gcs(gcs) => Kind::Gcs(pb::LakeGcsStorage {
            project: text(gcs.project.as_ref()),
            endpoint: text(gcs.endpoint.as_ref()),
        }),
        StorageKind::Adls(adls) => Kind::Adls(pb::LakeAdlsStorage {
            account: adls.account.clone(),
            endpoint_suffix: text(adls.endpoint_suffix.as_ref()),
        }),
    };
    pb::LakeStorage {
        kind: Some(kind),
        vended_credentials: config.storage.credentials == StorageCredentials::Vended,
    }
}

const fn ceiling(ceiling: EgressCeiling) -> pb::EgressCeiling {
    match ceiling {
        EgressCeiling::Public => pb::EgressCeiling::Public,
        EgressCeiling::Internal => pb::EgressCeiling::Internal,
        EgressCeiling::Pii => pb::EgressCeiling::Pii,
    }
}

/// The limits as the message has them: `0` is "not set". `None` when none is set, so that a connection without limits answers
/// without the member.
fn limits(limits: LakeLimits) -> Option<pb::LakeLimits> {
    if limits == LakeLimits::default() {
        return None;
    }
    Some(pb::LakeLimits {
        max_record_bytes: limits.max_record_bytes.unwrap_or(0),
        max_records: limits.max_records.unwrap_or(0),
        timeout_ms: limits.timeout_ms.unwrap_or(0),
    })
}

/// The message the lake runner is answered with for `config`, with the secrets that were resolved for it.
#[must_use]
pub fn iceberg_connection(
    config: &IcebergConfig,
    secrets: &LakeSecrets<'_>,
) -> pb::IcebergConnection {
    let mut credentials = std::collections::HashMap::new();
    if let Some(secret) = secrets.catalog {
        credentials.insert("catalog".to_owned(), secret.to_vec());
    }
    if let Some(secret) = secrets.storage {
        credentials.insert("storage".to_owned(), secret.to_vec());
    }
    pb::IcebergConnection {
        catalog: Some(catalog(&config.catalog)),
        storage: Some(storage(config)),
        warehouse_locations: config.warehouse_locations.clone(),
        allowed_modes: config
            .allowed_modes
            .iter()
            .map(|mode| mode.as_str().to_owned())
            .collect(),
        egress_ceiling: ceiling(config.egress_ceiling) as i32,
        limits: limits(config.limits),
        credentials,
    }
}

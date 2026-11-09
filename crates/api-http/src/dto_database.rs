//! Public connection contract: settings and secret references, never credentials/provider routing.
use sagat_application::{
    database::{ConnectionRecord, invalid_connection},
    error::AppError,
};
use sagat_domain::{
    classification::EgressCeiling,
    database::{
        AccessDelegation, AdlsStorage, ConnectionConfig, ConnectionStatus, DatabaseLimits,
        GcsStorage, GlueCatalog, HiveCatalog, HiveTls, IcebergCatalog, IcebergConfig, JdbcCatalog,
        JdbcEngine, LakeLimits, LakeStorage, RestAuth, RestCatalog, S3Storage, StorageCredentials,
        StorageKind, TlsPolicy, WriteMode,
    },
    secrets::SecretName,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct LimitsBody {
    pub max_statement_bytes: u32,
    pub max_parameters: u32,
    pub max_parameter_bytes: u32,
    pub max_rows: u32,
    pub max_result_bytes: u32,
    pub timeout_ms: u32,
    pub connect_timeout_ms: u32,
    pub pool_size: u32,
    pub idle_timeout_ms: u32,
}
impl From<DatabaseLimits> for LimitsBody {
    fn from(v: DatabaseLimits) -> Self {
        Self {
            max_statement_bytes: v.max_statement_bytes,
            max_parameters: v.max_parameters,
            max_parameter_bytes: v.max_parameter_bytes,
            max_rows: v.max_rows,
            max_result_bytes: v.max_result_bytes,
            timeout_ms: v.timeout_ms,
            connect_timeout_ms: v.connect_timeout_ms,
            pool_size: v.pool_size,
            idle_timeout_ms: v.idle_timeout_ms,
        }
    }
}
impl Default for LimitsBody {
    fn default() -> Self {
        DatabaseLimits::default().into()
    }
}
impl From<LimitsBody> for DatabaseLimits {
    fn from(v: LimitsBody) -> Self {
        Self {
            max_statement_bytes: v.max_statement_bytes,
            max_parameters: v.max_parameters,
            max_parameter_bytes: v.max_parameter_bytes,
            max_rows: v.max_rows,
            max_result_bytes: v.max_result_bytes,
            timeout_ms: v.timeout_ms,
            connect_timeout_ms: v.connect_timeout_ms,
            pool_size: v.pool_size,
            idle_timeout_ms: v.idle_timeout_ms,
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "mode", deny_unknown_fields)]
pub enum TlsBody {
    #[serde(rename = "verify-full")]
    VerifyFull {
        #[serde(rename = "caPem", skip_serializing_if = "Option::is_none")]
        ca_pem: Option<String>,
    },
    #[serde(rename = "development-plaintext")]
    DevelopmentPlaintext,
}
/// The highest level a step may write to a connection. The response omits `internal`, the default.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CeilingBody {
    Public,
    Internal,
    Pii,
}
impl From<CeilingBody> for EgressCeiling {
    fn from(v: CeilingBody) -> Self {
        match v {
            CeilingBody::Public => Self::Public,
            CeilingBody::Internal => Self::Internal,
            CeilingBody::Pii => Self::Pii,
        }
    }
}
impl From<EgressCeiling> for CeilingBody {
    fn from(v: EgressCeiling) -> Self {
        match v {
            EgressCeiling::Public => Self::Public,
            EgressCeiling::Internal => Self::Internal,
            EgressCeiling::Pii => Self::Pii,
        }
    }
}
/// `None` for the default, which a response leaves out and a revision stored without it reads as.
fn shown_ceiling(ceiling: EgressCeiling) -> Option<CeilingBody> {
    (ceiling != EgressCeiling::default()).then(|| ceiling.into())
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "driver", deny_unknown_fields)]
pub enum ConfigBody {
    #[serde(rename = "postgresql")]
    Postgres {
        host: String,
        port: u16,
        database: String,
        tls: TlsBody,
        #[serde(rename = "secretName")]
        secret_name: String,
        #[serde(default)]
        limits: LimitsBody,
        #[serde(
            rename = "egressCeiling",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        egress_ceiling: Option<CeilingBody>,
    },
    #[serde(rename = "iceberg")]
    Iceberg(Box<IcebergBody>),
}

/// The members of an Iceberg connection (the schema `IcebergConnectionConfig`, less its `driver`).
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IcebergBody {
    pub catalog: CatalogBody,
    pub storage: StorageBody,
    pub warehouse_locations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_modes: Option<Vec<ModeBody>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_ceiling: Option<CeilingBody>,
    #[serde(default)]
    pub limits: LakeLimitsBody,
}

#[derive(Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ModeBody {
    Append,
    Overwrite,
    Upsert,
}
impl From<ModeBody> for WriteMode {
    fn from(v: ModeBody) -> Self {
        match v {
            ModeBody::Append => Self::Append,
            ModeBody::Overwrite => Self::Overwrite,
            ModeBody::Upsert => Self::Upsert,
        }
    }
}
impl From<WriteMode> for ModeBody {
    fn from(v: WriteMode) -> Self {
        match v {
            WriteMode::Append => Self::Append,
            WriteMode::Overwrite => Self::Overwrite,
            WriteMode::Upsert => Self::Upsert,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CatalogKind {
    Rest,
    Glue,
    Hive,
    Jdbc,
}
/// `type` names the one member that is present: the schema's `oneOf`, checked when the body becomes a connection.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogBody {
    #[serde(rename = "type")]
    pub kind: CatalogKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest: Option<RestBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glue: Option<GlueBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hive: Option<HiveBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jdbc: Option<JdbcBody>,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum AuthBody {
    #[serde(rename = "oauth2", rename_all = "camelCase")]
    OAuth2 {
        token_uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
    },
    #[serde(rename = "bearer")]
    Bearer,
    #[serde(rename = "sigv4", rename_all = "camelCase")]
    SigV4 {
        region: String,
        signing_name: String,
    },
}
#[derive(Deserialize, Serialize, Default, Clone, Copy)]
pub enum DelegationBody {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "vended-credentials")]
    VendedCredentials,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestBody {
    pub uri: String,
    pub warehouse: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    pub auth: AuthBody,
    #[serde(default)]
    pub access_delegation: DelegationBody,
    #[serde(default)]
    pub remote_signing: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlueBody {
    pub region: String,
    pub catalog_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}
#[derive(Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum HiveTlsBody {
    Required,
    None,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HiveBody {
    pub uris: Vec<String>,
    pub tls: HiveTlsBody,
}
#[derive(Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum EngineBody {
    Postgresql,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JdbcBody {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub tls: TlsBody,
    pub engine: EngineBody,
}

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StorageKindBody {
    S3,
    Gcs,
    Adls,
}
#[derive(Deserialize, Serialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum CredentialsBody {
    #[default]
    Static,
    Vended,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageBody {
    #[serde(rename = "type")]
    pub kind: StorageKindBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3: Option<S3Body>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcs: Option<GcsBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adls: Option<AdlsBody>,
    #[serde(default)]
    pub credentials: CredentialsBody,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct S3Body {
    pub region: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub path_style: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GcsBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdlsBody {
    pub account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_suffix: Option<String>,
}
#[derive(Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LakeLimitsBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_record_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_records: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
}

/// The member of a `type`-selected object that is present, when it is the only one: the schema's `oneOf`.
fn only<T>(selected: Option<T>, others_present: bool, reason: &'static str) -> Result<T, AppError> {
    match selected {
        Some(member) if !others_present => Ok(member),
        _ => Err(AppError::InvalidConnection { reason }),
    }
}
fn secret_reference(name: Option<String>) -> Result<Option<SecretName>, AppError> {
    name.map(|name| {
        SecretName::parse(&name)
            .map_err(|_| AppError::NotAllowed("invalid database secret reference"))
    })
    .transpose()
}

impl CatalogBody {
    fn into_domain(self) -> Result<IcebergCatalog, AppError> {
        let invalid = "INVALID_CATALOG";
        match self.kind {
            CatalogKind::Rest => {
                let others = self.glue.is_some() || self.hive.is_some() || self.jdbc.is_some();
                let rest = only(self.rest, others, invalid)?;
                Ok(IcebergCatalog::Rest(RestCatalog {
                    uri: rest.uri,
                    warehouse: rest.warehouse,
                    prefix: rest.prefix,
                    auth: match rest.auth {
                        AuthBody::OAuth2 { token_uri, scope } => {
                            RestAuth::OAuth2 { token_uri, scope }
                        }
                        AuthBody::Bearer => RestAuth::Bearer,
                        AuthBody::SigV4 {
                            region,
                            signing_name,
                        } => RestAuth::SigV4 {
                            region,
                            signing_name,
                        },
                    },
                    access_delegation: match rest.access_delegation {
                        DelegationBody::None => AccessDelegation::None,
                        DelegationBody::VendedCredentials => AccessDelegation::VendedCredentials,
                    },
                    remote_signing: rest.remote_signing,
                }))
            }
            CatalogKind::Glue => {
                let others = self.rest.is_some() || self.hive.is_some() || self.jdbc.is_some();
                let glue = only(self.glue, others, invalid)?;
                Ok(IcebergCatalog::Glue(GlueCatalog {
                    region: glue.region,
                    catalog_id: glue.catalog_id,
                    role_arn: glue.role_arn,
                    external_id: glue.external_id,
                }))
            }
            CatalogKind::Hive => {
                let others = self.rest.is_some() || self.glue.is_some() || self.jdbc.is_some();
                let hive = only(self.hive, others, invalid)?;
                Ok(IcebergCatalog::Hive(HiveCatalog {
                    uris: hive.uris,
                    tls: match hive.tls {
                        HiveTlsBody::Required => HiveTls::Required,
                        HiveTlsBody::None => HiveTls::None,
                    },
                }))
            }
            CatalogKind::Jdbc => {
                let others = self.rest.is_some() || self.glue.is_some() || self.hive.is_some();
                let jdbc = only(self.jdbc, others, invalid)?;
                Ok(IcebergCatalog::Jdbc(JdbcCatalog {
                    host: jdbc.host,
                    port: jdbc.port,
                    database: jdbc.database,
                    tls: match jdbc.tls {
                        TlsBody::VerifyFull { ca_pem } => TlsPolicy::VerifyFull { ca_pem },
                        TlsBody::DevelopmentPlaintext => TlsPolicy::DevelopmentPlaintext,
                    },
                    engine: match jdbc.engine {
                        EngineBody::Postgresql => JdbcEngine::Postgresql,
                    },
                }))
            }
        }
    }
}
impl From<&IcebergCatalog> for CatalogBody {
    fn from(v: &IcebergCatalog) -> Self {
        let mut body = Self {
            kind: CatalogKind::Rest,
            rest: None,
            glue: None,
            hive: None,
            jdbc: None,
        };
        match v {
            IcebergCatalog::Rest(rest) => {
                body.rest = Some(RestBody {
                    uri: rest.uri.clone(),
                    warehouse: rest.warehouse.clone(),
                    prefix: rest.prefix.clone(),
                    auth: match &rest.auth {
                        RestAuth::OAuth2 { token_uri, scope } => AuthBody::OAuth2 {
                            token_uri: token_uri.clone(),
                            scope: scope.clone(),
                        },
                        RestAuth::Bearer => AuthBody::Bearer,
                        RestAuth::SigV4 {
                            region,
                            signing_name,
                        } => AuthBody::SigV4 {
                            region: region.clone(),
                            signing_name: signing_name.clone(),
                        },
                    },
                    access_delegation: match rest.access_delegation {
                        AccessDelegation::None => DelegationBody::None,
                        AccessDelegation::VendedCredentials => DelegationBody::VendedCredentials,
                    },
                    remote_signing: rest.remote_signing,
                });
            }
            IcebergCatalog::Glue(glue) => {
                body.kind = CatalogKind::Glue;
                body.glue = Some(GlueBody {
                    region: glue.region.clone(),
                    catalog_id: glue.catalog_id.clone(),
                    role_arn: glue.role_arn.clone(),
                    external_id: glue.external_id.clone(),
                });
            }
            IcebergCatalog::Hive(hive) => {
                body.kind = CatalogKind::Hive;
                body.hive = Some(HiveBody {
                    uris: hive.uris.clone(),
                    tls: match hive.tls {
                        HiveTls::Required => HiveTlsBody::Required,
                        HiveTls::None => HiveTlsBody::None,
                    },
                });
            }
            IcebergCatalog::Jdbc(jdbc) => {
                body.kind = CatalogKind::Jdbc;
                body.jdbc = Some(JdbcBody {
                    host: jdbc.host.clone(),
                    port: jdbc.port,
                    database: jdbc.database.clone(),
                    tls: match &jdbc.tls {
                        TlsPolicy::VerifyFull { ca_pem } => TlsBody::VerifyFull {
                            ca_pem: ca_pem.clone(),
                        },
                        TlsPolicy::DevelopmentPlaintext => TlsBody::DevelopmentPlaintext,
                    },
                    engine: EngineBody::Postgresql,
                });
            }
        }
        body
    }
}

impl StorageBody {
    fn into_domain(self) -> Result<LakeStorage, AppError> {
        let invalid = "INVALID_STORAGE";
        let kind = match self.kind {
            StorageKindBody::S3 => {
                let s3 = only(self.s3, self.gcs.is_some() || self.adls.is_some(), invalid)?;
                StorageKind::S3(S3Storage {
                    region: s3.region,
                    endpoint: s3.endpoint,
                    path_style: s3.path_style,
                    role_arn: s3.role_arn,
                    external_id: s3.external_id,
                })
            }
            StorageKindBody::Gcs => {
                let gcs = only(self.gcs, self.s3.is_some() || self.adls.is_some(), invalid)?;
                StorageKind::Gcs(GcsStorage {
                    project: gcs.project,
                    endpoint: gcs.endpoint,
                })
            }
            StorageKindBody::Adls => {
                let adls = only(self.adls, self.s3.is_some() || self.gcs.is_some(), invalid)?;
                StorageKind::Adls(AdlsStorage {
                    account: adls.account,
                    endpoint_suffix: adls.endpoint_suffix,
                })
            }
        };
        Ok(LakeStorage {
            kind,
            credentials: match self.credentials {
                CredentialsBody::Static => StorageCredentials::Static,
                CredentialsBody::Vended => StorageCredentials::Vended,
            },
        })
    }
}
impl From<&LakeStorage> for StorageBody {
    fn from(v: &LakeStorage) -> Self {
        let mut body = Self {
            kind: StorageKindBody::S3,
            s3: None,
            gcs: None,
            adls: None,
            credentials: match v.credentials {
                StorageCredentials::Static => CredentialsBody::Static,
                StorageCredentials::Vended => CredentialsBody::Vended,
            },
        };
        match &v.kind {
            StorageKind::S3(s3) => {
                body.s3 = Some(S3Body {
                    region: s3.region.clone(),
                    endpoint: s3.endpoint.clone(),
                    path_style: s3.path_style,
                    role_arn: s3.role_arn.clone(),
                    external_id: s3.external_id.clone(),
                });
            }
            StorageKind::Gcs(gcs) => {
                body.kind = StorageKindBody::Gcs;
                body.gcs = Some(GcsBody {
                    project: gcs.project.clone(),
                    endpoint: gcs.endpoint.clone(),
                });
            }
            StorageKind::Adls(adls) => {
                body.kind = StorageKindBody::Adls;
                body.adls = Some(AdlsBody {
                    account: adls.account.clone(),
                    endpoint_suffix: adls.endpoint_suffix.clone(),
                });
            }
        }
        body
    }
}

impl ConfigBody {
    pub fn into_domain(self) -> Result<ConnectionConfig, AppError> {
        match self {
            Self::Postgres {
                host,
                port,
                database,
                tls,
                secret_name,
                limits,
                egress_ceiling,
            } => {
                let tls = match tls {
                    TlsBody::VerifyFull { ca_pem } => TlsPolicy::VerifyFull { ca_pem },
                    TlsBody::DevelopmentPlaintext => TlsPolicy::DevelopmentPlaintext,
                };
                let secret = SecretName::parse(&secret_name)
                    .map_err(|_| AppError::NotAllowed("invalid database secret reference"))?;
                ConnectionConfig::postgres(&host, port, &database, tls, secret, limits.into())
                    .map(|config| {
                        config.with_egress_ceiling(
                            egress_ceiling.map_or_else(EgressCeiling::default, Into::into),
                        )
                    })
                    .map_err(|_| AppError::NotAllowed("invalid database connection configuration"))
            }
            Self::Iceberg(body) => {
                let IcebergBody {
                    catalog,
                    storage,
                    warehouse_locations,
                    allowed_modes,
                    catalog_secret,
                    storage_secret,
                    egress_ceiling,
                    limits,
                } = *body;
                let config = IcebergConfig {
                    catalog: catalog.into_domain()?,
                    storage: storage.into_domain()?,
                    warehouse_locations,
                    allowed_modes: allowed_modes
                        .map_or_else(IcebergConfig::default_modes, |modes| {
                            modes.into_iter().map(Into::into).collect()
                        }),
                    catalog_secret: secret_reference(catalog_secret)?,
                    storage_secret: secret_reference(storage_secret)?,
                    egress_ceiling: egress_ceiling.map_or_else(EgressCeiling::default, Into::into),
                    limits: LakeLimits {
                        max_record_bytes: limits.max_record_bytes,
                        max_records: limits.max_records,
                        timeout_ms: limits.timeout_ms,
                    },
                };
                ConnectionConfig::iceberg(config).map_err(invalid_connection)
            }
        }
    }
}
impl From<&ConnectionConfig> for ConfigBody {
    fn from(v: &ConnectionConfig) -> Self {
        match v {
            ConnectionConfig::Postgres {
                host,
                port,
                database,
                tls,
                secret,
                limits,
                egress_ceiling,
            } => {
                let tls = match tls {
                    TlsPolicy::VerifyFull { ca_pem } => TlsBody::VerifyFull {
                        ca_pem: ca_pem.clone(),
                    },
                    TlsPolicy::DevelopmentPlaintext => TlsBody::DevelopmentPlaintext,
                };
                Self::Postgres {
                    host: host.clone(),
                    port: *port,
                    database: database.clone(),
                    tls,
                    secret_name: secret.to_string(),
                    limits: (*limits).into(),
                    egress_ceiling: shown_ceiling(*egress_ceiling),
                }
            }
            ConnectionConfig::Iceberg(iceberg) => Self::Iceberg(Box::new(IcebergBody {
                catalog: (&iceberg.catalog).into(),
                storage: (&iceberg.storage).into(),
                warehouse_locations: iceberg.warehouse_locations.clone(),
                allowed_modes: Some(
                    iceberg
                        .allowed_modes
                        .iter()
                        .copied()
                        .map(Into::into)
                        .collect(),
                ),
                catalog_secret: iceberg.catalog_secret.as_ref().map(ToString::to_string),
                storage_secret: iceberg.storage_secret.as_ref().map(ToString::to_string),
                egress_ceiling: shown_ceiling(iceberg.egress_ceiling),
                limits: LakeLimitsBody {
                    max_record_bytes: iceberg.limits.max_record_bytes,
                    max_records: iceberg.limits.max_records,
                    timeout_ms: iceberg.limits.timeout_ms,
                },
            })),
        }
    }
}
#[derive(Deserialize, Serialize)]
pub enum StatusBody {
    #[serde(rename = "ACTIVE")]
    Active,
    #[serde(rename = "PAUSED")]
    Paused,
}
impl From<StatusBody> for ConnectionStatus {
    fn from(v: StatusBody) -> Self {
        match v {
            StatusBody::Active => Self::Active,
            StatusBody::Paused => Self::Paused,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PutConnectionBody {
    pub request_id: String,
    pub expected_revision: u32,
    pub config: ConfigBody,
    pub status: StatusBody,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteConnectionBody {
    pub request_id: String,
    pub expected_revision: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionBody {
    pub id: String,
    pub namespace_id: String,
    pub name: String,
    pub revision: u32,
    pub status: &'static str,
    pub config: ConfigBody,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}
impl From<&ConnectionRecord> for ConnectionBody {
    fn from(v: &ConnectionRecord) -> Self {
        Self {
            id: v.id.to_string(),
            namespace_id: v.namespace_id.to_string(),
            name: v.name.to_string(),
            revision: v.revision,
            status: v.status.as_str(),
            config: (&v.config).into(),
            created_at: v.created_at,
            updated_at: v.updated_at,
        }
    }
}
#[derive(Serialize)]
pub struct ConnectionListBody {
    pub items: Vec<ConnectionBody>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionQuery {
    pub revision: Option<u32>,
}

/// Implemented server capabilities; a healthy runner and target access still require a probe.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseDriverBody {
    pub id: &'static str,
    pub operations: [&'static str; 2],
    pub parameter_types: [&'static str; 10],
    pub default_limits: LimitsBody,
    pub maximum_limits: LimitsBody,
}
/// What the platform serves of Apache Iceberg **end to end**: a REST catalog over S3-compatible storage, appending, from the two record
/// formats. Not what a connection document may say (the domain accepts other catalogs and stores, which no runner opens yet), because the
/// list is what the Studio and the SDKs offer and so must not advertise a capability that is not built (G035-030). The integration test of the
/// listing holds the body to the OpenAPI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LakeDatabaseDriverBody {
    pub id: &'static str,
    pub catalog_types: &'static [&'static str],
    pub storage_types: &'static [&'static str],
    pub modes: &'static [&'static str],
    pub formats: &'static [&'static str],
}
impl LakeDatabaseDriverBody {
    /// Apache Iceberg as this build serves it.
    pub const ICEBERG: Self = Self {
        id: "iceberg",
        catalog_types: &["rest"],
        storage_types: &["s3"],
        modes: &["append"],
        formats: &["parquet", "ndjson"],
    };
}
/// A driver of the list: the SQL one with its operations, parameters and limits, or one of the lake deployment.
#[derive(Serialize)]
#[serde(untagged)]
pub enum DatabaseDriverItem {
    Sql(DatabaseDriverBody),
    Lake(LakeDatabaseDriverBody),
}
#[derive(Serialize)]
pub struct DatabaseDriverListBody {
    pub items: Vec<DatabaseDriverItem>,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn postgres() -> Value {
        json!({
            "driver": "postgresql", "host": "db.example.com", "port": 5432, "database": "orders",
            "tls": {"mode": "verify-full"}, "secretName": "orders-access"
        })
    }

    /// REST and S3 with every member the response states, written by hand from the OpenAPI schema.
    fn rest_s3() -> Value {
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
            "catalogSecret": "catalog-oauth",
            "storageSecret": "lake-s3",
            "limits": {}
        })
    }

    type Edits = Vec<(&'static str, Box<dyn Fn(&mut Value)>)>;

    fn body(value: Value) -> Result<ConfigBody, serde_json::Error> {
        serde_json::from_value(value)
    }

    fn domain(value: Value) -> Result<ConnectionConfig, AppError> {
        body(value).expect("it parses").into_domain()
    }

    /// The request and the response are one document: what a client sends comes back as it was sent.
    #[test]
    fn an_iceberg_connection_comes_back_as_the_document_it_was_sent_as() {
        let sent = rest_s3();
        let config = domain(sent.clone()).expect("a valid connection");

        assert_eq!(
            serde_json::to_value(ConfigBody::from(&config)).unwrap(),
            sent
        );
    }

    #[test]
    fn the_members_a_request_leaves_out_take_their_documented_defaults() {
        let minimal = json!({
            "driver": "iceberg",
            "catalog": {"type": "rest", "rest": {
                "uri": "https://catalog.example/api", "warehouse": "analytics", "auth": {"type": "bearer"}
            }},
            "storage": {"type": "s3", "s3": {"region": "eu-west-1"}},
            "warehouseLocations": ["s3://lake/w/"],
            "catalogSecret": "catalog-token",
            "storageSecret": "lake-s3"
        });

        let config = domain(minimal).expect("a valid connection");
        let shown = serde_json::to_value(ConfigBody::from(&config)).unwrap();

        assert_eq!(shown["allowedModes"], json!(["append"]));
        assert_eq!(shown["storage"]["credentials"], "static");
        assert_eq!(shown["catalog"]["rest"]["accessDelegation"], "none");
        assert_eq!(shown["catalog"]["rest"]["remoteSigning"], false);
        assert_eq!(shown["storage"]["s3"]["pathStyle"], false);
        assert!(shown.get("egressCeiling").is_none());
    }

    #[test]
    fn a_member_the_contract_does_not_name_is_refused_at_every_level() {
        let edits: Edits = vec![
            ("top", Box::new(|v| v["extra"] = json!(1))),
            ("catalog", Box::new(|v| v["catalog"]["extra"] = json!(1))),
            (
                "rest",
                Box::new(|v| v["catalog"]["rest"]["extra"] = json!(1)),
            ),
            (
                "auth",
                Box::new(|v| v["catalog"]["rest"]["auth"]["extra"] = json!(1)),
            ),
            ("storage", Box::new(|v| v["storage"]["extra"] = json!(1))),
            ("s3", Box::new(|v| v["storage"]["s3"]["extra"] = json!(1))),
            ("limits", Box::new(|v| v["limits"]["extra"] = json!(1))),
            (
                "a secret value",
                Box::new(|v| v["password"] = json!("hunter2")),
            ),
            (
                "an inline credential",
                Box::new(|v| v["storage"]["s3"]["secretAccessKey"] = json!("k")),
            ),
        ];
        for (level, edit) in edits {
            let mut sent = rest_s3();
            edit(&mut sent);
            assert!(body(sent).is_err(), "{level}");
        }
    }

    #[test]
    fn a_value_of_the_wrong_type_or_outside_an_enum_is_refused() {
        let edits: Edits = vec![
            (
                "an unknown catalog type",
                Box::new(|v| v["catalog"]["type"] = json!("nessie")),
            ),
            (
                "an unknown auth",
                Box::new(|v| v["catalog"]["rest"]["auth"]["type"] = json!("basic")),
            ),
            (
                "an unknown mode",
                Box::new(|v| v["allowedModes"] = json!(["merge"])),
            ),
            (
                "an unknown storage type",
                Box::new(|v| v["storage"]["type"] = json!("hdfs")),
            ),
            (
                "unknown credentials",
                Box::new(|v| v["storage"]["credentials"] = json!("ambient")),
            ),
            (
                "an unknown delegation",
                Box::new(|v| v["catalog"]["rest"]["accessDelegation"] = json!("sometimes")),
            ),
            (
                "a path style as a string",
                Box::new(|v| v["storage"]["s3"]["pathStyle"] = json!("yes")),
            ),
            (
                "locations as a string",
                Box::new(|v| v["warehouseLocations"] = json!("s3://lake/")),
            ),
            (
                "a limit as a string",
                Box::new(|v| v["limits"]["timeoutMs"] = json!("fast")),
            ),
            (
                "a negative limit",
                Box::new(|v| v["limits"]["maxRecords"] = json!(-1)),
            ),
            (
                "an unknown ceiling",
                Box::new(|v| v["egressCeiling"] = json!("secret")),
            ),
        ];
        for (what, edit) in edits {
            let mut sent = rest_s3();
            edit(&mut sent);
            assert!(body(sent).is_err(), "{what}");
        }
    }

    /// "The member that matches `type` is required and no other is allowed."
    #[test]
    fn only_the_member_that_matches_the_type_is_accepted() {
        let reason = |value: Value| match domain(value) {
            Err(AppError::InvalidConnection { reason }) => reason,
            other => panic!("not a refusal with a reason: {other:?}"),
        };
        let mut missing = rest_s3();
        missing["catalog"].as_object_mut().unwrap().remove("rest");
        assert_eq!(reason(missing), "INVALID_CATALOG");

        let mut two = rest_s3();
        two["catalog"]["glue"] = json!({"region": "eu-west-1", "catalogId": "1"});
        assert_eq!(reason(two), "INVALID_CATALOG");

        let mut storage_missing = rest_s3();
        storage_missing["storage"]
            .as_object_mut()
            .unwrap()
            .remove("s3");
        assert_eq!(reason(storage_missing), "INVALID_STORAGE");

        let mut storage_two = rest_s3();
        storage_two["storage"]["gcs"] = json!({});
        assert_eq!(reason(storage_two), "INVALID_STORAGE");
    }

    /// "A URI with user info or query credentials fails `VALIDATION_FAILED`, reason `CREDENTIAL_IN_URI`" from the body to
    /// the error the API words.
    #[test]
    fn a_credential_in_a_uri_is_refused_with_its_reason() {
        let mut sent = rest_s3();
        sent["catalog"]["rest"]["uri"] = json!("https://user:hunter2@catalog.example/api");

        let refusal = domain(sent).expect_err("refused");

        assert_eq!(
            refusal,
            AppError::InvalidConnection {
                reason: "CREDENTIAL_IN_URI"
            }
        );
        assert!(!format!("{refusal} {refusal:?}").contains("hunter2"));
    }

    #[test]
    fn a_rule_of_the_domain_is_refused_with_its_reason() {
        let mut vended = rest_s3();
        vended["storage"]["credentials"] = json!("vended");
        assert_eq!(
            domain(vended).expect_err("refused"),
            AppError::InvalidConnection {
                reason: "VENDED_NEEDS_DELEGATION"
            }
        );

        let mut no_secret = rest_s3();
        no_secret.as_object_mut().unwrap().remove("catalogSecret");
        assert_eq!(
            domain(no_secret).expect_err("refused"),
            AppError::InvalidConnection {
                reason: "CATALOG_SECRET_REQUIRED"
            }
        );

        let mut bad_secret = rest_s3();
        bad_secret["storageSecret"] = json!("Not A Name");
        assert!(matches!(domain(bad_secret), Err(AppError::NotAllowed(_))));
    }

    /// G035-027 item 7, closed: the ceiling is part of the body of both kinds, and `pii` is checked against the reveal
    /// grant where the revision is written (D035-074).
    #[test]
    fn the_egress_ceiling_is_accepted_for_both_kinds_and_shown_only_when_it_is_not_the_default() {
        for (kind, base) in [("postgresql", postgres()), ("iceberg", rest_s3())] {
            for (sent, expected) in [
                ("public", EgressCeiling::Public),
                ("internal", EgressCeiling::Internal),
                ("pii", EgressCeiling::Pii),
            ] {
                let mut with_ceiling = base.clone();
                with_ceiling["egressCeiling"] = json!(sent);
                let config = domain(with_ceiling).expect("a valid connection");
                assert_eq!(config.egress_ceiling(), expected, "{kind} {sent}");

                let shown = serde_json::to_value(ConfigBody::from(&config)).unwrap();
                if expected == EgressCeiling::Internal {
                    assert!(shown.get("egressCeiling").is_none(), "{kind}");
                } else {
                    assert_eq!(shown["egressCeiling"], sent, "{kind}");
                }
            }
            let left_out = domain(base).expect("a valid connection");
            assert_eq!(left_out.egress_ceiling(), EgressCeiling::Internal, "{kind}");
        }
    }

    #[test]
    fn a_postgresql_body_is_what_it_was_before() {
        let config = domain(postgres()).expect("a valid connection");

        let shown = serde_json::to_value(ConfigBody::from(&config)).unwrap();

        assert_eq!(shown["driver"], "postgresql");
        assert_eq!(shown["secretName"], "orders-access");
        assert!(shown.get("egressCeiling").is_none());
        assert!(shown["limits"].get("maxRows").is_some());
        for unknown in ["catalog", "storage", "warehouseLocations"] {
            let mut sent = postgres();
            sent[unknown] = json!({});
            assert!(body(sent).is_err(), "{unknown}");
        }
    }

    #[test]
    fn every_catalog_and_storage_shape_comes_back_as_sent() {
        let mut shapes = Vec::new();
        let mut glue = rest_s3();
        glue["catalog"] = json!({"type": "glue", "glue": {
            "region": "eu-west-1", "catalogId": "123456789012",
            "roleArn": "arn:aws:iam::123456789012:role/lake", "externalId": "tenant-a"
        }});
        shapes.push(glue);
        let mut hive = rest_s3();
        hive["catalog"] = json!({"type": "hive", "hive": {"uris": ["thrift://hive.example:9083"], "tls": "required"}});
        hive.as_object_mut().unwrap().remove("catalogSecret");
        shapes.push(hive);
        let mut jdbc = rest_s3();
        jdbc["catalog"] = json!({"type": "jdbc", "jdbc": {
            "host": "db.example.com", "port": 5432, "database": "iceberg",
            "tls": {"mode": "verify-full"}, "engine": "postgresql"
        }});
        shapes.push(jdbc);
        let mut gcs = rest_s3();
        gcs["storage"] = json!({"type": "gcs", "gcs": {"project": "p"}, "credentials": "static"});
        gcs["warehouseLocations"] = json!(["gs://lake/w/"]);
        shapes.push(gcs);
        let mut adls = rest_s3();
        adls["storage"] = json!({"type": "adls", "adls": {"account": "lakeaccount", "endpointSuffix": "core.windows.net"}, "credentials": "static"});
        adls["warehouseLocations"] = json!(["abfss://data@lakeaccount.dfs.core.windows.net/w/"]);
        shapes.push(adls);
        let mut vended = rest_s3();
        vended["catalog"]["rest"]["accessDelegation"] = json!("vended-credentials");
        vended["storage"]["credentials"] = json!("vended");
        vended.as_object_mut().unwrap().remove("storageSecret");
        shapes.push(vended);
        let mut limits = rest_s3();
        limits["limits"] = json!({"maxRecordBytes": 4096, "maxRecords": 500, "timeoutMs": 60000});
        limits["allowedModes"] = json!(["upsert", "append"]);
        limits["egressCeiling"] = json!("public");
        shapes.push(limits);

        for sent in shapes {
            let config = domain(sent.clone()).expect("a valid connection");
            assert_eq!(
                serde_json::to_value(ConfigBody::from(&config)).unwrap(),
                sent
            );
        }
    }
}

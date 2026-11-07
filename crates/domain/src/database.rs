//! Managed database targets and operations (feature 017). No driver or network I/O lives here.
mod failure;
mod iceberg;
mod iceberg_egress;
mod iceberg_write;
pub use failure::DatabaseFailure;
pub use iceberg::{
    AccessDelegation, AdlsStorage, GcsStorage, GlueCatalog, HiveCatalog, HiveTls, IcebergCatalog,
    IcebergConfig, JdbcCatalog, JdbcEngine, LakeConfigError, LakeLimits, LakeStorage, RestAuth,
    RestCatalog, S3Storage, StorageCredentials, StorageKind, WriteMode,
};
pub use iceberg::{
    DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, DEFAULT_TIMEOUT_MS, EffectiveLakeLimits,
};
pub use iceberg_egress::{EgressField, EgressRefusal, LakeRefusal};
pub use iceberg_write::{
    ColumnMapping, ICEBERG_REVERT_ACTIVITY, ICEBERG_WRITE_ACTIVITY, IcebergWrite,
    IcebergWriteError, IcebergWriteParts, MAX_MAPPED_COLUMNS, MAX_MAPPING_DEPTH, MAX_RECORD_BLOBS,
    RecordSource, TableIdentifier, WriteIntent,
};

use crate::{
    classification::EgressCeiling, identity::Slug, secrets::SecretName, workflow::Expression,
};
use std::{fmt, net::IpAddr};

uuid_id!(
    /// Stable identity of a namespace-scoped connection.
    ConnectionId
);

pub const MANAGED_DATABASE_ACTIVITY: &str = "sagat.database.postgresql";
pub const MANAGED_DATABASE_PREFIX: &str = "sagat.database.";
pub const MAX_CONNECTIONS: u32 = 1000;
pub const DEFAULT_CONNECTIONS: u32 = 100;

/// Safe categories: never include SQL, credentials, parameters or driver diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DatabaseError {
    #[error("a database host must be a bare DNS name or IP address")]
    InvalidHost,
    #[error("invalid database port or database name")]
    InvalidTarget,
    #[error("invalid database TLS configuration")]
    InvalidTls,
    #[error("connection revision must be positive")]
    InvalidRevision,
    #[error("database limits must be positive and within platform ceilings")]
    InvalidLimits,
    #[error("database statement is empty, contains NUL, or exceeds its byte limit")]
    InvalidStatement,
    #[error("database parameters exceed their count or byte limit")]
    ParameterLimit,
    #[error("unsupported or invalid database value")]
    InvalidValue,
    #[error("setting a PII egress ceiling needs the reveal grant of the namespace")]
    RevealGrantRequired,
    #[error("invalid lake connection: {0}")]
    Lake(LakeConfigError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsPolicy {
    VerifyFull {
        ca_pem: Option<String>,
    },
    /// Admission AND runner deployment policy must explicitly permit this in development.
    DevelopmentPlaintext,
}
impl TlsPolicy {
    fn validate(&self) -> Result<(), DatabaseError> {
        if let Self::VerifyFull { ca_pem: Some(pem) } = self
            && (pem.len() > 64 * 1024
                || !pem.starts_with("-----BEGIN CERTIFICATE-----")
                || !pem.trim_end().ends_with("-----END CERTIFICATE-----")
                || pem.contains("PRIVATE KEY")
                || pem.contains('\0'))
        {
            return Err(DatabaseError::InvalidTls);
        }
        Ok(())
    }
}

/// Shared finite ceilings. Public fields allow policy composition; all admission boundaries call
/// validate before use. Effective values are the component-wise minimum, never a sum/override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseLimits {
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
impl Default for DatabaseLimits {
    fn default() -> Self {
        Self {
            max_statement_bytes: 16 * 1024,
            max_parameters: 64,
            max_parameter_bytes: 64 * 1024,
            max_rows: 1000,
            max_result_bytes: 1024 * 1024,
            timeout_ms: 10_000,
            connect_timeout_ms: 5000,
            pool_size: 4,
            idle_timeout_ms: 30_000,
        }
    }
}
impl DatabaseLimits {
    pub const MAX: Self = Self {
        max_statement_bytes: 64 * 1024,
        max_parameters: 256,
        max_parameter_bytes: 256 * 1024,
        max_rows: 10_000,
        max_result_bytes: 4 * 1024 * 1024,
        timeout_ms: 30_000,
        connect_timeout_ms: 10_000,
        pool_size: 16,
        idle_timeout_ms: 60_000,
    };
    fn values(self) -> [u32; 9] {
        [
            self.max_statement_bytes,
            self.max_parameters,
            self.max_parameter_bytes,
            self.max_rows,
            self.max_result_bytes,
            self.timeout_ms,
            self.connect_timeout_ms,
            self.pool_size,
            self.idle_timeout_ms,
        ]
    }
    pub fn validate(self) -> Result<(), DatabaseError> {
        if self
            .values()
            .into_iter()
            .zip(Self::MAX.values())
            .any(|(v, max)| v == 0 || v > max)
        {
            return Err(DatabaseError::InvalidLimits);
        }
        Ok(())
    }
    pub fn intersect(self, other: Self) -> Self {
        Self {
            max_statement_bytes: self.max_statement_bytes.min(other.max_statement_bytes),
            max_parameters: self.max_parameters.min(other.max_parameters),
            max_parameter_bytes: self.max_parameter_bytes.min(other.max_parameter_bytes),
            max_rows: self.max_rows.min(other.max_rows),
            max_result_bytes: self.max_result_bytes.min(other.max_result_bytes),
            timeout_ms: self.timeout_ms.min(other.timeout_ms),
            connect_timeout_ms: self.connect_timeout_ms.min(other.connect_timeout_ms),
            pool_size: self.pool_size.min(other.pool_size),
            idle_timeout_ms: self.idle_timeout_ms.min(other.idle_timeout_ms),
        }
    }
}

/// A bare DNS name or an IP address: no scheme, port, path, query or user information.
fn valid_host(host: &str) -> bool {
    let dns = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    dns || host.parse::<IpAddr>().is_ok()
}

/// Driver-specific configuration keeps an extension boundary without advertising unbuilt drivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionConfig {
    Postgres {
        host: String,
        port: u16,
        database: String,
        tls: TlsPolicy,
        secret: SecretName,
        limits: DatabaseLimits,
        /// The highest level a step may write to this connection (feature 035). A revision stored without one reads
        /// as `internal`.
        egress_ceiling: EgressCeiling,
    },
    /// An Apache Iceberg catalog and its storage (feature 035). Boxed: it is far larger than the others.
    Iceberg(Box<IcebergConfig>),
}
impl ConnectionConfig {
    pub fn postgres(
        host: &str,
        port: u16,
        database: &str,
        tls: TlsPolicy,
        secret: SecretName,
        limits: DatabaseLimits,
    ) -> Result<Self, DatabaseError> {
        let config = Self::Postgres {
            host: host.to_owned(),
            port,
            database: database.to_owned(),
            tls,
            secret,
            limits,
            egress_ceiling: EgressCeiling::default(),
        };
        config.validate()?;
        Ok(config)
    }
    /// An Iceberg connection, checked against every rule of `data-model.md` section 1.
    pub fn iceberg(config: IcebergConfig) -> Result<Self, DatabaseError> {
        config.validate()?;
        Ok(Self::Iceberg(Box::new(config)))
    }
    /// The name of the driver, as the API and the stored configuration give it.
    #[must_use]
    pub const fn driver(&self) -> &'static str {
        match self {
            Self::Postgres { .. } => "postgresql",
            Self::Iceberg(_) => "iceberg",
        }
    }
    /// The same connection with another egress ceiling. Whether the actor may set it is `authorize_egress_ceiling`.
    #[must_use]
    pub fn with_egress_ceiling(mut self, ceiling: EgressCeiling) -> Self {
        match &mut self {
            Self::Postgres { egress_ceiling, .. } => *egress_ceiling = ceiling,
            Self::Iceberg(config) => config.egress_ceiling = ceiling,
        }
        self
    }
    #[must_use]
    pub fn egress_ceiling(&self) -> EgressCeiling {
        match self {
            Self::Postgres { egress_ceiling, .. } => *egress_ceiling,
            Self::Iceberg(config) => config.egress_ceiling,
        }
    }
    /// Setting `pii` as the ceiling needs the actor to hold the reveal grant for the namespace (FR-035-005); the other
    /// ceilings need nothing more than the right to manage connections.
    pub fn authorize_egress_ceiling(&self, holds_reveal_grant: bool) -> Result<(), DatabaseError> {
        if self.egress_ceiling().requires_reveal_grant() && !holds_reveal_grant {
            return Err(DatabaseError::RevealGrantRequired);
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), DatabaseError> {
        let (host, port, database, tls, limits) = match self {
            Self::Postgres {
                host,
                port,
                database,
                tls,
                limits,
                ..
            } => (host, port, database, tls, limits),
            Self::Iceberg(config) => return config.validate(),
        };
        if !valid_host(host) {
            return Err(DatabaseError::InvalidHost);
        }
        if *port == 0
            || database.is_empty()
            || database.len() > 63
            || database.chars().any(char::is_control)
        {
            return Err(DatabaseError::InvalidTarget);
        }
        tls.validate()?;
        limits.validate()
    }
    /// The settings of an Apache Iceberg connection, or `None` for any other kind: a step that writes to a lake reads the
    /// modes and the ceiling from them, and a connection of another kind is unusable to it.
    #[must_use]
    pub const fn as_iceberg(&self) -> Option<&IcebergConfig> {
        match self {
            Self::Postgres { .. } => None,
            Self::Iceberg(config) => Some(config),
        }
    }
    /// The logical secrets the connection refers to, for the checks that each exists with the right authority. A
    /// PostgreSQL connection has one; an Iceberg connection has a catalog secret, a storage secret, both or neither.
    #[must_use]
    pub fn secrets(&self) -> Vec<&SecretName> {
        match self {
            Self::Postgres { secret, .. } => vec![secret],
            Self::Iceberg(config) => config
                .catalog_secret
                .iter()
                .chain(config.storage_secret.iter())
                .collect(),
        }
    }
    /// The secret of a SQL connection. `None` for a lake connection: the managed executor runs SQL and nothing else, so
    /// a path that needs one refuses the connection instead of guessing.
    #[must_use]
    pub fn sql_secret(&self) -> Option<&SecretName> {
        match self {
            Self::Postgres { secret, .. } => Some(secret),
            Self::Iceberg(_) => None,
        }
    }
    /// The SQL limits of a SQL connection; `None` for a lake connection, whose limits are [`LakeLimits`].
    #[must_use]
    pub fn sql_limits(&self) -> Option<DatabaseLimits> {
        match self {
            Self::Postgres { limits, .. } => Some(*limits),
            Self::Iceberg(_) => None,
        }
    }
    /// Whether the connection reaches something without transport security, which only a development namespace allows.
    #[must_use]
    pub fn requires_development(&self) -> bool {
        match self {
            Self::Postgres { tls, .. } => *tls == TlsPolicy::DevelopmentPlaintext,
            Self::Iceberg(config) => config.requires_development(),
        }
    }
    /// Whether the connection uses a Hive metastore with `tls: none`, which the deployment must allow
    /// (`allowUnauthenticatedMetastore`).
    #[must_use]
    pub fn uses_unauthenticated_metastore(&self) -> bool {
        match self {
            Self::Postgres { .. } => false,
            Self::Iceberg(config) => config.uses_unauthenticated_metastore(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Active,
    Paused,
    Deleted,
}
impl ConnectionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
            Self::Deleted => "DELETED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRef {
    pub name: Slug,
    pub revision: Option<u32>,
}
impl ConnectionRef {
    pub fn new(name: Slug, revision: Option<u32>) -> Result<Self, DatabaseError> {
        if revision == Some(0) {
            return Err(DatabaseError::InvalidRevision);
        }
        Ok(Self { name, revision })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Query,
    Execute,
}
impl OperationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Execute => "execute",
        }
    }
}

/// Finite IEEE value; bits preserve Eq on immutable workflow definitions.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FiniteFloat(u64);
impl FiniteFloat {
    pub fn get(self) -> f64 {
        f64::from_bits(self.0)
    }
}
impl fmt::Debug for FiniteFloat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[database float]")
    }
}

/// Large integers/decimals are decimal strings in every SDK. JSON is opaque content here; the
/// wire/driver boundary must parse its syntax before creating the operation. Debug is redacted.
#[derive(Clone, PartialEq, Eq)]
pub enum TypedValue {
    Null,
    Boolean(bool),
    Integer(String),
    Decimal(String),
    Float(FiniteFloat),
    Text(String),
    Timestamp(time::OffsetDateTime),
    Date(time::Date),
    Binary(Vec<u8>),
    Json(String),
}
impl fmt::Debug for TypedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[database value]")
    }
}
impl TypedValue {
    pub fn float(value: f64) -> Result<Self, DatabaseError> {
        if !value.is_finite() {
            return Err(DatabaseError::InvalidValue);
        }
        Ok(Self::Float(FiniteFloat(value.to_bits())))
    }
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Null => 0,
            Self::Boolean(_) => 1,
            Self::Float(_) => 8,
            Self::Integer(s) | Self::Decimal(s) | Self::Text(s) | Self::Json(s) => s.len(),
            Self::Timestamp(_) => 40,
            Self::Date(_) => 10,
            Self::Binary(b) => b.len(),
        }
    }
    pub fn validate(&self) -> Result<(), DatabaseError> {
        let valid = match self {
            Self::Integer(s) => decimal(s, false) && s.len() <= 40,
            Self::Decimal(s) => decimal(s, true) && s.len() <= 16_384,
            Self::Text(s) => !s.contains('\0'),
            _ => true,
        };
        if !valid {
            return Err(DatabaseError::InvalidValue);
        }
        Ok(())
    }
}
fn decimal(s: &str, fractional: bool) -> bool {
    let unsigned = s.strip_prefix('-').unwrap_or(s);
    let mut parts = unsigned.split('.');
    let whole = parts.next().unwrap_or_default();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    digits(whole)
        && match parts.next() {
            None => true,
            Some(tail) => fractional && digits(tail) && parts.next().is_none(),
        }
}

/// An expression declares its wire type, so SDK number conventions cannot change a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseType {
    Null,
    Boolean,
    Integer,
    Decimal,
    Float,
    Text,
    Timestamp,
    Date,
    Binary,
    Json,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Parameter {
    Literal(TypedValue),
    Expression {
        value_type: DatabaseType,
        expression: Expression,
    },
}
impl fmt::Debug for Parameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[database parameter]")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DatabaseOperation {
    pub connection: ConnectionRef,
    pub kind: OperationKind,
    statement: String,
    parameters: Vec<Parameter>,
    pub limits: DatabaseLimits,
}
impl fmt::Debug for DatabaseOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseOperation")
            .field("kind", &self.kind)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
impl DatabaseOperation {
    pub fn new(
        connection: ConnectionRef,
        kind: OperationKind,
        statement: &str,
        parameters: Vec<Parameter>,
        limits: DatabaseLimits,
    ) -> Result<Self, DatabaseError> {
        let operation = Self {
            connection,
            kind,
            statement: statement.to_owned(),
            parameters,
            limits,
        };
        operation.validate()?;
        Ok(operation)
    }
    pub fn validate(&self) -> Result<(), DatabaseError> {
        self.limits.validate()?;
        if self.connection.revision == Some(0) {
            return Err(DatabaseError::InvalidRevision);
        }
        if self.statement.trim().is_empty()
            || self.statement.len() > self.limits.max_statement_bytes as usize
            || self.statement.contains('\0')
        {
            return Err(DatabaseError::InvalidStatement);
        }
        let mut bytes = 0_usize;
        for parameter in &self.parameters {
            let length = match parameter {
                Parameter::Literal(value) => {
                    value.validate()?;
                    value.byte_len()
                }
                Parameter::Expression { expression, .. } => expression.as_str().len(),
            };
            bytes = bytes
                .checked_add(length)
                .ok_or(DatabaseError::ParameterLimit)?;
        }
        if self.parameters.len() > self.limits.max_parameters as usize
            || bytes > self.limits.max_parameter_bytes as usize
        {
            return Err(DatabaseError::ParameterLimit);
        }
        Ok(())
    }
    pub fn statement(&self) -> &str {
        &self.statement
    }
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }
    pub fn footprint(&self) -> usize {
        self.statement.len()
            + self
                .parameters
                .iter()
                .map(|p| {
                    std::mem::size_of::<Parameter>()
                        + match p {
                            Parameter::Literal(value) => value.byte_len(),
                            Parameter::Expression { expression, .. } => expression.as_str().len(),
                        }
                })
                .sum::<usize>()
    }
}

//! The Iceberg connection (feature 035, `specs/035-iceberg-write-steps/data-model.md` section 1).
//!
//! A connection names a catalog, the storage the table files live in, the prefixes writes may target, the write modes
//! it allows and the logical secrets that hold its credentials. It holds references to secrets and never their values,
//! and nothing here opens a connection: the platform never creates a bucket, a namespace or a table.
//!
//! The shapes follow the `IcebergConnectionConfig` schema of the OpenAPI contract member by member, so the REST
//! adapter maps one to the other without interpretation.

use super::{DatabaseError, TlsPolicy, valid_host};
use crate::{classification::EgressCeiling, secrets::SecretName};

const MAX_URI_BYTES: usize = 2048;
const MAX_LOCATION_BYTES: usize = 1024;
const MAX_LOCATIONS: usize = 16;
const MAX_HIVE_URIS: usize = 4;

/// Why a lake connection is refused: a closed reason that names no value, so it can be returned, logged and audited.
/// The API shows [`LakeConfigError::reason`] as the `reason` of `VALIDATION_FAILED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LakeConfigError {
    #[error("a URI must be an http(s) address with a host and no control character")]
    InvalidUri,
    #[error("a URI carries user information or query credentials")]
    CredentialInUri,
    #[error("the catalog settings are invalid")]
    InvalidCatalog,
    #[error("the storage settings are invalid")]
    InvalidStorage,
    #[error("vended credentials need a REST catalog that delegates access")]
    VendedNeedsDelegation,
    #[error("the storage credentials are vended, so there is no storage secret")]
    StorageSecretWithVended,
    #[error("static storage credentials need a storage secret")]
    StorageSecretRequired,
    #[error("this catalog authenticates, so it needs a catalog secret")]
    CatalogSecretRequired,
    #[error("this catalog has no credentials, so it takes no catalog secret")]
    CatalogSecretForbidden,
    #[error("warehouse locations are 1 to 16 distinct prefixes without a way out of them")]
    InvalidLocations,
    #[error("a warehouse location uses the scheme of another storage type")]
    LocationSchemeMismatch,
    #[error("allowed modes are a non-empty set of distinct modes")]
    InvalidModes,
    #[error("a write limit is outside its range")]
    InvalidLimits,
}

impl LakeConfigError {
    /// The stable name the API gives this refusal.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::InvalidUri => "INVALID_URI",
            Self::CredentialInUri => "CREDENTIAL_IN_URI",
            Self::InvalidCatalog => "INVALID_CATALOG",
            Self::InvalidStorage => "INVALID_STORAGE",
            Self::VendedNeedsDelegation => "VENDED_NEEDS_DELEGATION",
            Self::StorageSecretWithVended => "STORAGE_SECRET_WITH_VENDED",
            Self::StorageSecretRequired => "STORAGE_SECRET_REQUIRED",
            Self::CatalogSecretRequired => "CATALOG_SECRET_REQUIRED",
            Self::CatalogSecretForbidden => "CATALOG_SECRET_FORBIDDEN",
            Self::InvalidLocations => "INVALID_LOCATIONS",
            Self::LocationSchemeMismatch => "LOCATION_SCHEME_MISMATCH",
            Self::InvalidModes => "INVALID_MODES",
            Self::InvalidLimits => "INVALID_LIMITS",
        }
    }
}

impl From<LakeConfigError> for DatabaseError {
    fn from(reason: LakeConfigError) -> Self {
        Self::Lake(reason)
    }
}

/// How a step may change a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WriteMode {
    Append,
    Overwrite,
    Upsert,
}

impl WriteMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Overwrite => "overwrite",
            Self::Upsert => "upsert",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestAuth {
    OAuth2 {
        token_uri: String,
        scope: Option<String>,
    },
    Bearer,
    SigV4 {
        region: String,
        signing_name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessDelegation {
    None,
    VendedCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestCatalog {
    pub uri: String,
    pub warehouse: String,
    pub prefix: Option<String>,
    pub auth: RestAuth,
    pub access_delegation: AccessDelegation,
    pub remote_signing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlueCatalog {
    pub region: String,
    pub catalog_id: String,
    pub role_arn: Option<String>,
    pub external_id: Option<String>,
}

/// A metastore with `none` has no transport security and no authentication: it is allowed only where the deployment
/// says so (`allowUnauthenticatedMetastore`), which the application checks with
/// [`ConnectionConfig::uses_unauthenticated_metastore`](super::ConnectionConfig::uses_unauthenticated_metastore).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiveTls {
    Required,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveCatalog {
    pub uris: Vec<String>,
    pub tls: HiveTls,
}

/// Others are refused in this delivery, so there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JdbcEngine {
    Postgresql,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JdbcCatalog {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub tls: TlsPolicy,
    pub engine: JdbcEngine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IcebergCatalog {
    Rest(RestCatalog),
    Glue(GlueCatalog),
    Hive(HiveCatalog),
    Jdbc(JdbcCatalog),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Storage {
    pub region: String,
    /// An S3-compatible store.
    pub endpoint: Option<String>,
    pub path_style: bool,
    pub role_arn: Option<String>,
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcsStorage {
    pub project: Option<String>,
    /// An emulator: development only.
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdlsStorage {
    pub account: String,
    pub endpoint_suffix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageKind {
    S3(S3Storage),
    Gcs(GcsStorage),
    Adls(AdlsStorage),
}

impl StorageKind {
    /// The scheme every warehouse location of this storage has.
    const fn scheme(&self) -> &'static str {
        match self {
            Self::S3(_) => "s3",
            Self::Gcs(_) => "gs",
            Self::Adls(_) => "abfss",
        }
    }
}

/// How the storage credentials are obtained: from the storage secret, or vended by the catalog inside the sandboxed
/// attempt (they never reach the platform, FR-035-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageCredentials {
    Static,
    Vended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LakeStorage {
    pub kind: StorageKind,
    pub credentials: StorageCredentials,
}

/// Ceilings for one write. The effective value is the minimum of the platform's, the namespace's, the connection's
/// and the step's, and zero is not unlimited: a ceiling that is absent here is the next one up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LakeLimits {
    pub max_record_bytes: Option<u32>,
    pub max_records: Option<u32>,
    pub timeout_ms: Option<u32>,
}

impl LakeLimits {
    fn validate(self) -> Result<(), LakeConfigError> {
        let inside = |value: Option<u32>, low: u32, high: u32| {
            value.is_none_or(|v| (low..=high).contains(&v))
        };
        if inside(self.max_record_bytes, 1024, 2_147_483_648)
            && inside(self.max_records, 1, 100_000_000)
            && inside(self.timeout_ms, 1000, 3_600_000)
        {
            Ok(())
        } else {
            Err(LakeConfigError::InvalidLimits)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcebergConfig {
    pub catalog: IcebergCatalog,
    pub storage: LakeStorage,
    /// Every table, data, delete and metadata location must start with one of them (FR-035-004).
    pub warehouse_locations: Vec<String>,
    pub allowed_modes: Vec<WriteMode>,
    pub catalog_secret: Option<SecretName>,
    pub storage_secret: Option<SecretName>,
    pub egress_ceiling: EgressCeiling,
    pub limits: LakeLimits,
}

impl IcebergConfig {
    /// The modes of a connection that names none: `append` only.
    #[must_use]
    pub fn default_modes() -> Vec<WriteMode> {
        vec![WriteMode::Append]
    }

    pub fn validate(&self) -> Result<(), DatabaseError> {
        self.validate_lake().map_err(DatabaseError::Lake)
    }

    fn validate_lake(&self) -> Result<(), LakeConfigError> {
        self.catalog.validate()?;
        self.storage.validate()?;
        self.validate_secrets()?;
        self.validate_locations()?;
        let mut modes = self.allowed_modes.clone();
        modes.sort_unstable();
        modes.dedup();
        if modes.is_empty() || modes.len() != self.allowed_modes.len() {
            return Err(LakeConfigError::InvalidModes);
        }
        self.limits.validate()
    }

    /// Which secrets this connection needs, and which it must not have.
    fn validate_secrets(&self) -> Result<(), LakeConfigError> {
        match (&self.catalog, self.catalog_secret.is_some()) {
            (IcebergCatalog::Hive(_), true) => return Err(LakeConfigError::CatalogSecretForbidden),
            (IcebergCatalog::Hive(_), false) => {}
            (_, false) => return Err(LakeConfigError::CatalogSecretRequired),
            (_, true) => {}
        }
        match self.storage.credentials {
            StorageCredentials::Vended => {
                let delegates = matches!(
                    &self.catalog,
                    IcebergCatalog::Rest(rest) if rest.access_delegation == AccessDelegation::VendedCredentials
                );
                if !delegates {
                    return Err(LakeConfigError::VendedNeedsDelegation);
                }
                if self.storage_secret.is_some() {
                    return Err(LakeConfigError::StorageSecretWithVended);
                }
            }
            StorageCredentials::Static if self.storage_secret.is_none() => {
                return Err(LakeConfigError::StorageSecretRequired);
            }
            StorageCredentials::Static => {}
        }
        Ok(())
    }

    fn validate_locations(&self) -> Result<(), LakeConfigError> {
        let locations = &self.warehouse_locations;
        if locations.is_empty() || locations.len() > MAX_LOCATIONS {
            return Err(LakeConfigError::InvalidLocations);
        }
        let scheme = self.storage.kind.scheme();
        let mut seen = Vec::with_capacity(locations.len());
        for location in locations {
            let (found, rest) = location
                .split_once("://")
                .ok_or(LakeConfigError::InvalidLocations)?;
            if found != scheme {
                return if matches!(found, "s3" | "gs" | "abfss") {
                    Err(LakeConfigError::LocationSchemeMismatch)
                } else {
                    Err(LakeConfigError::InvalidLocations)
                };
            }
            if !valid_location(&self.storage.kind, location, rest) {
                return Err(LakeConfigError::InvalidLocations);
            }
            let normalized = location.trim_end_matches('/');
            if seen.contains(&normalized) {
                return Err(LakeConfigError::InvalidLocations);
            }
            seen.push(normalized);
        }
        Ok(())
    }
}

/// A location (the part after the scheme is `rest`) that cannot climb out of its prefix.
fn valid_location(kind: &StorageKind, location: &str, rest: &str) -> bool {
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if location.len() > MAX_LOCATION_BYTES || authority.is_empty() || !clean_path(rest) {
        return false;
    }
    match kind {
        // `container@account.dfs.core.windows.net`: the one place user information is part of the name
        StorageKind::Adls(_) => {
            let (container, account) = authority.split_once('@').unwrap_or_default();
            !container.is_empty()
                && !account.is_empty()
                && !container.contains(':')
                && !account.contains('@')
                && !path.contains('@')
        }
        StorageKind::S3(_) | StorageKind::Gcs(_) => !rest.contains('@'),
    }
}

/// No control character, space, backslash, query, fragment, encoded dot or separator, and no `.` or `..` segment.
fn clean_path(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    !text
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '?' | '#'))
        && !["%2e", "%2f", "%5c", "%00"]
            .iter()
            .any(|encoded| lower.contains(encoded))
        && text
            .split('/')
            .all(|segment| segment != "." && segment != "..")
}

fn bounded(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.chars().count()) && !text.chars().any(char::is_control)
}

fn bounded_optional(text: Option<&str>, max: usize) -> bool {
    text.is_none_or(|text| text.chars().count() <= max && !text.chars().any(char::is_control))
}

/// An `http(s)` address: the scheme, a host with an optional port, and a path. User information, a query and a fragment
/// are credentials that would be stored, logged and shown in the clear, so they are refused as such. Returns whether it
/// is plain `http`.
fn http_uri(uri: &str) -> Result<bool, LakeConfigError> {
    let (scheme, rest) = uri.split_once("://").ok_or(LakeConfigError::InvalidUri)?;
    let plain = match scheme {
        "https" => false,
        "http" => true,
        _ => return Err(LakeConfigError::InvalidUri),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if rest[..end].contains('@') || rest.contains('?') || rest.contains('#') {
        return Err(LakeConfigError::CredentialInUri);
    }
    if uri.len() > MAX_URI_BYTES || uri.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(LakeConfigError::InvalidUri);
    }
    let authority = &rest[..end];
    let (host, port) = match authority.strip_prefix('[') {
        Some(bracketed) => {
            let (inner, after) = bracketed
                .split_once(']')
                .ok_or(LakeConfigError::InvalidUri)?;
            (inner, after.strip_prefix(':'))
        }
        None => match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    let port_holds = port.is_none_or(|port| {
        (1..=5).contains(&port.len()) && port.parse::<u16>().is_ok_and(|p| p != 0)
    });
    if host.is_empty() || !valid_host(host) || !port_holds {
        return Err(LakeConfigError::InvalidUri);
    }
    Ok(plain)
}

/// `thrift://host:port`
fn hive_uri(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("thrift://") else {
        return false;
    };
    if rest
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '@' | '/' | '?' | '#'))
    {
        return false;
    }
    rest.rsplit_once(':').is_some_and(|(host, port)| {
        !host.is_empty()
            && valid_host(host)
            && (1..=5).contains(&port.len())
            && port.bytes().all(|b| b.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|p| p != 0)
    })
}

impl IcebergCatalog {
    fn validate(&self) -> Result<(), LakeConfigError> {
        let invalid = LakeConfigError::InvalidCatalog;
        match self {
            Self::Rest(rest) => {
                http_uri(&rest.uri)?;
                if !bounded(&rest.warehouse, 1024) {
                    return Err(invalid);
                }
                if let Some(prefix) = &rest.prefix
                    && (prefix.is_empty()
                        || prefix.len() > 255
                        || !prefix.bytes().all(|b| {
                            b.is_ascii_alphanumeric()
                                || matches!(b, b'.' | b'_' | b'~' | b'/' | b'-')
                        })
                        || prefix
                            .split('/')
                            .any(|segment| segment == "." || segment == ".."))
                {
                    return Err(invalid);
                }
                match &rest.auth {
                    RestAuth::OAuth2 { token_uri, scope } => {
                        http_uri(token_uri)?;
                        if !bounded_optional(scope.as_deref(), 1024) {
                            return Err(invalid);
                        }
                    }
                    RestAuth::Bearer => {}
                    RestAuth::SigV4 {
                        region,
                        signing_name,
                    } => {
                        if !bounded(region, 63) || !bounded(signing_name, 63) {
                            return Err(invalid);
                        }
                    }
                }
                Ok(())
            }
            Self::Glue(glue) => {
                if bounded(&glue.region, 63)
                    && bounded(&glue.catalog_id, 255)
                    && bounded_optional(glue.role_arn.as_deref(), 2048)
                    && bounded_optional(glue.external_id.as_deref(), 1224)
                {
                    Ok(())
                } else {
                    Err(invalid)
                }
            }
            Self::Hive(hive) => {
                if (1..=MAX_HIVE_URIS).contains(&hive.uris.len())
                    && hive.uris.iter().all(|uri| hive_uri(uri))
                {
                    Ok(())
                } else {
                    Err(invalid)
                }
            }
            Self::Jdbc(jdbc) => {
                if !valid_host(&jdbc.host)
                    || jdbc.port == 0
                    || jdbc.database.is_empty()
                    || jdbc.database.len() > 63
                    || jdbc.database.chars().any(char::is_control)
                    || jdbc.tls.validate().is_err()
                {
                    return Err(invalid);
                }
                Ok(())
            }
        }
    }
}

impl LakeStorage {
    fn validate(&self) -> Result<(), LakeConfigError> {
        let invalid = LakeConfigError::InvalidStorage;
        match &self.kind {
            StorageKind::S3(s3) => {
                if !bounded(&s3.region, 63)
                    || !bounded_optional(s3.role_arn.as_deref(), 2048)
                    || !bounded_optional(s3.external_id.as_deref(), 1224)
                {
                    return Err(invalid);
                }
                if let Some(endpoint) = &s3.endpoint {
                    http_uri(endpoint)?;
                }
            }
            StorageKind::Gcs(gcs) => {
                if !bounded_optional(gcs.project.as_deref(), 255) {
                    return Err(invalid);
                }
                if let Some(endpoint) = &gcs.endpoint {
                    http_uri(endpoint)?;
                }
            }
            StorageKind::Adls(adls) => {
                let account = adls.account.as_bytes();
                if !(3..=24).contains(&account.len())
                    || !account
                        .iter()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                    || !bounded_optional(adls.endpoint_suffix.as_deref(), 255)
                {
                    return Err(invalid);
                }
            }
        }
        Ok(())
    }
}

impl IcebergConfig {
    /// Whether this connection reaches something without transport security, which is allowed in a development
    /// namespace only: a catalog or storage endpoint over plain `http`, a GCS emulator, a plaintext JDBC catalog.
    pub(super) fn requires_development(&self) -> bool {
        let plain = |uri: &str| http_uri(uri).unwrap_or(false);
        let catalog = match &self.catalog {
            IcebergCatalog::Rest(rest) => {
                plain(&rest.uri)
                    || matches!(&rest.auth, RestAuth::OAuth2 { token_uri, .. } if plain(token_uri))
            }
            IcebergCatalog::Jdbc(jdbc) => jdbc.tls == TlsPolicy::DevelopmentPlaintext,
            IcebergCatalog::Glue(_) | IcebergCatalog::Hive(_) => false,
        };
        let storage = match &self.storage.kind {
            StorageKind::S3(s3) => s3.endpoint.as_deref().is_some_and(plain),
            StorageKind::Gcs(gcs) => gcs.endpoint.is_some(),
            StorageKind::Adls(_) => false,
        };
        catalog || storage
    }

    pub(super) fn uses_unauthenticated_metastore(&self) -> bool {
        matches!(&self.catalog, IcebergCatalog::Hive(hive) if hive.tls == HiveTls::None)
    }
}

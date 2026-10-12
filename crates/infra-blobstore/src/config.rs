//! Reading and validating the blob store settings.
//!
//! Everything comes from `SAGAT_BLOB_*` variables, through a lookup function so tests need no
//! process environment. An empty value counts as unset (a compose file passes empty defaults). No
//! error ever repeats a value it was given: the wrong variable may hold a secret.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use sagat_application::blobs::{BlobLimits, DEFAULT_MAX_PENDING_BLOBS};

const MIB: usize = 1024 * 1024;
/// Largest blob an operator may allow: 5 GiB, what a single S3 `PutObject` could carry.
const MAX_BLOB_BYTES_CEILING: u64 = 5 * 1024 * 1024 * 1024;

/// A value that must never be printed.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one call that signs a request with it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: Secret,
    pub session_token: Option<Secret>,
}

/// Amazon S3, or any store that speaks its protocol at an endpoint of its own.
#[derive(Debug, Clone)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    /// Another S3-compatible service (MinIO, Ceph, a regional gateway). `None` is Amazon.
    pub endpoint: Option<String>,
    /// `None` uses the AWS credential chain: the environment, a web identity token (IRSA), the
    /// container or instance role.
    pub credentials: Option<Credentials>,
    /// `https://host/bucket/key` instead of `https://bucket.host/key`.
    pub path_style: bool,
    /// Allow an `http://` endpoint. Development only.
    pub allow_http: bool,
    /// Prepended to every key, so deployments can share a bucket.
    pub prefix: Option<String>,
}

/// Where Cloudflare keeps the data of an account, when it is pinned to a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jurisdiction {
    Eu,
    Fedramp,
}

/// Cloudflare R2.
#[derive(Debug, Clone)]
pub struct R2Config {
    pub account_id: String,
    pub bucket: String,
    pub credentials: Credentials,
    pub jurisdiction: Option<Jurisdiction>,
    /// Only a test points R2 somewhere else; production derives the endpoint from the account.
    pub endpoint_override: Option<String>,
    pub prefix: Option<String>,
}

impl R2Config {
    /// `https://<account>.r2.cloudflarestorage.com`, with the jurisdiction between the two when set.
    #[must_use]
    pub fn endpoint(&self) -> String {
        if let Some(endpoint) = &self.endpoint_override {
            return endpoint.clone();
        }
        let jurisdiction = match self.jurisdiction {
            None => "",
            Some(Jurisdiction::Eu) => "eu.",
            Some(Jurisdiction::Fedramp) => "fedramp.",
        };
        format!(
            "https://{}.{jurisdiction}r2.cloudflarestorage.com",
            self.account_id
        )
    }
}

#[derive(Debug, Clone)]
pub enum BlobStoreConfig {
    S3(S3Config),
    R2(R2Config),
    /// Lost on restart and private to one process: tests and a single-process development setup.
    Memory,
    /// A directory. With a volume shared by the control plane and the engine it is a complete
    /// development store.
    Fs {
        root: PathBuf,
    },
}

/// Size of one part of a multipart upload. Providers refuse parts under 5 MiB (except the last).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartSize(usize);

impl PartSize {
    pub const MIN: usize = 5 * MIB;
    pub const MAX: usize = 512 * MIB;

    #[must_use]
    pub const fn new(bytes: usize) -> Option<Self> {
        if bytes >= Self::MIN && bytes <= Self::MAX {
            Some(Self(bytes))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        self.0
    }
}

impl Default for PartSize {
    /// 8 MiB: a blob up to this size is one request.
    fn default() -> Self {
        Self(8 * MIB)
    }
}

/// Everything the processes need to use blobs, or nothing if they are off.
#[derive(Debug, Clone)]
pub struct BlobSettings {
    pub store: BlobStoreConfig,
    pub limits: BlobLimits,
    pub part_size: PartSize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{0}")]
    Incomplete(&'static str),
    #[error("{name} is not acceptable: {reason}")]
    Invalid {
        name: &'static str,
        reason: &'static str,
    },
    #[error(
        "an http:// endpoint needs SAGAT_BLOB_S3_ALLOW_HTTP=true, which is for development only"
    )]
    InsecureEndpoint,
    #[error("the object store could not be set up")]
    Build,
}

impl BlobSettings {
    /// Reads the settings. `Ok(None)` means blobs are off, which is the default.
    ///
    /// # Errors
    /// [`ConfigError`] naming the variable that is missing or unacceptable.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, ConfigError> {
        let get = |name: &'static str| lookup(name).filter(|v| !v.is_empty());
        let require = |name: &'static str| get(name).ok_or(ConfigError::Missing(name));

        let provider = match get("SAGAT_BLOB_PROVIDER") {
            None => return Ok(None),
            Some(p) => p,
        };
        let prefix = get("SAGAT_BLOB_PREFIX")
            .map(|p| clean_prefix(&p))
            .transpose()?
            .flatten();

        let store = match provider.to_ascii_lowercase().as_str() {
            "none" => return Ok(None),
            "memory" => BlobStoreConfig::Memory,
            "fs" => BlobStoreConfig::Fs {
                root: PathBuf::from(require("SAGAT_BLOB_FS_ROOT")?),
            },
            "s3" => {
                let credentials = credentials_of(
                    get("SAGAT_BLOB_S3_ACCESS_KEY_ID"),
                    get("SAGAT_BLOB_S3_SECRET_ACCESS_KEY"),
                    get("SAGAT_BLOB_S3_SESSION_TOKEN"),
                    "SAGAT_BLOB_S3_ACCESS_KEY_ID and SAGAT_BLOB_S3_SECRET_ACCESS_KEY go together",
                )?;
                let allow_http = flag(get("SAGAT_BLOB_S3_ALLOW_HTTP"), "SAGAT_BLOB_S3_ALLOW_HTTP")?;
                let endpoint = get("SAGAT_BLOB_S3_ENDPOINT")
                    .map(|e| check_endpoint(&e, allow_http, "SAGAT_BLOB_S3_ENDPOINT"))
                    .transpose()?;
                BlobStoreConfig::S3(S3Config {
                    bucket: require("SAGAT_BLOB_S3_BUCKET")?,
                    region: require("SAGAT_BLOB_S3_REGION")?,
                    endpoint,
                    credentials,
                    path_style: flag(get("SAGAT_BLOB_S3_PATH_STYLE"), "SAGAT_BLOB_S3_PATH_STYLE")?,
                    allow_http,
                    prefix: prefix.clone(),
                })
            }
            "r2" => {
                let account_id = require("SAGAT_BLOB_R2_ACCOUNT_ID")?;
                // The account id becomes part of a host name: only what a host label may hold.
                if account_id.len() > 64
                    || !account_id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
                {
                    return Err(ConfigError::Invalid {
                        name: "SAGAT_BLOB_R2_ACCOUNT_ID",
                        reason: "lower-case letters and digits only, at most 64",
                    });
                }
                let jurisdiction = match get("SAGAT_BLOB_R2_JURISDICTION").as_deref() {
                    None => None,
                    Some("eu") => Some(Jurisdiction::Eu),
                    Some("fedramp") => Some(Jurisdiction::Fedramp),
                    Some(_) => {
                        return Err(ConfigError::Invalid {
                            name: "SAGAT_BLOB_R2_JURISDICTION",
                            reason: "expected eu or fedramp, or leave it unset",
                        });
                    }
                };
                let credentials = credentials_of(
                    Some(require("SAGAT_BLOB_R2_ACCESS_KEY_ID")?),
                    Some(require("SAGAT_BLOB_R2_SECRET_ACCESS_KEY")?),
                    None,
                    "unreachable",
                )?
                .ok_or(ConfigError::Missing("SAGAT_BLOB_R2_ACCESS_KEY_ID"))?;
                BlobStoreConfig::R2(R2Config {
                    account_id,
                    bucket: require("SAGAT_BLOB_R2_BUCKET")?,
                    credentials,
                    jurisdiction,
                    endpoint_override: None,
                    prefix: prefix.clone(),
                })
            }
            _ => {
                return Err(ConfigError::Invalid {
                    name: "SAGAT_BLOB_PROVIDER",
                    reason: "expected one of: none, s3, r2, fs, memory",
                });
            }
        };

        let defaults = BlobLimits::default();
        let max_bytes = number(
            get("SAGAT_BLOB_MAX_BYTES"),
            "SAGAT_BLOB_MAX_BYTES",
            defaults.max_bytes,
            1,
            MAX_BLOB_BYTES_CEILING,
        )?;
        let max_pending = number(
            get("SAGAT_BLOB_MAX_PENDING"),
            "SAGAT_BLOB_MAX_PENDING",
            u64::from(DEFAULT_MAX_PENDING_BLOBS),
            1,
            100_000,
        )?;
        let ttl_secs = number(
            get("SAGAT_BLOB_UPLOAD_TTL_SECS"),
            "SAGAT_BLOB_UPLOAD_TTL_SECS",
            defaults.upload_ttl.as_secs(),
            1,
            365 * 24 * 3600,
        )?;
        let abandoned_secs = number(
            get("SAGAT_BLOB_ABANDONED_AFTER_SECS"),
            "SAGAT_BLOB_ABANDONED_AFTER_SECS",
            defaults.abandoned_after.as_secs(),
            1,
            365 * 24 * 3600,
        )?;
        let part_mib = number(
            get("SAGAT_BLOB_PART_SIZE_MIB"),
            "SAGAT_BLOB_PART_SIZE_MIB",
            (PartSize::default().bytes() / MIB) as u64,
            (PartSize::MIN / MIB) as u64,
            (PartSize::MAX / MIB) as u64,
        )?;
        let part_size = usize::try_from(part_mib)
            .ok()
            .and_then(|mib| PartSize::new(mib * MIB))
            .ok_or(ConfigError::Invalid {
                name: "SAGAT_BLOB_PART_SIZE_MIB",
                reason: "out of range",
            })?;

        Ok(Some(Self {
            store,
            limits: BlobLimits {
                max_bytes,
                max_pending: u32::try_from(max_pending).unwrap_or(u32::MAX),
                upload_ttl: Duration::from_secs(ttl_secs),
                abandoned_after: Duration::from_secs(abandoned_secs),
            },
            part_size,
        }))
    }
}

fn number(
    value: Option<String>,
    name: &'static str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, ConfigError> {
    let Some(value) = value else {
        return Ok(default);
    };
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .ok_or(ConfigError::Invalid {
            name,
            reason: "a whole number inside the allowed range",
        })
}

fn flag(value: Option<String>, name: &'static str) -> Result<bool, ConfigError> {
    match value.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("false" | "0") => Ok(false),
        Some("true" | "1") => Ok(true),
        Some(_) => Err(ConfigError::Invalid {
            name,
            reason: "expected true or false",
        }),
    }
}

fn credentials_of(
    key_id: Option<String>,
    secret: Option<String>,
    token: Option<String>,
    together: &'static str,
) -> Result<Option<Credentials>, ConfigError> {
    match (key_id, secret) {
        (None, None) => Ok(None),
        (Some(access_key_id), Some(secret)) => Ok(Some(Credentials {
            access_key_id,
            secret_access_key: Secret::new(secret),
            session_token: token.map(Secret::new),
        })),
        _ => Err(ConfigError::Incomplete(together)),
    }
}

fn check_endpoint(
    endpoint: &str,
    allow_http: bool,
    name: &'static str,
) -> Result<String, ConfigError> {
    let invalid = ConfigError::Invalid {
        name,
        reason: "expected an http(s) URL without credentials",
    };
    let url = reqwest::Url::parse(endpoint).map_err(|_| invalid.clone())?;
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err(invalid);
    }
    match url.scheme() {
        "https" => {}
        "http" if allow_http => {}
        "http" => return Err(ConfigError::InsecureEndpoint),
        _ => return Err(invalid),
    }
    Ok(endpoint.trim_end_matches('/').to_owned())
}

/// A prefix is a relative path of plain segments: no leading or trailing slash, no `..`.
fn clean_prefix(raw: &str) -> Result<Option<String>, ConfigError> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(None);
    }
    let plain = trimmed.split('/').all(|s| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    });
    if plain {
        Ok(Some(trimmed.to_owned()))
    } else {
        Err(ConfigError::Invalid {
            name: "SAGAT_BLOB_PREFIX",
            reason: "plain path segments of letters, digits, dot, dash and underscore",
        })
    }
}

//! Process configuration, read from environment variables (twelve-factor).
//!
//! The lookup is injected, so the parser is tested without touching the real environment. Nothing
//! here ever formats the database URL: it carries credentials.

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use sagat_api_http::{AuthConfig, HttpLimits, RateLimitConfig, RateLimitConfigError, ServerConfig};
use sagat_telemetry::LogFormat;

/// Where the process runs. Anything that could damage real data is gated on this; the default is
/// the safe one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Dev,
    Staging,
    Prod,
}

impl FromStr for Environment {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "dev" => Ok(Self::Dev),
            "staging" => Ok(Self::Staging),
            "prod" => Ok(Self::Prod),
            other => Err(format!("'{other}' is not one of dev, staging, prod")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable {0}")]
    Missing(&'static str),
    #[error("invalid value for {name}: {reason}")]
    Invalid { name: &'static str, reason: String },
}

impl From<sagat_telemetry::OtlpConfigError> for ConfigError {
    fn from(error: sagat_telemetry::OtlpConfigError) -> Self {
        Self::Invalid {
            name: error.name,
            reason: error.reason,
        }
    }
}

/// What a process installs for logs and spans: the log format, the name it goes by, and the exporter
/// the standard `OTEL_*` variables ask for (none by default).
///
/// # Errors
/// [`ConfigError::Invalid`] for an exporter setting that cannot be honoured.
pub fn telemetry(
    get: Lookup<'_>,
    service: &str,
    format: LogFormat,
) -> Result<sagat_telemetry::TelemetryConfig, ConfigError> {
    Ok(sagat_telemetry::TelemetryConfig {
        format,
        service: service.to_owned(),
        otlp: sagat_telemetry::OtlpConfig::from_lookup(get, service)?,
    })
}

/// How long a process waits at exit for the spans still queued to be sent.
pub const TELEMETRY_FLUSH: std::time::Duration = std::time::Duration::from_secs(5);

/// A way to read variables; `std::env::var(..).ok()` in production, a map in tests.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// A variable that must be set and non-blank.
///
/// # Errors
/// [`ConfigError::Missing`] otherwise.
pub fn required(get: Lookup<'_>, name: &'static str) -> Result<String, ConfigError> {
    get(name)
        .filter(|v| !v.trim().is_empty())
        .ok_or(ConfigError::Missing(name))
}

/// A variable parsed as `T`, or `default` when unset or blank.
///
/// # Errors
/// [`ConfigError::Invalid`] when it is set but does not parse.
pub fn optional<T: FromStr>(
    get: Lookup<'_>,
    name: &'static str,
    default: T,
) -> Result<T, ConfigError>
where
    T::Err: fmt::Display,
{
    match get(name).filter(|v| !v.trim().is_empty()) {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse()
            .map_err(|e: T::Err| ConfigError::Invalid {
                name,
                reason: e.to_string(),
            }),
    }
}

/// A boolean variable (`true`/`false`/`yes`/`no`/`1`/`0`); unset is `false`.
///
/// # Errors
/// [`ConfigError::Invalid`] for anything else.
pub fn flag(get: Lookup<'_>, name: &'static str) -> Result<bool, ConfigError> {
    match get(name).as_deref().map(str::trim) {
        None | Some("") => Ok(false),
        Some("1" | "true" | "yes") => Ok(true),
        Some("0" | "false" | "no") => Ok(false),
        Some(other) => Err(ConfigError::Invalid {
            name,
            reason: format!("'{other}' is not a boolean (use true or false)"),
        }),
    }
}

/// Variables of the per-tenant request budget: (requests per second, burst).
pub const TENANT_RATE_VARS: (&str, &str) = ("SAGAT_RATE_LIMIT_RPS", "SAGAT_RATE_LIMIT_BURST");
/// Variables of the per-subject request budget: (requests per second, burst).
pub const SUBJECT_RATE_VARS: (&str, &str) = (
    "SAGAT_SUBJECT_RATE_LIMIT_RPS",
    "SAGAT_SUBJECT_RATE_LIMIT_BURST",
);

/// Reads a request budget from a requests-per-second and a burst variable and validates it with the
/// same rules the limiter enforces (finite, rate at least 0.001, burst at least 1).
pub fn read_rate_limit(
    get: Lookup<'_>,
    (rps_var, burst_var): (&'static str, &'static str),
    defaults: RateLimitConfig,
) -> Result<RateLimitConfig, ConfigError> {
    let per_second: f64 = optional(get, rps_var, defaults.per_second)?;
    let burst: f64 = optional(get, burst_var, defaults.burst)?;
    RateLimitConfig { per_second, burst }
        .validate()
        .map_err(|e| rate_limit_error((rps_var, burst_var), e))
}

/// The configuration error for a refused budget, naming the variable that caused it.
#[must_use]
pub fn rate_limit_error(
    (rps_var, burst_var): (&'static str, &'static str),
    error: RateLimitConfigError,
) -> ConfigError {
    let name = match error {
        RateLimitConfigError::PerSecond => rps_var,
        RateLimitConfigError::Burst => burst_var,
    };
    ConfigError::Invalid {
        name,
        reason: error.to_string(),
    }
}

/// Database settings. `Debug` redacts the URL.
#[derive(Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub statement_timeout: Duration,
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("url", &"<redacted>")
            .field("max_connections", &self.max_connections)
            .field("statement_timeout", &self.statement_timeout)
            .finish()
    }
}

/// Settings every subcommand needs.
#[derive(Debug, Clone)]
pub struct Config {
    pub environment: Environment,
    pub log_format: LogFormat,
    pub database: DatabaseConfig,
}

impl Config {
    /// # Errors
    /// [`ConfigError`] naming the first missing or invalid variable.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        let environment = optional(get, "SAGAT_ENV", Environment::Prod)?;
        let log_format = match get("SAGAT_LOG_FORMAT").as_deref().map(str::trim) {
            None | Some("" | "json") => LogFormat::Json,
            Some("pretty") => LogFormat::Pretty,
            Some(other) => {
                return Err(ConfigError::Invalid {
                    name: "SAGAT_LOG_FORMAT",
                    reason: format!("'{other}' is not one of json, pretty"),
                });
            }
        };
        let max_connections: u32 = optional(get, "SAGAT_DB_MAX_CONNECTIONS", 10)?;
        if max_connections == 0 {
            return Err(ConfigError::Invalid {
                name: "SAGAT_DB_MAX_CONNECTIONS",
                reason: "must be at least 1".to_owned(),
            });
        }
        let statement_timeout_ms: u64 = optional(get, "SAGAT_DB_STATEMENT_TIMEOUT_MS", 5_000)?;
        let url = required(get, "DATABASE_URL")?;
        // sqlx defaults to `sslmode=prefer`, which silently falls back to plaintext and never
        // verifies the server. Outside development the connection must be encrypted.
        if environment != Environment::Dev && !requires_tls(&url) {
            return Err(ConfigError::Invalid {
                name: "DATABASE_URL",
                reason: "outside SAGAT_ENV=dev the database connection must require TLS \
                         (add sslmode=require, verify-ca or verify-full)"
                    .to_owned(),
            });
        }
        Ok(Self {
            environment,
            log_format,
            database: DatabaseConfig {
                url,
                max_connections,
                statement_timeout: Duration::from_millis(statement_timeout_ms),
            },
        })
    }
}

pub fn read_openbao(
    get: Lookup<'_>,
    environment: Environment,
) -> Result<Option<sagat_infra_secrets::openbao::OpenBaoSettings>, ConfigError> {
    sagat_infra_secrets::openbao::OpenBaoSettings::from_lookup(get, environment == Environment::Dev)
        .map_err(|_| ConfigError::Invalid {
            name: "SAGAT_OPENBAO_ADDRESS",
            reason: "invalid OpenBao transport, mount, token-file or development configuration"
                .to_owned(),
        })
}

/// The key ring that seals secrets, from `SAGAT_SECRETS_KEYS` (`id:base64,id2:base64`, the first
/// entry seals new values and every entry opens what it sealed). Absent means secrets are off.
/// The error never contains the value.
///
/// # Errors
/// [`ConfigError::Invalid`] when the value is not a usable key list.
pub fn read_secrets(
    get: Lookup<'_>,
) -> Result<Option<sagat_infra_secrets::SecretsConfig>, ConfigError> {
    match get("SAGAT_SECRETS_KEYS").map(|v| v.trim().to_owned()) {
        None => Ok(None),
        Some(spec) if spec.is_empty() => Ok(None),
        Some(spec) => sagat_infra_secrets::SecretsConfig::parse(&spec)
            .map(Some)
            .map_err(|e| ConfigError::Invalid {
                name: "SAGAT_SECRETS_KEYS",
                reason: e.to_string(),
            }),
    }
}

/// What serving scripts needs: the key that signs artifacts and the most a script may be given.
#[derive(Clone)]
pub struct ScriptsConfig {
    /// The private key, PKCS#8. Never printed.
    signing_key: Vec<u8>,
    pub ceiling: sagat_domain::scripts::Policy,
}

impl ScriptsConfig {
    #[must_use]
    pub fn new(signing_key: Vec<u8>, ceiling: sagat_domain::scripts::Policy) -> Self {
        Self {
            signing_key,
            ceiling,
        }
    }

    /// The key that signs artifacts.
    ///
    /// # Errors
    /// [`ConfigError::Invalid`] if the stored bytes are not a key (they were checked when read).
    pub fn signer(&self) -> Result<sagat_application::script_signing::Signer, ConfigError> {
        sagat_application::script_signing::Signer::from_pkcs8(&self.signing_key).map_err(|_| {
            ConfigError::Invalid {
                name: "SAGAT_SCRIPT_SIGNING_KEY",
                reason: "not an Ed25519 key in PKCS#8 form".to_owned(),
            }
        })
    }
}

impl std::fmt::Debug for ScriptsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key is not shown, nor anything derived from it.
        f.debug_struct("ScriptsConfig")
            .field("ceiling", &self.ceiling)
            .finish_non_exhaustive()
    }
}

/// The script settings, from `SAGAT_SCRIPT_SIGNING_KEY` and `SAGAT_SCRIPT_CEILING`. Absent key
/// means scripts are off: without a signature an artifact cannot be trusted. The error never
/// contains the value.
///
/// # Errors
/// [`ConfigError::Invalid`] when the key or the ceiling is not usable.
pub fn read_scripts(get: Lookup<'_>) -> Result<Option<ScriptsConfig>, ConfigError> {
    use base64::Engine as _;
    let ceiling = match get("SAGAT_SCRIPT_CEILING").map(|v| v.trim().to_owned()) {
        None => sagat_domain::scripts::Policy::default(),
        Some(text) if text.is_empty() => sagat_domain::scripts::Policy::default(),
        Some(text) => sagat_domain::scripts::Policy::parse_canonical(&text).map_err(|e| {
            ConfigError::Invalid {
                name: "SAGAT_SCRIPT_CEILING",
                reason: e.to_string(),
            }
        })?,
    };
    let Some(key) = get("SAGAT_SCRIPT_SIGNING_KEY")
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    let invalid = || ConfigError::Invalid {
        name: "SAGAT_SCRIPT_SIGNING_KEY",
        reason: "not an Ed25519 key in PKCS#8 form, base64 encoded".to_owned(),
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(key)
        .map_err(|_| invalid())?;
    sagat_application::script_signing::Signer::from_pkcs8(&bytes).map_err(|_| invalid())?;
    Ok(Some(ScriptsConfig {
        signing_key: bytes,
        ceiling,
    }))
}

/// How tokens are verified, from the `SAGAT_OIDC_*` variables. Every process that authenticates
/// callers (the API and the worker protocol) reads it the same way.
///
/// # Errors
/// [`ConfigError`] naming the first missing or invalid variable.
pub fn read_auth(get: Lookup<'_>) -> Result<AuthConfig, ConfigError> {
    let mut auth = AuthConfig::new(
        required(get, "SAGAT_OIDC_ISSUER")?,
        required(get, "SAGAT_OIDC_AUDIENCE")?,
        required(get, "SAGAT_OIDC_JWKS_URI")?,
    );
    auth.tenant_claim = optional(get, "SAGAT_OIDC_TENANT_CLAIM", auth.tenant_claim)?;
    auth.allow_insecure_jwks_http = flag(get, "SAGAT_ALLOW_INSECURE_JWKS_HTTP")?;
    auth.required_token_type = get("SAGAT_OIDC_REQUIRED_TYP")
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    Ok(auth)
}

/// Extra settings for `serve`.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// Opt in after every outbound process has been upgraded or stopped.
    pub redis_callbacks: bool,
    /// Whether the schedule routes exist (`SAGAT_SCHEDULES_ENABLED`, off by default until the
    /// feature is released). The engine fires schedules under the same flag.
    pub schedules_enabled: bool,
    /// Native TLS identity is required for the mTLS webhook profile; no forwarded headers are trusted.
    pub tls: Option<sagat_api_http::HttpTlsConfig>,
    pub webhook_oidc: Vec<sagat_api_http::auth::WebhookOidcConfig>,
    pub webhook_oidc_development: bool,
    pub bind_addr: SocketAddr,
    pub auth: AuthConfig,
    pub limits: HttpLimits,
    /// Connection-level protections (slow clients, connection cap, shutdown grace).
    pub server: ServerConfig,
    /// Per-tenant request budget, charged after authentication.
    pub rate_limit: RateLimitConfig,
    /// Per-subject request budget, charged right after the token verifies and before any database
    /// lookup.
    pub subject_rate_limit: RateLimitConfig,
    /// Where `/metrics` is served, on a listener separate from the public API. `None` disables it.
    pub admin_addr: Option<SocketAddr>,
    /// Apply pending migrations before listening. Off by default: production runs them as a
    /// separate, privileged step (`control-plane migrate`).
    pub migrate_on_start: bool,
    /// The key ring that seals secrets (`SAGAT_SECRETS_KEYS`); without it the secret routes do not
    /// exist.
    pub secrets: Option<sagat_infra_secrets::SecretsConfig>,
    pub openbao: Option<sagat_infra_secrets::openbao::OpenBaoSettings>,
    /// The key that signs artifacts and the most a script may be given; without a key the script
    /// routes do not exist.
    pub scripts: Option<ScriptsConfig>,
    /// The directory of the Studio build and how the browser signs in; without a directory the
    /// Studio is not served (`SAGAT_STUDIO_DIR`).
    pub studio: Option<sagat_api_http::studio::StudioConfig>,
    /// How many server-sent streams may be open (`SAGAT_STREAMS_*`).
    pub streams: sagat_api_http::streams::StreamLimits,
    /// The URL or path of the alerts runbook, to which each alert's anchor is appended
    /// (`SAGAT_RUNBOOK_BASE_URL`).
    pub runbook_base: String,
    /// How late a run's spans may be before its telemetry is called behind
    /// (`SAGAT_TELEMETRY_LAG_BUDGET_SECS`, default 30).
    pub telemetry_lag_budget: Duration,
    pub blobs: Option<sagat_infra_blobstore::BlobSettings>,
    pub blob_transfer: sagat_api_http::HttpBlobTransfer,
}

impl ServeConfig {
    /// # Errors
    /// [`ConfigError`] naming the first missing or invalid variable.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        let auth = read_auth(get)?;
        // The API takes secret values in request bodies and listens in plaintext: TLS is the
        // ingress's job. Outside development the operator has to say that it is there, as the
        // engine's operator does for the worker API.
        let secrets = read_secrets(get)?;
        let environment: Environment = optional(get, "SAGAT_ENV", Environment::Prod)?;
        let openbao = read_openbao(get, environment)?;

        let tls = read_http_tls(get)?;
        let webhook_oidc = read_webhook_oidc(get, environment == Environment::Dev)?;
        if (secrets.is_some() || openbao.is_some())
            && tls.is_none()
            && environment != Environment::Dev
            && !flag(get, "SAGAT_API_ALLOW_PLAINTEXT_SECRETS")?
        {
            return Err(ConfigError::Invalid {
                name: "SAGAT_API_ALLOW_PLAINTEXT_SECRETS",
                reason: "secret storage is configured, so this API receives secret values, and it \
                         listens in plaintext; set this to true to state that TLS is terminated \
                         in front of it"
                    .to_owned(),
            });
        }
        let defaults = HttpLimits::default();
        let max_body_bytes: usize = optional(get, "SAGAT_MAX_BODY_BYTES", defaults.max_body_bytes)?;
        if max_body_bytes == 0 {
            return Err(ConfigError::Invalid {
                name: "SAGAT_MAX_BODY_BYTES",
                reason: "must be greater than zero".to_owned(),
            });
        }
        let timeout_secs: u64 = optional(
            get,
            "SAGAT_REQUEST_TIMEOUT_SECS",
            defaults.request_timeout.as_secs(),
        )?;
        if timeout_secs == 0 {
            return Err(ConfigError::Invalid {
                name: "SAGAT_REQUEST_TIMEOUT_SECS",
                reason: "must be greater than zero".to_owned(),
            });
        }
        let max_in_flight: usize = optional(get, "SAGAT_MAX_IN_FLIGHT", defaults.max_in_flight)?;
        if max_in_flight == 0 {
            return Err(ConfigError::Invalid {
                name: "SAGAT_MAX_IN_FLIGHT",
                reason: "must be greater than zero".to_owned(),
            });
        }
        // A quarter of the global cap unless set: one tenant cannot hold every slot.
        let max_in_flight_per_tenant: usize = optional(
            get,
            "SAGAT_MAX_IN_FLIGHT_PER_TENANT",
            HttpLimits::default_per_tenant(max_in_flight),
        )?;
        if max_in_flight_per_tenant == 0 {
            return Err(ConfigError::Invalid {
                name: "SAGAT_MAX_IN_FLIGHT_PER_TENANT",
                reason: "must be greater than zero".to_owned(),
            });
        }
        let body_idle_secs: u64 = optional(
            get,
            "SAGAT_BODY_IDLE_TIMEOUT_SECS",
            defaults.body_idle_timeout.as_secs(),
        )?;
        if body_idle_secs == 0 {
            return Err(ConfigError::Invalid {
                name: "SAGAT_BODY_IDLE_TIMEOUT_SECS",
                reason: "must be greater than zero".to_owned(),
            });
        }
        let rate_limit = read_rate_limit(get, TENANT_RATE_VARS, RateLimitConfig::default())?;
        let subject_rate_limit =
            read_rate_limit(get, SUBJECT_RATE_VARS, RateLimitConfig::subject_default())?;
        let admin_addr =
            match get("SAGAT_ADMIN_ADDR").as_deref().map(str::trim) {
                None | Some("") => Some(SocketAddr::from(([127, 0, 0, 1], 9090))),
                Some("off") => None,
                Some(raw) => Some(raw.parse().map_err(|e: std::net::AddrParseError| {
                    ConfigError::Invalid {
                        name: "SAGAT_ADMIN_ADDR",
                        reason: e.to_string(),
                    }
                })?),
            };
        let server_defaults = ServerConfig::default();
        let header_secs: u64 = optional(
            get,
            "SAGAT_HEADER_READ_TIMEOUT_SECS",
            server_defaults.header_read_timeout.as_secs(),
        )?;
        let max_connections: usize = optional(
            get,
            "SAGAT_MAX_CONNECTIONS",
            server_defaults.max_connections,
        )?;
        let grace_secs: u64 = optional(
            get,
            "SAGAT_SHUTDOWN_GRACE_SECS",
            server_defaults.shutdown_grace.as_secs(),
        )?;
        let stall_secs: u64 = optional(
            get,
            "SAGAT_WRITE_STALL_TIMEOUT_SECS",
            server_defaults.write_stall_timeout.as_secs(),
        )?;
        for (name, value, ceiling) in [
            ("SAGAT_HEADER_READ_TIMEOUT_SECS", header_secs, 86400),
            ("SAGAT_SHUTDOWN_GRACE_SECS", grace_secs, 86400),
            ("SAGAT_WRITE_STALL_TIMEOUT_SECS", stall_secs, 86400),
            (
                "SAGAT_MAX_CONNECTIONS",
                max_connections as u64,
                tokio::sync::Semaphore::MAX_PERMITS.min(u32::MAX as usize) as u64,
            ),
        ] {
            if value == 0 || value > ceiling {
                return Err(ConfigError::Invalid {
                    name,
                    reason: format!("must be between 1 and {ceiling}"),
                });
            }
        }
        let streams = read_streams(get, max_connections)?;
        let blobs = read_blobs(get, environment)?;
        let blob_defaults = sagat_api_http::HttpBlobTransfer::default();
        let blob_max: usize = optional(
            get,
            "SAGAT_BLOB_MAX_TRANSFERS",
            blob_defaults.max_concurrent,
        )?;
        let blob_timeout: u64 = optional(
            get,
            "SAGAT_BLOB_TRANSFER_TIMEOUT_SECS",
            blob_defaults.timeout.as_secs(),
        )?;
        for (name, value, ceiling) in [
            ("SAGAT_BLOB_MAX_TRANSFERS", blob_max as u64, 4096),
            ("SAGAT_BLOB_TRANSFER_TIMEOUT_SECS", blob_timeout, 86400),
        ] {
            if value == 0 || value > ceiling {
                return Err(ConfigError::Invalid {
                    name,
                    reason: format!("must be between 1 and {ceiling}"),
                });
            }
        }
        Ok(Self {
            redis_callbacks: flag(get, "SAGAT_REDIS_CALLBACKS_ENABLED")?,
            schedules_enabled: flag(get, "SAGAT_SCHEDULES_ENABLED")?,
            tls,
            webhook_oidc,
            webhook_oidc_development: environment == Environment::Dev,
            bind_addr: optional(
                get,
                "SAGAT_BIND_ADDR",
                SocketAddr::from(([127, 0, 0, 1], 8080)),
            )?,
            auth,
            limits: HttpLimits {
                max_body_bytes,
                request_timeout: Duration::from_secs(timeout_secs),
                max_in_flight,
                max_in_flight_per_tenant,
                body_idle_timeout: Duration::from_secs(body_idle_secs),
            },
            rate_limit,
            subject_rate_limit,
            admin_addr,
            server: ServerConfig {
                header_read_timeout: Duration::from_secs(header_secs),
                max_connections,
                shutdown_grace: Duration::from_secs(grace_secs),
                write_stall_timeout: Duration::from_secs(stall_secs),
            },
            migrate_on_start: flag(get, "SAGAT_MIGRATE_ON_START")?,
            secrets,
            openbao,
            scripts: read_scripts(get)?,
            studio: read_studio(get)?,
            streams,
            blobs,
            blob_transfer: sagat_api_http::HttpBlobTransfer {
                max_concurrent: blob_max,
                timeout: Duration::from_secs(blob_timeout),
            },
            telemetry_lag_budget: Duration::from_secs(
                optional(get, "SAGAT_TELEMETRY_LAG_BUDGET_SECS", 30u64)?.max(1),
            ),
            runbook_base: get("SAGAT_RUNBOOK_BASE_URL")
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "docs/runbooks/alerts.md".to_owned()),
        })
    }
}

/// The limits of server-sent streams. A stream holds its connection for as long as it lives (fifteen
/// minutes at most), so the platform as a whole may not hold more of them than half of the connections
/// it accepts: the rest is for the API, the health probe and the Studio's files.
fn read_streams(
    get: &dyn Fn(&str) -> Option<String>,
    max_connections: usize,
) -> Result<sagat_api_http::streams::StreamLimits, ConfigError> {
    let defaults = sagat_api_http::streams::StreamLimits::default();
    let ceiling = (max_connections / 2).max(1);
    let global: usize = optional(get, "SAGAT_STREAMS_MAX", defaults.global.min(ceiling))?;
    let per_tenant: usize = optional(get, "SAGAT_STREAMS_PER_TENANT", defaults.per_tenant)?;
    let per_subject: usize = optional(get, "SAGAT_STREAMS_PER_SUBJECT", defaults.per_subject)?;
    for (name, value) in [
        ("SAGAT_STREAMS_MAX", global),
        ("SAGAT_STREAMS_PER_TENANT", per_tenant),
        ("SAGAT_STREAMS_PER_SUBJECT", per_subject),
    ] {
        if value == 0 {
            return Err(ConfigError::Invalid {
                name,
                reason: "must be greater than zero".to_owned(),
            });
        }
    }
    if global > ceiling {
        return Err(ConfigError::Invalid {
            name: "SAGAT_STREAMS_MAX",
            reason: format!(
                "at most half of SAGAT_MAX_CONNECTIONS ({ceiling}): an open stream holds its connection, and the API, the health probe and the Studio's files need the rest"
            ),
        });
    }
    Ok(sagat_api_http::streams::StreamLimits {
        global,
        per_tenant,
        per_subject,
        ..defaults
    })
}

/// The Studio, from `SAGAT_STUDIO_DIR`; its public sign-in settings; and the optional
/// `SAGAT_STUDIO_TRACE_URL` template. Browser-facing URLs require HTTPS (HTTP is accepted only on
/// loopback) and no credentials. Nothing here is secret: a public client has an id and no secret.
fn read_studio(
    get: Lookup<'_>,
) -> Result<Option<sagat_api_http::studio::StudioConfig>, ConfigError> {
    use sagat_api_http::studio::{StudioConfig, StudioOidc};
    let value = |name: &str| {
        get(name)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let issuer = value("SAGAT_STUDIO_OIDC_ISSUER");
    let client_id = value("SAGAT_STUDIO_OIDC_CLIENT_ID");
    let trace_url = value("SAGAT_STUDIO_TRACE_URL");
    if let Some(template) = &trace_url {
        sagat_api_http::studio::validate_trace_url_template(template).map_err(|_| {
            ConfigError::Invalid {
                name: "SAGAT_STUDIO_TRACE_URL",
                reason: "must be an absolute https URL (http only on a loopback address), contain exactly one {traceId} placeholder and have no credentials".to_owned(),
            }
        })?;
    }
    let oidc = match (issuer, client_id) {
        (None, None) => None,
        (Some(_), None) => {
            return Err(ConfigError::Missing("SAGAT_STUDIO_OIDC_CLIENT_ID"));
        }
        (None, Some(_)) => {
            return Err(ConfigError::Missing("SAGAT_STUDIO_OIDC_ISSUER"));
        }
        (Some(issuer), Some(client_id)) => {
            let invalid = |name, reason: &str| ConfigError::Invalid {
                name,
                reason: reason.to_owned(),
            };
            let https = issuer.starts_with("https://");
            let loopback = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
                .iter()
                .any(|prefix| {
                    issuer
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
                });
            let authority = issuer.split("://").nth(1).unwrap_or_default();
            let authority = authority.split(['/', '?', '#']).next().unwrap_or_default();
            if !(https || loopback)
                || authority.contains('@')
                || issuer.contains(char::is_whitespace)
            {
                return Err(invalid(
                    "SAGAT_STUDIO_OIDC_ISSUER",
                    "must be an https URL (http only on a loopback address) with no credentials",
                ));
            }
            if client_id.len() > 128
                || !client_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._~-:@".contains(&b))
            {
                return Err(invalid(
                    "SAGAT_STUDIO_OIDC_CLIENT_ID",
                    "must be 1 to 128 characters of letters, digits and . _ ~ - : @",
                ));
            }
            let scope =
                value("SAGAT_STUDIO_OIDC_SCOPE").unwrap_or_else(|| "openid profile".to_owned());
            if scope.len() > 256 || !scope.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
                return Err(invalid(
                    "SAGAT_STUDIO_OIDC_SCOPE",
                    "must be space-separated printable ASCII, at most 256 characters",
                ));
            }
            Some(StudioOidc {
                issuer,
                client_id,
                scope,
            })
        }
    };
    Ok(value("SAGAT_STUDIO_DIR").map(|dir| StudioConfig {
        dir: dir.into(),
        oidc,
        trace_url,
        limits: sagat_api_http::studio::StudioLimits::default(),
    }))
}

/// Settings for `seed-dev`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedConfig {
    pub tenant_slug: String,
    pub admin_subject: String,
}

impl SeedConfig {
    /// # Errors
    /// [`ConfigError::Invalid`] for an empty value.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        Ok(Self {
            tenant_slug: optional(get, "SAGAT_SEED_TENANT", "dev".to_owned())?,
            admin_subject: optional(get, "SAGAT_SEED_ADMIN_SUBJECT", "dev-admin".to_owned())?,
        })
    }
}

/// The workspace and the namespace `seed-dev` leaves for the development tenant, from
/// `SAGAT_SEED_NAMESPACE` (unset or empty: none), `SAGAT_SEED_WORKSPACE` and
/// `SAGAT_SEED_ENVIRONMENT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceSeed {
    pub workspace_slug: String,
    pub namespace_slug: String,
    pub environment: String,
}

impl NamespaceSeed {
    /// # Errors
    /// [`ConfigError::Invalid`] for an empty workspace or environment.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Option<Self>, ConfigError> {
        let Some(namespace_slug) = get("SAGAT_SEED_NAMESPACE")
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            workspace_slug: optional(get, "SAGAT_SEED_WORKSPACE", "dev".to_owned())?,
            namespace_slug,
            environment: optional(get, "SAGAT_SEED_ENVIRONMENT", "dev".to_owned())?,
        }))
    }
}

/// Whether the (unauthenticated) metrics listener can be reached from a network, that is, whether
/// it is bound to anything but a loopback address. IPv4-mapped IPv6 addresses count as the IPv4
/// address they carry.
#[must_use]
pub fn admin_addr_is_exposed(addr: SocketAddr) -> bool {
    !addr.ip().to_canonical().is_loopback()
}

/// Whether a PostgreSQL URL insists on TLS (`sslmode` / `ssl-mode` of `require`, `verify-ca` or
/// `verify-full`). `disable`, `allow`, `prefer` and an absent parameter do not.
#[must_use]
pub fn requires_tls(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else {
        return false;
    };
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .any(|(key, value)| {
            matches!(key, "sslmode" | "ssl-mode")
                && matches!(value, "require" | "verify-ca" | "verify-full")
        })
}

/// A URL that is safe to log: scheme, host, port and path only. Credentials (`user:password@`),
/// query strings (which may carry tokens) and fragments are dropped.
#[must_use]
pub fn redact_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "<unparseable url>".to_owned();
    };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if path.is_empty() {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}/{path}")
    }
}

/// Settings for `quota-ceiling`: the most a tenant's administrators may set for one quota. As with
/// provisioning there are no defaults; granting capacity is a deliberate act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CeilingConfig {
    pub tenant_id: String,
    pub resource: String,
    /// `None` withdraws the grant, so the platform default applies again.
    pub value: Option<u64>,
}

impl CeilingConfig {
    /// # Errors
    /// [`ConfigError::Missing`] when a value is absent; [`ConfigError::Invalid`] when the value is
    /// neither a number nor `default`.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        let raw = required(get, "SAGAT_CEILING_VALUE")?;
        let value = if raw.eq_ignore_ascii_case("default") {
            None
        } else {
            Some(raw.parse::<u64>().map_err(|_| ConfigError::Invalid {
                name: "SAGAT_CEILING_VALUE",
                reason: "expected a whole number or `default`".into(),
            })?)
        };
        Ok(Self {
            tenant_id: required(get, "SAGAT_CEILING_TENANT_ID")?,
            resource: required(get, "SAGAT_CEILING_RESOURCE")?,
            value,
        })
    }
}

/// Settings for `scheduling-weight`: how much of the scheduler's attention a tenant gets when others
/// have work due too (`FR-010-002`). As with quota ceilings there are no defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightConfig {
    pub tenant_id: String,
    /// `None` withdraws the weight, so the default (1) applies again.
    pub value: Option<u16>,
}

impl WeightConfig {
    /// # Errors
    /// [`ConfigError::Missing`] when a value is absent; [`ConfigError::Invalid`] when the value is
    /// neither a number from 1 to 16 nor `default`.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        let raw = required(get, "SAGAT_WEIGHT_VALUE")?;
        let value = if raw.eq_ignore_ascii_case("default") {
            None
        } else {
            let weight = raw
                .parse::<u16>()
                .ok()
                .filter(|w| (1..=sagat_domain::sharding::MAX_WEIGHT).contains(w));
            Some(weight.ok_or_else(|| ConfigError::Invalid {
                name: "SAGAT_WEIGHT_VALUE",
                reason: format!(
                    "expected a whole number from 1 to {} or `default`",
                    sagat_domain::sharding::MAX_WEIGHT
                ),
            })?)
        };
        Ok(Self {
            tenant_id: required(get, "SAGAT_WEIGHT_TENANT_ID")?,
            value,
        })
    }
}

/// Settings for `provision-tenant`. Unlike the development seed there are no defaults: creating a
/// real tenant must be a deliberate act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionConfig {
    pub tenant_slug: String,
    pub admin_subject: String,
}

impl ProvisionConfig {
    /// # Errors
    /// [`ConfigError::Missing`] when either value is absent.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        Ok(Self {
            tenant_slug: required(get, "SAGAT_PROVISION_TENANT")?,
            admin_subject: required(get, "SAGAT_PROVISION_ADMIN")?,
        })
    }
}

fn bounded_config_file(path: &str, name: &'static str) -> Result<Vec<u8>, ConfigError> {
    use std::io::Read as _;
    let invalid = || ConfigError::Invalid {
        name,
        reason: "must name a readable file of at most 256 KiB".to_owned(),
    };
    let file = std::fs::File::open(path).map_err(|_| invalid())?;
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() > 256 * 1024 {
        return Err(invalid());
    }
    Ok(bytes)
}

fn read_http_tls(get: Lookup<'_>) -> Result<Option<sagat_api_http::HttpTlsConfig>, ConfigError> {
    let names = [
        "SAGAT_HTTP_TLS_CERT",
        "SAGAT_HTTP_TLS_KEY",
        "SAGAT_HTTP_TLS_CLIENT_CA",
    ];
    if names.iter().all(|name| get(name).is_none()) {
        return Ok(None);
    }
    let mut values = Vec::new();
    for name in names {
        values.push(bounded_config_file(&required(get, name)?, name)?);
    }
    sagat_api_http::HttpTlsConfig::from_pem(&values[0], &values[1], &values[2])
        .map(Some)
        .map_err(|reason| ConfigError::Invalid {
            name: "SAGAT_HTTP_TLS_CERT",
            reason: reason.to_owned(),
        })
}

fn read_webhook_oidc(
    get: Lookup<'_>,
    development: bool,
) -> Result<Vec<sagat_api_http::auth::WebhookOidcConfig>, ConfigError> {
    const NAME: &str = "SAGAT_WEBHOOK_OIDC_PROFILES_FILE";
    let Some(path) = get(NAME) else {
        return Ok(Vec::new());
    };
    let bytes = bounded_config_file(&path, NAME)?;
    let profiles = serde_json::from_slice::<Vec<sagat_api_http::auth::WebhookOidcConfig>>(&bytes)
        .map_err(|_| ConfigError::Invalid {
        name: NAME,
        reason: "must contain a JSON array of webhook OIDC profiles".to_owned(),
    })?;
    sagat_api_http::auth::WebhookOidcVerifiers::new(&profiles, development).map_err(|reason| {
        ConfigError::Invalid {
            name: NAME,
            reason: reason.to_owned(),
        }
    })?;
    Ok(profiles)
}

/// The blob store settings, with the rules that depend on where the process runs.
fn read_blobs(
    get: Lookup<'_>,
    environment: Environment,
) -> Result<Option<sagat_infra_blobstore::BlobSettings>, ConfigError> {
    use sagat_infra_blobstore::{BlobSettings, BlobStoreConfig, ConfigError as BlobConfigError};
    let settings = BlobSettings::from_lookup(|name| get(name)).map_err(|error| match error {
        BlobConfigError::Missing(name) => ConfigError::Missing(name),
        BlobConfigError::Invalid { name, reason } => ConfigError::Invalid {
            name,
            reason: reason.to_owned(),
        },
        BlobConfigError::Incomplete(reason) => ConfigError::Invalid {
            name: "SAGAT_BLOB_S3_ACCESS_KEY_ID",
            reason: reason.to_owned(),
        },
        BlobConfigError::InsecureEndpoint => ConfigError::Invalid {
            name: "SAGAT_BLOB_S3_ENDPOINT",
            reason: "plain http needs SAGAT_BLOB_S3_ALLOW_HTTP=true".to_owned(),
        },
        BlobConfigError::Build => ConfigError::Invalid {
            name: "SAGAT_BLOB_PROVIDER",
            reason: "the object store could not be set up".to_owned(),
        },
    })?;
    if let Some(settings) = &settings {
        // Memory is private to one process, and the control plane and every engine replica are
        // different processes: a blob written by one could not be read by another, and a restart
        // would lose them all. It exists for tests and for a single-process development setup.
        if matches!(settings.store, BlobStoreConfig::Memory) {
            return Err(ConfigError::Invalid {
                name: "SAGAT_BLOB_PROVIDER",
                reason: "memory cannot share run inputs with engine processes; use fs or an object store".to_owned(),
            });
        }
        if let BlobStoreConfig::S3(s3) = &settings.store
            && s3.allow_http
            && environment != Environment::Dev
        {
            return Err(ConfigError::Invalid {
                name: "SAGAT_BLOB_S3_ALLOW_HTTP",
                reason: "plain http to the object store is only allowed when SAGAT_ENV=dev"
                    .to_owned(),
            });
        }
    }
    Ok(settings)
}

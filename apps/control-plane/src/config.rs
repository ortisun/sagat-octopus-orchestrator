//! Process configuration, read from environment variables (twelve-factor).
//!
//! The lookup is injected, so the parser is tested without touching the real environment. Nothing
//! here ever formats the database URL: it carries credentials.

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use sagat_api_http::{AuthConfig, HttpLimits, RateLimitConfig, ServerConfig};
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

/// A way to read variables; `std::env::var(..).ok()` in production, a map in tests.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

fn required(get: Lookup<'_>, name: &'static str) -> Result<String, ConfigError> {
    get(name)
        .filter(|v| !v.trim().is_empty())
        .ok_or(ConfigError::Missing(name))
}

fn optional<T: FromStr>(get: Lookup<'_>, name: &'static str, default: T) -> Result<T, ConfigError>
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

fn flag(get: Lookup<'_>, name: &'static str) -> Result<bool, ConfigError> {
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

/// Extra settings for `serve`.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub bind_addr: SocketAddr,
    pub auth: AuthConfig,
    pub limits: HttpLimits,
    /// Connection-level protections (slow clients, connection cap, shutdown grace).
    pub server: ServerConfig,
    /// Per-tenant request budget.
    pub rate_limit: RateLimitConfig,
    /// Where `/metrics` is served, on a listener separate from the public API. `None` disables it.
    pub admin_addr: Option<SocketAddr>,
    /// Apply pending migrations before listening. Off by default: production runs them as a
    /// separate, privileged step (`control-plane migrate`).
    pub migrate_on_start: bool,
}

impl ServeConfig {
    /// # Errors
    /// [`ConfigError`] naming the first missing or invalid variable.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        let mut auth = AuthConfig::new(
            required(get, "SAGAT_OIDC_ISSUER")?,
            required(get, "SAGAT_OIDC_AUDIENCE")?,
            required(get, "SAGAT_OIDC_JWKS_URI")?,
        );
        auth.tenant_claim = optional(get, "SAGAT_OIDC_TENANT_CLAIM", auth.tenant_claim)?;
        auth.allow_insecure_jwks_http = flag(get, "SAGAT_ALLOW_INSECURE_JWKS_HTTP")?;
        auth.required_token_type = get("SAGAT_OIDC_REQUIRED_TYP").filter(|v| !v.trim().is_empty());

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
        let rate_defaults = RateLimitConfig::default();
        let rate_per_second: f64 = optional(get, "SAGAT_RATE_LIMIT_RPS", rate_defaults.per_second)?;
        let rate_burst: f64 = optional(get, "SAGAT_RATE_LIMIT_BURST", rate_defaults.burst)?;
        if !(rate_per_second.is_finite() && rate_per_second > 0.0) {
            return Err(ConfigError::Invalid {
                name: "SAGAT_RATE_LIMIT_RPS",
                reason: "must be a positive number".to_owned(),
            });
        }
        if !(rate_burst.is_finite() && rate_burst >= 1.0) {
            return Err(ConfigError::Invalid {
                name: "SAGAT_RATE_LIMIT_BURST",
                reason: "must be at least 1".to_owned(),
            });
        }
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
        for (name, value) in [
            ("SAGAT_HEADER_READ_TIMEOUT_SECS", header_secs),
            ("SAGAT_SHUTDOWN_GRACE_SECS", grace_secs),
            ("SAGAT_MAX_CONNECTIONS", max_connections as u64),
        ] {
            if value == 0 {
                return Err(ConfigError::Invalid {
                    name,
                    reason: "must be greater than zero".to_owned(),
                });
            }
        }
        Ok(Self {
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
            },
            rate_limit: RateLimitConfig {
                per_second: rate_per_second,
                burst: rate_burst,
            },
            admin_addr,
            server: ServerConfig {
                header_read_timeout: Duration::from_secs(header_secs),
                max_connections,
                shutdown_grace: Duration::from_secs(grace_secs),
            },
            migrate_on_start: flag(get, "SAGAT_MIGRATE_ON_START")?,
        })
    }
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

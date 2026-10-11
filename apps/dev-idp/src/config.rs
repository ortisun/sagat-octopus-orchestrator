//! What the provider is told, and what it refuses to be told.

use std::net::SocketAddr;
use std::path::PathBuf;

/// A name the environment may give a value to.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error(
        "the development identity provider only runs when SAGAT_ENV=dev: it signs a token for anyone who asks"
    )]
    NotDevelopment,
    #[error("{name}: {reason}")]
    Invalid { name: &'static str, reason: String },
    #[error(
        "SAGAT_DEV_IDP_ADDR {0} is not a loopback address: set SAGAT_DEV_IDP_EXPOSED=true to say that only the machine's own stack can reach it"
    )]
    Exposed(SocketAddr),
}

/// The most a token may live: the platform refuses a longer one.
pub const MAX_TTL_SECS: u64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub addr: SocketAddr,
    /// The `iss` of every token, exactly as the platform's `SAGAT_OIDC_ISSUER` spells it.
    pub issuer: String,
    pub audience: String,
    /// The claim that carries the tenant.
    pub tenant_claim: String,
    /// Where the signing key is kept between runs; without it the key lives as long as the process.
    pub key_file: Option<PathBuf>,
}

fn value(get: Lookup<'_>, name: &str) -> Option<String> {
    get(name)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

impl Config {
    /// # Errors
    /// [`ConfigError`] when the environment is not `dev`, a value is not valid, or the address would
    /// expose the provider without having been told that is intended.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        if value(get, "SAGAT_ENV").as_deref() != Some("dev") {
            return Err(ConfigError::NotDevelopment);
        }
        let invalid = |name: &'static str, reason: &str| ConfigError::Invalid {
            name,
            reason: reason.to_owned(),
        };
        let addr: SocketAddr = value(get, "SAGAT_DEV_IDP_ADDR")
            .unwrap_or_else(|| "127.0.0.1:9100".to_owned())
            .parse()
            .map_err(|_| {
                invalid(
                    "SAGAT_DEV_IDP_ADDR",
                    "expected host:port, for example 127.0.0.1:9100",
                )
            })?;
        let exposed = value(get, "SAGAT_DEV_IDP_EXPOSED").as_deref() == Some("true");
        if !addr.ip().to_canonical().is_loopback() && !exposed {
            return Err(ConfigError::Exposed(addr));
        }
        let issuer = value(get, "SAGAT_DEV_IDP_ISSUER")
            .unwrap_or_else(|| format!("http://127.0.0.1:{}/", addr.port()));
        if !(issuer.starts_with("http://") || issuer.starts_with("https://"))
            || issuer.contains(char::is_whitespace)
            || issuer.contains('@')
        {
            return Err(invalid(
                "SAGAT_DEV_IDP_ISSUER",
                "expected an http(s) URL without credentials, spelled as the platform's SAGAT_OIDC_ISSUER is",
            ));
        }
        let audience = value(get, "SAGAT_DEV_IDP_AUDIENCE")
            .unwrap_or_else(|| "sagat-octopus-orchestrator".to_owned());
        let tenant_claim =
            value(get, "SAGAT_DEV_IDP_TENANT_CLAIM").unwrap_or_else(|| "tenant".to_owned());
        if tenant_claim.is_empty()
            || tenant_claim.len() > 64
            || ["iss", "aud", "sub", "exp", "iat", "nbf", "jti"].contains(&tenant_claim.as_str())
        {
            return Err(invalid(
                "SAGAT_DEV_IDP_TENANT_CLAIM",
                "a claim name of at most 64 characters that is not a registered one",
            ));
        }
        Ok(Self {
            addr,
            issuer,
            audience,
            tenant_claim,
            key_file: value(get, "SAGAT_DEV_IDP_KEY_FILE").map(PathBuf::from),
        })
    }

    /// The host names a request may carry in its `Host` header: the machine's own and the issuer's.
    /// A page that is made to resolve another name to this address is refused (DNS rebinding).
    #[must_use]
    pub fn hosts(&self) -> Vec<String> {
        let mut hosts = vec![
            format!("127.0.0.1:{}", self.addr.port()),
            format!("localhost:{}", self.addr.port()),
            format!("[::1]:{}", self.addr.port()),
        ];
        if let Some(authority) = self
            .issuer
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split(['/', '?', '#']).next())
        {
            hosts.push(authority.to_owned());
        }
        hosts
    }
}

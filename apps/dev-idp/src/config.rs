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
    /// Other `host:port` a request may carry in its `Host` header, for a published port that is not
    /// the one the process listens on (`SAGAT_DEV_IDP_HOSTS`, comma separated).
    pub extra_hosts: Vec<String>,
}

/// `host:port` names, comma separated: a name with a path, a scheme or a space is a mistake that
/// would silently never match.
fn parse_hosts(text: Option<&str>) -> Result<Vec<String>, ConfigError> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let mut hosts = Vec::new();
    for host in text.split(',').map(str::trim).filter(|h| !h.is_empty()) {
        let (name, port) = host.rsplit_once(':').unwrap_or((host, ""));
        let name_ok = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
        if !name_ok || port.parse::<u16>().is_err() {
            return Err(ConfigError::Invalid {
                name: "SAGAT_DEV_IDP_HOSTS",
                reason: format!("'{host}' is not host:port"),
            });
        }
        hosts.push(host.to_owned());
    }
    Ok(hosts)
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
            extra_hosts: parse_hosts(value(get, "SAGAT_DEV_IDP_HOSTS").as_deref())?,
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
            // A client writes the host in lower case and leaves out the port its scheme implies.
            let authority = authority.to_ascii_lowercase();
            let implied = if self.issuer.starts_with("https://") {
                ":443"
            } else {
                ":80"
            };
            if let Some(bare) = authority.strip_suffix(implied) {
                hosts.push(bare.to_owned());
            }
            hosts.push(authority);
        }
        hosts.extend(self.extra_hosts.iter().cloned());
        hosts
    }
}

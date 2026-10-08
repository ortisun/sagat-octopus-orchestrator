//! Authentication settings.

use std::time::Duration;

/// How tokens are verified. Every field has a safe default except the three identifiers.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Exact `iss` the tokens must carry.
    pub issuer: String,
    /// The `aud` the tokens must contain (this API's identifier at the OIDC provider).
    pub audience: String,
    /// Where the provider publishes its signing keys. Comes from configuration only: the
    /// `jku`/`x5u`/`jwk` token headers are never followed (they would let a token choose its own
    /// verification key).
    pub jwks_uri: String,
    /// Name of the claim holding the caller's tenant slug.
    pub tenant_claim: String,
    /// When set, only tokens whose JOSE `typ` header equals this value (case-insensitive) are
    /// accepted, e.g. `at+jwt` (RFC 9068), which keeps an ID token from being used as an access
    /// token. Off by default because providers disagree on the value (gap G14).
    pub required_token_type: Option<String>,
    /// Tolerated clock skew for `exp`, `nbf` and `iat`.
    pub leeway: Duration,
    /// Longest `exp - iat` accepted. Bounds the window in which a revoked token still works, since
    /// revocation lists are not consulted (see research.md, gap G7).
    pub max_token_lifetime: Duration,
    /// How long fetched keys are used before a refresh is attempted. Keys past this age keep
    /// verifying (see `jwks_max_stale`) while a refresh runs in the background, so a key that the
    /// provider has withdrawn stops working after this interval plus one background refresh.
    pub jwks_ttl: Duration,
    /// How long after `jwks_ttl` the last good keys may still be used if the provider is down.
    /// Beyond this the API fails closed (503).
    pub jwks_max_stale: Duration,
    /// Minimum spacing of refresh attempts, counted from the end of the previous attempt, so
    /// tokens with random `kid`s (or a hanging provider) cannot make the API hammer the provider.
    pub jwks_min_refresh_interval: Duration,
    /// Allow a plain-`http` `jwks_uri`. For local development and tests only.
    pub allow_insecure_jwks_http: bool,
}

impl AuthConfig {
    #[must_use]
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        jwks_uri: impl Into<String>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            audience: audience.into(),
            jwks_uri: jwks_uri.into(),
            tenant_claim: "tenant".to_owned(),
            required_token_type: None,
            leeway: Duration::from_secs(30),
            max_token_lifetime: Duration::from_secs(3600),
            jwks_ttl: Duration::from_secs(600),
            jwks_max_stale: Duration::from_secs(900),
            jwks_min_refresh_interval: Duration::from_secs(30),
            allow_insecure_jwks_http: false,
        }
    }
}

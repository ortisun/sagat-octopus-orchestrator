//! Bearer-token verification (FR-001-007, NFR-001-SEC).
//!
//! Fails closed: anything that is not positively valid is rejected, and when the signing keys
//! cannot be obtained the result is "unavailable", never "accepted".

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, Validation, decode, decode_header};
use serde::Deserialize;

use super::config::AuthConfig;
use super::jwks::{JwksCache, JwksError, JwksFetcher};

/// Tokens larger than this are rejected before any parsing.
const MAX_TOKEN_BYTES: usize = 8 * 1024;
const MAX_KID_BYTES: usize = 128;
const MAX_TENANT_BYTES: usize = 63;
const MAX_SUBJECT_BYTES: usize = 255;

/// Why a syntactically valid token was refused. Logged, never returned to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidReason {
    Algorithm,
    /// The token `typ` is not the required one (see `AuthConfig::required_token_type`).
    TokenType,
    /// A `crit` header names extensions this verifier does not implement (RFC 7515 §4.1.11).
    CriticalHeader,
    MissingKeyId,
    UnknownKey,
    Signature,
    Issuer,
    Audience,
    MissingClaim,
    NotYetValid,
    Lifetime,
    TenantClaim,
}

impl InvalidReason {
    /// Stable, low-cardinality name for metrics and logs.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Algorithm => "algorithm",
            Self::TokenType => "token_type",
            Self::CriticalHeader => "critical_header",
            Self::MissingKeyId => "missing_key_id",
            Self::UnknownKey => "unknown_key",
            Self::Signature => "signature",
            Self::Issuer => "issuer",
            Self::Audience => "audience",
            Self::MissingClaim => "missing_claim",
            Self::NotYetValid => "not_yet_valid",
            Self::Lifetime => "lifetime",
            Self::TenantClaim => "tenant_claim",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("token is malformed")]
    Malformed,
    #[error("token has expired")]
    Expired,
    #[error("token is not valid ({0:?})")]
    Invalid(InvalidReason),
    /// The verification keys could not be obtained; the request is refused but may be retried.
    #[error("signing keys are unavailable")]
    Unavailable,
}

/// What the rest of the system may trust after a token verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedClaims {
    pub subject: String,
    /// Raw value of the tenant claim; resolved against the database by the application layer.
    pub tenant: String,
}

/// Presence of `sub`/`exp` is enforced explicitly (not by serde) so that a missing claim is
/// reported as such instead of as a malformed token.
#[derive(Deserialize)]
struct Claims {
    sub: Option<String>,
    iat: Option<u64>,
    exp: Option<u64>,
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

/// Verifies access tokens issued by one OIDC provider.
pub struct TokenVerifier {
    cache: JwksCache,
    base: Validation,
    issuer: String,
    required_typ: Option<String>,
    tenant_claim: String,
    leeway: u64,
    max_lifetime: u64,
}

impl AuthError {
    /// Stable, low-cardinality name for metrics and logs.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::Expired => "expired",
            Self::Invalid(reason) => reason.label(),
            Self::Unavailable => "keys_unavailable",
        }
    }
}

impl TokenVerifier {
    #[must_use]
    pub fn new(fetcher: Arc<dyn JwksFetcher>, config: &AuthConfig) -> Self {
        let mut base = Validation::new(Algorithm::ES256);
        base.set_issuer(&[&config.issuer]);
        base.set_audience(&[&config.audience]);
        // The library validates these only when present unless they are also *required*.
        base.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        base.validate_nbf = true;
        base.leeway = config.leeway.as_secs();
        Self {
            cache: JwksCache::new(fetcher, config),
            base,
            issuer: config.issuer.clone(),
            // Trimmed because the value usually comes from a mounted file that ends in a newline;
            // normalised so the comparison is between media types, not between spellings.
            required_typ: config
                .required_token_type
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(media_type),
            tenant_claim: config.tenant_claim.clone(),
            leeway: config.leeway.as_secs(),
            max_lifetime: config.max_token_lifetime.as_secs(),
        }
    }

    /// Verifies signature and claims and extracts the caller's identity.
    ///
    /// # Errors
    /// See [`AuthError`]. All variants except [`AuthError::Unavailable`] mean "not authenticated".
    pub async fn verify(&self, token: &str) -> Result<VerifiedClaims, AuthError> {
        if token.len() > MAX_TOKEN_BYTES {
            return Err(AuthError::Malformed);
        }
        let header = decode_header(token).map_err(|_| AuthError::Malformed)?;
        // Algorithm confusion is blocked by three independent layers, kept on purpose: (1) this closed
        // allowlist (no HMAC, no `none`), (2) the key dictating its single algorithm below, and
        // (3) the library validating against that algorithm, never the token header's. Removing
        // all three is caught by `hmac_tokens_signed_with_the_public_key_are_refused`.
        if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
            return Err(AuthError::Invalid(InvalidReason::Algorithm));
        }
        if let Some(required) = &self.required_typ {
            let ok = header
                .typ
                .as_deref()
                .is_some_and(|t| media_type(t) == *required);
            if !ok {
                return Err(AuthError::Invalid(InvalidReason::TokenType));
            }
        }
        // We implement no JWS extensions, so a token that declares any as critical must be refused.
        if header.crit.is_some() {
            return Err(AuthError::Invalid(InvalidReason::CriticalHeader));
        }
        let kid = header
            .kid
            .as_deref()
            .filter(|k| !k.is_empty() && k.len() <= MAX_KID_BYTES)
            .ok_or(AuthError::Invalid(InvalidReason::MissingKeyId))?;

        let key = self.cache.key_for(kid).await.map_err(|error| match error {
            JwksError::UnknownKey => AuthError::Invalid(InvalidReason::UnknownKey),
            JwksError::Unavailable(_) | JwksError::Malformed => AuthError::Unavailable,
        })?;
        // The key dictates the algorithm: an RSA key can never be used to check an ES256 token.
        if key.algorithm != header.alg {
            return Err(AuthError::Invalid(InvalidReason::Algorithm));
        }

        let mut validation = self.base.clone();
        validation.algorithms = vec![key.algorithm];
        let claims = decode::<Claims>(token, &key.decoding_key, &validation)
            .map_err(|e| classify(e.kind()))?
            .claims;

        // `iss` is a string by definition (RFC 7519); the library also accepts an array that contains
        // the issuer, which this API never wants.
        if claims.extra.get("iss").and_then(serde_json::Value::as_str) != Some(self.issuer.as_str())
        {
            return Err(AuthError::Invalid(InvalidReason::Issuer));
        }
        self.check_lifetime(&claims)?;
        let tenant = self.tenant_of(&claims)?;
        let subject = claims
            .sub
            .filter(|s| !s.is_empty() && s.len() <= MAX_SUBJECT_BYTES)
            .ok_or(AuthError::Invalid(InvalidReason::MissingClaim))?;
        Ok(VerifiedClaims { subject, tenant })
    }

    fn check_lifetime(&self, claims: &Claims) -> Result<(), AuthError> {
        let missing = AuthError::Invalid(InvalidReason::MissingClaim);
        let iat = claims.iat.ok_or(missing)?;
        let exp = claims.exp.ok_or(missing)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if iat > now.saturating_add(self.leeway) {
            return Err(AuthError::Invalid(InvalidReason::NotYetValid));
        }
        if exp.saturating_sub(iat) > self.max_lifetime {
            return Err(AuthError::Invalid(InvalidReason::Lifetime));
        }
        Ok(())
    }

    fn tenant_of(&self, claims: &Claims) -> Result<String, AuthError> {
        claims
            .extra
            .get(&self.tenant_claim)
            .and_then(serde_json::Value::as_str)
            .filter(|t| !t.is_empty() && t.len() <= MAX_TENANT_BYTES)
            .map(str::to_owned)
            .ok_or(AuthError::Invalid(InvalidReason::TenantClaim))
    }
}

/// The JOSE `typ` as a lowercase media type. RFC 7515 4.1.9: a `typ` without a '/' is read as if
/// `application/` were prepended, so `at+jwt` and `application/at+jwt` are the same type (and
/// RFC 9068 expects resource servers to accept both). Media types are case-insensitive.
fn media_type(typ: &str) -> String {
    let lower = typ.to_ascii_lowercase();
    if lower.contains('/') {
        lower
    } else {
        format!("application/{lower}")
    }
}

fn classify(kind: &ErrorKind) -> AuthError {
    match kind {
        ErrorKind::ExpiredSignature => AuthError::Expired,
        ErrorKind::ImmatureSignature => AuthError::Invalid(InvalidReason::NotYetValid),
        ErrorKind::InvalidIssuer => AuthError::Invalid(InvalidReason::Issuer),
        ErrorKind::InvalidAudience => AuthError::Invalid(InvalidReason::Audience),
        ErrorKind::MissingRequiredClaim(_) => AuthError::Invalid(InvalidReason::MissingClaim),
        ErrorKind::InvalidSignature => AuthError::Invalid(InvalidReason::Signature),
        ErrorKind::InvalidAlgorithm
        | ErrorKind::InvalidAlgorithmName
        | ErrorKind::MissingAlgorithm => AuthError::Invalid(InvalidReason::Algorithm),
        _ => AuthError::Malformed,
    }
}

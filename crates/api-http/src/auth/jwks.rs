//! Signing-key discovery: fetch, filter and cache the provider's JWKS.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::{
    AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
};
use jsonwebtoken::{Algorithm, DecodingKey};
use tokio::sync::RwLock;

use super::config::AuthConfig;

/// Largest JWKS document accepted. Real sets are a few KiB.
const MAX_JWKS_BYTES: usize = 256 * 1024;

/// Boxed future so fetchers can be used behind `Arc<dyn ...>`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JwksError {
    /// The provider could not be reached or answered badly; retryable.
    #[error("key provider unavailable: {0}")]
    Unavailable(String),
    /// The provider answered with something that is not a JWKS.
    #[error("key provider returned a malformed key set")]
    Malformed,
    /// No usable key carries the requested `kid`, even after a refresh.
    #[error("no signing key with that id")]
    UnknownKey,
}

/// Source of the key set; abstracted so the cache can be tested without a network.
pub trait JwksFetcher: Send + Sync {
    fn fetch(&self) -> BoxFuture<'_, Result<JwkSet, JwksError>>;
}

/// Fetches the JWKS over HTTP(S) with strict limits: no redirects, short timeouts, bounded body.
pub struct HttpJwksFetcher {
    client: reqwest::Client,
    uri: String,
}

impl HttpJwksFetcher {
    /// # Errors
    /// [`JwksError::Unavailable`] when the URI is not allowed or the client cannot be built.
    pub fn new(config: &AuthConfig) -> Result<Self, JwksError> {
        install_crypto_provider();
        let scheme_ok = config.jwks_uri.starts_with("https://")
            || (config.allow_insecure_jwks_http && config.jwks_uri.starts_with("http://"));
        if !scheme_ok {
            return Err(JwksError::Unavailable("jwks_uri must use https".to_owned()));
        }
        let client = reqwest::Client::builder()
            .https_only(!config.allow_insecure_jwks_http)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .user_agent("sagat-octopus-orchestrator")
            .build()
            .map_err(|e| JwksError::Unavailable(e.to_string()))?;
        Ok(Self {
            client,
            uri: config.jwks_uri.clone(),
        })
    }
}

impl JwksFetcher for HttpJwksFetcher {
    fn fetch(&self) -> BoxFuture<'_, Result<JwkSet, JwksError>> {
        Box::pin(async move {
            let unavailable =
                |e: reqwest::Error| JwksError::Unavailable(e.without_url().to_string());
            let mut response = self
                .client
                .get(&self.uri)
                .send()
                .await
                .map_err(unavailable)?;
            if !response.status().is_success() {
                return Err(JwksError::Unavailable(format!(
                    "status {}",
                    response.status()
                )));
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_JWKS_BYTES as u64)
            {
                return Err(JwksError::Malformed);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(unavailable)? {
                if body.len() + chunk.len() > MAX_JWKS_BYTES {
                    return Err(JwksError::Malformed);
                }
                body.extend_from_slice(&chunk);
            }
            serde_json::from_slice::<JwkSet>(&body).map_err(|_| JwksError::Malformed)
        })
    }
}

/// Installs `ring` as the process-wide rustls provider. Idempotent: if another provider was
/// installed first, that one is kept.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// A verification key together with the single algorithm it may be used with.
#[derive(Clone)]
pub struct VerificationKey {
    pub decoding_key: DecodingKey,
    pub algorithm: Algorithm,
}

/// Turns a JWKS into usable keys, dropping everything that must not verify access tokens:
/// symmetric and unknown key types, keys without `kid`, keys marked for encryption, and keys whose
/// declared `alg` is not the one their type implies.
fn usable_keys(set: &JwkSet) -> HashMap<String, Arc<VerificationKey>> {
    set.keys.iter().filter_map(usable_key).collect()
}

fn usable_key(jwk: &Jwk) -> Option<(String, Arc<VerificationKey>)> {
    let kid = jwk.common.key_id.clone()?;
    if jwk
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|u| *u != PublicKeyUse::Signature)
    {
        return None;
    }
    let (algorithm, declared) = match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => (Algorithm::RS256, KeyAlgorithm::RS256),
        AlgorithmParameters::EllipticCurve(ec) if ec.curve == EllipticCurve::P256 => {
            (Algorithm::ES256, KeyAlgorithm::ES256)
        }
        // HMAC (`oct`), OKP, other curves and unknown types never verify tokens here.
        _ => return None,
    };
    if jwk.common.key_algorithm.is_some_and(|alg| alg != declared) {
        return None;
    }
    let decoding_key = DecodingKey::from_jwk(jwk).ok()?;
    Some((
        kid,
        Arc::new(VerificationKey {
            decoding_key,
            algorithm,
        }),
    ))
}

struct Cached {
    keys: HashMap<String, Arc<VerificationKey>>,
    fetched_at: Instant,
}

#[derive(Default)]
struct State {
    cached: Option<Cached>,
    last_attempt: Option<Instant>,
}

/// Caches the provider's keys with bounded staleness and rate-limited refreshes.
pub struct JwksCache {
    fetcher: Arc<dyn JwksFetcher>,
    ttl: Duration,
    max_stale: Duration,
    min_refresh_interval: Duration,
    state: RwLock<State>,
}

impl JwksCache {
    #[must_use]
    pub fn new(fetcher: Arc<dyn JwksFetcher>, config: &AuthConfig) -> Self {
        Self {
            fetcher,
            ttl: config.jwks_ttl,
            max_stale: config.jwks_max_stale,
            min_refresh_interval: config.jwks_min_refresh_interval,
            state: RwLock::new(State::default()),
        }
    }

    /// The key for `kid`, refreshing the set when it is expired or the `kid` is unknown.
    ///
    /// # Errors
    /// [`JwksError::UnknownKey`] when no key matches, [`JwksError::Unavailable`] when keys are
    /// needed but cannot be obtained within the staleness bound (fail closed).
    pub async fn key_for(&self, kid: &str) -> Result<Arc<VerificationKey>, JwksError> {
        {
            let state = self.state.read().await;
            if let Some(cached) = &state.cached {
                let fresh = cached.fetched_at.elapsed() < self.ttl;
                if let Some(key) = cached.keys.get(kid) {
                    if fresh {
                        return Ok(Arc::clone(key));
                    }
                } else if fresh && !self.may_refresh(&state) {
                    return Err(JwksError::UnknownKey);
                }
            }
        }
        self.refresh_and_lookup(kid).await
    }

    /// Whether another provider round trip is allowed now. With no keys at all the wait is short
    /// (at most a second) so a provider that was down at startup is picked up quickly; once keys
    /// exist, refreshes are spaced by `min_refresh_interval`.
    fn may_refresh(&self, state: &State) -> bool {
        let interval = if state.cached.is_none() {
            self.min_refresh_interval.min(Duration::from_secs(1))
        } else {
            self.min_refresh_interval
        };
        state.last_attempt.is_none_or(|t| t.elapsed() >= interval)
    }

    async fn refresh_and_lookup(&self, kid: &str) -> Result<Arc<VerificationKey>, JwksError> {
        // Single flight: concurrent callers queue here and the first one refreshes for all.
        let mut state = self.state.write().await;
        let horizon = self.ttl + self.max_stale;

        let expired = state
            .cached
            .as_ref()
            .is_none_or(|c| c.fetched_at.elapsed() >= self.ttl);
        let known = state
            .cached
            .as_ref()
            .is_some_and(|c| c.keys.contains_key(kid));
        if !expired && known {
            // Another caller refreshed while we waited for the lock.
            return from_cache(&state, kid, horizon);
        }
        // Whether the cache is expired or the kid is unknown, never contact the provider more than
        // once per interval: a down provider or a flood of random kids must not turn into a
        // queue of slow outbound requests.
        if !self.may_refresh(&state) {
            return from_cache(&state, kid, horizon);
        }

        state.last_attempt = Some(Instant::now());
        match self.fetcher.fetch().await {
            Ok(set) => {
                let keys = usable_keys(&set);
                let found = keys.get(kid).cloned();
                state.cached = Some(Cached {
                    keys,
                    fetched_at: Instant::now(),
                });
                found.ok_or(JwksError::UnknownKey)
            }
            Err(error) => {
                tracing::warn!(%error, "JWKS refresh failed");
                // Serve the last good keys for a bounded time so a short outage is survivable.
                from_cache(&state, kid, horizon)
            }
        }
    }
}

/// Looks `kid` up in the cache if it is younger than `horizon`; otherwise fails closed.
fn from_cache(
    state: &State,
    kid: &str,
    horizon: Duration,
) -> Result<Arc<VerificationKey>, JwksError> {
    match &state.cached {
        Some(cached) if cached.fetched_at.elapsed() < horizon => {
            cached.keys.get(kid).cloned().ok_or(JwksError::UnknownKey)
        }
        Some(_) => Err(JwksError::Unavailable(
            "cached keys are too old and the provider is unreachable".to_owned(),
        )),
        None => Err(JwksError::Unavailable(
            "no signing keys available".to_owned(),
        )),
    }
}

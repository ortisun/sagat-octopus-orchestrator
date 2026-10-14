//! Signing-key discovery: fetch, filter and cache the provider's JWKS.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::{
    AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse,
};
use jsonwebtoken::{Algorithm, DecodingKey};
use tokio::sync::Mutex;

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
    // `key_ops` is optional, but when the provider supplies it the key must explicitly permit
    // verification. This prevents a decrypt-only key published in the same set from crossing into
    // the token-verification role.
    if jwk
        .common
        .key_operations
        .as_ref()
        .is_some_and(|operations| !operations.contains(&KeyOperations::Verify))
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
    /// When the last provider round trip *finished* (successfully or not).
    last_attempt: Option<Instant>,
    /// An unknown `kid` is retryable after a failed refresh: the last good set cannot prove that a
    /// newly rotated key does not exist. Known cached keys remain usable under the stale policy.
    last_attempt_failed: bool,
}

/// What the cache can say about a `kid` right now, without any network.
enum Lookup {
    /// Known and within `ttl`.
    Fresh(Arc<VerificationKey>),
    /// Known, past `ttl` but within the stale window: usable, a refresh is due.
    Stale(Arc<VerificationKey>),
    /// The cache is usable but does not contain this `kid`.
    Unknown,
    /// Nothing cached yet.
    Missing,
    /// Cached keys are older than the stale window: too old to trust.
    Expired,
}

struct Inner {
    fetcher: Arc<dyn JwksFetcher>,
    ttl: Duration,
    max_stale: Duration,
    min_refresh_interval: Duration,
    /// Never held across an `.await`: lookups must not queue behind a slow provider.
    state: RwLock<State>,
    /// Single flight for refreshes. Held by the task that talks to the provider, so a caller that
    /// disconnects cannot release it while the fetch is still running.
    refresh_gate: Arc<Mutex<()>>,
}

impl Inner {
    fn lookup(&self, kid: &str) -> Lookup {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        let Some(cached) = &state.cached else {
            return Lookup::Missing;
        };
        let age = cached.fetched_at.elapsed();
        if age >= self.ttl + self.max_stale {
            return Lookup::Expired;
        }
        match cached.keys.get(kid) {
            Some(key) if age < self.ttl => Lookup::Fresh(Arc::clone(key)),
            Some(key) => Lookup::Stale(Arc::clone(key)),
            None => Lookup::Unknown,
        }
    }

    /// Whether another provider round trip is allowed now. With no keys at all the wait is short
    /// (at most a second) so a provider that was down at startup is picked up quickly; once keys
    /// exist, refreshes are spaced by `min_refresh_interval`. The interval counts from the end of
    /// the previous attempt, so a slow or hanging provider cannot be retried back to back.
    fn may_refresh(&self) -> bool {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        let interval = if state.cached.is_none() {
            self.min_refresh_interval.min(Duration::from_secs(1))
        } else {
            self.min_refresh_interval
        };
        state.last_attempt.is_none_or(|t| t.elapsed() >= interval)
    }

    fn last_attempt_failed(&self) -> bool {
        self.state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .last_attempt_failed
    }

    /// One provider round trip. Runs with no lock held.
    async fn refresh(&self) -> bool {
        let outcome = self.fetcher.fetch().await;
        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        state.last_attempt = Some(Instant::now());
        let failed = match outcome {
            Ok(set) => {
                let keys = usable_keys(&set);
                metrics::counter!("jwks_refresh_total", "outcome" => if keys.is_empty() { "no_usable_keys" } else { "ok" }).increment(1);
                if keys.is_empty() {
                    // A 200 without a single usable key is an incident at the provider, never a
                    // policy: keep whatever good keys we have rather than locking everyone out.
                    tracing::warn!(
                        "key provider returned no usable signing keys; keeping the previous keys"
                    );
                    true
                } else {
                    state.cached = Some(Cached {
                        keys,
                        fetched_at: Instant::now(),
                    });
                    false
                }
            }
            Err(error) => {
                metrics::counter!("jwks_refresh_total", "outcome" => "error").increment(1);
                tracing::warn!(%error, "JWKS refresh failed");
                true
            }
        };
        state.last_attempt_failed = failed;
        failed
    }
}

/// Caches the provider's keys with bounded staleness and rate-limited refreshes.
///
/// Availability properties (each covered by a fault test):
/// * a `kid` that is cached is served immediately, even while the provider hangs;
/// * expired-but-recent keys keep working while a refresh runs in the background;
/// * only a caller that truly needs new keys waits, and all of them share one bounded fetch;
/// * the provider is contacted at most once per interval, measured from the previous attempt's end.
pub struct JwksCache {
    inner: Arc<Inner>,
}

impl JwksCache {
    #[must_use]
    pub fn new(fetcher: Arc<dyn JwksFetcher>, config: &AuthConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                fetcher,
                ttl: config.jwks_ttl,
                max_stale: config.jwks_max_stale,
                min_refresh_interval: config.jwks_min_refresh_interval,
                state: RwLock::new(State::default()),
                refresh_gate: Arc::new(Mutex::new(())),
            }),
        }
    }

    /// The key for `kid`.
    ///
    /// # Errors
    /// [`JwksError::UnknownKey`] when no key matches, [`JwksError::Unavailable`] when keys are
    /// needed but cannot be obtained within the staleness bound (fail closed).
    pub async fn key_for(&self, kid: &str) -> Result<Arc<VerificationKey>, JwksError> {
        match self.inner.lookup(kid) {
            Lookup::Fresh(key) => return Ok(key),
            Lookup::Stale(key) => {
                self.refresh_in_background();
                return Ok(key);
            }
            Lookup::Unknown | Lookup::Missing | Lookup::Expired => {}
        }

        // Only a refresh can answer; share one bounded fetch with every other caller in this
        // position, and fall back to whatever the cache can say if refreshing is not allowed.
        let refresh_failed = self.refresh_and_wait().await;
        match self.inner.lookup(kid) {
            Lookup::Fresh(key) | Lookup::Stale(key) => Ok(key),
            Lookup::Unknown if refresh_failed => Err(JwksError::Unavailable(
                "signing-key refresh failed".to_owned(),
            )),
            Lookup::Unknown => Err(JwksError::UnknownKey),
            Lookup::Missing => Err(JwksError::Unavailable(
                "no signing keys available".to_owned(),
            )),
            Lookup::Expired => Err(JwksError::Unavailable(
                "cached keys are too old and the provider is unreachable".to_owned(),
            )),
        }
    }

    /// Refreshes if allowed, waiting for the result. Waiters queue on the gate only, never on the
    /// state, so callers whose key is cached are unaffected.
    async fn refresh_and_wait(&self) -> bool {
        if !self.inner.may_refresh() {
            return self.inner.last_attempt_failed();
        }
        let guard = Arc::clone(&self.inner.refresh_gate).lock_owned().await;
        if !self.inner.may_refresh() {
            return self.inner.last_attempt_failed(); // the caller ahead just refreshed (or tried)
        }
        let inner = Arc::clone(&self.inner);
        // Detached: if this request is dropped, the fetch still finishes and still releases the gate.
        let task = tokio::spawn(async move {
            let _guard = guard;
            inner.refresh().await
        });
        task.await.unwrap_or(true)
    }

    /// Starts a refresh without making anyone wait for it, if one is allowed and none is running.
    fn refresh_in_background(&self) {
        if !self.inner.may_refresh() {
            return;
        }
        let Ok(guard) = Arc::clone(&self.inner.refresh_gate).try_lock_owned() else {
            return; // a refresh is already running
        };
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let _guard = guard;
            if inner.may_refresh() {
                let _ = inner.refresh().await;
            }
        });
    }
}

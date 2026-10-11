//! Protection against one tenant (or one flood) starving everyone else (gap G13).
//!
//! * [`RateLimiter`]: a token bucket per key. It is used per tenant (after authentication) and per
//!   verified token subject (before the database is touched), so a caller that exceeds its share
//!   gets `429` while the others are unaffected.
//! * [`shed_excess`]: a global cap on requests in flight. Beyond it requests are refused at once with
//!   a retryable `503` instead of queueing, which would only add latency and memory.
//!
//! All of it is in-process. With several instances each applies its own limit; a shared limiter is a
//! later concern (feature 010).

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sagat_domain::identity::TenantId;
use tokio::sync::Semaphore;

use crate::error::ApiError;

/// Request budget of one key (a tenant or a subject).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimitConfig {
    /// Sustained requests per second.
    pub per_second: f64,
    /// Short-term burst allowed on top of the sustained rate.
    pub burst: f64,
}

impl Default for RateLimitConfig {
    /// The per-tenant budget.
    fn default() -> Self {
        Self {
            per_second: 100.0,
            burst: 200.0,
        }
    }
}

/// Why a [`RateLimitConfig`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RateLimitConfigError {
    #[error(
        "requests per second must be a finite number of at least {}",
        RateLimitConfig::MIN_PER_SECOND
    )]
    PerSecond,
    #[error(
        "burst must be a finite number of at least {}",
        RateLimitConfig::MIN_BURST
    )]
    Burst,
}

impl RateLimitConfig {
    /// Smallest sustained rate accepted. Below it the wait for the next token exceeds
    /// [`MAX_RETRY_AFTER`] and a tiny value only hides a typo.
    pub const MIN_PER_SECOND: f64 = 0.001;
    /// A bucket must hold at least one token or no request could ever pass.
    pub const MIN_BURST: f64 = 1.0;

    /// The default budget of one verified token subject, charged before the database is queried.
    /// Much lower than a tenant's: a person or a service account is one client of many in a tenant.
    #[must_use]
    pub const fn subject_default() -> Self {
        Self {
            per_second: 20.0,
            burst: 40.0,
        }
    }

    /// # Errors
    /// [`RateLimitConfigError`] naming the first unusable value (NaN, infinite, or too small).
    pub fn validate(self) -> Result<Self, RateLimitConfigError> {
        if !(self.per_second.is_finite() && self.per_second >= Self::MIN_PER_SECOND) {
            return Err(RateLimitConfigError::PerSecond);
        }
        if !(self.burst.is_finite() && self.burst >= Self::MIN_BURST) {
            return Err(RateLimitConfigError::Burst);
        }
        Ok(self)
    }
}

/// Longest `Retry-After` the limiter ever suggests.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

#[derive(Debug)]
struct Table<K> {
    buckets: HashMap<K, Bucket>,
    /// Shared by every key that arrives while the table is full (see [`RateLimiter::with_capacity`]).
    overflow: Option<Bucket>,
    last_prune: Option<Instant>,
}

/// Token-bucket rate limiter keyed by `K` (a tenant, a token subject, ...).
#[derive(Debug)]
pub struct RateLimiter<K = TenantId> {
    config: RateLimitConfig,
    max_keys: usize,
    prune_above: usize,
    prune_interval: Duration,
    state: Mutex<Table<K>>,
}

/// Keys tracked at most (about 10 MB for short keys). See [`RateLimiter::with_capacity`].
pub const DEFAULT_MAX_TRACKED_KEYS: usize = 100_000;

impl<K: Eq + Hash> RateLimiter<K> {
    /// # Errors
    /// [`RateLimitConfigError`] when the budget is not a usable one (see [`RateLimitConfig::validate`]).
    pub fn new(config: RateLimitConfig) -> Result<Self, RateLimitConfigError> {
        Self::with_capacity(config, DEFAULT_MAX_TRACKED_KEYS)
    }

    /// A limiter that tracks at most `max_keys` distinct keys.
    ///
    /// Idle keys (those whose bucket has refilled completely, so they carry no information) are
    /// dropped by a sweep that runs at most once per refill window, never on every call. If the
    /// table is still full after that, keys that are not tracked yet **share one overflow bucket**
    /// with the same budget: memory stops growing, whoever fills the table cannot raise their own
    /// allowance by doing so, and the price is paid only by other untracked keys (which are then
    /// throttled together) until the next sweep frees room. Keys already tracked keep their own
    /// bucket.
    ///
    /// # Errors
    /// [`RateLimitConfigError`] when the budget is not a usable one.
    pub fn with_capacity(
        config: RateLimitConfig,
        max_keys: usize,
    ) -> Result<Self, RateLimitConfigError> {
        Ok(Self::build(config.validate()?, max_keys))
    }

    /// For budgets that are valid by construction (the defaults).
    pub(crate) fn from_valid(config: RateLimitConfig) -> Self {
        Self::build(config, DEFAULT_MAX_TRACKED_KEYS)
    }

    fn build(config: RateLimitConfig, max_keys: usize) -> Self {
        let max_keys = max_keys.max(1);
        // A bucket is full again after `burst / per_second` seconds idle; sweeping more often than
        // once a second only burns CPU under the lock, less often than once a minute only holds
        // memory.
        let refill_window = (config.burst / config.per_second).clamp(1.0, 60.0);
        Self {
            config,
            max_keys,
            prune_above: (max_keys / 10).max(1),
            prune_interval: Duration::from_secs_f64(refill_window),
            state: Mutex::new(Table {
                buckets: HashMap::new(),
                overflow: None,
                last_prune: None,
            }),
        }
    }

    /// Takes one token for `key`.
    ///
    /// # Errors
    /// The time after which a token will be available, when the key is over its budget (never more
    /// than [`MAX_RETRY_AFTER`]).
    pub fn check(&self, key: K) -> Result<(), Duration> {
        self.check_at(key, Instant::now())
    }

    /// [`Self::check`] with an explicit clock, for deterministic tests.
    ///
    /// # Errors
    /// The time after which a token will be available.
    pub fn check_at(&self, key: K, now: Instant) -> Result<(), Duration> {
        let RateLimitConfig { per_second, burst } = self.config;
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let state = &mut *guard;

        let due = state
            .last_prune
            .is_none_or(|at| now.saturating_duration_since(at) >= self.prune_interval);
        if state.buckets.len() >= self.prune_above && due {
            state.last_prune = Some(now);
            // A bucket idle long enough to be full again carries no information.
            state.buckets.retain(|_, b| {
                now.saturating_duration_since(b.updated).as_secs_f64() * per_second < burst
            });
        }

        if let Some(bucket) = state.buckets.get_mut(&key) {
            take(bucket, now, self.config)
        } else if state.buckets.len() < self.max_keys {
            let mut bucket = Bucket {
                tokens: burst,
                updated: now,
            };
            let outcome = take(&mut bucket, now, self.config);
            state.buckets.insert(key, bucket);
            outcome
        } else {
            let overflow = state.overflow.get_or_insert(Bucket {
                tokens: burst,
                updated: now,
            });
            take(overflow, now, self.config)
        }
    }

    /// Number of keys currently tracked (for tests and diagnostics).
    #[must_use]
    pub fn tracked_keys(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .buckets
            .len()
    }
}

/// Refills `bucket` up to `now` and takes a token if there is one.
fn take(bucket: &mut Bucket, now: Instant, config: RateLimitConfig) -> Result<(), Duration> {
    let RateLimitConfig { per_second, burst } = config;
    let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
    bucket.tokens = elapsed.mul_add(per_second, bucket.tokens).min(burst);
    // Never move the refill clock backwards: calls are timestamped before they take the lock, so a
    // slow one can arrive "from the past", and rewinding would credit the same interval twice.
    bucket.updated = bucket.updated.max(now);
    if bucket.tokens >= 1.0 {
        bucket.tokens -= 1.0;
        Ok(())
    } else {
        let wait = (1.0 - bucket.tokens) / per_second;
        Err(Duration::try_from_secs_f64(wait)
            .unwrap_or(MAX_RETRY_AFTER)
            .min(MAX_RETRY_AFTER))
    }
}

/// `Retry-After` for a shed request: 3 to 8 seconds, cycling, so clients that were refused together
/// do not all come back in the same second.
fn shed_retry_after() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    3 + NEXT.fetch_add(1, Ordering::Relaxed) % 6
}

/// Middleware: refuse immediately when too many requests are already being served.
pub async fn shed_excess(
    State(slots): State<Arc<Semaphore>>,
    request: Request,
    next: Next,
) -> Response {
    match slots.try_acquire_owned() {
        Ok(permit) => {
            let response = next.run(request).await;
            drop(permit);
            response
        }
        Err(_) => {
            metrics::counter!("http_load_shed_total").increment(1);
            ApiError::unavailable_after(shed_retry_after()).into_response()
        }
    }
}

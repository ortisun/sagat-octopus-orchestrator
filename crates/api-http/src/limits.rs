//! Protection against one tenant (or one flood) starving everyone else (gap G13).
//!
//! * [`RateLimiter`]: a token bucket per tenant, checked after authentication, so a tenant that
//!   exceeds its share gets `429` while the others are unaffected.
//! * [`shed_excess`]: a global cap on requests in flight. Beyond it requests are refused at once with
//!   a retryable `503` instead of queueing, which would only add latency and memory.
//!
//! Both are in-process. With several instances each applies its own limit; a shared limiter is a
//! later concern (feature 010).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sagat_domain::identity::TenantId;
use tokio::sync::Semaphore;

use crate::error::ApiError;

/// Per-tenant request budget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimitConfig {
    /// Sustained requests per second a tenant may make.
    pub per_second: f64,
    /// Short-term burst a tenant may use on top of the sustained rate.
    pub burst: f64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            per_second: 100.0,
            burst: 200.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// Token-bucket rate limiter keyed by tenant.
#[derive(Debug)]
pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: Mutex<HashMap<TenantId, Bucket>>,
}

/// Idle buckets are dropped once the map grows past this, so tenant churn cannot grow it forever.
const PRUNE_ABOVE: usize = 10_000;

impl RateLimiter {
    #[must_use]
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes one token for `tenant`.
    ///
    /// # Errors
    /// The time after which a token will be available, when the tenant is over its budget.
    pub fn check(&self, tenant: TenantId) -> Result<(), Duration> {
        self.check_at(tenant, Instant::now())
    }

    /// [`Self::check`] with an explicit clock, for deterministic tests.
    ///
    /// # Errors
    /// The time after which a token will be available.
    pub fn check_at(&self, tenant: TenantId, now: Instant) -> Result<(), Duration> {
        let RateLimitConfig { per_second, burst } = self.config;
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if buckets.len() > PRUNE_ABOVE {
            // A bucket idle long enough to be full again carries no information.
            buckets.retain(|_, b| {
                now.saturating_duration_since(b.updated).as_secs_f64() * per_second < burst
            });
        }
        let bucket = buckets.entry(tenant).or_insert(Bucket {
            tokens: burst,
            updated: now,
        });
        let elapsed = now.saturating_duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = elapsed.mul_add(per_second, bucket.tokens).min(burst);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let missing = 1.0 - bucket.tokens;
            Err(Duration::from_secs_f64(missing / per_second))
        }
    }

    /// Number of tenants currently tracked (for tests and metrics).
    #[must_use]
    pub fn tracked_tenants(&self) -> usize {
        self.buckets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
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
            ApiError::unavailable().into_response()
        }
    }
}

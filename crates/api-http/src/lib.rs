//! HTTP adapter of the control plane.
//!
//! * [`auth`]: OIDC/JWT verification with a hardened JWKS cache.
//! * [`error`]: the single error envelope, completed with the trace id by [`context`].
//! * [`router`]: the Axum router with body-size, timeout and panic protection.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::middleware::from_fn;
use axum::routing::get;
use sagat_application::clock::Clock;
use sagat_application::identity::{IdentityService, IdentityStore};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

pub mod auth;
mod context;
mod dto;
pub mod error;
mod extract;
mod limits;
mod routes;
mod server;

pub use auth::{AuthConfig, HttpJwksFetcher, TokenVerifier};
pub use limits::{
    DEFAULT_MAX_TRACKED_KEYS, MAX_RETRY_AFTER, RateLimitConfig, RateLimitConfigError, RateLimiter,
};
pub use server::{ServerConfig, serve};

/// Shared handler state. Cloning is cheap (a few `Arc`s).
pub struct AppState<S, C> {
    pub identity: Arc<IdentityService<S, C>>,
    pub verifier: Arc<TokenVerifier>,
    /// Per-tenant request budget, checked after authentication.
    pub limiter: Arc<RateLimiter>,
    /// Per-subject request budget, checked right after the token verifies and BEFORE any database
    /// lookup, so a caller who holds a valid token but no membership (or names a tenant that does
    /// not exist) cannot make the database work for free. Keyed by the token's `sub`: the verifier
    /// accepts a single configured issuer, so `(iss, sub)` would add nothing.
    pub subject_limiter: Arc<RateLimiter<String>>,
    /// Last readiness probe result, so a flood of `/v1/ready` calls cannot exhaust the pool.
    pub(crate) ready_cache: Arc<Mutex<Option<(Instant, bool)>>>,
}

impl<S, C> AppState<S, C> {
    /// State with the default budgets.
    #[must_use]
    pub fn new(identity: IdentityService<S, C>, verifier: TokenVerifier) -> Self {
        Self::assemble(
            identity,
            verifier,
            RateLimiter::from_valid(RateLimitConfig::default()),
        )
    }

    /// State with an explicit per-tenant budget.
    ///
    /// # Errors
    /// [`RateLimitConfigError`] when the budget is not usable (NaN, infinite or too small).
    pub fn with_rate_limit(
        identity: IdentityService<S, C>,
        verifier: TokenVerifier,
        rate_limit: RateLimitConfig,
    ) -> Result<Self, RateLimitConfigError> {
        Ok(Self::assemble(
            identity,
            verifier,
            RateLimiter::new(rate_limit)?,
        ))
    }

    fn assemble(
        identity: IdentityService<S, C>,
        verifier: TokenVerifier,
        limiter: RateLimiter,
    ) -> Self {
        Self {
            identity: Arc::new(identity),
            verifier: Arc::new(verifier),
            limiter: Arc::new(limiter),
            subject_limiter: Arc::new(RateLimiter::from_valid(RateLimitConfig::subject_default())),
            ready_cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Replaces the per-subject budget (the default is [`RateLimitConfig::subject_default`]).
    ///
    /// # Errors
    /// [`RateLimitConfigError`] when the budget is not usable (NaN, infinite or too small).
    pub fn with_subject_rate_limit(
        mut self,
        rate_limit: RateLimitConfig,
    ) -> Result<Self, RateLimitConfigError> {
        self.subject_limiter = Arc::new(RateLimiter::new(rate_limit)?);
        Ok(self)
    }
}

impl<S, C> Clone for AppState<S, C> {
    fn clone(&self) -> Self {
        Self {
            identity: Arc::clone(&self.identity),
            verifier: Arc::clone(&self.verifier),
            limiter: Arc::clone(&self.limiter),
            subject_limiter: Arc::clone(&self.subject_limiter),
            ready_cache: Arc::clone(&self.ready_cache),
        }
    }
}

/// Protective limits applied to every request.
#[derive(Debug, Clone, Copy)]
pub struct HttpLimits {
    /// Largest accepted request body.
    pub max_body_bytes: usize,
    /// Longest a request may take before it is answered `408`.
    pub request_timeout: Duration,
    /// Requests served at the same time; beyond it requests are refused at once with a retryable
    /// `503` instead of queueing. Probes are exempt.
    pub max_in_flight: usize,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 64 * 1024,
            request_timeout: Duration::from_secs(15),
            max_in_flight: 512,
        }
    }
}

/// Builds the API router (all routes live under `/v1`).
pub fn router<S, C>(state: AppState<S, C>, limits: HttpLimits) -> Router
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    use routes::{health, identity};

    // Everything except the probes competes for the in-flight budget; probes must keep answering
    // while the API is saturated or the orchestrator would restart a healthy but busy pod.
    let slots = Arc::new(tokio::sync::Semaphore::new(limits.max_in_flight.max(1)));
    let api = Router::new()
        .route("/auth/context", get(identity::auth_context))
        .route(
            "/workspaces",
            get(identity::list_workspaces::<S, C>).post(identity::create_workspace::<S, C>),
        )
        .route(
            "/workspaces/{workspace_id}",
            get(identity::get_workspace::<S, C>),
        )
        .route(
            "/workspaces/{workspace_id}/namespaces",
            axum::routing::post(identity::create_namespace::<S, C>),
        )
        .route("/namespaces", get(identity::list_namespaces::<S, C>))
        .route(
            "/namespaces/{namespace_id}",
            get(identity::get_namespace::<S, C>),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            slots,
            limits::shed_excess,
        ));
    let v1 = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready::<S, C>))
        .merge(api)
        .route_layer(from_fn(context::record_route));

    protect(Router::new().nest("/v1", v1).with_state(state), limits)
}

/// Wraps a router with the protections every endpoint must have: panic containment, a request
/// timeout, a body-size limit, and the request context that completes the error envelope.
///
/// Layers added later run first, so the request context is outermost and also completes the error
/// envelope of timeouts, oversized bodies and panics produced below it.
pub fn protect(router: Router, limits: HttpLimits) -> Router {
    router
        .layer(CatchPanicLayer::new())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            limits.request_timeout,
        ))
        .layer(RequestBodyLimitLayer::new(limits.max_body_bytes))
        .layer(from_fn(context::request_context))
}

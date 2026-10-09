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
use tower_http::timeout::{RequestBodyTimeoutLayer, TimeoutLayer};

pub mod auth;
mod concurrency;
mod context;
mod core;
mod dto;
mod dto_exec;
mod dto_gov;
pub mod error;
pub mod exec;
mod extract;
pub mod governance;
mod limits;
mod routes;
mod server;

pub use auth::{AuthConfig, HttpJwksFetcher, TokenVerifier};
pub use concurrency::{TenantAtCapacity, TenantConcurrency, TenantPermit};
pub use core::Core;
pub use exec::{Exec, ExecState};
pub use extract::{Authenticated, authenticate_bearer, parse_bearer};
pub use governance::{GovState, Governance};
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
    /// Requests of one tenant being served at once. [`router`] replaces it with the cap of its
    /// [`HttpLimits`], so the cap is configured in one place together with the other limits.
    pub tenant_concurrency: Arc<TenantConcurrency>,
    /// Last readiness probe result, so a flood of `/v1/ready` calls cannot exhaust the pool.
    pub(crate) ready_cache: Arc<Mutex<Option<(Instant, bool)>>>,
    /// Serialises readiness refreshes so concurrent probes share one storage ping.
    pub(crate) ready_refresh: Arc<tokio::sync::Mutex<()>>,
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
            tenant_concurrency: Arc::new(TenantConcurrency::new(
                HttpLimits::default().max_in_flight_per_tenant,
            )),
            ready_cache: Arc::new(Mutex::new(None)),
            ready_refresh: Arc::new(tokio::sync::Mutex::new(())),
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
            tenant_concurrency: Arc::clone(&self.tenant_concurrency),
            ready_cache: Arc::clone(&self.ready_cache),
            ready_refresh: Arc::clone(&self.ready_refresh),
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
    /// Requests of ONE tenant served at the same time; beyond it that tenant's requests are
    /// refused at once with a retryable `429`, so a tenant whose clients hold requests open (a slow
    /// or stalled upload) cannot occupy every slot of `max_in_flight`.
    pub max_in_flight_per_tenant: usize,
    /// Longest the server waits for the next piece of a request body before it gives up with `408`.
    /// Bounds the slot a client holds by announcing a body and not sending it.
    pub body_idle_timeout: Duration,
}

impl HttpLimits {
    /// The per-tenant concurrency to use with a given global cap: a quarter of it, at least one.
    #[must_use]
    pub fn default_per_tenant(max_in_flight: usize) -> usize {
        (max_in_flight / 4).max(1)
    }
}

impl Default for HttpLimits {
    fn default() -> Self {
        let max_in_flight = 512;
        Self {
            max_body_bytes: 64 * 1024,
            request_timeout: Duration::from_secs(15),
            max_in_flight,
            max_in_flight_per_tenant: Self::default_per_tenant(max_in_flight),
            body_idle_timeout: Duration::from_secs(5),
        }
    }
}

/// Builds the API router (all routes live under `/v1`).
pub fn router<S, C>(mut state: AppState<S, C>, limits: HttpLimits) -> Router
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    use routes::{health, identity};

    // One place configures every limit: the per-tenant cap comes from `limits`, like the global one.
    state.tenant_concurrency = Arc::new(TenantConcurrency::new(limits.max_in_flight_per_tenant));

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

/// Largest body of the definition and run routes: a workflow document (1 MiB) plus a little room
/// for the envelope. Every other route keeps [`HttpLimits::max_body_bytes`].
pub const EXEC_MAX_BODY_BYTES: usize = 1024 * 1024 + 4096;

/// Builds the API router with the definition and run routes next to the identity ones.
pub fn router_with_execution<S, C, P, X, E>(
    state: ExecState<S, C, P, X, E>,
    limits: HttpLimits,
) -> Router
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
    ExecState<S, C, P, X, E>: Exec,
{
    compose(state, limits, |_, _| Router::new())
}

/// Builds the API router with the governance routes (quotas) next to the definition, run and
/// identity ones. The governance routes share the in-flight budget and the protective layers.
pub fn router_with_governance<S, C, P, X, E, G>(
    state: ExecState<S, C, P, X, E>,
    governance: GovState<S, C, G>,
    limits: HttpLimits,
) -> Router
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
    G: sagat_application::quotas::QuotaStore
        + sagat_application::bindings::BindingStore
        + sagat_application::secrets::SecretStore
        + 'static,
    ExecState<S, C, P, X, E>: Exec,
{
    compose(state, limits, move |slots, limits| {
        governance_routes(governance, slots, limits)
    })
}

fn governance_routes<S, C, G, T>(
    state: GovState<S, C, G>,
    slots: &Arc<tokio::sync::Semaphore>,
    limits: &HttpLimits,
) -> Router<T>
where
    T: Clone + Send + Sync + 'static,
    S: IdentityStore + 'static,
    C: Clock + 'static,
    G: sagat_application::quotas::QuotaStore
        + sagat_application::bindings::BindingStore
        + sagat_application::secrets::SecretStore
        + 'static,
{
    use routes::governance as gov;
    type B<S, C, G> = GovState<S, C, G>;

    let serves_secrets = state.secrets().is_some();
    let mut router = Router::new()
        .route("/quotas", get(gov::list_tenant_quotas::<B<S, C, G>>))
        .route(
            "/quotas/{resource}",
            axum::routing::put(gov::set_tenant_quota::<B<S, C, G>>)
                .delete(gov::clear_tenant_quota::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/quotas",
            get(gov::list_namespace_quotas::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/quotas/{resource}",
            axum::routing::put(gov::set_namespace_quota::<B<S, C, G>>)
                .delete(gov::clear_namespace_quota::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/worker-bindings",
            get(gov::list_bindings::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/worker-bindings/{subject}",
            axum::routing::put(gov::bind_worker::<B<S, C, G>>)
                .delete(gov::unbind_worker::<B<S, C, G>>),
        );
    // Without a key ring there is nothing to seal with: the secret routes do not exist.
    if serves_secrets {
        router = router
            .route(
                "/namespaces/{namespace_id}/secrets",
                get(gov::list_secrets::<B<S, C, G>>),
            )
            .route(
                "/namespaces/{namespace_id}/secrets/{name}",
                axum::routing::put(gov::put_secret::<B<S, C, G>>)
                    .delete(gov::delete_secret::<B<S, C, G>>),
            )
            .route(
                "/namespaces/{namespace_id}/secrets/{name}/access",
                get(gov::list_secret_access::<B<S, C, G>>),
            )
            .route(
                "/namespaces/{namespace_id}/secrets/{name}/access/{subject}",
                axum::routing::put(gov::grant_secret_access::<B<S, C, G>>)
                    .delete(gov::revoke_secret_access::<B<S, C, G>>),
            );
    }
    router
        .layer(RequestBodyLimitLayer::new(limits.max_body_bytes))
        .route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(slots),
            limits::shed_excess,
        ))
        .with_state(state)
}

fn compose<S, C, P, X, E>(
    mut state: ExecState<S, C, P, X, E>,
    limits: HttpLimits,
    extra: impl FnOnce(&Arc<tokio::sync::Semaphore>, &HttpLimits) -> Router<AppState<S, C>>,
) -> Router
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
    ExecState<S, C, P, X, E>: Exec,
{
    use routes::execution as ex;
    use routes::{health, identity};

    // One place configures every limit: the per-tenant cap comes from `limits`, like the global one.
    state.core.tenant_concurrency =
        Arc::new(TenantConcurrency::new(limits.max_in_flight_per_tenant));
    let core = state.core.clone();
    type B<S, C, P, X, E> = ExecState<S, C, P, X, E>;

    let slots = Arc::new(tokio::sync::Semaphore::new(limits.max_in_flight.max(1)));
    let shed = || axum::middleware::from_fn_with_state(Arc::clone(&slots), limits::shed_excess);

    let identity_routes = Router::new()
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
        .layer(RequestBodyLimitLayer::new(limits.max_body_bytes))
        .route_layer(shed())
        .with_state(core.clone());

    let exec_routes = Router::new()
        .route(
            "/workflows",
            get(ex::list_workflows::<B<S, C, P, X, E>>)
                .post(ex::create_workflow::<B<S, C, P, X, E>>),
        )
        .route(
            "/workflows/{workflow_id}",
            get(ex::get_workflow::<B<S, C, P, X, E>>).put(ex::update_draft::<B<S, C, P, X, E>>),
        )
        .route(
            "/workflows/{workflow_id}/publish",
            axum::routing::post(ex::publish_workflow::<B<S, C, P, X, E>>),
        )
        .route(
            "/workflows/{workflow_id}/versions",
            get(ex::list_versions::<B<S, C, P, X, E>>),
        )
        .route(
            "/workflows/{workflow_id}/versions/{number}",
            get(ex::get_version::<B<S, C, P, X, E>>),
        )
        .route(
            "/runs",
            get(ex::list_runs::<B<S, C, P, X, E>>).post(ex::start_run::<B<S, C, P, X, E>>),
        )
        .route("/runs/{run_id}", get(ex::get_run::<B<S, C, P, X, E>>))
        .route(
            "/runs/{run_id}/events",
            get(ex::run_journal::<B<S, C, P, X, E>>),
        )
        .route(
            "/runs/{run_id}/graph",
            get(ex::run_graph::<B<S, C, P, X, E>>),
        )
        .route(
            "/runs/{run_id}/operations",
            axum::routing::post(ex::operate_run::<B<S, C, P, X, E>>),
        )
        .route(
            "/runs/{run_id}/signals/{signal_name}",
            axum::routing::post(ex::signal_run::<B<S, C, P, X, E>>),
        )
        .layer(RequestBodyLimitLayer::new(EXEC_MAX_BODY_BYTES))
        .route_layer(shed())
        .with_state(state);

    let extra_routes = extra(&slots, &limits);
    let v1 = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready::<S, C>))
        .merge(identity_routes)
        .merge(exec_routes)
        .merge(extra_routes)
        .route_layer(from_fn(context::record_route))
        .with_state(core);

    // The outer limit must admit the largest route; each route then enforces its own.
    let outer = HttpLimits {
        max_body_bytes: limits.max_body_bytes.max(EXEC_MAX_BODY_BYTES),
        ..limits
    };
    protect(Router::new().nest("/v1", v1), outer)
}

/// Wraps a router with the protections every endpoint must have: panic containment, a request
/// timeout, a body-idle timeout, a body-size limit, and the request context that completes the
/// error envelope.
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
        // A client that announces a body and then stalls (or trickles it) holds an in-flight slot
        // until the request timeout; this ends it after `body_idle_timeout` without a frame.
        .layer(RequestBodyTimeoutLayer::new(limits.body_idle_timeout))
        .layer(from_fn(context::request_context))
}

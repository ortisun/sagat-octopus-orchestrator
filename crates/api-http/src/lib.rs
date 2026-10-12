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
pub mod blobs;
mod concurrency;
mod context;
mod core;
mod dto;
mod dto_events;
mod dto_exec;
mod dto_gov;
mod dto_ops;
mod dto_scripts;
mod dto_versioning;
pub mod error;
pub mod exec;
mod extract;
pub mod governance;
mod limits;
mod routes;
mod server;
pub mod streams;
pub mod studio;

pub use auth::{AuthConfig, HttpJwksFetcher, TokenVerifier};
pub use blobs::HttpBlobTransfer;
pub use concurrency::{TenantAtCapacity, TenantConcurrency, TenantPermit};
pub use core::Core;
pub use exec::{Exec, ExecState};
pub use extract::{Authenticated, authenticate_bearer, parse_bearer};
pub use governance::{GovState, Governance};
pub use limits::{
    DEFAULT_MAX_TRACKED_KEYS, MAX_RETRY_AFTER, RateLimitConfig, RateLimitConfigError, RateLimiter,
};
pub use server::{HttpTlsConfig, ServerConfig, serve, serve_tls};

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
        .route("/version", get(identity::platform_version))
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
        + sagat_application::egress::EgressStore
        + sagat_application::events::EventStore
        + sagat_application::outbound::OutboundStore
        + sagat_application::connectors::ConnectorStore
        + sagat_application::scripts::ScriptStore
        + sagat_application::scripts::RunnerStore
        + sagat_application::operations::AlertReader
        + sagat_application::operations::WorkerAdmin
        + sagat_application::operations::AuditStore
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
        + sagat_application::egress::EgressStore
        + sagat_application::events::EventStore
        + sagat_application::outbound::OutboundStore
        + sagat_application::connectors::ConnectorStore
        + sagat_application::scripts::ScriptStore
        + sagat_application::scripts::RunnerStore
        + sagat_application::operations::AlertReader
        + sagat_application::operations::WorkerAdmin
        + sagat_application::operations::AuditStore
        + 'static,
{
    use routes::governance as gov;
    use routes::operations as ops;
    type B<S, C, G> = GovState<S, C, G>;

    let serves_secrets = state.secrets().is_some();
    let serves_events = state.events().is_some();
    let serves_outbound = state.outbound().is_some();
    let serves_connectors = state.connectors().is_some();
    let serves_scripts = state.scripts().is_some();
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
            "/namespaces/{namespace_id}/activity-quotas",
            get(gov::list_activity_quotas::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/activity-quotas/{activity}",
            axum::routing::put(gov::set_activity_quota::<B<S, C, G>>)
                .delete(gov::clear_activity_quota::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/worker-bindings",
            get(gov::list_bindings::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/worker-bindings/{subject}",
            axum::routing::put(gov::bind_worker::<B<S, C, G>>)
                .delete(gov::unbind_worker::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/alerts",
            get(ops::list_alerts::<B<S, C, G>>),
        )
        .route("/audit", get(ops::list_audit::<B<S, C, G>>))
        .route(
            "/namespaces/{namespace_id}/workers",
            get(ops::list_workers::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/workers/{subject}/drain",
            axum::routing::post(ops::drain_worker::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/workers/{subject}/undrain",
            axum::routing::post(ops::undrain_worker::<B<S, C, G>>),
        )
        .route(
            "/namespaces/{namespace_id}/egress-policy",
            get(gov::get_egress_policy::<B<S, C, G>>)
                .put(gov::set_egress_policy::<B<S, C, G>>)
                .delete(gov::reset_egress_policy::<B<S, C, G>>),
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
    let mut router = router.layer(RequestBodyLimitLayer::new(limits.max_body_bytes));
    // The event service is deployment-gated with the key ring required by its HMAC profiles.
    if serves_events {
        use routes::events as ev;
        router = router.merge(
            Router::new()
                .route(
                    "/namespaces/{namespace_id}/event-endpoints",
                    get(ev::list_endpoints::<B<S, C, G>>).post(ev::create_endpoint::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/event-endpoints/{endpoint_id}",
                    get(ev::get_endpoint::<B<S, C, G>>)
                        .put(ev::update_endpoint::<B<S, C, G>>)
                        .delete(ev::delete_endpoint::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/events/{source}/{event_id}",
                    get(ev::get_event::<B<S, C, G>>),
                )
                .route(
                    "/events",
                    axum::routing::post(ev::publish_event::<B<S, C, G>>),
                )
                .route(
                    "/webhooks/{endpoint_id}",
                    axum::routing::post(ev::receive_webhook::<B<S, C, G>>),
                )
                .layer(RequestBodyLimitLayer::new(ev::EVENT_MAX_BODY_BYTES)),
        );
    }
    if serves_outbound {
        use routes::outbound as ob;
        router = router.merge(
            Router::new()
                .route(
                    "/namespaces/{namespace_id}/webhook-subscriptions",
                    get(ob::list_subscriptions::<B<S, C, G>>)
                        .post(ob::create_subscription::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/webhook-subscriptions/{subscription_id}",
                    get(ob::get_subscription::<B<S, C, G>>)
                        .put(ob::update_subscription::<B<S, C, G>>)
                        .delete(ob::delete_subscription::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/webhook-subscriptions/{subscription_id}/deliveries",
                    get(ob::list_deliveries::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/webhook-subscriptions/{subscription_id}/deliveries/{delivery_id}",
                    get(ob::get_delivery::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/webhook-subscriptions/{subscription_id}/deliveries/{delivery_id}/redrive",
                    axum::routing::post(ob::redrive_delivery::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/webhook-subscriptions/{subscription_id}/redrive",
                    axum::routing::post(ob::redrive_dead::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/broker-subscriptions",
                    get(ob::list_broker_subscriptions::<B<S, C, G>>)
                        .post(ob::create_broker_subscription::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/broker-subscriptions/{subscription_id}",
                    get(ob::get_broker_subscription::<B<S, C, G>>)
                        .put(ob::update_broker_subscription::<B<S, C, G>>)
                        .delete(ob::delete_broker_subscription::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/broker-subscriptions/{subscription_id}/deliveries",
                    get(ob::list_broker_deliveries::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/broker-subscriptions/{subscription_id}/deliveries/{delivery_id}",
                    get(ob::get_broker_delivery::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/broker-subscriptions/{subscription_id}/deliveries/{delivery_id}/redrive",
                    axum::routing::post(ob::redrive_broker_delivery::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/broker-subscriptions/{subscription_id}/redrive",
                    axum::routing::post(ob::redrive_broker_dead::<B<S, C, G>>),
                )
                .layer(RequestBodyLimitLayer::new(limits.max_body_bytes)),
        );
    }
    if serves_connectors {
        use routes::connectors as cn;
        router = router.merge(
            Router::new()
                .route(
                    "/namespaces/{namespace_id}/connectors",
                    get(cn::list_connectors::<B<S, C, G>>)
                        .post(cn::create_connector::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/connectors/{connector_id}",
                    get(cn::get_connector::<B<S, C, G>>)
                        .put(cn::update_connector::<B<S, C, G>>)
                        .delete(cn::delete_connector::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/connectors/{connector_id}/dead-letters",
                    get(cn::list_dead_letters::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/connectors/{connector_id}/dead-letters/{dead_letter_id}",
                    get(cn::get_dead_letter::<B<S, C, G>>)
                        .delete(cn::discard_dead_letter::<B<S, C, G>>),
                )
                .route(
                    "/namespaces/{namespace_id}/connectors/{connector_id}/dead-letters/{dead_letter_id}/redrive",
                    axum::routing::post(cn::redrive_dead_letter::<B<S, C, G>>),
                )
                .layer(RequestBodyLimitLayer::new(limits.max_body_bytes)),
        );
    }
    // Without a signing key an artifact cannot be trusted: the script routes do not exist.
    if serves_scripts {
        use axum::routing::{get as get_, post};
        use routes::runner as rn;
        use routes::scripts as sc;
        router = router
            .merge(
                Router::new()
                    .route(
                        "/namespaces/{namespace_id}/scripts",
                        get_(sc::list_scripts::<B<S, C, G>>).post(sc::create_script::<B<S, C, G>>),
                    )
                    .route(
                        "/namespaces/{namespace_id}/scripts/{script_id}",
                        get_(sc::get_script::<B<S, C, G>>)
                            .patch(sc::update_script::<B<S, C, G>>)
                            .delete(sc::delete_script::<B<S, C, G>>),
                    )
                    .route(
                        "/namespaces/{namespace_id}/scripts/{script_id}/versions",
                        get_(sc::list_versions::<B<S, C, G>>)
                            .post(sc::publish_version::<B<S, C, G>>),
                    )
                    .route(
                        "/namespaces/{namespace_id}/scripts/{script_id}/versions/{number}",
                        get_(sc::get_version::<B<S, C, G>>),
                    )
                    .route(
                        "/namespaces/{namespace_id}/scripts/{script_id}/test-runs",
                        get_(sc::list_test_runs::<B<S, C, G>>)
                            .post(sc::create_test_run::<B<S, C, G>>),
                    )
                    .route(
                        "/namespaces/{namespace_id}/scripts/{script_id}/test-runs/{run_id}",
                        get_(sc::get_test_run::<B<S, C, G>>),
                    )
                    .route(
                        "/namespaces/{namespace_id}/script-executions",
                        get_(sc::list_executions::<B<S, C, G>>),
                    )
                    .route(
                        "/namespaces/{namespace_id}/script-executions/{execution_id}",
                        get_(sc::get_execution::<B<S, C, G>>),
                    )
                    .route(
                        "/script-artifacts/{digest}",
                        get_(sc::get_artifact::<B<S, C, G>>),
                    )
                    .layer(RequestBodyLimitLayer::new(sc::SCRIPT_MAX_BODY_BYTES)),
            )
            .merge(
                Router::new()
                    .route(
                        "/script-runner/builds/claim",
                        post(rn::claim_build::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/builds/{job_id}/heartbeat",
                        post(rn::heartbeat_build::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/builds/{job_id}/complete",
                        post(rn::complete_build::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/test-runs/claim",
                        post(rn::claim_test::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/test-runs/{run_id}/heartbeat",
                        post(rn::heartbeat_test::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/test-runs/{run_id}/complete",
                        post(rn::complete_test::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/artifacts/{digest}",
                        get_(rn::get_artifact::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/signing-key",
                        get_(rn::signing_key::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/namespaces/{namespace_id}/egress-policy",
                        get_(rn::egress_policy::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/namespaces/{namespace_id}/pending-activities",
                        get_(rn::pending_activities::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/executions",
                        post(rn::record_execution::<B<S, C, G>>),
                    )
                    .route(
                        "/script-runner/incidents",
                        post(rn::record_incident::<B<S, C, G>>),
                    )
                    .layer(RequestBodyLimitLayer::new(rn::RUNNER_MAX_BODY_BYTES))
                    // The `Json` extractor limits a body to 2 MiB by itself, which is under what an
                    // artifact of 2 MiB takes once it is encoded.
                    .layer(axum::extract::DefaultBodyLimit::max(
                        rn::RUNNER_MAX_BODY_BYTES,
                    )),
            );
    }
    router
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

    // Large streaming uploads have their own size/time budget. Ordinary JSON routes retain
    // their existing protection; no caller can smuggle a larger run request through this path.
    let uploads = if let Some(blobs) = state.runs().input_blobs() {
        let upload_limits = HttpLimits {
            max_body_bytes: usize::try_from(blobs.limits().max_bytes).unwrap_or(usize::MAX),
            request_timeout: state.blob_transfers().limits.timeout,
            ..limits
        };
        protect(
            Router::new()
                .route(
                    "/v1/run-input-blobs",
                    axum::routing::post(blobs::upload::<B<S, C, P, X, E>>),
                )
                .route_layer(shed())
                .route_layer(from_fn(context::record_route))
                .with_state(state.clone()),
            upload_limits,
        )
    } else {
        Router::new()
    };

    let identity_routes = Router::new()
        .route("/auth/context", get(identity::auth_context))
        .route("/version", get(identity::platform_version))
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

    // Approvals, the active version and promotion are served when the instance has the service.
    let versioning_routes = if state.publishing().is_some() {
        use axum::routing::{post, put};
        use routes::versioning as ver;
        Router::new()
            .route(
                "/namespaces/{namespace_id}/publication-policy",
                get(ver::get_policy::<B<S, C, P, X, E>>).put(ver::set_policy::<B<S, C, P, X, E>>),
            )
            .route(
                "/workflows/{workflow_id}/publication-requests",
                get(ver::list_requests::<B<S, C, P, X, E>>)
                    .post(ver::open_request::<B<S, C, P, X, E>>),
            )
            .route(
                "/publication-requests/{request_id}",
                get(ver::get_request::<B<S, C, P, X, E>>),
            )
            .route(
                "/publication-requests/{request_id}/approve",
                post(ver::approve_request::<B<S, C, P, X, E>>),
            )
            .route(
                "/publication-requests/{request_id}/reject",
                post(ver::reject_request::<B<S, C, P, X, E>>),
            )
            .route(
                "/publication-requests/{request_id}/withdraw",
                post(ver::withdraw_request::<B<S, C, P, X, E>>),
            )
            .route(
                "/publication-requests/{request_id}/publish",
                post(ver::publish_request::<B<S, C, P, X, E>>),
            )
            .route(
                "/workflows/{workflow_id}/activations",
                get(ver::list_activations::<B<S, C, P, X, E>>),
            )
            .route(
                "/workflows/{workflow_id}/active-version",
                put(ver::activate_version::<B<S, C, P, X, E>>),
            )
            .route(
                "/workflows/{workflow_id}/versions/{number}/promote",
                post(ver::promote_version::<B<S, C, P, X, E>>),
            )
    } else {
        Router::new()
    };

    let exec_routes = Router::new()
        .merge(versioning_routes)
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
            "/workflows/{workflow_id}/validate",
            axum::routing::post(ex::validate_workflow::<B<S, C, P, X, E>>),
        )
        .route(
            "/workflows/{workflow_id}/diff",
            get(ex::diff_workflow::<B<S, C, P, X, E>>),
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
            "/workflows/{workflow_id}/versions/{number}/compensation-plans",
            get(ex::get_compensation_plans::<B<S, C, P, X, E>>),
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
            "/runs/{run_id}/telemetry",
            get(ex::run_telemetry::<B<S, C, P, X, E>>),
        )
        .route(
            "/runs/{run_id}/attempts",
            get(ex::run_attempts::<B<S, C, P, X, E>>),
        )
        .route(
            "/runs/{run_id}/graph",
            get(ex::run_graph::<B<S, C, P, X, E>>),
        )
        .route(
            "/runs/{run_id}/graph/changes",
            get(ex::run_graph_changes::<B<S, C, P, X, E>>),
        )
        .route(
            "/runs/{run_id}/stream",
            get(ex::run_stream::<B<S, C, P, X, E>>),
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
        max_body_bytes: limits
            .max_body_bytes
            .max(EXEC_MAX_BODY_BYTES)
            .max(routes::runner::RUNNER_MAX_BODY_BYTES),
        ..limits
    };
    // Keep the protected fallback explicit: merging the upload router's default fallback must
    // not replace the error envelope, correlation headers and telemetry of unmatched routes.
    protect(
        Router::new()
            .nest("/v1", v1)
            .fallback(|| async { axum::http::StatusCode::NOT_FOUND }),
        outer,
    )
    .merge(uploads)
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

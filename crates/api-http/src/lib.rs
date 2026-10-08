//! HTTP adapter of the control plane.
//!
//! * [`auth`]: OIDC/JWT verification with a hardened JWKS cache.
//! * [`error`]: the single error envelope, completed with the trace id by [`context`].
//! * [`router`]: the Axum router with body-size, timeout and panic protection.

use std::sync::Arc;
use std::time::Duration;

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
mod routes;
mod server;

pub use auth::{AuthConfig, HttpJwksFetcher, TokenVerifier};
pub use server::{ServerConfig, serve};

/// Shared handler state. Cloning is cheap (two `Arc`s).
pub struct AppState<S, C> {
    pub identity: Arc<IdentityService<S, C>>,
    pub verifier: Arc<TokenVerifier>,
}

impl<S, C> AppState<S, C> {
    #[must_use]
    pub fn new(identity: IdentityService<S, C>, verifier: TokenVerifier) -> Self {
        Self {
            identity: Arc::new(identity),
            verifier: Arc::new(verifier),
        }
    }
}

impl<S, C> Clone for AppState<S, C> {
    fn clone(&self) -> Self {
        Self {
            identity: Arc::clone(&self.identity),
            verifier: Arc::clone(&self.verifier),
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
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 64 * 1024,
            request_timeout: Duration::from_secs(15),
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

    let v1 = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready::<S, C>))
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

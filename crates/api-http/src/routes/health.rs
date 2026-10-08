//! Liveness and readiness probes (FR-001-002). Neither requires credentials.

use axum::Json;
use axum::extract::State;
use sagat_application::clock::Clock;
use sagat_application::identity::IdentityStore;

use crate::AppState;
use crate::dto::HealthBody;
use crate::error::ApiError;

/// The process is up. Touches nothing else, so a database outage never restarts the pod.
pub async fn health() -> Json<HealthBody> {
    Json(HealthBody { status: "ok" })
}

/// The service can do useful work: storage answers.
pub async fn ready<S, C>(State(state): State<AppState<S, C>>) -> Result<Json<HealthBody>, ApiError>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    state.identity.ping().await?;
    Ok(Json(HealthBody { status: "ok" }))
}

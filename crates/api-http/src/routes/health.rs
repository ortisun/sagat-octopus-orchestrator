//! Liveness and readiness probes (FR-001-002). Neither requires credentials.

use axum::Json;
use axum::extract::State;
use sagat_application::clock::Clock;
use sagat_application::identity::IdentityStore;

use crate::AppState;
use crate::dto::HealthBody;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

use crate::error::ApiError;

/// The process is up. Touches nothing else, so a database outage never restarts the pod.
pub async fn health() -> Json<HealthBody> {
    Json(HealthBody { status: "ok" })
}

/// A probe result is reused for this long, so probe floods cost one query per interval.
const READY_TTL: Duration = Duration::from_secs(1);

/// The service can do useful work: storage answers.
pub async fn ready<S, C>(State(state): State<AppState<S, C>>) -> Result<Json<HealthBody>, ApiError>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    let cached = *state
        .ready_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let fresh = cached
        .filter(|(at, _)| at.elapsed() < READY_TTL)
        .map(|(_, ok)| ok);
    let ok = if let Some(ok) = fresh {
        ok
    } else {
        let ok = state.identity.ping().await.is_ok();
        *state
            .ready_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), ok));
        ok
    };
    if ok {
        Ok(Json(HealthBody { status: "ok" }))
    } else {
        Err(ApiError::unavailable())
    }
}

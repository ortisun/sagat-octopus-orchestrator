//! Liveness and readiness probes (FR-001-002). Neither requires credentials.

use std::sync::PoisonError;
use std::time::{Duration, Instant};

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

/// A probe result is reused for this long, so probe floods cost one query per interval.
const READY_TTL: Duration = Duration::from_secs(1);

/// A storage ping that takes longer than this counts as "not ready". A database that hangs must
/// not hang the probe (and with it every waiter queued behind the refresh).
const PING_TIMEOUT: Duration = Duration::from_millis(500);

/// The service can do useful work: storage answers.
///
/// The result is cached for [`READY_TTL`], and refreshing it is single-flight: concurrent probes
/// that find the cache stale wait for ONE ping instead of issuing their own, so a flood of probes
/// costs one query per interval no matter how many arrive at the same instant.
pub async fn ready<S, C>(State(state): State<AppState<S, C>>) -> Result<Json<HealthBody>, ApiError>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    let ok = if let Some(ok) = cached(&state) {
        ok
    } else {
        // Held across the ping on purpose. Whoever waits here re-checks the cache afterwards.
        let _refreshing = state.ready_refresh.lock().await;
        if let Some(ok) = cached(&state) {
            ok
        } else {
            let ok = probe(&state).await;
            *state
                .ready_cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), ok));
            ok
        }
    };
    if ok {
        Ok(Json(HealthBody { status: "ok" }))
    } else {
        Err(ApiError::unavailable())
    }
}

/// The last probe result, if it is still fresh.
fn cached<S, C>(state: &AppState<S, C>) -> Option<bool> {
    let cached = *state
        .ready_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    cached
        .filter(|(at, _)| at.elapsed() < READY_TTL)
        .map(|(_, ok)| ok)
}

/// One storage ping, bounded in time. The reason for a failure goes to the log (the response only
/// says "unavailable"; the store logs the cause in detail).
async fn probe<S, C>(state: &AppState<S, C>) -> bool
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    match tokio::time::timeout(PING_TIMEOUT, state.identity.ping()).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::warn!(%error, "readiness probe failed");
            false
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = u64::try_from(PING_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
                "readiness probe failed: storage did not answer in time"
            );
            false
        }
    }
}

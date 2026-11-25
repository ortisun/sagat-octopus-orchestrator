//! The rate-limit keys of a namespace (feature 022, FR-022-048; tasks T022-078, T022-111; plan "Visibility of the rate
//! gate").
//!
//! The protections of the concurrency-key listing, through its opening (`routes/concurrency.rs`, decision D022-068 item
//! 6): a namespace that belongs to another tenant is a `404` like one that does not exist; keys appear only as labels,
//! and only to a caller who may reveal personal data in the namespace, every other caller reading one row per group
//! (decision D022-086); the cursor is sealed, of this listing's own kind and view (D022-059, decision D022-067 item 5).

use axum::Json;
use axum::extract::{Path, Query, State};
use sagat_application::flow_control_keys::KeyListing;

use crate::dto_flow_control::RateLimitKeyPageBody;
use crate::error::ApiError;
use crate::extract::Caller;
use crate::governance::Governance;
use crate::routes::concurrency::{ConcurrencyKeyQuery, open_key_listing};

/// `GET /namespaces/{namespaceId}/rate-limit-keys`
pub async fn list_rate_limit_keys<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(query): Query<ConcurrencyKeyQuery>,
) -> Result<Json<RateLimitKeyPageBody>, ApiError> {
    let service = state.operations().ok_or_else(ApiError::not_found)?;
    let opened =
        open_key_listing(&caller, &state, &namespace, &query, KeyListing::RateLimit).await?;
    let page = service
        .rate_limit_keys(
            &caller.session.principal,
            opened.namespace,
            opened.group.as_ref(),
            opened.after.as_ref(),
            Some(opened.limit + 1),
            &caller.ctx,
        )
        .await?
        .ok_or_else(|| ApiError::bad_request("the cursor is not valid"))?;
    let body = RateLimitKeyPageBody::of(&page, opened.limit as usize, |last| {
        service.rate_limit_key_cursor(&caller.session.principal, opened.namespace, &page, last)
    })?;
    Ok(Json(body))
}

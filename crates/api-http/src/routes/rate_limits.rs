//! The rate-limit keys of a namespace (feature 022, FR-022-048; task T022-078; plan "Visibility of the rate gate").
//!
//! The protections of the concurrency-key listing (`routes/concurrency.rs`): a namespace that belongs to another tenant
//! is a `404` like one that does not exist; keys appear only as labels, and only to a caller who may reveal personal data
//! in the namespace (D022-060); the cursor is sealed, of this listing's own kind (D022-059, decision D022-067 item 5).

use axum::Json;
use axum::extract::{Path, Query, State};
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_domain::identity::NamespaceId;
use sagat_domain::workflow::ResolvedGroup;

use crate::admission::admit;
use crate::dto_flow_control::RateLimitKeyPageBody;
use crate::error::ApiError;
use crate::extract::Caller;
use crate::governance::Governance;
use crate::routes::concurrency::ConcurrencyKeyQuery;

/// `GET /namespaces/{namespaceId}/rate-limit-keys`
pub async fn list_rate_limit_keys<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(query): Query<ConcurrencyKeyQuery>,
) -> Result<Json<RateLimitKeyPageBody>, ApiError> {
    let service = state.operations().ok_or_else(ApiError::not_found)?;
    let namespace: NamespaceId = namespace.parse().map_err(|_| ApiError::not_found())?;
    let group = query
        .group
        .as_deref()
        .map(|g| {
            ResolvedGroup::parse(g)
                .map_err(|_| ApiError::validation("the group is not a valid group name"))
        })
        .transpose()?;
    // a cursor is this listing's position sealed for this namespace of this tenant: anything else, a concurrency
    // listing's included, is a `400` decided before admission, the same whether the namespace exists or not
    let after = query
        .cursor
        .as_deref()
        .map(|c| {
            service
                .rate_limit_key_position(&caller.session.principal, namespace, c)
                .ok_or_else(|| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    if let (Some(group), Some(after)) = (&group, &after)
        && after.group != *group
    {
        return Err(ApiError::bad_request("the cursor is not valid"));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    admit(
        &state,
        &caller,
        ApiOperation::OpsRead,
        ApiTarget::Namespace(namespace),
    )
    .await?;
    if let Some(cursor) = query.cursor.as_deref()
        && !service
            .concurrency_cursor_in_force(cursor)
            .await
            .map_err(ApiError::from)?
    {
        return Err(ApiError::bad_request("the cursor is not valid"));
    }
    let page = service
        .rate_limit_keys(
            &caller.session.principal,
            namespace,
            group.as_ref(),
            after.as_ref(),
            Some(limit + 1),
            &caller.ctx,
        )
        .await?;
    let body = RateLimitKeyPageBody::of(&page, limit as usize, |last| {
        service.rate_limit_key_cursor(&caller.session.principal, namespace, &page, last)
    })?;
    Ok(Json(body))
}

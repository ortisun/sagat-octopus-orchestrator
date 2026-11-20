//! The live concurrency keys of a namespace (feature 022, FR-022-022; T022-026).
//!
//! A namespace that belongs to another tenant is a `404` like one that does not exist. Keys appear only as labels, and
//! only to a caller who may reveal personal data in the namespace (D022-060); the cursor is sealed (D022-059).

use axum::Json;
use axum::extract::{Path, Query, State};
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_domain::identity::NamespaceId;
use sagat_domain::workflow::ResolvedGroup;
use serde::Deserialize;

use crate::admission::admit;
use crate::dto_flow_control::ConcurrencyKeyPageBody;
use crate::error::ApiError;
use crate::extract::Caller;
use crate::governance::Governance;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConcurrencyKeyQuery {
    /// An explicit group or a derived one (`=<workflow>` or `=<workflow>/<node>`).
    pub group: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// `GET /namespaces/{namespaceId}/concurrency-keys`
pub async fn list_concurrency_keys<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(query): Query<ConcurrencyKeyQuery>,
) -> Result<Json<ConcurrencyKeyPageBody>, ApiError> {
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
    // A cursor is the position sealed for this namespace of this tenant (D022-059): anything else, altered, or sealed
    // for another namespace is a `400`, decided before admission and the same whether the namespace exists or not. One
    // sealed under a digest key version out of force is a `400` too, asked after admission (it reads the database).
    let after = query
        .cursor
        .as_deref()
        .map(|c| {
            service
                .concurrency_key_position(&caller.session.principal, namespace, c)
                .ok_or_else(|| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    // a group's listing resumes only after a key of that group (the order within one group is the namespace's order)
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
        .concurrency_keys(
            &caller.session.principal,
            namespace,
            group.as_ref(),
            after.as_ref(),
            Some(limit + 1),
            &caller.ctx,
        )
        .await?;
    let body = ConcurrencyKeyPageBody::of(&page, limit as usize, |last| {
        service.concurrency_key_cursor(&caller.session.principal, namespace, &page, last)
    })?;
    Ok(Json(body))
}

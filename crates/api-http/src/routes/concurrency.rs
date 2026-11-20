//! The live concurrency keys of a namespace (feature 022, FR-022-022; T022-026).
//!
//! A namespace that belongs to another tenant is a `404` like one that does not exist. Keys appear only as labels.

use axum::Json;
use axum::extract::{Path, Query, State};
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_application::flow_control_keys::KeyPosition;
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
    let after = query
        .cursor
        .as_deref()
        .map(|c| {
            KeyPosition::decode(c).ok_or_else(|| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    admit(
        &state,
        &caller,
        ApiOperation::OpsRead,
        ApiTarget::Namespace(namespace),
    )
    .await?;
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
    Ok(Json(ConcurrencyKeyPageBody::of(&page, limit as usize)))
}

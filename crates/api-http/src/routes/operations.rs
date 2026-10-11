//! Operations: the alerts of a namespace (FR-008-005).
//!
//! A namespace that belongs to another tenant is a `404` like one that does not exist.

use axum::Json;
use axum::extract::{Path, Query, State};
use sagat_domain::identity::NamespaceId;
use sagat_domain::operations::AlertState;
use serde::Deserialize;
use uuid::Uuid;

use crate::dto_ops::{AlertBody, AlertPageBody};
use crate::error::ApiError;
use crate::extract::Caller;
use crate::governance::Governance;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlertQuery {
    /// `FIRING` or `RESOLVED`; both when absent.
    pub state: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// `GET /namespaces/{namespaceId}/alerts`
pub async fn list_alerts<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(query): Query<AlertQuery>,
) -> Result<Json<AlertPageBody>, ApiError> {
    let service = state.operations().ok_or_else(ApiError::not_found)?;
    let namespace: NamespaceId = namespace.parse().map_err(|_| ApiError::not_found())?;
    let wanted = query
        .state
        .as_deref()
        .map(|s| {
            s.parse::<AlertState>()
                .map_err(|()| ApiError::validation("the state is FIRING or RESOLVED"))
        })
        .transpose()?;
    let before = query
        .cursor
        .as_deref()
        .map(|c| {
            c.parse::<Uuid>()
                .map_err(|_| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let mut found = service
        .alerts(
            &caller.session.principal,
            namespace,
            wanted,
            before,
            Some(limit + 1),
            &caller.ctx,
        )
        .await?;
    let next_cursor = if found.len() > limit as usize {
        found.truncate(limit as usize);
        found.last().map(|a| a.id.to_string())
    } else {
        None
    };
    Ok(Json(AlertPageBody {
        items: found
            .iter()
            .map(|alert| AlertBody::of(alert, service.runbook(alert)))
            .collect(),
        next_cursor,
    }))
}

//! Operations: the alerts of a namespace (FR-008-005).
//!
//! A namespace that belongs to another tenant is a `404` like one that does not exist.

use axum::Json;
use axum::extract::{Path, Query, State};
use sagat_domain::identity::NamespaceId;
use sagat_domain::operations::AlertState;
use serde::Deserialize;
use uuid::Uuid;

use crate::admission::admit;
use crate::dto_ops::{
    AlertBody, AlertPageBody, AuditPageBody, AuditRecordBody, DrainBody, ReasonBody, UndrainBody,
    WorkerBody, WorkerListBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;
use sagat_application::api_admission::{ApiOperation, ApiTarget};

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
    admit(
        &state,
        &caller,
        ApiOperation::OpsRead,
        ApiTarget::Namespace(namespace),
    )
    .await?;
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

/// `GET /namespaces/{namespaceId}/workers`
pub async fn list_workers<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<WorkerListBody>, ApiError> {
    let service = state.operations().ok_or_else(ApiError::not_found)?;
    let namespace: NamespaceId = namespace.parse().map_err(|_| ApiError::not_found())?;
    admit(
        &state,
        &caller,
        ApiOperation::OpsRead,
        ApiTarget::Namespace(namespace),
    )
    .await?;
    let items = service
        .workers(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    let drains = service
        .drains(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    Ok(Json(WorkerListBody {
        items: items.iter().map(WorkerBody::from).collect(),
        drains: drains.iter().map(DrainBody::from).collect(),
    }))
}

/// `POST /namespaces/{namespaceId}/workers/{subject}/drain`
pub async fn drain_worker<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, subject)): Path<(String, String)>,
    ApiJson(body): ApiJson<ReasonBody>,
) -> Result<Json<DrainBody>, ApiError> {
    let service = state.operations().ok_or_else(ApiError::not_found)?;
    let namespace: NamespaceId = namespace.parse().map_err(|_| ApiError::not_found())?;
    admit(
        &state,
        &caller,
        ApiOperation::WorkerManage,
        ApiTarget::Namespace(namespace),
    )
    .await?;
    let record = service
        .drain(
            &caller.session.principal,
            namespace,
            &subject,
            &body.reason,
            &caller.ctx,
        )
        .await?;
    Ok(Json(DrainBody::from(&record)))
}

/// `POST /namespaces/{namespaceId}/workers/{subject}/undrain`
pub async fn undrain_worker<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, subject)): Path<(String, String)>,
    ApiJson(body): ApiJson<ReasonBody>,
) -> Result<Json<UndrainBody>, ApiError> {
    let service = state.operations().ok_or_else(ApiError::not_found)?;
    let namespace: NamespaceId = namespace.parse().map_err(|_| ApiError::not_found())?;
    admit(
        &state,
        &caller,
        ApiOperation::WorkerManage,
        ApiTarget::Namespace(namespace),
    )
    .await?;
    let was_draining = service
        .undrain(
            &caller.session.principal,
            namespace,
            &subject,
            &body.reason,
            &caller.ctx,
        )
        .await?;
    Ok(Json(UndrainBody { was_draining }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditQuery {
    pub action: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub actor: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

fn instant(
    raw: Option<&str>,
    what: &'static str,
) -> Result<Option<time::OffsetDateTime>, ApiError> {
    raw.map(|text| {
        time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
            .map_err(|_| ApiError::validation(what))
    })
    .transpose()
}

/// `GET /audit`
///
/// The current endpoint is tenant-wide and therefore exempt from namespace admission. If a
/// canonical namespace filter is added, that filtered request must be charged to its resolved
/// namespace exactly once before the audit query executes.
pub async fn list_audit<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditPageBody>, ApiError> {
    let service = state.operations().ok_or_else(ApiError::not_found)?;
    let bounded = |value: Option<String>, what: &'static str| -> Result<Option<String>, ApiError> {
        value
            .map(|v| {
                if v.is_empty() || v.chars().count() > 255 || v.chars().any(char::is_control) {
                    Err(ApiError::validation(what))
                } else {
                    Ok(v)
                }
            })
            .transpose()
    };
    let filter = sagat_application::operations::AuditFilter {
        action: bounded(query.action, "action is not valid")?,
        resource_type: bounded(query.resource_type, "resourceType is not valid")?,
        resource_id: bounded(query.resource_id, "resourceId is not valid")?,
        actor: bounded(query.actor, "actor is not valid")?,
        from: instant(query.from.as_deref(), "from is an RFC 3339 time")?,
        to: instant(query.to.as_deref(), "to is an RFC 3339 time")?,
    };
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
        .audit(
            &caller.session.principal,
            &filter,
            before,
            Some(limit + 1),
            &caller.ctx,
        )
        .await?;
    let next_cursor = if found.len() > limit as usize {
        found.truncate(limit as usize);
        found.last().map(|r| r.id.to_string())
    } else {
        None
    };
    Ok(Json(AuditPageBody {
        items: found.iter().map(AuditRecordBody::from).collect(),
        next_cursor,
    }))
}

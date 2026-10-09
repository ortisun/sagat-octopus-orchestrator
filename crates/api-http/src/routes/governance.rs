//! Quotas. The tenant is never read from the request; a namespace is addressed by its id, and one
//! that belongs to another tenant is a `404`, exactly like one that does not exist.

use axum::Json;
use axum::extract::{Path, State};
use sagat_domain::identity::NamespaceId;
use sagat_domain::quota::QuotaResource;

use crate::dto_gov::{QuotaBody, QuotaListBody, SetQuotaBody};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

fn namespace_id(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn resource(raw: &str) -> Result<QuotaResource, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

// ------------------------------------------------------------------------------ tenant level

pub async fn list_tenant_quotas<B: Governance>(
    caller: Caller,
    State(state): State<B>,
) -> Result<Json<QuotaListBody>, ApiError> {
    let views = state
        .quotas()
        .list(&caller.session.principal, None, &caller.ctx)
        .await?;
    Ok(Json(QuotaListBody {
        items: views.iter().map(QuotaBody::from).collect(),
    }))
}

pub async fn set_tenant_quota<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(name): Path<String>,
    ApiJson(body): ApiJson<SetQuotaBody>,
) -> Result<Json<QuotaBody>, ApiError> {
    let view = state
        .quotas()
        .set(
            &caller.session.principal,
            None,
            resource(&name)?,
            body.limit,
            &caller.ctx,
        )
        .await?;
    Ok(Json(QuotaBody::from(&view)))
}

pub async fn clear_tenant_quota<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(name): Path<String>,
) -> Result<Json<QuotaBody>, ApiError> {
    let view = state
        .quotas()
        .clear(
            &caller.session.principal,
            None,
            resource(&name)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(QuotaBody::from(&view)))
}

// ------------------------------------------------------------------------------ namespace level

pub async fn list_namespace_quotas<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<QuotaListBody>, ApiError> {
    let views = state
        .quotas()
        .list(
            &caller.session.principal,
            Some(namespace_id(&namespace)?),
            &caller.ctx,
        )
        .await?;
    Ok(Json(QuotaListBody {
        items: views.iter().map(QuotaBody::from).collect(),
    }))
}

pub async fn set_namespace_quota<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name)): Path<(String, String)>,
    ApiJson(body): ApiJson<SetQuotaBody>,
) -> Result<Json<QuotaBody>, ApiError> {
    let view = state
        .quotas()
        .set(
            &caller.session.principal,
            Some(namespace_id(&namespace)?),
            resource(&name)?,
            body.limit,
            &caller.ctx,
        )
        .await?;
    Ok(Json(QuotaBody::from(&view)))
}

pub async fn clear_namespace_quota<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<QuotaBody>, ApiError> {
    let view = state
        .quotas()
        .clear(
            &caller.session.principal,
            Some(namespace_id(&namespace)?),
            resource(&name)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(QuotaBody::from(&view)))
}

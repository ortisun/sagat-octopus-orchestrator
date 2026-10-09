//! Quotas. The tenant is never read from the request; a namespace is addressed by its id, and one
//! that belongs to another tenant is a `404`, exactly like one that does not exist.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sagat_domain::identity::NamespaceId;
use sagat_domain::quota::QuotaResource;

use crate::dto_gov::{
    BindingBody, BindingListBody, QuotaBody, QuotaListBody, RevokedBody, SecretAccessBody,
    SecretBody, SecretListBody, SetBindingBody, SetQuotaBody, SetSecretBody,
};
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

// ------------------------------------------------------------------------------ worker bindings

pub async fn list_bindings<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<BindingListBody>, ApiError> {
    let views = state
        .bindings()
        .list(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(BindingListBody {
        items: views.iter().map(BindingBody::listed).collect(),
    }))
}

pub async fn bind_worker<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, subject)): Path<(String, String)>,
    ApiJson(body): ApiJson<SetBindingBody>,
) -> Result<Json<BindingBody>, ApiError> {
    let view = state
        .bindings()
        .bind(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &subject,
            &body.activities,
            &caller.ctx,
        )
        .await?;
    Ok(Json(BindingBody::changed(&view)))
}

pub async fn unbind_worker<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, subject)): Path<(String, String)>,
) -> Result<Json<RevokedBody>, ApiError> {
    let revoked = state
        .bindings()
        .unbind(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &subject,
            &caller.ctx,
        )
        .await?;
    Ok(Json(RevokedBody::from(revoked)))
}

// ------------------------------------------------------------------------------ secrets

fn secrets<B: Governance>(
    state: &B,
) -> Result<&sagat_application::secrets::SecretService<B::Store, B::Clock>, ApiError> {
    state.secrets().ok_or_else(ApiError::not_found)
}

pub async fn list_secrets<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<SecretListBody>, ApiError> {
    let views = secrets(&state)?
        .list(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(SecretListBody {
        items: views.iter().map(SecretBody::from).collect(),
    }))
}

pub async fn put_secret<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name)): Path<(String, String)>,
    ApiJson(body): ApiJson<SetSecretBody>,
) -> Result<Json<SecretBody>, ApiError> {
    let view = secrets(&state)?
        .put(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &name,
            body.value.into_bytes(),
            &caller.ctx,
        )
        .await?;
    Ok(Json(SecretBody::from(&view)))
}

pub async fn delete_secret<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    secrets(&state)?
        .delete(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &name,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_secret_access<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<SecretAccessBody>, ApiError> {
    let subjects = secrets(&state)?
        .access(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &name,
            &caller.ctx,
        )
        .await?;
    Ok(Json(SecretAccessBody { subjects }))
}

pub async fn grant_secret_access<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name, subject)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    secrets(&state)?
        .grant(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &name,
            &subject,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke_secret_access<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name, subject)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    secrets(&state)?
        .revoke(
            &caller.session.principal,
            namespace_id(&namespace)?,
            &name,
            &subject,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

//! Quotas. The tenant is never read from the request; a namespace is addressed by its id, and one
//! that belongs to another tenant is a `404`, exactly like one that does not exist.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_domain::identity::NamespaceId;
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::ActivityName;

use crate::admission::admit;
use crate::dto_gov::{
    ActivityQuotaBody, ActivityQuotaListBody, BindingBody, BindingListBody, EgressPolicyBody,
    QuotaBody, QuotaListBody, RevokedBody, SecretAccessBody, SecretBody, SecretListBody,
    SetBindingBody, SetEgressPolicyBody, SetQuotaBody, SetSecretBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

fn namespace_id(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

async fn admitted_namespace<B: Governance>(
    state: &B,
    caller: &Caller,
    raw: &str,
    operation: ApiOperation,
) -> Result<NamespaceId, ApiError> {
    let namespace = namespace_id(raw)?;
    admit(state, caller, operation, ApiTarget::Namespace(namespace)).await?;
    Ok(namespace)
}

fn resource(raw: &str) -> Result<QuotaResource, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn activity(raw: &str) -> Result<ActivityName, ApiError> {
    ActivityName::parse(raw).map_err(|_| ApiError::not_found())
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
    // Quota policy is the recovery control for a zero API rate. The use case performs the same
    // permission check and canonical tenant/namespace lookup, but this route deliberately does not
    // debit the bucket it administers.
    let namespace = namespace_id(&namespace)?;
    let views = state
        .quotas()
        .list(&caller.session.principal, Some(namespace), &caller.ctx)
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
    let resource = resource(&name)?;
    let namespace = namespace_id(&namespace)?;
    let view = state
        .quotas()
        .set(
            &caller.session.principal,
            Some(namespace),
            resource,
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
    let resource = resource(&name)?;
    let namespace = namespace_id(&namespace)?;
    let view = state
        .quotas()
        .clear(
            &caller.session.principal,
            Some(namespace),
            resource,
            &caller.ctx,
        )
        .await?;
    Ok(Json(QuotaBody::from(&view)))
}

pub async fn list_activity_quotas<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<ActivityQuotaListBody>, ApiError> {
    let namespace = namespace_id(&namespace)?;
    let views = state
        .quotas()
        .list_activity_limits(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    Ok(Json(ActivityQuotaListBody {
        items: views.iter().map(ActivityQuotaBody::from).collect(),
    }))
}

pub async fn set_activity_quota<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, activity_name)): Path<(String, String)>,
    ApiJson(body): ApiJson<SetQuotaBody>,
) -> Result<Json<ActivityQuotaBody>, ApiError> {
    let activity = activity(&activity_name)?;
    let namespace = namespace_id(&namespace)?;
    let view = state
        .quotas()
        .set_activity_limit(
            &caller.session.principal,
            namespace,
            activity,
            body.limit,
            &caller.ctx,
        )
        .await?;
    Ok(Json(ActivityQuotaBody::from(&view)))
}

pub async fn clear_activity_quota<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, activity_name)): Path<(String, String)>,
) -> Result<Json<ActivityQuotaBody>, ApiError> {
    let activity = activity(&activity_name)?;
    let namespace = namespace_id(&namespace)?;
    let view = state
        .quotas()
        .clear_activity_limit(&caller.session.principal, namespace, activity, &caller.ctx)
        .await?;
    Ok(Json(ActivityQuotaBody::from(&view)))
}

// ------------------------------------------------------------------------------ worker bindings

pub async fn list_bindings<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<BindingListBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::WorkerBind).await?;
    let views = state
        .bindings()
        .list(&caller.session.principal, namespace, &caller.ctx)
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
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::WorkerBind).await?;
    let view = state
        .bindings()
        .bind(
            &caller.session.principal,
            namespace,
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
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::WorkerBind).await?;
    let revoked = state
        .bindings()
        .unbind(&caller.session.principal, namespace, &subject, &caller.ctx)
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
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::SecretRead).await?;
    let views = secrets(&state)?
        .list(&caller.session.principal, namespace, &caller.ctx)
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
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::SecretManage).await?;
    if body.required_provider.is_some() && state.managed_secrets().is_none() {
        return Err(sagat_application::error::AppError::Unavailable.into());
    }
    let view = if let Some(managed) = state.managed_secrets() {
        let request_id = body
            .request_id
            .ok_or(sagat_application::error::AppError::NotAllowed(
                "requestId is required for managed secrets",
            ))?;
        let expected_version =
            body.expected_version
                .ok_or(sagat_application::error::AppError::NotAllowed(
                    "expectedVersion is required for managed secrets",
                ))?;
        let value =
            sagat_domain::secrets::SecretValue::new(body.value.into_bytes()).map_err(|_| {
                sagat_application::error::AppError::NotAllowed("a secret value is 1 to 16384 bytes")
            })?;
        managed
            .put(
                &caller.session.principal,
                namespace,
                sagat_application::managed_secrets::ManagedSecretWrite {
                    name: &name,
                    value,
                    expected_version,
                    request_id: request_id.parse().map_err(|_| {
                        sagat_application::error::AppError::NotAllowed("requestId must be a UUID")
                    })?,
                },
                &caller.ctx,
            )
            .await?
    } else {
        secrets(&state)?
            .put(
                &caller.session.principal,
                namespace,
                &name,
                body.value.into_bytes(),
                &caller.ctx,
            )
            .await?
    };
    Ok(Json(SecretBody::from(&view)))
}

pub async fn migrate_secret<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name)): Path<(String, String)>,
    ApiJson(body): ApiJson<crate::dto_gov::MigrateSecretBody>,
) -> Result<Json<SecretBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::SecretManage).await?;
    let service = state.managed_secrets().ok_or_else(ApiError::not_found)?;
    let view = service
        .migrate(
            &caller.session.principal,
            namespace,
            &name,
            body.expected_version,
            body.request_id.parse().map_err(|_| {
                sagat_application::error::AppError::NotAllowed("requestId must be a UUID")
            })?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(SecretBody::from(&view)))
}

pub async fn reconcile_secret<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, request)): Path<(String, String)>,
) -> Result<Json<SecretBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::SecretManage).await?;
    let request = request.parse().map_err(|_| ApiError::not_found())?;
    let service = state.managed_secrets().ok_or_else(ApiError::not_found)?;
    let view = service
        .reconcile_request(&caller.session.principal, namespace, request, &caller.ctx)
        .await?;
    Ok(Json(SecretBody::from(&view)))
}

pub async fn delete_secret<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::SecretManage).await?;
    secrets(&state)?
        .delete(&caller.session.principal, namespace, &name, &caller.ctx)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_secret_access<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<SecretAccessBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::SecretManage).await?;
    let subjects = secrets(&state)?
        .access(&caller.session.principal, namespace, &name, &caller.ctx)
        .await?;
    Ok(Json(SecretAccessBody { subjects }))
}

pub async fn grant_secret_access<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, name, subject)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::SecretManage).await?;
    secrets(&state)?
        .grant(
            &caller.session.principal,
            namespace,
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
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::SecretManage).await?;
    secrets(&state)?
        .revoke(
            &caller.session.principal,
            namespace,
            &name,
            &subject,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------------------ egress policy

pub async fn get_egress_policy<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<EgressPolicyBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EgressManage).await?;
    let policy = state
        .egress()
        .get(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    Ok(Json(EgressPolicyBody::from(&policy)))
}

pub async fn set_egress_policy<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    ApiJson(body): ApiJson<SetEgressPolicyBody>,
) -> Result<Json<EgressPolicyBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EgressManage).await?;
    let policy = state
        .egress()
        .set(
            &caller.session.principal,
            namespace,
            &body.rules,
            &body.ports,
            body.allow_http,
            &caller.ctx,
        )
        .await?;
    Ok(Json(EgressPolicyBody::from(&policy)))
}

pub async fn reset_egress_policy<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<StatusCode, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::EgressManage).await?;
    state
        .egress()
        .reset(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

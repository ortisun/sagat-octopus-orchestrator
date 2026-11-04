//! Batch operations (feature 018): preview, create, read, list, items and cancel.
//!
//! Admission comes first and is what authorizes the caller for the namespace of the path; the service
//! then asks for `run:batch` and every permission of the single action. A job of any other namespace
//! (or tenant) is as absent as one that never existed. The Idempotency-Key is required, as for every
//! operation that changes something; the creation is replayed by it, the cancellation is idempotent
//! by itself (a job being cancelled is cancelled once) and only checks the key is there.

use std::str::FromStr;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_application::batch_api::BatchUseCase;
use sagat_application::identity::PageRequest;
use sagat_domain::batch::JobState;
use sagat_domain::identity::NamespaceId;
use serde::Deserialize;
use uuid::Uuid;

use crate::admission::admit;
use crate::dto_batch::{
    CancelBody, CreateBody, ItemBody, ItemPageBody, JobBody, JobPageBody, PreviewBody,
    PreviewResponse,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

use super::execution::idempotency_key;

/// Largest body of the batch routes: the payload of a signal or an update (256 KiB) and room for the
/// selector and the envelope. A payload over its own limit is refused by the service with a reason.
pub const BATCH_MAX_BODY_BYTES: usize = 288 * 1024;

fn batches<B: Governance>(state: &B) -> Result<&std::sync::Arc<dyn BatchUseCase>, ApiError> {
    state.batches().ok_or_else(ApiError::not_found)
}

fn namespace_id(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn batch_id(raw: &str) -> Result<Uuid, ApiError> {
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

/// The job of the path, which must be in the namespace of the path.
async fn admitted_job<B: Governance>(
    state: &B,
    caller: &Caller,
    raw_namespace: &str,
    raw_id: &str,
    operation: ApiOperation,
) -> Result<Uuid, ApiError> {
    let namespace = admitted_namespace(state, caller, raw_namespace, operation).await?;
    let id = batch_id(raw_id)?;
    let found = batches(state)?
        .namespace_of(&caller.session.principal, id)
        .await?;
    if found != namespace {
        return Err(ApiError::not_found());
    }
    Ok(id)
}

pub async fn preview_batch<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    ApiJson(body): ApiJson<PreviewBody>,
) -> Result<Json<PreviewResponse>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::BatchPreview).await?;
    let selector = body.selector.into_selector()?;
    let preview = batches(&state)?
        .preview(
            &caller.session.principal,
            namespace,
            &selector,
            body.max_items,
        )
        .await?;
    Ok(Json(PreviewResponse::from(&preview)))
}

pub async fn create_batch<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<CreateBody>,
) -> Result<(StatusCode, Json<JobBody>), ApiError> {
    let key = idempotency_key(&headers)?;
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::BatchWrite).await?;
    let write = body.into_write()?;
    let job = batches(&state)?
        .create(
            &caller.session.principal,
            namespace,
            write,
            key,
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::ACCEPTED, Json(JobBody::from(&job))))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub state: Option<String>,
}

/// A cursor is what the service handed out: a job id for jobs, an ordinal for items.
fn cursor_is<T: FromStr>(cursor: Option<&str>) -> Result<(), ApiError> {
    match cursor {
        Some(cursor) if cursor.parse::<T>().is_err() => {
            Err(ApiError::bad_request("the cursor is not valid"))
        }
        _ => Ok(()),
    }
}

fn named<T: FromStr>(raw: Option<&str>) -> Result<Option<T>, ApiError> {
    raw.map(|name| {
        name.parse::<T>()
            .map_err(|_| ApiError::bad_request("the state is not valid"))
    })
    .transpose()
}

pub async fn list_batches<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<JobPageBody>, ApiError> {
    let wanted = named::<JobState>(query.state.as_deref())?;
    cursor_is::<Uuid>(query.cursor.as_deref())?;
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::BatchRead).await?;
    let page = batches(&state)?
        .list(
            &caller.session.principal,
            namespace,
            wanted,
            &PageRequest {
                limit: query.limit,
                cursor: query.cursor,
            },
        )
        .await?;
    Ok(Json(JobPageBody {
        items: page.items.iter().map(JobBody::from).collect(),
        next_cursor: page.next_cursor,
    }))
}

pub async fn get_batch<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<Json<JobBody>, ApiError> {
    let id = admitted_job(&state, &caller, &namespace, &id, ApiOperation::BatchRead).await?;
    let job = batches(&state)?.get(&caller.session.principal, id).await?;
    Ok(Json(JobBody::from(&job)))
}

pub async fn cancel_batch<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<CancelBody>,
) -> Result<(StatusCode, Json<JobBody>), ApiError> {
    idempotency_key(&headers)?;
    let id = admitted_job(&state, &caller, &namespace, &id, ApiOperation::BatchWrite).await?;
    body.check()?;
    let job = batches(&state)?
        .cancel(&caller.session.principal, id, &caller.ctx)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(JobBody::from(&job))))
}

pub async fn list_batch_items<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    Query(query): Query<ListQuery>,
) -> Result<Json<ItemPageBody>, ApiError> {
    let wanted = query
        .state
        .as_deref()
        .map(|name| {
            sagat_application::batch::item_state_from(name)
                .map_err(|_| ApiError::bad_request("the state is not valid"))
        })
        .transpose()?;
    cursor_is::<u32>(query.cursor.as_deref())?;
    let id = admitted_job(&state, &caller, &namespace, &id, ApiOperation::BatchRead).await?;
    let page = batches(&state)?
        .items(
            &caller.session.principal,
            id,
            wanted,
            &PageRequest {
                limit: query.limit,
                cursor: query.cursor,
            },
        )
        .await?;
    Ok(Json(ItemPageBody {
        items: page.items.iter().map(ItemBody::from).collect(),
        next_cursor: page.next_cursor,
    }))
}

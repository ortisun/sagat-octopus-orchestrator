//! Schedules (feature 018): create, read, list, replace, delete, pause and resume.
//!
//! Admission comes first and is what authorizes the caller for the namespace of the path; a schedule
//! of any other namespace (or tenant) is then as absent as one that never existed. The Idempotency-Key
//! is required, as for every operation that changes something: a client that sends a request again
//! after a lost answer is told what the first one did (a name in use, a stale revision, a schedule
//! that is gone), never applies it twice.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_application::schedules::ScheduleUseCase;
use sagat_domain::identity::NamespaceId;
use sagat_domain::schedules::{Note, ScheduleId};
use serde::Deserialize;

use crate::admission::admit;
use crate::dto_schedules::{
    ScheduleActionBody, ScheduleBody, ScheduleDescriptionBody, ScheduleNoteBody, SchedulePageBody,
    ScheduleUpdateBody, ScheduleWriteBody, TriggerBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

use super::execution::idempotency_key;

/// Largest body of the schedule routes: the input of the runs (64 KiB) and room for the spec, the
/// policies and the envelope. An input over its own limit is refused by the service with a reason.
pub const SCHEDULE_MAX_BODY_BYTES: usize = 96 * 1024;

fn schedules<B: Governance>(state: &B) -> Result<&std::sync::Arc<dyn ScheduleUseCase>, ApiError> {
    state.schedules().ok_or_else(ApiError::not_found)
}

fn namespace_id(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn schedule_id(raw: &str) -> Result<ScheduleId, ApiError> {
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

/// The schedule of the path, which must be in the namespace of the path.
async fn admitted_schedule<B: Governance>(
    state: &B,
    caller: &Caller,
    raw_namespace: &str,
    raw_id: &str,
    operation: ApiOperation,
) -> Result<ScheduleId, ApiError> {
    let namespace = admitted_namespace(state, caller, raw_namespace, operation).await?;
    let id = schedule_id(raw_id)?;
    let found = schedules(state)?
        .namespace_of(&caller.session.principal, id)
        .await?;
    if found != namespace {
        return Err(ApiError::not_found());
    }
    Ok(id)
}

fn note(body: ScheduleNoteBody) -> Result<Note, ApiError> {
    if body.note.is_empty() {
        return Err(ApiError::validation("the note must not be empty"));
    }
    Note::new(&body.note).map_err(|_| ApiError::validation("the note is at most 512 characters"))
}

pub async fn create_schedule<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ScheduleWriteBody>,
) -> Result<(StatusCode, Json<ScheduleBody>), ApiError> {
    idempotency_key(&headers)?;
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::ScheduleWrite).await?;
    let created = schedules(&state)?
        .create(
            &caller.session.principal,
            namespace,
            body.into_document(),
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(ScheduleBody::from(&created))))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_schedules<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<SchedulePageBody>, ApiError> {
    let after = query
        .cursor
        .as_deref()
        .map(|c| {
            c.parse::<ScheduleId>()
                .map_err(|_| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::ScheduleRead).await?;
    // one more than a page, to know whether there is a next one
    let mut found = schedules(&state)?
        .list(&caller.session.principal, namespace, after, limit + 1)
        .await?;
    let next_cursor = if found.len() > limit as usize {
        found.truncate(limit as usize);
        found.last().map(|s| s.id.to_string())
    } else {
        None
    };
    Ok(Json(SchedulePageBody {
        items: found.iter().map(ScheduleBody::from).collect(),
        next_cursor,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeQuery {
    pub next_count: Option<u32>,
    pub recent_count: Option<u32>,
}

pub async fn get_schedule<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    Query(query): Query<DescribeQuery>,
) -> Result<Json<ScheduleDescriptionBody>, ApiError> {
    let id =
        admitted_schedule(&state, &caller, &namespace, &id, ApiOperation::ScheduleRead).await?;
    let description = schedules(&state)?
        .describe(
            &caller.session.principal,
            id,
            query.next_count.unwrap_or(10).min(100),
            query.recent_count.unwrap_or(10).min(100),
        )
        .await?;
    Ok(Json(ScheduleDescriptionBody::from(&description)))
}

pub async fn update_schedule<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ScheduleUpdateBody>,
) -> Result<Json<ScheduleBody>, ApiError> {
    idempotency_key(&headers)?;
    let id = admitted_schedule(
        &state,
        &caller,
        &namespace,
        &id,
        ApiOperation::ScheduleWrite,
    )
    .await?;
    let (document, expected_revision) = body.into_parts();
    let updated = schedules(&state)?
        .update(
            &caller.session.principal,
            id,
            document,
            expected_revision,
            &caller.ctx,
        )
        .await?;
    Ok(Json(ScheduleBody::from(&updated)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteQuery {
    pub expected_revision: Option<u32>,
}

pub async fn delete_schedule<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    Query(query): Query<DeleteQuery>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    idempotency_key(&headers)?;
    let expected_revision = query
        .expected_revision
        .filter(|revision| *revision >= 1)
        .ok_or_else(|| ApiError::bad_request("the expectedRevision query parameter is required"))?;
    let id = admitted_schedule(
        &state,
        &caller,
        &namespace,
        &id,
        ApiOperation::ScheduleWrite,
    )
    .await?;
    schedules(&state)?
        .delete(
            &caller.session.principal,
            id,
            expected_revision,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pause_schedule<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ScheduleNoteBody>,
) -> Result<Json<ScheduleBody>, ApiError> {
    idempotency_key(&headers)?;
    let note = note(body)?;
    let id = admitted_schedule(
        &state,
        &caller,
        &namespace,
        &id,
        ApiOperation::ScheduleOperate,
    )
    .await?;
    let paused = schedules(&state)?
        .pause(&caller.session.principal, id, note, &caller.ctx)
        .await?;
    Ok(Json(ScheduleBody::from(&paused)))
}

pub async fn resume_schedule<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ScheduleNoteBody>,
) -> Result<Json<ScheduleBody>, ApiError> {
    idempotency_key(&headers)?;
    let note = note(body)?;
    let id = admitted_schedule(
        &state,
        &caller,
        &namespace,
        &id,
        ApiOperation::ScheduleResume,
    )
    .await?;
    let resumed = schedules(&state)?
        .resume(&caller.session.principal, id, note, &caller.ctx)
        .await?;
    Ok(Json(ScheduleBody::from(&resumed)))
}

pub async fn trigger_schedule<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<ScheduleActionBody>), ApiError> {
    let key = idempotency_key(&headers)?;
    // the whole body is optional
    let request = if body.is_empty() {
        TriggerBody::default()
    } else {
        serde_json::from_slice::<TriggerBody>(&body)
            .map_err(|_| ApiError::validation("request body does not match the expected schema"))?
    };
    let id = admitted_schedule(
        &state,
        &caller,
        &namespace,
        &id,
        ApiOperation::ScheduleTrigger,
    )
    .await?;
    let action = schedules(&state)?
        .trigger(
            &caller.session.principal,
            id,
            request.overlap.map(Into::into),
            key,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ScheduleActionBody::from(&action)),
    ))
}

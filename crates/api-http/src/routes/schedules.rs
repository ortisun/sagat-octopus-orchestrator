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
use axum::response::Response;
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_application::schedules::{
    ActionCursor, ScheduleFilter, ScheduleOrigin, ScheduleState, ScheduleUseCase,
};
use sagat_domain::identity::NamespaceId;
use sagat_domain::schedules::{Note, Outcome, ScheduleId};
use serde::Deserialize;
use uuid::Uuid;

use crate::admission::admit;
use crate::dto_schedules::{
    BackfillBody, BackfillListBody, BackfillRequestBody, ScheduleActionBody,
    ScheduleActionPageBody, ScheduleBody, ScheduleDescriptionBody, ScheduleNoteBody,
    SchedulePageBody, ScheduleUpdateBody, ScheduleWriteBody, TriggerBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

use super::execution::idempotency_key;
use super::payload_disclosure::{disclosure_error, disclosure_response, encode_schedule_input};

/// The body of `discloseScheduleInput`: whether the PII reveal is asked for. Nothing else is accepted.
#[derive(Debug, Deserialize)]
pub struct ScheduleInputDisclosureBody {
    reveal: Option<Vec<String>>,
    #[serde(flatten)]
    unexpected: std::collections::BTreeMap<String, serde::de::IgnoredAny>,
}

impl ScheduleInputDisclosureBody {
    fn reveal_pii(self) -> Result<bool, ApiError> {
        if !self.unexpected.is_empty() {
            return Err(ApiError::bad_request(
                "request body contains an unknown field",
            ));
        }
        match self.reveal.as_deref() {
            None | Some([]) => Ok(false),
            Some([level]) if level == "pii" => Ok(true),
            Some(_) => Err(ApiError::validation(
                "reveal may contain only the single value pii",
            )),
        }
    }
}

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
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub state: Option<String>,
    pub origin: Option<String>,
    pub workflow_id: Option<String>,
    pub name: Option<String>,
}

/// The filter a query asks for, or why it is not one. A listing never shows a deleted schedule, so
/// `DELETED` is not a state to ask for.
fn filter_of(query: &ListQuery) -> Result<ScheduleFilter, ApiError> {
    let state = query
        .state
        .as_deref()
        .map(|text| match text.parse::<ScheduleState>() {
            Ok(state @ (ScheduleState::Active | ScheduleState::Paused)) => Ok(state),
            _ => Err(ApiError::bad_request("state is ACTIVE or PAUSED")),
        })
        .transpose()?;
    let origin = query
        .origin
        .as_deref()
        .map(|text| {
            text.parse::<ScheduleOrigin>()
                .map_err(|()| ApiError::bad_request("origin is API or TRIGGER"))
        })
        .transpose()?;
    let workflow = query
        .workflow_id
        .as_deref()
        .map(|text| {
            Uuid::parse_str(text).map_err(|_| ApiError::bad_request("workflowId is not a UUID"))
        })
        .transpose()?;
    let name_prefix = query
        .name
        .as_deref()
        .map(|text| {
            if text.chars().count() > 128 {
                Err(ApiError::bad_request("name is at most 128 characters"))
            } else {
                Ok(text.to_owned())
            }
        })
        .transpose()?;
    Ok(ScheduleFilter {
        state,
        origin,
        workflow,
        name_prefix,
    })
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
    let filter = filter_of(&query)?;
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::ScheduleRead).await?;
    // one more than a page, to know whether there is a next one
    let mut found = schedules(&state)?
        .list(
            &caller.session.principal,
            namespace,
            &filter,
            after,
            limit + 1,
        )
        .await?;
    let next_cursor = if found.len() > limit as usize {
        found.truncate(limit as usize);
        found.last().map(|s| s.id.to_string())
    } else {
        None
    };
    // one read of the handover for the page, and only when a schedule of a cron trigger is on it
    let control_open = if found
        .iter()
        .any(|s| s.origin == sagat_application::schedules::ScheduleOrigin::Trigger)
    {
        schedules(&state)?.trigger_control_open().await?
    } else {
        false
    };
    Ok(Json(SchedulePageBody {
        items: found
            .iter()
            .map(|record| ScheduleBody::from(record).controllable_when(control_open))
            .collect(),
        next_cursor,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeQuery {
    pub next_count: Option<u32>,
    pub recent_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ActionsQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub outcome: Option<String>,
}

/// The decisions of a schedule, newest first, optionally of one outcome.
pub async fn list_schedule_actions<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    Query(query): Query<ActionsQuery>,
) -> Result<Json<ScheduleActionPageBody>, ApiError> {
    let after = query
        .cursor
        .as_deref()
        .map(|c| {
            ActionCursor::decode(c).ok_or_else(|| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    let outcome = query
        .outcome
        .as_deref()
        .map(|name| {
            name.parse::<Outcome>()
                .map_err(|_| ApiError::bad_request("the outcome is not valid"))
        })
        .transpose()?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let id =
        admitted_schedule(&state, &caller, &namespace, &id, ApiOperation::ScheduleRead).await?;
    // one more than a page, to know whether there is a next one
    let mut found = schedules(&state)?
        .actions(&caller.session.principal, id, outcome, after, limit + 1)
        .await?;
    let next_cursor = if found.len() > limit as usize {
        found.truncate(limit as usize);
        found
            .last()
            .map(|action| ActionCursor::after(action).encode())
    } else {
        None
    };
    Ok(Json(ScheduleActionPageBody {
        items: found.iter().map(ScheduleActionBody::from).collect(),
        next_cursor,
    }))
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
    let mut body = ScheduleDescriptionBody::from(&description);
    if description.record.origin == sagat_application::schedules::ScheduleOrigin::Trigger {
        let open = schedules(&state)?.trigger_control_open().await?;
        body.schedule = body.schedule.controllable_when(open);
    }
    Ok(Json(body))
}

/// `POST …/schedules/{scheduleId}/input-disclosures` (`FR-018-042`): the values of the input of a
/// schedule, projected for the audiences of feature 016 and audited as `schedule.input_disclosed` in
/// the read's transaction. Every response is `private, no-store`.
pub async fn disclose_schedule_input<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<ScheduleInputDisclosureBody>,
) -> Result<Response, ApiError> {
    let reveal_pii = body.reveal_pii()?;
    let disclosures = state
        .schedule_input_disclosures()
        .ok_or_else(ApiError::not_found)?;
    let id =
        admitted_schedule(&state, &caller, &namespace, &id, ApiOperation::ScheduleRead).await?;
    let disclosure = disclosures
        .disclose_input(
            &caller.session.principal,
            namespace_id(&namespace)?,
            id,
            reveal_pii,
            &caller.ctx,
        )
        .await
        .map_err(disclosure_error)?;
    Ok(disclosure_response(encode_schedule_input(&disclosure)?))
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
    // a pause of a schedule a cron trigger owns succeeded: the control is open
    Ok(Json(ScheduleBody::from(&paused).controllable_when(true)))
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
    Ok(Json(ScheduleBody::from(&resumed).controllable_when(true)))
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

pub async fn create_schedule_backfill<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<BackfillRequestBody>,
) -> Result<(StatusCode, Json<BackfillBody>), ApiError> {
    idempotency_key(&headers)?;
    let id = admitted_schedule(
        &state,
        &caller,
        &namespace,
        &id,
        ApiOperation::ScheduleBackfill,
    )
    .await?;
    let backfill = schedules(&state)?
        .create_backfill(
            &caller.session.principal,
            id,
            sagat_application::schedules::BackfillRequest {
                start: body.start,
                end: body.end,
                overlap: body.overlap.map(Into::into),
            },
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::ACCEPTED, Json(BackfillBody::from(&backfill))))
}

pub async fn list_schedule_backfills<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<Json<BackfillListBody>, ApiError> {
    let id =
        admitted_schedule(&state, &caller, &namespace, &id, ApiOperation::ScheduleRead).await?;
    let backfills = schedules(&state)?
        .backfills(&caller.session.principal, id)
        .await?;
    Ok(Json(BackfillListBody {
        items: backfills.iter().map(BackfillBody::from).collect(),
    }))
}

pub async fn cancel_schedule_backfill<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id, backfill)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Json<BackfillBody>, ApiError> {
    idempotency_key(&headers)?;
    let backfill = backfill
        .parse::<uuid::Uuid>()
        .map_err(|_| ApiError::not_found())?;
    let id = admitted_schedule(
        &state,
        &caller,
        &namespace,
        &id,
        ApiOperation::ScheduleOperate,
    )
    .await?;
    let stopped = schedules(&state)?
        .cancel_backfill(&caller.session.principal, id, backfill, &caller.ctx)
        .await?;
    Ok(Json(BackfillBody::from(&stopped)))
}

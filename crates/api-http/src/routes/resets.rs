//! Reset to a point (feature 018, US06, `FR-018-035`..`FR-018-037`): the points a run can be reset to, what a
//! reset would do, the reset itself, and the register of the resets made from a run.
//!
//! Admission comes first and authorizes the caller for the namespace of the run; the service then decides
//! what the run allows. A run of another tenant is as absent as one that never existed. The `Idempotency-Key`
//! is required by the reset, which creates a run: a retry is answered with the entry of the first request.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_application::error::AppError;
use sagat_application::runs::{CANCEL_SOURCE_NOT_AVAILABLE, REAPPLY_NOT_AVAILABLE, ResetRequest};
use sagat_domain::execution::{ResetId, ResetReapply, RunId};
use serde::Deserialize;

use crate::admission::admit;
use crate::dto::PageBody;
use crate::dto_exec::RerunBusinessIdBody;
use crate::dto_resets::{
    ReapplyPolicyBody, ResetBody, ResetListBody, ResetPlanBody, ResetPointBody, ResetPreviewBody,
    ResetRequestBody, SourcePolicyBody,
};
use crate::error::ApiError;
use crate::exec::Exec;
use crate::extract::{ApiJson, Caller};

use super::execution::{holder_for_readers, idempotency_key, parse_id};

/// The page of points, as the contract says: 1 to 200, 50 by default.
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

#[derive(Debug, Default, Deserialize)]
pub struct PointsQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// A cursor over the points is the sequence of the last one served.
fn decode_cursor(cursor: &str) -> Result<u64, ApiError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| ApiError::bad_request("the cursor is not valid"))?;
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| ApiError::bad_request("the cursor is not valid"))?;
    Ok(u64::from_be_bytes(bytes))
}

fn encode_cursor(sequence: u64) -> String {
    URL_SAFE_NO_PAD.encode(sequence.to_be_bytes())
}

fn reapply_of(policy: Option<ReapplyPolicyBody>) -> Result<ResetReapply, AppError> {
    match policy.map_or(ResetReapply::None, ResetReapply::from) {
        ResetReapply::None => Ok(ResetReapply::None),
        _ => Err(AppError::NotAllowed(REAPPLY_NOT_AVAILABLE)),
    }
}

/// `GET /runs/{runId}/reset-points`: where a run can be reset to, by ascending sequence.
pub async fn list_reset_points<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    Query(query): Query<PointsQuery>,
) -> Result<Json<PageBody<ResetPointBody>>, ApiError> {
    let id = parse_id::<RunId>(&id)?;
    admit(
        &state,
        &caller,
        ApiOperation::ResetInspect,
        ApiTarget::Run(id),
    )
    .await?;
    // as the shared `limit` parameter says: outside 1..200 is clamped
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let after = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let points = state
        .runs()
        .reset_points(&caller.session.principal, id, &caller.ctx)
        .await?;
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut page: Vec<_> = points
        .iter()
        .filter(|point| after.is_none_or(|after| point.sequence > after))
        .take(limit.saturating_add(1))
        .collect();
    let next_cursor = if page.len() > limit {
        page.truncate(limit);
        page.last().map(|point| encode_cursor(point.sequence))
    } else {
        None
    };
    Ok(Json(PageBody {
        items: page.into_iter().map(ResetPointBody::from).collect(),
        next_cursor,
    }))
}

/// `POST /runs/{runId}/reset-previews`: what a reset to a point would restore and run again; changes nothing.
pub async fn preview_reset<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ResetPreviewBody>,
) -> Result<Json<ResetPlanBody>, ApiError> {
    let id = parse_id::<RunId>(&id)?;
    admit(
        &state,
        &caller,
        ApiOperation::ResetInspect,
        ApiTarget::Run(id),
    )
    .await?;
    if body.sequence == 0 {
        return Err(ApiError::validation("the sequence is at least 1"));
    }
    reapply_of(body.reapply)?;
    let plan = state
        .runs()
        .reset_preview(&caller.session.principal, id, body.sequence, &caller.ctx)
        .await?;
    Ok(Json(ResetPlanBody::from(&plan)))
}

/// `POST /runs/{runId}/resets`: a new run from the prefix of an ended one.
pub async fn reset_run<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ResetRequestBody>,
) -> Result<(StatusCode, Json<ResetBody>), ApiError> {
    let id = parse_id::<RunId>(&id)?;
    // the header is part of the typed request: refuse it before admission spends a token
    let key = idempotency_key(&headers)?;
    admit(&state, &caller, ApiOperation::RunReset, ApiTarget::Run(id)).await?;
    if body.sequence == 0 {
        return Err(ApiError::validation("the sequence is at least 1"));
    }
    let reapply = reapply_of(body.reapply)?;
    if matches!(body.source_policy, Some(SourcePolicyBody::CancelSource)) {
        return Err(AppError::NotAllowed(CANCEL_SOURCE_NOT_AVAILABLE).into());
    }
    let business = match body.business_id {
        None | Some(RerunBusinessIdBody::Keep) => sagat_application::runs::RerunBusinessId::Keep,
        Some(RerunBusinessIdBody::Drop) => sagat_application::runs::RerunBusinessId::Drop,
    };
    let made = state
        .runs()
        .reset(
            &caller.session.principal,
            id,
            ResetRequest {
                sequence: body.sequence,
                reason: &body.reason,
                acknowledge_reexecution: body.acknowledge_reexecution,
                business,
                reapply,
            },
            key,
            &caller.ctx,
        )
        .await
        .map_err(|error| holder_for_readers(error, &caller))?;
    Ok((StatusCode::ACCEPTED, Json(ResetBody::from(&made.reset))))
}

/// `GET /runs/{runId}/resets`: the resets made from this run, newest first.
pub async fn list_run_resets<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
) -> Result<Json<ResetListBody>, ApiError> {
    let id = parse_id::<RunId>(&id)?;
    admit(&state, &caller, ApiOperation::RunRead, ApiTarget::Run(id)).await?;
    let resets = state
        .runs()
        .resets(&caller.session.principal, id, &caller.ctx)
        .await?;
    Ok(Json(ResetListBody {
        items: resets.iter().map(ResetBody::from).collect(),
    }))
}

/// `GET /runs/{runId}/resets/{resetId}`: one reset made from this run.
pub async fn get_run_reset<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path((id, reset)): Path<(String, String)>,
) -> Result<Json<ResetBody>, ApiError> {
    let id = parse_id::<RunId>(&id)?;
    let reset = parse_id::<ResetId>(&reset)?;
    admit(&state, &caller, ApiOperation::RunRead, ApiTarget::Run(id)).await?;
    let record = state
        .runs()
        .reset_record(&caller.session.principal, id, reset, &caller.ctx)
        .await?;
    Ok(Json(ResetBody::from(&record)))
}

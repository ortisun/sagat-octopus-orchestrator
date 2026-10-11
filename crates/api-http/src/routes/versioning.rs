//! Approvals, the active version and promotion (feature 011).
//!
//! Every handler takes [`Caller`] first, so authentication is decided before any path, query,
//! header or body is looked at; the tenant is never read from the request.

use std::str::FromStr;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use sagat_application::identity::PageRequest;
use sagat_application::publishing::Promotion;
use sagat_domain::definition::WorkflowId;
use sagat_domain::identity::NamespaceId;
use sagat_domain::versioning::{RequestId, RequestState, Verdict};
use serde::Deserialize;

use crate::dto::PageBody;
use crate::dto_exec::{PublishedBody, VersionSummary};
use crate::dto_versioning::{
    ActivateBody, ActivationBody, ActivationResultBody, DecisionBody, OpenRequestBody, PolicyBody,
    PromoteBody, PromotionBody, RequestBody, SetPolicyBody,
};
use crate::error::ApiError;
use crate::exec::Exec;
use crate::extract::{ApiJson, Caller};

const IDEMPOTENCY_HEADER: &str = "idempotency-key";

fn idempotency_key(headers: &HeaderMap) -> Result<&str, ApiError> {
    let key = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("the Idempotency-Key header is required"))?;
    if !(8..=160).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(ApiError::bad_request(
            "the Idempotency-Key must be 8 to 160 printable ASCII characters",
        ));
    }
    Ok(key)
}

fn parse_id<T: FromStr>(raw: &str) -> Result<T, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub state: Option<String>,
}

fn page_request(query: &PageQuery) -> PageRequest {
    PageRequest {
        limit: query.limit,
        cursor: query.cursor.clone(),
    }
}

type Service<B> = sagat_application::publishing::PublishingService<
    <B as Exec>::Store,
    <B as Exec>::Clock,
    <B as Exec>::Codec,
>;

fn service<B: Exec>(state: &B) -> Result<&Service<B>, ApiError> {
    state.publishing().ok_or_else(ApiError::not_found)
}

pub async fn get_policy<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<PolicyBody>, ApiError> {
    let view = service(&state)?
        .policy(
            &caller.session.principal,
            parse_id::<NamespaceId>(&namespace)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(PolicyBody::from(&view)))
}

pub async fn set_policy<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    ApiJson(body): ApiJson<SetPolicyBody>,
) -> Result<Json<PolicyBody>, ApiError> {
    let view = service(&state)?
        .set_policy(
            &caller.session.principal,
            parse_id::<NamespaceId>(&namespace)?,
            body.required_approvals,
            &body.reason,
            &caller.ctx,
        )
        .await?;
    Ok(Json(PolicyBody::from(&view)))
}

pub async fn open_request<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(workflow): Path<String>,
    ApiJson(body): ApiJson<OpenRequestBody>,
) -> Result<(StatusCode, Json<RequestBody>), ApiError> {
    let request = service(&state)?
        .open_request(
            &caller.session.principal,
            parse_id::<WorkflowId>(&workflow)?,
            body.draft_revision,
            body.release_notes.as_deref(),
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(RequestBody::from(&request))))
}

pub async fn list_requests<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(workflow): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PageBody<RequestBody>>, ApiError> {
    let wanted = query
        .state
        .as_deref()
        .map(|s| {
            RequestState::parse(s)
                .ok_or_else(|| ApiError::validation("the state is not one of the request states"))
        })
        .transpose()?;
    let result = service(&state)?
        .requests(
            &caller.session.principal,
            parse_id::<WorkflowId>(&workflow)?,
            wanted,
            &page_request(&query),
            &caller.ctx,
        )
        .await?;
    Ok(Json(PageBody {
        items: result.items.iter().map(RequestBody::from).collect(),
        next_cursor: result.next_cursor,
    }))
}

pub async fn get_request<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(request): Path<String>,
) -> Result<Json<RequestBody>, ApiError> {
    let request = service(&state)?
        .get_request(
            &caller.session.principal,
            parse_id::<RequestId>(&request)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(RequestBody::from(&request)))
}

async fn decide<B: Exec>(
    caller: Caller,
    state: B,
    request: String,
    verdict: Verdict,
    body: DecisionBody,
) -> Result<Json<RequestBody>, ApiError> {
    let request = service(&state)?
        .decide(
            &caller.session.principal,
            parse_id::<RequestId>(&request)?,
            verdict,
            &body.reason,
            &caller.ctx,
        )
        .await?;
    Ok(Json(RequestBody::from(&request)))
}

pub async fn approve_request<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(request): Path<String>,
    ApiJson(body): ApiJson<DecisionBody>,
) -> Result<Json<RequestBody>, ApiError> {
    decide(caller, state, request, Verdict::Approve, body).await
}

pub async fn reject_request<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(request): Path<String>,
    ApiJson(body): ApiJson<DecisionBody>,
) -> Result<Json<RequestBody>, ApiError> {
    decide(caller, state, request, Verdict::Reject, body).await
}

pub async fn withdraw_request<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(request): Path<String>,
) -> Result<Json<RequestBody>, ApiError> {
    let request = service(&state)?
        .withdraw(
            &caller.session.principal,
            parse_id::<RequestId>(&request)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(RequestBody::from(&request)))
}

pub async fn publish_request<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path(request): Path<String>,
) -> Result<(StatusCode, Json<PublishedBody>), ApiError> {
    let published = service(&state)?
        .publish_request(
            &caller.session.principal,
            parse_id::<RequestId>(&request)?,
            idempotency_key(&headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(PublishedBody {
            version: VersionSummary::from(&published.version),
            created: published.created,
            warnings: sagat_application::definitions::warnings_text(&published.warnings),
        }),
    ))
}

pub async fn list_activations<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(workflow): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PageBody<ActivationBody>>, ApiError> {
    let result = service(&state)?
        .activations(
            &caller.session.principal,
            parse_id::<WorkflowId>(&workflow)?,
            &page_request(&query),
            &caller.ctx,
        )
        .await?;
    Ok(Json(PageBody {
        items: result.items.iter().map(ActivationBody::from).collect(),
        next_cursor: result.next_cursor,
    }))
}

pub async fn activate_version<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(workflow): Path<String>,
    ApiJson(body): ApiJson<ActivateBody>,
) -> Result<Json<ActivationResultBody>, ApiError> {
    let activation = service(&state)?
        .activate(
            &caller.session.principal,
            parse_id::<WorkflowId>(&workflow)?,
            body.version,
            &body.reason,
            body.expected_version,
            body.accept_breaking,
            &caller.ctx,
        )
        .await?;
    Ok(Json(ActivationResultBody {
        activation: ActivationBody::from(&activation.record),
        changed: activation.changed,
    }))
}

pub async fn promote_version<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path((workflow, number)): Path<(String, u32)>,
    ApiJson(body): ApiJson<PromoteBody>,
) -> Result<(StatusCode, Json<PromotionBody>), ApiError> {
    let promotion = service(&state)?
        .promote(
            &caller.session.principal,
            parse_id::<WorkflowId>(&workflow)?,
            number,
            parse_id::<NamespaceId>(&body.namespace_id)?,
            &body.reason,
            body.accept_breaking,
            idempotency_key(&headers)?,
            &caller.ctx,
        )
        .await?;
    let status = match &promotion {
        Promotion::Published(_) => StatusCode::CREATED,
        Promotion::Requested(_) => StatusCode::ACCEPTED,
        Promotion::AlreadyActive { .. } => StatusCode::OK,
    };
    Ok((status, Json(PromotionBody::of(&promotion))))
}

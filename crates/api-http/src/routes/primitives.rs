//! Public HTTP operations for durable workflow interaction.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sagat_application::api_admission::{
    ApiAdmissionService, ApiAdmissionStore, ApiOperation, ApiTarget,
};
use sagat_application::approval_progress::{
    ApprovalAddress, ApprovalDecisionAttempt, ApprovalPageSize, ApprovalProgressStore,
};
use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::identity::IdentityStore;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{
    ApprovalDecisionCommand, ApprovalError, ApprovalReceiptId, ApprovalRequestId, RunId,
};
use sagat_domain::workflow::Digest;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tower_http::limit::RequestBodyLimitLayer;
use uuid::Uuid;

use crate::admission::{Admission, admit};
use crate::concurrency::TenantConcurrency;
use crate::core::Core;
use crate::dto_primitives::{
    ApprovalDecisionReceiptBody, ApprovalListBody, ApprovalListItemBody, DecideApprovalBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::limits::RateLimiter;
use crate::{AppState, TokenVerifier};

pub struct ApprovalApiState<S, C, P, A> {
    core: AppState<S, C>,
    admission: Arc<ApiAdmissionService<P, C>>,
    approvals: Arc<A>,
    clock: C,
}

impl<S, C, P, A> ApprovalApiState<S, C, P, A> {
    #[must_use]
    pub fn new(
        core: AppState<S, C>,
        admission: ApiAdmissionService<P, C>,
        approvals: A,
        clock: C,
    ) -> Self {
        Self {
            core,
            admission: Arc::new(admission),
            approvals: Arc::new(approvals),
            clock,
        }
    }
}

impl<S, C, P, A> Clone for ApprovalApiState<S, C, P, A>
where
    C: Clone,
{
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            admission: Arc::clone(&self.admission),
            approvals: Arc::clone(&self.approvals),
            clock: self.clock.clone(),
        }
    }
}

impl<S, C, P, A> Core for ApprovalApiState<S, C, P, A>
where
    S: IdentityStore + 'static,
    C: Clock + Clone + 'static,
    P: Send + Sync + 'static,
    A: Send + Sync + 'static,
{
    fn verifier(&self) -> &TokenVerifier {
        self.core.verifier()
    }

    fn subject_limiter(&self) -> &RateLimiter<String> {
        self.core.subject_limiter()
    }

    fn tenant_limiter(&self) -> &RateLimiter {
        self.core.tenant_limiter()
    }

    fn tenant_concurrency(&self) -> &Arc<TenantConcurrency> {
        self.core.tenant_concurrency()
    }

    async fn authenticate(
        &self,
        tenant: &str,
        subject: &str,
    ) -> Result<sagat_application::identity::Session, sagat_application::error::AppError> {
        self.core.authenticate(tenant, subject).await
    }
}

impl<S, C, P, A> Admission for ApprovalApiState<S, C, P, A>
where
    S: IdentityStore + 'static,
    C: Clock + Clone + 'static,
    P: ApiAdmissionStore + 'static,
    A: Send + Sync + 'static,
{
    type AdmissionStore = P;
    type AdmissionClock = C;

    fn api_admission(&self) -> &ApiAdmissionService<P, C> {
        &self.admission
    }
}

pub trait ApprovalApi: Core + Admission {
    type ApprovalStore: ApprovalProgressStore + 'static;
    type ApprovalClock: Clock + 'static;

    fn approval_store(&self) -> &Self::ApprovalStore;
    fn approval_clock(&self) -> &Self::ApprovalClock;
}

impl<S, C, P, A> ApprovalApi for ApprovalApiState<S, C, P, A>
where
    S: IdentityStore + 'static,
    C: Clock + Clone + 'static,
    P: ApiAdmissionStore + 'static,
    A: ApprovalProgressStore + 'static,
{
    type ApprovalStore = A;
    type ApprovalClock = C;

    fn approval_store(&self) -> &A {
        &self.approvals
    }

    fn approval_clock(&self) -> &C {
        &self.clock
    }
}

pub fn routes<B, T>(state: B, max_body_bytes: usize) -> Router<T>
where
    B: ApprovalApi,
    T: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/runs/{run_id}/approvals", get(list_approvals::<B>))
        .route(
            "/approvals/{approval_id}/decisions",
            post(decide_approval::<B>),
        )
        .layer(RequestBodyLimitLayer::new(max_body_bytes))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalListQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

fn decode_cursor(cursor: &str) -> Result<u64, ApiError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| ApiError::bad_request("the cursor is not valid"))?;
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| ApiError::bad_request("the cursor is not valid"))?;
    let sequence = u64::from_be_bytes(bytes);
    if sequence == 0 {
        return Err(ApiError::bad_request("the cursor is not valid"));
    }
    Ok(sequence)
}

fn encode_cursor(sequence: u64) -> String {
    URL_SAFE_NO_PAD.encode(sequence.to_be_bytes())
}

pub async fn list_approvals<B: ApprovalApi>(
    caller: Caller,
    State(state): State<B>,
    Path(run_id): Path<String>,
    Query(query): Query<ApprovalListQuery>,
) -> Result<(HeaderMap, Json<ApprovalListBody>), ApiError> {
    let run_id: RunId = run_id.parse().map_err(|_| ApiError::not_found())?;
    let namespace = admit(
        &state,
        &caller,
        ApiOperation::ApprovalRead,
        ApiTarget::Run(run_id),
    )
    .await?;
    let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let mut found = state
        .approval_store()
        .list_for_run(
            caller.session.principal.tenant_id(),
            namespace,
            run_id,
            cursor,
            ApprovalPageSize::new(limit + 1),
        )
        .await?;
    let next_cursor = if found.len() > limit as usize {
        found.truncate(limit as usize);
        found
            .last()
            .map(|item| encode_cursor(item.opened_sequence()))
    } else {
        None
    };
    metrics::counter!("workflow_approval_http_reads_total", "outcome" => "listed").increment(1);
    tracing::info!(item_count = found.len(), "workflow approvals listed");
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((
        headers,
        Json(ApprovalListBody {
            items: found.iter().map(ApprovalListItemBody::from).collect(),
            next_cursor,
        }),
    ))
}

pub async fn decide_approval<B: ApprovalApi>(
    caller: Caller,
    State(state): State<B>,
    Path(approval_id): Path<String>,
    ApiJson(body): ApiJson<DecideApprovalBody>,
) -> Result<(HeaderMap, Json<ApprovalDecisionReceiptBody>), ApiError> {
    let request_id: ApprovalRequestId = approval_id.parse().map_err(|_| ApiError::not_found())?;
    let namespace = admit(
        &state,
        &caller,
        ApiOperation::ApprovalDecide,
        ApiTarget::ApprovalRequest(request_id),
    )
    .await
    .map_err(|error| {
        if error.parts().status == StatusCode::FORBIDDEN {
            ApiError::from(ApprovalError::Forbidden)
        } else {
            error
        }
    })?;
    let decision_key = body.decision_key()?;
    let reason_code = body.reason_code()?;
    let request_hash = Digest::new(
        Sha256::digest(serde_json::to_vec(&body).map_err(|_| ApiError::from(AppError::Internal))?)
            .into(),
    );
    let occurred_at = state.approval_clock().now();
    let audit = AuditRecord::by(
        &caller.session.principal,
        AuditAction::ApprovalDecided,
        ResourceType::ApprovalRequest,
        request_id.to_string(),
        caller.ctx.trace_id.clone(),
        occurred_at,
    );
    let resolution = state
        .approval_store()
        .decide(&ApprovalDecisionAttempt::new(
            ApprovalAddress::new(caller.session.principal.tenant_id(), namespace, request_id),
            caller.session.principal.subject().clone(),
            ApprovalReceiptId::generate(),
            ApprovalDecisionCommand::new(
                decision_key,
                request_hash,
                body.decision.into(),
                reason_code,
            ),
            audit,
            Uuid::now_v7(),
            occurred_at,
        ))
        .await?;
    let outcome = if resolution.replayed() {
        "replayed"
    } else {
        "committed"
    };
    metrics::counter!("workflow_approval_http_decisions_total", "outcome" => outcome).increment(1);
    tracing::info!(outcome, "workflow approval decision accepted");

    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((
        headers,
        Json(ApprovalDecisionReceiptBody::from(&resolution)),
    ))
}

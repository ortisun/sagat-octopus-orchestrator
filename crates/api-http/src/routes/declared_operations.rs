//! Authorized HTTP boundary for declared business Query and Update operations.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::routing::post;
use axum::{Json, Router};
use sagat_application::api_admission::{
    ApiAdmissionService, ApiAdmissionStore, ApiOperation, ApiTarget,
};
use sagat_application::clock::Clock;
use sagat_application::declared_operations::{
    DeclaredOperationEvaluator, DeclaredOperationStore, DeclaredOperationStoreError, DeclaredQuery,
    DeclaredUpdate,
};
use sagat_application::error::AppError;
use sagat_application::identity::IdentityStore;
use sagat_application::operation_evaluation::{
    DeclaredOperationArguments, OperationDataField, OperationEvaluationError,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{
    OperationAddress, OperationError, OperationScope, RunId, UpdateKey, UpdateReceiptStatus,
    UpdateRequest,
};
use sagat_domain::identity::NamespaceId;
use sagat_domain::identity::Permission;
use sagat_domain::workflow::{ActivityName, Digest};
use sagat_telemetry::{
    WorkflowPrimitiveKind, WorkflowPrimitiveMetric, WorkflowPrimitiveOutcome,
    WorkflowPrimitiveRejectionCode, WorkflowPrimitiveSpan,
};
use tower_http::limit::RequestBodyLimitLayer;
use tracing::Instrument;
use uuid::Uuid;

use crate::admission::{Admission, admit};
use crate::concurrency::TenantConcurrency;
use crate::core::Core;
use crate::dto_declared_operations::{
    ChainDeclaredQueryBody, ChainDeclaredUpdateBody, DeclaredQueryBody, DeclaredQueryResponseBody,
    DeclaredUpdateBody, DeclaredUpdateResponseBody, RejectedUpdateDetails,
};
use crate::error::{ApiError, ErrorCode};
use crate::extract::{ApiJson, Caller};
use crate::limits::RateLimiter;
use crate::{AppState, TokenVerifier};

pub struct DeclaredOperationApiState<S, C, P, O, E> {
    core: AppState<S, C>,
    admission: Arc<ApiAdmissionService<P, C>>,
    operations: Arc<O>,
    evaluator: Arc<E>,
    clock: C,
}

impl<S, C, P, O, E> DeclaredOperationApiState<S, C, P, O, E> {
    #[must_use]
    pub fn new(
        core: AppState<S, C>,
        admission: ApiAdmissionService<P, C>,
        operations: O,
        evaluator: E,
        clock: C,
    ) -> Self {
        Self {
            core,
            admission: Arc::new(admission),
            operations: Arc::new(operations),
            evaluator: Arc::new(evaluator),
            clock,
        }
    }
}

impl<S, C, P, O, E> Clone for DeclaredOperationApiState<S, C, P, O, E>
where
    C: Clone,
{
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            admission: Arc::clone(&self.admission),
            operations: Arc::clone(&self.operations),
            evaluator: Arc::clone(&self.evaluator),
            clock: self.clock.clone(),
        }
    }
}

impl<S, C, P, O, E> Core for DeclaredOperationApiState<S, C, P, O, E>
where
    S: IdentityStore + 'static,
    C: Clock + Clone + 'static,
    P: Send + Sync + 'static,
    O: Send + Sync + 'static,
    E: Send + Sync + 'static,
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
    ) -> Result<sagat_application::identity::Session, AppError> {
        self.core.authenticate(tenant, subject).await
    }

    async fn record_access_denied(
        &self,
        principal: &sagat_domain::identity::Principal,
        call: &str,
        denial: sagat_application::error::AccessDenial,
        ctx: &sagat_application::identity::RequestContext,
    ) -> Result<(), sagat_application::error::StoreError> {
        self.core
            .record_access_denied(principal, call, denial, ctx)
            .await
    }
}

impl<S, C, P, O, E> Admission for DeclaredOperationApiState<S, C, P, O, E>
where
    S: IdentityStore + 'static,
    C: Clock + Clone + 'static,
    P: ApiAdmissionStore + 'static,
    O: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    type AdmissionStore = P;
    type AdmissionClock = C;

    fn api_admission(&self) -> &ApiAdmissionService<P, C> {
        &self.admission
    }
}

pub trait DeclaredOperationApi: Core + Admission {
    type OperationStore: DeclaredOperationStore + 'static;
    type Evaluator: DeclaredOperationEvaluator + 'static;
    type OperationClock: Clock + 'static;

    fn operation_store(&self) -> &Self::OperationStore;
    fn operation_evaluator(&self) -> &Self::Evaluator;
    fn operation_clock(&self) -> &Self::OperationClock;
}

impl<S, C, P, O, E> DeclaredOperationApi for DeclaredOperationApiState<S, C, P, O, E>
where
    S: IdentityStore + 'static,
    C: Clock + Clone + 'static,
    P: ApiAdmissionStore + 'static,
    O: DeclaredOperationStore + 'static,
    E: DeclaredOperationEvaluator + 'static,
{
    type OperationStore = O;
    type Evaluator = E;
    type OperationClock = C;

    fn operation_store(&self) -> &O {
        &self.operations
    }

    fn operation_evaluator(&self) -> &E {
        &self.evaluator
    }

    fn operation_clock(&self) -> &C {
        &self.clock
    }
}

pub fn routes<B, T>(state: B, max_body_bytes: usize) -> Router<T>
where
    B: DeclaredOperationApi,
    T: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/runs/{run_id}/queries/{name}", post(query::<B>))
        .route("/runs/{run_id}/updates/{name}", post(update::<B>))
        .route(
            "/namespaces/{namespace_id}/chains/{chain_id}/queries/{name}",
            post(query_chain::<B>),
        )
        .route(
            "/namespaces/{namespace_id}/chains/{chain_id}/updates/{name}",
            post(update_chain::<B>),
        )
        .layer(RequestBodyLimitLayer::new(max_body_bytes))
        .with_state(state)
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}

fn arguments(value: &serde_json::Value) -> Result<DeclaredOperationArguments, ApiError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ApiError::from(AppError::Internal))?;
    DeclaredOperationArguments::parse(&bytes).map_err(evaluation_error)
}

fn evaluation_error(error: OperationEvaluationError) -> ApiError {
    let reason = match error {
        OperationEvaluationError::MalformedJson { .. } => "MALFORMED_OPERATION_VALUE",
        OperationEvaluationError::SchemaMismatch {
            field: OperationDataField::Arguments,
        } => "INVALID_ARGUMENTS",
        OperationEvaluationError::SchemaMismatch { .. } => "INVALID_OPERATION_STATE",
        OperationEvaluationError::UndeclaredOperation => "UNDECLARED_OPERATION",
        OperationEvaluationError::ArgumentsTooLarge | OperationEvaluationError::ValueTooLarge => {
            "OPERATION_VALUE_TOO_LARGE"
        }
        OperationEvaluationError::InvalidSchema { .. }
        | OperationEvaluationError::DeclarationMismatch
        | OperationEvaluationError::ExpressionFailed
        | OperationEvaluationError::ContextTooLarge
        | OperationEvaluationError::RetainedValueMismatch
        | OperationEvaluationError::StateChanged => "OPERATION_EVALUATION_FAILED",
    };
    ApiError::validation("declared operation evaluation failed").with_reason(reason)
}

fn operation_error(error: OperationError) -> ApiError {
    match error {
        OperationError::InvalidUpdateKey => ApiError::validation(
            "updateKey must be 1..128 printable ASCII characters without spaces",
        )
        .with_reason("INVALID_UPDATE_KEY"),
        OperationError::InvalidArguments => {
            ApiError::validation("operation arguments do not match the declared schema")
                .with_reason("INVALID_ARGUMENTS")
        }
        OperationError::UpdateKeyReused => {
            ApiError::conflict("this update key was already used for different request content")
                .with_reason("IDEMPOTENCY_KEY_REUSED")
        }
        OperationError::RunContinued => {
            ApiError::conflict("the run was continued by a successor").with_reason("RUN_CONTINUED")
        }
        OperationError::RunNotMutable(_) => {
            ApiError::conflict("the run does not admit a new update").with_reason("RUN_NOT_MUTABLE")
        }
        OperationError::RevisionExhausted => {
            ApiError::conflict("the run revision is exhausted").with_reason("REVISION_EXHAUSTED")
        }
        OperationError::EvaluationFailed
        | OperationError::StateSchemaChanged
        | OperationError::ValueTooLarge
        | OperationError::UnsupportedDisclosure => {
            ApiError::validation("declared operation evaluation failed")
                .with_reason("INVALID_OPERATION_STATE")
        }
        OperationError::InvalidStoredOperation => {
            tracing::error!(%error, "declared operation storage invariant failed");
            ApiError::from(AppError::Internal)
        }
    }
}

fn update_argument_error(error: OperationEvaluationError) -> OperationError {
    match error {
        OperationEvaluationError::MalformedJson {
            field: OperationDataField::Arguments,
        }
        | OperationEvaluationError::SchemaMismatch {
            field: OperationDataField::Arguments,
        }
        | OperationEvaluationError::ArgumentsTooLarge => OperationError::InvalidArguments,
        _ => OperationError::EvaluationFailed,
    }
}

fn store_error(error: DeclaredOperationStoreError, permission: Permission) -> ApiError {
    match error {
        DeclaredOperationStoreError::NotFound
        | DeclaredOperationStoreError::DefinitionUnavailable => ApiError::not_found(),
        DeclaredOperationStoreError::Forbidden => {
            ApiError::from(AppError::Forbidden { permission })
        }
        DeclaredOperationStoreError::Operation(error) => operation_error(error),
        DeclaredOperationStoreError::Storage(error) => ApiError::from(AppError::from(error)),
        DeclaredOperationStoreError::AddressUnavailable => {
            ApiError::conflict("the operation address is unavailable")
                .with_reason("ADDRESS_UNAVAILABLE")
        }
        DeclaredOperationStoreError::ChainHeadChanged => {
            ApiError::from(AppError::StateConflict("the execution chain head changed"))
                .with_reason("CHAIN_HEAD_CHANGED")
        }
        DeclaredOperationStoreError::AuditMismatch => {
            tracing::error!(%error, "declared operation audit invariant failed");
            ApiError::from(AppError::Internal)
        }
    }
}

/// The contracted telemetry of one declared operation (feature 014 `contracts/README.md` §6): a
/// `workflow.query` or `workflow.update` span under the request span, the
/// `workflow_operation_duration_seconds{kind,outcome}` histogram and, when the operation is refused
/// for a known reason, `workflow_primitive_rejections_total{kind,code}`. Every label and attribute
/// comes from a closed enum; the operation name, arguments, state and result never reach them.
struct OperationTelemetry {
    kind: WorkflowPrimitiveKind,
    span: tracing::Span,
    started: Instant,
}

impl OperationTelemetry {
    fn start(kind: WorkflowPrimitiveKind) -> Self {
        let span = if kind == WorkflowPrimitiveKind::Update {
            tracing::info_span!(
                WorkflowPrimitiveSpan::Update.as_str(),
                sagat.primitive.kind = kind.as_str(),
                sagat.outcome = tracing::field::Empty,
                sagat.rejection.code = tracing::field::Empty,
                sagat.operation.replayed = tracing::field::Empty,
                otel.status_code = tracing::field::Empty,
            )
        } else {
            tracing::info_span!(
                WorkflowPrimitiveSpan::Query.as_str(),
                sagat.primitive.kind = kind.as_str(),
                sagat.outcome = tracing::field::Empty,
                sagat.rejection.code = tracing::field::Empty,
                otel.status_code = tracing::field::Empty,
            )
        };
        Self {
            kind,
            span,
            started: Instant::now(),
        }
    }

    /// A refusal with a closed rejection code is `rejected`; anything else that failed is `failed`.
    fn rejection(error: &ApiError) -> Option<WorkflowPrimitiveRejectionCode> {
        let parts = error.parts();
        parts
            .details
            .as_ref()
            .and_then(|details| details.get("reason"))
            .and_then(serde_json::Value::as_str)
            .and_then(WorkflowPrimitiveRejectionCode::from_reason)
            .or(match parts.code {
                ErrorCode::RateLimited => Some(WorkflowPrimitiveRejectionCode::RateLimited),
                ErrorCode::QuotaExceeded => Some(WorkflowPrimitiveRejectionCode::QuotaExceeded),
                _ => None,
            })
    }

    fn finish<T>(self, result: &Result<T, ApiError>, replayed: impl FnOnce(&T) -> Option<bool>) {
        let outcome = match result {
            Ok(value) => {
                if let Some(replayed) = replayed(value) {
                    self.span.record("sagat.operation.replayed", replayed);
                }
                if self.kind == WorkflowPrimitiveKind::Update {
                    WorkflowPrimitiveOutcome::Applied
                } else {
                    WorkflowPrimitiveOutcome::Succeeded
                }
            }
            Err(error) => match Self::rejection(error) {
                Some(code) => {
                    self.span.record("sagat.rejection.code", code.as_str());
                    metrics::counter!(
                        WorkflowPrimitiveMetric::Rejections.as_str(),
                        "kind" => self.kind.as_str(),
                        "code" => code.as_str(),
                    )
                    .increment(1);
                    WorkflowPrimitiveOutcome::Rejected
                }
                None => {
                    if error.parts().status.is_server_error() {
                        self.span.record("otel.status_code", "error");
                    }
                    WorkflowPrimitiveOutcome::Failed
                }
            },
        };
        self.span.record("sagat.outcome", outcome.as_str());
        metrics::histogram!(
            WorkflowPrimitiveMetric::OperationDuration.as_str(),
            "kind" => self.kind.as_str(),
            "outcome" => outcome.as_str(),
        )
        .record(self.started.elapsed().as_secs_f64());
    }
}

pub async fn query_chain<B: DeclaredOperationApi>(
    caller: Caller,
    State(state): State<B>,
    Path(path): Path<(String, String, String)>,
    ApiJson(body): ApiJson<ChainDeclaredQueryBody>,
) -> Result<(HeaderMap, Json<DeclaredQueryResponseBody>), ApiError> {
    let telemetry = OperationTelemetry::start(WorkflowPrimitiveKind::Query);
    let result = query_chain_operation(caller, state, path, body)
        .instrument(telemetry.span.clone())
        .await;
    telemetry.finish(&result, |_| None);
    result
}

pub async fn query<B: DeclaredOperationApi>(
    caller: Caller,
    State(state): State<B>,
    Path(path): Path<(String, String)>,
    ApiJson(body): ApiJson<DeclaredQueryBody>,
) -> Result<(HeaderMap, Json<DeclaredQueryResponseBody>), ApiError> {
    let telemetry = OperationTelemetry::start(WorkflowPrimitiveKind::Query);
    let result = query_operation(caller, state, path, body)
        .instrument(telemetry.span.clone())
        .await;
    telemetry.finish(&result, |_| None);
    result
}

pub async fn update<B: DeclaredOperationApi>(
    caller: Caller,
    State(state): State<B>,
    Path(path): Path<(String, String)>,
    ApiJson(body): ApiJson<DeclaredUpdateBody>,
) -> Result<(HeaderMap, Json<DeclaredUpdateResponseBody>), ApiError> {
    let telemetry = OperationTelemetry::start(WorkflowPrimitiveKind::Update);
    let result = update_operation(caller, state, path, body)
        .instrument(telemetry.span.clone())
        .await;
    telemetry.finish(&result, |(_, Json(body))| Some(body.replayed));
    result
}

pub async fn update_chain<B: DeclaredOperationApi>(
    caller: Caller,
    State(state): State<B>,
    Path(path): Path<(String, String, String)>,
    ApiJson(body): ApiJson<ChainDeclaredUpdateBody>,
) -> Result<(HeaderMap, Json<DeclaredUpdateResponseBody>), ApiError> {
    let telemetry = OperationTelemetry::start(WorkflowPrimitiveKind::Update);
    let result = update_chain_operation(caller, state, path, body)
        .instrument(telemetry.span.clone())
        .await;
    telemetry.finish(&result, |(_, Json(body))| Some(body.replayed));
    result
}

async fn query_chain_operation<B: DeclaredOperationApi>(
    caller: Caller,
    state: B,
    (namespace_id, chain_id, name): (String, String, String),
    body: ChainDeclaredQueryBody,
) -> Result<(HeaderMap, Json<DeclaredQueryResponseBody>), ApiError> {
    let namespace_id = namespace_id
        .parse::<Uuid>()
        .map(NamespaceId::new)
        .map_err(|_| ApiError::not_found())?;
    let chain_id = chain_id
        .parse::<RunId>()
        .map_err(|_| ApiError::not_found())?;
    let operation = ActivityName::parse(&name).map_err(|_| ApiError::not_found())?;
    let namespace = admit(
        &state,
        &caller,
        ApiOperation::RunQuery,
        ApiTarget::Namespace(namespace_id),
    )
    .await?;
    let expected_run_id = body
        .expected_run_id
        .as_deref()
        .map(str::parse::<RunId>)
        .transpose()
        .map_err(|_| {
            ApiError::validation("expectedRunId must be a valid run identifier")
                .with_reason("INVALID_EXPECTED_RUN_ID")
        })?;
    let arguments = arguments(&body.args)?;
    let query = DeclaredQuery::for_chain(
        OperationScope::new(
            caller.session.principal.tenant_id(),
            namespace,
            OperationAddress::Chain(chain_id),
        ),
        expected_run_id,
        operation.clone(),
        caller.session.principal.subject().clone(),
    );
    let audit = AuditRecord::by(
        &caller.session.principal,
        AuditAction::RunQueried,
        ResourceType::Run,
        chain_id.to_string(),
        caller.ctx.trace_id.clone(),
        state.operation_clock().now(),
    );
    let snapshot = state
        .operation_store()
        .query_snapshot(&query, &audit)
        .await
        .map_err(|error| store_error(error, Permission::RunQuery))?;
    let evaluated = state
        .operation_evaluator()
        .query(&snapshot, &operation, &arguments)
        .map_err(evaluation_error)?;
    let result = serde_json::from_slice(evaluated.result().content())
        .map_err(|_| ApiError::from(AppError::Internal))?;
    tracing::info!(operation = %operation, chain_id = %chain_id, run_id = %evaluated.response().run_id(), "chain declared query returned");
    Ok((
        no_store(),
        Json(DeclaredQueryResponseBody {
            run_id: evaluated.response().run_id().to_string(),
            run_version: evaluated.response().run_version(),
            state_revision: evaluated.response().state_revision(),
            result,
        }),
    ))
}

async fn query_operation<B: DeclaredOperationApi>(
    caller: Caller,
    state: B,
    (run_id, name): (String, String),
    body: DeclaredQueryBody,
) -> Result<(HeaderMap, Json<DeclaredQueryResponseBody>), ApiError> {
    let run_id = run_id.parse::<RunId>().map_err(|_| ApiError::not_found())?;
    let operation = ActivityName::parse(&name).map_err(|_| ApiError::not_found())?;
    let namespace = admit(
        &state,
        &caller,
        ApiOperation::RunQuery,
        ApiTarget::Run(run_id),
    )
    .await?;
    let arguments = arguments(&body.args)?;
    let query = DeclaredQuery::new(
        OperationScope::new(
            caller.session.principal.tenant_id(),
            namespace,
            OperationAddress::Run(run_id),
        ),
        run_id,
        operation.clone(),
        caller.session.principal.subject().clone(),
    );
    let audit = AuditRecord::by(
        &caller.session.principal,
        AuditAction::RunQueried,
        ResourceType::Run,
        run_id.to_string(),
        caller.ctx.trace_id.clone(),
        state.operation_clock().now(),
    );
    let snapshot = state
        .operation_store()
        .query_snapshot(&query, &audit)
        .await
        .map_err(|error| store_error(error, Permission::RunQuery))?;
    let evaluated = state
        .operation_evaluator()
        .query(&snapshot, &operation, &arguments)
        .map_err(evaluation_error)?;
    let result = serde_json::from_slice(evaluated.result().content())
        .map_err(|_| ApiError::from(AppError::Internal))?;
    tracing::info!(operation = %operation, run_id = %run_id, "declared query returned");
    Ok((
        no_store(),
        Json(DeclaredQueryResponseBody {
            run_id: evaluated.response().run_id().to_string(),
            run_version: evaluated.response().run_version(),
            state_revision: evaluated.response().state_revision(),
            result,
        }),
    ))
}

async fn update_operation<B: DeclaredOperationApi>(
    caller: Caller,
    state: B,
    (run_id, name): (String, String),
    body: DeclaredUpdateBody,
) -> Result<(HeaderMap, Json<DeclaredUpdateResponseBody>), ApiError> {
    let run_id = run_id.parse::<RunId>().map_err(|_| ApiError::not_found())?;
    let operation = ActivityName::parse(&name).map_err(|_| ApiError::not_found())?;
    let namespace = admit(
        &state,
        &caller,
        ApiOperation::RunUpdate,
        ApiTarget::Run(run_id),
    )
    .await?;
    let update_key = UpdateKey::parse(&body.update_key).map_err(operation_error)?;
    let arguments = arguments(&body.args)?;
    let occurred_at = state.operation_clock().now();
    let audit = AuditRecord::by(
        &caller.session.principal,
        AuditAction::RunUpdated,
        ResourceType::Run,
        run_id.to_string(),
        caller.ctx.trace_id.clone(),
        occurred_at,
    );
    let request = UpdateRequest::new(
        OperationScope::new(
            caller.session.principal.tenant_id(),
            namespace,
            OperationAddress::Run(run_id),
        ),
        run_id,
        operation.clone(),
        update_key,
        Digest::new([0; 32]),
        body.expected_state_revision,
        caller.session.principal.subject().clone(),
    );
    let command = DeclaredUpdate::new(request, arguments, audit, Uuid::now_v7(), occurred_at);
    let evaluator = state.operation_evaluator();
    let evaluated_arguments = command.arguments().clone();
    let resolution = state
        .operation_store()
        .apply_update(
            &command,
            |snapshot, arguments| {
                evaluator
                    .validate_update_arguments(snapshot, &operation, arguments)
                    .map_err(update_argument_error)
            },
            |snapshot| {
                evaluator
                    .update(snapshot, &operation, &evaluated_arguments)
                    .map_err(|_| OperationError::EvaluationFailed)
            },
        )
        .await
        .map_err(|error| store_error(error, Permission::RunUpdate))?;
    if resolution.receipt().status() == UpdateReceiptStatus::Rejected {
        let details =
            RejectedUpdateDetails::from_receipt(resolution.receipt(), resolution.replayed());
        // A revision conflict is the state moving under the caller (CONFLICT, 409, D291); a
        // validator that said no is the request being refused (VALIDATION_FAILED, 422).
        let rejected = if details.reason == "REVISION_CONFLICT" {
            ApiError::conflict("the declared update was rejected")
        } else {
            ApiError::validation("the declared update was rejected")
        };
        tracing::info!(operation = %operation, run_id = %run_id, outcome = "rejected", "declared update resolved");
        return Err(rejected.with_details(
            serde_json::to_value(details).map_err(|_| ApiError::from(AppError::Internal))?,
        ));
    }
    let result = state
        .operation_store()
        .update_result(&command, &resolution)
        .await
        .map_err(|error| store_error(error, Permission::RunUpdate))?
        .ok_or_else(|| ApiError::from(AppError::Internal))?;
    let result =
        serde_json::from_slice(result.content()).map_err(|_| ApiError::from(AppError::Internal))?;
    let outcome = if resolution.replayed() {
        "replayed"
    } else {
        "applied"
    };
    tracing::info!(operation = %operation, run_id = %run_id, outcome, "declared update resolved");
    Ok((
        no_store(),
        Json(DeclaredUpdateResponseBody::applied(
            resolution.receipt(),
            result,
            resolution.replayed(),
        )),
    ))
}

async fn update_chain_operation<B: DeclaredOperationApi>(
    caller: Caller,
    state: B,
    (namespace_id, chain_id, name): (String, String, String),
    body: ChainDeclaredUpdateBody,
) -> Result<(HeaderMap, Json<DeclaredUpdateResponseBody>), ApiError> {
    let namespace_id = namespace_id
        .parse::<Uuid>()
        .map(NamespaceId::new)
        .map_err(|_| ApiError::not_found())?;
    let chain_id = chain_id
        .parse::<RunId>()
        .map_err(|_| ApiError::not_found())?;
    let operation = ActivityName::parse(&name).map_err(|_| ApiError::not_found())?;
    let namespace = admit(
        &state,
        &caller,
        ApiOperation::RunUpdate,
        ApiTarget::Namespace(namespace_id),
    )
    .await?;
    let expected_run_id = body
        .expected_run_id
        .as_deref()
        .map(str::parse::<RunId>)
        .transpose()
        .map_err(|_| {
            ApiError::validation("expectedRunId must be a valid run identifier")
                .with_reason("INVALID_EXPECTED_RUN_ID")
        })?;
    let update_key = UpdateKey::parse(&body.update_key).map_err(operation_error)?;
    let arguments = arguments(&body.args)?;
    let occurred_at = state.operation_clock().now();
    let audit = AuditRecord::by(
        &caller.session.principal,
        AuditAction::RunUpdated,
        ResourceType::Run,
        chain_id.to_string(),
        caller.ctx.trace_id.clone(),
        occurred_at,
    );
    let command = DeclaredUpdate::for_chain(
        OperationScope::new(
            caller.session.principal.tenant_id(),
            namespace,
            OperationAddress::Chain(chain_id),
        ),
        expected_run_id,
        operation.clone(),
        update_key,
        body.expected_state_revision,
        caller.session.principal.subject().clone(),
        arguments,
        audit,
        Uuid::now_v7(),
        occurred_at,
    )
    .map_err(|error| store_error(error, Permission::RunUpdate))?;
    let evaluator = state.operation_evaluator();
    let evaluated_arguments = command.arguments().clone();
    let resolution = state
        .operation_store()
        .apply_update(
            &command,
            |snapshot, arguments| {
                evaluator
                    .validate_update_arguments(snapshot, &operation, arguments)
                    .map_err(update_argument_error)
            },
            |snapshot| {
                evaluator
                    .update(snapshot, &operation, &evaluated_arguments)
                    .map_err(|_| OperationError::EvaluationFailed)
            },
        )
        .await
        .map_err(|error| store_error(error, Permission::RunUpdate))?;
    if resolution.receipt().status() == UpdateReceiptStatus::Rejected {
        let details =
            RejectedUpdateDetails::from_receipt(resolution.receipt(), resolution.replayed());
        // A revision conflict is the state moving under the caller (CONFLICT, 409, D291); a
        // validator that said no is the request being refused (VALIDATION_FAILED, 422).
        let rejected = if details.reason == "REVISION_CONFLICT" {
            ApiError::conflict("the declared update was rejected")
        } else {
            ApiError::validation("the declared update was rejected")
        };
        tracing::info!(operation = %operation, chain_id = %chain_id, run_id = %resolution.receipt().resolved_run_id(), outcome = "rejected", "chain declared update resolved");
        return Err(rejected.with_details(
            serde_json::to_value(details).map_err(|_| ApiError::from(AppError::Internal))?,
        ));
    }
    let result = state
        .operation_store()
        .update_result(&command, &resolution)
        .await
        .map_err(|error| store_error(error, Permission::RunUpdate))?
        .ok_or_else(|| ApiError::from(AppError::Internal))?;
    let result =
        serde_json::from_slice(result.content()).map_err(|_| ApiError::from(AppError::Internal))?;
    let outcome = if resolution.replayed() {
        "replayed"
    } else {
        "applied"
    };
    tracing::info!(operation = %operation, chain_id = %chain_id, run_id = %resolution.receipt().resolved_run_id(), outcome, "chain declared update resolved");
    Ok((
        no_store(),
        Json(DeclaredUpdateResponseBody::applied(
            resolution.receipt(),
            result,
            resolution.replayed(),
        )),
    ))
}

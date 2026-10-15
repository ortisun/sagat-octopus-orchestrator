//! Authorized HTTP boundary for declared business Query and Update operations.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
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
use sagat_domain::identity::Permission;
use sagat_domain::workflow::{ActivityName, Digest};
use tower_http::limit::RequestBodyLimitLayer;
use uuid::Uuid;

use crate::admission::{Admission, admit};
use crate::concurrency::TenantConcurrency;
use crate::core::Core;
use crate::dto_declared_operations::{
    DeclaredQueryBody, DeclaredQueryResponseBody, DeclaredUpdateBody, DeclaredUpdateResponseBody,
    RejectedUpdateDetails,
};
use crate::error::ApiError;
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
            ApiError::validation("this update key was already used for different request content")
                .with_status(StatusCode::CONFLICT)
                .with_reason("IDEMPOTENCY_KEY_REUSED")
        }
        OperationError::RunNotMutable(_) => {
            ApiError::validation("the run does not admit a new update")
                .with_status(StatusCode::CONFLICT)
                .with_reason("RUN_NOT_MUTABLE")
        }
        OperationError::RevisionExhausted => ApiError::validation("the run revision is exhausted")
            .with_status(StatusCode::CONFLICT)
            .with_reason("REVISION_EXHAUSTED"),
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
            ApiError::validation("the operation address is unavailable")
                .with_status(StatusCode::CONFLICT)
                .with_reason("ADDRESS_UNAVAILABLE")
        }
        DeclaredOperationStoreError::AuditMismatch => {
            tracing::error!(%error, "declared operation audit invariant failed");
            ApiError::from(AppError::Internal)
        }
    }
}

pub async fn query<B: DeclaredOperationApi>(
    caller: Caller,
    State(state): State<B>,
    Path((run_id, name)): Path<(String, String)>,
    ApiJson(body): ApiJson<DeclaredQueryBody>,
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
    metrics::counter!("workflow_declared_operations_total", "kind" => "query", "outcome" => "returned").increment(1);
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

pub async fn update<B: DeclaredOperationApi>(
    caller: Caller,
    State(state): State<B>,
    Path((run_id, name)): Path<(String, String)>,
    ApiJson(body): ApiJson<DeclaredUpdateBody>,
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
        let status = if details.reason == "REVISION_CONFLICT" {
            StatusCode::CONFLICT
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        };
        metrics::counter!("workflow_declared_operations_total", "kind" => "update", "outcome" => "rejected").increment(1);
        tracing::info!(operation = %operation, run_id = %run_id, outcome = "rejected", "declared update resolved");
        return Err(ApiError::validation("the declared update was rejected")
            .with_status(status)
            .with_details(
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
    metrics::counter!("workflow_declared_operations_total", "kind" => "update", "outcome" => outcome).increment(1);
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

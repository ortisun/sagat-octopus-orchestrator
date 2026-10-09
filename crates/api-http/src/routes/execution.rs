//! Workflow definitions and runs.
//!
//! Every handler takes [`Caller`] first, so authentication (401) is decided before any path,
//! query, header or body is looked at. The namespace is a header (`X-Octopus-Namespace`, a slug);
//! the tenant is never read from the request.

use std::str::FromStr;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use sagat_application::identity::{Page, PageRequest};
use sagat_application::runs::{RunFilter, RunOperation, StartRun};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{RunId, RunStatus};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::dto::PageBody;
use crate::dto_exec::{
    CreateWorkflowBody, JournalEventBody, OperationBody, OperationResultBody, PublishBody,
    PublishedBody, RunBody, SignalResultBody, StartRunBody, UpdateDraftBody, VersionBody,
    VersionSummary, WorkflowBody, runtime_graph,
};
use crate::error::ApiError;
use crate::exec::Exec;
use crate::extract::{ApiJson, Caller};

const NAMESPACE_HEADER: &str = "x-octopus-namespace";
const IDEMPOTENCY_HEADER: &str = "idempotency-key";

fn namespace(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get(NAMESPACE_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 63)
        .ok_or_else(|| ApiError::bad_request("the X-Octopus-Namespace header is required"))
}

/// `Idempotency-Key` is mandatory on every operation that creates or changes something, so a client
/// that retries after an ambiguous failure can never apply it twice.
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
pub struct ListQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    #[serde(rename = "workflowId")]
    pub workflow_id: Option<String>,
    pub status: Option<String>,
}

fn page_request(query: &ListQuery) -> PageRequest {
    PageRequest {
        limit: query.limit,
        cursor: query.cursor.clone(),
    }
}

fn page<T, B: for<'a> From<&'a T>>(page: &Page<T>) -> Json<PageBody<B>> {
    Json(PageBody {
        items: page.items.iter().map(B::from).collect(),
        next_cursor: page.next_cursor.clone(),
    })
}

fn to_bytes(value: &Value) -> Result<Vec<u8>, ApiError> {
    serde_json::to_vec(value).map_err(|_| ApiError::bad_request("the document cannot be encoded"))
}

// ------------------------------------------------------------------------------ definitions

pub async fn create_workflow<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<CreateWorkflowBody>,
) -> Result<(StatusCode, Json<WorkflowBody>), ApiError> {
    let record = state
        .definitions()
        .create_workflow(
            &caller.session.principal,
            namespace(&headers)?,
            &body.name,
            &to_bytes(&body.ir)?,
            idempotency_key(&headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(WorkflowBody::from(&record))))
}

pub async fn list_workflows<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageBody<WorkflowBody>>, ApiError> {
    let result = state
        .definitions()
        .list_workflows(
            &caller.session.principal,
            namespace(&headers)?,
            &page_request(&query),
        )
        .await?;
    Ok(page(&result))
}

pub async fn get_workflow<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
) -> Result<Json<WorkflowBody>, ApiError> {
    let record = state
        .definitions()
        .get_workflow(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(WorkflowBody::from(&record)))
}

pub async fn update_draft<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateDraftBody>,
) -> Result<Json<WorkflowBody>, ApiError> {
    let record = state
        .definitions()
        .update_draft(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            body.draft_revision,
            &to_bytes(&body.ir)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(WorkflowBody::from(&record)))
}

pub async fn publish_workflow<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<PublishBody>,
) -> Result<(StatusCode, Json<PublishedBody>), ApiError> {
    let published = state
        .definitions()
        .publish(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            body.draft_revision,
            body.release_notes.as_deref(),
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

pub async fn list_versions<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageBody<VersionBody>>, ApiError> {
    let result = state
        .definitions()
        .list_versions(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            &page_request(&query),
            &caller.ctx,
        )
        .await?;
    Ok(page(&result))
}

pub async fn get_version<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path((id, number)): Path<(String, String)>,
) -> Result<Json<VersionBody>, ApiError> {
    let number = number.parse::<u32>().map_err(|_| ApiError::not_found())?;
    let record = state
        .definitions()
        .get_version(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            number,
            &caller.ctx,
        )
        .await?;
    Ok(Json(VersionBody::from(&record)))
}

// ------------------------------------------------------------------------------ runs

pub async fn start_run<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<StartRunBody>,
) -> Result<(StatusCode, Json<RunBody>), ApiError> {
    let input = to_bytes(&body.input)?;
    let version = body
        .version_id
        .as_deref()
        .map(parse_id::<WorkflowVersionId>)
        .transpose()?;
    let record = state
        .runs()
        .start(
            &caller.session.principal,
            &StartRun {
                namespace: namespace(&headers)?,
                workflow: parse_id::<WorkflowId>(&body.workflow_id)?,
                version,
                input: &input,
                correlation_id: body.correlation_id.as_deref(),
                deadline: body.deadline,
                key: idempotency_key(&headers)?,
            },
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::ACCEPTED, Json(RunBody::from(&record))))
}

pub async fn list_runs<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageBody<RunBody>>, ApiError> {
    let filter = RunFilter {
        workflow: query
            .workflow_id
            .as_deref()
            .map(parse_id::<WorkflowId>)
            .transpose()?,
        status: query
            .status
            .as_deref()
            .map(|s| {
                s.parse::<RunStatus>()
                    .map_err(|_| ApiError::validation("unknown run status"))
            })
            .transpose()?,
    };
    let result = state
        .runs()
        .list(
            &caller.session.principal,
            namespace(&headers)?,
            &filter,
            &page_request(&query),
        )
        .await?;
    Ok(page(&result))
}

pub async fn get_run<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
) -> Result<Json<RunBody>, ApiError> {
    let run = state
        .runs()
        .get(
            &caller.session.principal,
            parse_id::<RunId>(&id)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(RunBody::from(&run)))
}

pub async fn run_journal<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageBody<JournalEventBody>>, ApiError> {
    let result = state
        .runs()
        .journal(
            &caller.session.principal,
            parse_id::<RunId>(&id)?,
            &page_request(&query),
            &caller.ctx,
        )
        .await?;
    Ok(page(&result))
}

pub async fn run_graph<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let graph = state
        .runs()
        .graph(
            &caller.session.principal,
            parse_id::<RunId>(&id)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(runtime_graph(&graph)))
}

pub async fn operate_run<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<OperationBody>,
) -> Result<(StatusCode, Json<OperationResultBody>), ApiError> {
    if !(8..=2000).contains(&body.reason.chars().count()) {
        return Err(ApiError::validation(
            "the reason must be 8 to 2000 characters",
        ));
    }
    let operation = match body.action.as_str() {
        "CANCEL" => RunOperation::Cancel,
        "PAUSE" => RunOperation::Pause,
        "RESUME" => RunOperation::Resume,
        // Declared in the contract, built in later features: said plainly instead of ignored.
        "RETRY_STEP" | "RETRY_COMPENSATION" | "RERUN_AS_NEW" | "MARK_MANUAL_REVIEW" => {
            return Err(ApiError::validation(
                "this operation is declared in the contract but not supported yet",
            ));
        }
        _ => return Err(ApiError::validation("unknown operation")),
    };
    if body.step_id.is_some() {
        return Err(ApiError::validation(
            "CANCEL, PAUSE and RESUME apply to the whole run",
        ));
    }
    let run = state
        .runs()
        .operate(
            &caller.session.principal,
            parse_id::<RunId>(&id)?,
            operation,
            &body.reason,
            body.expected_run_version,
            idempotency_key(&headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(OperationResultBody {
            operation_id: Uuid::now_v7().to_string(),
            accepted: true,
            run_status: run.status.as_str().to_owned(),
            run_version: run.version,
        }),
    ))
}

pub async fn signal_run<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<SignalResultBody>), ApiError> {
    let delivered = state
        .runs()
        .signal(
            &caller.session.principal,
            parse_id::<RunId>(&id)?,
            &name,
            &body,
            idempotency_key(&headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(SignalResultBody {
            delivered_to: delivered.iter().map(ToString::to_string).collect(),
        }),
    ))
}

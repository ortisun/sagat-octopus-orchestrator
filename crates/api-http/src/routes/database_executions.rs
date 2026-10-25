//! Ad hoc and fixed-probe admission; all SQL runs through the leased managed executor.
use crate::{
    admission::admit,
    dto_exec::RunBody,
    error::ApiError,
    exec::Exec,
    extract::{ApiJson, Caller},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use sagat_application::{
    api_admission::{ApiOperation, ApiTarget},
    runs::{AdHocMode, StartDatabaseExecution},
};
use sagat_domain::{
    database::ConnectionRef,
    identity::{NamespaceId, Slug},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionBody {
    mode: Mode,
    definition: Value,
    #[serde(default = "empty_input")]
    input: Value,
    timeout_ms: Option<u32>,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Step,
    Workflow,
}
fn empty_input() -> Value {
    serde_json::json!({})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeBody {
    revision: Option<u32>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeMetadata {
    run: RunBody,
    revision: u32,
    failure_code: Option<String>,
}
fn namespace(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}
fn connection(raw: &str, revision: Option<u32>) -> Result<ConnectionRef, ApiError> {
    if revision == Some(0) {
        return Err(ApiError::validation("revision must be positive"));
    }
    Ok(ConnectionRef {
        name: Slug::parse(raw).map_err(|_| ApiError::not_found())?,
        revision,
    })
}
fn encode(value: &Value) -> Result<Vec<u8>, ApiError> {
    serde_json::to_vec(value).map_err(|_| ApiError::validation("invalid execution document"))
}

pub async fn create<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(ns): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ExecutionBody>,
) -> Result<(StatusCode, Json<RunBody>), ApiError> {
    let namespace = namespace(&ns)?;
    let key = super::execution::idempotency_key(&headers)?;
    admit(
        &state,
        &caller,
        ApiOperation::DatabaseAdHoc,
        ApiTarget::Namespace(namespace),
    )
    .await?;
    let definition = encode(&body.definition)?;
    let input = encode(&body.input)?;
    let result = state
        .runs()
        .start_database(
            &caller.session.principal,
            &StartDatabaseExecution {
                namespace,
                mode: match body.mode {
                    Mode::Step => AdHocMode::Step,
                    Mode::Workflow => AdHocMode::Workflow,
                },
                document: &definition,
                input: &input,
                timeout_ms: body.timeout_ms,
                key,
            },
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(RunBody::from(&result))))
}

pub async fn probe<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path((ns, name)): Path<(String, String)>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ProbeBody>,
) -> Result<(StatusCode, Json<RunBody>), ApiError> {
    let namespace = namespace(&ns)?;
    let connection = connection(&name, body.revision)?;
    let key = super::execution::idempotency_key(&headers)?;
    admit(
        &state,
        &caller,
        ApiOperation::DatabaseAdHoc,
        ApiTarget::Namespace(namespace),
    )
    .await?;
    let result = state
        .runs()
        .probe_connection(
            &caller.session.principal,
            namespace,
            &connection,
            key,
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(RunBody::from(&result))))
}

pub async fn latest_probe<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path((ns, name)): Path<(String, String)>,
) -> Result<Json<Option<ProbeMetadata>>, ApiError> {
    let namespace = namespace(&ns)?;
    let connection = connection(&name, None)?;
    admit(
        &state,
        &caller,
        ApiOperation::RunRead,
        ApiTarget::Namespace(namespace),
    )
    .await?;
    let result = state
        .runs()
        .latest_connection_probe(
            &caller.session.principal,
            namespace,
            &connection.name,
            &caller.ctx,
        )
        .await?;
    Ok(Json(result.map(|p| ProbeMetadata {
        run: RunBody::from(&p.run),
        revision: p.revision,
        failure_code: p.failure_code,
    })))
}

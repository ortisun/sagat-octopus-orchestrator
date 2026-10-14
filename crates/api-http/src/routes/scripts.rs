//! Scripts: the drafts authors write, the versions they publish, the test runs that try a draft and
//! the executions that are kept (`FR-005-001`, `FR-005-005`, `FR-005-007`).
//!
//! A script is addressed through its namespace, and one that belongs to another tenant is a `404`
//! like one that does not exist.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_application::scripts::{DraftChange, ScriptService, ScriptSpec};
use sagat_domain::identity::{NamespaceId, Slug};
use sagat_domain::scripts::{
    Digest, Language, Lockfile, Retention, ScriptId, Source, TestRunId, VersionNumber,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::admission::admit;
use crate::dto_scripts::{
    ArtifactSummaryBody, CreateScriptBody, ExecutionBody, ExecutionListBody, ExecutionSummaryBody,
    PublishedBody, ScriptBody, ScriptListBody, ScriptSummaryBody, TestRunBody, TestRunListBody,
    TestRunRequestBody, TestRunSummaryBody, UpdateScriptBody, VersionBody, VersionListBody,
    VersionSummaryBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

/// Largest body of an authoring request: a source, a lockfile and two schemas, with the escaping
/// JSON adds, at their limits.
pub const SCRIPT_MAX_BODY_BYTES: usize = 1024 * 1024 + 4096;

fn namespace_id(raw: &str) -> Result<NamespaceId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn script_id(raw: &str) -> Result<ScriptId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn scripts<B: Governance>(state: &B) -> Result<&ScriptService<B::Store, B::Clock>, ApiError> {
    state.scripts().ok_or_else(ApiError::not_found)
}

fn invalid(message: &'static str) -> ApiError {
    ApiError::validation(message)
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

async fn admitted_script<B: Governance>(
    state: &B,
    caller: &Caller,
    raw_namespace: &str,
    raw_script: &str,
    operation: ApiOperation,
) -> Result<sagat_application::scripts::ScriptRecord, ApiError> {
    let namespace = namespace_id(raw_namespace)?;
    let script = scripts(state)?
        .get(
            &caller.session.principal,
            namespace,
            script_id(raw_script)?,
            &caller.ctx,
        )
        .await?;
    admit(
        state,
        caller,
        operation,
        ApiTarget::Namespace(script.namespace_id),
    )
    .await?;
    Ok(script)
}

fn source(text: &str) -> Result<Source, ApiError> {
    Source::parse(text).map_err(|e| ApiError::validation(e.to_string()))
}

fn lockfile(text: &str) -> Result<Lockfile, ApiError> {
    Lockfile::parse(text).map_err(|e| ApiError::validation(e.to_string()))
}

fn retention(days: u16) -> Result<Retention, ApiError> {
    Retention::days(days).map_err(|e| ApiError::validation(e.to_string()))
}

pub async fn create_script<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    ApiJson(body): ApiJson<CreateScriptBody>,
) -> Result<(StatusCode, Json<ScriptBody>), ApiError> {
    let spec = ScriptSpec {
        name: Slug::parse(&body.name).map_err(|_| invalid("the name is not a valid slug"))?,
        language: body
            .language
            .parse::<Language>()
            .map_err(|_| invalid("the language is typescript, python, csharp, go or java"))?,
        source: source(&body.source)?,
        lockfile: body.lockfile.as_deref().map(lockfile).transpose()?,
        input_schema: body.input_schema_bytes()?,
        output_schema: body.output_schema_bytes()?,
        policy: body.policy.clone().unwrap_or_default().request()?,
        retention: body.retention_days.map(retention).transpose()?,
    };
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::ScriptWrite).await?;
    let created = scripts(&state)?
        .create(&caller.session.principal, namespace, spec, &caller.ctx)
        .await?;
    Ok((StatusCode::CREATED, Json(ScriptBody::from(&created))))
}

#[derive(Debug, Default, Deserialize)]
pub struct ScriptPage {
    /// The name after which the listing continues.
    pub after: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_scripts<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(page): Query<ScriptPage>,
) -> Result<Json<ScriptListBody>, ApiError> {
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::ScriptRead).await?;
    let items = scripts(&state)?
        .list(
            &caller.session.principal,
            namespace,
            page.after.as_deref(),
            page.limit.unwrap_or(50),
            &caller.ctx,
        )
        .await?;
    Ok(Json(ScriptListBody {
        items: items.iter().map(ScriptSummaryBody::from).collect(),
    }))
}

pub async fn get_script<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<Json<ScriptBody>, ApiError> {
    let found = admitted_script(&state, &caller, &namespace, &id, ApiOperation::ScriptRead).await?;
    Ok(Json(ScriptBody::from(&found)))
}

pub async fn update_script<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<UpdateScriptBody>,
) -> Result<Json<ScriptBody>, ApiError> {
    let change = DraftChange {
        source: body.source.as_deref().map(source).transpose()?,
        lockfile: body
            .lockfile
            .as_ref()
            .map(|inner| inner.as_deref().map(lockfile).transpose())
            .transpose()?,
        input_schema: body.input_schema_change()?,
        output_schema: body.output_schema_change()?,
        policy: body.policy.as_ref().map(|p| p.request()).transpose()?,
        retention: body.retention_days.map(retention).transpose()?,
    };
    admitted_script(&state, &caller, &namespace, &id, ApiOperation::ScriptWrite).await?;
    let updated = scripts(&state)?
        .update(
            &caller.session.principal,
            namespace_id(&namespace)?,
            script_id(&id)?,
            change,
            body.revision,
            &caller.ctx,
        )
        .await?;
    Ok(Json(ScriptBody::from(&updated)))
}

pub async fn delete_script<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    admitted_script(&state, &caller, &namespace, &id, ApiOperation::ScriptWrite).await?;
    scripts(&state)?
        .delete(
            &caller.session.principal,
            namespace_id(&namespace)?,
            script_id(&id)?,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Publishes the draft. `202` for a new version (its build is queued), `200` for the latest
/// version when the draft is the same.
pub async fn publish_version<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<(StatusCode, Json<PublishedBody>), ApiError> {
    admitted_script(
        &state,
        &caller,
        &namespace,
        &id,
        ApiOperation::ScriptPublish,
    )
    .await?;
    let published = scripts(&state)?
        .publish(
            &caller.session.principal,
            namespace_id(&namespace)?,
            script_id(&id)?,
            &caller.ctx,
        )
        .await?;
    let status = if published.created {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(PublishedBody {
            version: VersionBody::from(&published.version),
            created: published.created,
        }),
    ))
}

#[derive(Debug, Default, Deserialize)]
pub struct VersionPage {
    /// The number below which the listing continues.
    pub before: Option<u32>,
    pub limit: Option<u32>,
}

pub async fn list_versions<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    Query(page): Query<VersionPage>,
) -> Result<Json<VersionListBody>, ApiError> {
    let before = page
        .before
        .map(|n| VersionNumber::new(n).map_err(|_| invalid("a version number is 1 or more")))
        .transpose()?;
    admitted_script(&state, &caller, &namespace, &id, ApiOperation::ScriptRead).await?;
    let items = scripts(&state)?
        .versions(
            &caller.session.principal,
            namespace_id(&namespace)?,
            script_id(&id)?,
            before,
            page.limit.unwrap_or(50),
            &caller.ctx,
        )
        .await?;
    Ok(Json(VersionListBody {
        items: items.iter().map(VersionSummaryBody::from).collect(),
    }))
}

pub async fn get_version<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id, number)): Path<(String, String, u32)>,
) -> Result<Json<VersionBody>, ApiError> {
    let number = VersionNumber::new(number).map_err(|_| ApiError::not_found())?;
    admitted_script(&state, &caller, &namespace, &id, ApiOperation::ScriptRead).await?;
    let found = scripts(&state)?
        .version(
            &caller.session.principal,
            namespace_id(&namespace)?,
            script_id(&id)?,
            number,
            &caller.ctx,
        )
        .await?;
    Ok(Json(VersionBody::from(&found)))
}

pub async fn create_test_run<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    ApiJson(body): ApiJson<TestRunRequestBody>,
) -> Result<(StatusCode, Json<TestRunBody>), ApiError> {
    let input = serde_json::to_vec(&body.input).map_err(|_| invalid("the input is not JSON"))?;
    admitted_script(&state, &caller, &namespace, &id, ApiOperation::ScriptWrite).await?;
    let run = scripts(&state)?
        .test(
            &caller.session.principal,
            namespace_id(&namespace)?,
            script_id(&id)?,
            input,
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::ACCEPTED, Json(TestRunBody::from(&run))))
}

pub async fn get_test_run<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id, run)): Path<(String, String, String)>,
) -> Result<Json<TestRunBody>, ApiError> {
    let run: TestRunId = run.parse().map_err(|_| ApiError::not_found())?;
    admitted_script(&state, &caller, &namespace, &id, ApiOperation::ScriptWrite).await?;
    let found = scripts(&state)?
        .test_run(
            &caller.session.principal,
            namespace_id(&namespace)?,
            script_id(&id)?,
            run,
            &caller.ctx,
        )
        .await?;
    Ok(Json(TestRunBody::from(&found)))
}

#[derive(Debug, Default, Deserialize)]
pub struct Limit {
    pub limit: Option<u32>,
}

pub async fn list_test_runs<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
    Query(page): Query<Limit>,
) -> Result<Json<TestRunListBody>, ApiError> {
    admitted_script(&state, &caller, &namespace, &id, ApiOperation::ScriptWrite).await?;
    let items = scripts(&state)?
        .test_runs(
            &caller.session.principal,
            namespace_id(&namespace)?,
            script_id(&id)?,
            page.limit.unwrap_or(20),
            &caller.ctx,
        )
        .await?;
    Ok(Json(TestRunListBody {
        items: items.iter().map(TestRunSummaryBody::from).collect(),
    }))
}

#[derive(Debug, Default, Deserialize)]
pub struct ExecutionPage {
    /// Only the executions of this artifact.
    pub digest: Option<String>,
    /// The id after which (older) the listing continues.
    pub before: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_executions<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(page): Query<ExecutionPage>,
) -> Result<Json<ExecutionListBody>, ApiError> {
    let digest = page
        .digest
        .as_deref()
        .map(|d| {
            Digest::parse(d).map_err(|_| invalid("a digest is `sha256:` and 64 hexadecimal digits"))
        })
        .transpose()?;
    let before = page
        .before
        .as_deref()
        .map(|b| {
            b.parse::<Uuid>()
                .map_err(|_| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    let namespace =
        admitted_namespace(&state, &caller, &namespace, ApiOperation::ScriptRead).await?;
    let items = scripts(&state)?
        .executions(
            &caller.session.principal,
            namespace,
            digest.as_ref(),
            before,
            page.limit.unwrap_or(50),
            &caller.ctx,
        )
        .await?;
    Ok(Json(ExecutionListBody {
        items: items.iter().map(ExecutionSummaryBody::from).collect(),
    }))
}

pub async fn get_artifact<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(digest): Path<String>,
) -> Result<Json<ArtifactSummaryBody>, ApiError> {
    let digest = Digest::parse(&digest).map_err(|_| ApiError::not_found())?;
    let found = scripts(&state)?
        .artifact(&caller.session.principal, &digest)
        .await?;
    Ok(Json(ArtifactSummaryBody::from(&found)))
}

pub async fn get_execution<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<Json<ExecutionBody>, ApiError> {
    let id: Uuid = id.parse().map_err(|_| ApiError::not_found())?;
    let found = scripts(&state)?
        .execution(
            &caller.session.principal,
            namespace_id(&namespace)?,
            id,
            &caller.ctx,
        )
        .await?;
    admit(
        &state,
        &caller,
        ApiOperation::ScriptRead,
        ApiTarget::Namespace(found.execution.namespace_id),
    )
    .await?;
    Ok(Json(ExecutionBody::from(&found)))
}

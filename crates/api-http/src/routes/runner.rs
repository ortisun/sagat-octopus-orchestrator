//! What the sandbox's runner asks of the control plane: the builds and test runs to take, the
//! artifacts to run, the policy to enforce, and where to say what happened.
//!
//! The runner is trusted with a tenant's source and artifacts and with nothing else: it holds the
//! `runner` role, which has no right to read, write or publish a script, to read a secret, or to
//! reach another tenant. It never touches the database, and the control plane never touches the
//! container engine.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sagat_application::scripts::{
    BuildFailureKind, BuildReport, ExecutionStatus, NewExecution, RunnerService, TestReport,
};
use sagat_domain::identity::NamespaceId;
use sagat_domain::scripts::{BuildJobId, Digest, Failure, Language, TestRunId};
use uuid::Uuid;

use crate::dto_scripts::{
    BuildAssignmentBody, ClaimBuildBody, ClaimTestBody, CompleteBuildBody, CompleteTestBody,
    LeaseBody, LeaseExtendedBody, PendingBody, RecordExecutionBody, RecordedBody,
    RunnerArtifactBody, RunnerEgressBody, SigningKeyBody, TestAssignmentBody, decode_artifact,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::governance::Governance;

/// Largest body a runner sends: an artifact of 2 MiB in base64 with its bill of materials and log.
pub const RUNNER_MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

fn runner<B: Governance>(state: &B) -> Result<&RunnerService<B::Store, B::Clock>, ApiError> {
    state.runner().ok_or_else(ApiError::not_found)
}

fn invalid(message: &'static str) -> ApiError {
    ApiError::validation(message)
}

fn language(text: &str) -> Result<Language, ApiError> {
    text.parse()
        .map_err(|_| invalid("the language is typescript, python, csharp, go or java"))
}

pub async fn claim_build<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    ApiJson(body): ApiJson<ClaimBuildBody>,
) -> Result<Response, ApiError> {
    let toolchains = body
        .toolchains
        .iter()
        .map(|(l, t)| Ok((language(l)?, t.clone())))
        .collect::<Result<Vec<_>, ApiError>>()?;
    let claimed = runner(&state)?
        .claim_build(&caller.session.principal, &toolchains, &caller.ctx)
        .await?;
    Ok(claimed.map_or_else(
        || StatusCode::NO_CONTENT.into_response(),
        |job| Json(BuildAssignmentBody::from(&job)).into_response(),
    ))
}

fn job_id(raw: &str) -> Result<BuildJobId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

fn test_id(raw: &str) -> Result<TestRunId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

pub async fn heartbeat_build<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(job): Path<String>,
    ApiJson(body): ApiJson<LeaseBody>,
) -> Result<Json<LeaseExtendedBody>, ApiError> {
    runner(&state)?
        .heartbeat_build(&caller.session.principal, job_id(&job)?, &body.lease_token)
        .await?
        .map(|lease| {
            Json(LeaseExtendedBody {
                lease_seconds: lease.as_secs(),
            })
        })
        .ok_or_else(|| ApiError::from(sagat_application::error::AppError::StaleAttempt))
}

pub async fn complete_build<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(job): Path<String>,
    ApiJson(body): ApiJson<CompleteBuildBody>,
) -> Result<StatusCode, ApiError> {
    let log = body.log.clone().unwrap_or_default();
    let report = match body.outcome.as_str() {
        "built" => BuildReport::Built {
            artifact: decode_artifact(
                body.artifact
                    .as_deref()
                    .ok_or_else(|| invalid("a built artifact is required"))?,
            )?,
            sbom: body
                .sbom
                .as_ref()
                .map(serde_json::to_vec)
                .transpose()
                .map_err(|_| invalid("the bill of materials is not JSON"))?
                .unwrap_or_else(|| b"{}".to_vec()),
            toolchain: body
                .toolchain
                .clone()
                .filter(|t| !t.is_empty() && t.len() <= 512)
                .ok_or_else(|| invalid("the toolchain that built it is required"))?,
            log,
        },
        "failed" => BuildReport::Failed {
            kind: match body.kind.as_deref() {
                Some("rejected") => BuildFailureKind::Rejected,
                Some("lockfile") => BuildFailureKind::Lockfile,
                Some("limit") => BuildFailureKind::Limit,
                Some("unsupported") => BuildFailureKind::Unsupported,
                Some("unavailable") => BuildFailureKind::Unavailable,
                _ => {
                    return Err(invalid(
                        "kind is rejected, lockfile, limit, unsupported or unavailable",
                    ));
                }
            },
            message: body.message.clone().unwrap_or_default(),
            log,
        },
        _ => return Err(invalid("outcome is built or failed")),
    };
    runner(&state)?
        .complete_build(
            &caller.session.principal,
            job_id(&job)?,
            &body.lease_token,
            report,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn claim_test<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    ApiJson(body): ApiJson<ClaimTestBody>,
) -> Result<Response, ApiError> {
    let languages = body
        .languages
        .iter()
        .map(|l| language(l))
        .collect::<Result<Vec<_>, _>>()?;
    let claimed = runner(&state)?
        .claim_test(&caller.session.principal, &languages)
        .await?;
    Ok(claimed.map_or_else(
        || StatusCode::NO_CONTENT.into_response(),
        |run| Json(TestAssignmentBody::from(&run)).into_response(),
    ))
}

pub async fn heartbeat_test<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(run): Path<String>,
    ApiJson(body): ApiJson<LeaseBody>,
) -> Result<Json<LeaseExtendedBody>, ApiError> {
    runner(&state)?
        .heartbeat_test(&caller.session.principal, test_id(&run)?, &body.lease_token)
        .await?
        .map(|lease| {
            Json(LeaseExtendedBody {
                lease_seconds: lease.as_secs(),
            })
        })
        .ok_or_else(|| ApiError::from(sagat_application::error::AppError::StaleAttempt))
}

fn failure(code: &str) -> Result<Failure, ApiError> {
    Failure::from_code(code).ok_or_else(|| invalid("failure is one of the SCRIPT_* codes"))
}

fn schema(value: Option<&serde_json::Value>) -> Result<Option<Vec<u8>>, ApiError> {
    value
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|_| invalid("a schema is JSON"))
}

pub async fn complete_test<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(run): Path<String>,
    ApiJson(body): ApiJson<CompleteTestBody>,
) -> Result<StatusCode, ApiError> {
    let result = match (&body.result, &body.failure) {
        (Some(value), None) => Ok(serde_json::to_vec(&value.clone().unwrap_or_default())
            .map_err(|_| invalid("the result is JSON"))?),
        (None, Some(code)) => Err(failure(code)?),
        _ => {
            return Err(invalid(
                "a test run has a result or a failure, not both and not neither",
            ));
        }
    };
    let report = TestReport {
        result,
        log: body.log.clone(),
        log_truncated: body.log_truncated,
        denied: body.denied.clone(),
        exit_code: body.exit_code,
        wall_ms: body.wall_ms,
        start_ms: body.start_ms,
        memory_peak_bytes: body.memory_peak_bytes,
        inferred_input_schema: schema(body.inferred_input_schema.as_ref())?,
        inferred_output_schema: schema(body.inferred_output_schema.as_ref())?,
    };
    runner(&state)?
        .complete_test(
            &caller.session.principal,
            test_id(&run)?,
            &body.lease_token,
            report,
            &caller.ctx,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_artifact<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(digest): Path<String>,
) -> Result<Json<RunnerArtifactBody>, ApiError> {
    let digest = Digest::parse(&digest).map_err(|_| ApiError::not_found())?;
    let found = runner(&state)?
        .artifact(&caller.session.principal, &digest)
        .await?;
    Ok(Json(RunnerArtifactBody::from(&found.artifact)))
}

pub async fn signing_key<B: Governance>(
    caller: Caller,
    State(state): State<B>,
) -> Result<Json<SigningKeyBody>, ApiError> {
    caller
        .session
        .principal
        .authorize(sagat_domain::identity::Permission::ScriptExecute)
        .map_err(sagat_application::error::AppError::from)?;
    let (key_id, public_key) = runner(&state)?.public_key();
    Ok(Json(SigningKeyBody::new(&key_id, &public_key)))
}

pub async fn egress_policy<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<RunnerEgressBody>, ApiError> {
    let namespace: NamespaceId = namespace.parse().map_err(|_| ApiError::not_found())?;
    let policy = state
        .egress()
        .for_runner(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    Ok(Json(RunnerEgressBody {
        rules: policy.rules,
        ports: policy.ports,
        allow_http: policy.allow_http,
    }))
}

pub async fn pending_activities<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
) -> Result<Json<PendingBody>, ApiError> {
    let namespace: NamespaceId = namespace.parse().map_err(|_| ApiError::not_found())?;
    let activities = runner(&state)?
        .pending_activities(&caller.session.principal, namespace, &caller.ctx)
        .await?;
    Ok(Json(PendingBody { activities }))
}

pub async fn record_execution<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    ApiJson(body): ApiJson<RecordExecutionBody>,
) -> Result<(StatusCode, Json<RecordedBody>), ApiError> {
    let execution = NewExecution {
        namespace_id: body
            .namespace_id
            .parse()
            .map_err(|_| invalid("namespaceId is not an id"))?,
        digest: Digest::parse(&body.digest)
            .map_err(|_| invalid("a digest is `sha256:` and 64 hexadecimal digits"))?,
        run_id: body
            .run_id
            .as_deref()
            .map(|id| {
                id.parse::<Uuid>()
                    .map_err(|_| invalid("runId is not an id"))
            })
            .transpose()?,
        node: body.node.clone(),
        attempt: body.attempt,
        status: match body.status.as_str() {
            "SUCCEEDED" => ExecutionStatus::Succeeded,
            "FAILED" => ExecutionStatus::Failed,
            _ => return Err(invalid("status is SUCCEEDED or FAILED")),
        },
        failure: body.failure.as_deref().map(failure).transpose()?,
        exit_code: body.exit_code,
        log: body.log.clone(),
        log_truncated: body.log_truncated,
        denied: body.denied.clone(),
        wall_ms: body.wall_ms,
        start_ms: body.start_ms,
        memory_peak_bytes: body.memory_peak_bytes,
        trace_id: body.trace_id.clone(),
    };
    let recorded = runner(&state)?
        .record_execution(&caller.session.principal, execution, &caller.ctx)
        .await?;
    Ok((StatusCode::CREATED, Json(RecordedBody { recorded })))
}

//! The worker protocol over gRPC: authenticate the worker, check the request, call the use case,
//! map the outcome. No business rule lives here; the lease, fencing and idempotency semantics are
//! the application's (`WorkerService`).

use std::future::Future;
use std::time::Instant;

use sagat_api_http::error::ApiError;
use sagat_api_http::{Authenticated, Core, authenticate_bearer, parse_bearer};
use sagat_application::clock::Clock;
use sagat_application::definitions::DefinitionStore;
use sagat_application::entropy::Entropy;
use sagat_application::error::AppError;
use sagat_application::execution::{ExpressionEvaluator, MAX_PAYLOAD_BYTES};
use sagat_application::runs::RunStore;
use sagat_application::worker::{Capabilities, ClaimedTask, StepAck, WorkerService, WorkerStore};
use sagat_telemetry::TraceContext;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tonic::{Code, Request, Response, Status};
use tracing::Instrument;

use crate::pb;
use crate::request::{Attempt, idempotency_key, worker_id};
use crate::status::{invalid, to_status};

/// The gRPC adapter of [`WorkerService`]. `St` carries token verification, the budgets and the
/// identity lookup (the same [`Core`] the REST API authenticates with).
pub struct WorkerGrpc<St, S, C, E, R> {
    core: St,
    workers: WorkerService<S, C, E, R>,
}

impl<St, S, C, E, R> WorkerGrpc<St, S, C, E, R> {
    #[must_use]
    pub const fn new(core: St, workers: WorkerService<S, C, E, R>) -> Self {
        Self { core, workers }
    }
}

fn app(error: AppError) -> Status {
    to_status(&ApiError::from(error))
}

fn api(error: &ApiError) -> Status {
    to_status(error)
}

fn trace_of<T>(request: &Request<T>) -> TraceContext {
    TraceContext::from_traceparent(
        request
            .metadata()
            .get("traceparent")
            .and_then(|value| value.to_str().ok()),
    )
}

const fn code_label(code: Code) -> &'static str {
    match code {
        Code::Ok => "ok",
        Code::Cancelled => "cancelled",
        Code::Unknown => "unknown",
        Code::InvalidArgument => "invalid_argument",
        Code::DeadlineExceeded => "deadline_exceeded",
        Code::NotFound => "not_found",
        Code::AlreadyExists => "already_exists",
        Code::PermissionDenied => "permission_denied",
        Code::ResourceExhausted => "resource_exhausted",
        Code::FailedPrecondition => "failed_precondition",
        Code::Aborted => "aborted",
        Code::OutOfRange => "out_of_range",
        Code::Unimplemented => "unimplemented",
        Code::Internal => "internal",
        Code::Unavailable => "unavailable",
        Code::DataLoss => "data_loss",
        Code::Unauthenticated => "unauthenticated",
    }
}

/// Runs one call under a span carrying the trace id, and counts and times it. The `method` label
/// is one of five constants, so the metric cardinality is fixed.
async fn observe<T>(
    method: &'static str,
    trace: TraceContext,
    call: impl Future<Output = Result<T, Status>>,
) -> Result<Response<T>, Status> {
    let started = Instant::now();
    let span = tracing::info_span!("grpc.worker", method, trace_id = %trace.trace_id());
    let result = call.instrument(span).await;
    let code = result.as_ref().err().map_or(Code::Ok, Status::code);
    metrics::counter!("grpc_requests_total", "method" => method, "code" => code_label(code))
        .increment(1);
    metrics::histogram!("grpc_request_duration_seconds", "method" => method)
        .record(started.elapsed().as_secs_f64());
    if code == Code::Internal {
        tracing::error!(method, "worker call failed internally");
    }
    result.map(Response::new)
}

impl<St, S, C, E, R> WorkerGrpc<St, S, C, E, R>
where
    St: Core,
{
    /// Who is calling: the bearer token of the `authorization` metadata, verified and resolved to a
    /// member of a tenant. The returned value holds the tenant's concurrency slot until dropped.
    async fn authenticate<T>(&self, request: &Request<T>) -> Result<Authenticated, Status> {
        let mut values = request.metadata().get_all("authorization").iter();
        let (Some(value), None) = (values.next(), values.next()) else {
            return Err(api(&ApiError::missing_credentials()));
        };
        let value = value
            .to_str()
            .map_err(|_| api(&ApiError::invalid_credentials()))?;
        let token = parse_bearer(value).map_err(|e| api(&e))?;
        authenticate_bearer(&self.core, token)
            .await
            .map_err(|e| api(&e))
    }
}

#[tonic::async_trait]
impl<St, S, C, E, R> pb::worker_service_server::WorkerService for WorkerGrpc<St, S, C, E, R>
where
    St: Core,
    S: RunStore + WorkerStore + DefinitionStore + 'static,
    C: Clock + 'static,
    E: ExpressionEvaluator + 'static,
    R: Entropy + 'static,
{
    async fn register_worker(
        &self,
        request: Request<pb::RegisterWorkerRequest>,
    ) -> Result<Response<pb::RegisterWorkerResponse>, Status> {
        observe("RegisterWorker", trace_of(&request), async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let wanted = message
                .capabilities
                .as_ref()
                .ok_or_else(|| invalid("capabilities are required"))?;
            let capabilities = Capabilities {
                activities: wanted.activities.clone(),
                languages: wanted.languages.clone(),
                pools: wanted.pools.clone(),
                max_inflight: u32::try_from(wanted.max_inflight)
                    .map_err(|_| invalid("max_inflight must be positive"))?,
                protocol_version: wanted.protocol_version.clone(),
            };
            let registered = self
                .workers
                .register(
                    &auth.session.principal,
                    &message.namespace,
                    &message.worker_name,
                    &capabilities,
                )
                .await
                .map_err(app)?;
            // Which version workers are served at: the operator's answer to "can an old minor be dropped".
            // The accepted version is one of the few the server speaks, so the label stays small.
            metrics::counter!(
                "worker_registrations_total",
                "protocol" => registered.accepted_protocol_version.clone()
            )
            .increment(1);
            Ok(pb::RegisterWorkerResponse {
                worker_id: registered.worker_id.to_string(),
                session_token: registered.session_token.expose().to_owned(),
                accepted_protocol_version: registered.accepted_protocol_version,
            })
        })
        .await
    }

    async fn poll_tasks(
        &self,
        request: Request<pb::PollTasksRequest>,
    ) -> Result<Response<pb::PollTasksResponse>, Status> {
        let trace = trace_of(&request);
        observe("PollTasks", trace, async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let worker = worker_id(&message.worker_id)?;
            let max_tasks = u32::try_from(message.max_tasks)
                .map_err(|_| invalid("max_tasks must not be negative"))?;
            let tasks = self
                .workers
                .poll(
                    &auth.session.principal,
                    worker,
                    &message.session_token,
                    max_tasks,
                    Some(&trace.trace_id().to_string()),
                )
                .await
                .map_err(app)?;
            let retry_after_ms = if tasks.is_empty() {
                i32::try_from(self.workers.settings().idle_retry.as_millis()).unwrap_or(i32::MAX)
            } else {
                0
            };
            Ok(pb::PollTasksResponse {
                tasks: tasks.iter().map(lease).collect(),
                retry_after_ms,
            })
        })
        .await
    }

    async fn resolve_secrets(
        &self,
        request: Request<pb::ResolveSecretsRequest>,
    ) -> Result<Response<pb::ResolveSecretsResponse>, Status> {
        let trace = trace_of(&request);
        observe("ResolveSecrets", trace, async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let attempt = Attempt::parse(
                &message.task_id,
                &message.run_id,
                &message.step_id,
                message.attempt,
                message.generation,
                &message.lease_token,
            )?;
            let resolved = self
                .workers
                .resolve_secrets(
                    &auth.session.principal,
                    &attempt.task_ref(),
                    Some(&trace.trace_id().to_string()),
                )
                .await
                .map_err(app)?;
            Ok(pb::ResolveSecretsResponse {
                secrets: resolved
                    .iter()
                    .map(|secret| pb::ResolvedSecret {
                        name: secret.name.to_string(),
                        value: secret.value.expose().to_vec(),
                        version: secret.version,
                    })
                    .collect(),
            })
        })
        .await
    }

    async fn heartbeat(
        &self,
        request: Request<pb::HeartbeatRequest>,
    ) -> Result<Response<pb::HeartbeatResponse>, Status> {
        observe("Heartbeat", trace_of(&request), async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            if !message.worker_id.is_empty() {
                worker_id(&message.worker_id)?;
            }
            let attempt = Attempt::parse_heartbeat(
                &message.task_id,
                &message.run_id,
                &message.step_id,
                message.attempt,
                message.generation,
                &message.lease_token,
            )?;
            let ack = self
                .workers
                .heartbeat(&auth.session.principal, &attempt.task_ref())
                .await
                .map_err(app)?;
            Ok(pb::HeartbeatResponse {
                lease_expires_at_epoch_ms: epoch_ms(ack.lease_expires_at),
                cancel_requested: ack.cancel_requested,
            })
        })
        .await
    }

    async fn complete_step(
        &self,
        request: Request<pb::CompleteStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        observe("CompleteStep", trace_of(&request), async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let attempt = Attempt::parse(
                &message.task_id,
                &message.run_id,
                &message.step_id,
                message.attempt,
                message.generation,
                &message.lease_token,
            )?;
            let key = idempotency_key(&message.idempotency_key)?;
            if !message.output_blob_ref.is_empty() {
                // Large outputs need the payload store (gap G19); refusing is honest, ignoring
                // the reference would silently lose the output.
                return Err(invalid(
                    "output_blob_ref is not supported yet; send the output inline (at most 256 KiB)",
                ));
            }
            let output = output_of(&message.output_json, &message.output_sha256)?;
            let ack = self
                .workers
                .complete(&auth.session.principal, &attempt.task_ref(), &output, key)
                .await
                .map_err(app)?;
            Ok(step_response(&ack))
        })
        .await
    }

    async fn fail_step(
        &self,
        request: Request<pb::FailStepRequest>,
    ) -> Result<Response<pb::StepCommandResponse>, Status> {
        observe("FailStep", trace_of(&request), async {
            let auth = self.authenticate(&request).await?;
            let message = request.get_ref();
            let attempt = Attempt::parse(
                &message.task_id,
                &message.run_id,
                &message.step_id,
                message.attempt,
                message.generation,
                &message.lease_token,
            )?;
            let key = idempotency_key(&message.idempotency_key)?;
            if message.error_code.len() > 128 || message.safe_message.len() > 2048 {
                return Err(invalid("error_code or safe_message is too long"));
            }
            let ack = self
                .workers
                .fail(
                    &auth.session.principal,
                    &attempt.task_ref(),
                    &message.error_code,
                    message.retryable,
                    key,
                )
                .await
                .map_err(app)?;
            Ok(step_response(&ack))
        })
        .await
    }
}

/// The inline output of a completed step. An empty output means "no value" and is recorded as JSON
/// `null`; a stated `output_sha256` must match the bytes received (end-to-end integrity).
fn output_of(output_json: &str, stated_sha256: &str) -> Result<Vec<u8>, Status> {
    if output_json.len() > MAX_PAYLOAD_BYTES {
        return Err(invalid("output_json is larger than 256 KiB"));
    }
    if !stated_sha256.is_empty() {
        let digest = Sha256::digest(output_json.as_bytes());
        let actual = digest
            .iter()
            .fold(String::with_capacity(64), |mut hex, byte| {
                use std::fmt::Write as _;
                let _ = write!(hex, "{byte:02x}");
                hex
            });
        if !actual.eq_ignore_ascii_case(stated_sha256) {
            return Err(invalid("output_sha256 does not match output_json"));
        }
    }
    Ok(if output_json.is_empty() {
        b"null".to_vec()
    } else {
        output_json.as_bytes().to_vec()
    })
}

fn step_response(ack: &StepAck) -> pb::StepCommandResponse {
    pb::StepCommandResponse {
        command_id: ack.command_id.clone(),
        already_applied: ack.already_applied,
        step_status: ack.step_status.as_str().to_owned(),
        run_status: ack.run_status.as_str().to_owned(),
    }
}

fn lease(task: &ClaimedTask) -> pb::TaskLease {
    pb::TaskLease {
        task_id: task.task_id(),
        run_id: task.run_id.to_string(),
        step_id: task.step_id.to_string(),
        attempt: i32::try_from(task.fence.attempt).unwrap_or(i32::MAX),
        generation: i64::try_from(task.fence.generation).unwrap_or(i64::MAX),
        lease_token: task.lease_token.expose().to_owned(),
        expires_at_epoch_ms: epoch_ms(task.expires_at),
        activity_name: task.activity.to_string(),
        input_json: String::from_utf8_lossy(&task.input).into_owned(),
        input_blob_ref: String::new(),
        traceparent: traceparent(task),
        // The tenant is implicit in the worker's identity; its slug is not disclosed to code that
        // does not need it.
        tenant_alias: String::new(),
        idempotency_key: task.idempotency_key.clone(),
        task_kind: task.kind.as_str().to_owned(),
        target_node: task
            .target
            .as_ref()
            .map(|t| t.node.to_string())
            .unwrap_or_default(),
        target_idempotency_key: task
            .target
            .as_ref()
            .map(|t| t.idempotency_key.clone())
            .unwrap_or_default(),
    }
}

/// The run's trace continued into the worker: same trace id, a span id derived from the step.
fn traceparent(task: &ClaimedTask) -> String {
    let Some(trace) = task
        .trace_id
        .as_deref()
        .filter(|id| sagat_telemetry::TraceId::parse(id).is_some())
    else {
        return String::new();
    };
    let span: String = task
        .step_id
        .as_bytes()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let span = if span.bytes().all(|b| b == b'0') {
        "0000000000000001".to_owned()
    } else {
        span
    };
    format!("00-{trace}-{span}-01")
}

fn epoch_ms(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos() / 1_000_000).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_output_is_json_null_and_a_stated_digest_is_checked() {
        assert_eq!(output_of("", "").expect("ok"), b"null");
        assert_eq!(output_of("{\"a\":1}", "").expect("ok"), b"{\"a\":1}");
        // sha256("{}")
        let digest = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
        assert!(output_of("{}", digest).is_ok());
        assert!(output_of("{}", &digest.to_uppercase()).is_ok());
        assert!(output_of("{ }", digest).is_err());
        assert!(output_of("{}", "00").is_err());
    }

    #[test]
    fn oversized_output_is_refused() {
        let big = "1".repeat(MAX_PAYLOAD_BYTES + 1);
        assert!(output_of(&big, "").is_err());
    }
}

//! One capability, one SQL dispatch, and idempotent terminal reporting. SQL is never retried here.
use crate::{
    RunnerError, capability,
    config::Config,
    rpc::{Failure, Rpc},
};
use sagat_api_grpc::pb;
use sagat_domain::{
    database::{DatabaseFailure, MANAGED_DATABASE_ACTIVITY, OperationKind},
    identity::NamespaceId,
};
use sagat_infra_database::{DatabaseResult, PostgresExecutor};
use sha2::{Digest as _, Sha256};
use std::{path::Path, sync::Arc};
use tokio::{
    sync::watch,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub(crate) struct Task {
    pub lease: pb::TaskLease,
    pub namespace: NamespaceId,
    pub slug: String,
    pub worker_id: String,
}

pub(crate) async fn run(
    rpc: Rpc,
    config_path: Arc<Path>,
    task: Task,
    development: bool,
    cancelled: CancellationToken,
    slots: Arc<crate::slots::Slots>,
) -> Result<(), RunnerError> {
    let lease = &task.lease;
    let initial = capability::deadline(lease.expires_at_epoch_ms).ok_or(RunnerError::Capability)?;
    if lease.activity_name != MANAGED_DATABASE_ACTIVITY {
        return Err(RunnerError::Capability);
    }
    let heartbeat = Heartbeat::start(
        rpc.clone(),
        lease,
        task.worker_id.clone(),
        initial,
        cancelled.clone(),
    );
    let outcome = execute(
        &rpc,
        &config_path,
        &task,
        development,
        &cancelled,
        initial,
        &slots,
    )
    .await;
    let outcome_code = outcome
        .as_ref()
        .err()
        .map_or("success", |failure| failure.code());
    metrics::counter!("database_runner_execution_total", "outcome"=>outcome_code).increment(1);
    tracing::info!(outcome = outcome_code, "database execution finished");
    let report = match outcome {
        Ok((kind, result)) => match upload(
            &rpc,
            lease,
            &result,
            &heartbeat,
            initial + Duration::from_secs(30),
        )
        .await
        {
            Ok(blob) => Report::Complete {
                bytes: result,
                blob,
            },
            Err(_) => Report::Fail(if kind == OperationKind::Execute {
                DatabaseFailure::OutcomeUnknown
            } else {
                DatabaseFailure::ResultUnavailable
            }),
        },
        Err(failure) => Report::Fail(failure),
    };
    // An output/ack may be lost, but a report retry always reuses exactly the same content and key.
    let result = report_until_ack(&rpc, lease, &report, &heartbeat).await;
    heartbeat.finish().await;
    result
}
async fn execute(
    rpc: &Rpc,
    path: &Path,
    task: &Task,
    development: bool,
    cancelled: &CancellationToken,
    deadline: Instant,
    slots: &Arc<crate::slots::Slots>,
) -> Result<(OperationKind, DatabaseResult), DatabaseFailure> {
    let prepare = async {
        let config = Config::read(path)
            .await
            .map_err(|_| DatabaseFailure::Configuration)?;
        if config.development_loopback != development {
            return Err(DatabaseFailure::Configuration);
        }
        let policy = config
            .policy(task.namespace, &task.slug)
            .map_err(|_| DatabaseFailure::DestinationDenied)?;
        let scope = policy
            .scope()
            .map_err(|_| DatabaseFailure::DestinationDenied)?;
        let ceiling = config
            .deployment
            .limits
            .validated()
            .map_err(|_| DatabaseFailure::Configuration)?
            .intersect(
                policy
                    .limits
                    .validated()
                    .map_err(|_| DatabaseFailure::Configuration)?,
            );
        let request = rpc
            .request(crate::rpc::task(&task.lease))
            .await
            .map_err(|_| DatabaseFailure::Credential)?;
        let response = rpc
            .client
            .clone()
            .resolve_database(request)
            .await
            .map_err(|status| match status.code() {
                tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => {
                    DatabaseFailure::Credential
                }
                _ => DatabaseFailure::Capability,
            })?
            .into_inner();
        let capability = capability::decode(response, task.namespace, ceiling)
            .map_err(|_| DatabaseFailure::Capability)?;
        let executor = PostgresExecutor::new(
            config
                .network()
                .map_err(|_| DatabaseFailure::Configuration)?,
            1,
        )
        .map_err(|e| e.failure())?;
        Ok((capability, executor, scope))
    };
    let prepared = tokio::select! {
        ()=cancelled.cancelled() => return Err(DatabaseFailure::Cancelled),
        result=tokio::time::timeout_at(deadline,prepare) => result.map_err(|_| DatabaseFailure::Timeout)??,
    };
    let (capability, executor, scope) = prepared;
    tracing::Span::current().record("tenant_id", tracing::field::display(capability.tenant_id));
    let _permit = slots.acquire(
        (task.namespace, capability.connection_id),
        capability.config.limits().pool_size,
    )?;
    let kind = capability.operation.kind;
    let result = executor
        .execute(
            &capability.config,
            &scope,
            capability.credentials,
            capability.operation,
            capability.deadline,
            cancelled,
        )
        .await
        .map_err(|e| e.failure())?;
    Ok((kind, result))
}

struct Heartbeat {
    latest: watch::Receiver<Instant>,
    stop: CancellationToken,
    join: Option<tokio::task::JoinHandle<()>>,
}
impl Heartbeat {
    fn start(
        rpc: Rpc,
        lease: &pb::TaskLease,
        worker_id: String,
        initial: Instant,
        cancelled: CancellationToken,
    ) -> Self {
        let (updates, latest) = watch::channel(initial);
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let heartbeat = pb::HeartbeatRequest {
            task_id: lease.task_id.clone(),
            worker_id,
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            run_id: lease.run_id.clone(),
            step_id: lease.step_id.clone(),
            attempt: lease.attempt,
        };
        let join = tokio::spawn(async move {
            let mut expires = initial;
            loop {
                let remaining = expires.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    cancelled.cancel();
                    return;
                }
                let pause = (remaining / 3)
                    .min(Duration::from_secs(2))
                    .max(Duration::from_millis(10));
                tokio::select! { ()=stopped.cancelled()=>return, ()=tokio::time::sleep(pause)=>{} }
                let response = tokio::time::timeout_at(expires, async {
                    let request = rpc.request(heartbeat.clone()).await?;
                    rpc.client
                        .clone()
                        .heartbeat(request)
                        .await
                        .map(tonic::Response::into_inner)
                        .map_err(Failure::from)
                })
                .await;
                let Ok(Ok(response)) = response else {
                    cancelled.cancel();
                    return;
                };
                if response.cancel_requested {
                    cancelled.cancel();
                    return;
                }
                let Some(next) = capability::deadline(response.lease_expires_at_epoch_ms) else {
                    cancelled.cancel();
                    return;
                };
                expires = next;
                updates.send_replace(next);
            }
        });
        Self {
            latest,
            stop,
            join: Some(join),
        }
    }
    fn deadline(&self, bound: Instant) -> Instant {
        (*self.latest.borrow()).min(bound)
    }
    async fn finish(mut self) {
        self.stop.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}
impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(join) = &self.join {
            join.abort();
        }
    }
}

enum Report {
    Complete {
        bytes: DatabaseResult,
        blob: Option<String>,
    },
    Fail(DatabaseFailure),
}
async fn upload(
    rpc: &Rpc,
    lease: &pb::TaskLease,
    result: &DatabaseResult,
    heartbeat: &Heartbeat,
    bound: Instant,
) -> Result<Option<String>, RunnerError> {
    if result.as_bytes().len() <= sagat_application::execution::MAX_PAYLOAD_BYTES {
        return Ok(None);
    }
    let digest =
        sagat_domain::workflow::Digest::new(Sha256::digest(result.as_bytes()).into()).to_hex();
    let header = pb::WriteBlobHeader {
        task_id: lease.task_id.clone(),
        run_id: lease.run_id.clone(),
        step_id: lease.step_id.clone(),
        attempt: lease.attempt,
        generation: lease.generation,
        lease_token: lease.lease_token.clone(),
        content_type: "application/json".into(),
        expected_size: result.as_bytes().len() as i64,
        expected_sha256: digest.clone(),
    };
    let mut frames = vec![pb::WriteBlobRequest {
        part: Some(pb::write_blob_request::Part::Header(header)),
    }];
    frames.extend(
        result
            .as_bytes()
            .chunks(64 * 1024)
            .map(|chunk| pb::WriteBlobRequest {
                part: Some(pb::write_blob_request::Part::Chunk(
                    bytes::Bytes::copy_from_slice(chunk),
                )),
            }),
    );
    let response = tokio::time::timeout_at(heartbeat.deadline(bound), async {
        let request = rpc.request(tokio_stream::iter(frames)).await?;
        rpc.client
            .clone()
            .write_blob(request)
            .await
            .map(tonic::Response::into_inner)
            .map_err(Failure::from)
    })
    .await
    .map_err(|_| RunnerError::Transport)?
    .map_err(|_| RunnerError::Transport)?;
    if response.blob_id.parse::<uuid::Uuid>().is_err()
        || response.sha256 != digest
        || response.size != result.as_bytes().len() as i64
        || response.content_type != "application/json"
    {
        return Err(RunnerError::Capability);
    }
    // Upload retries could allocate another blob after a lost ack; leave reconciliation to TTL.
    Ok(Some(response.blob_id))
}
async fn report_until_ack(
    rpc: &Rpc,
    lease: &pb::TaskLease,
    report: &Report,
    heartbeat: &Heartbeat,
) -> Result<(), RunnerError> {
    let bound = Instant::now() + Duration::from_secs(30);
    let mut pause = Duration::from_millis(100);
    loop {
        let deadline = heartbeat.deadline(bound);
        if deadline <= Instant::now() {
            return Err(RunnerError::Transport);
        }
        let outcome = tokio::time::timeout_at(deadline, send_report(rpc, lease, report))
            .await
            .map_err(|_| RunnerError::Transport)?;
        match outcome {
            Ok(()) => return Ok(()),
            Err(error) if error.retryable() => {
                metrics::counter!("database_runner_report_retries_total").increment(1);
                tokio::time::sleep_until((Instant::now() + pause).min(deadline)).await;
                pause = (pause * 2).min(Duration::from_secs(1));
            }
            Err(_) => return Err(RunnerError::Transport),
        }
    }
}
async fn send_report(rpc: &Rpc, lease: &pb::TaskLease, report: &Report) -> Result<(), Failure> {
    match report {
        Report::Complete { bytes, blob } => {
            let output_json = if blob.is_some() {
                String::new()
            } else {
                std::str::from_utf8(bytes.as_bytes())
                    .map_err(|_| Failure {
                        code: tonic::Code::DataLoss,
                        quota: false,
                    })?
                    .to_owned()
            };
            let request = pb::CompleteStepRequest {
                task_id: lease.task_id.clone(),
                run_id: lease.run_id.clone(),
                step_id: lease.step_id.clone(),
                attempt: lease.attempt,
                generation: lease.generation,
                lease_token: lease.lease_token.clone(),
                idempotency_key: format!(
                    "database-complete-{}-{}",
                    lease.task_id, lease.generation
                ),
                output_json,
                output_blob_ref: blob.clone().unwrap_or_default(),
                output_sha256: sagat_domain::workflow::Digest::new(
                    Sha256::digest(bytes.as_bytes()).into(),
                )
                .to_hex(),
                traceparent: lease.traceparent.clone(),
            };
            rpc.client
                .clone()
                .complete_step(rpc.request(request).await?)
                .await
                .map_err(Failure::from)?;
        }
        Report::Fail(failure) => {
            let request = pb::FailStepRequest {
                task_id: lease.task_id.clone(),
                run_id: lease.run_id.clone(),
                step_id: lease.step_id.clone(),
                attempt: lease.attempt,
                generation: lease.generation,
                lease_token: lease.lease_token.clone(),
                idempotency_key: format!("database-fail-{}-{}", lease.task_id, lease.generation),
                error_code: failure.code().into(),
                safe_message: String::new(),
                retryable: false,
                traceparent: lease.traceparent.clone(),
            };
            rpc.client
                .clone()
                .fail_step(rpc.request(request).await?)
                .await
                .map_err(Failure::from)?;
        }
    }
    Ok(())
}

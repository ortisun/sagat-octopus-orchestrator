//! One capability, one SQL dispatch, and idempotent terminal reporting. SQL is never retried here.
use crate::{
    RunnerError, capability,
    config::Config,
    lease::{Heartbeat, LeaseEnd, Report, report_until_ack},
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
use tokio::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// A SQL failure is never retried by the runner: the statement may have run.
fn failed(failure: DatabaseFailure) -> Report {
    Report::Fail {
        code: failure.code(),
        retryable: false,
    }
}

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
    if lease
        .instance
        .as_ref()
        .is_some_and(|instance| !instance.instance_path.is_empty())
        || lease.activity_name != MANAGED_DATABASE_ACTIVITY
    {
        return Err(RunnerError::Capability);
    }
    // A SQL attempt cancels whether the control plane asked for it or the lease is gone: it is never retried here.
    let on_end = {
        let cancelled = cancelled.clone();
        move |_: LeaseEnd| cancelled.cancel()
    };
    let heartbeat = Heartbeat::start(rpc.clone(), lease, task.worker_id.clone(), initial, on_end);
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
                bytes: result.as_bytes().to_vec(),
                blob,
            },
            Err(_) => failed(if kind == OperationKind::Execute {
                DatabaseFailure::OutcomeUnknown
            } else {
                DatabaseFailure::ResultUnavailable
            }),
        },
        Err(failure) => failed(failure),
    };
    // An output/ack may be lost, but a report retry always reuses exactly the same content and key.
    let result = report_until_ack(&rpc, lease, "database", &report, &heartbeat)
        .await
        .map_err(|_| RunnerError::Transport);
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
    // The SQL profile runs SQL connections only; a lake connection is answered by the lake profile (feature 035).
    let pool_size = capability
        .config
        .sql_limits()
        .ok_or(DatabaseFailure::Configuration)?
        .pool_size;
    let _permit = slots.acquire((task.namespace, capability.connection_id), pool_size)?;
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
        instance: lease.instance.clone(),
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

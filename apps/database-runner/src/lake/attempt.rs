//! One lake lease, from the task the runner polled to the report it sends (feature 035, T035-058, D035-143).
//!
//! The driver ([`Driver`]) knows how to run an attempt once it has its input, a slot and a connection. This is the part that talks
//! to the control plane around it:
//!
//! - *the lease is checked* before anything is asked for: a root forward task of either lake activity (the write, or the probe of
//!   a connection, D035-150), with an expiry;
//! - *the heartbeats* renew it, and what their end means is decided here: a cancel the control plane asked for is `cancelled`
//!   (a write that was started is then `OUTCOME_UNKNOWN`), a lease that is lost is `lease_lost` (there is no one to report to);
//! - *the attempt is admitted* (an input it can read, a free slot) **before the connection is asked for**, because asking for it
//!   discloses and audits the secrets;
//! - *the connection is resolved* under the same signals, and read by [`decode`](super::resolve::decode), which zeroises what it
//!   was given on every path; for a probe, by [`decode_for_probe`], which also holds the answer to the connection and the
//!   revision the lease names;
//! - *the report* is the driver's, sent with the flag the contract gives its code: `CompleteStep` with the receipt (or, for a
//!   probe, with its result inline), `FailStep` with a closed code, or nothing at all when the lease is gone. A completion the
//!   control plane refuses as not the step's becomes `LAKE_HOST_PROTOCOL`: the host did not keep the protocol.
use std::future::Future;
use std::sync::Arc;

use sagat_api_grpc::pb;
use sagat_domain::{
    database::{DatabaseFailure, ICEBERG_PROBE_ACTIVITY, ICEBERG_WRITE_ACTIVITY},
    identity::NamespaceId,
};
use sagat_infra_lake::attempt::{LakeSandbox, Signals};
use sagat_infra_lake::protocol::{IcebergOutcome, Receipt};
use sagat_infra_lake::relay::{Connector, Resolver};
use sagat_telemetry::lake::{
    LakeCatalogType, LakeMode, LakeRelayDenial, LakeStorageType, LakeWriteOutcome,
    record_relay_denial, record_sandbox_exit, record_write,
};
use tokio_util::sync::CancellationToken;

use super::driver::{Admitted, Driver, Finished, Report as Finish};
use super::prepare::Blobs;
use super::resolve::{decode, decode_for_probe};
use crate::{
    RunnerError, capability,
    lease::{Heartbeat, LeaseEnd, Policy, Report, Unreported, report_until_ack},
    rpc::Rpc,
};

pub(crate) struct Task {
    pub lease: pb::TaskLease,
    pub namespace: NamespaceId,
    pub worker_id: String,
}

/// How a lease ended for the runner's books: told to the control plane, or given up with nothing to tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disposition {
    Reported,
    /// The lease is gone, or the control plane could not be asked before anything was written: no report, and the lease expires on its own.
    Abandoned,
}

/// What an attempt tells the control plane, or nothing.
enum Outcome {
    Report(Report),
    /// The lease is gone: there is no one to tell.
    Nothing,
}

/// A failure as the report of a lake step: the code, and whether the control plane may try the step again.
fn failed(code: DatabaseFailure) -> Outcome {
    Outcome::Report(Report::Fail {
        code: code.code(),
        retryable: code.retryable(),
    })
}

/// What the driver decided, as what to send.
fn outcome_of(finished: Finished) -> Outcome {
    match finished.report {
        Finish::Complete(receipt) => match serde_json::to_vec(&*receipt) {
            Ok(bytes) => Outcome::Report(Report::Complete { bytes, blob: None }),
            // a receipt that cannot be written out is a host that did not keep the protocol
            Err(_) => failed(DatabaseFailure::LakeHostProtocol),
        },
        // the result of a probe, written again from its typed fields: the bytes the server reads and stores
        Finish::Probe(result) => Outcome::Report(Report::Complete {
            bytes: result.output_json(),
            blob: None,
        }),
        Finish::Fail(failure) => failed(failure.code),
        Finish::Abandon => Outcome::Nothing,
    }
}

/// The lease is for a probe of a connection and not a write (D035-150).
fn is_probe(lease: &pb::TaskLease) -> bool {
    lease.activity_name == ICEBERG_PROBE_ACTIVITY
}

/// The lease is one this profile serves: a root, forward task of a lake activity (a write, or a probe of a connection).
fn serves(lease: &pb::TaskLease) -> bool {
    (lease.activity_name == ICEBERG_WRITE_ACTIVITY || is_probe(lease))
        && lease
            .instance
            .as_ref()
            .is_none_or(|instance| instance.instance_path.is_empty())
        && matches!(lease.task_kind.as_str(), "" | "FORWARD")
}

/// Starts one lake lease. **The attempt is admitted here, when the lease is spawned, and not later inside the task**: a slot is taken
/// before the serve loop looks at its free slots again, so that what it counts is exact. (Taking it inside the task left a window in
/// which the loop spawned more attempts than there were free slots whenever a slot was held back for a container that might still be
/// running.) The returned future holds the slot until the lease is over.
pub(crate) fn launch<S, B, R, C>(
    rpc: Rpc,
    task: Task,
    driver: Driver<S, B, R, C>,
    drain: CancellationToken,
) -> impl Future<Output = Result<Disposition, RunnerError>> + Send
where
    S: LakeSandbox,
    B: Blobs,
    R: Resolver + Clone,
    C: Connector + Clone,
{
    let admission = serves(&task.lease).then(|| {
        let input = task.lease.input_json.as_bytes();
        if is_probe(&task.lease) {
            driver.admit_probe(input)
        } else {
            driver.admit(input)
        }
    });
    run(rpc, task, driver, admission, drain)
}

/// Runs one lake lease to its report. `drain` is the runner's: once it is cancelled the lease is given up as lost, never cancelled,
/// so a write that is under way is not reported as unknown by a rollout (the control plane's lease expiry decides what happens to it).
///
/// # Errors
/// [`RunnerError::Capability`] for a lease this profile does not serve, and [`RunnerError::Transport`] when the report did not
/// reach the control plane.
async fn run<S, B, R, C>(
    rpc: Rpc,
    task: Task,
    driver: Driver<S, B, R, C>,
    admission: Option<Result<Admitted, Finished>>,
    drain: CancellationToken,
) -> Result<Disposition, RunnerError>
where
    S: LakeSandbox,
    B: Blobs,
    R: Resolver + Clone,
    C: Connector + Clone,
{
    let lease = &task.lease;
    let initial = capability::deadline(lease.expires_at_epoch_ms).ok_or(RunnerError::Capability)?;
    // a lease this profile does not serve gave up its slot with the admission that was never made
    let admission = admission.ok_or(RunnerError::Capability)?;
    let signals = Arc::new(Signals {
        cancelled: CancellationToken::new(),
        lease_lost: drain.child_token(),
    });
    let heartbeat = Heartbeat::start(
        rpc.clone(),
        lease,
        task.worker_id.clone(),
        initial,
        Policy::Lake,
        {
            let signals = Arc::clone(&signals);
            move |end| match end {
                LeaseEnd::CancelRequested => signals.cancelled.cancel(),
                LeaseEnd::Lost => signals.lease_lost.cancel(),
            }
        },
    );
    let (outcome, written) = execute(&rpc, &task, &driver, admission, &signals).await;
    let (result, sent) = match outcome {
        Outcome::Nothing => (Ok(Disposition::Abandoned), Sent::Nothing),
        Outcome::Report(report) => {
            let (result, sent) = send(&rpc, lease, report, &heartbeat).await;
            (result.map(|()| Disposition::Reported), sent)
        }
    };
    record_written(written, sent);
    record_probe_report(is_probe(lease), sent);
    heartbeat.finish().await;
    result
}

/// A completion of a probe that the control plane refused as not the step's is failed as a protocol failure, which a write counts
/// as a failed write ([`record_written`]); a probe has no write to count it as, so it is counted on its own, and a runner whose
/// results are refused does not look like one whose probes all work.
fn record_probe_report(probe: bool, sent: Sent) {
    if probe && sent == Sent::Downgraded {
        metrics::counter!("database_runner_lake_probe_reports_total", "outcome" => "refused")
            .increment(1);
    }
}

/// What an attempt that reached a container says to the metrics about its write, once the report is settled.
type Written = Option<(Labels, LakeWriteOutcome)>;

/// The write is counted when it is settled, not when the host answered: a receipt the control plane refused is a write that failed.
fn record_written(written: Written, sent: Sent) {
    if let Some((labels, outcome)) = written {
        let outcome = if sent == Sent::Downgraded {
            LakeWriteOutcome::Failed
        } else {
            outcome
        };
        // append is the only mode this delivery writes
        record_write(LakeMode::Append, labels.catalog, labels.storage, outcome);
    }
}

async fn execute<S, B, R, C>(
    rpc: &Rpc,
    task: &Task,
    driver: &Driver<S, B, R, C>,
    admission: Result<Admitted, Finished>,
    signals: &Signals,
) -> (Outcome, Written)
where
    S: LakeSandbox,
    B: Blobs,
    R: Resolver + Clone,
    C: Connector + Clone,
{
    // before the connection is asked for: a full node or an input that is not one costs no disclosure
    let admitted = match admission {
        Ok(admitted) => admitted,
        Err(finished) => return (outcome_of(finished), None),
    };
    // what a probe was leased for: the connection that is resolved for it has to be that one
    let leased = admitted.leased_probe();
    let connection = tokio::select! {
        biased;
        () = signals.lease_lost.cancelled() => return (Outcome::Nothing, None),
        () = signals.cancelled.cancelled() => return (failed(DatabaseFailure::Cancelled), None),
        connection = resolve(rpc, task, leased.as_ref().map(|(name, revision)| (name.as_str(), *revision))) => connection,
    };
    let connection = match connection {
        Ok(connection) => connection,
        // the slot goes back with the admission
        Err(Unresolved::Failed(code)) => return (failed(code), None),
        Err(Unresolved::Transient) => return (Outcome::Nothing, None),
    };
    // the labels of a write come from its connection; whether the attempt was a write is for `write_outcome` to say, and a
    // probe, whatever its end, is not
    let labels = Labels::of(&connection);
    let finished = driver
        .run_admitted(admitted, connection, signals, || {})
        .await;
    record_container(&finished);
    let written = write_outcome(&finished).map(|outcome| (labels, outcome));
    (outcome_of(finished), written)
}

/// The labels of the metrics of a write that come from its connection: closed sets, never a connection, a table or a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Labels {
    catalog: LakeCatalogType,
    storage: LakeStorageType,
}

impl Labels {
    fn of(connection: &super::connection::Converted) -> Self {
        let catalog = match connection.catalog["type"].as_str() {
            Some("glue") => LakeCatalogType::Glue,
            Some("hive") => LakeCatalogType::Hive,
            Some("jdbc") => LakeCatalogType::Jdbc,
            _ => LakeCatalogType::Rest,
        };
        let storage = match connection.storage["type"].as_str() {
            Some("gcs") => LakeStorageType::Gcs,
            Some("adls") => LakeStorageType::Adls,
            _ => LakeStorageType::S3,
        };
        Self { catalog, storage }
    }
}

/// What the container's attempt tells the metrics at once: how it ended, what the relay refused, and whether its directory had to be
/// kept for the sweep. Nothing but the closed vocabulary of the telemetry crate, and a count, can be said here.
fn record_container(finished: &Finished) {
    if let Some(exit) = finished.exit {
        record_sandbox_exit(exit);
    }
    let denials = finished.relay_denials;
    for (reason, count) in [
        (
            LakeRelayDenial::EndpointNotDeclared,
            denials.endpoint_not_declared,
        ),
        (LakeRelayDenial::AddressForbidden, denials.address_forbidden),
        (LakeRelayDenial::ResolutionFailed, denials.resolution_failed),
        (LakeRelayDenial::TokenRefused, denials.token_refused),
        (
            LakeRelayDenial::RequestUnsupported,
            denials.request_unsupported,
        ),
    ] {
        for _ in 0..count {
            record_relay_denial(reason);
        }
    }
    // a directory that could not be removed, or whose container was not confirmed gone, is left to the sweep: that is worth seeing
    if finished.left.is_some() {
        metrics::counter!("database_runner_lake_directories_total", "outcome" => "kept")
            .increment(1);
    }
}

/// How the write ended, as the metrics say it: `None` for an attempt that never reached a container, which is not a write, and for a
/// lease that is gone, which has no outcome.
fn write_outcome(finished: &Finished) -> Option<LakeWriteOutcome> {
    // a probe changes no table: it is not a write, however it ended
    if finished.cause.is_some() || finished.probe {
        return None;
    }
    Some(match &finished.report {
        Finish::Complete(receipt) => match &**receipt {
            Receipt::Iceberg(committed) => match committed.outcome {
                IcebergOutcome::Committed => LakeWriteOutcome::Committed,
                IcebergOutcome::EmptyNoCommit => LakeWriteOutcome::EmptyNoCommit,
                IcebergOutcome::Reverted => LakeWriteOutcome::Reverted,
            },
            Receipt::ObjectStorage(_) => LakeWriteOutcome::Failed,
        },
        Finish::Fail(failure) if failure.code == DatabaseFailure::OutcomeUnknown => {
            LakeWriteOutcome::Unknown
        }
        Finish::Fail(_) => LakeWriteOutcome::Failed,
        // (a probe was left above)
        Finish::Probe(_) | Finish::Abandon => return None,
    })
}

/// Why the connection of a lease could not be had.
enum Unresolved {
    /// The control plane said no, or the answer is not one the runner reads: the step fails with this code.
    Failed(DatabaseFailure),
    /// The control plane could not answer now (unavailable, slow, rate limited, a conflict, an error of its own). Nothing was written and
    /// no secret was disclosed, so the step is not failed: the lease is given up and expires, and the step is leased again. Failing it
    /// would spend an attempt and, for a code that is not retryable, the step itself, on a blip.
    Transient,
}

/// Asks the control plane for the connection of the lease, and reads it. A probe says which connection and which revision it was
/// leased for (`probe`), and the answer has to be that one.
async fn resolve(
    rpc: &Rpc,
    task: &Task,
    probe: Option<(&str, u32)>,
) -> Result<super::connection::Converted, Unresolved> {
    let request = rpc
        .request(crate::rpc::task(&task.lease))
        .await
        .map_err(|_| Unresolved::Failed(DatabaseFailure::Credential))?;
    let response = rpc
        .client
        .clone()
        .resolve_database(request)
        .await
        .map_err(|status| match status.code() {
            tonic::Code::Unavailable
            | tonic::Code::DeadlineExceeded
            | tonic::Code::ResourceExhausted
            | tonic::Code::Aborted
            | tonic::Code::Internal => Unresolved::Transient,
            _ => Unresolved::Failed(DatabaseFailure::Capability),
        })?
        .into_inner();
    // the tenant is known from here: it belongs in the span, with the identifiers the poll put there, and in no metric
    if let Ok(tenant) = response
        .tenant_id
        .parse::<sagat_domain::identity::TenantId>()
    {
        tracing::Span::current().record("tenant_id", tracing::field::display(tenant));
    }
    match probe {
        Some((name, revision)) => decode_for_probe(response, task.namespace, name, revision),
        None => decode(response, task.namespace),
    }
    .map_err(Unresolved::Failed)
}

/// What became of the report, for the metrics of the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sent {
    /// There was nothing to send.
    Nothing,
    /// Sent as it was, or not delivered at all.
    AsIs,
    /// A completion the control plane refused was sent as a failure instead.
    Downgraded,
}

/// Sends the report. A completion the control plane refuses as invalid is the host's breach of the protocol: the step is failed
/// with that code instead, which is retryable (the next attempt of a write looks the effect up by its key; a probe has no effect,
/// and is simply run again).
async fn send(
    rpc: &Rpc,
    lease: &pb::TaskLease,
    report: Report,
    heartbeat: &Heartbeat,
) -> (Result<(), RunnerError>, Sent) {
    match report_until_ack(rpc, lease, "lake", &report, heartbeat).await {
        Ok(()) => (Ok(()), Sent::AsIs),
        Err(Unreported::Refused(tonic::Code::InvalidArgument))
            if matches!(report, Report::Complete { .. }) =>
        {
            let protocol = DatabaseFailure::LakeHostProtocol;
            let report = Report::Fail {
                code: protocol.code(),
                retryable: protocol.retryable(),
            };
            let result = report_until_ack(rpc, lease, "lake", &report, heartbeat)
                .await
                .map_err(|_| RunnerError::Transport);
            (result, Sent::Downgraded)
        }
        Err(_) => (Err(RunnerError::Transport), Sent::AsIs),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::Duration;

    use sagat_domain::database::DatabaseFailure;
    use sagat_infra_lake::protocol::Receipt;
    use sagat_telemetry::lake::LakeSandboxExit;
    use serde_json::Value;
    use tonic::Status;

    use super::super::doubles::{
        NoBlobs, NoDns, NoNetwork, Script, Scripted, SharedSandbox, World, answer,
        plain_failure_line, receipt_line, world,
    };
    use super::super::fixtures::{input_json, my_uid, resolved_database, settings};
    use super::*;
    use crate::{
        config::Config,
        testing::{Fake, Home, NAMESPACE_ID, Server, epoch_ms_in, home, serve_fake},
    };

    /// Everything one lease needs: the service it talks to, the runner's side of the transport, and a driver whose sandbox is scripted.
    struct Setup {
        fake: Fake,
        rpc: Rpc,
        world: World,
        _server: Server,
        _home: Home,
    }

    async fn setup() -> Setup {
        let fake = Fake::default();
        fake.with(|script| script.database = Some(resolved_database(NAMESPACE_ID)));
        let server = serve_fake(fake.clone()).await;
        let home = home(&server, None);
        let config = Config::read(&home.config).await.unwrap();
        Setup {
            fake,
            rpc: Rpc::connect(&config).await.unwrap(),
            world: world(),
            _server: server,
            _home: home,
        }
    }

    impl Setup {
        fn driver(&self, script: Script) -> Driver<Scripted, NoBlobs, NoDns, NoNetwork> {
            Driver::new(
                Scripted::new(self.world.work.path(), script),
                NoBlobs,
                NoDns,
                NoNetwork,
                settings(self.world.work.path()),
                false,
            )
            .for_tests(|_| Some(my_uid()))
        }

        async fn run<S: LakeSandbox>(
            &self,
            lease: pb::TaskLease,
            driver: Driver<S, NoBlobs, NoDns, NoNetwork>,
        ) -> Result<Disposition, RunnerError> {
            let task = Task {
                lease,
                namespace: NAMESPACE_ID.parse().unwrap(),
                worker_id: uuid::Uuid::from_u128(7).to_string(),
            };
            launch(self.rpc.clone(), task, driver, CancellationToken::new()).await
        }
    }

    fn lease(lasts: Duration) -> pb::TaskLease {
        pb::TaskLease {
            task_id: "task-1".into(),
            run_id: uuid::Uuid::from_u128(9).to_string(),
            step_id: uuid::Uuid::from_u128(10).to_string(),
            attempt: 1,
            generation: 3,
            lease_token: "lease-token".into(),
            expires_at_epoch_ms: epoch_ms_in(lasts),
            activity_name: ICEBERG_WRITE_ACTIVITY.into(),
            input_json: String::from_utf8(input_json(|_| {})).unwrap(),
            ..Default::default()
        }
    }

    fn receipt() -> Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/contract/iceberg-writes/fixtures/receipts/iceberg-committed.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn a_lease_the_host_commits_is_completed_with_its_receipt_and_nothing_else() {
        let setup = setup().await;
        let driver = setup.driver(answer(receipt_line()));
        setup
            .run(lease(Duration::from_secs(60)), driver)
            .await
            .expect("reported");
        setup.fake.with(|script| {
            assert_eq!(
                script.resolved.len(),
                1,
                "the connection was asked for once"
            );
            assert_eq!(script.completed.len(), 1);
            assert!(script.failed.is_empty());
            let sent = &script.completed[0];
            assert_eq!(sent.idempotency_key, "lake-complete-task-1-3");
            let output: Value = serde_json::from_str(&sent.output_json).unwrap();
            assert_eq!(output, receipt(), "the receipt, as the host's typed fields");
            assert!(sent.output_blob_ref.is_empty());
            // the secrets of the connection are in no request the control plane was sent
            let seen = format!(
                "{:?}{:?}{:?}{:?}",
                script.registered, script.resolved, script.completed, script.heartbeats
            );
            assert!(!seen.contains("canary"), "{seen}");
        });
    }

    #[tokio::test]
    async fn a_failure_of_the_host_is_reported_with_its_code_and_the_flag_of_the_contract() {
        for (code, failure, retryable) in [
            (
                "ICEBERG_COMMIT_CONFLICT",
                DatabaseFailure::IcebergCommitConflict,
                true,
            ),
            (
                "ICEBERG_SCHEMA_MISMATCH",
                DatabaseFailure::IcebergSchemaMismatch,
                false,
            ),
        ] {
            let setup = setup().await;
            let driver = setup.driver(answer(plain_failure_line(code)));
            setup
                .run(lease(Duration::from_secs(60)), driver)
                .await
                .expect("reported");
            setup.fake.with(|script| {
                assert!(script.completed.is_empty());
                let sent = &script.failed[0];
                assert_eq!(sent.error_code, failure.code());
                assert_eq!(sent.retryable, retryable, "{code}");
                assert_eq!(sent.retryable, failure.retryable());
                assert_eq!(sent.idempotency_key, "lake-fail-task-1-3");
            });
        }
    }

    #[tokio::test]
    async fn a_lease_that_is_not_a_root_forward_task_of_the_lake_is_not_served() {
        for edit in [
            (|lease: &mut pb::TaskLease| lease.activity_name = "sagat.database.postgresql".into())
                as fn(&mut pb::TaskLease),
            |lease| lease.task_kind = "COMPENSATION".into(),
            |lease| {
                lease.instance = Some(pb::ExecutionInstanceIdentity {
                    instance_path: vec![pb::ScopePathSegment::default()],
                    ..Default::default()
                });
            },
            |lease| lease.expires_at_epoch_ms = 0,
        ] {
            let setup = setup().await;
            let mut other = lease(Duration::from_secs(60));
            edit(&mut other);
            let driver = setup.driver(answer(receipt_line()));
            assert_eq!(setup.run(other, driver).await, Err(RunnerError::Capability));
            setup.fake.with(|script| {
                assert!(script.resolved.is_empty() && script.completed.is_empty());
                assert!(script.failed.is_empty());
            });
        }
    }

    #[tokio::test]
    async fn the_slot_of_a_lease_is_taken_when_it_is_launched_and_not_when_the_task_first_runs() {
        let setup = setup().await;
        let driver = setup.driver(answer(receipt_line()));
        let slots = std::sync::Arc::clone(driver.slots());
        let task = Task {
            lease: lease(Duration::from_secs(60)),
            namespace: NAMESPACE_ID.parse().unwrap(),
            worker_id: uuid::Uuid::from_u128(7).to_string(),
        };
        // the future has not been polled: the runner counts its free slots right after it spawns, and the count must already be exact
        let launched = launch(setup.rpc.clone(), task, driver, CancellationToken::new());
        assert_eq!(slots.in_use(), 1);
        drop(launched);
        assert_eq!(
            slots.in_use(),
            0,
            "and a lease that is dropped gives it back"
        );
        // a lease the profile does not serve never takes one
        let driver = setup.driver(answer(receipt_line()));
        let slots = std::sync::Arc::clone(driver.slots());
        let mut other = lease(Duration::from_secs(60));
        other.activity_name = "sagat.database.postgresql".into();
        let task = Task {
            lease: other,
            namespace: NAMESPACE_ID.parse().unwrap(),
            worker_id: uuid::Uuid::from_u128(7).to_string(),
        };
        let launched = launch(setup.rpc.clone(), task, driver, CancellationToken::new());
        assert_eq!(slots.in_use(), 0);
        assert_eq!(launched.await, Err(RunnerError::Capability));
    }

    #[tokio::test]
    async fn a_full_node_is_told_and_the_connection_is_never_asked_for() {
        let setup = setup().await;
        let driver = setup.driver(answer(receipt_line()));
        let _all: Vec<_> = (0..8).map(|_| driver.slots().take().unwrap()).collect();
        setup
            .run(lease(Duration::from_secs(60)), driver)
            .await
            .expect("reported");
        setup.fake.with(|script| {
            assert!(
                script.resolved.is_empty(),
                "asking for the connection discloses the secrets: not for an attempt that cannot run"
            );
            let sent = &script.failed[0];
            assert_eq!(sent.error_code, "LAKE_SANDBOX_UNAVAILABLE");
            assert!(sent.retryable);
        });
    }

    #[tokio::test]
    async fn an_answer_that_is_not_for_this_runner_starts_no_container() {
        let setup = setup().await;
        setup.fake.with(|script| {
            script.database = Some(resolved_database("00000000-0000-0000-0000-000000000002"));
        });
        let driver = setup.driver(answer(receipt_line()));
        let slots = std::sync::Arc::clone(driver.slots());
        let sandbox = std::sync::Arc::new(Scripted::new(
            setup.world.work.path(),
            answer(receipt_line()),
        ));
        let driver = Driver::with_slots(
            SharedSandbox(std::sync::Arc::clone(&sandbox)),
            NoBlobs,
            NoDns,
            NoNetwork,
            settings(setup.world.work.path()),
            false,
            slots.clone(),
        )
        .for_tests(|_| Some(my_uid()));
        setup
            .run(lease(Duration::from_secs(60)), driver)
            .await
            .expect("reported");
        setup.fake.with(|script| {
            assert_eq!(
                script.failed[0].error_code,
                "DATABASE_CAPABILITY_UNAVAILABLE"
            );
        });
        assert!(sandbox.asked().is_empty(), "no container was asked for");
        assert_eq!(slots.in_use(), 0, "and the slot of the lease is back");
    }

    #[tokio::test]
    async fn a_connection_the_control_plane_does_not_resolve_fails_the_step_with_the_capability_code()
     {
        let setup = setup().await;
        setup.fake.with(|script| script.database = None);
        let driver = setup.driver(answer(receipt_line()));
        let slots = std::sync::Arc::clone(driver.slots());
        setup
            .run(lease(Duration::from_secs(60)), driver)
            .await
            .expect("reported");
        setup.fake.with(|script| {
            assert_eq!(
                script.failed[0].error_code,
                "DATABASE_CAPABILITY_UNAVAILABLE"
            );
            assert!(!script.failed[0].retryable);
        });
        assert_eq!(slots.in_use(), 0, "and the slot is back");
    }

    #[tokio::test]
    async fn a_control_plane_that_cannot_answer_for_a_moment_does_not_fail_a_step_that_wrote_nothing()
     {
        for code in [
            tonic::Code::Unavailable,
            tonic::Code::DeadlineExceeded,
            tonic::Code::ResourceExhausted,
            tonic::Code::Aborted,
            tonic::Code::Internal,
        ] {
            let setup = setup().await;
            setup
                .fake
                .with(|script| script.database_error = Some(Status::new(code, "not now")));
            let driver = setup.driver(answer(receipt_line()));
            let slots = std::sync::Arc::clone(driver.slots());
            let ended = setup
                .run(lease(Duration::from_secs(60)), driver)
                .await
                .expect("nothing to report is not a failure");
            assert_eq!(ended, Disposition::Abandoned, "{code:?}");
            setup.fake.with(|script| {
                assert_eq!(script.resolved.len(), 1);
                assert!(
                    script.completed.is_empty() && script.failed.is_empty(),
                    "{code:?}: no write was started, and the lease expires on its own and is leased again"
                );
            });
            assert_eq!(slots.in_use(), 0);
        }
        // an answer that says no for good is a failure of the step
        for code in [tonic::Code::PermissionDenied, tonic::Code::NotFound] {
            let setup = setup().await;
            setup
                .fake
                .with(|script| script.database_error = Some(Status::new(code, "no")));
            let driver = setup.driver(answer(receipt_line()));
            setup
                .run(lease(Duration::from_secs(60)), driver)
                .await
                .expect("reported");
            setup.fake.with(|script| {
                assert_eq!(
                    script.failed[0].error_code, "DATABASE_CAPABILITY_UNAVAILABLE",
                    "{code:?}"
                );
            });
        }
    }

    #[tokio::test]
    async fn a_completion_the_control_plane_refuses_as_not_the_step_s_is_a_breach_of_the_protocol()
    {
        let setup = setup().await;
        setup.fake.with(|script| {
            script.report_errors = [Status::invalid_argument("not the step's")].into()
        });
        let driver = setup.driver(answer(receipt_line()));
        setup
            .run(lease(Duration::from_secs(60)), driver)
            .await
            .expect("reported");
        setup.fake.with(|script| {
            assert_eq!(script.completed.len(), 1, "refused once, not retried");
            assert_eq!(script.failed[0].error_code, "LAKE_HOST_PROTOCOL");
            assert!(script.failed[0].retryable);
        });
    }

    #[tokio::test]
    async fn a_completion_refused_for_any_other_reason_is_not_turned_into_a_failure() {
        let setup = setup().await;
        setup.fake.with(|script| {
            script.report_errors = [Status::permission_denied("not yours")].into();
        });
        let driver = setup.driver(answer(receipt_line()));
        assert_eq!(
            setup.run(lease(Duration::from_secs(60)), driver).await,
            Err(RunnerError::Transport)
        );
        setup.fake.with(|script| {
            assert_eq!(script.completed.len(), 1, "refused once, not retried");
            assert!(
                script.failed.is_empty(),
                "and not reported as another ending"
            );
        });
    }

    #[tokio::test]
    async fn a_cancel_after_the_host_has_the_request_is_an_unknown_outcome_through_the_shell() {
        let setup = setup().await;
        let sandbox = std::sync::Arc::new(Scripted::new(setup.world.work.path(), Script::Silent));
        let driver = Driver::new(
            SharedSandbox(std::sync::Arc::clone(&sandbox)),
            NoBlobs,
            NoDns,
            NoNetwork,
            settings(setup.world.work.path()),
            false,
        )
        .for_tests(|_| Some(my_uid()));
        // the control plane asks for the cancel only once the host has been sent the write
        let requested = std::sync::Arc::clone(&sandbox.requested);
        let fake = setup.fake.clone();
        let asking = tokio::spawn(async move {
            requested.notified().await;
            fake.with(|script| {
                script.heartbeat = Some(Box::new(|_| {
                    Ok(pb::HeartbeatResponse {
                        lease_expires_at_epoch_ms: epoch_ms_in(Duration::from_secs(60)),
                        cancel_requested: true,
                    })
                }));
            });
        });
        let ended = setup
            .run(lease(Duration::from_secs(60)), driver)
            .await
            .expect("reported");
        asking.await.unwrap();
        assert_eq!(ended, Disposition::Reported);
        setup.fake.with(|script| {
            assert_eq!(script.failed[0].error_code, "OUTCOME_UNKNOWN");
            assert!(
                !script.failed[0].retryable,
                "an unknown outcome is never repeated blindly"
            );
            assert!(script.completed.is_empty());
        });
    }

    #[tokio::test]
    async fn a_lease_that_is_lost_reports_nothing() {
        let setup = setup().await;
        setup.fake.with(|script| {
            script.heartbeat = Some(Box::new(|_| Err(Status::permission_denied("not yours"))));
        });
        // the host never connects: the attempt is waiting when the heartbeat finds the lease gone
        let driver = setup.driver(Script::NeverConnects);
        let ended = setup
            .run(lease(Duration::from_millis(900)), driver)
            .await
            .expect("nothing to report is not a failure");
        assert_eq!(
            ended,
            Disposition::Abandoned,
            "and it is not counted as reported"
        );
        setup.fake.with(|script| {
            assert!(!script.heartbeats.is_empty());
            assert!(script.completed.is_empty() && script.failed.is_empty());
        });
    }

    #[tokio::test]
    async fn a_cancel_the_control_plane_asks_for_before_the_write_starts_is_reported_as_cancelled()
    {
        let setup = setup().await;
        setup.fake.with(|script| {
            script.heartbeat = Some(Box::new(|_| {
                Ok(pb::HeartbeatResponse {
                    lease_expires_at_epoch_ms: epoch_ms_in(Duration::from_secs(60)),
                    cancel_requested: true,
                })
            }));
        });
        let driver = setup.driver(Script::NeverConnects);
        setup
            .run(lease(Duration::from_millis(900)), driver)
            .await
            .expect("reported");
        setup.fake.with(|script| {
            assert_eq!(
                script.failed[0].error_code,
                DatabaseFailure::Cancelled.code()
            );
            assert!(script.completed.is_empty());
        });
    }

    // -- the metrics (D035-143) ----------------------------------------------------------------------------------------

    fn finished(report: Finish, exit: Option<LakeSandboxExit>) -> Finished {
        Finished {
            report,
            probe: false,
            cause: None,
            request_sent: true,
            container_ended: true,
            relay_denials: sagat_infra_lake::relay::RelayDenials::default(),
            sandbox_refusal: None,
            exit,
            left: None,
        }
    }

    fn labels() -> Labels {
        Labels {
            catalog: LakeCatalogType::Rest,
            storage: LakeStorageType::S3,
        }
    }

    /// What the attempt counts for what it finished and how its report was sent: the container at once, the write when it is settled.
    fn settle(finished: &Finished, sent: Sent) {
        record_container(finished);
        record_written(
            write_outcome(finished).map(|outcome| (labels(), outcome)),
            sent,
        );
    }

    /// What was counted, as `name{label=value,...}` and the amount, in order.
    fn seen(counted: Vec<crate::testing::Seen>) -> Vec<(String, u64)> {
        counted
            .into_iter()
            .map(|(name, labels, count)| {
                let labels: Vec<String> = labels
                    .into_iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect();
                (format!("{name}{{{}}}", labels.join(",")), count)
            })
            .collect()
    }

    fn committed() -> Finish {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/contract/iceberg-writes/fixtures/receipts/iceberg-committed.json");
        let receipt: Receipt =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        Finish::Complete(Box::new(receipt))
    }

    fn failure(code: DatabaseFailure) -> Finish {
        Finish::Fail(sagat_infra_lake::supervisor::Failure {
            code,
            column: None,
            reason: None,
        })
    }

    #[test]
    fn a_commit_is_counted_by_its_closed_labels_with_how_the_container_ended() {
        let counted = seen(crate::testing::counted(|| {
            settle(
                &finished(committed(), Some(LakeSandboxExit::Completed)),
                Sent::AsIs,
            );
        }));
        assert_eq!(
            counted,
            [
                ("lake_sandbox_exits_total{reason=completed}".to_owned(), 1),
                (
                    "lake_writes_total{mode=append,catalog_type=rest,storage_type=s3,outcome=committed}"
                        .to_owned(),
                    1
                ),
            ]
        );
    }

    #[test]
    fn a_completion_the_control_plane_refused_is_counted_as_the_failure_it_became() {
        let counted = seen(crate::testing::counted(|| {
            settle(
                &finished(committed(), Some(LakeSandboxExit::Completed)),
                Sent::Downgraded,
            );
        }));
        assert!(
            counted
                .iter()
                .any(|(name, _)| name.ends_with("outcome=failed}")),
            "{counted:?}"
        );
        assert!(!counted.iter().any(|(name, _)| name.contains("committed")));
    }

    #[test]
    fn a_directory_kept_for_the_sweep_is_counted() {
        let mut kept = finished(committed(), Some(LakeSandboxExit::Completed));
        kept.left = Some(std::path::PathBuf::from("/work/kept"));
        let counted = seen(crate::testing::counted(|| settle(&kept, Sent::AsIs)));
        assert!(
            counted.contains(&(
                "database_runner_lake_directories_total{outcome=kept}".to_owned(),
                1
            )),
            "{counted:?}"
        );
    }

    #[test]
    fn a_failure_and_an_unknown_outcome_are_not_the_same_series() {
        let outcome = |report: Finish| {
            seen(crate::testing::counted(|| {
                settle(&finished(report, None), Sent::AsIs);
            }))
        };
        assert!(
            outcome(failure(DatabaseFailure::IcebergSchemaMismatch))[0]
                .0
                .ends_with("outcome=failed}")
        );
        assert!(
            outcome(failure(DatabaseFailure::OutcomeUnknown))[0]
                .0
                .ends_with("outcome=unknown}")
        );
        assert!(
            outcome(Finish::Abandon).is_empty(),
            "a lease that is gone has no outcome to count"
        );
    }

    #[test]
    fn what_the_relay_refused_is_counted_by_reason_and_never_by_destination() {
        let mut ended = finished(
            failure(DatabaseFailure::LakeHostProtocol),
            Some(LakeSandboxExit::ProtocolError),
        );
        ended.relay_denials.address_forbidden = 2;
        ended.relay_denials.token_refused = 1;
        let counted = seen(crate::testing::counted(|| settle(&ended, Sent::AsIs)));
        let denials: Vec<_> = counted
            .iter()
            .filter(|(name, _)| name.starts_with("lake_relay_denials_total"))
            .map(|(name, count)| (name.as_str(), *count))
            .collect();
        assert_eq!(
            denials,
            [
                ("lake_relay_denials_total{reason=address_forbidden}", 1),
                ("lake_relay_denials_total{reason=address_forbidden}", 1),
                ("lake_relay_denials_total{reason=token_refused}", 1),
            ]
        );
    }

    #[test]
    fn an_attempt_that_never_reached_a_container_is_not_a_write() {
        let mut before = finished(failure(DatabaseFailure::LakeSandboxUnavailable), None);
        before.cause = Some(super::super::driver::Cause::NoSlot);
        assert!(seen(crate::testing::counted(|| settle(&before, Sent::AsIs))).is_empty());
    }

    // -- the probe (D035-150, slice S6b) ---------------------------------------------------------------------------------

    use super::super::doubles::{probe_line, probe_line_with};
    use super::super::fixtures::probe_input_json;

    fn probe_lease(lasts: Duration) -> pb::TaskLease {
        pb::TaskLease {
            activity_name: ICEBERG_PROBE_ACTIVITY.into(),
            input_json: String::from_utf8(probe_input_json(|_| {})).unwrap(),
            ..lease(lasts)
        }
    }

    /// The text of the result in the corpus, which is the canonical form the server stores.
    fn probe_result() -> String {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/contract/iceberg-writes/fixtures/probe-results/accepted/all-passed.json",
        );
        std::fs::read_to_string(path).unwrap().trim_end().to_owned()
    }

    #[tokio::test]
    async fn a_probe_lease_the_host_answers_is_completed_with_the_result_as_the_server_stores_it() {
        let setup = setup().await;
        let driver = setup.driver(answer(probe_line()));

        let disposition = setup
            .run(probe_lease(Duration::from_secs(60)), driver)
            .await
            .expect("reported");

        assert_eq!(disposition, Disposition::Reported);
        setup.fake.with(|script| {
            assert_eq!(
                script.resolved.len(),
                1,
                "the connection was asked for once"
            );
            assert_eq!(script.completed.len(), 1);
            assert!(script.failed.is_empty());
            let sent = &script.completed[0];
            assert_eq!(sent.idempotency_key, "lake-complete-task-1-3");
            // inline, typed fields written again: the bytes the server's codec reads and stores
            assert_eq!(sent.output_json, probe_result());
            assert!(sent.output_blob_ref.is_empty());
            let seen = format!(
                "{:?}{:?}{:?}{:?}",
                script.registered, script.resolved, script.completed, script.heartbeats
            );
            assert!(!seen.contains("canary"), "{seen}");
        });
    }

    #[tokio::test]
    async fn a_probe_lease_whose_host_cannot_run_it_is_failed_with_the_code_the_host_gave() {
        let setup = setup().await;
        let driver = setup.driver(answer(plain_failure_line(
            "DATABASE_CREDENTIAL_UNAVAILABLE",
        )));

        setup
            .run(probe_lease(Duration::from_secs(60)), driver)
            .await
            .expect("reported");

        setup.fake.with(|script| {
            assert!(script.completed.is_empty());
            let sent = &script.failed[0];
            assert_eq!(sent.error_code, "DATABASE_CREDENTIAL_UNAVAILABLE");
            assert_eq!(sent.retryable, DatabaseFailure::Credential.retryable());
            assert_eq!(sent.idempotency_key, "lake-fail-task-1-3");
        });
    }

    #[tokio::test]
    async fn a_probe_lease_answered_for_another_connection_is_failed_and_not_completed() {
        let setup = setup().await;
        let driver = setup.driver(answer(probe_line_with(|receipt| {
            receipt["connection"]["revision"] = serde_json::json!(4);
        })));

        setup
            .run(probe_lease(Duration::from_secs(60)), driver)
            .await
            .expect("reported");

        setup.fake.with(|script| {
            assert!(
                script.completed.is_empty(),
                "the server would refuse it: it is not this probe's"
            );
            assert_eq!(script.failed[0].error_code, "LAKE_HOST_PROTOCOL");
        });
    }

    #[tokio::test]
    async fn a_probe_lease_that_is_not_a_probe_input_is_failed_without_asking_for_the_connection() {
        let setup = setup().await;
        let driver = setup.driver(answer(probe_line()));
        let mut malformed = probe_lease(Duration::from_secs(60));
        malformed.input_json =
            String::from_utf8(probe_input_json(|d| d["table"] = serde_json::json!("x"))).unwrap();

        setup.run(malformed, driver).await.expect("reported");

        setup.fake.with(|script| {
            assert!(
                script.resolved.is_empty(),
                "asking for the connection discloses the secrets: not for a probe that cannot run"
            );
            assert_eq!(
                script.failed[0].error_code,
                "DATABASE_CONFIGURATION_INVALID"
            );
        });
    }

    #[tokio::test]
    async fn a_probe_lease_is_served_only_as_a_root_forward_task_like_a_write() {
        for edit in [
            (|lease: &mut pb::TaskLease| lease.task_kind = "COMPENSATION".into())
                as fn(&mut pb::TaskLease),
            |lease| {
                lease.instance = Some(pb::ExecutionInstanceIdentity {
                    instance_path: vec![pb::ScopePathSegment::default()],
                    ..Default::default()
                });
            },
            |lease| lease.expires_at_epoch_ms = 0,
        ] {
            let setup = setup().await;
            let mut other = probe_lease(Duration::from_secs(60));
            edit(&mut other);
            let driver = setup.driver(answer(probe_line()));
            assert_eq!(setup.run(other, driver).await, Err(RunnerError::Capability));
            setup.fake.with(|script| {
                assert!(script.resolved.is_empty() && script.completed.is_empty());
                assert!(script.failed.is_empty());
            });
        }
    }

    /// What `execute` decides for a lease, as the shell calls it: the report, and what is counted for the write.
    async fn executed<S: LakeSandbox>(
        setup: &Setup,
        driver: &Driver<S, NoBlobs, NoDns, NoNetwork>,
        lease: pb::TaskLease,
        probe: bool,
    ) -> (Outcome, Written) {
        let input = lease.input_json.clone();
        let admission = if probe {
            driver.admit_probe(input.as_bytes())
        } else {
            driver.admit(input.as_bytes())
        };
        let task = Task {
            lease,
            namespace: NAMESPACE_ID.parse().unwrap(),
            worker_id: uuid::Uuid::from_u128(7).to_string(),
        };
        let signals = super::super::doubles::signals();
        execute(&setup.rpc, &task, driver, admission, &signals).await
    }

    #[tokio::test]
    async fn a_probe_counts_no_write_whether_it_works_or_not_and_a_write_still_does() {
        let setup = setup().await;
        for script in [
            answer(probe_line()),
            answer(plain_failure_line("DATABASE_CREDENTIAL_UNAVAILABLE")),
        ] {
            let driver = setup.driver(script);
            let (_, written) =
                executed(&setup, &driver, probe_lease(Duration::from_secs(60)), true).await;
            assert!(written.is_none(), "a probe changes no table");
        }
        // and the same path counts a write that committed
        let driver = setup.driver(answer(receipt_line()));
        let (_, written) = executed(&setup, &driver, lease(Duration::from_secs(60)), false).await;
        assert!(written.is_some());
    }

    #[tokio::test]
    async fn a_probe_whose_connection_was_resolved_at_another_revision_is_failed_and_starts_nothing()
     {
        // the control plane answers for revision 4 a probe that was leased for revision 3: what would be tried is not what
        // was leased, and the report would say that revision 3 works
        let setup = setup().await;
        setup
            .fake
            .with(|script| script.database.as_mut().unwrap().connection_revision = 4);
        // the sandbox is shared, so that what it was asked can be read after the lease
        let sandbox = Arc::new(Scripted::new(setup.world.work.path(), answer(probe_line())));
        let driver = Driver::new(
            SharedSandbox(Arc::clone(&sandbox)),
            NoBlobs,
            NoDns,
            NoNetwork,
            settings(setup.world.work.path()),
            false,
        )
        .for_tests(|_| Some(my_uid()));

        let (outcome, written) =
            executed(&setup, &driver, probe_lease(Duration::from_secs(60)), true).await;

        let Outcome::Report(Report::Fail { code, .. }) = outcome else {
            panic!("not a failure");
        };
        assert_eq!(code, DatabaseFailure::Capability.code());
        assert!(written.is_none());
        assert!(sandbox.asked().is_empty(), "no container was started");
        assert_eq!(driver.slots().in_use(), 0, "and the slot is back");
    }

    #[test]
    fn a_probe_completion_that_the_control_plane_refuses_is_counted_apart_from_the_writes() {
        // the downgrade of a refused completion is counted as a failed write for a write; a probe has no write to count it
        // as, and a runner whose results the server refuses must not look like one whose probes are all fine
        let counted =
            |probe, sent| seen(crate::testing::counted(|| record_probe_report(probe, sent)));

        assert_eq!(
            counted(true, Sent::Downgraded),
            [(
                "database_runner_lake_probe_reports_total{outcome=refused}".to_owned(),
                1
            )]
        );
        assert!(counted(true, Sent::AsIs).is_empty());
        assert!(counted(true, Sent::Nothing).is_empty());
        assert!(
            counted(false, Sent::Downgraded).is_empty(),
            "a write is counted as a write"
        );
    }

    /// What the host's line says, read by the decoder the conversation uses.
    fn probe_report() -> sagat_infra_lake::protocol::ProbeReport {
        use sagat_infra_lake::protocol::{Decoder, Event, Outcome};
        let line = probe_line();
        let event = Decoder::new()
            .line(&line[..line.len() - 1], Duration::ZERO)
            .unwrap();
        match event {
            Event::Result(result) => match result.outcome {
                Outcome::Probe(report) => report,
                other => panic!("not a probe report: {other:?}"),
            },
            Event::Heartbeat => panic!("a heartbeat"),
        }
    }

    fn probed(report: Finish, exit: Option<LakeSandboxExit>) -> Finished {
        Finished {
            probe: true,
            ..finished(report, exit)
        }
    }

    #[test]
    fn a_probe_is_counted_for_its_container_and_never_as_a_write() {
        // whatever the probe did, and whether it failed: a write is what changes a table, and a probe changes none
        let reports = [
            Finish::Probe(Box::new(probe_report())),
            failure(DatabaseFailure::Credential),
            failure(DatabaseFailure::Timeout),
            failure(DatabaseFailure::LakeHostProtocol),
        ];
        for report in reports {
            let counted = seen(crate::testing::counted(|| {
                settle(
                    &probed(report.clone(), Some(LakeSandboxExit::Completed)),
                    Sent::AsIs,
                );
            }));
            assert_eq!(
                counted,
                [("lake_sandbox_exits_total{reason=completed}".to_owned(), 1)],
                "{report:?}"
            );
        }
    }

    #[test]
    fn the_labels_of_a_write_come_from_the_kind_of_its_connection() {
        let mut connection = super::super::fixtures::converted();
        assert_eq!(
            Labels::of(&connection),
            Labels {
                catalog: LakeCatalogType::Rest,
                storage: LakeStorageType::S3
            }
        );
        connection.catalog = serde_json::json!({"type": "glue"});
        connection.storage = serde_json::json!({"type": "gcs"});
        assert_eq!(
            Labels::of(&connection),
            Labels {
                catalog: LakeCatalogType::Glue,
                storage: LakeStorageType::Gcs
            }
        );
    }
}

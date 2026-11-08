//! One lake lease, from the task the runner polled to the report it sends (feature 035, T035-058, D035-143).
//!
//! The driver ([`Driver`]) knows how to run an attempt once it has its input, a slot and a connection. This is the part that talks
//! to the control plane around it:
//!
//! - *the lease is checked* before anything is asked for: a root forward task of the lake activity, with an expiry;
//! - *the heartbeats* renew it, and what their end means is decided here: a cancel the control plane asked for is `cancelled`
//!   (a write that was started is then `OUTCOME_UNKNOWN`), a lease that is lost is `lease_lost` (there is no one to report to);
//! - *the attempt is admitted* (an input it can read, a free slot) **before the connection is asked for**, because asking for it
//!   discloses and audits the secrets;
//! - *the connection is resolved* under the same signals, and read by [`decode`](super::resolve::decode), which zeroises what it
//!   was given on every path;
//! - *the report* is the driver's, sent with the flag the contract gives its code: `CompleteStep` with the receipt, `FailStep`
//!   with a closed code, or nothing at all when the lease is gone. A completion the control plane refuses as not the step's
//!   becomes `LAKE_HOST_PROTOCOL`: the host did not keep the protocol.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the serve loop of the lake profile calls it (G5)")
)]

use std::sync::Arc;

use sagat_api_grpc::pb;
use sagat_domain::{
    database::{DatabaseFailure, ICEBERG_WRITE_ACTIVITY},
    identity::NamespaceId,
};
use sagat_infra_lake::attempt::{LakeSandbox, Signals};
use sagat_infra_lake::relay::{Connector, Resolver};
use tokio_util::sync::CancellationToken;

use super::driver::{Driver, Finished, Report as Finish};
use super::prepare::Blobs;
use super::resolve::decode;
use crate::{
    RunnerError, capability,
    lease::{Heartbeat, LeaseEnd, Report, Unreported, report_until_ack},
    rpc::Rpc,
};

pub(crate) struct Task {
    pub lease: pb::TaskLease,
    pub namespace: NamespaceId,
    pub worker_id: String,
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
        Finish::Fail(failure) => failed(failure.code),
        Finish::Abandon => Outcome::Nothing,
    }
}

/// The lease is one this profile serves: a root, forward task of the lake activity.
fn serves(lease: &pb::TaskLease) -> bool {
    lease.activity_name == ICEBERG_WRITE_ACTIVITY
        && lease
            .instance
            .as_ref()
            .is_none_or(|instance| instance.instance_path.is_empty())
        && matches!(lease.task_kind.as_str(), "" | "FORWARD")
}

/// Runs one lake lease to its report. `drain` is the runner's: once it is cancelled the lease is given up as lost, never cancelled,
/// so a write that is under way is not reported as unknown by a rollout (the control plane's lease expiry decides what happens to it).
///
/// # Errors
/// [`RunnerError::Capability`] for a lease this profile does not serve, and [`RunnerError::Transport`] when the report did not
/// reach the control plane.
pub(crate) async fn run<S, B, R, C>(
    rpc: Rpc,
    task: Task,
    driver: Driver<S, B, R, C>,
    drain: CancellationToken,
) -> Result<(), RunnerError>
where
    S: LakeSandbox,
    B: Blobs,
    R: Resolver + Clone,
    C: Connector + Clone,
{
    let lease = &task.lease;
    let initial = capability::deadline(lease.expires_at_epoch_ms).ok_or(RunnerError::Capability)?;
    if !serves(lease) {
        return Err(RunnerError::Capability);
    }
    let signals = Arc::new(Signals {
        cancelled: CancellationToken::new(),
        lease_lost: drain.child_token(),
    });
    let heartbeat = Heartbeat::start(rpc.clone(), lease, task.worker_id.clone(), initial, {
        let signals = Arc::clone(&signals);
        move |end| match end {
            LeaseEnd::CancelRequested => signals.cancelled.cancel(),
            LeaseEnd::Lost => signals.lease_lost.cancel(),
        }
    });
    let outcome = execute(&rpc, &task, &driver, &signals).await;
    let result = match outcome {
        Outcome::Nothing => Ok(()),
        Outcome::Report(report) => send(&rpc, lease, report, &heartbeat).await,
    };
    heartbeat.finish().await;
    result
}

async fn execute<S, B, R, C>(
    rpc: &Rpc,
    task: &Task,
    driver: &Driver<S, B, R, C>,
    signals: &Signals,
) -> Outcome
where
    S: LakeSandbox,
    B: Blobs,
    R: Resolver + Clone,
    C: Connector + Clone,
{
    let lease = &task.lease;
    // before the connection is asked for: a full node or an input that is not one costs no disclosure
    let admitted = match driver.admit(lease.input_json.as_bytes()) {
        Ok(admitted) => admitted,
        Err(finished) => return outcome_of(finished),
    };
    let connection = tokio::select! {
        biased;
        () = signals.lease_lost.cancelled() => return Outcome::Nothing,
        () = signals.cancelled.cancelled() => return failed(DatabaseFailure::Cancelled),
        connection = resolve(rpc, task) => connection,
    };
    let connection = match connection {
        Ok(connection) => connection,
        // the slot goes back with the admission
        Err(code) => return failed(code),
    };
    outcome_of(
        driver
            .run_admitted(admitted, connection, signals, || {})
            .await,
    )
}

/// Asks the control plane for the connection of the lease, and reads it.
async fn resolve(rpc: &Rpc, task: &Task) -> Result<super::connection::Converted, DatabaseFailure> {
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
            tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => DatabaseFailure::Credential,
            _ => DatabaseFailure::Capability,
        })?
        .into_inner();
    decode(response, task.namespace)
}

/// Sends the report. A completion the control plane refuses as invalid is the host's breach of the protocol: the step is failed
/// with that code instead, which is retryable (the next attempt looks the effect up by its key).
async fn send(
    rpc: &Rpc,
    lease: &pb::TaskLease,
    report: Report,
    heartbeat: &Heartbeat,
) -> Result<(), RunnerError> {
    match report_until_ack(rpc, lease, "lake", &report, heartbeat).await {
        Ok(()) => Ok(()),
        Err(Unreported::Refused(tonic::Code::InvalidArgument))
            if matches!(report, Report::Complete { .. }) =>
        {
            let protocol = DatabaseFailure::LakeHostProtocol;
            let report = Report::Fail {
                code: protocol.code(),
                retryable: protocol.retryable(),
            };
            report_until_ack(rpc, lease, "lake", &report, heartbeat)
                .await
                .map_err(|_| RunnerError::Transport)
        }
        Err(_) => Err(RunnerError::Transport),
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::Duration;

    use sagat_domain::database::DatabaseFailure;
    use serde_json::Value;
    use tonic::Status;

    use super::super::doubles::{
        NoBlobs, NoDns, NoNetwork, Script, Scripted, World, answer, plain_failure_line,
        receipt_line, world,
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

        async fn run(
            &self,
            lease: pb::TaskLease,
            driver: Driver<Scripted, NoBlobs, NoDns, NoNetwork>,
        ) -> Result<(), RunnerError> {
            let task = Task {
                lease,
                namespace: NAMESPACE_ID.parse().unwrap(),
                worker_id: uuid::Uuid::from_u128(7).to_string(),
            };
            run(self.rpc.clone(), task, driver, CancellationToken::new()).await
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
        let nothing = setup.driver(answer(receipt_line()));
        assert!(nothing.slots().in_use() == 0);
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
    async fn a_lease_that_is_lost_reports_nothing() {
        let setup = setup().await;
        setup.fake.with(|script| {
            script.heartbeat = Some(Box::new(|_| Err(Status::permission_denied("not yours"))));
        });
        // the host never connects: the attempt is waiting when the heartbeat finds the lease gone
        let driver = setup.driver(Script::NeverConnects);
        setup
            .run(lease(Duration::from_millis(900)), driver)
            .await
            .expect("nothing to report is not a failure");
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
}

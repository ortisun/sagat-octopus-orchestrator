//! What a lease costs the runner, whatever the attempt does: the heartbeats that renew it, and the terminal report that must
//! reach the control plane exactly once in effect (feature 035, T035-058, D035-143). The SQL and the lake attempt share it; each
//! says what the end of a lease means to it.
use crate::{
    capability,
    rpc::{Failure, Rpc},
};
use sagat_api_grpc::pb;
use sha2::{Digest as _, Sha256};
use tokio::{
    sync::watch,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

/// Why the heartbeats stopped. What each means is the attempt's: a SQL attempt cancels in both cases; a lake attempt tells the two
/// apart, because a write cancelled after its request was sent has an unknown outcome and a lease that is gone has no one to report to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeaseEnd {
    /// The control plane asked for the attempt to be cancelled.
    CancelRequested,
    /// The lease is not ours any more, or cannot be shown to be: a heartbeat refused, failed, answered with no expiry, or an expiry that
    /// came with no beat that renewed it.
    Lost,
}

pub(crate) struct Heartbeat {
    latest: watch::Receiver<Instant>,
    stop: CancellationToken,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl Heartbeat {
    /// Renews the lease until it ends or is finished. `on_end` is called at most once, from the heartbeat task, and must not block.
    pub(crate) fn start(
        rpc: Rpc,
        lease: &pb::TaskLease,
        worker_id: String,
        initial: Instant,
        on_end: impl FnOnce(LeaseEnd) + Send + 'static,
    ) -> Self {
        let (updates, latest) = watch::channel(initial);
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let heartbeat = pb::HeartbeatRequest {
            instance: lease.instance.clone(),
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
            let end = loop {
                let remaining = expires.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break LeaseEnd::Lost;
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
                    break LeaseEnd::Lost;
                };
                if response.cancel_requested {
                    break LeaseEnd::CancelRequested;
                }
                let Some(next) = capability::deadline(response.lease_expires_at_epoch_ms) else {
                    break LeaseEnd::Lost;
                };
                expires = next;
                updates.send_replace(next);
            };
            on_end(end);
        });
        Self {
            latest,
            stop,
            join: Some(join),
        }
    }

    /// The end of the lease as last renewed, or `bound` if that is sooner.
    pub(crate) fn deadline(&self, bound: Instant) -> Instant {
        (*self.latest.borrow()).min(bound)
    }

    pub(crate) async fn finish(mut self) {
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

/// What to tell the control plane when an attempt is over.
pub(crate) enum Report {
    Complete {
        /// The output of the step. Sent inline unless it was uploaded as a blob; the digest is of these bytes either way.
        bytes: Vec<u8>,
        blob: Option<String>,
    },
    Fail {
        code: &'static str,
        retryable: bool,
    },
}

/// Why a report did not reach the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unreported {
    /// The control plane answered and refused it, with this status. Sending it again would be refused again.
    Refused(tonic::Code),
    /// It could not be delivered before the lease or the bound on reporting ran out.
    Gone,
}

/// An output or an acknowledgement may be lost, but a retry of a report always sends exactly the same content under exactly the
/// same key. `profile` names the kind of attempt in the key (`database`, `lake`).
pub(crate) async fn report_until_ack(
    rpc: &Rpc,
    lease: &pb::TaskLease,
    profile: &str,
    report: &Report,
    heartbeat: &Heartbeat,
) -> Result<(), Unreported> {
    let bound = Instant::now() + Duration::from_secs(30);
    let mut pause = Duration::from_millis(100);
    loop {
        let deadline = heartbeat.deadline(bound);
        if deadline <= Instant::now() {
            return Err(Unreported::Gone);
        }
        let outcome = tokio::time::timeout_at(deadline, send_report(rpc, lease, profile, report))
            .await
            .map_err(|_| Unreported::Gone)?;
        match outcome {
            Ok(()) => return Ok(()),
            Err(error) if error.retryable() => {
                metrics::counter!("database_runner_report_retries_total").increment(1);
                tokio::time::sleep_until((Instant::now() + pause).min(deadline)).await;
                pause = (pause * 2).min(Duration::from_secs(1));
            }
            Err(error) => return Err(Unreported::Refused(error.code)),
        }
    }
}

async fn send_report(
    rpc: &Rpc,
    lease: &pb::TaskLease,
    profile: &str,
    report: &Report,
) -> Result<(), Failure> {
    match report {
        Report::Complete { bytes, blob } => {
            let output_json = if blob.is_some() {
                String::new()
            } else {
                std::str::from_utf8(bytes)
                    .map_err(|_| Failure {
                        code: tonic::Code::DataLoss,
                        quota: false,
                    })?
                    .to_owned()
            };
            let request = pb::CompleteStepRequest {
                instance: lease.instance.clone(),
                task_id: lease.task_id.clone(),
                run_id: lease.run_id.clone(),
                step_id: lease.step_id.clone(),
                attempt: lease.attempt,
                generation: lease.generation,
                lease_token: lease.lease_token.clone(),
                idempotency_key: format!(
                    "{profile}-complete-{}-{}",
                    lease.task_id, lease.generation
                ),
                output_json,
                output_blob_ref: blob.clone().unwrap_or_default(),
                output_sha256: sagat_domain::workflow::Digest::new(Sha256::digest(bytes).into())
                    .to_hex(),
                traceparent: lease.traceparent.clone(),
            };
            rpc.client
                .clone()
                .complete_step(rpc.request(request).await?)
                .await
                .map_err(Failure::from)?;
        }
        Report::Fail { code, retryable } => {
            let request = pb::FailStepRequest {
                instance: lease.instance.clone(),
                task_id: lease.task_id.clone(),
                run_id: lease.run_id.clone(),
                step_id: lease.step_id.clone(),
                attempt: lease.attempt,
                generation: lease.generation,
                lease_token: lease.lease_token.clone(),
                idempotency_key: format!("{profile}-fail-{}-{}", lease.task_id, lease.generation),
                error_code: (*code).into(),
                safe_message: String::new(),
                retryable: *retryable,
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        testing::{Fake, Home, Server, epoch_ms_in, home, serve_fake},
    };
    use tokio::sync::oneshot;
    use tonic::Status;

    fn lease() -> pb::TaskLease {
        pb::TaskLease {
            task_id: "task-1".into(),
            run_id: uuid::Uuid::from_u128(9).to_string(),
            step_id: uuid::Uuid::from_u128(10).to_string(),
            attempt: 1,
            generation: 3,
            lease_token: "lease-token".into(),
            ..Default::default()
        }
    }

    async fn rpc(fake: &Fake) -> (Rpc, Server, Home) {
        let server = serve_fake(fake.clone()).await;
        let home = home(&server, None);
        let config = Config::read(&home.config).await.unwrap();
        (Rpc::connect(&config).await.unwrap(), server, home)
    }

    /// Starts heartbeats with a lease that lasts `lasts`, and says how they ended and how long it took.
    async fn ended_after(fake: &Fake, lasts: Duration) -> (LeaseEnd, Duration) {
        let (rpc, _server, _home) = rpc(fake).await;
        let (ended, end) = oneshot::channel();
        let started = Instant::now();
        let _heartbeat = Heartbeat::start(
            rpc,
            &lease(),
            uuid::Uuid::from_u128(7).to_string(),
            started + lasts,
            move |end| {
                let _ = ended.send(end);
            },
        );
        let end = tokio::time::timeout(Duration::from_secs(20), end)
            .await
            .expect("the heartbeats end")
            .expect("and say how");
        (end, started.elapsed())
    }

    async fn ended(fake: &Fake) -> LeaseEnd {
        ended_after(fake, Duration::from_millis(600)).await.0
    }

    #[tokio::test]
    async fn a_cancel_the_control_plane_asks_for_is_not_a_lost_lease() {
        let fake = Fake::default();
        fake.with(|script| {
            script.heartbeat = Some(Box::new(|_| {
                Ok(pb::HeartbeatResponse {
                    lease_expires_at_epoch_ms: epoch_ms_in(Duration::from_secs(60)),
                    cancel_requested: true,
                })
            }));
        });
        assert_eq!(ended(&fake).await, LeaseEnd::CancelRequested);
    }

    #[tokio::test]
    async fn a_heartbeat_that_is_refused_or_answered_without_an_expiry_is_a_lost_lease() {
        let refused = Fake::default();
        refused.with(|script| {
            script.heartbeat = Some(Box::new(|_| Err(Status::permission_denied("not yours"))));
        });
        assert_eq!(ended(&refused).await, LeaseEnd::Lost);
        let without = Fake::default();
        without.with(|script| {
            script.heartbeat = Some(Box::new(|_| {
                Ok(pb::HeartbeatResponse {
                    lease_expires_at_epoch_ms: 0,
                    ..Default::default()
                })
            }));
        });
        // an answer with no expiry ends the lease at once, not when the expiry it had runs out
        let (end, elapsed) = ended_after(&without, Duration::from_secs(6)).await;
        assert_eq!(end, LeaseEnd::Lost);
        assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
    }

    #[tokio::test]
    async fn a_lease_that_is_renewed_does_not_end_until_it_is_finished() {
        let fake = Fake::default();
        let (rpc, _server, _home) = rpc(&fake).await;
        let (ended, mut end) = oneshot::channel();
        let heartbeat = Heartbeat::start(
            rpc,
            &lease(),
            uuid::Uuid::from_u128(7).to_string(),
            Instant::now() + Duration::from_millis(600),
            move |end| {
                let _ = ended.send(end);
            },
        );
        fake.until("two heartbeats", |script| script.heartbeats.len() >= 2)
            .await;
        assert!(end.try_recv().is_err(), "renewed, the lease has not ended");
        assert!(
            heartbeat.deadline(Instant::now() + Duration::from_secs(3600))
                > Instant::now() + Duration::from_secs(30),
            "and the deadline is the one the control plane gave"
        );
        heartbeat.finish().await;
        assert!(
            end.try_recv().is_err(),
            "finishing is not an end of the lease"
        );
        fake.with(|script| {
            let beat = &script.heartbeats[0];
            assert_eq!(beat.task_id, "task-1");
            assert_eq!(beat.generation, 3);
            assert_eq!(beat.lease_token, "lease-token");
        });
    }

    #[tokio::test]
    async fn a_report_that_is_retried_is_the_same_request_under_the_same_key() {
        let fake = Fake::default();
        fake.with(|script| {
            script.report_errors = [
                Status::unavailable("down"),
                Status::unavailable("still down"),
            ]
            .into();
        });
        let (rpc, _server, _home) = rpc(&fake).await;
        let lease = lease();
        let heartbeat = Heartbeat::start(
            rpc.clone(),
            &lease,
            uuid::Uuid::from_u128(7).to_string(),
            Instant::now() + Duration::from_secs(60),
            |_| {},
        );
        let report = Report::Complete {
            bytes: br#"{"kind":"iceberg"}"#.to_vec(),
            blob: None,
        };
        report_until_ack(&rpc, &lease, "lake", &report, &heartbeat)
            .await
            .expect("acknowledged at last");
        fake.with(|script| {
            assert_eq!(script.completed.len(), 3, "refused twice, accepted once");
            let first = &script.completed[0];
            assert_eq!(first.idempotency_key, "lake-complete-task-1-3");
            assert_eq!(first.output_json, r#"{"kind":"iceberg"}"#);
            assert!(script.completed.iter().all(|sent| sent == first));
        });
    }

    #[tokio::test]
    async fn a_failure_carries_the_flag_the_attempt_gave_it_under_a_key_of_its_profile() {
        let fake = Fake::default();
        let (rpc, _server, _home) = rpc(&fake).await;
        let lease = lease();
        let heartbeat = Heartbeat::start(
            rpc.clone(),
            &lease,
            uuid::Uuid::from_u128(7).to_string(),
            Instant::now() + Duration::from_secs(60),
            |_| {},
        );
        for (profile, retryable) in [("database", false), ("lake", true)] {
            let report = Report::Fail {
                code: "CATALOG_UNAVAILABLE",
                retryable,
            };
            report_until_ack(&rpc, &lease, profile, &report, &heartbeat)
                .await
                .unwrap();
        }
        fake.with(|script| {
            let sent: Vec<_> = script
                .failed
                .iter()
                .map(|fail| {
                    (
                        fail.idempotency_key.as_str(),
                        fail.retryable,
                        fail.error_code.as_str(),
                    )
                })
                .collect();
            assert_eq!(
                sent,
                [
                    ("database-fail-task-1-3", false, "CATALOG_UNAVAILABLE"),
                    ("lake-fail-task-1-3", true, "CATALOG_UNAVAILABLE"),
                ]
            );
        });
    }
}

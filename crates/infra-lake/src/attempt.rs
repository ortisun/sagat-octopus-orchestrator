//! One attempt, from the container to the report (feature 035, T035-035, D035-113).
//!
//! `run_attempt` joins what the earlier slices built: the relay on the attempt's socket ([`crate::relay`]), the container
//! the sandbox starts, the control conversation ([`crate::control`]) and the table of what an ending reports
//! ([`crate::supervisor`]). The caller has already made and filled the attempt directory ([`crate::staging`]) and built the
//! request line; this function runs the attempt and says what happened:
//!
//! 1. An attempt that is over before it starts (cancelled, its lease lost, its deadline past) starts nothing.
//! 2. It binds the relay's socket (mode 0660), takes the hand-over of `CONTROL`, and starts serving.
//! 3. It asks the sandbox for the container. A start that fails is [`Ended::SandboxUnavailable`].
//! 4. It waits for the host to open the control conversation, for at most [`Plan::host_start`], and stops waiting if the
//!    container is gone, the step is cancelled, the lease is lost or the deadline passes.
//! 5. It runs the conversation, with the same four things able to end it. **The request goes out only now**, so an attempt
//!    that ended before this point never sent the credentials ([`AttemptOutcome::request_sent`]).
//! 6. Whichever ended first is joined with what is ready in the same instant ([`Ended::strongest`]), the container is
//!    killed, the relay stops, and [`dispose`] says what to report.
//!
//! The container is killed once, after every start that was attempted, whatever the ending. The directory is the caller's
//! to remove once this returns, and so is the request buffer (it holds the credentials) to zeroise.
//!
//! The sandbox is a port. In production it is a client of the sandbox runner (G035-055); the tests give it a scripted host
//! that reads the token, connects to the real socket and plays one ending, so the runner's side of every ending is tested
//! without a container.

use std::fs::Permissions;
use std::future::Future;
use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use tokio::net::UnixListener;
use tokio::sync::oneshot;
use tokio::time::{Instant, sleep, sleep_until};
use tokio_util::sync::CancellationToken;

use crate::control::{ControlConfig, converse};
use crate::relay::{Connector, ControlStream, Relay, RelayDenials, Resolver};
use crate::staging::AttemptDir;
use crate::supervisor::{AttemptFacts, Disposition, Ended, dispose};

/// The mode of the relay's socket: the container is in the runner's group and connects to it (D035-086).
const SOCKET_MODE: u32 = 0o660;

/// What the sandbox is asked for. It carries no credential, no image, no mount and no command: the sandbox decides all of
/// those, from the attempt's id (D035-110 on the API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest {
    pub attempt_id: String,
    /// How long the container may live. The sandbox ends it at this time even if the runner is gone.
    pub deadline: Duration,
}

/// Why a container did not start. Closed kinds: the engine's own words are the operator's, not the report's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    #[error("the sandbox could not be reached or could not start the container")]
    Unavailable,
    #[error("the sandbox refused the container (the ceiling of containers on the node)")]
    Refused,
}

/// What the engine said about a container when it was ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerExit {
    /// The memory limit killed it. `false` when the engine could not be asked.
    pub memory_killed: bool,
}

/// The sandbox, as the attempt sees it.
pub trait LakeSandbox: Send + Sync {
    /// Starts the attempt's container.
    fn start(
        &self,
        request: &StartRequest,
    ) -> impl Future<Output = Result<(), SandboxError>> + Send;

    /// Resolves when the container is gone. It never resolves for one that keeps running.
    fn exited(&self, attempt_id: &str) -> impl Future<Output = ()> + Send;

    /// Ends the container, if there is one, and says how it died. Safe to call for one that never started.
    fn kill(&self, attempt_id: &str) -> impl Future<Output = ContainerExit> + Send;
}

/// What the attempt is to do and in what time.
#[derive(Debug, Clone, Copy)]
pub struct Plan<'a> {
    /// The request line, newline included. It holds the credentials: the caller zeroises it.
    pub request: &'a [u8],
    /// When the attempt is over, whatever it is doing.
    pub deadline: Instant,
    /// How long the host has to open the control conversation once its container is started.
    pub host_start: Duration,
    pub control: ControlConfig,
}

/// What can end the attempt from outside.
#[derive(Debug, Clone)]
pub struct Signals {
    /// The server asked for the step to be cancelled.
    pub cancelled: CancellationToken,
    /// The lease is gone: a heartbeat was refused or failed, or it ran out.
    pub lease_lost: CancellationToken,
}

/// How the attempt went.
#[derive(Debug)]
pub struct AttemptOutcome {
    pub ended: Ended,
    pub disposition: Disposition,
    /// The request line was handed to the host. Before that nothing can have been written to the table.
    pub request_sent: bool,
    pub memory_killed: bool,
    pub relay_denials: RelayDenials,
}

/// The endings that are ready now, apart from the one that woke the caller: the lease, a cancellation, the clock.
fn ready(first: Option<Ended>, plan: &Plan<'_>, signals: &Signals) -> Option<Ended> {
    let mut candidates: Vec<Ended> = first.into_iter().collect();
    if signals.lease_lost.is_cancelled() {
        candidates.push(Ended::LeaseLost);
    }
    if signals.cancelled.is_cancelled() {
        candidates.push(Ended::Cancelled);
    }
    if Instant::now() >= plan.deadline {
        candidates.push(Ended::DeadlinePassed);
    }
    Ended::strongest(candidates)
}

fn outcome<R: Resolver, C: Connector>(
    ended: Ended,
    request_sent: bool,
    memory_killed: bool,
    relay: &Relay<R, C>,
) -> AttemptOutcome {
    let facts = AttemptFacts {
        memory_killed,
        request_sent,
    };
    AttemptOutcome {
        disposition: dispose(ended.clone(), facts),
        ended,
        request_sent,
        memory_killed,
        relay_denials: relay.denials(),
    }
}

/// Binds the relay's socket with the mode of the decision. The directory is 0770 in the runner's group and the socket is
/// reached through it, so the instant between `bind` and the `chmod` exposes nothing the directory does not.
fn bind(attempt: &AttemptDir) -> std::io::Result<UnixListener> {
    let path = attempt.socket_path();
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, Permissions::from_mode(SOCKET_MODE))?;
    Ok(listener)
}

/// Runs the attempt (see the module). `on_heartbeat` is called once for each beat the host sent in time; it must not block.
pub async fn run_attempt<S, R, C>(
    sandbox: &S,
    attempt: &AttemptDir,
    relay: Relay<R, C>,
    plan: &Plan<'_>,
    signals: &Signals,
    on_heartbeat: impl FnMut(),
) -> AttemptOutcome
where
    S: LakeSandbox,
    R: Resolver,
    C: Connector,
{
    if let Some(ended) = ready(None, plan, signals) {
        return outcome(ended, false, false, &relay);
    }
    let Ok(listener) = bind(attempt) else {
        return outcome(Ended::SandboxUnavailable, false, false, &relay);
    };
    let Some(control) = relay.take_control() else {
        return outcome(Ended::SandboxUnavailable, false, false, &relay);
    };
    let shutdown = CancellationToken::new();
    let serving = tokio::spawn({
        let relay = relay.clone();
        let shutdown = shutdown.clone();
        async move { relay.serve(listener, shutdown).await }
    });

    let (ended, request_sent) =
        drive(sandbox, attempt.id(), plan, signals, control, on_heartbeat).await;

    // every start that was attempted is ended, whatever the ending: a container may be up after a start that failed
    let exit = sandbox.kill(attempt.id()).await;
    shutdown.cancel();
    let _ = serving.await;
    outcome(ended, request_sent, exit.memory_killed, &relay)
}

/// The container, the wait for the host and the conversation. Returns how it ended and whether the request went out.
async fn drive<S: LakeSandbox>(
    sandbox: &S,
    id: &str,
    plan: &Plan<'_>,
    signals: &Signals,
    control: oneshot::Receiver<ControlStream>,
    on_heartbeat: impl FnMut(),
) -> (Ended, bool) {
    let began = Instant::now();
    // The time left is read once and is the container's whole life. The deadline can pass between the check that began the
    // attempt and here (a slow bind, a busy machine); a container with no time left is not started.
    let remaining = plan.deadline.saturating_duration_since(began);
    if remaining.is_zero() {
        return (Ended::DeadlinePassed, false);
    }
    let start = StartRequest {
        attempt_id: id.to_owned(),
        deadline: remaining,
    };
    if sandbox.start(&start).await.is_err() {
        return (
            ready(Some(Ended::SandboxUnavailable), plan, signals)
                .unwrap_or(Ended::SandboxUnavailable),
            false,
        );
    }

    let connected = tokio::select! {
        stream = control => stream.map_err(|_| Ended::HostNeverConnected),
        () = sleep(plan.host_start) => Err(Ended::HostNeverConnected),
        () = sandbox.exited(id) => Err(Ended::HostNeverConnected),
        () = signals.cancelled.cancelled() => Err(Ended::Cancelled),
        () = signals.lease_lost.cancelled() => Err(Ended::LeaseLost),
        () = sleep_until(plan.deadline) => Err(Ended::DeadlinePassed),
    };
    let stream = match connected {
        Ok(stream) => stream,
        Err(first) => {
            return (
                ready(Some(first.clone()), plan, signals).unwrap_or(first),
                false,
            );
        }
    };

    let ended = tokio::select! {
        result = converse(stream, plan.request, plan.control, began, on_heartbeat) => Ended::Conversation(result),
        () = signals.cancelled.cancelled() => Ended::Cancelled,
        () = signals.lease_lost.cancelled() => Ended::LeaseLost,
        () = sleep_until(plan.deadline) => Ended::DeadlinePassed,
    };
    (
        ready(Some(ended.clone()), plan, signals).unwrap_or(ended),
        true,
    )
}

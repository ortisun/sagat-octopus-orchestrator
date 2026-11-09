//! One attempt, from the container to the report (feature 035, T035-035, D035-113).
//!
//! `run_attempt` joins what the earlier slices built: the relay on the attempt's socket ([`crate::relay`]), the container
//! the sandbox starts, the control conversation ([`crate::control`]) and the table of what an ending reports
//! ([`crate::supervisor`]). The caller has already made and filled the attempt directory ([`crate::staging`]), built the
//! request line, and built a relay that checks its peer; this function runs the attempt and says what happened:
//!
//! 1. A relay that does not check its peer is refused: `CONTROL` is the request that is handed the credentials, and the
//!    token alone is a file the runner's group can read (G035-056).
//! 2. An attempt that is over before it starts (cancelled, its lease lost, its deadline past) starts nothing.
//! 3. It binds the relay's socket (mode 0660), takes the hand-over of `CONTROL`, and starts serving.
//! 4. It asks the sandbox for the container. Cancellation, a lost lease and the deadline end the wait for a start that does
//!    not return; a start that fails is [`Ended::SandboxUnavailable`].
//! 5. It waits for the host to open the control conversation, for at most [`Plan::host_start`], and stops waiting if the
//!    container is gone, the step is cancelled, the lease is lost or the deadline passes.
//! 6. **Only if none of those is ready** the request goes out, and the conversation runs with the same three able to end it.
//!    The signals are polled first, so a signal that is ready when the host connects is seen before the credentials leave.
//!    [`AttemptOutcome::request_sent`] is set by the conversation itself when it begins to write, not by reaching this point.
//! 7. Whichever ended first is joined with what is ready in the same instant ([`Ended::strongest`]), **a result already
//!    decoded stands** even if a cancellation or the deadline ends the wait for the host to close, the relay stops, the
//!    container is killed (within [`Plan::kill_limit`]; whether it was is in [`AttemptOutcome::container_ended`]), and
//!    [`dispose`] says what to report.
//!
//! The relay stops **before** the container is ended: its tunnels are the container's only route out. A lease lost while
//! the container was being ended still reports nothing.
//!
//! **Dropping the future.** If the caller drops `run_attempt` (its own deadline, a panic), the relay stops and its task is
//! aborted, but the container is not killed: that needs an `await`, and the backstop is the life time in
//! [`StartRequest::deadline`], which the sandbox enforces even if the runner is gone. The attempt directory (the caller's)
//! should then be removed only after that time, or after the sandbox says the container is gone.
//!
//! The container is killed once, after every start that was attempted, whatever the ending. The directory is the caller's
//! to remove once this returns, and the request buffer (it holds the credentials) is the caller's to drop: it is the `Zeroizing`
//! buffer of [`crate::protocol::encode_request`].
//!
//! The sandbox is a port. In production it is a client of the sandbox runner (G035-055); the tests give it a scripted host
//! that reads the token, connects to the real socket and plays one ending, so the runner's side of every ending is tested
//! without a container.

use std::fmt;
use std::fs::Permissions;
use std::future::Future;
use std::os::unix::fs::PermissionsExt as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, sleep_until, timeout};
use tokio_util::sync::CancellationToken;

use crate::control::{ControlConfig, Observer, converse};
use crate::protocol::HostResult;
use crate::relay::{Connector, ControlStream, Relay, RelayDenials, Resolver};
use crate::sandbox_api::{Class, Reason};
use crate::staging::AttemptDir;
use crate::supervisor::{AttemptFacts, Disposition, Ended, dispose};

/// The mode of the relay's socket: the container is in the runner's group and connects to it (D035-086).
const SOCKET_MODE: u32 = 0o660;

/// What the sandbox is asked for. It carries no credential, no image, no mount and no command: the sandbox decides all of
/// those, from the attempt's id (ADR-0079). It does carry the slot, because the relay needs the user of the container before
/// the container exists, and the class of budget, which is a name and not a limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest {
    pub attempt_id: String,
    /// The slot the container runs in; its user is [`crate::sandbox_api::uid_for_slot`], which the relay admits and nobody else.
    pub slot: u32,
    pub class: Class,
    /// How long the container may live, from the moment of the request. The sandbox ends it at this time even if the runner
    /// is gone: it is the backstop for a runner that dies or drops the attempt, so it is never zero.
    pub deadline: Duration,
}

/// Why a container did not start. Closed kinds: the engine's own words are the operator's, not the report's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    #[error("the sandbox could not be reached or could not start the container")]
    Unavailable,
    /// The sandbox answered and said no, for a reason that names a rule and never a value.
    #[error("the sandbox refused the container ({})", .0.as_str())]
    Refused(Reason),
}

/// What the engine said about a container when it was ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerExit {
    /// The memory limit killed it. `false` when the engine could not be asked.
    pub memory_killed: bool,
    /// The sandbox confirms the container is gone. `false` when it could not be reached, or the engine did not answer: a
    /// container with the credentials may still be up, and the report says so.
    pub ended: bool,
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

    /// Ends the container, if there is one, and says how it died. Safe to call for one that never started, or whose start
    /// did not return.
    fn kill(&self, attempt_id: &str) -> impl Future<Output = ContainerExit> + Send;
}

/// What the attempt is to do and in what time.
#[derive(Clone, Copy)]
pub struct Plan<'a> {
    /// The request line, newline included. It holds the credentials: it is the `Zeroizing` buffer of
    /// [`crate::protocol::encode_request`], and the caller drops it as soon as the attempt is over.
    pub request: &'a [u8],
    /// When the attempt is over, whatever it is doing.
    pub deadline: Instant,
    /// The slot of the attempt on this node. The relay is built for the user of this slot ([`crate::sandbox_api::uid_for_slot`]).
    pub slot: u32,
    /// The budget class of the attempt: the lake runner's own table maps a kind of write to one.
    pub class: Class,
    /// How long the host has to open the control conversation once its container is started.
    pub host_start: Duration,
    pub control: ControlConfig,
    /// How long the sandbox has to end the container before the attempt gives up waiting and says it could not.
    pub kill_limit: Duration,
}

/// The request holds the credentials, so a `Debug` of the plan shows its size and nothing else.
impl fmt::Debug for Plan<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Plan")
            .field(
                "request",
                &format_args!("<{} bytes, not shown>", self.request.len()),
            )
            .field("deadline", &self.deadline)
            .field("slot", &self.slot)
            .field("class", &self.class)
            .field("host_start", &self.host_start)
            .field("control", &self.control)
            .field("kill_limit", &self.kill_limit)
            .finish()
    }
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
pub struct AttemptOutcome {
    pub ended: Ended,
    pub disposition: Disposition,
    /// The runner began to hand the request to the host. Before that nothing can have been written to the table.
    pub request_sent: bool,
    pub memory_killed: bool,
    /// The sandbox confirmed the container is gone. When it is not, a host with the credentials may still be running, until
    /// the life time it was started with runs out.
    pub container_ended: bool,
    pub relay_denials: RelayDenials,
    /// Why the sandbox said no, when it answered and said no (ADR-0079, rule 15): kept apart from an outage, which this is
    /// `None` for, so that a node at its ceiling and a dead engine are not the same line in the driver's metrics and in the
    /// failure's detail. Both end the attempt as the sandbox being unavailable.
    pub sandbox_refusal: Option<Reason>,
}

/// `ended` is left out: it holds what the host sent, the column of a failure among it, which [`dispose`] filters and a
/// `Debug` of the raw value would not.
impl fmt::Debug for AttemptOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttemptOutcome")
            .field("disposition", &self.disposition)
            .field("request_sent", &self.request_sent)
            .field("memory_killed", &self.memory_killed)
            .field("container_ended", &self.container_ended)
            .field("relay_denials", &self.relay_denials)
            .field("sandbox_refusal", &self.sandbox_refusal)
            .finish_non_exhaustive()
    }
}

/// The ending that wins among `first`, a result already decoded, and the endings that are ready now (the lease, a
/// cancellation, the clock). A result that was decoded stands against everything but a lost lease; a ending that is itself
/// a conversation (broken or not) is not replaced by an earlier result.
fn settle(
    first: Option<Ended>,
    seen: Option<HostResult>,
    plan: &Plan<'_>,
    signals: &Signals,
) -> Option<Ended> {
    let mut candidates: Vec<Ended> = first.into_iter().collect();
    if let Some(result) = seen
        && !candidates
            .iter()
            .any(|ended| matches!(ended, Ended::Conversation(_)))
    {
        candidates.push(Ended::Conversation(Ok(result)));
    }
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

fn ready(plan: &Plan<'_>, signals: &Signals) -> Option<Ended> {
    settle(None, None, plan, signals)
}

fn outcome<R: Resolver, C: Connector>(
    ended: Ended,
    request_sent: bool,
    exit: ContainerExit,
    relay: &Relay<R, C>,
) -> AttemptOutcome {
    let facts = AttemptFacts {
        memory_killed: exit.memory_killed,
        request_sent,
    };
    AttemptOutcome {
        disposition: dispose(ended.clone(), facts),
        ended,
        request_sent,
        memory_killed: exit.memory_killed,
        container_ended: exit.ended,
        relay_denials: relay.denials(),
        sandbox_refusal: None,
    }
}

/// An attempt that ran nothing: no container, so none to end.
const NOTHING_RAN: ContainerExit = ContainerExit {
    memory_killed: false,
    ended: true,
};

/// Binds the relay's socket with the mode of the decision. The directory is 0770 in the runner's group and the socket is
/// reached through it; the `chmod` is by path, which a member of that group could race with a symlink before any container
/// exists (the work root is mounted in nothing else), a residual noted in D035-113.
fn bind(attempt: &AttemptDir) -> std::io::Result<UnixListener> {
    let path = attempt.socket_path();
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, Permissions::from_mode(SOCKET_MODE))?;
    Ok(listener)
}

/// The relay's task. Dropping it stops the relay and aborts the task, so a dropped attempt leaves nothing serving.
struct Serving {
    stop: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl Serving {
    fn start<R: Resolver, C: Connector>(relay: Relay<R, C>, listener: UnixListener) -> Self {
        let stop = CancellationToken::new();
        let task = tokio::spawn({
            let stop = stop.clone();
            async move { relay.serve(listener, stop).await }
        });
        Self {
            stop,
            task: Some(task),
        }
    }

    /// Stops the relay and waits until it has.
    async fn finish(mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

/// What the conversation tells the attempt while it runs.
struct Watch<F> {
    beat: F,
    sent: Arc<AtomicBool>,
    seen: Arc<Mutex<Option<HostResult>>>,
}

impl<F: FnMut()> Observer for Watch<F> {
    fn heartbeat(&mut self) {
        (self.beat)();
    }

    fn request_sent(&mut self) {
        self.sent.store(true, Ordering::SeqCst);
    }

    fn result(&mut self, result: &HostResult) {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(result.clone());
        }
    }
}

/// Runs the attempt (see the module). `on_heartbeat` is called once for each beat the host sent in time; it must not block.
pub async fn run_attempt<S, R, C>(
    sandbox: &S,
    attempt: &AttemptDir,
    relay: Relay<R, C>,
    plan: &Plan<'_>,
    signals: &Signals,
    on_heartbeat: impl FnMut() + Send,
) -> AttemptOutcome
where
    S: LakeSandbox,
    R: Resolver,
    C: Connector,
{
    // a relay that does not check who connects would hand the credentials to whoever has a copy of the token
    if !relay.requires_peer() {
        return outcome(Ended::SandboxUnavailable, false, NOTHING_RAN, &relay);
    }
    if let Some(ended) = ready(plan, signals) {
        return outcome(ended, false, NOTHING_RAN, &relay);
    }
    let Ok(listener) = bind(attempt) else {
        return outcome(Ended::SandboxUnavailable, false, NOTHING_RAN, &relay);
    };
    let Some(control) = relay.take_control() else {
        return outcome(Ended::SandboxUnavailable, false, NOTHING_RAN, &relay);
    };
    let serving = Serving::start(relay.clone(), listener);

    let mut refusal = None;
    let (ended, request_sent, started) = drive(
        sandbox,
        attempt.id(),
        plan,
        signals,
        control,
        on_heartbeat,
        &mut refusal,
    )
    .await;

    // The relay goes first: its tunnels are the container's only route out, and a kill that hangs must not leave them open.
    serving.finish().await;
    let exit = if started {
        // every start that was attempted is ended, whatever the ending: a container may be up after a start that failed
        timeout(plan.kill_limit, sandbox.kill(attempt.id()))
            .await
            .unwrap_or(ContainerExit {
                memory_killed: false,
                ended: false,
            })
    } else {
        NOTHING_RAN
    };
    // a lease lost while the container was being ended is lost all the same
    let ended = if signals.lease_lost.is_cancelled() {
        Ended::LeaseLost
    } else {
        ended
    };
    AttemptOutcome {
        sandbox_refusal: refusal,
        ..outcome(ended, request_sent, exit, &relay)
    }
}

/// The container, the wait for the host and the conversation. Returns how it ended, whether the request was begun, and
/// whether a start was attempted (so a container may need ending).
async fn drive<S: LakeSandbox>(
    sandbox: &S,
    id: &str,
    plan: &Plan<'_>,
    signals: &Signals,
    control: oneshot::Receiver<ControlStream>,
    on_heartbeat: impl FnMut() + Send,
    refusal: &mut Option<Reason>,
) -> (Ended, bool, bool) {
    let began = Instant::now();
    // The deadline, a cancellation or a lease can have come since the check that began the attempt (a slow bind, a busy
    // machine). The time left is read once and is the container's whole life; a container with none is not started.
    if let Some(ended) = ready(plan, signals) {
        return (ended, false, false);
    }
    let remaining = plan.deadline.saturating_duration_since(began);
    if remaining.is_zero() {
        return (Ended::DeadlinePassed, false, false);
    }
    let request = StartRequest {
        attempt_id: id.to_owned(),
        slot: plan.slot,
        class: plan.class,
        deadline: remaining,
    };
    let started = tokio::select! {
        biased;
        () = signals.lease_lost.cancelled() => Err(Ended::LeaseLost),
        () = signals.cancelled.cancelled() => Err(Ended::Cancelled),
        () = sleep_until(plan.deadline) => Err(Ended::DeadlinePassed),
        result = sandbox.start(&request) => result.map_err(|error| {
            if let SandboxError::Refused(reason) = error {
                *refusal = Some(reason);
            }
            Ended::SandboxUnavailable
        }),
    };
    if let Err(first) = started {
        return (settled(first, None, plan, signals), false, true);
    }

    let connected = tokio::select! {
        biased;
        () = signals.lease_lost.cancelled() => Err(Ended::LeaseLost),
        () = signals.cancelled.cancelled() => Err(Ended::Cancelled),
        () = sleep_until(plan.deadline) => Err(Ended::DeadlinePassed),
        stream = control => stream.map_err(|_| Ended::HostNeverConnected),
        () = sandbox.exited(id) => Err(Ended::HostNeverConnected),
        () = sleep(plan.host_start) => Err(Ended::HostNeverConnected),
    };
    let stream = match connected {
        Ok(stream) => stream,
        Err(first) => return (settled(first, None, plan, signals), false, true),
    };
    let (ended, request_sent) = conversation(stream, began, plan, signals, on_heartbeat).await;
    (ended, request_sent, true)
}

/// The conversation with a host whose stream is in hand. Returns how it ended and whether the request was begun.
///
/// **Nothing may be ready when the credentials leave.** A signal can be ready at the moment the stream arrives, and the
/// request is written by the first poll of `converse`; `select!` polls its arms in an order that is not the order they are
/// listed in unless it is `biased`. So the signals are checked first, here, and are the first arms of the `select!` too.
async fn conversation<S>(
    stream: S,
    began: Instant,
    plan: &Plan<'_>,
    signals: &Signals,
    on_heartbeat: impl FnMut() + Send,
) -> (Ended, bool)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Some(ended) = ready(plan, signals) {
        return (ended, false);
    }
    let sent = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(None));
    let watch = Watch {
        beat: on_heartbeat,
        sent: Arc::clone(&sent),
        seen: Arc::clone(&seen),
    };
    let first = tokio::select! {
        biased;
        () = signals.lease_lost.cancelled() => Ended::LeaseLost,
        () = signals.cancelled.cancelled() => Ended::Cancelled,
        () = sleep_until(plan.deadline) => Ended::DeadlinePassed,
        result = converse(stream, plan.request, plan.control, began, watch) => Ended::Conversation(result),
    };
    let result = seen.lock().ok().and_then(|mut seen| seen.take());
    (
        settled(first, result, plan, signals),
        sent.load(Ordering::SeqCst),
    )
}

/// [`settle`] for an ending that is known: it is never replaced by nothing.
fn settled(first: Ended, seen: Option<HostResult>, plan: &Plan<'_>, signals: &Signals) -> Ended {
    let fallback = first.clone();
    settle(Some(first), seen, plan, signals).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ControlFailure;
    use crate::protocol::{Decoder, Event, ProtocolFailure};

    fn plan(deadline: Instant) -> Plan<'static> {
        Plan {
            request: b"{}\n",
            deadline,
            slot: 1,
            class: Class::Small,
            host_start: Duration::from_secs(1),
            control: ControlConfig::default(),
            kill_limit: Duration::from_secs(1),
        }
    }

    fn signals() -> Signals {
        Signals {
            cancelled: CancellationToken::new(),
            lease_lost: CancellationToken::new(),
        }
    }

    fn later() -> Plan<'static> {
        plan(Instant::now() + Duration::from_secs(3600))
    }

    fn passed() -> Plan<'static> {
        plan(Instant::now() - Duration::from_millis(1))
    }

    fn a_result() -> HostResult {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/contract/iceberg-writes/fixtures/receipts/iceberg-committed.json");
        let receipt: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_default())
                .unwrap_or_default();
        let line = serde_json::to_vec(&serde_json::json!({"receipt": receipt, "relayDenials": 0}))
            .unwrap_or_default();
        match Decoder::new().line(&line, Duration::ZERO) {
            Ok(Event::Result(result)) => result,
            other => unreachable!("the fixture receipt is a result: {other:?}"),
        }
    }

    #[test]
    fn a_result_already_decoded_stands_against_a_cancellation_and_a_deadline() {
        let signals = signals();
        signals.cancelled.cancel();
        let decided = settled(Ended::Cancelled, Some(a_result()), &passed(), &signals);
        assert!(matches!(decided, Ended::Conversation(Ok(_))), "{decided:?}");
        let decided = settled(
            Ended::DeadlinePassed,
            Some(a_result()),
            &passed(),
            &self::signals(),
        );
        assert!(matches!(decided, Ended::Conversation(Ok(_))), "{decided:?}");
    }

    #[test]
    fn a_lost_lease_still_beats_a_result_already_decoded() {
        let signals = signals();
        signals.lease_lost.cancel();
        let decided = settled(Ended::Cancelled, Some(a_result()), &later(), &signals);
        assert_eq!(decided, Ended::LeaseLost);
    }

    #[test]
    fn a_conversation_that_broke_after_its_result_is_not_replaced_by_the_result() {
        // a byte after the result is a break of the protocol: the attempt fails and the next one finds the commit
        let broken = Ended::Conversation(Err(ControlFailure::Protocol(
            ProtocolFailure::LineAfterResult,
        )));
        let decided = settled(broken.clone(), Some(a_result()), &later(), &signals());
        assert_eq!(decided, broken);
    }

    #[test]
    fn what_is_ready_with_the_ending_is_joined_to_it() {
        let signals = signals();
        signals.lease_lost.cancel();
        assert_eq!(
            settled(
                Ended::Conversation(Err(ControlFailure::Silent)),
                None,
                &later(),
                &signals
            ),
            Ended::LeaseLost
        );
        assert_eq!(
            settled(Ended::HostNeverConnected, None, &passed(), &self::signals()),
            Ended::DeadlinePassed
        );
        assert_eq!(
            settled(Ended::SandboxUnavailable, None, &passed(), &self::signals()),
            Ended::DeadlinePassed
        );
    }

    /// A signal that is ready when the stream is in hand must stop the request: the conversation is entered with a live
    /// stream and a signal already set, and the host's side must hear nothing. `select!` would write in about half of the
    /// rounds if the signals were not checked first, so the round is run many times.
    #[tokio::test]
    async fn a_signal_that_is_ready_with_the_stream_stops_the_request_from_being_written() {
        use tokio::io::AsyncReadExt as _;
        for round in 0..60 {
            for lose_lease in [false, true] {
                let (runner, mut host) = tokio::io::duplex(1 << 16);
                let signals = signals();
                if lose_lease {
                    signals.lease_lost.cancel();
                } else {
                    signals.cancelled.cancel();
                }
                let plan = later();
                let (ended, sent) =
                    conversation(runner, Instant::now(), &plan, &signals, || {}).await;
                assert!(!sent, "round {round}, lease lost: {lose_lease}");
                assert_eq!(
                    ended,
                    if lose_lease {
                        Ended::LeaseLost
                    } else {
                        Ended::Cancelled
                    }
                );
                // the runner's side is gone: whatever it wrote would be read before the end of the stream
                let mut heard = Vec::new();
                host.read_to_end(&mut heard).await.unwrap_or_default();
                assert!(
                    heard.is_empty(),
                    "round {round}: the host was given the request"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_deadline_that_is_past_with_the_stream_stops_the_request_too() {
        use tokio::io::AsyncReadExt as _;
        let (runner, mut host) = tokio::io::duplex(1 << 16);
        let plan = passed();
        let (ended, sent) = conversation(runner, Instant::now(), &plan, &signals(), || {}).await;
        assert_eq!((ended, sent), (Ended::DeadlinePassed, false));
        let mut heard = Vec::new();
        host.read_to_end(&mut heard).await.unwrap_or_default();
        assert!(heard.is_empty());
    }

    #[test]
    fn nothing_ready_is_nothing() {
        assert_eq!(ready(&later(), &signals()), None);
        assert_eq!(ready(&passed(), &signals()), Some(Ended::DeadlinePassed));
    }
}

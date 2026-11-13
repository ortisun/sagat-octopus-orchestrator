//! One attempt, end to end without a container (feature 035, T035-035, D035-113): staging, the relay on a real Unix socket,
//! the control conversation and the supervisor, joined by `run_attempt`, with a scripted sandbox in place of the sandbox
//! runner.
//!
//! The scripted host does what the Python host does at the wire: it reads `relay.token` from the attempt directory,
//! connects to `relay.sock`, presents the token, asks for `CONTROL`, reads the request line, and then plays one ending.
//! The tests therefore see the runner's real side of every ending: the container is killed once, the relay stops, the
//! request goes out only when it should, and what is reported is what the supervisor's table says.
//!
//! Times are real and short (a few hundred milliseconds): a paused clock does not advance while real socket I/O is
//! pending, so these tests would skip ahead of it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant as StdInstant};

use sagat_domain::database::DatabaseFailure;
use sagat_infra_lake::attempt::{
    ContainerExit, LakeSandbox, Plan, SandboxError, Signals, StartRequest, run_attempt,
};
use sagat_infra_lake::control::ControlConfig;
use sagat_infra_lake::protocol::RelayToken;
use sagat_infra_lake::relay::{Connector, Endpoint, EndpointPolicy, Relay, RelayConfig, Resolver};
use sagat_infra_lake::sandbox_api::{Class, Reason};
use sagat_infra_lake::staging::{AttemptDir, StagingLimits};
use sagat_infra_lake::supervisor::{Disposition, Ended};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "relay-token-canary-0123456789abcdef";
const ID: &str = "a1b2";
const REQUEST: &[u8] = b"{\"protocol\":\"sagat.lake.v1\",\"operation\":\"icebergWrite\"}\n";
const BEAT: &[u8] = b"{\"heartbeat\":true}\n";

fn receipt() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/receipts/iceberg-committed.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn line(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    bytes
}

fn result() -> Vec<u8> {
    line(&json!({"receipt": receipt(), "relayDenials": 0}))
}

fn failure(code: &str) -> Vec<u8> {
    line(&json!({"failure": {"code": code}, "relayDenials": 0}))
}

// -- the doubles ------------------------------------------------------------------------------------------------------

struct NoDns;

impl Resolver for NoDns {
    async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
        Err(io::Error::other("no name server in this test"))
    }
}

struct NoNetwork;

impl Connector for NoNetwork {
    type Stream = DuplexStream;

    async fn connect(&self, _address: SocketAddr) -> io::Result<DuplexStream> {
        Err(io::Error::other("no network in this test"))
    }
}

/// A signal the scripted host fires at the moment it has asked for `CONTROL`, before it reads anything.
#[derive(Clone)]
enum Signal {
    Cancel(CancellationToken),
    LoseLease(CancellationToken),
}

impl Signal {
    fn fire(&self) {
        match self {
            Self::Cancel(token) | Self::LoseLease(token) => token.cancel(),
        }
    }
}

/// What the scripted host does once it is connected, or instead of connecting.
#[derive(Clone)]
enum Script {
    /// Asks for `CONTROL`, fires the signal in the same breath, then waits for a request that should not come.
    ConnectsThenSignals(Signal),
    /// Reads the request, answers with these bytes (a beat first if asked), and closes.
    Answer { beats: usize, line: Vec<u8> },
    /// Reads the request and then says nothing, with the connection open.
    Silent,
    /// Reads the request and goes away with no result.
    DiesAfterTheRequest,
    /// Is started and does nothing: it never connects.
    NeverConnects,
    /// Is started and is gone at once.
    ExitsAtOnce,
    /// Asks the relay for an endpoint nobody declared, reads the refusal, and then answers.
    ProbesThenAnswers(Vec<u8>),
}

struct Scripted {
    work: PathBuf,
    script: Script,
    start_fails: Option<SandboxError>,
    start_hangs: bool,
    kill_hangs: bool,
    kill_unconfirmed: bool,
    /// Lost during the kill, as a lease can be while a slow container is being ended.
    lose_lease_in_kill: Option<CancellationToken>,
    /// Whether the relay still accepted a connection when the container was ended.
    relay_up_in_kill: Arc<Mutex<Option<bool>>>,
    memory_killed: bool,
    starts: AtomicUsize,
    kills: AtomicUsize,
    /// What the sandbox was asked, in order.
    asked: Mutex<Vec<StartRequest>>,
    host: Mutex<Option<JoinHandle<()>>>,
    /// What the host read as its request line.
    request: Arc<Mutex<Vec<u8>>>,
    /// Told once the host has the request.
    has_request: Arc<tokio::sync::Notify>,
    /// The mode of `relay.sock` when the host connected.
    socket_mode: Arc<Mutex<Option<u32>>>,
}

impl Scripted {
    fn new(work: &std::path::Path, script: Script) -> Self {
        Self {
            work: work.to_path_buf(),
            script,
            start_fails: None,
            start_hangs: false,
            kill_hangs: false,
            kill_unconfirmed: false,
            lose_lease_in_kill: None,
            relay_up_in_kill: Arc::default(),
            memory_killed: false,
            starts: AtomicUsize::new(0),
            kills: AtomicUsize::new(0),
            asked: Mutex::new(Vec::new()),
            host: Mutex::new(None),
            request: Arc::default(),
            has_request: Arc::default(),
            socket_mode: Arc::default(),
        }
    }

    fn kills(&self) -> usize {
        self.kills.load(Ordering::SeqCst)
    }
}

async fn host(
    work: PathBuf,
    id: String,
    script: Script,
    request: Arc<Mutex<Vec<u8>>>,
    has_request: Arc<tokio::sync::Notify>,
    socket_mode: Arc<Mutex<Option<u32>>>,
) {
    match &script {
        Script::NeverConnects => return std::future::pending().await,
        Script::ExitsAtOnce => return,
        _ => {}
    }
    let directory = work.join(&id);
    // as the Python host does: the token and a newline from the file in the attempt directory
    let token = std::fs::read_to_string(directory.join("relay.token")).unwrap();
    let socket = directory.join("relay.sock");
    *socket_mode.lock().unwrap() =
        Some(std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777);

    if matches!(script, Script::ProbesThenAnswers(_)) {
        let mut probe = UnixStream::connect(&socket).await.unwrap();
        probe
            .write_all(
                format!(
                    "SAGAT-RELAY/1 {}\nCONNECT evil.example:443 HTTP/1.1\r\n\r\n",
                    token.trim_end()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut refusal = Vec::new();
        probe.read_to_end(&mut refusal).await.unwrap();
        assert!(refusal.starts_with(b"HTTP/1.1 403"), "{refusal:?}");
    }

    let mut stream = BufReader::new(UnixStream::connect(&socket).await.unwrap());
    stream
        .get_mut()
        .write_all(format!("SAGAT-RELAY/1 {}\nCONTROL\n", token.trim_end()).as_bytes())
        .await
        .unwrap();
    let mut asked = Vec::new();
    stream.get_mut().flush().await.unwrap();
    if let Script::ConnectsThenSignals(signal) = &script {
        signal.fire();
    }
    stream.read_until(b'\n', &mut asked).await.unwrap();
    *request.lock().unwrap() = asked;
    has_request.notify_waiters();
    has_request.notify_one();

    match script {
        Script::Answer { beats, line } => {
            for _ in 0..beats {
                stream.get_mut().write_all(BEAT).await.unwrap();
            }
            stream.get_mut().write_all(&line).await.unwrap();
            stream.get_mut().shutdown().await.unwrap();
        }
        Script::ProbesThenAnswers(answer) => {
            stream.get_mut().write_all(&answer).await.unwrap();
            stream.get_mut().shutdown().await.unwrap();
        }
        Script::Silent | Script::ConnectsThenSignals(_) => std::future::pending().await,
        Script::DiesAfterTheRequest | Script::NeverConnects | Script::ExitsAtOnce => {}
    }
}

impl LakeSandbox for Scripted {
    async fn start(&self, request: &StartRequest) -> Result<(), SandboxError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.asked.lock().unwrap().push(request.clone());
        if let Some(error) = self.start_fails {
            return Err(error);
        }
        if self.start_hangs {
            return std::future::pending().await;
        }
        assert!(request.deadline > Duration::ZERO);
        let task = tokio::spawn(host(
            self.work.clone(),
            request.attempt_id.clone(),
            self.script.clone(),
            Arc::clone(&self.request),
            Arc::clone(&self.has_request),
            Arc::clone(&self.socket_mode),
        ));
        *self.host.lock().unwrap() = Some(task);
        Ok(())
    }

    async fn exited(&self, _attempt_id: &str) {
        match self.script {
            Script::ExitsAtOnce => {
                // the host task is over as soon as it has run
                let task = self.host.lock().unwrap().take();
                if let Some(task) = task {
                    let _ = task.await;
                }
            }
            _ => std::future::pending().await,
        }
    }

    async fn kill(&self, attempt_id: &str) -> ContainerExit {
        self.kills.fetch_add(1, Ordering::SeqCst);
        // is the relay still there to be reached? The runner stops it before it ends the container
        let socket = self.work.join(attempt_id).join("relay.sock");
        *self.relay_up_in_kill.lock().unwrap() = Some(UnixStream::connect(&socket).await.is_ok());
        if let Some(lease) = &self.lose_lease_in_kill {
            lease.cancel();
        }
        if self.kill_hangs {
            return std::future::pending().await;
        }
        if let Some(task) = self.host.lock().unwrap().take() {
            task.abort();
        }
        ContainerExit {
            memory_killed: self.memory_killed,
            ended: !self.kill_unconfirmed,
        }
    }
}

// -- the fixture ------------------------------------------------------------------------------------------------------

struct World {
    work: tempfile::TempDir,
    attempt: AttemptDir,
}

fn world() -> World {
    // a short path: a Unix socket's path is limited to about a hundred bytes
    let work = tempfile::Builder::new()
        .prefix("lk")
        .tempdir_in("/tmp")
        .unwrap();
    let token = RelayToken::new(TOKEN).unwrap();
    let limits = StagingLimits::new(1 << 20, 1 << 20).unwrap();
    let attempt = AttemptDir::create(work.path(), ID, &token, limits).unwrap();
    World { work, attempt }
}

/// The user this test runs as: the scripted host is in this process, so the kernel reports it as the peer.
fn my_uid() -> u32 {
    use std::os::unix::fs::MetadataExt as _;
    tempfile::tempfile().unwrap().metadata().unwrap().uid()
}

fn relay_for(peer: Option<u32>) -> Relay<NoDns, NoNetwork> {
    let endpoints = EndpointPolicy::new([Endpoint::new("catalog.example", 443).unwrap()]);
    let mut config = RelayConfig::new(RelayToken::new(TOKEN).unwrap(), endpoints, Vec::new());
    if let Some(uid) = peer {
        config = config.with_peer_uid(uid);
    }
    Relay::new(config, NoDns, NoNetwork).unwrap()
}

fn relay() -> Relay<NoDns, NoNetwork> {
    relay_for(Some(my_uid()))
}

fn control(silence_ms: u64) -> ControlConfig {
    ControlConfig {
        silence: Duration::from_millis(silence_ms),
        result_grace: Duration::from_millis(100),
    }
}

fn plan(deadline_ms: u64, host_start_ms: u64, silence_ms: u64) -> Plan<'static> {
    Plan {
        request: REQUEST,
        deadline: Instant::now() + Duration::from_millis(deadline_ms),
        slot: 1,
        class: Class::Small,
        host_start: Duration::from_millis(host_start_ms),
        control: control(silence_ms),
        kill_limit: Duration::from_secs(30),
    }
}

fn signals() -> Signals {
    Signals {
        cancelled: CancellationToken::new(),
        lease_lost: CancellationToken::new(),
    }
}

fn failed(disposition: &Disposition) -> DatabaseFailure {
    match disposition {
        Disposition::Fail(failure) => failure.code,
        other => panic!("not a failure: {other:?}"),
    }
}

/// A generous plan: nothing times out unless the test makes it.
fn easy() -> Plan<'static> {
    plan(60_000, 60_000, 60_000)
}

// -- the host answers -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_host_that_answers_with_a_receipt_completes_the_attempt() {
    let world = world();
    let sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 2,
            line: result(),
        },
    );
    let beats = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&beats);

    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
        },
    )
    .await;

    assert!(matches!(outcome.disposition, Disposition::Complete(_)));
    assert!(outcome.request_sent);
    assert_eq!(beats.load(Ordering::SeqCst), 2);
    // the host read exactly the request line the runner was given
    assert_eq!(*sandbox.request.lock().unwrap(), REQUEST);
    assert_eq!(outcome.relay_denials.total(), 0);
    assert_eq!(
        (sandbox.starts.load(Ordering::SeqCst), sandbox.kills()),
        (1, 1)
    );
}

#[tokio::test]
async fn a_failure_the_host_reports_is_reported_with_its_code() {
    let world = world();
    let sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 0,
            line: failure("ICEBERG_TABLE_NOT_FOUND"),
        },
    );
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::IcebergTableNotFound
    );
    assert_eq!(sandbox.kills(), 1);
}

#[tokio::test]
async fn the_socket_has_the_mode_of_the_decision_when_the_host_connects() {
    let world = world();
    let sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 0,
            line: result(),
        },
    );
    run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(*sandbox.socket_mode.lock().unwrap(), Some(0o660));
}

#[tokio::test]
async fn what_the_relay_refused_is_counted_and_returned() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::ProbesThenAnswers(result()));
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert!(matches!(outcome.disposition, Disposition::Complete(_)));
    assert_eq!(outcome.relay_denials.endpoint_not_declared, 1);
    assert_eq!(outcome.relay_denials.total(), 1);
}

#[tokio::test]
async fn the_relay_stops_with_the_attempt() {
    let world = world();
    let sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 0,
            line: result(),
        },
    );
    run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    // nothing listens any more: a process that finds the socket gets a refusal
    assert!(
        UnixStream::connect(world.attempt.socket_path())
            .await
            .is_err()
    );
}

// -- the host does not answer ------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_host_that_goes_silent_after_the_request_breaks_the_protocol() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::Silent);
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &plan(60_000, 60_000, 150),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::LakeHostProtocol
    );
    assert!(outcome.request_sent);
    assert_eq!(sandbox.kills(), 1);
}

#[tokio::test]
async fn a_host_that_dies_after_the_request_gave_no_result() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::DiesAfterTheRequest);
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::LakeHostProtocol
    );
    assert!(outcome.request_sent);
    assert!(!outcome.memory_killed);
}

#[tokio::test]
async fn a_host_the_memory_limit_killed_exceeded_the_limit() {
    let world = world();
    let mut sandbox = Scripted::new(world.work.path(), Script::DiesAfterTheRequest);
    sandbox.memory_killed = true;
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::IcebergLimitExceeded
    );
    assert!(outcome.memory_killed);
}

#[tokio::test]
async fn a_host_that_never_connects_is_given_the_time_it_has_and_no_request() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
    let started = StdInstant::now();
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &plan(60_000, 200, 60_000),
        &signals(),
        || {},
    )
    .await;
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::LakeHostProtocol
    );
    assert!(
        !outcome.request_sent,
        "the credentials never left the runner"
    );
    assert_eq!(sandbox.kills(), 1);
}

#[tokio::test]
async fn a_container_that_is_gone_before_it_connected_is_not_waited_for() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::ExitsAtOnce);
    let started = StdInstant::now();
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    // the host start time is a minute; the exit ended the wait at once
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::LakeHostProtocol
    );
    assert!(!outcome.request_sent);
}

#[tokio::test]
async fn a_sandbox_that_cannot_start_the_container_is_unavailable_and_nothing_is_sent() {
    for error in [
        SandboxError::Unavailable,
        SandboxError::Refused(Reason::Ceiling),
    ] {
        let world = world();
        let mut sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
        sandbox.start_fails = Some(error);
        let outcome = run_attempt(
            &sandbox,
            &world.attempt,
            relay(),
            &easy(),
            &signals(),
            || {},
        )
        .await;
        assert_eq!(
            failed(&outcome.disposition),
            DatabaseFailure::LakeSandboxUnavailable,
            "{error:?}"
        );
        assert!(!outcome.request_sent);
        // a start that failed may have left something behind: it is ended all the same
        assert_eq!(sandbox.kills(), 1);
    }
}

// -- the clock, a person and the lease ---------------------------------------------------------------------------------

/// The clock is paused once the host has the request, and the runtime then jumps to the deadline: nothing more will arrive
/// on the socket, so the jump is safe, and the test does not depend on how fast a loaded machine connects.
#[tokio::test]
async fn the_deadline_after_the_request_was_sent_is_an_unknown_outcome() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::Silent);
    let has_request = Arc::clone(&sandbox.has_request);
    tokio::spawn(async move {
        has_request.notified().await;
        tokio::time::pause();
    });
    // the deadline is an hour away and every other limit two: only the deadline can end this
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &plan(3_600_000, 7_200_000, 7_200_000),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::OutcomeUnknown
    );
    assert_eq!(outcome.ended, Ended::DeadlinePassed);
    assert!(outcome.request_sent);
    assert_eq!(sandbox.kills(), 1);
}

#[tokio::test]
async fn the_deadline_before_the_host_connected_is_a_timeout() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &plan(200, 60_000, 60_000),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(failed(&outcome.disposition), DatabaseFailure::Timeout);
    assert!(!outcome.request_sent);
}

#[tokio::test]
async fn a_cancellation_after_the_request_was_sent_is_an_unknown_outcome() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::Silent);
    let signals = signals();
    let cancel = signals.cancelled.clone();
    let has_request = Arc::clone(&sandbox.has_request);
    let canceller = tokio::spawn(async move {
        has_request.notified().await;
        cancel.cancel();
    });
    let outcome = run_attempt(&sandbox, &world.attempt, relay(), &easy(), &signals, || {}).await;
    canceller.await.unwrap();
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::OutcomeUnknown
    );
    assert!(outcome.request_sent);
}

#[tokio::test]
async fn a_cancellation_while_the_host_starts_is_a_cancellation() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
    let signals = signals();
    let cancel = signals.cancelled.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
    });
    let outcome = run_attempt(&sandbox, &world.attempt, relay(), &easy(), &signals, || {}).await;
    assert_eq!(failed(&outcome.disposition), DatabaseFailure::Cancelled);
    assert!(!outcome.request_sent);
    assert_eq!(sandbox.kills(), 1);
}

#[tokio::test]
async fn a_lease_lost_during_the_conversation_reports_nothing() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::Silent);
    let signals = signals();
    let lost = signals.lease_lost.clone();
    let has_request = Arc::clone(&sandbox.has_request);
    let loser = tokio::spawn(async move {
        has_request.notified().await;
        lost.cancel();
    });
    let outcome = run_attempt(&sandbox, &world.attempt, relay(), &easy(), &signals, || {}).await;
    loser.await.unwrap();
    assert!(matches!(outcome.disposition, Disposition::Abandon));
    assert_eq!(sandbox.kills(), 1);
}

#[tokio::test]
async fn an_attempt_that_is_over_before_it_starts_starts_nothing() {
    for (cancel, lose, expected) in [(true, false, "cancelled"), (false, true, "lost")] {
        let world = world();
        let sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
        let signals = signals();
        if cancel {
            signals.cancelled.cancel();
        }
        if lose {
            signals.lease_lost.cancel();
        }
        let outcome =
            run_attempt(&sandbox, &world.attempt, relay(), &easy(), &signals, || {}).await;
        assert_eq!(sandbox.starts.load(Ordering::SeqCst), 0, "{expected}");
        assert_eq!(sandbox.kills(), 0, "{expected}");
        assert!(!outcome.request_sent);
        match expected {
            "cancelled" => assert_eq!(failed(&outcome.disposition), DatabaseFailure::Cancelled),
            _ => assert!(matches!(outcome.disposition, Disposition::Abandon)),
        }
    }
}

/// A deadline that is past when the attempt begins is the same: no container, and a timeout. (The deadline can also pass
/// between that check and the start; `drive` reads the time left once and starts nothing with none left.)
#[tokio::test]
async fn a_deadline_that_is_past_before_the_start_starts_nothing() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
    let past = Plan {
        deadline: Instant::now() - Duration::from_millis(1),
        ..easy()
    };
    let outcome = run_attempt(&sandbox, &world.attempt, relay(), &past, &signals(), || {}).await;
    assert_eq!(sandbox.starts.load(Ordering::SeqCst), 0);
    assert_eq!(sandbox.kills(), 0);
    assert!(!outcome.request_sent);
    assert_eq!(failed(&outcome.disposition), DatabaseFailure::Timeout);
}

#[tokio::test]
async fn a_lost_lease_beats_a_cancellation_that_is_ready_with_it() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
    let signals = signals();
    signals.cancelled.cancel();
    signals.lease_lost.cancel();
    let outcome = run_attempt(&sandbox, &world.attempt, relay(), &easy(), &signals, || {}).await;
    assert!(matches!(outcome.disposition, Disposition::Abandon));
    assert_eq!(outcome.ended, Ended::LeaseLost);
}

// -- what the review asked for ----------------------------------------------------------------------------------------

/// A signal the host fires the moment it asks for `CONTROL` is seen, and the request never goes out. On this runtime the
/// signal wakes the attempt before the relay has handed the stream over, so the signal wins the wait for the host: this is
/// the end-to-end behaviour, not the race. The race (the stream and a signal ready at the same poll) is entered directly by
/// the unit tests of `attempt.rs`, which I checked fail when the protections are removed.
#[tokio::test]
async fn a_signal_that_is_ready_when_the_host_connects_stops_the_request_from_going_out() {
    for round in 0..40 {
        for lose_lease in [false, true] {
            let world = world();
            let signals = signals();
            let signal = if lose_lease {
                Signal::LoseLease(signals.lease_lost.clone())
            } else {
                Signal::Cancel(signals.cancelled.clone())
            };
            let sandbox = Scripted::new(world.work.path(), Script::ConnectsThenSignals(signal));
            let outcome =
                run_attempt(&sandbox, &world.attempt, relay(), &easy(), &signals, || {}).await;
            assert!(
                !outcome.request_sent,
                "round {round}, lease lost: {lose_lease}"
            );
            assert!(
                sandbox.request.lock().unwrap().is_empty(),
                "the host was given the request (round {round}, lease lost: {lose_lease})"
            );
            if lose_lease {
                assert!(matches!(outcome.disposition, Disposition::Abandon));
            } else {
                assert_eq!(failed(&outcome.disposition), DatabaseFailure::Cancelled);
            }
        }
    }
}

/// The clock is paused from the start: a start that hangs leaves no real I/O to wait for, so the runtime jumps to the
/// deadline when it is idle, and the deadline cannot pass before the attempt has reached `start` however loaded the machine.
#[tokio::test(start_paused = true)]
async fn a_start_that_never_returns_is_ended_by_the_deadline() {
    let world = world();
    let mut sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
    sandbox.start_hangs = true;
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &plan(200, 60_000, 60_000),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(failed(&outcome.disposition), DatabaseFailure::Timeout);
    assert!(!outcome.request_sent);
    // a start that did not return may have left a container: it is ended
    assert_eq!(sandbox.kills(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_start_that_never_returns_is_ended_by_a_cancellation_and_by_a_lost_lease() {
    for lose_lease in [false, true] {
        let world = world();
        let mut sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
        sandbox.start_hangs = true;
        let signals = signals();
        let token = if lose_lease {
            signals.lease_lost.clone()
        } else {
            signals.cancelled.clone()
        };
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        });
        let outcome =
            run_attempt(&sandbox, &world.attempt, relay(), &easy(), &signals, || {}).await;
        if lose_lease {
            assert!(matches!(outcome.disposition, Disposition::Abandon));
        } else {
            assert_eq!(failed(&outcome.disposition), DatabaseFailure::Cancelled);
        }
        assert_eq!(sandbox.kills(), 1);
    }
}

#[tokio::test]
async fn a_kill_that_never_returns_is_given_up_on_and_the_outcome_says_so() {
    let world = world();
    let mut sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 0,
            line: result(),
        },
    );
    sandbox.kill_hangs = true;
    let plan = Plan {
        kill_limit: Duration::from_millis(150),
        ..easy()
    };
    let started = StdInstant::now();
    let outcome = run_attempt(&sandbox, &world.attempt, relay(), &plan, &signals(), || {}).await;
    assert!(started.elapsed() < Duration::from_secs(20));
    // the receipt is the host's and stands; that the container may still be up is said, not hidden
    assert!(matches!(outcome.disposition, Disposition::Complete(_)));
    assert!(!outcome.container_ended);
}

#[tokio::test]
async fn a_kill_the_sandbox_cannot_confirm_is_said() {
    let world = world();
    let mut sandbox = Scripted::new(world.work.path(), Script::DiesAfterTheRequest);
    sandbox.kill_unconfirmed = true;
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert!(!outcome.container_ended);
}

#[tokio::test]
async fn a_container_that_was_ended_is_said_to_be() {
    let world = world();
    let sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 0,
            line: result(),
        },
    );
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert!(outcome.container_ended);
}

/// The relay is the container's only route out, so it stops first: a container that is being ended, or that cannot be, has
/// no tunnels left.
#[tokio::test]
async fn the_relay_is_stopped_before_the_container_is_ended() {
    let world = world();
    let sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 0,
            line: result(),
        },
    );
    run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(*sandbox.relay_up_in_kill.lock().unwrap(), Some(false));
}

#[tokio::test]
async fn a_lease_lost_while_the_container_is_being_ended_reports_nothing() {
    let world = world();
    let signals = signals();
    let mut sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 0,
            line: result(),
        },
    );
    sandbox.lose_lease_in_kill = Some(signals.lease_lost.clone());
    let outcome = run_attempt(&sandbox, &world.attempt, relay(), &easy(), &signals, || {}).await;
    assert!(matches!(outcome.disposition, Disposition::Abandon));
    assert_eq!(outcome.ended, Ended::LeaseLost);
}

/// Dropping the call (the caller's own deadline, a panic) must not leave the relay serving an unlinked socket. The
/// container is the sandbox's to end at its deadline; the relay is ours.
#[tokio::test]
async fn dropping_the_attempt_midway_stops_the_relay() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
    let (plan, signals) = (easy(), signals());
    tokio::select! {
        biased;
        _ = run_attempt(&sandbox, &world.attempt, relay(), &plan, &signals, || {}) => {
            panic!("the attempt ended by itself")
        }
        () = async {} => {}
    }
    let mut stopped = false;
    for _ in 0..500 {
        if UnixStream::connect(world.attempt.socket_path())
            .await
            .is_err()
        {
            stopped = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        stopped,
        "the relay was still serving after the attempt was dropped"
    );
}

#[tokio::test]
async fn a_relay_that_does_not_check_its_peer_is_not_run() {
    let world = world();
    let sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 0,
            line: result(),
        },
    );
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay_for(None),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::LakeSandboxUnavailable
    );
    assert_eq!(sandbox.starts.load(Ordering::SeqCst), 0);
    assert!(!outcome.request_sent);
}

#[tokio::test]
async fn a_process_of_another_user_that_has_the_token_is_not_given_the_request() {
    let world = world();
    // the kernel says the scripted host runs as `my_uid()`; the relay is told to serve another user only
    let sandbox = Scripted::new(world.work.path(), Script::Silent);
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay_for(Some(my_uid() + 1)),
        &plan(60_000, 300, 60_000),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::LakeHostProtocol
    );
    assert!(!outcome.request_sent);
    assert!(sandbox.request.lock().unwrap().is_empty());
    assert_eq!(outcome.relay_denials.token_refused, 1);
}

#[tokio::test]
async fn the_plan_shows_the_size_of_the_request_and_not_the_request() {
    let shown = format!("{:?}", easy());
    assert!(!shown.contains("icebergWrite"), "{shown}");
    assert!(shown.contains(&REQUEST.len().to_string()), "{shown}");
}

/// The host chooses the column of a failure. `dispose` keeps it only if it is a plain name, and the outcome must not hand
/// the raw text back through `Debug`.
#[tokio::test]
async fn the_outcome_shows_no_text_of_the_hosts_and_no_secret() {
    let world = world();
    let hostile = line(&json!({
        "failure": {"code": "ICEBERG_SCHEMA_MISMATCH", "column": "YWJj+/=ZGVm", "reason": "TYPE"},
        "relayDenials": 0
    }));
    let sandbox = Scripted::new(
        world.work.path(),
        Script::Answer {
            beats: 0,
            line: hostile,
        },
    );
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    let shown = format!("{outcome:?}");
    assert!(!shown.contains("YWJj"), "{shown}");
    assert!(!shown.contains(TOKEN), "{shown}");
    // what is reported is still there, with the column dropped
    match &outcome.disposition {
        Disposition::Fail(failure) => assert_eq!(failure.column, None),
        other => panic!("{other:?}"),
    }
}

/// The sandbox is asked for the slot and the class of the plan and for nothing else: the class is the only thing of the budget
/// that the lake runner chooses, and the slot is the user the container runs as. That the relay admits **that** user is the
/// driver's to build (`with_peer_uid(uid_for_slot(slot))`) and its to test: this case, whose host connects as the user of the
/// test, does not show it.
#[tokio::test]
async fn the_sandbox_is_asked_for_the_slot_and_the_class_of_the_plan() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::ExitsAtOnce);
    let plan = Plan {
        slot: 4095,
        class: Class::Large,
        ..easy()
    };
    let _ = run_attempt(&sandbox, &world.attempt, relay(), &plan, &signals(), || {}).await;
    let asked = sandbox.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].slot, 4095);
    assert_eq!(asked[0].class, Class::Large);
    assert_eq!(asked[0].attempt_id, world.attempt.id());
    assert!(asked[0].deadline > Duration::ZERO);
}

/// A refusal of the sandbox keeps its reason in the error, for the driver's metrics and the failure's detail, and ends the
/// attempt as the sandbox being unavailable (retryable) whatever the reason.
#[tokio::test]
async fn a_refusal_of_the_sandbox_keeps_its_reason_and_ends_the_attempt_as_unavailable() {
    for reason in [
        Reason::Ceiling,
        Reason::Slot,
        Reason::Class,
        Reason::Deadline,
        Reason::Directory,
    ] {
        let error = SandboxError::Refused(reason);
        assert!(error.to_string().contains(reason.as_str()), "{error}");
        let world = world();
        let mut sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
        sandbox.start_fails = Some(error);
        let outcome = run_attempt(
            &sandbox,
            &world.attempt,
            relay(),
            &easy(),
            &signals(),
            || {},
        )
        .await;
        assert_eq!(outcome.ended, Ended::SandboxUnavailable, "{reason:?}");
        assert_eq!(outcome.sandbox_refusal, Some(reason), "{reason:?}");
    }
    // an outage is not a refusal
    let world = world();
    let mut sandbox = Scripted::new(world.work.path(), Script::NeverConnects);
    sandbox.start_fails = Some(SandboxError::Unavailable);
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        &easy(),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(outcome.ended, Ended::SandboxUnavailable);
    assert_eq!(outcome.sandbox_refusal, None);
}

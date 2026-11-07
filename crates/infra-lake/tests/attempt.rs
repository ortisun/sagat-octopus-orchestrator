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

/// What the scripted host does once it is connected, or instead of connecting.
#[derive(Clone)]
enum Script {
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
    memory_killed: bool,
    starts: AtomicUsize,
    kills: AtomicUsize,
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
            memory_killed: false,
            starts: AtomicUsize::new(0),
            kills: AtomicUsize::new(0),
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
        Script::Silent => std::future::pending().await,
        Script::DiesAfterTheRequest | Script::NeverConnects | Script::ExitsAtOnce => {}
    }
}

impl LakeSandbox for Scripted {
    async fn start(&self, request: &StartRequest) -> Result<(), SandboxError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.start_fails {
            return Err(error);
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

    async fn kill(&self, _attempt_id: &str) -> ContainerExit {
        self.kills.fetch_add(1, Ordering::SeqCst);
        if let Some(task) = self.host.lock().unwrap().take() {
            task.abort();
        }
        ContainerExit {
            memory_killed: self.memory_killed,
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

fn relay() -> Relay<NoDns, NoNetwork> {
    let endpoints = EndpointPolicy::new([Endpoint::new("catalog.example", 443).unwrap()]);
    let config = RelayConfig::new(RelayToken::new(TOKEN).unwrap(), endpoints, Vec::new());
    Relay::new(config, NoDns, NoNetwork).unwrap()
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
        host_start: Duration::from_millis(host_start_ms),
        control: control(silence_ms),
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
    for error in [SandboxError::Unavailable, SandboxError::Refused] {
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

#[tokio::test]
async fn the_deadline_after_the_request_was_sent_is_an_unknown_outcome() {
    let world = world();
    let sandbox = Scripted::new(world.work.path(), Script::Silent);
    let outcome = run_attempt(
        &sandbox,
        &world.attempt,
        relay(),
        // long enough for the host to connect and read its request on a loaded machine; the deadline ends the rest
        &plan(2_000, 60_000, 60_000),
        &signals(),
        || {},
    )
    .await;
    assert_eq!(
        failed(&outcome.disposition),
        DatabaseFailure::OutcomeUnknown
    );
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

// -- what the outcome shows -------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_outcome_shows_no_secret_and_no_request() {
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
    let shown = format!("{outcome:?}");
    assert!(!shown.contains(TOKEN));
    assert!(!shown.contains("icebergWrite"));
}

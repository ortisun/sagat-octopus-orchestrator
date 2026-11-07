//! One lake attempt, from a lease's input to what is reported (feature 035, T035-035, D035-123 slice C).
//!
//! The driver is the join: it reads the lease input ([`AttemptInput`]), takes a slot, prepares the attempt
//! ([`prepare`](super::prepare::prepare)), builds the relay for the user of the slot, runs the attempt behind the sandbox port
//! ([`run_attempt`]) and says what to report. It does not talk to the control plane: the connection it is given is already
//! resolved (two-phase, fenced and audited by the server), the signals it is given are the lease's cancellation and loss, and
//! the report it returns is for the caller to send. That keeps what is decided here (which code, whether the directory may be
//! removed, whether the slot is free again) testable against a scripted host on the real relay socket.
//!
//! **What this decides.**
//!
//! - *Before anything is made:* an input that is not an attempt input, no free slot and a slot with no user fail with their
//!   code. The deadline is the least of the lease's end and the step's own timeout (the strictest of the step's and the
//!   connection's, [`effective`]).
//! - *The class:* an attempt whose staged input is above `largeAboveBytes` asks for the `large` class, when the settings name one.
//! - *After the attempt:* the request, which holds the credentials, is dropped (zeroised) the moment the attempt is over. The
//!   directory is removed **only if the sandbox confirmed the container gone**; if it did not, the container may still have it
//!   mounted and its credentials are good until its deadline, so the directory is released to the sweep and the caller is told
//!   where it is.
//! - *The slot* is free again when the attempt is over, whatever the outcome (the sandbox refuses a slot that is still in use).
//!
//! A start refused for the slot is **not** retried here with another one: the code is retryable, the step is retried by the
//! server, and the next attempt takes the next slot (D035-126).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use sagat_domain::database::DatabaseFailure;
use sagat_infra_lake::attempt::{LakeSandbox, Plan, Signals, run_attempt};
use sagat_infra_lake::control::ControlConfig;
use sagat_infra_lake::protocol::{AttemptInput, Receipt};
use sagat_infra_lake::relay::{Connector, Relay, RelayDenials, Resolver};
use sagat_infra_lake::sandbox_api::{Class, Reason, uid_for_slot};
use sagat_infra_lake::supervisor::{Disposition, Failure};
use tokio::time::Instant;

use super::connection::Converted;
use super::prepare::{Attempt, Blobs, Prepared, effective, prepare};
use super::settings::Settings;

/// The slots of the lake runner: which of the containers' users are in use. A slot is taken for the whole attempt and is
/// handed back when the guard is dropped. Taken in turn, not always the lowest: the user of the container that has just
/// ended is not the user of the next one.
#[derive(Debug)]
pub struct SlotPool {
    count: u32,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    taken: BTreeSet<u32>,
    next: u32,
}

impl SlotPool {
    #[must_use]
    pub fn new(count: u32) -> Arc<Self> {
        Arc::new(Self {
            count,
            state: Mutex::new(State::default()),
        })
    }

    /// The next free slot, or `None` when every one is in use.
    #[must_use]
    pub fn take(self: &Arc<Self>) -> Option<SlotGuard> {
        let mut state = self.state.lock().ok()?;
        let slot = (0..self.count)
            .map(|offset| (state.next + offset) % self.count)
            .find(|slot| !state.taken.contains(slot))?;
        state.taken.insert(slot);
        state.next = (slot + 1) % self.count;
        Some(SlotGuard {
            pool: Arc::clone(self),
            slot,
        })
    }

    /// How many are in use.
    #[must_use]
    pub fn in_use(&self) -> usize {
        self.state.lock().map_or(0, |state| state.taken.len())
    }
}

/// A slot taken from the pool.
#[derive(Debug)]
pub struct SlotGuard {
    pool: Arc<SlotPool>,
    slot: u32,
}

impl SlotGuard {
    #[must_use]
    pub const fn number(&self) -> u32 {
        self.slot
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.pool.state.lock() {
            state.taken.remove(&self.slot);
        }
    }
}

/// What to tell the control plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// `CompleteStep` with this receipt, rebuilt from the host's typed fields.
    Complete(Box<Receipt>),
    /// `FailStep`.
    Fail(Failure),
    /// Nothing: the lease is gone.
    Abandon,
}

/// How an attempt ended, for the report and for the runner's own books.
#[derive(Debug)]
pub struct Finished {
    pub report: Report,
    /// The request line was written to the host. Before that, nothing can have been written to the table.
    pub request_sent: bool,
    /// The sandbox confirmed the container gone (`true` when there was none).
    pub container_ended: bool,
    /// What the relay refused.
    pub relay_denials: RelayDenials,
    /// Why the sandbox said no, when it answered and said no.
    pub sandbox_refusal: Option<Reason>,
    /// Where the attempt directory was left, when it could not be removed: because the container was not confirmed gone, or
    /// because something in it could not be removed. The caller counts it and the sweep takes it.
    pub left: Option<PathBuf>,
}

impl Finished {
    fn failed(code: DatabaseFailure) -> Self {
        Self {
            report: Report::Fail(Failure {
                code,
                column: None,
                reason: None,
            }),
            request_sent: false,
            container_ended: true,
            relay_denials: RelayDenials::default(),
            sandbox_refusal: None,
            left: None,
        }
    }
}

/// The lease, as far as the driver needs it.
#[derive(Debug, Clone, Copy)]
pub struct Job<'a> {
    /// The `input_json` of the lease.
    pub input_json: &'a [u8],
    /// When the lease ends: the attempt is over by then whatever the step says.
    pub deadline: Instant,
}

/// The lake driver of one runner.
pub struct Driver<S, B, R, C> {
    pub sandbox: S,
    pub blobs: B,
    pub resolver: R,
    pub connector: C,
    pub settings: Settings,
    pub development: bool,
    pub slots: Arc<SlotPool>,
    /// The user of the container in a slot: [`uid_for_slot`] in production, the test's own user in a test (the scripted host
    /// is in the test's process, so the kernel reports that user as the peer).
    pub peer_for_slot: fn(u32) -> Option<u32>,
}

impl<S, B, R, C> Driver<S, B, R, C>
where
    S: LakeSandbox,
    B: Blobs,
    R: Resolver + Clone,
    C: Connector + Clone,
{
    /// A driver for a production runner: the containers' users are `40000 + slot`, and the pool is as large as the settings say.
    #[must_use]
    pub fn new(
        sandbox: S,
        blobs: B,
        resolver: R,
        connector: C,
        settings: Settings,
        development: bool,
    ) -> Self {
        let slots = SlotPool::new(settings.slots);
        Self {
            sandbox,
            blobs,
            resolver,
            connector,
            settings,
            development,
            slots,
            peer_for_slot: uid_for_slot,
        }
    }

    /// Runs one attempt (see the module). `connection` is the resolved connection of the lease; it is consumed.
    /// `on_heartbeat` is called once for each beat the host sent in time and must not block.
    pub async fn run(
        &self,
        job: &Job<'_>,
        connection: Converted,
        signals: &Signals,
        on_heartbeat: impl FnMut() + Send,
    ) -> Finished {
        let Ok(input) = AttemptInput::read(job.input_json) else {
            return Finished::failed(DatabaseFailure::Configuration);
        };
        let limits = effective(input.limits(), connection.limits, &self.settings);
        let Some(slot) = self.slots.take() else {
            return Finished::failed(DatabaseFailure::LakeSandboxUnavailable);
        };
        let Some(peer_uid) = (self.peer_for_slot)(slot.number()) else {
            return Finished::failed(DatabaseFailure::Configuration);
        };

        let now = Instant::now();
        let deadline = limits
            .timeout
            .map_or(job.deadline, |timeout| job.deadline.min(now + timeout));
        let deadline_at = SystemTime::now() + deadline.saturating_duration_since(now);
        // a fresh id: lower-case hexadecimal digits and hyphens, which is what the directory and the sandbox accept
        let attempt_id = uuid::Uuid::now_v7().to_string();

        let prepared = prepare(
            &input,
            connection,
            &self.blobs,
            &self.settings,
            Attempt {
                id: &attempt_id,
                peer_uid,
                deadline_at,
                development: self.development,
            },
        )
        .await;
        let Prepared {
            mut directory,
            request,
            relay,
            staged_bytes,
        } = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return Finished::failed(error.code()),
        };
        let relay = match Relay::new(relay, self.resolver.clone(), self.connector.clone()) {
            Ok(relay) => relay,
            Err(_) => return Finished::failed(DatabaseFailure::Configuration),
        };
        let class = if self
            .settings
            .large_above_bytes
            .is_some_and(|above| staged_bytes > above)
        {
            Class::Large
        } else {
            Class::Small
        };

        let plan = Plan {
            request: &request,
            deadline,
            slot: slot.number(),
            class,
            host_start: self.settings.host_start,
            control: ControlConfig::default(),
            kill_limit: self.settings.kill_limit,
        };
        let outcome = run_attempt(
            &self.sandbox,
            &directory,
            relay,
            &plan,
            signals,
            on_heartbeat,
        )
        .await;
        // the request holds the credentials: it goes as soon as the attempt is over
        drop(request);

        let left = if outcome.container_ended {
            match directory.remove() {
                Ok(()) => None,
                // the handle is still ours; `Drop` tries once more, and what it cannot remove is the sweep's
                Err(_) => Some(directory.release()),
            }
        } else {
            Some(directory.release())
        };
        drop(slot);

        let report = match outcome.disposition {
            Disposition::Complete(receipt) => Report::Complete(receipt),
            Disposition::Fail(failure) => Report::Fail(failure),
            Disposition::Abandon => Report::Abandon,
        };
        Finished {
            report,
            request_sent: outcome.request_sent,
            container_ended: outcome.container_ended,
            relay_denials: outcome.relay_denials,
            sandbox_refusal: outcome.sandbox_refusal,
            left,
        }
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::io;
    use std::net::{IpAddr, SocketAddr};
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use sagat_infra_lake::attempt::{ContainerExit, SandboxError, StartRequest};
    use serde_json::{Value, json};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream};
    use tokio::net::UnixStream;
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;

    use super::super::fixtures::{converted, input_json, message, settings};
    use super::*;

    // -- the doubles ---------------------------------------------------------------------------------------------------

    #[derive(Clone)]
    struct NoDns;

    impl Resolver for NoDns {
        async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<IpAddr>> {
            Err(io::Error::other("no name server in this test"))
        }
    }

    #[derive(Clone)]
    struct NoNetwork;

    impl Connector for NoNetwork {
        type Stream = DuplexStream;

        async fn connect(&self, _address: SocketAddr) -> io::Result<DuplexStream> {
            Err(io::Error::other("no network in this test"))
        }
    }

    struct NoBlobs;

    impl Blobs for NoBlobs {
        type Reader = io::Cursor<Vec<u8>>;

        async fn open(&self, _blob: sagat_domain::blobs::BlobId) -> Result<Self::Reader, ()> {
            Err(())
        }
    }

    /// What the scripted host does once the sandbox "starts" it.
    #[derive(Clone)]
    enum Script {
        /// Connects, reads the request, sends this many beats and then this line, and closes.
        Answer { beats: usize, line: Vec<u8> },
        /// Is started and never connects.
        NeverConnects,
    }

    struct Scripted {
        work: std::path::PathBuf,
        script: Script,
        start_error: Option<SandboxError>,
        kill_unconfirmed: bool,
        asked: Mutex<Vec<StartRequest>>,
        kills: AtomicUsize,
        host: Mutex<Option<JoinHandle<()>>>,
        request: Arc<Mutex<Vec<u8>>>,
    }

    impl Scripted {
        fn new(work: &Path, script: Script) -> Self {
            Self {
                work: work.to_path_buf(),
                script,
                start_error: None,
                kill_unconfirmed: false,
                asked: Mutex::new(Vec::new()),
                kills: AtomicUsize::new(0),
                host: Mutex::new(None),
                request: Arc::default(),
            }
        }

        fn asked(&self) -> Vec<StartRequest> {
            self.asked.lock().unwrap().clone()
        }
    }

    /// The host at the wire, as `crates/infra-lake/tests/attempt.rs` plays it: the token from the file in the attempt
    /// directory, the relay socket, `CONTROL`, the request line, then one ending.
    async fn host(
        work: std::path::PathBuf,
        id: String,
        script: Script,
        request: Arc<Mutex<Vec<u8>>>,
    ) {
        let Script::Answer { beats, line } = script else {
            return std::future::pending().await;
        };
        let directory = work.join(&id);
        let token = std::fs::read_to_string(directory.join("relay.token")).unwrap();
        let mut stream = BufReader::new(
            UnixStream::connect(directory.join("relay.sock"))
                .await
                .unwrap(),
        );
        stream
            .get_mut()
            .write_all(format!("SAGAT-RELAY/1 {}\nCONTROL\n", token.trim_end()).as_bytes())
            .await
            .unwrap();
        let mut asked = Vec::new();
        stream.read_until(b'\n', &mut asked).await.unwrap();
        *request.lock().unwrap() = asked;
        for _ in 0..beats {
            stream
                .get_mut()
                .write_all(b"{\"heartbeat\":true}\n")
                .await
                .unwrap();
        }
        stream.get_mut().write_all(&line).await.unwrap();
        stream.get_mut().shutdown().await.unwrap();
    }

    impl LakeSandbox for Scripted {
        async fn start(&self, request: &StartRequest) -> Result<(), SandboxError> {
            self.asked.lock().unwrap().push(request.clone());
            if let Some(error) = self.start_error {
                return Err(error);
            }
            let task = tokio::spawn(host(
                self.work.clone(),
                request.attempt_id.clone(),
                self.script.clone(),
                Arc::clone(&self.request),
            ));
            *self.host.lock().unwrap() = Some(task);
            Ok(())
        }

        async fn exited(&self, _attempt_id: &str) {
            std::future::pending().await
        }

        async fn kill(&self, _attempt_id: &str) -> ContainerExit {
            self.kills.fetch_add(1, Ordering::SeqCst);
            if let Some(task) = self.host.lock().unwrap().take() {
                task.abort();
            }
            ContainerExit {
                memory_killed: false,
                ended: !self.kill_unconfirmed,
            }
        }
    }

    // -- the fixture ---------------------------------------------------------------------------------------------------

    /// The user this test runs as: the scripted host is in this process, so the kernel reports it as the peer.
    fn my_uid() -> u32 {
        use std::os::unix::fs::MetadataExt as _;
        tempfile::tempfile().unwrap().metadata().unwrap().uid()
    }

    fn receipt_line() -> Vec<u8> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/contract/iceberg-writes/fixtures/receipts/iceberg-committed.json");
        let receipt: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut line = serde_json::to_vec(&json!({"receipt": receipt, "relayDenials": 0})).unwrap();
        line.push(b'\n');
        line
    }

    fn failure_line(code: &str, column: &str) -> Vec<u8> {
        let mut line = serde_json::to_vec(
            &json!({"failure": {"code": code, "column": column}, "relayDenials": 0}),
        )
        .unwrap();
        line.push(b'\n');
        line
    }

    fn answer(line: Vec<u8>) -> Script {
        Script::Answer { beats: 0, line }
    }

    struct World {
        // a short path: a Unix socket's path is limited to about a hundred bytes
        work: tempfile::TempDir,
    }

    fn world() -> World {
        World {
            work: tempfile::Builder::new()
                .prefix("dr")
                .tempdir_in("/tmp")
                .unwrap(),
        }
    }

    fn driver(world: &World, script: Script) -> Driver<Scripted, NoBlobs, NoDns, NoNetwork> {
        Driver {
            sandbox: Scripted::new(world.work.path(), script),
            blobs: NoBlobs,
            resolver: NoDns,
            connector: NoNetwork,
            settings: settings(world.work.path()),
            development: false,
            slots: SlotPool::new(4),
            peer_for_slot: |_| Some(my_uid()),
        }
    }

    fn signals() -> Signals {
        Signals {
            cancelled: CancellationToken::new(),
            lease_lost: CancellationToken::new(),
        }
    }

    fn entries(directory: &Path) -> Vec<String> {
        std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    async fn run(
        driver: &Driver<Scripted, NoBlobs, NoDns, NoNetwork>,
        input: &[u8],
        lease: Duration,
        signals: &Signals,
    ) -> Finished {
        let job = Job {
            input_json: input,
            deadline: Instant::now() + lease,
        };
        driver.run(&job, converted(), signals, || {}).await
    }

    fn failed_with(finished: &Finished) -> DatabaseFailure {
        match &finished.report {
            Report::Fail(failure) => failure.code,
            other => panic!("not a failure: {other:?}"),
        }
    }

    // -- the pool ------------------------------------------------------------------------------------------------------

    #[test]
    fn slots_are_taken_in_turn_and_come_back_when_the_guard_goes() {
        let pool = SlotPool::new(3);
        let first = pool.take().unwrap();
        assert_eq!(first.number(), 0);
        drop(first);
        // the user of the container that has just ended is not the user of the next one
        assert_eq!(pool.take().unwrap().number(), 1);
        assert_eq!(pool.in_use(), 0, "the guard of that take was dropped");

        let (a, b, c) = (
            pool.take().unwrap(),
            pool.take().unwrap(),
            pool.take().unwrap(),
        );
        assert_eq!(
            [a.number(), b.number(), c.number()]
                .iter()
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        assert!(pool.take().is_none(), "all three are in use");
        assert_eq!(pool.in_use(), 3);
        drop(b);
        assert!(pool.take().is_some());
    }

    #[test]
    fn a_production_driver_gives_the_container_the_user_of_its_slot() {
        let world = world();
        let production = Driver::new(
            Scripted::new(world.work.path(), Script::NeverConnects),
            NoBlobs,
            NoDns,
            NoNetwork,
            settings(world.work.path()),
            false,
        );
        assert_eq!((production.peer_for_slot)(3), Some(40_003));
        assert_eq!((production.peer_for_slot)(4096), None);
        assert_eq!(production.slots.in_use(), 0);
    }

    // -- one attempt ---------------------------------------------------------------------------------------------------

    #[tokio::test]
    async fn an_attempt_that_commits_reports_the_receipt_and_leaves_nothing_behind() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let finished = run(
            &driver,
            &input_json(|_| {}),
            Duration::from_secs(60),
            &signals(),
        )
        .await;

        match &finished.report {
            Report::Complete(receipt) => assert!(format!("{receipt:?}").contains("Iceberg")),
            other => panic!("not complete: {other:?}"),
        }
        assert!(finished.request_sent);
        assert!(finished.container_ended);
        assert_eq!(finished.left, None);
        assert!(
            entries(world.work.path()).is_empty(),
            "the directory is gone"
        );
        assert_eq!(driver.slots.in_use(), 0, "the slot is free again");
        assert_eq!(driver.sandbox.kills.load(Ordering::SeqCst), 1);

        // what the host was sent is the request of the attempt
        let request: Value =
            serde_json::from_slice(&driver.sandbox.request.lock().unwrap()).unwrap();
        assert_eq!(request["effectKey"], "run-7f3a/load/1");
        assert_eq!(request["table"]["name"], "orders");
        // and the sandbox was asked for the slot, the class and the id of the attempt, and for nothing else
        let asked = driver.sandbox.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].slot, 0);
        assert_eq!(asked[0].class, Class::Small);
        assert!(uuid::Uuid::parse_str(&asked[0].attempt_id).is_ok());
    }

    #[tokio::test]
    async fn a_failure_of_the_host_is_reported_with_its_code_and_column() {
        let world = world();
        let driver = driver(
            &world,
            answer(failure_line("ICEBERG_SCHEMA_MISMATCH", "amount")),
        );
        let finished = run(
            &driver,
            &input_json(|_| {}),
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        let Report::Fail(failure) = &finished.report else {
            panic!("not a failure: {:?}", finished.report);
        };
        assert_eq!(failure.code, DatabaseFailure::IcebergSchemaMismatch);
        assert_eq!(failure.column.as_deref(), Some("amount"));
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn the_heartbeats_of_the_host_reach_the_caller() {
        let world = world();
        let driver = driver(
            &world,
            Script::Answer {
                beats: 1,
                line: receipt_line(),
            },
        );
        let beats = AtomicUsize::new(0);
        let job = Job {
            input_json: &input_json(|_| {}),
            deadline: Instant::now() + Duration::from_secs(60),
        };
        let finished = driver
            .run(&job, converted(), &signals(), || {
                beats.fetch_add(1, Ordering::SeqCst);
            })
            .await;
        assert!(matches!(finished.report, Report::Complete(_)));
        assert_eq!(beats.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_input_that_is_not_an_attempt_input_fails_before_anything_is_taken_or_made() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let finished = run(
            &driver,
            b"{\"not\":\"an input\"}",
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Configuration);
        assert!(driver.sandbox.asked().is_empty());
        assert_eq!(driver.slots.in_use(), 0);
        assert!(entries(world.work.path()).is_empty());
    }

    #[tokio::test]
    async fn with_no_free_slot_the_attempt_fails_retryably_and_the_sandbox_is_not_asked() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let _all: Vec<_> = (0..4).map(|_| driver.slots.take().unwrap()).collect();
        let finished = run(
            &driver,
            &input_json(|_| {}),
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        assert_eq!(
            failed_with(&finished),
            DatabaseFailure::LakeSandboxUnavailable
        );
        assert!(DatabaseFailure::LakeSandboxUnavailable.retryable());
        assert!(driver.sandbox.asked().is_empty());
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(
            driver.slots.in_use(),
            4,
            "the slots that were taken are still taken"
        );
    }

    #[tokio::test]
    async fn a_slot_with_no_user_fails_as_a_configuration_and_gives_the_slot_back() {
        let world = world();
        let mut driver = driver(&world, answer(receipt_line()));
        driver.peer_for_slot = |_| None;
        let finished = run(
            &driver,
            &input_json(|_| {}),
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Configuration);
        assert_eq!(driver.slots.in_use(), 0);
        assert!(driver.sandbox.asked().is_empty());
    }

    #[tokio::test]
    async fn a_sandbox_that_cannot_start_the_container_is_unavailable_and_nothing_stays() {
        let world = world();
        let mut driver = driver(&world, answer(receipt_line()));
        driver.sandbox.start_error = Some(SandboxError::Unavailable);
        let finished = run(
            &driver,
            &input_json(|_| {}),
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        assert_eq!(
            failed_with(&finished),
            DatabaseFailure::LakeSandboxUnavailable
        );
        assert!(
            !finished.request_sent,
            "the host was never given the credentials"
        );
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn a_refusal_of_the_sandbox_keeps_its_reason_and_is_the_same_code() {
        let world = world();
        let mut driver = driver(&world, answer(receipt_line()));
        driver.sandbox.start_error = Some(SandboxError::Refused(Reason::Ceiling));
        let finished = run(
            &driver,
            &input_json(|_| {}),
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        assert_eq!(
            failed_with(&finished),
            DatabaseFailure::LakeSandboxUnavailable
        );
        assert_eq!(finished.sandbox_refusal, Some(Reason::Ceiling));
    }

    #[tokio::test]
    async fn a_container_the_sandbox_did_not_confirm_gone_leaves_its_directory_to_the_sweep() {
        let world = world();
        let mut driver = driver(&world, answer(receipt_line()));
        driver.sandbox.kill_unconfirmed = true;
        let finished = run(
            &driver,
            &input_json(|_| {}),
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        assert!(!finished.container_ended);
        let left = finished.left.expect("the directory is left where it is");
        assert!(
            left.join("relay.token").is_file(),
            "the container may still be using it"
        );
        assert_eq!(entries(world.work.path()).len(), 1);
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn an_attempt_cancelled_before_it_starts_starts_nothing_and_leaves_nothing() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let signals = signals();
        signals.cancelled.cancel();
        let finished = run(
            &driver,
            &input_json(|_| {}),
            Duration::from_secs(60),
            &signals,
        )
        .await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Cancelled);
        assert!(driver.sandbox.asked().is_empty());
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn a_lost_lease_reports_nothing() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let signals = signals();
        signals.lease_lost.cancel();
        let finished = run(
            &driver,
            &input_json(|_| {}),
            Duration::from_secs(60),
            &signals,
        )
        .await;
        assert_eq!(finished.report, Report::Abandon);
        assert!(entries(world.work.path()).is_empty());
    }

    #[tokio::test]
    async fn the_container_gets_the_least_of_the_lease_and_the_timeout_of_the_step() {
        let world = world();
        // the step says two seconds and the lease an hour: the step's is the deadline
        let driver_a = driver(&world, answer(receipt_line()));
        let by_step = input_json(|d| d["limits"]["timeoutMs"] = json!(2000));
        run(&driver_a, &by_step, Duration::from_secs(3600), &signals()).await;
        let asked = driver_a.sandbox.asked();
        assert!(
            asked[0].deadline <= Duration::from_secs(2),
            "{:?}",
            asked[0].deadline
        );
        assert!(
            asked[0].deadline > Duration::from_millis(1000),
            "{:?}",
            asked[0].deadline
        );

        // the lease says half a second and the step an hour: the lease's is
        let driver_b = driver(&world, answer(receipt_line()));
        let by_lease = input_json(|d| d["limits"]["timeoutMs"] = json!(3_600_000));
        run(&driver_b, &by_lease, Duration::from_millis(500), &signals()).await;
        let asked = driver_b.sandbox.asked();
        assert!(
            asked[0].deadline <= Duration::from_millis(500),
            "{:?}",
            asked[0].deadline
        );
    }

    #[tokio::test]
    async fn an_attempt_above_the_threshold_asks_for_the_large_class_and_one_below_does_not() {
        let world = world();
        let records = json!({"inline": [{"a": "xxxxxxxxxxxxxxxxxxxx"}]});
        let mut with_large = driver(&world, answer(receipt_line()));
        with_large.settings.large_above_bytes = Some(10);
        run(
            &with_large,
            &input_json(|d| d["records"] = records.clone()),
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        assert_eq!(with_large.sandbox.asked()[0].class, Class::Large);

        let mut roomy = driver(&world, answer(receipt_line()));
        roomy.settings.large_above_bytes = Some(10_000);
        run(
            &roomy,
            &input_json(|d| d["records"] = records.clone()),
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        assert_eq!(roomy.sandbox.asked()[0].class, Class::Small);

        let none = driver(&world, answer(receipt_line()));
        run(
            &none,
            &input_json(|d| d["records"] = records.clone()),
            Duration::from_secs(60),
            &signals(),
        )
        .await;
        assert_eq!(
            none.sandbox.asked()[0].class,
            Class::Small,
            "no threshold, no large class"
        );
    }

    #[tokio::test]
    async fn the_threshold_itself_is_still_the_small_class_and_one_byte_over_is_not() {
        let world = world();
        let record = json!({"a": "xxxxxxxxxxxxxxxxxxxx"});
        // a record is one line: its compact text and a newline
        let staged = serde_json::to_vec(&record).unwrap().len() as u64 + 1;
        let input = input_json(|d| d["records"] = json!({"inline": [record.clone()]}));
        for (threshold, class) in [(staged, Class::Small), (staged - 1, Class::Large)] {
            let mut driver = driver(&world, answer(receipt_line()));
            driver.settings.large_above_bytes = Some(threshold);
            run(&driver, &input, Duration::from_secs(60), &signals()).await;
            assert_eq!(
                driver.sandbox.asked()[0].class,
                class,
                "threshold {threshold}"
            );
        }
    }

    #[tokio::test]
    async fn a_connection_that_does_not_allow_the_mode_fails_before_the_sandbox_is_asked() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let mut message = message();
        message.allowed_modes = vec!["overwrite".into()];
        let job = Job {
            input_json: &input_json(|_| {}),
            deadline: Instant::now() + Duration::from_secs(60),
        };
        let finished = driver
            .run(
                &job,
                super::super::connection::convert(&mut message).unwrap(),
                &signals(),
                || {},
            )
            .await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Configuration);
        assert!(driver.sandbox.asked().is_empty());
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }
}

//! The client of the lake runner against the server of the sandbox runner, over a real socket (feature 035, ADR-0079 rule 14):
//! a client and a server that disagree fail here, and a server that is wrong, slow or gone is read the way the rule says.
//!
//! The wire itself is held to one corpus on each side (`tests/contract/iceberg-writes/fixtures/sandbox-api/cases.json`); these
//! cases are the calls: what the client sends, what it makes of what comes back, what it does when nothing does.

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sagat_api_http::{ServerConfig, serve_unix};
use sagat_infra_lake::attempt::{ContainerExit, LakeSandbox, SandboxError, StartRequest};
use sagat_infra_lake::sandbox_api::client::SocketSandbox;
use sagat_infra_lake::sandbox_api::{Class, Reason, StartBody};
use tokio::net::UnixListener;
use tokio::sync::oneshot;

use super::admission::Admission;
use super::api::router;
use super::config::LakeConfig;
use super::engine::fake::Fake;

const NOW: u64 = 1_800_000_000;
const BOUND: Duration = Duration::from_secs(10);

/// The sandbox runner's server over a real socket, with a fake engine behind it.
struct Harness {
    socket: PathBuf,
    work: tempfile::TempDir,
    _socket_dir: tempfile::TempDir,
    admission: Arc<Admission<Fake>>,
    stop: Option<oneshot::Sender<()>>,
}

impl Harness {
    fn start(extra: &[(&str, &str)]) -> Self {
        let work = tempfile::tempdir().expect("a work directory");
        let socket_dir = tempfile::tempdir().expect("a socket directory");
        let uid = std::fs::metadata(work.path()).expect("metadata").uid();
        let mut env: HashMap<String, String> = [
            (
                "SAGAT_LAKE_SOCKET_DIR",
                socket_dir.path().to_string_lossy().into_owned(),
            ),
            ("SAGAT_LAKE_RUNNER_UID", uid.to_string()),
            ("SAGAT_LAKE_RUNNER_GID", "10001".to_owned()),
            ("SAGAT_LAKE_WORK_VOLUME", "sagat-lake-work".to_owned()),
            (
                "SAGAT_LAKE_WORK_ROOT",
                work.path().to_string_lossy().into_owned(),
            ),
            ("SAGAT_LAKE_IMAGE", "sagat/lake@sha256:abc".to_owned()),
            ("SAGAT_LAKE_OWNER", "lake-runner".to_owned()),
            ("SAGAT_LAKE_NODE_MEMORY_MIB", "16384".to_owned()),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        for (key, value) in extra {
            env.insert((*key).to_owned(), (*value).to_owned());
        }
        let lookup = |name: &str| env.get(name).cloned();
        let config = LakeConfig::from_lookup(&lookup).expect("a valid configuration");
        let socket = config.socket_path();
        let admission = Arc::new(Admission::new(Fake::new(), config, Arc::new(|| NOW)));
        let listener = UnixListener::bind(&socket).expect("a socket");
        let (stop, stopped) = oneshot::channel::<()>();
        tokio::spawn(serve_unix(
            listener,
            move |peer| peer == uid,
            router(Arc::clone(&admission)),
            ServerConfig {
                header_read_timeout: Duration::from_secs(5),
                max_connections: 16,
                write_stall_timeout: Duration::from_secs(5),
                shutdown_grace: Duration::from_millis(300),
            },
            async move {
                let _ = stopped.await;
            },
        ));
        Self {
            socket,
            work,
            _socket_dir: socket_dir,
            admission,
            stop: Some(stop),
        }
    }

    fn sandbox(&self) -> SocketSandbox {
        SocketSandbox::new(&self.socket)
    }

    /// The attempt directory the lake runner leaves in the work volume.
    fn directory_of(&self, attempt: &str) {
        let path = self.work.path().join(attempt);
        if !path.exists() {
            std::fs::create_dir(path).expect("an attempt directory");
        }
    }

    fn asks(&self, attempt: &str, slot: u32) -> StartRequest {
        self.directory_of(attempt);
        asks(attempt, slot)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

fn asks(attempt: &str, slot: u32) -> StartRequest {
    StartRequest {
        attempt_id: attempt.to_owned(),
        slot,
        class: Class::Small,
        deadline: Duration::from_secs(60),
    }
}

/// A server that answers as it is told and says what it was asked, for the cases the real one will not make.
struct Raw {
    socket: PathBuf,
    seen: Arc<Mutex<Vec<String>>>,
    /// How many connections the client closed while the server was still holding them.
    closed_by_client: Arc<AtomicUsize>,
    _directory: tempfile::TempDir,
}

enum Reply {
    /// A whole response, bytes as they are.
    Bytes(Vec<u8>),
    /// A connection closed without a word.
    Close,
    /// A request read and never answered: the connection is held until the client lets go of it.
    Hold,
    /// A response that never ends: chunks of a kilobyte for as long as the client reads.
    Endless,
}

impl Raw {
    fn start(reply: Reply) -> Self {
        let directory = tempfile::tempdir().expect("a directory");
        let socket = directory.path().join("raw.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("a socket");
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let record = Arc::clone(&seen);
        let closed_by_client: Arc<AtomicUsize> = Arc::default();
        let counted = Arc::clone(&closed_by_client);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                // the head, then as many bytes of body as it says
                let (head_end, length) = loop {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break (None, 0),
                        Ok(read) => request.extend_from_slice(&chunk[..read]),
                    }
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (Some(end + 4), length);
                    }
                };
                if let Some(start) = head_end {
                    while request.len() < start + length {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(read) => request.extend_from_slice(&chunk[..read]),
                        }
                    }
                }
                record
                    .lock()
                    .expect("the record")
                    .push(String::from_utf8_lossy(&request).into_owned());
                match &reply {
                    Reply::Bytes(bytes) => {
                        let _ = stream.write_all(bytes);
                    }
                    Reply::Close => {}
                    Reply::Hold => {
                        // until the client closes, which a read that returns nothing says
                        loop {
                            match stream.read(&mut chunk) {
                                Ok(0) | Err(_) => break,
                                Ok(_) => {}
                            }
                        }
                        counted.fetch_add(1, Ordering::SeqCst);
                    }
                    Reply::Endless => {
                        let head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n";
                        if stream.write_all(head).is_ok() {
                            let mut piece = b"400\r\n".to_vec();
                            piece.extend_from_slice(&[b'x'; 1024]);
                            piece.extend_from_slice(b"\r\n");
                            while stream.write_all(&piece).is_ok() {}
                        }
                    }
                }
            }
        });
        Self {
            socket,
            seen,
            closed_by_client,
            _directory: directory,
        }
    }

    fn sandbox(&self) -> SocketSandbox {
        SocketSandbox::new(&self.socket)
    }

    fn requests(&self) -> Vec<String> {
        self.seen.lock().expect("the record").clone()
    }
}

fn http(status: u16, body: &str) -> Reply {
    Reply::Bytes(
        format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes(),
    )
}

async fn eventually(condition: impl Fn() -> bool) -> bool {
    for _ in 0..200 {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

const NOT_CONFIRMED: ContainerExit = ContainerExit {
    memory_killed: false,
    ended: false,
};

// ---- what the client sends -------------------------------------------------------------------------------------------------

/// The client asks the API for what the API reads: a POST to the attempt, the three members, and a JSON content type.
#[tokio::test(flavor = "multi_thread")]
async fn the_client_asks_what_the_api_reads_and_nothing_else() {
    let raw = Raw::start(http(200, r#"{"result":"started"}"#));
    let request = StartRequest {
        attempt_id: "0198f2c4-7b1e".to_owned(),
        slot: 7,
        class: Class::Large,
        deadline: Duration::from_millis(1500),
    };
    assert_eq!(raw.sandbox().start(&request).await, Ok(()));
    let seen = raw.requests();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let text = &seen[0];
    assert!(
        text.starts_with("POST /v1/attempts/0198f2c4-7b1e HTTP/1.1\r\n"),
        "{text}"
    );
    let lower = text.to_ascii_lowercase();
    assert!(lower.contains("content-type: application/json"), "{text}");
    assert!(lower.contains("host: node"), "{text}");
    assert!(lower.contains("connection: close"), "{text}");
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("a body");
    // read by the server's own reader: exactly the three members, with these values
    assert_eq!(
        StartBody::parse(body.as_bytes()),
        Ok(StartBody {
            slot: 7,
            class: Class::Large,
            deadline_ms: 1500
        })
    );
}

/// A deadline is milliseconds rounded up and never zero: the API refuses a zero, and a few nanoseconds are still a deadline.
#[tokio::test(flavor = "multi_thread")]
async fn the_deadline_goes_in_milliseconds_rounded_up_and_never_zero() {
    for (deadline, expected) in [
        (Duration::ZERO, 1),
        (Duration::from_nanos(1), 1),
        (Duration::from_micros(1500), 2),
        (Duration::from_millis(1), 1),
        (Duration::from_millis(1001), 1001),
        (Duration::from_secs(3600), 3_600_000),
    ] {
        let raw = Raw::start(http(200, r#"{"result":"started"}"#));
        let mut request = asks("a1", 1);
        request.deadline = deadline;
        assert_eq!(raw.sandbox().start(&request).await, Ok(()));
        let seen = raw.requests();
        let body = seen[0]
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("a body");
        assert_eq!(
            StartBody::parse(body.as_bytes()).map(|b| b.deadline_ms),
            Ok(expected),
            "{deadline:?}"
        );
    }
}

/// An id the API would refuse is not sent to be refused: nothing reaches the server, and a wait for it never ends.
#[tokio::test(flavor = "multi_thread")]
async fn an_attempt_id_the_api_would_refuse_is_never_sent() {
    let raw = Raw::start(http(200, r#"{"result":"started"}"#));
    let sandbox = raw.sandbox();
    for bad in ["", "A1", "-a", "a/b", "../a", &"a".repeat(61)] {
        assert_eq!(
            sandbox.start(&asks(bad, 1)).await,
            Err(SandboxError::Unavailable),
            "{bad:?}"
        );
        assert_eq!(sandbox.kill(bad).await, NOT_CONFIRMED, "{bad:?}");
        let waited = tokio::time::timeout(Duration::from_millis(100), sandbox.exited(bad)).await;
        assert!(waited.is_err(), "a wait for {bad:?} ended");
    }
    assert!(raw.requests().is_empty(), "{:?}", raw.requests());
}

// ---- the client and the real server ------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_start_through_the_real_server_makes_the_container_of_the_request() {
    let harness = Harness::start(&[("SAGAT_LAKE_LARGE_MEMORY_MIB", "4096")]);
    let mut request = harness.asks("a1", 11);
    request.class = Class::Large;
    request.deadline = Duration::from_secs(90);
    assert_eq!(harness.sandbox().start(&request).await, Ok(()));
    let specs = harness.admission.engine().specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(
        (
            specs[0].attempt_id.as_str(),
            specs[0].slot,
            specs[0].class.as_str()
        ),
        ("a1", 11, "large")
    );
    // and a retry of the same start, as after a lost answer, is the same answer and no second container
    assert_eq!(harness.sandbox().start(&request).await, Ok(()));
    assert_eq!(harness.admission.engine().launches_made(), 1);
}

/// Each reason the server gives is the reason the client reports.
#[tokio::test(flavor = "multi_thread")]
async fn each_refusal_of_the_real_server_arrives_with_its_reason() {
    let harness = Harness::start(&[("SAGAT_LAKE_CEILING", "1")]);
    let sandbox = harness.sandbox();
    // directory
    assert_eq!(
        sandbox.start(&asks("de01", 1)).await,
        Err(SandboxError::Refused(Reason::Directory))
    );
    // class (the node has no large class)
    let mut large = harness.asks("a0", 1);
    large.class = Class::Large;
    assert_eq!(
        sandbox.start(&large).await,
        Err(SandboxError::Refused(Reason::Class))
    );
    // deadline (above the node's ten minutes)
    let mut long = harness.asks("a0", 1);
    long.deadline = Duration::from_secs(3600);
    assert_eq!(
        sandbox.start(&long).await,
        Err(SandboxError::Refused(Reason::Deadline))
    );
    // slot (past the last)
    assert_eq!(
        sandbox.start(&harness.asks("a0", 4096)).await,
        Err(SandboxError::Refused(Reason::Slot))
    );
    // ceiling
    assert_eq!(sandbox.start(&harness.asks("a1", 1)).await, Ok(()));
    assert_eq!(
        sandbox.start(&harness.asks("a2", 2)).await,
        Err(SandboxError::Refused(Reason::Ceiling))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_engine_that_cannot_answer_is_unavailable_to_the_client_and_never_a_start() {
    let harness = Harness::start(&[]);
    harness.admission.engine().set_list_fails(true);
    assert_eq!(
        harness.sandbox().start(&harness.asks("a1", 1)).await,
        Err(SandboxError::Unavailable)
    );
    assert_eq!(harness.admission.engine().launches_made(), 0);
    assert_eq!(harness.sandbox().kill("a1").await, NOT_CONFIRMED);
}

/// A container that keeps running keeps the wait waiting, and its end is what ends it.
#[tokio::test(flavor = "multi_thread")]
async fn a_wait_ends_when_the_container_ends_and_not_before() {
    let harness = Harness::start(&[]);
    assert_eq!(
        harness.sandbox().start(&harness.asks("a1", 1)).await,
        Ok(())
    );
    let sandbox = harness.sandbox();
    let mut waiting = Box::pin(sandbox.exited("a1"));
    let early = tokio::time::timeout(Duration::from_millis(600), &mut waiting).await;
    assert!(early.is_err(), "the wait ended with the container running");
    harness.admission.engine().finish("a1");
    tokio::time::timeout(BOUND, waiting)
        .await
        .expect("the wait ends when the container has");
}

/// A wait is not an ending when the sandbox cannot answer: it asks again, and the ending comes when it is there.
#[tokio::test(flavor = "multi_thread")]
async fn a_wait_survives_an_engine_that_does_not_answer_and_ends_when_the_container_has() {
    let harness = Harness::start(&[]);
    assert_eq!(
        harness.sandbox().start(&harness.asks("a1", 1)).await,
        Ok(())
    );
    harness.admission.engine().finish("a1");
    harness.admission.engine().set_list_fails(true);
    let sandbox = harness.sandbox();
    let mut waiting = Box::pin(sandbox.exited("a1"));
    // the container is over, but nobody can say so: not an ending
    let early = tokio::time::timeout(Duration::from_millis(800), &mut waiting).await;
    assert!(
        early.is_err(),
        "a wait that could not be answered was taken for an ending"
    );
    harness.admission.engine().set_list_fails(false);
    tokio::time::timeout(BOUND, waiting)
        .await
        .expect("the wait ends once the sandbox can say so");
}

/// No socket, no ending: a sandbox that is not there is not a container that has ended.
#[tokio::test(flavor = "multi_thread")]
async fn a_wait_on_a_socket_that_is_not_there_does_not_end_and_a_start_and_a_kill_say_so() {
    let directory = tempfile::tempdir().expect("a directory");
    let sandbox = SocketSandbox::new(directory.path().join("nothing.sock"));
    assert_eq!(
        sandbox.start(&asks("a1", 1)).await,
        Err(SandboxError::Unavailable)
    );
    assert_eq!(sandbox.kill("a1").await, NOT_CONFIRMED);
    let waited = tokio::time::timeout(Duration::from_millis(700), sandbox.exited("a1")).await;
    assert!(
        waited.is_err(),
        "a sandbox that is not there ended a container"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kill_through_the_real_server_says_what_the_engine_confirmed() {
    let harness = Harness::start(&[]);
    assert_eq!(
        harness.sandbox().start(&harness.asks("a1", 1)).await,
        Ok(())
    );
    harness.admission.engine().mark_memory_killed("a1");
    assert_eq!(
        harness.sandbox().kill("a1").await,
        ContainerExit {
            memory_killed: true,
            ended: true
        }
    );
    // nothing left to end, and the engine said so
    assert_eq!(
        harness.sandbox().kill("a1").await,
        ContainerExit {
            memory_killed: false,
            ended: true
        }
    );
    // the engine refuses the removal: not confirmed
    assert_eq!(
        harness.sandbox().start(&harness.asks("a2", 2)).await,
        Ok(())
    );
    harness.admission.engine().set_end_refuses(true);
    assert_eq!(harness.sandbox().kill("a2").await, NOT_CONFIRMED);
}

/// What the lake runner does when it gives up on a start: it drops the call. The server goes on to the end of the start, and a
/// kill that follows ends what it made: the container that holds the credentials is not left behind by a cancelled call.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_the_caller_gave_up_on_is_ended_by_the_kill_that_follows() {
    let harness = Harness::start(&[]);
    harness.admission.engine().gate_launches();
    let sandbox = harness.sandbox();
    let request = harness.asks("a1", 1);
    // the start is dropped when the server is known to be inside the engine, not after a guess at how long that takes
    tokio::select! {
        _ = sandbox.start(&request) => panic!("the start was answered while the launch was held"),
        () = harness.admission.engine().wait_entered() => {}
    }
    harness.admission.engine().ungate_launches();
    // the kill waits for the start of its attempt, and ends what it made
    assert_eq!(
        sandbox.kill("a1").await,
        ContainerExit {
            memory_killed: false,
            ended: true
        }
    );
    assert!(eventually(|| harness.admission.engine().alive() == 0).await);
    assert_eq!(harness.admission.engine().launches_made(), 1);
    assert!(
        harness
            .admission
            .engine()
            .ended()
            .contains(&"a1".to_owned())
    );
}

// ---- a server that is wrong ----------------------------------------------------------------------------------------------

/// Whatever a wrong server says, the client does not read a start or an ending out of it.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_is_wrong_is_not_believed() {
    let big = "x".repeat(4096);
    let big_started = format!(r#"{{"result":"started","pad":"{big}"}}"#);
    let wrong_for_start = [
        http(200, &big_started),
        http(200, r#"{"result":"started","container":"abc"}"#),
        http(500, r#"{"result":"started"}"#),
        http(409, r#"{"result":"refused","reason":"tired"}"#),
        http(200, "started"),
        http(200, ""),
        Reply::Close,
        Reply::Bytes(b"not http at all".to_vec()),
    ];
    for reply in wrong_for_start {
        let raw = Raw::start(reply);
        assert_eq!(
            raw.sandbox().start(&asks("a1", 1)).await,
            Err(SandboxError::Unavailable)
        );
    }
    let wrong_for_kill = [
        http(
            200,
            r#"{"ended":true,"memory_killed":false,"container":"abc"}"#,
        ),
        http(200, r#"{"ended":true}"#),
        http(500, r#"{"ended":true,"memory_killed":false}"#),
        http(200, r#"{"ended":"true","memory_killed":false}"#),
        http(
            200,
            &format!(r#"{{"ended":true,"memory_killed":false,"pad":"{big}"}}"#),
        ),
        Reply::Close,
        Reply::Bytes(b"not http at all".to_vec()),
    ];
    for reply in wrong_for_kill {
        let raw = Raw::start(reply);
        assert_eq!(raw.sandbox().kill("a1").await, NOT_CONFIRMED);
    }
}

/// A server that says a container ended in a way that is not exactly that does not end a wait.
#[tokio::test(flavor = "multi_thread")]
async fn a_wait_is_not_ended_by_an_answer_that_is_not_exactly_an_ending() {
    for reply in [
        http(200, r#"{"state":"ended","memory_killed":true}"#),
        http(202, r#"{"state":"ended"}"#),
        http(200, r#"{"state":"ENDED"}"#),
        http(200, r#"{"ended":true,"memory_killed":false}"#),
        http(503, r#"{"result":"unavailable"}"#),
        Reply::Close,
    ] {
        let raw = Raw::start(reply);
        let sandbox = raw.sandbox();
        let waited = tokio::time::timeout(Duration::from_millis(500), sandbox.exited("a1")).await;
        assert!(
            waited.is_err(),
            "an answer that was not an ending ended the wait"
        );
    }
    // and the exact ending ends it
    let raw = Raw::start(http(200, r#"{"state":"ended"}"#));
    tokio::time::timeout(BOUND, raw.sandbox().exited("a1"))
        .await
        .expect("an ending ends the wait");
}

/// A server that answers `running` at once is not asked again at once: the pause between two waits is the client's, so a server
/// that holds nothing does not turn the client into a loop that asks as fast as it can.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_answers_running_at_once_is_not_asked_in_a_tight_loop() {
    let raw = Raw::start(http(200, r#"{"state":"running"}"#));
    let sandbox = raw.sandbox();
    let waited = tokio::time::timeout(Duration::from_millis(900), sandbox.exited("a1")).await;
    assert!(waited.is_err(), "a container that runs ended");
    let asked = raw.requests().len();
    // 250 ms apart in 900 ms: four or five, never dozens
    assert!(
        (2..=6).contains(&asked),
        "{asked} waits in 900 ms: the client asks faster than it should"
    );
}

/// What a start refused for is never taken for the engine being out: the reasons are kept apart.
#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_is_not_an_outage_and_an_outage_is_not_a_refusal() {
    let raw = Raw::start(http(409, r#"{"result":"refused","reason":"ceiling"}"#));
    assert_eq!(
        raw.sandbox().start(&asks("a1", 1)).await,
        Err(SandboxError::Refused(Reason::Ceiling))
    );
    let raw = Raw::start(http(503, r#"{"result":"unavailable"}"#));
    assert_eq!(
        raw.sandbox().start(&asks("a1", 1)).await,
        Err(SandboxError::Unavailable)
    );
}

// ---- a server that does not answer, one that does not stop, and one that is not the sandbox runner ----------------------------

/// A server that takes the call and never answers: the call is dropped at its limit, is `Unavailable`, and the connection is
/// let go of, so that nothing is left holding the server's side.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_never_answers_is_given_up_on_and_the_connection_is_closed() {
    let raw = Raw::start(Reply::Hold);
    let sandbox = raw
        .sandbox()
        .with_limits(Duration::from_millis(300), Duration::from_millis(300));
    let began = std::time::Instant::now();
    // bounded here too: a client with no limit of its own would otherwise hang this case instead of failing it
    let answer = tokio::time::timeout(Duration::from_secs(8), sandbox.start(&asks("a1", 1)))
        .await
        .expect("the client gave up on a server that never answers");
    assert_eq!(answer, Err(SandboxError::Unavailable));
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    assert!(
        eventually(|| raw.closed_by_client.load(Ordering::SeqCst) >= 1).await,
        "the connection of a call that was given up on was kept"
    );
    assert_eq!(sandbox.kill("a1").await, NOT_CONFIRMED);
    // and a wait on it is not an ending: it asks again
    let waited = tokio::time::timeout(Duration::from_millis(900), sandbox.exited("a1")).await;
    assert!(waited.is_err(), "a server that never answered ended a wait");
}

/// An answer that does not stop is cut at the bound: the client does not read it for ever, or until its limit.
#[tokio::test(flavor = "multi_thread")]
async fn an_answer_that_does_not_stop_is_cut_at_the_bound() {
    let raw = Raw::start(Reply::Endless);
    // a limit far past what the cut needs: if the body were read to the limit, this would take that long
    let sandbox = raw
        .sandbox()
        .with_limits(Duration::from_secs(20), Duration::from_secs(20));
    let began = std::time::Instant::now();
    assert_eq!(
        sandbox.start(&asks("a1", 1)).await,
        Err(SandboxError::Unavailable)
    );
    assert!(
        began.elapsed() < Duration::from_secs(3),
        "{:?}: the answer was read past its bound",
        began.elapsed()
    );
}

/// Framing that is legitimate and says the same thing is read: the client is strict about what is said and not about how.
#[tokio::test(flavor = "multi_thread")]
async fn an_exact_answer_is_read_whatever_the_framing_it_comes_in() {
    let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n14\r\n{\"result\":\"started\"}\r\n0\r\n\r\n";
    let raw = Raw::start(Reply::Bytes(chunked.to_vec()));
    assert_eq!(raw.sandbox().start(&asks("a1", 1)).await, Ok(()));
    let interim = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n{\"result\":\"started\"}";
    let raw = Raw::start(Reply::Bytes(interim.to_vec()));
    assert_eq!(raw.sandbox().start(&asks("a1", 1)).await, Ok(()));
}

/// Who is at the other end: a server of another user is not the sandbox runner, whatever it says.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_of_another_user_is_not_believed() {
    let harness = Harness::start(&[]);
    let uid = std::fs::metadata(harness.work.path())
        .expect("metadata")
        .uid();
    // the right user: served as before
    let right = SocketSandbox::new(&harness.socket).expecting_server(uid);
    assert_eq!(right.start(&harness.asks("a1", 1)).await, Ok(()));
    harness.admission.engine().mark_memory_killed("a1");
    assert_eq!(
        right.kill("a1").await,
        ContainerExit {
            memory_killed: true,
            ended: true
        }
    );
    // another user: nothing it says is taken for a start, an ending or a kill, and nothing is asked of it
    let wrong = SocketSandbox::new(&harness.socket).expecting_server(uid.wrapping_add(1));
    assert_eq!(
        wrong.start(&harness.asks("a2", 2)).await,
        Err(SandboxError::Unavailable)
    );
    assert_eq!(wrong.kill("a2").await, NOT_CONFIRMED);
    let waited = tokio::time::timeout(Duration::from_millis(600), wrong.exited("a2")).await;
    assert!(waited.is_err(), "a server of another user ended a wait");
    assert_eq!(harness.admission.engine().launches_made(), 1);
}

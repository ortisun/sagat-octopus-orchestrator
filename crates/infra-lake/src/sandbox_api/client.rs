//! The client of the lake sandbox API: the [`LakeSandbox`] of the lake runner, over the node-local socket (feature 035,
//! ADR-0079 rules 10, 14 and 15).
//!
//! It is the only code that reads what the sandbox runner says, and it reads it by the rule of this module's parent: **an answer
//! that is not recognised exactly is never read as started or as ended.** What that comes to, call by call:
//!
//! * `start`: `Started` only for a `started` answer with its status. A refusal keeps its reason. Anything else, including a
//!   socket that is not there, a connection that closes, a body over the bound or a call that takes too long, is `Unavailable`.
//! * `exited`: resolves only when the sandbox says the container has ended. A transport error, an answer it does not
//!   recognise and `unavailable` are not an ending; it asks again, after a pause that grows to two seconds and no more. A
//!   container that keeps running keeps it waiting, as the port says. (Some other part of the attempt ends it: the host
//!   closing its connection, a deadline, a cancel, a lost lease.)
//! * `kill`: `ended` only for the sandbox's own `ended: true`. Anything else is "not confirmed": the container, with the
//!   credentials in it, may still be up, and the report says so.
//!
//! It makes one call per connection and never reuses one: a call that is cancelled leaves nothing on the socket to confuse the
//! next. A call that does not finish in its time is dropped, which closes the connection; the sandbox runner runs a start to its
//! end all the same (ADR-0079, rule 8), so a `kill` that follows *usually* finds what the start made. Usually: the two travel on
//! separate connections, and the kill can reach the server before the start does; it then answers that there is nothing to end,
//! and the start goes on to make a container. By then the relay has been stopped, so that host can never reach the credentials,
//! and the container ends by its label deadline (ADR-0079, rule 9); a driver that wants it gone sooner asks again.
//!
//! **Who answers.** The server checks who connects; the client can check who answers. With [`SocketSandbox::expecting_server`] the
//! user of the process at the other end of the socket must be the one given, or the call is `Unavailable`: something that took
//! the place of the socket file is not believed when it says a container has ended.

use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::header::{CONNECTION, CONTENT_TYPE, HOST};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};

use super::{
    MAX_BODY_BYTES, StartBody, StartOutcome, WAIT_HOLD, WaitOutcome, attempt_id_ok, attempt_path,
    read_kill, read_start, read_wait, wait_path,
};
use crate::attempt::{ContainerExit, LakeSandbox, SandboxError, StartRequest};

/// How long a connection to the socket may take. The socket is on this node: it is there, or it is not.
const CONNECT: Duration = Duration::from_secs(2);
/// How long a start or a kill may take in all. The sandbox runner's engine calls have their own limit of twenty seconds.
const CALL: Duration = Duration::from_secs(30);
/// A wait is held by the server for [`WAIT_HOLD`]; the client gives it that and a margin.
const WAIT_CALL: Duration = Duration::from_secs(WAIT_HOLD.as_secs() + 10);
/// The shortest time between two waits. A server that answers `running` at once would otherwise be asked in a tight loop.
const AGAIN: Duration = Duration::from_millis(250);
/// The longest pause after a wait that could not be answered.
const RETRY_LONGEST: Duration = Duration::from_secs(2);

/// Why a call has no answer. Not shown to anyone: every one of them is "unavailable" to the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    Unreachable,
    TooSlow,
    Malformed,
}

/// Ends the task that drives a connection when the call is over or dropped. hyper itself ends a connection whose request was
/// dropped (a hand mutation that removed the abort survived for that reason: it is an equivalent mutant today), so this is the
/// guarantee that does not depend on that behaviour of the library staying as it is.
struct Driver(JoinHandle<()>);

impl Drop for Driver {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The lake sandbox, reached at a Unix socket.
#[derive(Debug, Clone)]
pub struct SocketSandbox {
    path: PathBuf,
    /// The user the process at the other end must run as, when it is known.
    server: Option<u32>,
    /// How long a start or a kill may take in all, and how long a wait may.
    call_limit: Duration,
    wait_limit: Duration,
}

impl SocketSandbox {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            server: None,
            call_limit: CALL,
            wait_limit: WAIT_CALL,
        }
    }

    /// Believes only a server that runs as `uid`: the kernel says who is at the other end of the socket, and a socket file that
    /// has been replaced by something of another user is not the sandbox runner.
    #[must_use]
    pub const fn expecting_server(mut self, uid: u32) -> Self {
        self.server = Some(uid);
        self
    }

    /// How long a start or a kill may take, and how long a wait may (the server holds a wait for [`WAIT_HOLD`], so the limit of
    /// a wait is longer than that). A call that takes longer is dropped and is `Unavailable`.
    #[must_use]
    pub const fn with_limits(mut self, call: Duration, wait: Duration) -> Self {
        self.call_limit = call;
        self.wait_limit = wait;
        self
    }

    /// One call on a connection of its own: the status and the body, the body at most [`MAX_BODY_BYTES`].
    async fn call(
        &self,
        method: Method,
        target: &str,
        body: Option<Vec<u8>>,
        limit: Duration,
    ) -> Result<(u16, Vec<u8>), Failure> {
        let work = async {
            let stream = timeout(CONNECT, UnixStream::connect(&self.path))
                .await
                .map_err(|_| Failure::Unreachable)?
                .map_err(|_| Failure::Unreachable)?;
            if let Some(expected) = self.server {
                match stream.peer_cred() {
                    Ok(peer) if peer.uid() == expected => {}
                    _ => return Err(Failure::Unreachable),
                }
            }
            let (mut sender, connection) =
                hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
                    .await
                    .map_err(|_| Failure::Malformed)?;
            let _driver = Driver(tokio::spawn(async move {
                let _ = connection.await;
            }));
            let mut request = Request::builder()
                .method(method)
                .uri(target)
                .header(HOST, "node")
                .header(CONNECTION, "close");
            if body.is_some() {
                request = request.header(CONTENT_TYPE, "application/json");
            }
            let request = request
                .body(Full::new(Bytes::from(body.unwrap_or_default())))
                .map_err(|_| Failure::Malformed)?;
            let response = sender
                .send_request(request)
                .await
                .map_err(|_| Failure::Malformed)?;
            let status = response.status().as_u16();
            let body = Limited::new(response.into_body(), MAX_BODY_BYTES)
                .collect()
                .await
                .map_err(|_| Failure::Malformed)?
                .to_bytes();
            Ok((status, body.to_vec()))
        };
        timeout(limit, work).await.map_err(|_| Failure::TooSlow)?
    }
}

/// The deadline in whole milliseconds, rounded up and never zero: the API refuses a zero, and a deadline of a few nanoseconds
/// is still one.
fn milliseconds(deadline: Duration) -> u64 {
    let nanos = deadline.as_nanos();
    u64::try_from(nanos.div_ceil(1_000_000).max(1)).unwrap_or(u64::MAX)
}

impl LakeSandbox for SocketSandbox {
    async fn start(&self, request: &StartRequest) -> Result<(), SandboxError> {
        // an id the API would refuse is a defect of the caller; it is not sent to be refused
        if !attempt_id_ok(&request.attempt_id) {
            return Err(SandboxError::Unavailable);
        }
        let body = StartBody {
            slot: request.slot,
            class: request.class,
            deadline_ms: milliseconds(request.deadline),
        }
        .to_json();
        let answer = self
            .call(
                Method::POST,
                &attempt_path(&request.attempt_id),
                Some(body),
                self.call_limit,
            )
            .await;
        match answer {
            Ok((status, body)) => match read_start(status, &body) {
                StartOutcome::Started => Ok(()),
                StartOutcome::Refused(reason) => Err(SandboxError::Refused(reason)),
                StartOutcome::Unavailable => Err(SandboxError::Unavailable),
            },
            Err(_) => Err(SandboxError::Unavailable),
        }
    }

    async fn exited(&self, attempt_id: &str) {
        if !attempt_id_ok(attempt_id) {
            // there is no container of an attempt that is not one: this never resolves, as for a container that keeps running
            return std::future::pending().await;
        }
        let target = wait_path(attempt_id);
        let mut pause = AGAIN;
        loop {
            let asked = Instant::now();
            // A call that has no answer is not an ending: it falls through to the pause and asks again.
            if let Ok((status, body)) = self.call(Method::GET, &target, None, self.wait_limit).await
            {
                match read_wait(status, &body) {
                    WaitOutcome::Ended => return,
                    WaitOutcome::Running => {
                        // the server holds a wait; one that answers at once is not asked again at once
                        if asked.elapsed() < AGAIN {
                            sleep(AGAIN).await;
                        }
                        pause = AGAIN;
                        continue;
                    }
                    WaitOutcome::Unavailable => {}
                }
            }
            sleep(pause).await;
            pause = (pause * 2).min(RETRY_LONGEST);
        }
    }

    async fn kill(&self, attempt_id: &str) -> ContainerExit {
        let unconfirmed = ContainerExit {
            memory_killed: false,
            ended: false,
        };
        if !attempt_id_ok(attempt_id) {
            return unconfirmed;
        }
        match self
            .call(
                Method::DELETE,
                &attempt_path(attempt_id),
                None,
                self.call_limit,
            )
            .await
        {
            Ok((status, body)) => {
                let outcome = read_kill(status, &body);
                ContainerExit {
                    memory_killed: outcome.memory_killed,
                    ended: outcome.ended,
                }
            }
            Err(_) => unconfirmed,
        }
    }
}

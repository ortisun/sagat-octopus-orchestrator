//! The control conversation of an attempt (feature 035, T035-035, D035-110): the runner writes the request line on the
//! connection the host opened with `CONTROL`, then reads the host's lines until its one result line.
//!
//! The host is a script over an in-memory duplex stream and the clock is paused, so a silence of fifteen seconds takes no
//! time and no test sleeps. What the runner owes the host's lines is bounded memory, a time limit on each one, and the
//! rules of `sagat.lake.v1` (`protocol::Decoder`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use sagat_infra_lake::control::{ControlConfig, ControlFailure, converse};
use sagat_infra_lake::protocol::{MAX_LINE_BYTES, Outcome, ProtocolFailure};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream, ReadBuf, duplex,
};
use tokio::time::{Instant, sleep};

const REQUEST: &[u8] = b"{\"protocol\":\"sagat.lake.v1\",\"operation\":\"icebergWrite\"}\n";
const BEAT: &[u8] = b"{\"heartbeat\":true}\n";
/// What a `BufReader` takes from the stream at a time; a bound on how far past a limit the runner can have read.
const READ_AHEAD: usize = 8 * 1024;

/// The runner's side of a stream, counting the bytes it takes from the host: the one way to see that a limit stops the
/// reading and does not only stop the parsing.
struct Counting<S> {
    inner: S,
    taken: Arc<AtomicUsize>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Counting<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buffer.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(context, buffer);
        self.taken
            .fetch_add(buffer.filled().len() - before, Ordering::Relaxed);
        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counting<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

fn receipt() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/receipts/iceberg-committed.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn result_line() -> Vec<u8> {
    let mut line = serde_json::to_vec(&json!({"receipt": receipt(), "relayDenials": 0})).unwrap();
    line.push(b'\n');
    line
}

fn failure_line() -> Vec<u8> {
    let failure =
        json!({"code": "ICEBERG_SCHEMA_MISMATCH", "column": "amount", "reason": "SCALE_OVERFLOW"});
    let mut line = serde_json::to_vec(&json!({"failure": failure, "relayDenials": 0})).unwrap();
    line.push(b'\n');
    line
}

fn config() -> ControlConfig {
    ControlConfig::default()
}

/// The runner's side and a host that the script drives. `script` gets the host's side after the request was read.
struct Run {
    outcome: Result<sagat_infra_lake::protocol::HostResult, ControlFailure>,
    beats: u32,
    seen_by_host: Vec<u8>,
    /// How many bytes of the host's the runner took from the stream.
    taken: usize,
}

async fn run<F, Fut>(settings: ControlConfig, script: F) -> Run
where
    F: FnOnce(BufReader<DuplexStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (runner, host) = duplex(1 << 20);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let host = tokio::spawn(async move {
        let mut host = BufReader::new(host);
        let mut request = Vec::new();
        host.read_until(b'\n', &mut request).await.unwrap();
        *recorder.lock().unwrap() = request;
        script(host).await;
    });
    let beats = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&beats);
    let taken = Arc::new(AtomicUsize::new(0));
    let runner = Counting {
        inner: runner,
        taken: Arc::clone(&taken),
    };
    let outcome = converse(runner, REQUEST, settings, Instant::now(), move || {
        counter.fetch_add(1, Ordering::Relaxed);
    })
    .await;
    host.abort();
    let _ = host.await;
    Run {
        outcome,
        beats: beats.load(Ordering::Relaxed),
        seen_by_host: seen.lock().unwrap().clone(),
        taken: taken.load(Ordering::Relaxed),
    }
}

async fn say(host: &mut BufReader<DuplexStream>, bytes: &[u8]) {
    host.get_mut().write_all(bytes).await.unwrap();
    host.get_mut().flush().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_host_reads_the_request_and_answers_with_a_receipt() {
    let result = run(config(), |mut host| async move {
        say(&mut host, BEAT).await;
        sleep(Duration::from_secs(1)).await;
        say(&mut host, BEAT).await;
        say(&mut host, &result_line()).await;
    })
    .await;

    assert_eq!(result.seen_by_host, REQUEST);
    assert_eq!(result.beats, 2);
    let host_result = result.outcome.unwrap();
    assert_eq!(host_result.relay_denials, 0);
    assert!(matches!(host_result.outcome, Outcome::Receipt(_)));
}

#[tokio::test(start_paused = true)]
async fn a_failure_of_the_host_is_a_result_and_not_a_break() {
    let result = run(config(), |mut host| async move {
        say(&mut host, &failure_line()).await;
    })
    .await;
    assert!(matches!(
        result.outcome.unwrap().outcome,
        Outcome::Failure(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_host_that_says_nothing_for_the_silence_is_stopped_at_the_silence() {
    let started = Instant::now();
    // the host stays connected and says nothing; `_keep` holds the stream, which an unused parameter would not
    let result = run(config(), |host| async move {
        let _keep = host;
        sleep(Duration::from_secs(3600)).await;
    })
    .await;
    assert_eq!(result.outcome.unwrap_err(), ControlFailure::Silent);
    assert_eq!(started.elapsed(), config().silence);
}

#[tokio::test(start_paused = true)]
async fn every_heartbeat_starts_the_silence_again() {
    let beats = 40_u32;
    let result = run(config(), move |mut host| async move {
        for _ in 0..beats {
            sleep(Duration::from_secs(1)).await;
            say(&mut host, BEAT).await;
        }
        say(&mut host, &result_line()).await;
    })
    .await;
    assert_eq!(result.beats, beats);
    assert!(result.outcome.is_ok());
}

#[tokio::test(start_paused = true)]
async fn a_line_that_trickles_in_does_not_keep_the_host_alive() {
    let result = run(config(), |mut host| async move {
        // never a newline: each byte is on time, the line is not
        for _ in 0..100 {
            say(&mut host, b" ").await;
            sleep(Duration::from_secs(1)).await;
        }
    })
    .await;
    assert_eq!(result.outcome.unwrap_err(), ControlFailure::Silent);
}

#[tokio::test(start_paused = true)]
async fn a_host_that_never_reads_the_request_is_silent_too() {
    let (runner, _host_that_does_not_read) = duplex(64);
    let big = vec![b'x'; 4096];
    let started = Instant::now();
    let outcome = converse(runner, &big, config(), Instant::now(), || {}).await;
    assert_eq!(outcome.unwrap_err(), ControlFailure::Silent);
    assert_eq!(started.elapsed(), config().silence);
}

#[tokio::test(start_paused = true)]
async fn a_host_that_closes_without_a_result_gave_none() {
    let result = run(config(), |_host| async move {}).await;
    assert_eq!(
        result.outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::NoResult)
    );
}

#[tokio::test(start_paused = true)]
async fn a_host_that_closes_in_the_middle_of_a_line_gave_none() {
    let result = run(config(), |mut host| async move {
        say(&mut host, b"{\"receipt\":").await;
    })
    .await;
    assert_eq!(
        result.outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::NoResult)
    );
}

#[tokio::test(start_paused = true)]
async fn a_host_that_goes_away_before_it_reads_the_request_gave_none() {
    let (runner, host) = duplex(64);
    drop(host);
    let outcome = converse(runner, REQUEST, config(), Instant::now(), || {}).await;
    assert_eq!(
        outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::NoResult)
    );
}

#[tokio::test(start_paused = true)]
async fn more_beats_than_the_protocol_allows_are_a_flood() {
    let result = run(config(), |mut host| async move {
        for _ in 0..3 {
            say(&mut host, BEAT).await;
        }
        sleep(Duration::from_secs(3600)).await;
    })
    .await;
    assert_eq!(
        result.outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::HeartbeatFlood)
    );
    // the third beat is the one refused; it is not passed on
    assert_eq!(result.beats, 2);
}

#[tokio::test(start_paused = true)]
async fn a_line_longer_than_the_limit_is_refused_without_reading_it_all() {
    // a host that never sends a newline and never stops: only a bound on the reading can stop the runner
    let result = run(config(), |mut host| async move {
        let chunk = vec![b'a'; 4 * MAX_LINE_BYTES];
        for _ in 0..64 {
            if host.get_mut().write_all(&chunk).await.is_err() {
                break;
            }
        }
    })
    .await;
    assert_eq!(
        result.outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::LineTooLong)
    );
    // the longest line, its newline and one byte more, and what the buffered reader had already fetched: not a byte more
    assert!(
        result.taken <= MAX_LINE_BYTES + 2 + READ_AHEAD,
        "{} bytes were taken for a limit of {MAX_LINE_BYTES}",
        result.taken
    );
}

#[tokio::test(start_paused = true)]
async fn a_line_one_byte_over_the_limit_with_its_newline_is_too_long() {
    let result = run(config(), |mut host| async move {
        let mut line = vec![b'a'; MAX_LINE_BYTES + 1];
        line.push(b'\n');
        say(&mut host, &line).await;
        sleep(Duration::from_secs(3600)).await;
    })
    .await;
    assert_eq!(
        result.outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::LineTooLong)
    );
}

/// The limit exists because the runner parses what it reads into a tree before it checks the shape: a megabyte of `0,`
/// becomes sixteen megabytes of values, in the process that holds the credentials, outside the container's cgroup. The
/// longest line the contract allows is a receipt (no collection in it, an effect key of at most 512 characters and a ref of
/// at most 256), so the limit is a multiple of that and not a number of megabytes.
#[tokio::test(start_paused = true)]
async fn the_longest_receipt_the_contract_allows_fits_in_half_the_limit() {
    let mut longest = receipt();
    longest["effectKey"] = json!("e".repeat(512));
    longest["ref"] = json!("r".repeat(256));
    let mut line =
        serde_json::to_vec(&json!({"receipt": longest, "relayDenials": 4_294_967_295_u64}))
            .unwrap();
    line.push(b'\n');
    assert!(
        line.len() * 2 <= MAX_LINE_BYTES,
        "{} bytes against a limit of {MAX_LINE_BYTES}",
        line.len()
    );
    let result = run(config(), move |mut host| async move {
        say(&mut host, &line).await;
    })
    .await;
    assert!(matches!(
        result.outcome.unwrap().outcome,
        Outcome::Receipt(_)
    ));
}

#[test]
fn the_limit_is_small_enough_for_a_tree_of_it_to_be_small() {
    // 16 bytes of `0,` per 32-byte value is the worst amplification of a JSON line: the tree must stay far below a megabyte
    const {
        assert!(
            MAX_LINE_BYTES * 16 <= 512 * 1024,
            "a line of the host's is parsed into a tree sixteen times its size"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_line_of_exactly_the_limit_is_read_and_judged_by_the_protocol() {
    let result = run(config(), |mut host| async move {
        let mut line = vec![b'a'; MAX_LINE_BYTES];
        line.push(b'\n');
        say(&mut host, &line).await;
    })
    .await;
    // not too long: it is not a JSON object
    assert_eq!(
        result.outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::Malformed)
    );
}

#[tokio::test(start_paused = true)]
async fn a_line_that_is_not_a_line_of_the_protocol_is_a_break() {
    for (line, expected) in [
        (b"not json\n".to_vec(), ProtocolFailure::Malformed),
        (b"[]\n".to_vec(), ProtocolFailure::Malformed),
        (
            b"{\"heartbeat\":false}\n".to_vec(),
            ProtocolFailure::Malformed,
        ),
        (
            b"{\"receipt\":{},\"relayDenials\":0,\"extra\":1}\n".to_vec(),
            ProtocolFailure::UnknownMember,
        ),
    ] {
        let line_to_send = line.clone();
        let result = run(config(), move |mut host| async move {
            say(&mut host, &line_to_send).await;
            sleep(Duration::from_secs(3600)).await;
        })
        .await;
        assert_eq!(
            result.outcome.unwrap_err(),
            ControlFailure::Protocol(expected),
            "{}",
            String::from_utf8_lossy(&line)
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_line_after_the_result_is_a_break() {
    let result = run(config(), |mut host| async move {
        say(&mut host, &result_line()).await;
        say(&mut host, BEAT).await;
        sleep(Duration::from_secs(3600)).await;
    })
    .await;
    assert_eq!(
        result.outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::LineAfterResult)
    );
}

#[tokio::test(start_paused = true)]
async fn a_host_that_keeps_the_connection_open_after_its_result_still_has_given_it() {
    let started = Instant::now();
    let result = run(config(), |mut host| async move {
        say(&mut host, &result_line()).await;
        sleep(Duration::from_secs(3600)).await;
    })
    .await;
    assert!(result.outcome.is_ok());
    // the runner waited for the host to close and gave up after the grace, no longer
    assert_eq!(started.elapsed(), config().result_grace);
}

#[tokio::test(start_paused = true)]
async fn a_host_that_closes_after_its_result_is_not_waited_for() {
    let started = Instant::now();
    let result = run(config(), |mut host| async move {
        say(&mut host, &result_line()).await;
        host.get_mut().shutdown().await.unwrap();
    })
    .await;
    assert!(result.outcome.is_ok());
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn the_bytes_after_the_result_are_not_read_beyond_a_line() {
    // a flood after the result must not be read: the check stops at the first byte
    let sent = result_line().len();
    let result = run(config(), |mut host| async move {
        say(&mut host, &result_line()).await;
        let chunk = vec![b'z'; 64 * 1024];
        for _ in 0..64 {
            if host.get_mut().write_all(&chunk).await.is_err() {
                break;
            }
        }
    })
    .await;
    assert_eq!(
        result.outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::LineAfterResult)
    );
    // the result, and what the buffered reader fetches in the reads that tell the runner there is more: a few buffers, not
    // the four megabytes on offer
    assert!(
        result.taken <= sent + 2 * READ_AHEAD,
        "{} bytes were taken",
        result.taken
    );
}

/// A byte of a line that never ends is as much a break as a line that does: the host had its result accepted, and anything
/// it says after is outside the protocol. It must not matter whether the host then keeps still or closes.
#[tokio::test(start_paused = true)]
async fn a_byte_after_the_result_is_a_break_whether_or_not_the_host_goes_on() {
    for closes in [false, true] {
        let result = run(config(), move |mut host| async move {
            say(&mut host, &result_line()).await;
            say(&mut host, b"z").await;
            if closes {
                host.get_mut().shutdown().await.unwrap();
            } else {
                sleep(Duration::from_secs(3600)).await;
            }
        })
        .await;
        assert_eq!(
            result.outcome.unwrap_err(),
            ControlFailure::Protocol(ProtocolFailure::LineAfterResult),
            "the host closes: {closes}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_line_that_arrives_with_the_result_in_one_write_is_a_break() {
    let result = run(config(), |mut host| async move {
        let mut together = result_line();
        together.extend_from_slice(BEAT);
        say(&mut host, &together).await;
        sleep(Duration::from_secs(3600)).await;
    })
    .await;
    assert_eq!(
        result.outcome.unwrap_err(),
        ControlFailure::Protocol(ProtocolFailure::LineAfterResult)
    );
}

#[test]
fn the_failure_of_a_conversation_is_always_a_host_protocol_failure() {
    use sagat_domain::database::DatabaseFailure;
    for failure in [
        ControlFailure::Silent,
        ControlFailure::Protocol(ProtocolFailure::NoResult),
        ControlFailure::Protocol(ProtocolFailure::HeartbeatFlood),
    ] {
        assert_eq!(failure.code(), DatabaseFailure::LakeHostProtocol);
    }
}

#[test]
fn the_defaults_leave_room_for_a_slow_machine_and_still_end_a_dead_host() {
    let settings = ControlConfig::default();
    // the host beats once a second; fifteen of them in a row missing is not a busy machine
    assert_eq!(settings.silence, Duration::from_secs(15));
    assert_eq!(settings.result_grace, Duration::from_secs(2));
}

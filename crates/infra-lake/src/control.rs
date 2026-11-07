//! The control conversation of one attempt (feature 035, T035-035, D035-110, `contracts/lake-runtime.md` "One attempt").
//!
//! The host connects to the attempt's socket, presents the token and asks for `CONTROL` (the relay hands that connection
//! over, see [`crate::relay`]). From then on this module is the runner's half: it writes the **one request line**, then
//! reads the host's lines until its **one result line**, and hands back the typed result. What it owes those lines is
//! what the host cannot be trusted to keep:
//!
//! - **Bounded memory.** A line is read through a limit of [`MAX_LINE_BYTES`] and a host that never sends a newline is
//!   refused at the limit, not buffered.
//! - **A time limit on each line.** The host beats once a second. A line that is not complete within
//!   [`ControlConfig::silence`] of the last one ends the conversation as [`ControlFailure::Silent`], whether the host sent
//!   nothing, one byte a second, or never read the request. The limit is on the whole line and not on each byte.
//! - **The rules of `sagat.lake.v1`**, which [`Decoder`] owns: at most a beat a second, nothing after the result, no member
//!   the contract does not name.
//!
//! The conversation does not decide to kill the container. It ends, with a result or a [`ControlFailure`], and the driver
//! (T035-035) kills what it started, because the result alone does not say the host exited: a host that keeps the
//! connection open after its result still gave the result, and the driver stops it.
//!
//! Time is `tokio::time`, so a test pauses the clock and a silence of fifteen seconds takes none.

use std::time::Duration;

use sagat_domain::database::DatabaseFailure;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite,
    AsyncWriteExt as _, BufReader,
};
use tokio::time::{Instant, timeout};

use crate::protocol::{Decoder, Event, HostResult, MAX_LINE_BYTES, ProtocolFailure};

/// How long the host may be silent. It beats once a second, so fifteen missing in a row is not a busy machine; the
/// asyncio task that beats runs beside the operation, which does its blocking work in threads.
const SILENCE: Duration = Duration::from_secs(15);
/// How long the host has, after its result line, to close the connection. The result stands either way; a line within
/// this time is a break.
const RESULT_GRACE: Duration = Duration::from_secs(2);

/// The time limits of a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlConfig {
    /// The most the host may take, from the end of one line (or of the request) to the end of the next.
    pub silence: Duration,
    /// How long the runner waits for the host to close the connection once it has its result.
    pub result_grace: Duration,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            silence: SILENCE,
            result_grace: RESULT_GRACE,
        }
    }
}

/// Why a conversation ended without a result. Both are `LAKE_HOST_PROTOCOL`: the host did not keep the protocol, by what
/// it sent or by what it did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ControlFailure {
    #[error("the host did not finish a line, or take the request, within the silence limit")]
    Silent,
    #[error(transparent)]
    Protocol(#[from] ProtocolFailure),
}

impl ControlFailure {
    #[must_use]
    pub const fn code(self) -> DatabaseFailure {
        DatabaseFailure::LakeHostProtocol
    }
}

/// What the caller is told while the conversation runs. A closure that takes no argument is an observer of the beats, which
/// is all most callers need; the driver also wants to know when the request was handed over and the moment the result was
/// decoded, because the conversation goes on after the result (the host's grace to close) and may be dropped there.
pub trait Observer {
    /// A beat the host sent in time.
    fn heartbeat(&mut self) {}
    /// The runner is about to write the request: from here the host may hold some of it.
    fn request_sent(&mut self) {}
    /// The result line was decoded.
    fn result(&mut self, _result: &HostResult) {}
}

impl<F: FnMut()> Observer for F {
    fn heartbeat(&mut self) {
        self();
    }
}

/// What one read of a line came to.
enum Read {
    /// A complete line, without its terminator, in the buffer.
    Line,
    /// The host closed its side with nothing pending.
    Closed,
    /// The host closed its side in the middle of a line.
    Partial,
    /// More than [`MAX_LINE_BYTES`] without a terminator, or a longer line.
    TooLong,
}

/// The next line, read through a limit so that a host without a newline costs one line of memory and no more.
async fn next_line<R: AsyncBufRead + Unpin>(reader: &mut R, line: &mut Vec<u8>) -> Read {
    line.clear();
    // The longest line, its newline and one byte more: the extra byte tells a line of exactly the limit from a longer one.
    let allowance = (MAX_LINE_BYTES + 2) as u64;
    match (&mut *reader).take(allowance).read_until(b'\n', line).await {
        Ok(0) | Err(_) => Read::Closed,
        Ok(_) if line.last() == Some(&b'\n') => {
            line.pop();
            if line.len() > MAX_LINE_BYTES {
                Read::TooLong
            } else {
                Read::Line
            }
        }
        Ok(read) if read as u64 >= allowance => Read::TooLong,
        Ok(_) => Read::Partial,
    }
}

/// The conversation: `request` (the encoded line, newline included) is written, then the host's lines are read to its
/// result. `began` is when the attempt began, for the arrival time the [`Decoder`] measures beats by; `on_heartbeat` is
/// called once for each beat that was accepted.
///
/// **The caller bounds the whole conversation.** Every line has a time limit and no line is longer than the protocol allows,
/// but a host that beats at least every [`ControlConfig::silence`] keeps the conversation going for as long as it likes.
/// The attempt's deadline, a lease lost and a cancellation end it by dropping this future, which closes the stream (the
/// supervisor of T035-035). Two more things hold the caller to its side: `on_heartbeat` must not block, because the
/// [`Decoder`] judges a beat by when it is *read* and a runner that stalls for a second or two reads the host's beats in
/// one burst and calls it a flood; and the request line holds the credentials, so the caller keeps it no longer than this
/// call and zeroises it afterwards.
///
/// # Errors
/// [`ControlFailure::Silent`] when the host takes the request or a line longer than the silence limit, and
/// [`ControlFailure::Protocol`] for what [`Decoder`] refuses, a line over the limit, a host that closed with no result or
/// in the middle of a line, and any byte after the result line.
pub async fn converse<S>(
    stream: S,
    request: &[u8],
    config: ControlConfig,
    began: Instant,
    mut observer: impl Observer,
) -> Result<HostResult, ControlFailure>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    // told before the write, not after: from here the host may hold part of the request, whatever happens to the write
    observer.request_sent();
    let sent = timeout(config.silence, async {
        write.write_all(request).await?;
        write.flush().await
    })
    .await;
    match sent {
        Err(_) => return Err(ControlFailure::Silent),
        // the host went away before it read its request
        Ok(Err(_)) => return Err(ProtocolFailure::NoResult.into()),
        Ok(Ok(())) => {}
    }

    let mut decoder = Decoder::new();
    let mut line = Vec::new();
    loop {
        let read = timeout(config.silence, next_line(&mut reader, &mut line))
            .await
            .map_err(|_| ControlFailure::Silent)?;
        match read {
            Read::Closed | Read::Partial => return Ok(decoder.finish()?),
            Read::TooLong => return Err(ProtocolFailure::LineTooLong.into()),
            Read::Line => match decoder.line(&line, began.elapsed())? {
                Event::Heartbeat => observer.heartbeat(),
                Event::Result(result) => {
                    // told now, before the wait for the host to close: a caller that races this future against a
                    // cancellation must not lose a receipt that was already decoded
                    observer.result(&result);
                    return after_result(&mut reader, config.result_grace, result).await;
                }
            },
        }
    }
}

/// The host has given its result. It closes the connection, or it does not within the grace, and the result stands;
/// **any byte** it sends first is a break, a complete line or not.
///
/// Nothing is read for this: the question is whether a byte is there, and `fill_buf` answers it with the bytes the
/// buffered reader already holds (a line sent in the same write as the result) or one read of its own buffer's size. A
/// host that floods after its result costs one buffer, not a line limit. A read that fails is the end of the connection:
/// the result stands.
async fn after_result<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    grace: Duration,
    result: HostResult,
) -> Result<HostResult, ControlFailure> {
    match timeout(grace, reader.fill_buf()).await {
        Ok(Ok(bytes)) if !bytes.is_empty() => Err(ProtocolFailure::LineAfterResult.into()),
        Ok(Ok(_) | Err(_)) | Err(_) => Ok(result),
    }
}

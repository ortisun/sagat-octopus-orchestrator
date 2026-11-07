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
/// # Errors
/// [`ControlFailure::Silent`] when the host takes the request or a line longer than the silence limit, and
/// [`ControlFailure::Protocol`] for what [`Decoder`] refuses, a line over the limit, a host that closed with no result or
/// in the middle of a line, and any byte after the result line.
pub async fn converse<S>(
    stream: S,
    request: &[u8],
    config: ControlConfig,
    began: Instant,
    mut on_heartbeat: impl FnMut(),
) -> Result<HostResult, ControlFailure>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
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
                Event::Heartbeat => on_heartbeat(),
                Event::Result(result) => {
                    return after_result(&mut reader, &mut line, config.result_grace, result).await;
                }
            },
        }
    }
}

/// The host has given its result. It closes the connection, or it does not within the grace, and the result stands;
/// anything it sends first is a break.
async fn after_result<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut Vec<u8>,
    grace: Duration,
    result: HostResult,
) -> Result<HostResult, ControlFailure> {
    match timeout(grace, next_line(reader, line)).await {
        Err(_) | Ok(Read::Closed) => Ok(result),
        Ok(Read::Line | Read::Partial | Read::TooLong) => {
            Err(ProtocolFailure::LineAfterResult.into())
        }
    }
}

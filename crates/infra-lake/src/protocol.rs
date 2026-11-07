//! `sagat.lake.v1`: the conversation between the lake runner and the sandboxed Python host (feature 035, D035-062,
//! D035-067, `contracts/lake-runtime.md`).
//!
//! The runner sends **one request line**. The host answers with **progress lines** (`{"heartbeat":true}`, at most one
//! a second) and **exactly one result line**, then exits 0. Anything else is `LAKE_HOST_PROTOCOL`: the runner has no
//! reason to believe a host that does not speak the protocol, so it ends the attempt and the effect-key lookup of the
//! next attempt settles what was already sent (D035-030).
//!
//! This module is pure: it frames and checks lines and does no I/O. The attempt driver reads the host's socket a line
//! at a time with a bound of [`MAX_LINE_BYTES`] and gives each line to [`Decoder::line`] with the time it arrived, so a
//! test needs no sleep.

mod receipt;
mod write_body;

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use sagat_domain::database::DatabaseFailure;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};

pub use receipt::{
    Count, EffectKey, FormatVersion, IcebergOutcome, IcebergReceipt, ObjectFormat, ObjectOutcome,
    ObjectStorageReceipt, Receipt, RefName, RowLevelMode, Sha256Hex, SnapshotId, WriteMode,
};

pub use write_body::{
    BodyError, Connection, InputFile, InputFormat, Limits, SNAPSHOT_PROPERTIES, WriteAppend,
};

/// The protocol's name and version, the first member of every request.
pub const PROTOCOL: &str = "sagat.lake.v1";

/// The longest line the runner reads from the host (16 KiB). A result line is a receipt or a failure: no collection in
/// it, an effect key of at most 512 characters and a ref of at most 256, so the longest the contract allows is about 4 KiB
/// and this is a multiple of it. The limit also bounds what the runner builds from a hostile line: it parses a line into a
/// tree before it checks the shape, and a line of `0,` becomes sixteen times its size in values, in the process that
/// holds the credentials and outside the container's cgroup (a megabyte limit allowed sixteen megabytes per attempt).
pub const MAX_LINE_BYTES: usize = 16 * 1024;

/// The longest request line the host reads (4 MiB, without its newline; `MAX_REQUEST_BYTES` of
/// `runners/lake/sagat_lake/host.py`). A longer one is dropped there as a protocol break, so the runner refuses to send it
/// ([`EncodeError::TooLong`]). The members of the body that a person controls are bounded one by one; the `catalog` and
/// `storage` documents are forwarded as stored, and this is the bound on them.
pub const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
/// The longest effect key the host reads, in bytes.
const MAX_EFFECT_KEY_BYTES: usize = 512;

/// Progress lines are at most one a second. The runner measures when a line *arrived*, and a socket delivers late lines
/// in a burst, so the check is a window of one second that holds two lines: the one on time and one of slack. A third
/// inside the window is a flood (D035-077).
const HEARTBEAT_WINDOW: Duration = Duration::from_secs(1);
const HEARTBEATS_PER_WINDOW: usize = 2;
const MAX_COLUMN_CHARS: usize = 128;
const MAX_REASON_CHARS: usize = 48;

/// The members of the request that belong to the envelope; an operation's body cannot name them.
const ENVELOPE: [&str; 5] = [
    "protocol",
    "operation",
    "effectKey",
    "firstAttemptStartedAt",
    "relay",
];

/// What the host is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Operation {
    IcebergWrite,
    IcebergRevert,
    ObjectWrite,
    Probe,
    Browse,
}

/// The per-attempt token the host presents on the relay socket. It is a credential: it is never shown.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayToken(String);

/// Why a relay token is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("a relay token is 32 to 128 characters of letters, digits, '-' and '_'")]
    Shape,
    #[error("no entropy for a relay token")]
    Entropy,
}

impl RelayToken {
    /// A token the relay compares and the host presents on one line, so it has no space, newline or separator, and it
    /// is long enough not to be guessed: 32 to 128 characters of `A-Z a-z 0-9 - _`.
    pub fn new(token: impl Into<String>) -> Result<Self, TokenError> {
        let token = token.into();
        let shape = (32..=128).contains(&token.len())
            && token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        if shape {
            Ok(Self(token))
        } else {
            Err(TokenError::Shape)
        }
    }

    /// A fresh token: 32 bytes from the operating system's random source as 64 hexadecimal digits.
    pub fn generate() -> Result<Self, TokenError> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| TokenError::Entropy)?;
        let mut token = String::with_capacity(64);
        for byte in bytes {
            token.push_str(&format!("{byte:02x}"));
        }
        Self::new(token)
    }

    /// For the comparison the relay makes; the token is never shown or logged.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for RelayToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelayToken(<redacted>)")
    }
}

impl Serialize for RelayToken {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Where the host reaches the relay and how it proves it is the attempt's host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelayAccess {
    socket: String,
    token: RelayToken,
}

impl RelayAccess {
    #[must_use]
    pub fn new(socket: impl Into<String>, token: RelayToken) -> Self {
        Self {
            socket: socket.into(),
            token,
        }
    }
}

/// The members of a request that are not the envelope. They hold the resolved credentials, so the value shows nothing:
/// a `Map` would print every secret in a `{:?}`, a panic message or an assertion that fails.
#[derive(Clone)]
pub struct Body(Map<String, Value>);

impl Body {
    #[must_use]
    pub fn contains_key(&self, member: &str) -> bool {
        self.0.contains_key(member)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The members as a map. The caller takes over the duty of not showing them.
    #[must_use]
    pub fn into_map(self) -> Map<String, Value> {
        self.0
    }
}

impl From<Map<String, Value>> for Body {
    fn from(members: Map<String, Value>) -> Self {
        Self(members)
    }
}

impl fmt::Debug for Body {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Body(<{} members, not shown>)", self.0.len())
    }
}

/// The request line: an envelope this module owns and an operation body its builder owns.
///
/// The body holds the pinned connection with its resolved credentials, the table, the mode and the limits. Each
/// operation gets its own typed builder with the task that adds the operation: [`WriteAppend`] builds the body of an
/// `icebergWrite` in the append mode (D035-109), and the others are an open map until theirs exists. The envelope
/// members cannot be overridden by it.
#[derive(Clone)]
pub struct Request {
    operation: Operation,
    effect_key: String,
    first_attempt_started_at: String,
    relay: RelayAccess,
    body: Body,
}

impl Request {
    /// `first_attempt_started_at` is an RFC 3339 instant in UTC, formatted by the caller; [`encode_request`] refuses one
    /// the host would not read.
    #[must_use]
    pub fn new(
        operation: Operation,
        effect_key: impl Into<String>,
        first_attempt_started_at: impl Into<String>,
        relay: RelayAccess,
        body: impl Into<Body>,
    ) -> Self {
        Self {
            operation,
            effect_key: effect_key.into(),
            first_attempt_started_at: first_attempt_started_at.into(),
            relay,
            body: body.into(),
        }
    }
}

/// The body carries resolved credentials and the relay carries a token, so neither is shown.
impl fmt::Debug for Request {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Request")
            .field("operation", &self.operation)
            .field("effect_key", &self.effect_key)
            .field("body", &self.body)
            .finish_non_exhaustive()
    }
}

/// Why a request could not be framed. All of them are faults of the runner, not of the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("the body of the request names a member of the envelope")]
    ReservedMember,
    #[error("the effect key or the instant of the first attempt is not one the host reads")]
    Envelope,
    #[error("the request line is longer than the host reads")]
    TooLong,
    #[error("the request cannot be serialized")]
    Serialization,
}

/// The request as one line: compact JSON that ends with a single newline. A newline inside a value is escaped by the
/// serializer, so the line cannot be split.
///
/// The line holds every secret of the attempt and the relay token as plain bytes. The caller keeps it only as long as it
/// takes to write it, never shows it, and zeroises it (T035-035).
///
/// # Errors
/// [`EncodeError`] for a body that names an envelope member, an effect key or an instant the host would refuse, or a line
/// of more than [`MAX_REQUEST_BYTES`] without its newline.
pub fn encode_request(request: &Request) -> Result<Vec<u8>, EncodeError> {
    if request
        .body
        .0
        .keys()
        .any(|key| ENVELOPE.contains(&key.as_str()))
    {
        return Err(EncodeError::ReservedMember);
    }
    if !write_body::plain(&request.effect_key, MAX_EFFECT_KEY_BYTES)
        || !write_body::is_instant(&request.first_attempt_started_at)
    {
        return Err(EncodeError::Envelope);
    }
    let relay = serde_json::to_value(&request.relay).map_err(|_| EncodeError::Serialization)?;
    let operation =
        serde_json::to_value(request.operation).map_err(|_| EncodeError::Serialization)?;
    let mut members = request.body.0.clone();
    members.insert("protocol".to_owned(), Value::from(PROTOCOL));
    members.insert("operation".to_owned(), operation);
    members.insert(
        "effectKey".to_owned(),
        Value::from(request.effect_key.clone()),
    );
    members.insert(
        "firstAttemptStartedAt".to_owned(),
        Value::from(request.first_attempt_started_at.clone()),
    );
    members.insert("relay".to_owned(), relay);
    let mut line =
        serde_json::to_vec(&Value::Object(members)).map_err(|_| EncodeError::Serialization)?;
    if line.len() > MAX_REQUEST_BYTES {
        return Err(EncodeError::TooLong);
    }
    line.push(b'\n');
    Ok(line)
}

/// A break of the protocol by the host. The reason is a closed kind and never carries content of the line, so it can
/// be logged and audited; every one of them is `LAKE_HOST_PROTOCOL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolFailure {
    #[error("the host ended without a result line")]
    NoResult,
    #[error("a line is not a JSON object of the protocol")]
    Malformed,
    #[error("a line has a member the protocol does not define, or lacks one it requires")]
    UnknownMember,
    #[error("a line is longer than the limit")]
    LineTooLong,
    #[error("progress lines arrive faster than the protocol allows")]
    HeartbeatFlood,
    #[error("a line follows the result line")]
    LineAfterResult,
    #[error("the receipt does not hold as typed fields")]
    InvalidReceipt,
    #[error("the failure is not one of the closed set")]
    InvalidFailure,
}

impl ProtocolFailure {
    #[must_use]
    pub const fn code(self) -> DatabaseFailure {
        DatabaseFailure::LakeHostProtocol
    }
}

/// The codes a host may report: those its operations raise (`runners/lake/sagat_lake`, `contracts/errors.md`). The rest of
/// the table belongs to the runner (a cancellation, a deadline, a container that did not start, an admission refusal) or
/// to another feature, and a host that claims one is not the host that was written: it could make a run look cancelled
/// or timed out. `OUTCOME_UNKNOWN` is here because the host reports it when its lookup of the effect key is inconclusive
/// (D035-030, step 3). A new operation of the host adds its codes here, and a code missing from this list fails closed as
/// `LAKE_HOST_PROTOCOL`.
const HOST_MAY_REPORT: [DatabaseFailure; 18] = [
    DatabaseFailure::IcebergTableNotFound,
    DatabaseFailure::IcebergUnsupportedTable,
    DatabaseFailure::IcebergLocationNotAllowed,
    DatabaseFailure::IcebergSchemaMismatch,
    DatabaseFailure::IcebergDuplicateKey,
    DatabaseFailure::IcebergScopeViolation,
    DatabaseFailure::IcebergCommitConflict,
    DatabaseFailure::IcebergRevertBlocked,
    DatabaseFailure::IcebergLimitExceeded,
    DatabaseFailure::CatalogUnavailable,
    DatabaseFailure::StorageUnavailable,
    DatabaseFailure::Authentication,
    DatabaseFailure::Credential,
    DatabaseFailure::Tls,
    DatabaseFailure::Connection,
    DatabaseFailure::OutcomeUnknown,
    DatabaseFailure::WorkerError,
    DatabaseFailure::LakeHostProtocol,
];

/// A failure the host reports: a code the host may report ([`HOST_MAY_REPORT`]) and, for a schema mismatch, the column
/// and a reason. There is no free text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFailure {
    pub code: DatabaseFailure,
    pub column: Option<String>,
    pub reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFailure {
    code: String,
    #[serde(default)]
    column: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

impl HostFailure {
    fn from_wire(value: Value) -> Result<Self, ProtocolFailure> {
        let wire: WireFailure =
            serde_json::from_value(value).map_err(|_| ProtocolFailure::InvalidFailure)?;
        let code = DatabaseFailure::parse(&wire.code)
            .filter(|code| HOST_MAY_REPORT.contains(code))
            .ok_or(ProtocolFailure::InvalidFailure)?;
        let column_holds = wire.column.as_deref().is_none_or(|column| {
            (1..=MAX_COLUMN_CHARS).contains(&column.chars().count())
                && !column.chars().any(char::is_control)
        });
        let reason_holds = wire.reason.as_deref().is_none_or(|reason| {
            (1..=MAX_REASON_CHARS).contains(&reason.len())
                && reason
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        });
        if !column_holds || !reason_holds {
            return Err(ProtocolFailure::InvalidFailure);
        }
        Ok(Self {
            code,
            column: wire.column,
            reason: wire.reason,
        })
    }
}

/// How the attempt ended, as the host says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Receipt(Receipt),
    Failure(HostFailure),
}

/// The result line: the outcome and the number of connections the relay refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostResult {
    pub outcome: Outcome,
    pub relay_denials: u32,
}

/// What one line of the host is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Heartbeat,
    Result(HostResult),
}

/// Checks the lines of one attempt, in order.
#[derive(Debug, Default)]
pub struct Decoder {
    beats: VecDeque<Duration>,
    result: Option<HostResult>,
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The next line of the host, without its terminator, and the time since the attempt began at which it arrived.
    pub fn line(&mut self, bytes: &[u8], at: Duration) -> Result<Event, ProtocolFailure> {
        if bytes.len() > MAX_LINE_BYTES {
            return Err(ProtocolFailure::LineTooLong);
        }
        if self.result.is_some() {
            return Err(ProtocolFailure::LineAfterResult);
        }
        let Ok(Value::Object(mut members)) = serde_json::from_slice::<Value>(bytes) else {
            return Err(ProtocolFailure::Malformed);
        };
        if members.len() == 1 && members.contains_key("heartbeat") {
            if members.get("heartbeat") != Some(&Value::Bool(true)) {
                return Err(ProtocolFailure::Malformed);
            }
            self.beat(at)?;
            return Ok(Event::Heartbeat);
        }
        let is_receipt = members.len() == 2
            && members.contains_key("receipt")
            && members.contains_key("relayDenials");
        let is_failure = members.len() == 2
            && members.contains_key("failure")
            && members.contains_key("relayDenials");
        if !is_receipt && !is_failure {
            return Err(ProtocolFailure::UnknownMember);
        }
        let relay_denials = members
            .remove("relayDenials")
            .and_then(|count| count.as_u64())
            .and_then(|count| u32::try_from(count).ok())
            .ok_or(ProtocolFailure::Malformed)?;
        let outcome = if is_receipt {
            let receipt = members.remove("receipt").unwrap_or(Value::Null);
            Outcome::Receipt(
                serde_json::from_value(receipt).map_err(|_| ProtocolFailure::InvalidReceipt)?,
            )
        } else {
            Outcome::Failure(HostFailure::from_wire(
                members.remove("failure").unwrap_or(Value::Null),
            )?)
        };
        let result = HostResult {
            outcome,
            relay_denials,
        };
        self.result = Some(result.clone());
        Ok(Event::Result(result))
    }

    fn beat(&mut self, at: Duration) -> Result<(), ProtocolFailure> {
        // Before the first second of the attempt there is no beat old enough to be out of the window. A saturated floor
        // of zero would drop a beat that arrived at zero, and any number of beats at that instant would pass.
        if let Some(floor) = at.checked_sub(HEARTBEAT_WINDOW) {
            while self.beats.front().is_some_and(|beat| *beat <= floor) {
                self.beats.pop_front();
            }
        }
        if self.beats.len() >= HEARTBEATS_PER_WINDOW {
            return Err(ProtocolFailure::HeartbeatFlood);
        }
        self.beats.push_back(at);
        Ok(())
    }

    /// The host closed its side. The result it sent, or [`ProtocolFailure::NoResult`].
    pub fn finish(self) -> Result<HostResult, ProtocolFailure> {
        self.result.ok_or(ProtocolFailure::NoResult)
    }
}

/// The class of the last exception a host reported on standard error, from a closed set: the name is mapped, never
/// kept, because a message can carry a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionClass {
    /// No line looked like an exception.
    None,
    MemoryError,
    OsError,
    ConnectionError,
    TimeoutError,
    ValueError,
    KeyError,
    TypeError,
    RuntimeError,
    /// An exception with another name.
    Other,
}

/// What the runner keeps of the host's standard error (D035-062): a closed class and the number of lines. The text is
/// not stored or logged, so this type holds nothing that could carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StderrSummary {
    pub exception: ExceptionClass,
    pub lines: u32,
}

/// Reduces the host's standard error to a [`StderrSummary`].
#[must_use]
pub fn summarize_stderr(bytes: &[u8]) -> StderrSummary {
    let mut lines: u32 = 0;
    let mut last: &[u8] = &[];
    let mut pieces = bytes.split(|byte| *byte == b'\n').peekable();
    while let Some(piece) = pieces.next() {
        if piece.is_empty() && pieces.peek().is_none() {
            break;
        }
        lines = lines.saturating_add(1);
        if !piece.iter().all(u8::is_ascii_whitespace) {
            last = piece;
        }
    }
    StderrSummary {
        exception: exception_class(last),
        lines,
    }
}

/// A Python traceback ends with `QualifiedName` or `QualifiedName: message`, and exception names are CamelCase.
fn exception_class(line: &[u8]) -> ExceptionClass {
    let Ok(text) = std::str::from_utf8(line) else {
        return ExceptionClass::None;
    };
    let head = text.split(':').next().unwrap_or_default().trim();
    let identifier = head
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    let name = head.rsplit('.').next().unwrap_or_default();
    if head.is_empty() || !identifier || !name.starts_with(|c: char| c.is_ascii_uppercase()) {
        return ExceptionClass::None;
    }
    match name {
        "MemoryError" => ExceptionClass::MemoryError,
        "OSError" | "IOError" | "FileNotFoundError" | "PermissionError" => ExceptionClass::OsError,
        "ConnectionError"
        | "ConnectionResetError"
        | "ConnectionAbortedError"
        | "ConnectionRefusedError"
        | "BrokenPipeError" => ExceptionClass::ConnectionError,
        "TimeoutError" => ExceptionClass::TimeoutError,
        "ValueError" => ExceptionClass::ValueError,
        "KeyError" => ExceptionClass::KeyError,
        "TypeError" => ExceptionClass::TypeError,
        "RuntimeError" => ExceptionClass::RuntimeError,
        _ => ExceptionClass::Other,
    }
}

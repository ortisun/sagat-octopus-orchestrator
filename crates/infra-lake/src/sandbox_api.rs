//! The wire of the lake sandbox API (feature 035, ADR-0079, G035-055).
//!
//! The lake runner holds a destination's credentials and asks a lake-mode sandbox runner, over a node-local Unix socket,
//! for an **attempt**, never for a container: three calls, HTTP/1.1 with small JSON bodies, versioned in the path.
//!
//! - `POST /v1/attempts/{attempt_id}` with [`StartBody`] starts the attempt's container ([`StartAnswer`]).
//! - `GET /v1/attempts/{attempt_id}/wait` holds for up to [`WAIT_HOLD`] ([`WaitState`]).
//! - `DELETE /v1/attempts/{attempt_id}` ends it and says whether the engine confirmed that and whether the memory limit
//!   did it ([`kill_body`]).
//!
//! A request carries a slot, a budget class and a deadline, and nothing else: no image, mount, capability, command, limit,
//! and no user outside the lake range. The types here are the one home of the wire, so that the server of
//! `apps/sandbox-runner` and the client of the lake runner cannot drift; both are held to one corpus
//! (`tests/contract/iceberg-writes/fixtures/sandbox-api/cases.json`).
//!
//! **The rule the client reads by:** an answer that is not recognised exactly is never read as started or as ended. A
//! status that does not belong to the answer, a member that is added or missing, a name twice, a variant nobody knows, a
//! body that is not text, one over the bound: each is `Unavailable` (for a kill, "not confirmed"), because a container
//! that holds the credentials may still be up. A change to a request or an answer is a `/v2` path served beside `/v1`
//! for one release; within `/v1` both ends are strict.
//!
//! This module has no engine dependency and no transport: the server, the client and the listener are built on it.

pub mod client;

use std::collections::BTreeMap;
use std::time::Duration;

use serde::de::{self, Deserialize, Deserializer, MapAccess, Visitor};
use serde::{Deserialize as DeserializeDerive, Serialize};
use serde_json::{Value, json};

/// The user id of slot 0 and the number of slots: the lake range of users, well above the range of system accounts. The
/// sandbox profile (`crates/infra-sandbox/src/lake.rs`) has the same two numbers; neither crate depends on the other, so a
/// test in the sandbox runner, which depends on both, holds the two functions equal.
pub const FIRST_UID: u32 = 40_000;
pub const MAX_SLOTS: u32 = 4096;
/// The longest body of a request or an answer, and the longest headers (a `traceparent` fits).
pub const MAX_BODY_BYTES: usize = 1024;
pub const MAX_HEADER_BYTES: usize = 4096;
/// How long the server holds a wait before it answers that the container is still running. The client asks again.
pub const WAIT_HOLD: Duration = Duration::from_secs(25);
/// The path of the API, versioned.
pub const PREFIX: &str = "/v1/attempts";

/// The user id of a slot, or `None` past the last slot. The relay's peer check compares it, so the lake runner needs it
/// before the container exists (ADR-0079, rule 5).
#[must_use]
pub const fn uid_for_slot(slot: u32) -> Option<u32> {
    if slot < MAX_SLOTS {
        Some(FIRST_UID + slot)
    } else {
        None
    }
}

/// An attempt id as the API and the staging directory both have it: 1 to 60 lower-case hexadecimal digits and hyphens
/// that start with a hexadecimal digit (ADR-0079, rule 10).
#[must_use]
pub fn attempt_id_ok(id: &str) -> bool {
    crate::staging::attempt_id_holds(id)
}

/// `POST` and `DELETE` go here.
#[must_use]
pub fn attempt_path(attempt_id: &str) -> String {
    format!("{PREFIX}/{attempt_id}")
}

/// `GET` goes here.
#[must_use]
pub fn wait_path(attempt_id: &str) -> String {
    format!("{PREFIX}/{attempt_id}/wait")
}

/// A body the API does not read. It names nothing: the server answers [`Plain::Rejected`] and the client never sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the body is not one the sandbox API reads")]
pub struct WireError;

/// The budget class of an attempt: a closed set of two names, each a budget the node configures. The lake runner maps a
/// kind of write to a class by its own table; a class is never a limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, DeserializeDerive)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    Small,
    Large,
}

impl Class {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Large => "large",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "small" => Some(Self::Small),
            "large" => Some(Self::Large),
            _ => None,
        }
    }
}

/// Why a start was refused. Closed: it names a rule, never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// The node is at its ceiling of containers or of reserved memory.
    Ceiling,
    /// The slot is outside the range or is in use.
    Slot,
    /// The class is not one this node configures.
    Class,
    /// The deadline is zero or above the node's ceiling.
    Deadline,
    /// The attempt directory is missing, is a link, or is not the lake runner's.
    Directory,
}

impl Reason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ceiling => "ceiling",
            Self::Slot => "slot",
            Self::Class => "class",
            Self::Deadline => "deadline",
            Self::Directory => "directory",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "ceiling" => Some(Self::Ceiling),
            "slot" => Some(Self::Slot),
            "class" => Some(Self::Class),
            "deadline" => Some(Self::Deadline),
            "directory" => Some(Self::Directory),
            _ => None,
        }
    }
}

/// The body of a start: a slot, a class and a deadline, and nothing else. A member nobody knows is not ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartBody {
    /// The lake runner's choice within the node's range: its user id is [`uid_for_slot`].
    pub slot: u32,
    pub class: Class,
    /// How long the container may live, from the request. The sandbox refuses one above its ceiling instead of clamping it.
    pub deadline_ms: u64,
}

impl StartBody {
    /// Reads the body of a request: at most [`MAX_BODY_BYTES`], an **object** (not an array in the order of the members)
    /// with these three members and no other, no name twice, each of its type.
    ///
    /// # Errors
    /// [`WireError`] for anything else.
    pub fn parse(bytes: &[u8]) -> Result<Self, WireError> {
        // not the derived reader: it also takes an array in the order of the members, and a body that says the same thing
        // in two shapes is one more to agree on
        let members = members(bytes).ok_or(WireError)?;
        if members.len() != 3 {
            return Err(WireError);
        }
        let slot = members
            .get("slot")
            .and_then(Value::as_u64)
            .and_then(|slot| u32::try_from(slot).ok())
            .ok_or(WireError)?;
        let class = members
            .get("class")
            .and_then(Value::as_str)
            .and_then(Class::parse)
            .ok_or(WireError)?;
        let deadline_ms = members
            .get("deadline_ms")
            .and_then(Value::as_u64)
            .ok_or(WireError)?;
        Ok(Self {
            slot,
            class,
            deadline_ms,
        })
    }

    /// What the body says that no node may accept: a slot outside the range, a deadline of zero. What depends on the node
    /// (its ceilings, its classes, whether the slot is in use) is the server's.
    ///
    /// # Errors
    /// The [`Reason`] to refuse with.
    pub const fn validate(&self) -> Result<(), Reason> {
        if self.slot >= MAX_SLOTS {
            return Err(Reason::Slot);
        }
        if self.deadline_ms == 0 {
            return Err(Reason::Deadline);
        }
        Ok(())
    }

    /// The text of the request.
    #[must_use]
    pub fn to_json(&self) -> Vec<u8> {
        json!({"slot": self.slot, "class": self.class.as_str(), "deadline_ms": self.deadline_ms})
            .to_string()
            .into_bytes()
    }
}

/// What the server answers to a start, and what the client reads: the same three outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAnswer {
    Started,
    Refused(Reason),
    /// The engine, the volume or the image could not be used, or the node is draining.
    Unavailable,
}

/// The client's reading of an answer to a start.
pub type StartOutcome = StartAnswer;

/// What a wait saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitState {
    Running,
    Ended,
}

/// The client's reading of an answer to a wait. `Unavailable` is not an ending: the caller asks again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Running,
    Ended,
    Unavailable,
}

/// The client's reading of an answer to a kill. `ended: false` is "not confirmed": the container may still be up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KillOutcome {
    pub ended: bool,
    pub memory_killed: bool,
}

/// What the server answers when it cannot answer the question, as a closed body: a body it cannot read, a route it does
/// not have, a method it does not allow, a body too large, a peer it does not serve, or the engine being unavailable. No
/// engine text, no echo of what was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plain {
    Rejected,
    Unavailable,
}

fn body(value: &Value) -> Vec<u8> {
    value.to_string().into_bytes()
}

/// The status and body of an answer to a start.
#[must_use]
pub fn start_body(answer: StartAnswer) -> (u16, Vec<u8>) {
    match answer {
        StartAnswer::Started => (200, body(&json!({"result": "started"}))),
        StartAnswer::Refused(reason) => (
            409,
            body(&json!({"result": "refused", "reason": reason.as_str()})),
        ),
        StartAnswer::Unavailable => (503, body(&json!({"result": "unavailable"}))),
    }
}

/// The status and body of an answer to a wait.
#[must_use]
pub fn wait_body(state: WaitState) -> (u16, Vec<u8>) {
    let state = match state {
        WaitState::Running => "running",
        WaitState::Ended => "ended",
    };
    (200, body(&json!({"state": state})))
}

/// The status and body of an answer to a kill.
#[must_use]
pub fn kill_body(ended: bool, memory_killed: bool) -> (u16, Vec<u8>) {
    (
        200,
        body(&json!({"ended": ended, "memory_killed": memory_killed})),
    )
}

/// The status and body of an answer that is not an answer to the question.
#[must_use]
pub fn plain_body(plain: Plain) -> (u16, Vec<u8>) {
    match plain {
        Plain::Rejected => (400, body(&json!({"result": "rejected"}))),
        Plain::Unavailable => (503, body(&json!({"result": "unavailable"}))),
    }
}

/// An object whose names are all different. `serde_json::Value` keeps the last of a name written twice and drops the
/// first without a trace; for an answer that says whether a container is gone, that is a way to read an ending that the
/// first member denied.
struct Members(BTreeMap<String, Value>);

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Reads;
        impl<'de> Visitor<'de> for Reads {
            type Value = Members;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an object with no name twice")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Members, A::Error> {
                let mut members = BTreeMap::new();
                while let Some(name) = map.next_key::<String>()? {
                    let value: Value = map.next_value()?;
                    if members.insert(name, value).is_some() {
                        return Err(de::Error::custom("a name twice"));
                    }
                }
                Ok(Members(members))
            }
        }
        deserializer.deserialize_map(Reads)
    }
}

/// The members of an answer, if it is a small object with no name twice.
fn members(body: &[u8]) -> Option<BTreeMap<String, Value>> {
    if body.len() > MAX_BODY_BYTES {
        return None;
    }
    serde_json::from_slice::<Members>(body)
        .ok()
        .map(|members| members.0)
}

/// Reads the answer to a start. Anything that is not exactly one of the three answers, with the status that belongs to it,
/// is [`StartAnswer::Unavailable`].
#[must_use]
pub fn read_start(status: u16, body: &[u8]) -> StartOutcome {
    let Some(members) = members(body) else {
        return StartAnswer::Unavailable;
    };
    let text = |name: &str| members.get(name).and_then(Value::as_str);
    match (status, text("result"), members.len()) {
        (200, Some("started"), 1) => StartAnswer::Started,
        (409, Some("refused"), 2) => text("reason")
            .and_then(Reason::parse)
            .map_or(StartAnswer::Unavailable, StartAnswer::Refused),
        _ => StartAnswer::Unavailable,
    }
}

/// Reads the answer to a wait. Anything that is not exactly `running` or `ended` with status 200 is
/// [`WaitOutcome::Unavailable`], which the caller answers by asking again and never by taking it for an ending.
#[must_use]
pub fn read_wait(status: u16, body: &[u8]) -> WaitOutcome {
    let Some(members) = members(body) else {
        return WaitOutcome::Unavailable;
    };
    if status != 200 || members.len() != 1 {
        return WaitOutcome::Unavailable;
    }
    match members.get("state").and_then(Value::as_str) {
        Some("running") => WaitOutcome::Running,
        Some("ended") => WaitOutcome::Ended,
        _ => WaitOutcome::Unavailable,
    }
}

/// Reads the answer to a kill. Anything that is not exactly the two booleans with status 200 is "not confirmed, nothing
/// read": the lake runner then reports that a container with the credentials may still be up.
#[must_use]
pub fn read_kill(status: u16, body: &[u8]) -> KillOutcome {
    let unconfirmed = KillOutcome {
        ended: false,
        memory_killed: false,
    };
    let Some(members) = members(body) else {
        return unconfirmed;
    };
    if status != 200 || members.len() != 2 {
        return unconfirmed;
    }
    match (
        members.get("ended").and_then(Value::as_bool),
        members.get("memory_killed").and_then(Value::as_bool),
    ) {
        (Some(ended), Some(memory_killed)) => KillOutcome {
            ended,
            memory_killed,
        },
        _ => unconfirmed,
    }
}

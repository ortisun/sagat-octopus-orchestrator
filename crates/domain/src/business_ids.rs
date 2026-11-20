//! Business identifiers (feature 018): a caller's name for a piece of work ("order 123"), unique among
//! the open executions of a namespace, with a policy for a second start while one is open and a
//! policy for a start after one has closed.
//!
//! The binding row is claimed in the creation transaction of the run; openness is *derived* from the
//! holder's own status and never written back, so a transition driven by a run does not touch it.
//! This module holds the value, the two policies and the pure admission over what the store read.

use std::fmt;
use std::str::FromStr;

use time::{Duration, OffsetDateTime};

use crate::execution::RunStatus;

pub const MAX_BUSINESS_ID_CHARS: usize = 200;
/// 30 days (O1001).
pub const DEFAULT_REUSE_WINDOW_SECONDS: u32 = 2_592_000;
/// 366 days.
pub const MAX_REUSE_WINDOW_SECONDS: u32 = 31_622_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BusinessIdError {
    #[error("a business id has at least one character")]
    Empty,
    #[error("a business id has at most {MAX_BUSINESS_ID_CHARS} characters")]
    TooLong,
    #[error("a business id is visible ASCII: no space, control or non-ASCII character")]
    NotVisibleAscii,
    #[error("the reuse window is one second to 366 days")]
    BadReuseWindow,
}

/// 1 to 200 visible ASCII characters (`!` to `~`). Customer content: it is kept out of logs, spans,
/// metrics and audit reasons (FR-018-042).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BusinessId(String);

impl BusinessId {
    /// # Errors
    /// [`BusinessIdError`] naming the first rule broken: empty, over 200 characters, or a character
    /// outside `!`..`~`.
    pub fn new(value: &str) -> Result<Self, BusinessIdError> {
        if value.is_empty() {
            return Err(BusinessIdError::Empty);
        }
        if value.chars().count() > MAX_BUSINESS_ID_CHARS {
            return Err(BusinessIdError::TooLong);
        }
        if !value.chars().all(|c| c.is_ascii_graphic()) {
            return Err(BusinessIdError::NotVisibleAscii);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Never printed by `Debug`: a business id is caller text and must not reach a log by accident.
impl fmt::Debug for BusinessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BusinessId(..)")
    }
}

/// What a start does when an open execution already holds the id. There is no terminate-existing
/// policy (D1013).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Refuse `409 BUSINESS_ID_IN_USE`, naming the holder.
    #[default]
    Fail,
    /// Return the holder with `200` and create nothing.
    UseExisting,
}

impl ConflictPolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fail => "FAIL",
            Self::UseExisting => "USE_EXISTING",
        }
    }
}

impl FromStr for ConflictPolicy {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "FAIL" => Ok(Self::Fail),
            "USE_EXISTING" => Ok(Self::UseExisting),
            _ => Err(()),
        }
    }
}

/// What a start does when the holder has closed, inside the reuse window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReusePolicy {
    #[default]
    AllowDuplicate,
    /// Allowed unless the holder ended `SUCCEEDED` (`COMPENSATED` and `PARTIAL` count as not
    /// succeeded).
    AllowDuplicateFailedOnly,
    /// Refused `BUSINESS_ID_REUSE_REJECTED`.
    RejectDuplicate,
}

impl ReusePolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AllowDuplicate => "ALLOW_DUPLICATE",
            Self::AllowDuplicateFailedOnly => "ALLOW_DUPLICATE_FAILED_ONLY",
            Self::RejectDuplicate => "REJECT_DUPLICATE",
        }
    }
}

impl FromStr for ReusePolicy {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ALLOW_DUPLICATE" => Ok(Self::AllowDuplicate),
            "ALLOW_DUPLICATE_FAILED_ONLY" => Ok(Self::AllowDuplicateFailedOnly),
            "REJECT_DUPLICATE" => Ok(Self::RejectDuplicate),
            _ => Err(()),
        }
    }
}

/// How long a closed id is remembered: 30 days by default, up to 366 (O1001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReuseWindow(u32);

impl Default for ReuseWindow {
    fn default() -> Self {
        Self(DEFAULT_REUSE_WINDOW_SECONDS)
    }
}

impl ReuseWindow {
    /// # Errors
    /// [`BusinessIdError::BadReuseWindow`] outside one second to 366 days.
    pub fn from_seconds(seconds: u32) -> Result<Self, BusinessIdError> {
        if !(1..=MAX_REUSE_WINDOW_SECONDS).contains(&seconds) {
            return Err(BusinessIdError::BadReuseWindow);
        }
        Ok(Self(seconds))
    }

    #[must_use]
    pub const fn seconds(self) -> u32 {
        self.0
    }
}

/// The stored binding. Its window is the one in force when the id was bound, so a later change of
/// the namespace setting does not move it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    window: ReuseWindow,
}

impl Binding {
    #[must_use]
    pub const fn new(window: ReuseWindow) -> Self {
        Self { window }
    }

    #[must_use]
    pub const fn window(self) -> ReuseWindow {
        self.window
    }
}

/// The holder of a binding, read from its run or chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HolderStatus {
    /// Not final yet (a chain is open until its final outcome).
    Open,
    Closed {
        status: RunStatus,
        finished_at: OffsetDateTime,
    },
}

impl HolderStatus {
    /// A holder is closed only with evidence: a terminal status *and* the time it finished. A
    /// terminal run with no finish time stays held, so a gap in the data cannot free an id.
    #[must_use]
    pub fn of(status: RunStatus, finished_at: Option<OffsetDateTime>) -> Self {
        match finished_at {
            Some(finished_at) if status.is_terminal() => Self::Closed {
                status,
                finished_at,
            },
            Some(_) | None => Self::Open,
        }
    }
}

/// A binding with its holder, as the store read them under the binding row's lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    pub binding: Binding,
    pub holder: HolderStatus,
}

/// What a start with a business id becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// The id is free: bind it to the new run.
    Bind,
    /// An open holder and `USE_EXISTING`: return the holder, create nothing.
    UseExisting,
    /// An open holder and `FAIL`: `409 BUSINESS_ID_IN_USE`.
    InUse,
    /// A closed holder inside the window and a policy that refuses: `BUSINESS_ID_REUSE_REJECTED`.
    ReuseRejected,
}

impl Admission {
    #[must_use]
    pub const fn creates_a_run(self) -> bool {
        matches!(self, Self::Bind)
    }
}

/// Decides a start with a business id from the binding the store found (none when the id is free).
///
/// A closed holder holds the id until `finished_at + window`; from that instant the id is free,
/// whatever the policy. A finish time that is in the future (clock skew) keeps the id held.
#[must_use]
pub fn admit(
    held: Option<&Held>,
    conflict: ConflictPolicy,
    reuse: ReusePolicy,
    now: OffsetDateTime,
) -> Admission {
    let Some(held) = held else {
        return Admission::Bind;
    };
    match held.holder {
        HolderStatus::Open => match conflict {
            ConflictPolicy::Fail => Admission::InUse,
            ConflictPolicy::UseExisting => Admission::UseExisting,
        },
        HolderStatus::Closed {
            status,
            finished_at,
        } => {
            let window = Duration::seconds(i64::from(held.binding.window().seconds()));
            let free = finished_at
                .checked_add(window)
                .is_some_and(|until| now >= until);
            if free {
                return Admission::Bind;
            }
            match reuse {
                ReusePolicy::AllowDuplicate => Admission::Bind,
                ReusePolicy::AllowDuplicateFailedOnly => match status {
                    RunStatus::Succeeded => Admission::ReuseRejected,
                    RunStatus::Failed
                    | RunStatus::Cancelled
                    | RunStatus::TimedOut
                    | RunStatus::Compensated
                    | RunStatus::Partial
                    | RunStatus::CompensationFailed
                    | RunStatus::Running
                    | RunStatus::Waiting
                    | RunStatus::Paused
                    | RunStatus::Cancelling
                    | RunStatus::Compensating
                    | RunStatus::ManualReview => Admission::Bind,
                },
                ReusePolicy::RejectDuplicate => Admission::ReuseRejected,
            }
        }
    }
}

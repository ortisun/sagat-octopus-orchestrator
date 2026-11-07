//! What the runner reports when an attempt ends (feature 035, T035-035, D035-110, G035-054).
//!
//! The driver waits on several things at once: the conversation with the host, the container, the attempt's deadline, the
//! lease and a cancellation. Whichever ends first ends the attempt, and the driver then kills the container and asks the
//! engine how it died. This module is the table from that pair, [`Ended`] and [`ContainerFacts`], to [`Disposition`]: it
//! does no I/O and reads no clock, so every row is a test.
//!
//! The rows, and why:
//!
//! | The attempt ended with | Reported |
//! |---|---|
//! | the host's receipt | `CompleteStep` with the receipt (whatever happens to the container afterwards) |
//! | the host's failure | `FailStep` with its code, and its column when it is a plain name |
//! | no result, a break of the protocol, silence | `LAKE_HOST_PROTOCOL` (retryable) |
//! | no result or silence, and the memory limit killed the container | `ICEBERG_LIMIT_EXCEEDED` (not retryable: the write does not fit) |
//! | a container that could not be started | `LAKE_SANDBOX_UNAVAILABLE` (retryable) |
//! | the attempt's deadline | `DATABASE_TIMEOUT` |
//! | a cancellation | `DATABASE_CANCELLED` |
//! | a lease lost | nothing: the report would be refused (fenced) |
//!
//! **No ending is `OUTCOME_UNKNOWN`.** An attempt that ends without a word may have committed, but the next attempt's
//! lookup of the effect key in the table's history settles it (D035-030), so the runner does not make a person decide
//! what the lookup decides. A receipt the host sent stands even if the container was then killed: the work was done.

use sagat_domain::database::DatabaseFailure;

use crate::control::ControlFailure;
use crate::protocol::{HostResult, Outcome, ProtocolFailure, Receipt};

/// How the wait on an attempt ended. Exhaustive on purpose: a new way to end is decided here, in one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The conversation ran to its end (the host's result) or broke.
    Conversation(Result<HostResult, ControlFailure>),
    /// The sandbox runner did not start the container.
    SandboxUnavailable,
    /// The container started and the host never opened the control conversation in the time it had.
    HostNeverConnected,
    /// The attempt's deadline passed.
    DeadlinePassed,
    /// The step was cancelled.
    Cancelled,
    /// The lease was lost (the heartbeat was refused, or it expired).
    LeaseLost,
}

impl Ended {
    /// How strongly this ending claims the attempt when another is ready at the same time: a lost lease first (nothing can
    /// be reported), then a result the host gave (the work was done, whatever the clock says), then what a person asked
    /// for, then the clock, then what the host did and what the sandbox did. Exhaustive on purpose.
    const fn rank(&self) -> u8 {
        match self {
            Self::LeaseLost => 6,
            Self::Conversation(Ok(_)) => 5,
            Self::Cancelled => 4,
            Self::DeadlinePassed => 3,
            Self::Conversation(Err(_)) => 2,
            Self::HostNeverConnected => 1,
            Self::SandboxUnavailable => 0,
        }
    }

    /// The ending that wins among those that are ready together, or `None` when none is. Among equals the first wins.
    /// The driver collects what is ready after its wait wakes and asks here, so which ending counts is a decision with a
    /// test and not the order a `select!` lists its arms.
    #[must_use]
    pub fn strongest(candidates: impl IntoIterator<Item = Self>) -> Option<Self> {
        candidates.into_iter().fold(None, |best, next| match best {
            Some(best) if best.rank() >= next.rank() => Some(best),
            _ => Some(next),
        })
    }
}

/// What the engine said about the container when the driver ended it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerFacts {
    /// The memory limit killed it.
    pub memory_killed: bool,
}

/// A failure to report. The column and the reason are the host's, and the column is kept only if it is a plain name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: DatabaseFailure,
    pub column: Option<String>,
    pub reason: Option<String>,
}

impl Failure {
    const fn code(code: DatabaseFailure) -> Self {
        Self {
            code,
            column: None,
            reason: None,
        }
    }
}

/// What the runner does about the attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// Report `CompleteStep` with this receipt, rebuilt from the host's typed fields.
    Complete(Box<Receipt>),
    /// Report `FailStep`.
    Fail(Failure),
    /// Report nothing: the lease is gone.
    Abandon,
}

/// The table above.
#[must_use]
pub fn dispose(ended: Ended, container: ContainerFacts) -> Disposition {
    match ended {
        Ended::Conversation(Ok(result)) => match result.outcome {
            Outcome::Receipt(receipt) => Disposition::Complete(Box::new(receipt)),
            Outcome::Failure(failure) => Disposition::Fail(Failure {
                code: failure.code,
                column: failure.column.filter(|column| is_plain_name(column)),
                reason: failure.reason,
            }),
        },
        // A killed host says nothing, or goes quiet while it swaps; any other break happened before the kill.
        Ended::Conversation(Err(
            ControlFailure::Silent | ControlFailure::Protocol(ProtocolFailure::NoResult),
        ))
        | Ended::HostNeverConnected
            if container.memory_killed =>
        {
            Disposition::Fail(Failure::code(DatabaseFailure::IcebergLimitExceeded))
        }
        Ended::Conversation(Err(_)) | Ended::HostNeverConnected => {
            Disposition::Fail(Failure::code(DatabaseFailure::LakeHostProtocol))
        }
        Ended::SandboxUnavailable => {
            Disposition::Fail(Failure::code(DatabaseFailure::LakeSandboxUnavailable))
        }
        Ended::DeadlinePassed => Disposition::Fail(Failure::code(DatabaseFailure::Timeout)),
        Ended::Cancelled => Disposition::Fail(Failure::code(DatabaseFailure::Cancelled)),
        Ended::LeaseLost => Disposition::Abandon,
    }
}

/// A column name as a person writes one: letters and digits of any script, `_`, `.`, `-` and a space. The host holds the
/// credentials and the column is text it chooses (G035-057), so anything that carries an encoding (`+ / =`), a markup, a
/// separator or an invisible character is not repeated; the code is reported without it. This narrows the channel and
/// does not close it: letters and digits still carry data, a few dozen bytes an attempt.
fn is_plain_name(column: &str) -> bool {
    !column.is_empty()
        && column
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | ' '))
}

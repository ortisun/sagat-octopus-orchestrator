//! What the runner reports when an attempt ends (feature 035, T035-035, D035-111, G035-054).
//!
//! The driver waits on several things at once: the conversation with the host, the container, the attempt's deadline, the
//! lease and a cancellation. Whichever ends first ends the attempt, and the driver then kills the container and asks the
//! engine how it died. This module is the table from that pair, [`Ended`] and [`AttemptFacts`], to [`Disposition`]: it
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
//! | a host that never connected, or a container lost before it | `LAKE_HOST_PROTOCOL` / `LAKE_SANDBOX_UNAVAILABLE` (retryable) |
//! | the deadline or a cancellation **after the request was sent** | `OUTCOME_UNKNOWN` |
//! | the deadline or a cancellation before it | `DATABASE_TIMEOUT` / `DATABASE_CANCELLED` |
//! | a lease lost, or a heartbeat that failed | nothing |
//!
//! **Why `OUTCOME_UNKNOWN` after the request, and not before.** Once the request line is written the host may have
//! committed, and a deadline or a cancellation does not retry: no attempt would ever look the effect key up. The 017
//! runner does the same for a statement after dispatch, and G035-043 says to treat a killed attempt as an unknown
//! outcome. A retry of an unknown outcome (it needs `saga:override`, D035-069) finds the commit by the lookup of D035-030
//! and reports it (`foundByLookup`) instead of writing again. The endings the engine **does** retry (`LAKE_HOST_PROTOCOL`,
//! `LAKE_SANDBOX_UNAVAILABLE`) are settled by that retry's lookup and are not unknown. The one ending that is neither
//! retried nor unknown is the memory kill, and it is `ICEBERG_LIMIT_EXCEEDED` because the write does not fit; if the
//! commit had landed before the kill (it would have been the last thing the host did), a manual retry finds it too.
//!
//! A receipt the host sent stands even if the container was then killed: the work was done. A lost lease reports nothing,
//! also when the cause was only a heartbeat that failed and the lease is still valid at the server: the step is then
//! leased again when the lease expires, the next attempt's lookup finds a commit that was already made, and an uncertain
//! situation never changes the step's state.

use sagat_domain::database::DatabaseFailure;

use crate::control::ControlFailure;
use crate::protocol::{HostResult, Outcome, ProtocolFailure, Receipt};

/// How the wait on an attempt ended. Exhaustive on purpose: a new way to end is decided here, in one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The conversation ran to its end (the host's result) or broke.
    Conversation(Result<HostResult, ControlFailure>),
    /// The sandbox runner did not start the container, or the driver lost it and cannot say what became of it. The attempt
    /// may have left a host running, which the next attempt's conditional commit does not mind (D035-030).
    SandboxUnavailable,
    /// The host never opened the control conversation in the time it had: it did not start, it crashed on import, or the
    /// container exited before it connected.
    HostNeverConnected,
    /// The attempt's deadline passed.
    DeadlinePassed,
    /// The server asked for the step to be cancelled (and only that: a heartbeat that failed is [`Ended::LeaseLost`]).
    Cancelled,
    /// The lease was lost: a heartbeat was refused or failed, or the lease ran out.
    LeaseLost,
}

impl Ended {
    /// How strongly this ending claims the attempt when another is ready at the same time: a lost lease first (a report
    /// from a lease the server has given up would be refused), then a result the host gave (the work was done, whatever the
    /// clock says), then what a person asked for, then the clock, then what the host did and what the sandbox did.
    /// Exhaustive on purpose.
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

/// What the driver knows about the attempt when it ends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptFacts {
    /// The engine says the memory limit killed the container. `false` when the engine could not be asked.
    pub memory_killed: bool,
    /// The request line was written to the host. Before that the host had nothing to do and nothing can have been
    /// written to the table; after it, the host may have committed.
    pub request_sent: bool,
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

    /// The flag reported with the failure: the contract's flag for the code (the worker reads it, G035-027 item 8).
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.code.retryable()
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
pub fn dispose(ended: Ended, facts: AttemptFacts) -> Disposition {
    match ended {
        Ended::Conversation(Ok(result)) => match result.outcome {
            Outcome::Receipt(receipt) => Disposition::Complete(Box::new(receipt)),
            Outcome::Failure(failure) => Disposition::Fail(Failure {
                code: failure.code,
                column: failure.column.filter(|column| is_plain_name(column)),
                reason: failure.reason,
            }),
        },
        // A killed host says nothing, or goes quiet while it swaps; any other break happened before the kill. A host that
        // never connected died of how it was started, which the deployment decides, and not of what the write needs.
        Ended::Conversation(Err(
            ControlFailure::Silent | ControlFailure::Protocol(ProtocolFailure::NoResult),
        )) if facts.memory_killed => {
            Disposition::Fail(Failure::code(DatabaseFailure::IcebergLimitExceeded))
        }
        Ended::Conversation(Err(_)) | Ended::HostNeverConnected => {
            Disposition::Fail(Failure::code(DatabaseFailure::LakeHostProtocol))
        }
        Ended::SandboxUnavailable => {
            Disposition::Fail(Failure::code(DatabaseFailure::LakeSandboxUnavailable))
        }
        Ended::DeadlinePassed => Disposition::Fail(Failure::code(if facts.request_sent {
            DatabaseFailure::OutcomeUnknown
        } else {
            DatabaseFailure::Timeout
        })),
        Ended::Cancelled => Disposition::Fail(Failure::code(if facts.request_sent {
            DatabaseFailure::OutcomeUnknown
        } else {
            DatabaseFailure::Cancelled
        })),
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

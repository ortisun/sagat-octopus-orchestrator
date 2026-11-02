//! Pure gates for the dormant Vacate and Acquire commands (B1d.2). Possession is not a permit.
//!
//! These decide only what the head run's own status and lease fact allow. The adapter must still
//! supply them from one consistent snapshot taken under the chain, run and group guards, and must
//! apply `CapacityGroup::vacate` or `acquire` for holder, epoch and revision. Neither gate says
//! that a member has work to run: that needs one shared runnable predicate (G715).

use thiserror::Error;

use super::RunStatus;

/// A transfer command refused before the group transition is attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CapacityTransferRefusal {
    #[error("the head run is terminal: finish the member instead")]
    HeadFinal,
    #[error("a step of the head run still holds a worker lease")]
    NotQuiescent,
    #[error("the head run cannot move in its current status")]
    NotMovable,
}

/// Vacate: the head run is not terminal and no step of it is `LEASED` or `RUNNING`, whether or not
/// the lease has expired. Ready, pending, waiting and retry-waiting steps may remain.
///
/// # Errors
/// [`CapacityTransferRefusal::HeadFinal`] for a terminal run, [`CapacityTransferRefusal::NotQuiescent`]
/// when a step holds a lease.
pub const fn vacate_gate(run: RunStatus, leased_step: bool) -> Result<(), CapacityTransferRefusal> {
    // Every status is decided here on purpose: a new `RunStatus` must not compile unnoticed.
    match run {
        RunStatus::Succeeded
        | RunStatus::Failed
        | RunStatus::Cancelled
        | RunStatus::TimedOut
        | RunStatus::Compensated
        | RunStatus::Partial
        | RunStatus::CompensationFailed => Err(CapacityTransferRefusal::HeadFinal),
        RunStatus::Running
        | RunStatus::Waiting
        | RunStatus::Paused
        | RunStatus::Cancelling
        | RunStatus::Compensating
        | RunStatus::ManualReview => {
            if leased_step {
                Err(CapacityTransferRefusal::NotQuiescent)
            } else {
                Ok(())
            }
        }
    }
}

/// Acquire: the head run is not terminal and is in a status that can move.
///
/// # Errors
/// [`CapacityTransferRefusal::HeadFinal`] for a terminal run, [`CapacityTransferRefusal::NotMovable`]
/// for a paused run or one waiting for an operator.
pub const fn acquire_gate(run: RunStatus) -> Result<(), CapacityTransferRefusal> {
    match run {
        RunStatus::Succeeded
        | RunStatus::Failed
        | RunStatus::Cancelled
        | RunStatus::TimedOut
        | RunStatus::Compensated
        | RunStatus::Partial
        | RunStatus::CompensationFailed => Err(CapacityTransferRefusal::HeadFinal),
        RunStatus::Paused | RunStatus::ManualReview => Err(CapacityTransferRefusal::NotMovable),
        RunStatus::Running
        | RunStatus::Waiting
        | RunStatus::Cancelling
        | RunStatus::Compensating => Ok(()),
    }
}

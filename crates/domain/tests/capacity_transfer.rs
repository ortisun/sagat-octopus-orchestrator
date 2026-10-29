//! Pure Vacate and Acquire gates (B1d.2): what the head run's status and lease fact allow, and no more.
//! The finite input space is enumerated, so the independent truth table below is the specification.

use sagat_domain::execution::{CapacityTransferRefusal, RunStatus, acquire_gate, vacate_gate};

/// Every run status. `ordinal` has no wildcard arm, so adding a variant to `RunStatus` fails to
/// compile here until the new status is decided in both gates.
const ALL: [RunStatus; 13] = [
    RunStatus::Running,
    RunStatus::Waiting,
    RunStatus::Paused,
    RunStatus::Cancelling,
    RunStatus::Compensating,
    RunStatus::ManualReview,
    RunStatus::Succeeded,
    RunStatus::Failed,
    RunStatus::Cancelled,
    RunStatus::TimedOut,
    RunStatus::Compensated,
    RunStatus::Partial,
    RunStatus::CompensationFailed,
];

const fn ordinal(run: RunStatus) -> usize {
    match run {
        RunStatus::Running => 0,
        RunStatus::Waiting => 1,
        RunStatus::Paused => 2,
        RunStatus::Cancelling => 3,
        RunStatus::Compensating => 4,
        RunStatus::ManualReview => 5,
        RunStatus::Succeeded => 6,
        RunStatus::Failed => 7,
        RunStatus::Cancelled => 8,
        RunStatus::TimedOut => 9,
        RunStatus::Compensated => 10,
        RunStatus::Partial => 11,
        RunStatus::CompensationFailed => 12,
    }
}

/// Independent truth table. Terminal statuses belong to Finish; a lease blocks Vacate whatever the
/// status; paused and manual-review runs may vacate but cannot acquire.
fn expected_vacate(run: RunStatus, leased: bool) -> Result<(), CapacityTransferRefusal> {
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
            if leased {
                Err(CapacityTransferRefusal::NotQuiescent)
            } else {
                Ok(())
            }
        }
    }
}

fn expected_acquire(run: RunStatus) -> Result<(), CapacityTransferRefusal> {
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

#[test]
fn the_status_list_covers_every_run_status_once() {
    let mut seen = [false; 13];
    for run in &ALL {
        let slot = ordinal(*run);
        assert!(!seen[slot], "{run:?} listed twice");
        seen[slot] = true;
    }
    assert!(seen.iter().all(|s| *s), "a run status is missing from ALL");
}

#[test]
fn vacate_gate_matches_the_truth_table_for_every_status_and_lease_fact() {
    for run in &ALL {
        for leased in [false, true] {
            assert_eq!(
                vacate_gate(*run, leased),
                expected_vacate(*run, leased),
                "vacate {run:?} leased={leased}"
            );
        }
    }
}

#[test]
fn acquire_gate_matches_the_truth_table_for_every_status() {
    for run in &ALL {
        assert_eq!(
            acquire_gate(*run),
            expected_acquire(*run),
            "acquire {run:?}"
        );
    }
}

#[test]
fn a_held_lease_never_allows_vacate() {
    for run in &ALL {
        assert_ne!(vacate_gate(*run, true), Ok(()), "{run:?}");
    }
}

#[test]
fn both_gates_send_exactly_the_terminal_statuses_to_finish() {
    for run in &ALL {
        let final_vacate = vacate_gate(*run, false) == Err(CapacityTransferRefusal::HeadFinal);
        let final_acquire = acquire_gate(*run) == Err(CapacityTransferRefusal::HeadFinal);
        assert_eq!(final_vacate, run.is_terminal(), "vacate {run:?}");
        assert_eq!(final_acquire, run.is_terminal(), "acquire {run:?}");
    }
}

#[test]
fn a_terminal_head_is_final_even_with_a_contradictory_lease_fact() {
    assert_eq!(
        vacate_gate(RunStatus::Succeeded, true),
        Err(CapacityTransferRefusal::HeadFinal)
    );
}

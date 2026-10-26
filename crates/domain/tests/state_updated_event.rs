#![allow(clippy::expect_used)]

use sagat_domain::execution::{RunEvent, RunId, RunState, RunStatus, StepStatus};
use sagat_domain::workflow::{ActivityName, Digest, NodeId};

fn state_update(before_revision: u64, after_revision: u64) -> RunEvent {
    RunEvent::StateUpdated {
        operation: ActivityName::parse("balance.credit").expect("operation"),
        before_revision,
        after_revision,
        state_digest: Digest::new([41; 32]),
    }
}

#[test]
fn state_update_advances_only_the_journal_projection() {
    let entrypoint = NodeId::parse("entry").expect("entrypoint");
    let mut state = RunState::new(RunId::generate());
    state
        .apply(&RunEvent::RunStarted {
            entrypoint: entrypoint.clone(),
        })
        .expect("run starts");

    state.apply(&state_update(0, 1)).expect("state update");

    assert_eq!(state.version(), 2);
    assert_eq!(state.status(), RunStatus::Running);
    assert_eq!(
        state.step(&entrypoint).expect("entrypoint").status(),
        StepStatus::Pending
    );
}

#[test]
fn state_update_rejects_a_non_consecutive_revision_without_advancing() {
    let entrypoint = NodeId::parse("entry").expect("entrypoint");
    let mut state = RunState::new(RunId::generate());
    state
        .apply(&RunEvent::RunStarted { entrypoint })
        .expect("run starts");

    assert!(state.apply(&state_update(0, 2)).is_err());
    assert_eq!(state.version(), 1);
}

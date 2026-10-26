//! Collection projections restore the same state as the addressed journal.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use sagat_domain::execution::{
    AddressedRunEvent, CollectionDrainReason, CollectionIterationRestore, CollectionOutcome,
    CollectionPhase, ExecutionInstanceId, FailureCode, InstancePath, RunEvent, RunId, RunState,
};
use sagat_domain::workflow::{Digest, NodeId};

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node")
}

fn owner() -> ExecutionInstanceId {
    ExecutionInstanceId::root(node("items"))
}

fn member(local: &str) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(node("items"), 0)
            .expect("bounded member path"),
        node(local),
    )
}

fn addressed_member(event: RunEvent) -> AddressedRunEvent {
    AddressedRunEvent::in_path(member("work").path().clone(), event)
        .expect("member event can be addressed")
}

fn checkpointed_events() -> Vec<AddressedRunEvent> {
    vec![
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: node("items"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: node("items"),
            item_count: 1,
            max_items: 1,
            max_concurrency: 1,
            snapshot_digest: Digest::new([1; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationAdmitted {
            node: node("items"),
            index: 0,
            entry: member("work"),
            item_digest: Digest::new([2; 32]),
        }),
        addressed_member(RunEvent::StepReleased { node: node("work") }),
        addressed_member(RunEvent::StepLeased {
            node: node("work"),
            attempt: 1,
            generation: 1,
        }),
        addressed_member(RunEvent::StepStarted {
            node: node("work"),
            attempt: 1,
            generation: 1,
        }),
        addressed_member(RunEvent::StepCompleted {
            node: node("work"),
            attempt: 1,
            generation: 1,
        }),
        addressed_member(RunEvent::StepRouted {
            node: node("work"),
            activated: vec![node("return")],
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationReturned {
            node: node("items"),
            index: 0,
            return_instance: member("return"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationCheckpointed {
            node: node("items"),
            index: 0,
            return_instance: member("return"),
            result_digest: Digest::new([3; 32]),
        }),
    ]
}

fn drained_events(reason: CollectionDrainReason) -> Vec<AddressedRunEvent> {
    vec![
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: node("items"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: node("items"),
            item_count: 1,
            max_items: 1,
            max_concurrency: 1,
            snapshot_digest: Digest::new([1; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationAdmitted {
            node: node("items"),
            index: 0,
            entry: member("work"),
            item_digest: Digest::new([2; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionDrainStarted {
            node: node("items"),
            reason,
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationDrained {
            node: node("items"),
            index: 0,
        }),
    ]
}

fn restore_from_steps(
    replayed: &RunState,
    phase: CollectionPhase,
    result_digest: Option<Digest>,
) -> RunState {
    let mut restored = RunState::restore_instances(
        replayed.run_id(),
        replayed.status(),
        None,
        false,
        replayed
            .instances()
            .map(|(instance, step)| (instance.clone(), step.clone()))
            .collect::<BTreeMap<_, _>>(),
        BTreeMap::new(),
        replayed.version(),
    );
    restored
        .restore_collection(
            owner(),
            phase,
            1,
            1,
            1,
            Digest::new([1; 32]),
            result_digest,
            vec![CollectionIterationRestore::checkpointed(
                0,
                member("work"),
                Digest::new([2; 32]),
                member("return"),
                Digest::new([3; 32]),
            )],
        )
        .expect("collection projection restores");
    restored
}

fn restore_drained_from_steps(
    replayed: &RunState,
    phase: CollectionPhase,
    reason: CollectionDrainReason,
) -> RunState {
    let mut restored = RunState::restore_instances(
        replayed.run_id(),
        replayed.status(),
        replayed
            .stop_reason()
            .map(|stop| (stop, replayed.stop_code().cloned())),
        replayed.is_paused(),
        replayed
            .instances()
            .map(|(instance, step)| (instance.clone(), step.clone()))
            .collect::<BTreeMap<_, _>>(),
        BTreeMap::new(),
        replayed.version(),
    );
    restored
        .restore_collection_progress(
            owner(),
            phase,
            1,
            1,
            1,
            Digest::new([1; 32]),
            None,
            Some(reason),
            vec![CollectionIterationRestore::drained(
                0,
                member("work"),
                Digest::new([2; 32]),
            )],
        )
        .expect("drained collection projection restores");
    restored
}

fn restore_checkpointed_data_limit_from_steps(
    replayed: &RunState,
    phase: CollectionPhase,
) -> RunState {
    let mut restored = RunState::restore_instances(
        replayed.run_id(),
        replayed.status(),
        replayed
            .stop_reason()
            .map(|stop| (stop, replayed.stop_code().cloned())),
        replayed.is_paused(),
        replayed
            .instances()
            .map(|(instance, step)| (instance.clone(), step.clone()))
            .collect::<BTreeMap<_, _>>(),
        BTreeMap::new(),
        replayed.version(),
    );
    restored
        .restore_collection_progress(
            owner(),
            phase,
            1,
            1,
            1,
            Digest::new([1; 32]),
            None,
            Some(CollectionDrainReason::RunDataLimit),
            vec![CollectionIterationRestore::checkpointed(
                0,
                member("work"),
                Digest::new([2; 32]),
                member("return"),
                Digest::new([3; 32]),
            )],
        )
        .expect("checkpointed data-limit projection restores");
    restored
}

#[test]
fn checkpointed_open_projection_equals_its_addressed_journal() {
    let run = RunId::generate();
    let replayed = RunState::replay_addressed(run, &checkpointed_events())
        .expect("checkpointed journal replays");
    let restored = restore_from_steps(&replayed, CollectionPhase::Open, None);
    assert_eq!(restored, replayed);
}

#[test]
fn succeeded_projection_equals_its_addressed_journal() {
    let run = RunId::generate();
    let mut events = checkpointed_events();
    events.push(AddressedRunEvent::root(RunEvent::CollectionFinished {
        node: node("items"),
        outcome: CollectionOutcome::Succeeded,
        result_digest: Some(Digest::new([4; 32])),
    }));
    let replayed = RunState::replay_addressed(run, &events).expect("finished journal replays");
    let restored = restore_from_steps(
        &replayed,
        CollectionPhase::Succeeded,
        Some(Digest::new([4; 32])),
    );
    assert_eq!(restored, replayed);
}

#[test]
fn draining_projection_equals_its_addressed_journal() {
    let run = RunId::generate();
    let reason = CollectionDrainReason::Failed {
        iteration: 0,
        code: FailureCode::worker_error(),
    };
    let replayed = RunState::replay_addressed(run, &drained_events(reason.clone()))
        .expect("draining journal replays");
    let restored = restore_drained_from_steps(&replayed, CollectionPhase::Draining, reason);
    assert_eq!(restored, replayed);
}

#[test]
fn failed_projection_equals_its_addressed_journal() {
    let run = RunId::generate();
    let code = FailureCode::worker_error();
    let reason = CollectionDrainReason::Failed {
        iteration: 0,
        code: code.clone(),
    };
    let mut events = drained_events(reason.clone());
    events.push(AddressedRunEvent::root(RunEvent::CollectionFinished {
        node: node("items"),
        outcome: CollectionOutcome::Failed(code),
        result_digest: None,
    }));
    let replayed = RunState::replay_addressed(run, &events).expect("failed journal replays");
    let restored = restore_drained_from_steps(&replayed, CollectionPhase::Failed, reason);
    assert_eq!(restored, replayed);
}

#[test]
fn checkpointed_run_data_limit_projection_equals_its_actual_event_state() {
    let run = RunId::generate();
    let code = FailureCode::run_data_limit();
    let mut events = checkpointed_events();
    events.extend([
        AddressedRunEvent::root(RunEvent::RunStopRequested {
            reason: sagat_domain::execution::StopReason::Failed,
            code: Some(code.clone()),
        }),
        AddressedRunEvent::root(RunEvent::CollectionDrainStarted {
            node: node("items"),
            reason: CollectionDrainReason::RunDataLimit,
        }),
    ]);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event(), RunEvent::CollectionIterationDrained { .. }))
    );

    let draining = RunState::replay_addressed(run, &events).expect("draining journal replays");
    let restored = restore_checkpointed_data_limit_from_steps(&draining, CollectionPhase::Draining);
    assert_eq!(restored, draining);

    events.push(AddressedRunEvent::root(RunEvent::CollectionFinished {
        node: node("items"),
        outcome: CollectionOutcome::Failed(code),
        result_digest: None,
    }));
    let failed = RunState::replay_addressed(run, &events).expect("failed journal replays");
    let restored = restore_checkpointed_data_limit_from_steps(&failed, CollectionPhase::Failed);
    assert_eq!(restored, failed);
}

#[test]
fn cancelled_projection_equals_its_addressed_journal() {
    let run = RunId::generate();
    let reason = CollectionDrainReason::Cancelled;
    let mut events = drained_events(reason.clone());
    events.push(AddressedRunEvent::root(RunEvent::CollectionFinished {
        node: node("items"),
        outcome: CollectionOutcome::Cancelled,
        result_digest: None,
    }));
    let replayed = RunState::replay_addressed(run, &events).expect("cancelled journal replays");
    let restored = restore_drained_from_steps(&replayed, CollectionPhase::Cancelled, reason);
    assert_eq!(restored, replayed);
}

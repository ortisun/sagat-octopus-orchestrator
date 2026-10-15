//! Collection progress is derived only from explicit journal events.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use sagat_domain::execution::{
    AddressedRunEvent, CollectionDrainReason, CollectionOutcome, CollectionPhase, ExecutionError,
    ExecutionInstanceId, FailureCode, Fence, InstancePath, RunEvent, RunId, RunState, StepStatus,
    lease_instance,
};
use sagat_domain::workflow::Digest;
use sagat_domain::workflow::NodeId;

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node")
}

fn collection() -> ExecutionInstanceId {
    ExecutionInstanceId::root(node("items"))
}

fn body(index: u32) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(node("items"), index)
            .expect("bounded path"),
        node("work"),
    )
}

fn started(item_count: u32, max_concurrency: u16) -> RunState {
    let mut state = RunState::new(RunId::generate());
    state
        .apply(&RunEvent::RunStarted {
            entrypoint: node("items"),
        })
        .expect("run starts");
    state
        .apply(&RunEvent::CollectionEntered {
            node: node("items"),
            item_count,
            max_items: item_count.max(1),
            max_concurrency,
            snapshot_digest: Digest::new([0x51; 32]),
        })
        .expect("collection enters");
    state
}

fn admit(state: &mut RunState, index: u32) {
    state
        .apply(&RunEvent::CollectionIterationAdmitted {
            node: node("items"),
            index,
            entry: body(index),
            item_digest: Digest::new([index as u8; 32]),
        })
        .expect("iteration admitted");
}

fn return_and_checkpoint(state: &mut RunState, index: u32) {
    state
        .apply(&RunEvent::CollectionIterationReturned {
            node: node("items"),
            index,
            return_instance: body(index),
        })
        .expect("body returned");
    state
        .apply(&RunEvent::CollectionIterationCheckpointed {
            node: node("items"),
            index,
            return_instance: body(index),
            result_digest: Digest::new([0x80 + index as u8; 32]),
        })
        .expect("result checkpointed");
}

#[test]
fn equal_body_nodes_at_separate_indices_have_distinct_instances() {
    let mut state = started(2, 2);

    admit(&mut state, 0);
    admit(&mut state, 1);

    assert_ne!(body(0), body(1));
    assert_eq!(
        state.instance(&body(0)).map(|step| step.status()),
        Some(StepStatus::Pending)
    );
    assert_eq!(
        state.instance(&body(1)).map(|step| step.status()),
        Some(StepStatus::Pending)
    );
    assert_eq!(
        state
            .collection(&collection())
            .expect("collection state")
            .active_count(),
        2
    );
}

#[test]
fn checkpoint_results_remain_in_input_order_after_reverse_completion() {
    let mut state = started(2, 2);
    admit(&mut state, 0);
    admit(&mut state, 1);

    return_and_checkpoint(&mut state, 1);
    return_and_checkpoint(&mut state, 0);

    let results = state
        .collection(&collection())
        .expect("collection state")
        .ordered_results()
        .map(|(index, result)| (index, result.return_instance().clone(), result.digest()))
        .collect::<Vec<_>>();
    assert_eq!(
        results,
        vec![
            (0, body(0), Digest::new([0x80; 32])),
            (1, body(1), Digest::new([0x81; 32])),
        ]
    );
    let scope = state.collection(&collection()).expect("collection state");
    assert_eq!(scope.snapshot_digest(), Digest::new([0x51; 32]));
    assert_eq!(scope.item_digest(0), Some(Digest::new([0; 32])));
    assert_eq!(scope.item_digest(1), Some(Digest::new([1; 32])));
}

#[test]
fn duplicate_admission_is_rejected_without_changing_state() {
    let mut state = started(2, 2);
    admit(&mut state, 0);
    let before = state.clone();

    let result = state.apply(&RunEvent::CollectionIterationAdmitted {
        node: node("items"),
        index: 0,
        entry: body(0),
        item_digest: Digest::new([0; 32]),
    });

    assert_eq!(
        result,
        Err(ExecutionError::DuplicateCollectionAdmission {
            collection: collection(),
            index: 0,
        })
    );
    assert_eq!(state, before);
}

#[test]
fn concurrency_one_requires_a_durable_checkpoint_before_next_admission() {
    let mut state = started(2, 1);
    admit(&mut state, 0);
    let before = state.clone();

    assert_eq!(
        state.apply(&RunEvent::CollectionIterationAdmitted {
            node: node("items"),
            index: 1,
            entry: body(1),
            item_digest: Digest::new([1; 32]),
        }),
        Err(ExecutionError::CollectionConcurrencyExceeded {
            collection: collection(),
            limit: 1,
        })
    );
    assert_eq!(state, before);

    return_and_checkpoint(&mut state, 0);
    admit(&mut state, 1);
    assert_eq!(
        state
            .collection(&collection())
            .expect("collection state")
            .active_count(),
        1
    );
}

#[test]
fn replay_rejects_a_return_without_its_admission_event() {
    let run = RunId::generate();
    let events = [
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: node("items"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: node("items"),
            item_count: 1,
            max_items: 1,
            max_concurrency: 1,
            snapshot_digest: Digest::new([0x51; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationReturned {
            node: node("items"),
            index: 0,
            return_instance: body(0),
        }),
    ];

    assert_eq!(
        RunState::replay_addressed(run, &events),
        Err(ExecutionError::UnknownCollectionIteration {
            collection: collection(),
            index: 0,
        })
    );
}

#[test]
fn replay_uses_only_admitted_instances_and_recorded_checkpoints() {
    let run = RunId::generate();
    let events = [
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: node("items"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: node("items"),
            item_count: 1,
            max_items: 1,
            max_concurrency: 1,
            snapshot_digest: Digest::new([0x51; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationAdmitted {
            node: node("items"),
            index: 0,
            entry: body(0),
            item_digest: Digest::new([0; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationReturned {
            node: node("items"),
            index: 0,
            return_instance: body(0),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationCheckpointed {
            node: node("items"),
            index: 0,
            return_instance: body(0),
            result_digest: Digest::new([0x80; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionFinished {
            node: node("items"),
            outcome: CollectionOutcome::Succeeded,
            result_digest: Some(Digest::new([0x90; 32])),
        }),
    ];

    let state = RunState::replay_addressed(run, &events).expect("events replay");
    let scope = state.collection(&collection()).expect("collection state");
    assert_eq!(scope.phase(), CollectionPhase::Succeeded);
    assert_eq!(scope.result_digest(), Some(Digest::new([0x90; 32])));
    assert_eq!(
        scope
            .ordered_results()
            .map(|(index, result)| (index, result.return_instance().clone()))
            .collect::<Vec<_>>(),
        vec![(0, body(0))]
    );
    assert_eq!(
        state.step(&node("items")).map(|step| step.status()),
        Some(StepStatus::Succeeded)
    );
}

#[test]
fn permanent_failure_stops_admission_and_finishes_only_after_drain() {
    let mut state = started(3, 2);
    admit(&mut state, 0);
    admit(&mut state, 1);
    return_and_checkpoint(&mut state, 0);

    state
        .apply(&RunEvent::CollectionDrainStarted {
            node: node("items"),
            reason: CollectionDrainReason::Failed {
                iteration: 1,
                code: FailureCode::worker_error(),
            },
        })
        .expect("drain starts");
    assert_eq!(
        state.apply(&RunEvent::CollectionIterationAdmitted {
            node: node("items"),
            index: 2,
            entry: body(2),
            item_digest: Digest::new([2; 32]),
        }),
        Err(ExecutionError::CollectionNotOpen {
            collection: collection(),
            phase: CollectionPhase::Draining,
        })
    );

    state
        .apply(&RunEvent::CollectionIterationDrained {
            node: node("items"),
            index: 1,
        })
        .expect("accepted iteration drains");
    state
        .apply(&RunEvent::CollectionFinished {
            node: node("items"),
            outcome: CollectionOutcome::Failed(FailureCode::worker_error()),
            result_digest: None,
        })
        .expect("collection fails after drain");

    assert_eq!(
        state.collection(&collection()).map(|scope| scope.phase()),
        Some(CollectionPhase::Failed)
    );
    assert_eq!(
        state.step(&node("items")).map(|step| step.status()),
        Some(StepStatus::Failed)
    );
}

#[test]
fn empty_collection_finishes_without_allocating_an_iteration() {
    let mut state = started(0, 1);

    state
        .apply(&RunEvent::CollectionFinished {
            node: node("items"),
            outcome: CollectionOutcome::Succeeded,
            result_digest: Some(Digest::new([0x90; 32])),
        })
        .expect("empty collection succeeds");

    let scope = state.collection(&collection()).expect("collection state");
    assert_eq!(scope.phase(), CollectionPhase::Succeeded);
    assert_eq!(scope.active_count(), 0);
    assert_eq!(scope.ordered_results().count(), 0);
}

#[test]
fn collection_finish_requires_a_digest_only_for_success() {
    let mut success = started(0, 1);
    assert_eq!(
        success.apply(&RunEvent::CollectionFinished {
            node: node("items"),
            outcome: CollectionOutcome::Succeeded,
            result_digest: None,
        }),
        Err(ExecutionError::IllegalCollectionTransition {
            collection: collection(),
            event: "collection.finished",
        })
    );

    let mut cancelled = started(0, 1);
    cancelled
        .apply(&RunEvent::CollectionDrainStarted {
            node: node("items"),
            reason: CollectionDrainReason::Cancelled,
        })
        .expect("cancellation starts drain");
    assert_eq!(
        cancelled.apply(&RunEvent::CollectionFinished {
            node: node("items"),
            outcome: CollectionOutcome::Cancelled,
            result_digest: Some(Digest::new([0x90; 32])),
        }),
        Err(ExecutionError::IllegalCollectionTransition {
            collection: collection(),
            event: "collection.finished",
        })
    );
}

#[test]
fn cancellation_drains_only_admitted_work_before_finishing() {
    let mut state = started(2, 1);
    admit(&mut state, 0);

    state
        .apply(&RunEvent::CollectionDrainStarted {
            node: node("items"),
            reason: CollectionDrainReason::Cancelled,
        })
        .expect("cancellation starts drain");
    state
        .apply(&RunEvent::CollectionIterationDrained {
            node: node("items"),
            index: 0,
        })
        .expect("accepted iteration drains");
    state
        .apply(&RunEvent::CollectionFinished {
            node: node("items"),
            outcome: CollectionOutcome::Cancelled,
            result_digest: None,
        })
        .expect("cancelled collection finishes");

    assert_eq!(
        state.collection(&collection()).map(|scope| scope.phase()),
        Some(CollectionPhase::Cancelled)
    );
    assert!(state.instance(&body(1)).is_none());
}

#[test]
fn drain_refuses_to_hide_an_attempt_that_is_still_in_flight() {
    let mut state = started(1, 1);
    admit(&mut state, 0);
    state
        .apply_addressed(
            &AddressedRunEvent::in_path(
                body(0).path().clone(),
                RunEvent::StepReleased { node: node("work") },
            )
            .expect("scoped release"),
        )
        .expect("body becomes ready");
    lease_instance(&mut state, &body(0)).expect("attempt receives a lease");
    state
        .apply(&RunEvent::CollectionDrainStarted {
            node: node("items"),
            reason: CollectionDrainReason::Cancelled,
        })
        .expect("cancellation starts drain");
    let before = state.clone();

    assert_eq!(
        state.apply(&RunEvent::CollectionIterationDrained {
            node: node("items"),
            index: 0,
        }),
        Err(ExecutionError::IllegalCollectionTransition {
            collection: collection(),
            event: "collection.iteration_drained",
        })
    );
    assert_eq!(state, before);
}

#[test]
fn drain_refuses_to_hide_failed_derived_obligations() {
    for derived_node in [node("work").compensation(), node("work").reconciliation()] {
        let mut state = started(1, 1);
        admit(&mut state, 0);
        let path = body(0).path().clone();
        for event in [
            RunEvent::StepReleased { node: node("work") },
            RunEvent::StepLeased {
                node: node("work"),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepStarted {
                node: node("work"),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepCompleted {
                node: node("work"),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepRouted {
                node: node("work"),
                activated: vec![derived_node.clone()],
            },
            RunEvent::StepReleased {
                node: derived_node.clone(),
            },
            RunEvent::StepLeased {
                node: derived_node.clone(),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepStarted {
                node: derived_node.clone(),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepFailed {
                node: derived_node.clone(),
                code: FailureCode::worker_error(),
                fence: Some(Fence {
                    attempt: 1,
                    generation: 1,
                }),
            },
        ] {
            state
                .apply_addressed(
                    &AddressedRunEvent::in_path(path.clone(), event).expect("scoped event"),
                )
                .expect("prepare failed derived obligation");
        }
        let reason = CollectionDrainReason::Failed {
            iteration: 0,
            code: FailureCode::worker_error(),
        };
        state
            .apply(&RunEvent::CollectionDrainStarted {
                node: node("items"),
                reason: reason.clone(),
            })
            .expect("collection begins draining");
        let before = state.clone();

        assert_eq!(
            state.apply(&RunEvent::CollectionIterationDrained {
                node: node("items"),
                index: 0,
            }),
            Err(ExecutionError::IllegalCollectionTransition {
                collection: collection(),
                event: "collection.iteration_drained",
            })
        );
        assert_eq!(state, before);

        let mut restored = RunState::restore_instances(
            state.run_id(),
            state.status(),
            state
                .stop_reason()
                .map(|stop| (stop, state.stop_code().cloned())),
            state.is_paused(),
            state
                .instances()
                .map(|(instance, step)| (instance.clone(), step.clone()))
                .collect::<BTreeMap<_, _>>(),
            BTreeMap::new(),
            state.version(),
        );
        assert_eq!(
            restored.restore_collection_progress(
                collection(),
                CollectionPhase::Draining,
                1,
                1,
                1,
                Digest::new([0x51; 32]),
                None,
                Some(reason),
                vec![
                    sagat_domain::execution::CollectionIterationRestore::drained(
                        0,
                        body(0),
                        Digest::new([0; 32]),
                    )
                ],
            ),
            Err(ExecutionError::IllegalCollectionTransition {
                collection: collection(),
                event: "collection.restore",
            })
        );
        assert!(restored.collection(&collection()).is_none());
    }
}

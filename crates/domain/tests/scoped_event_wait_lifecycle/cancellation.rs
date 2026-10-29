//! Draining a recorded event suspension never waits for a worker capability it does not own.
use super::*;
use sagat_domain::execution::{CollectionDrainReason, StepState, SwitchSelection};

#[test]
fn a_failed_drain_cannot_manufacture_a_failure_from_a_waiting_event_entry() {
    let mut full = state();
    full.apply_addressed(&started())
        .expect("actual subscribed entry");
    full.apply(&RunEvent::CollectionDrainStarted {
        node: id("lines"),
        reason: CollectionDrainReason::Failed {
            iteration: 0,
            code: FailureCode::expression_error(),
        },
    })
    .expect("record inconsistent failed-entry request");
    let before = full.clone();
    assert!(
        full.apply(&RunEvent::CollectionIterationDrained {
            node: id("lines"),
            index: 0
        })
        .is_err(),
        "a Waiting event cannot become a generic failed admission"
    );
    assert_eq!(full, before, "refusal precedes every member mutation");
}

#[test]
fn a_failed_drain_keeps_the_existing_pending_entry_failure_shape() {
    let mut full = state();
    full.apply(&RunEvent::CollectionDrainStarted {
        node: id("lines"),
        reason: CollectionDrainReason::Failed {
            iteration: 0,
            code: FailureCode::expression_error(),
        },
    })
    .expect("failed pending entry");
    full.apply(&RunEvent::CollectionIterationDrained {
        node: id("lines"),
        index: 0,
    })
    .expect("pending entry keeps its existing failure handling");
    let step = full.instance(&wait(0)).expect("failed entry");
    assert_eq!(step.status(), StepStatus::Failed);
    assert_eq!(step.failure(), Some(&FailureCode::expression_error()));
    assert!(step.routed());
    assert!(step.waiting_on().is_none());
}

#[test]
fn collection_drain_cancels_an_event_suspension_after_user_cancel_or_a_sibling_failure() {
    for failed_sibling in [false, true] {
        let def = definition();
        let mut full = state();
        full.apply_addressed(&started())
            .expect("recorded waiting member");
        let reason = if failed_sibling {
            start_event_wait_instance(
                &def,
                &mut full,
                &wait(1),
                Err(()),
                OffsetDateTime::UNIX_EPOCH,
            )
            .expect("actual sibling expression failure");
            CollectionDrainReason::Failed {
                iteration: 1,
                code: FailureCode::expression_error(),
            }
        } else {
            CollectionDrainReason::Cancelled
        };
        full.apply(&RunEvent::CollectionDrainStarted {
            node: id("lines"),
            reason,
        })
        .expect("drain begins");
        let sibling = full.instance(&wait(1)).expect("sibling").clone();
        full.apply(&RunEvent::CollectionIterationDrained {
            node: id("lines"),
            index: 0,
        })
        .expect("a suspension owns no worker and can drain");
        let step = full.instance(&wait(0)).expect("cancelled wait");
        assert_eq!(step.status(), StepStatus::Cancelled);
        assert!(step.routed());
        assert!(step.waiting_on().is_none());
        assert!(step.failure().is_none());
        assert!(
            step.is_valid_scoped_event_wait(def.node_for_instance(&wait(0)).expect("pinned wait"))
        );
        assert_eq!(full.instance(&wait(1)), Some(&sibling));
    }
}

#[test]
fn collection_drain_refuses_malformed_event_suspensions_without_partial_mutation() {
    let now = OffsetDateTime::UNIX_EPOCH;
    for field in 0..9 {
        let source = state();
        let damaged = StepState::restore(
            StepStatus::Waiting,
            u32::from(field == 0),
            u64::from(field == 1),
            field == 2,
            field == 3,
            (field == 4).then_some(now),
            Some(WaitingOn::Event {
                correlation_key: CorrelationKey::parse("key").expect("key"),
                deadline: now,
            }),
            (field == 5).then(FailureCode::expression_error),
            (field == 6).then_some(false),
            field == 7,
        )
        .with_restored_switch_selection((field == 8).then_some(SwitchSelection::Default));
        let mut full = restored_member_projection(&source, damaged);
        full.apply(&RunEvent::CollectionDrainStarted {
            node: id("lines"),
            reason: CollectionDrainReason::Cancelled,
        })
        .expect("drain starts");
        let before = full.clone();
        assert!(
            full.apply(&RunEvent::CollectionIterationDrained {
                node: id("lines"),
                index: 0
            })
            .is_err(),
            "field={field}"
        );
        assert_eq!(full, before, "field={field}");
    }
}

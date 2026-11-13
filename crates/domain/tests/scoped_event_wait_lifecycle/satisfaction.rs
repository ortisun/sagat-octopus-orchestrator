//! Pure matching/window commands prepare an outcome; they grant no receipt/payload authority.
use super::*;
use sagat_domain::execution::{
    satisfy_event_wait_instance, satisfy_event_wait_path_instance, scoped_event_wait_since,
};

#[test]
fn reviewed_routing_failure_does_not_partially_apply_either_satisfaction_command() {
    use sagat_domain::execution::StepState;
    let def = definition();
    let now = OffsetDateTime::UNIX_EPOCH;
    let key = CorrelationKey::parse("payment-1").expect("key");
    let kind = EventType::parse("payment.notified").expect("type");
    let mut source = state();
    start_event_wait_instance(&def, &mut source, &wait(0), Ok(key.clone()), now).expect("start");
    let pending = StepState::restore(
        StepStatus::Pending,
        0,
        0,
        false,
        false,
        None,
        None,
        None,
        None,
        false,
    );
    let mut full = restored_local_projection(
        &source,
        ExecutionInstanceId::in_path(path(0), id("returned")),
        pending,
    );
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("malformed route source");
    let before = full.clone();
    let bounded_before = bounded.clone();
    let full_result = satisfy_event_wait_instance(&def, &mut full, &wait(0), &kind, &key, now);
    let path_result =
        satisfy_event_wait_path_instance(&def, &mut bounded, &wait(0), &kind, &key, now);
    assert!(
        full_result.is_err() && path_result.is_err(),
        "duplicate successor is an invalid route"
    );
    assert_eq!(
        (&full, &bounded),
        (&before, &bounded_before),
        "no WaitSatisfied may survive a later routing error"
    );
}

#[test]
fn reviewed_root_and_unadmitted_candidates_have_typed_full_path_error_parity() {
    use sagat_domain::execution::ExecutionError;
    let def = definition();
    let now = OffsetDateTime::UNIX_EPOCH;
    let key = CorrelationKey::parse("payment-1").expect("key");
    let kind = EventType::parse("payment.notified").expect("type");
    let mut full = state();
    full.apply_addressed(&started()).expect("recorded source");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let before = full.clone();
    let bounded_before = bounded.clone();
    for candidate in [ExecutionInstanceId::root(id("notified")), wait(2)] {
        let full_result =
            satisfy_event_wait_instance(&def, &mut full, &candidate, &kind, &key, now);
        let path_result =
            satisfy_event_wait_path_instance(&def, &mut bounded, &candidate, &kind, &key, now);
        let expected = Err(ExecutionError::UnknownInstance(candidate));
        assert_eq!((full_result, path_result), (expected.clone(), expected));
        assert_eq!(&full, &before);
        assert_eq!(&bounded, &bounded_before);
    }
}

#[test]
fn a_matching_event_at_the_window_boundary_records_the_exact_member_and_normal_route() {
    let def = definition();
    let mut full = state();
    let now = OffsetDateTime::UNIX_EPOCH;
    let key = CorrelationKey::parse("payment-1").expect("key");
    let kind = EventType::parse("payment.notified").expect("type");
    start_event_wait_instance(&def, &mut full, &wait(0), Ok(key.clone()), now).expect("start");
    let sibling = full.instance(&wait(1)).expect("sibling").clone();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let progress = satisfy_event_wait_instance(&def, &mut full, &wait(0), &kind, &key, now)
        .expect("matching full outcome");
    assert_eq!(
        progress,
        satisfy_event_wait_path_instance(&def, &mut bounded, &wait(0), &kind, &key, now)
            .expect("matching bounded outcome")
    );
    assert_eq!(
        progress.events,
        vec![
            event(RunEvent::WaitSatisfied {
                node: id("notified")
            }),
            event(RunEvent::StepRouted {
                node: id("notified"),
                activated: vec![id("returned")]
            })
        ]
    );
    assert_eq!(full.instance(&wait(1)), Some(&sibling));
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("oracle")
    );
}

#[test]
fn the_scoped_window_uses_the_recorded_deadline_and_pinned_timeout() {
    let def = definition();
    let node = def.node_for_instance(&wait(0)).expect("pinned declaration");
    assert_eq!(
        scoped_event_wait_since(
            node,
            OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(60)
        )
        .expect("canonical boundary"),
        OffsetDateTime::UNIX_EPOCH
    );
}

#[test]
fn wrong_type_key_and_early_receipt_are_typed_refusals_without_mutation() {
    use sagat_domain::execution::ExecutionError;
    let def = definition();
    let now = OffsetDateTime::UNIX_EPOCH;
    let kind = EventType::parse("payment.notified").expect("type");
    let key = CorrelationKey::parse("payment-1").expect("key");
    let mut source = state();
    start_event_wait_instance(&def, &mut source, &wait(0), Ok(key.clone()), now)
        .expect("actual waiting source");
    for (candidate_type, candidate_key, received_at) in [
        (
            EventType::parse("another.event").expect("different type"),
            key.clone(),
            now,
        ),
        (
            kind.clone(),
            CorrelationKey::parse("another-key").expect("different key"),
            now,
        ),
        (
            kind.clone(),
            key.clone(),
            now - time::Duration::microseconds(1),
        ),
    ] {
        let mut full = source.clone();
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        let before = full.clone();
        let bounded_before = bounded.clone();
        assert_eq!(
            satisfy_event_wait_instance(
                &def,
                &mut full,
                &wait(0),
                &candidate_type,
                &candidate_key,
                received_at
            ),
            Err(ExecutionError::EventNotMatching { instance: wait(0) })
        );
        assert_eq!(
            satisfy_event_wait_path_instance(
                &def,
                &mut bounded,
                &wait(0),
                &candidate_type,
                &candidate_key,
                received_at
            ),
            Err(ExecutionError::EventNotMatching { instance: wait(0) })
        );
        assert_eq!(full, before);
        assert_eq!(bounded, bounded_before);
    }
}

#[test]
fn lookback_is_inclusive_and_events_can_win_after_the_deadline_until_timeout_commits() {
    use sagat_domain::execution::{ExecutionError, time_out_event_wait_instance};
    let def = definition_with_window(false, true, Duration::from_secs(5));
    let now = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(10);
    let key = CorrelationKey::parse("payment-1").expect("key");
    let kind = EventType::parse("payment.notified").expect("type");
    for offset in [-1_i64, 0, 1, 70_000_000] {
        let mut full = state();
        start_event_wait_instance(&def, &mut full, &wait(0), Ok(key.clone()), now).expect("start");
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        let before = full.clone();
        let bounded_before = bounded.clone();
        let received_at = now - time::Duration::seconds(5) + time::Duration::microseconds(offset);
        let outcome =
            satisfy_event_wait_instance(&def, &mut full, &wait(0), &kind, &key, received_at);
        assert_eq!(
            outcome,
            satisfy_event_wait_path_instance(
                &def,
                &mut bounded,
                &wait(0),
                &kind,
                &key,
                received_at
            )
        );
        if offset < 0 {
            assert_eq!(
                outcome,
                Err(ExecutionError::EventNotMatching { instance: wait(0) })
            );
            assert_eq!(full, before);
            assert_eq!(bounded, bounded_before);
        } else {
            outcome.expect("inclusive accepted window with no upper deadline");
            let before = full.clone();
            assert!(
                time_out_event_wait_instance(
                    &def,
                    &mut full,
                    &wait(0),
                    now + time::Duration::days(1)
                )
                .is_err()
            );
            assert_eq!(full, before);
        }
    }
    let mut full = state();
    start_event_wait_instance(&def, &mut full, &wait(0), Ok(key.clone()), now).expect("start");
    time_out_event_wait_instance(&def, &mut full, &wait(0), now + time::Duration::seconds(60))
        .expect("timeout wins first");
    let before = full.clone();
    assert!(
        satisfy_event_wait_instance(
            &def,
            &mut full,
            &wait(0),
            &kind,
            &key,
            now + time::Duration::seconds(61)
        )
        .is_err()
    );
    assert_eq!(full, before);
}

#[test]
fn paused_satisfaction_records_only_the_outcome_and_cold_resume_selects_normal_next_once() {
    let def = definition();
    let mut full = state();
    let now = OffsetDateTime::UNIX_EPOCH;
    let key = CorrelationKey::parse("payment-1").expect("key");
    let kind = EventType::parse("payment.notified").expect("type");
    start_event_wait_instance(&def, &mut full, &wait(0), Ok(key.clone()), now).expect("start");
    full.apply(&RunEvent::RunPaused).expect("pause");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("paused path");
    let progress = satisfy_event_wait_instance(&def, &mut full, &wait(0), &kind, &key, now)
        .expect("paused full outcome");
    assert_eq!(
        progress,
        satisfy_event_wait_path_instance(&def, &mut bounded, &wait(0), &kind, &key, now)
            .expect("paused path outcome")
    );
    assert_eq!(
        progress.events,
        vec![event(RunEvent::WaitSatisfied {
            node: id("notified")
        })]
    );
    assert!(!full.instance(&wait(0)).expect("deferred wait").routed());
    assert!(full.is_paused());
    let before = full.clone();
    assert!(satisfy_event_wait_instance(&def, &mut full, &wait(0), &kind, &key, now).is_err());
    assert_eq!(full, before);
    full.apply(&RunEvent::RunResumed).expect("resume");
    let mut cold = PathExecutionState::from_run(&full, path(0)).expect("cold resumed path");
    let route = sagat_domain::execution::release_pending_path(&def, &mut cold)
        .expect("cold deferred route");
    assert_eq!(
        route.events,
        vec![event(RunEvent::StepRouted {
            node: id("notified"),
            activated: vec![id("returned")]
        })]
    );
    assert!(
        sagat_domain::execution::release_pending_path(&def, &mut cold)
            .expect("already routed")
            .events
            .is_empty()
    );
}

#[test]
fn stopped_draining_canceled_pending_and_foreign_satisfactions_do_not_mutate_either_model() {
    let def = definition();
    let now = OffsetDateTime::UNIX_EPOCH;
    let key = CorrelationKey::parse("payment-1").expect("key");
    let kind = EventType::parse("payment.notified").expect("type");
    for prefix in [
        vec![],
        vec![
            started(),
            AddressedRunEvent::root(RunEvent::RunStopRequested {
                reason: sagat_domain::execution::StopReason::Cancelled,
                code: None,
            }),
        ],
        vec![
            started(),
            AddressedRunEvent::root(RunEvent::CollectionDrainStarted {
                node: id("lines"),
                reason: sagat_domain::execution::CollectionDrainReason::Cancelled,
            }),
        ],
        vec![
            started(),
            event(RunEvent::StepCancelled {
                node: id("notified"),
            }),
        ],
    ] {
        let mut full = state();
        for recorded in prefix {
            full.apply_addressed(&recorded).expect("guard source");
        }
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        let before = full.clone();
        let bounded_before = bounded.clone();
        assert!(satisfy_event_wait_instance(&def, &mut full, &wait(0), &kind, &key, now).is_err());
        assert!(
            satisfy_event_wait_path_instance(&def, &mut bounded, &wait(0), &kind, &key, now)
                .is_err()
        );
        assert_eq!(full, before);
        assert_eq!(bounded, bounded_before);
    }
    let mut full = state();
    full.apply_addressed(&started()).expect("wait");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let before = full.clone();
    let bounded_before = bounded.clone();
    for candidate in [
        ExecutionInstanceId::root(id("notified")),
        wait(1),
        wait(2),
        ExecutionInstanceId::in_path(path(0), id("fallback")),
    ] {
        assert!(
            satisfy_event_wait_instance(&def, &mut full, &candidate, &kind, &key, now).is_err()
        );
        assert!(
            satisfy_event_wait_path_instance(&def, &mut bounded, &candidate, &kind, &key, now)
                .is_err()
        );
        assert_eq!(full, before);
        assert_eq!(bounded, bounded_before);
    }
}

#[test]
fn malformed_waiting_metadata_refuses_satisfaction_before_recording() {
    use sagat_domain::execution::{StepState, SwitchSelection};
    let def = definition();
    let now = OffsetDateTime::UNIX_EPOCH;
    let key = CorrelationKey::parse("payment-1").expect("key");
    let kind = EventType::parse("payment.notified").expect("type");
    for field in 0..9 {
        let damaged = StepState::restore(
            StepStatus::Waiting,
            u32::from(field == 0),
            u64::from(field == 1),
            field == 2,
            field == 3,
            (field == 4).then_some(now),
            Some(WaitingOn::Event {
                correlation_key: key.clone(),
                deadline: now + time::Duration::seconds(60),
            }),
            (field == 5).then(FailureCode::expression_error),
            (field == 6).then_some(false),
            field == 7,
        )
        .with_restored_switch_selection((field == 8).then_some(SwitchSelection::Default));
        let mut full = restored_member_projection(&state(), damaged);
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        let before = full.clone();
        let bounded_before = bounded.clone();
        assert!(
            satisfy_event_wait_instance(&def, &mut full, &wait(0), &kind, &key, now).is_err(),
            "field={field}"
        );
        assert!(
            satisfy_event_wait_path_instance(&def, &mut bounded, &wait(0), &kind, &key, now)
                .is_err(),
            "field={field}"
        );
        assert_eq!(full, before);
        assert_eq!(bounded, bounded_before);
    }
}

#[test]
fn an_unrepresentable_window_or_a_non_event_declaration_never_authorizes_satisfaction() {
    use sagat_domain::execution::StepState;
    let def = definition();
    let node = def.node_for_instance(&wait(0)).expect("pinned wait");
    let min = time::PrimitiveDateTime::MIN.assume_utc();
    assert!(scoped_event_wait_since(node, min).is_err());
    let timer = Node::new(
        id("notified"),
        NodeKind::Timer {
            delay: Duration::from_secs(1),
        },
    );
    assert!(scoped_event_wait_since(&timer, OffsetDateTime::UNIX_EPOCH).is_err());
    let key = CorrelationKey::parse("payment-1").expect("key");
    let kind = EventType::parse("payment.notified").expect("type");
    let damaged = StepState::restore(
        StepStatus::Waiting,
        0,
        0,
        false,
        false,
        None,
        Some(WaitingOn::Event {
            correlation_key: key.clone(),
            deadline: min,
        }),
        None,
        None,
        false,
    );
    let mut full = restored_member_projection(&state(), damaged);
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let before = full.clone();
    let bounded_before = bounded.clone();
    assert!(
        satisfy_event_wait_instance(
            &def,
            &mut full,
            &wait(0),
            &kind,
            &key,
            OffsetDateTime::UNIX_EPOCH
        )
        .is_err()
    );
    assert!(
        satisfy_event_wait_path_instance(
            &def,
            &mut bounded,
            &wait(0),
            &kind,
            &key,
            OffsetDateTime::UNIX_EPOCH
        )
        .is_err()
    );
    assert_eq!(full, before);
    assert_eq!(bounded, bounded_before);
}

proptest::proptest! {
    #[test]
    fn received_time_boundary_matches_in_full_and_path_models(
        index in 0_u32..2,
        offset in -1_000_000_i64..1_000_000,
        lookback in 0_u64..2000,
        delta in -1_i64..=1,
    ) {
        let def=definition_with_window(false,true,Duration::from_millis(lookback));let now=OffsetDateTime::UNIX_EPOCH+time::Duration::microseconds(offset);
        let key=CorrelationKey::parse("payment-1").expect("key");let kind=EventType::parse("payment.notified").expect("type");let mut full=state();
        start_event_wait_instance(&def,&mut full,&wait(index),Ok(key.clone()),now).expect("start");let mut bounded=PathExecutionState::from_run(&full,path(index)).expect("path");let before=full.clone();let bounded_before=bounded.clone();
        let at=now-time::Duration::milliseconds(i64::try_from(lookback).expect("bounded lookback"))+time::Duration::microseconds(delta);
        let result=satisfy_event_wait_instance(&def,&mut full,&wait(index),&kind,&key,at);
        proptest::prop_assert_eq!(&result,&satisfy_event_wait_path_instance(&def,&mut bounded,&wait(index),&kind,&key,at));
        if delta<0 {proptest::prop_assert!(result.is_err());proptest::prop_assert_eq!(full,before);proptest::prop_assert_eq!(bounded,bounded_before);} else {proptest::prop_assert!(result.is_ok());proptest::prop_assert_eq!(bounded,PathExecutionState::from_run(&full,path(index)).expect("oracle"));proptest::prop_assert_eq!(full.instance(&wait(1-index)),before.instance(&wait(1-index)));}
    }
}

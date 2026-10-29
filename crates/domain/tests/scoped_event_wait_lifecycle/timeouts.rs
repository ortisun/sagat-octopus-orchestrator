//! Due-time decisions use the same local replay fixtures without enabling a native producer.
use super::*;
use sagat_domain::execution::{
    ExecutionError, time_out_event_wait_instance, time_out_event_wait_path_instance,
};

#[test]
fn timeout_uses_recorded_deadline_and_pinned_local_timeout_route() {
    let def = definition();
    let mut full = state();
    let now = OffsetDateTime::UNIX_EPOCH;
    start_event_wait_instance(
        &def,
        &mut full,
        &wait(0),
        Ok(CorrelationKey::parse("key").expect("key")),
        now,
    )
    .expect("start");
    let sibling = full.instance(&wait(1)).expect("sibling").clone();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let before = full.clone();
    let bounded_before = bounded.clone();
    let early = now + time::Duration::seconds(59);
    assert!(matches!(
        time_out_event_wait_instance(&def, &mut full, &wait(0), early),
        Err(ExecutionError::NotYetDue(_))
    ));
    assert!(matches!(
        time_out_event_wait_path_instance(&def, &mut bounded, &wait(0), early),
        Err(ExecutionError::NotYetDue(_))
    ));
    assert_eq!(full, before);
    assert_eq!(bounded, bounded_before);
    let due = now + time::Duration::seconds(60);
    let progress =
        time_out_event_wait_instance(&def, &mut full, &wait(0), due).expect("due timeout");
    assert_eq!(
        progress,
        time_out_event_wait_path_instance(&def, &mut bounded, &wait(0), due)
            .expect("bounded timeout")
    );
    assert_eq!(
        progress.events,
        vec![
            event(RunEvent::WaitTimedOut {
                node: id("notified"),
                handled: true
            }),
            event(RunEvent::StepRouted {
                node: id("notified"),
                activated: vec![id("fallback")]
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
fn paused_wait_records_timeout_but_routes_only_after_explicit_resume() {
    use sagat_domain::execution::{release_pending_instance_path, release_pending_path};
    let def = definition();
    let mut full = state();
    full.apply_addressed(&started())
        .expect("recorded waiting member");
    full.apply(&RunEvent::RunPaused).expect("pause");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("paused path");
    let now = OffsetDateTime::UNIX_EPOCH;
    let progress =
        time_out_event_wait_instance(&def, &mut full, &wait(0), now).expect("due paused timeout");
    assert_eq!(
        progress,
        time_out_event_wait_path_instance(&def, &mut bounded, &wait(0), now)
            .expect("bounded paused timeout")
    );
    assert_eq!(
        progress.events,
        vec![event(RunEvent::WaitTimedOut {
            node: id("notified"),
            handled: true
        })]
    );
    assert!(!full.instance(&wait(0)).expect("timeout").routed());
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("paused oracle")
    );
    full.apply(&RunEvent::RunResumed).expect("resume");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("resumed path");
    let routed =
        release_pending_instance_path(&def, &mut full, &path(0)).expect("route committed timeout");
    assert_eq!(
        routed,
        release_pending_path(&def, &mut bounded).expect("bounded resume")
    );
    assert_eq!(
        routed.events,
        vec![event(RunEvent::StepRouted {
            node: id("notified"),
            activated: vec![id("fallback")]
        })]
    );
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("resumed oracle")
    );
}

#[test]
fn missing_timeout_route_records_typed_failure_and_only_its_declared_error_route() {
    for local_error in [false, true] {
        let def = definition_with_routes(local_error, false);
        let mut full = state();
        full.apply_addressed(&started()).expect("waiting member");
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        let progress =
            time_out_event_wait_instance(&def, &mut full, &wait(0), OffsetDateTime::UNIX_EPOCH)
                .expect("unhandled timeout");
        assert_eq!(
            progress,
            time_out_event_wait_path_instance(
                &def,
                &mut bounded,
                &wait(0),
                OffsetDateTime::UNIX_EPOCH
            )
            .expect("bounded unhandled timeout")
        );
        assert_eq!(
            progress.events,
            vec![
                event(RunEvent::WaitTimedOut {
                    node: id("notified"),
                    handled: false
                }),
                event(RunEvent::StepRouted {
                    node: id("notified"),
                    activated: if local_error {
                        vec![id("fallback")]
                    } else {
                        vec![]
                    }
                })
            ]
        );
        assert_eq!(
            full.instance(&wait(0)).expect("failure").failure(),
            Some(&FailureCode::wait_timed_out())
        );
        assert!(!full.instance(&wait(0)).expect("failure").timed_out());
        assert_eq!(
            bounded,
            PathExecutionState::from_run(&full, path(0)).expect("oracle")
        );
    }
}

#[test]
fn recorded_event_and_timeout_are_mutually_exclusive_in_either_pure_order() {
    for event_wins in [true, false] {
        let def = definition();
        let mut full = state();
        full.apply_addressed(&started()).expect("waiting member");
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        let received = event(RunEvent::WaitSatisfied {
            node: id("notified"),
        });
        let late = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1);
        if event_wins {
            full.apply_addressed(&received)
                .expect("recorded event wins");
            bounded
                .apply_addressed(&received)
                .expect("bounded event wins");
        } else {
            let progress = time_out_event_wait_instance(&def, &mut full, &wait(0), late)
                .expect("timeout wins");
            assert_eq!(
                progress,
                time_out_event_wait_path_instance(&def, &mut bounded, &wait(0), late)
                    .expect("bounded timeout wins")
            );
        }
        let before = full.clone();
        let bounded_before = bounded.clone();
        assert!(time_out_event_wait_instance(&def, &mut full, &wait(0), late).is_err());
        assert!(time_out_event_wait_path_instance(&def, &mut bounded, &wait(0), late).is_err());
        if !event_wins {
            assert!(full.apply_addressed(&received).is_err());
            assert!(bounded.apply_addressed(&received).is_err());
        }
        assert_eq!(full, before);
        assert_eq!(bounded, bounded_before);
        assert_eq!(
            bounded,
            PathExecutionState::from_run(&full, path(0)).expect("oracle")
        );
    }
}

#[test]
fn timeout_refuses_pending_canceled_stopped_draining_and_foreign_sources_without_mutation() {
    use sagat_domain::execution::{CollectionDrainReason, StopReason};
    let cases = [
        (vec![], wait(0)),
        (
            vec![
                started(),
                event(RunEvent::StepCancelled {
                    node: id("notified"),
                }),
            ],
            wait(0),
        ),
        (
            vec![
                started(),
                AddressedRunEvent::root(RunEvent::RunStopRequested {
                    reason: StopReason::Cancelled,
                    code: None,
                }),
            ],
            wait(0),
        ),
        (
            vec![
                started(),
                AddressedRunEvent::root(RunEvent::CollectionDrainStarted {
                    node: id("lines"),
                    reason: CollectionDrainReason::Cancelled,
                }),
            ],
            wait(0),
        ),
        (vec![started()], ExecutionInstanceId::root(id("notified"))),
        (vec![started()], wait(2)),
    ];
    for (prefix, candidate) in cases {
        let def = definition();
        let mut full = state();
        for recorded in prefix {
            full.apply_addressed(&recorded).expect("source fixture");
        }
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        let before = full.clone();
        let bounded_before = bounded.clone();
        assert!(
            time_out_event_wait_instance(&def, &mut full, &candidate, OffsetDateTime::UNIX_EPOCH)
                .is_err()
        );
        assert!(
            time_out_event_wait_path_instance(
                &def,
                &mut bounded,
                &candidate,
                OffsetDateTime::UNIX_EPOCH
            )
            .is_err()
        );
        assert_eq!(full, before);
        assert_eq!(bounded, bounded_before);
    }
}

#[test]
fn timeout_requires_the_declared_event_kind_and_recorded_event_binding() {
    let def = definition();
    let mut full = state();
    full.apply_addressed(&event(RunEvent::TimerStarted {
        node: id("notified"),
        due_at: OffsetDateTime::UNIX_EPOCH,
    }))
    .expect("wrong binding fixture");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let before = full.clone();
    let bounded_before = bounded.clone();
    assert!(matches!(
        time_out_event_wait_instance(&def, &mut full, &wait(0), OffsetDateTime::UNIX_EPOCH),
        Err(ExecutionError::Invariant(
            "event timeout requires an event binding"
        ))
    ));
    assert!(matches!(
        time_out_event_wait_path_instance(&def, &mut bounded, &wait(0), OffsetDateTime::UNIX_EPOCH),
        Err(ExecutionError::Invariant(
            "event timeout requires an event binding"
        ))
    ));
    assert_eq!(full, before);
    assert_eq!(bounded, bounded_before);
    let mut full = state();
    full.apply_addressed(&started()).expect("event wait");
    time_out_event_wait_instance(&def, &mut full, &wait(0), OffsetDateTime::UNIX_EPOCH)
        .expect("activate fallback timer");
    full.apply_addressed(&event(RunEvent::TimerStarted {
        node: id("fallback"),
        due_at: OffsetDateTime::UNIX_EPOCH,
    }))
    .expect("real pending timer started");
    let timer = ExecutionInstanceId::in_path(path(0), id("fallback"));
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("timer path");
    let before = full.clone();
    let bounded_before = bounded.clone();
    assert!(matches!(
        time_out_event_wait_instance(&def, &mut full, &timer, OffsetDateTime::UNIX_EPOCH),
        Err(ExecutionError::Invariant(
            "event timeout targeted another kind"
        ))
    ));
    assert!(matches!(
        time_out_event_wait_path_instance(&def, &mut bounded, &timer, OffsetDateTime::UNIX_EPOCH),
        Err(ExecutionError::Invariant(
            "event timeout targeted another kind"
        ))
    ));
    assert_eq!(full, before);
    assert_eq!(bounded, bounded_before);
}

#[test]
fn malformed_waiting_metadata_cannot_authorize_a_timeout() {
    use sagat_domain::execution::{StepState, SwitchSelection};
    let now = OffsetDateTime::UNIX_EPOCH;
    for field in 0..9 {
        let mut source = state();
        source.apply_addressed(&started()).expect("waiting source");
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
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("waiting projection");
        let before = full.clone();
        let bounded_before = bounded.clone();
        let def = definition();
        assert!(
            matches!(
                time_out_event_wait_instance(&def, &mut full, &wait(0), now),
                Err(ExecutionError::Invariant(
                    "invalid scoped event waiting state"
                ))
            ),
            "field {field}"
        );
        assert!(
            matches!(
                time_out_event_wait_path_instance(&def, &mut bounded, &wait(0), now),
                Err(ExecutionError::Invariant(
                    "invalid scoped event waiting state"
                ))
            ),
            "bounded field {field}"
        );
        assert_eq!(full, before);
        assert_eq!(bounded, bounded_before);
    }
}

proptest::proptest! {
    #[test]
    fn timeout_boundary_and_recorded_route_match_for_varied_deadlines(
        offset in -1_000_000_i64..1_000_000,
        delta in -1_i64..=1,
        handled in proptest::bool::ANY,
    ) {
        let def=definition_with_routes(false,handled);let mut full=state();
        let now=OffsetDateTime::UNIX_EPOCH+time::Duration::microseconds(offset);
        start_event_wait_instance(&def,&mut full,&wait(0),Ok(CorrelationKey::parse("key").expect("key")),now).expect("start");
        let mut bounded=PathExecutionState::from_run(&full,path(0)).expect("path");
        let before=full.clone();let bounded_before=bounded.clone();
        let at=now+time::Duration::seconds(60)+time::Duration::microseconds(delta);
        let actual=time_out_event_wait_instance(&def,&mut full,&wait(0),at);
        let bounded_actual=time_out_event_wait_path_instance(&def,&mut bounded,&wait(0),at);
        if delta<0 {
            proptest::prop_assert!(matches!(actual,Err(ExecutionError::NotYetDue(_))));
            proptest::prop_assert!(matches!(bounded_actual,Err(ExecutionError::NotYetDue(_))));
            proptest::prop_assert_eq!(full,before);proptest::prop_assert_eq!(bounded,bounded_before);
        } else {
            let progress=actual.expect("due timeout");
            proptest::prop_assert_eq!(&progress,&bounded_actual.expect("bounded due timeout"));
            proptest::prop_assert_eq!(&progress.events[0],&event(RunEvent::WaitTimedOut {node:id("notified"),handled}));
            proptest::prop_assert_eq!(bounded,PathExecutionState::from_run(&full,path(0)).expect("oracle"));
        }
    }
}

#[test]
fn event_lifecycle_projection_checks_the_pinned_timeout_policy_and_terminal_metadata() {
    for timeout_route in [false, true] {
        let def = definition_with_routes(false, timeout_route);
        let member = wait(0);
        let declared = def.node_for_instance(&member).expect("wait declaration");
        let mut full = state();
        assert!(
            full.instance(&member)
                .expect("pending")
                .is_valid_scoped_event_wait(declared)
        );
        full.apply_addressed(&started()).expect("waiting");
        assert!(
            full.instance(&member)
                .expect("waiting")
                .is_valid_scoped_event_wait(declared)
        );
        for outcome in [
            RunEvent::WaitSatisfied {
                node: id("notified"),
            },
            RunEvent::WaitTimedOut {
                node: id("notified"),
                handled: timeout_route,
            },
            RunEvent::StepFailed {
                node: id("notified"),
                code: FailureCode::expression_error(),
                fence: None,
            },
            RunEvent::StepCancelled {
                node: id("notified"),
            },
        ] {
            let mut settled = full.clone();
            settled
                .apply_addressed(&event(outcome))
                .expect("recorded terminal projection");
            assert!(
                settled
                    .instance(&member)
                    .expect("terminal")
                    .is_valid_scoped_event_wait(declared)
            );
        }
        let mut wrong = full.clone();
        wrong
            .apply_addressed(&event(RunEvent::WaitTimedOut {
                node: id("notified"),
                handled: !timeout_route,
            }))
            .expect("malformed policy projection");
        assert!(
            !wrong
                .instance(&member)
                .expect("wrong policy")
                .is_valid_scoped_event_wait(declared)
        );
        let mut wrong = full.clone();
        wrong
            .apply_addressed(&event(RunEvent::StepFailed {
                node: id("notified"),
                code: FailureCode::parse("OTHER_FAILURE").expect("code"),
                fence: None,
            }))
            .expect("unknown failure projection");
        assert!(
            !wrong
                .instance(&member)
                .expect("wrong failure")
                .is_valid_scoped_event_wait(declared)
        );
    }
}

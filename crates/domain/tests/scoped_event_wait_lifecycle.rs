//! Pure recorded event-wait outcomes preserve exact member identity and root branch semantics.
#![allow(clippy::expect_used)]
use sagat_domain::{
    execution::{
        AddressedPlan, AddressedRunEvent, CorrelationKey, ExecutionInstanceId, FailureCode,
        InstancePath, PathExecutionState, RunEvent, RunId, RunState, StepStatus, WaitingOn,
        plan_addressed, plan_path, start_event_wait_instance, start_event_wait_path_instance,
    },
    identity::Slug,
    workflow::{
        BodyDefinition, Digest, EventType, Expression, Node, NodeId, NodeKind, WorkflowDefinition,
        WorkflowLimits,
    },
};
use std::time::Duration;
use time::OffsetDateTime;
fn id(s: &str) -> NodeId {
    NodeId::parse(s).expect("node")
}
fn path(index: u32) -> InstancePath {
    InstancePath::root()
        .child(id("lines"), index)
        .expect("path")
}
fn wait(index: u32) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(path(index), id("notified"))
}

fn definition() -> WorkflowDefinition {
    definition_with_error_route(false)
}

fn definition_with_error_route(local_error: bool) -> WorkflowDefinition {
    definition_with_routes(local_error, true)
}

fn definition_with_routes(local_error: bool, timeout_route: bool) -> WorkflowDefinition {
    definition_with_window(local_error, timeout_route, Duration::ZERO)
}

fn definition_with_window(
    local_error: bool,
    timeout_route: bool,
    lookback: Duration,
) -> WorkflowDefinition {
    let waiting = Node::new(
        id("notified"),
        NodeKind::WaitEvent {
            event_type: EventType::parse("payment.notified").expect("event type"),
            correlation: Expression::parse("item.key").expect("correlation"),
            timeout: Duration::from_secs(60),
            on_timeout: timeout_route.then(|| id("fallback")),
            lookback,
        },
    )
    .with_next(id("returned"));
    let waiting = if local_error {
        waiting.with_on_error(id("fallback"))
    } else {
        waiting
    };
    let mut body = vec![waiting];
    if local_error || timeout_route {
        body.push(
            Node::new(
                id("fallback"),
                NodeKind::Timer {
                    delay: Duration::from_secs(1),
                },
            )
            .with_next(id("returned")),
        );
    }
    body.push(Node::new(
        id("returned"),
        NodeKind::Return {
            result: Expression::parse("item").expect("result"),
        },
    ));
    WorkflowDefinition::new_structured(
        Slug::parse("scoped-event").expect("name"),
        id("lines"),
        vec![
            Node::new(
                id("lines"),
                NodeKind::ForEach {
                    items: Expression::parse("input.lines").expect("items"),
                    body: id("body"),
                    result: Expression::parse("bodyResult").expect("result"),
                    max_items: 2,
                    max_concurrency: 2,
                },
            )
            .with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![BodyDefinition::new(id("body"), id("notified"), body).expect("closed body")],
        WorkflowLimits::default(),
    )
    .expect("definition")
}

fn state() -> RunState {
    let mut state = RunState::new(RunId::generate());
    state
        .apply(&RunEvent::RunStarted {
            entrypoint: id("lines"),
        })
        .expect("run");
    state
        .apply(&RunEvent::CollectionEntered {
            node: id("lines"),
            item_count: 2,
            max_items: 2,
            max_concurrency: 2,
            snapshot_digest: Digest::new([1; 32]),
        })
        .expect("scope");
    for index in 0..2 {
        state
            .apply(&RunEvent::CollectionIterationAdmitted {
                node: id("lines"),
                index,
                entry: wait(index),
                item_digest: Digest::new([2; 32]),
            })
            .expect("member");
    }
    state
}

fn event(event: RunEvent) -> AddressedRunEvent {
    AddressedRunEvent::in_path(path(0), event).expect("qualified step event")
}

#[test]
fn a_pending_scoped_event_wait_is_owned_by_its_dedicated_start_action() {
    let full = state();
    let def = definition();
    let bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    assert_eq!(
        plan_addressed(&def, &full, &path(0)).expect("pending wait"),
        AddressedPlan::Idle
    );
    assert_eq!(
        plan_path(&def, &bounded).expect("bounded pending wait"),
        AddressedPlan::Idle
    );
}

fn started() -> AddressedRunEvent {
    event(RunEvent::WaitStarted {
        node: id("notified"),
        correlation_key: CorrelationKey::parse("payment-1").expect("key"),
        deadline: OffsetDateTime::UNIX_EPOCH,
    })
}

#[test]
fn recorded_event_satisfaction_preserves_the_sibling_and_routes_next() {
    let mut full = state();
    let def = definition();
    let sibling = full.instance(&wait(1)).expect("sibling").clone();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    for addressed in [
        started(),
        event(RunEvent::WaitSatisfied {
            node: id("notified"),
        }),
    ] {
        full.apply_addressed(&addressed)
            .expect("recorded full wait");
        bounded
            .apply_addressed(&addressed)
            .expect("recorded bounded wait");
        assert_eq!(
            bounded,
            PathExecutionState::from_run(&full, path(0)).expect("replay")
        );
        assert_eq!(full.instance(&wait(1)), Some(&sibling));
    }
    let step = full.instance(&wait(0)).expect("wait");
    assert_eq!(
        (step.status(), step.attempt(), step.generation()),
        (StepStatus::Succeeded, 0, 0)
    );
    assert!(step.waiting_on().is_none());
    let route = AddressedPlan::Emit(event(RunEvent::StepRouted {
        node: id("notified"),
        activated: vec![id("returned")],
    }));
    assert_eq!(
        plan_addressed(&def, &full, &path(0)).expect("normal next"),
        route
    );
    assert_eq!(
        plan_path(&def, &bounded).expect("bounded normal next"),
        route
    );
}

#[test]
fn a_recorded_handled_timeout_routes_the_local_timeout_edge_in_both_models() {
    let mut full = state();
    let def = definition();
    full.apply_addressed(&started()).expect("wait start");
    full.apply_addressed(&event(RunEvent::WaitTimedOut {
        node: id("notified"),
        handled: true,
    }))
    .expect("handled timeout");
    let bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    assert!(full.instance(&wait(0)).expect("wait").timed_out());
    let route = AddressedPlan::Emit(event(RunEvent::StepRouted {
        node: id("notified"),
        activated: vec![id("fallback")],
    }));
    assert_eq!(
        plan_addressed(&def, &full, &path(0)).expect("timeout edge"),
        route
    );
    assert_eq!(
        plan_path(&def, &bounded).expect("bounded timeout edge"),
        route
    );
}

#[test]
fn timeout_or_event_records_one_outcome_and_an_unhandled_timeout_keeps_its_typed_failure() {
    for timeout_first in [false, true] {
        let mut full = state();
        full.apply_addressed(&started()).expect("start");
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        let timeout = event(RunEvent::WaitTimedOut {
            node: id("notified"),
            handled: false,
        });
        let received = event(RunEvent::WaitSatisfied {
            node: id("notified"),
        });
        let (first, late) = if timeout_first {
            (&timeout, &received)
        } else {
            (&received, &timeout)
        };
        full.apply_addressed(first).expect("first outcome");
        bounded.apply_addressed(first).expect("bounded outcome");
        if timeout_first {
            let step = full.instance(&wait(0)).expect("failed wait");
            assert_eq!(step.status(), StepStatus::Failed);
            assert_eq!(step.failure(), Some(&FailureCode::wait_timed_out()));
        }
        let before = full.clone();
        let before_bounded = bounded.clone();
        assert!(full.apply_addressed(late).is_err());
        assert!(bounded.apply_addressed(late).is_err());
        assert_eq!(full, before);
        assert_eq!(bounded, before_bounded);
    }
}

#[test]
fn cancelling_an_event_wait_prevents_late_satisfaction_and_timeout() {
    let mut full = state();
    full.apply_addressed(&started()).expect("start");
    assert!(matches!(
        full.instance(&wait(0)).expect("event wait").waiting_on(),
        Some(WaitingOn::Event { .. })
    ));
    full.apply_addressed(&event(RunEvent::StepCancelled {
        node: id("notified"),
    }))
    .expect("cancel");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    for late in [
        RunEvent::WaitSatisfied {
            node: id("notified"),
        },
        RunEvent::WaitTimedOut {
            node: id("notified"),
            handled: true,
        },
    ] {
        let before = full.clone();
        let before_bounded = bounded.clone();
        let addressed = event(late);
        assert!(full.apply_addressed(&addressed).is_err());
        assert!(bounded.apply_addressed(&addressed).is_err());
        assert_eq!(full, before);
        assert_eq!(bounded, before_bounded);
    }
}

#[test]
fn an_accepted_event_may_record_completion_while_paused_but_routing_waits_for_resume() {
    let def = definition();
    let mut full = state();
    full.apply_addressed(&started()).expect("accepted wait");
    full.apply(&RunEvent::RunPaused).expect("pause");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("paused path");
    let received = event(RunEvent::WaitSatisfied {
        node: id("notified"),
    });
    full.apply_addressed(&received).expect("accepted event");
    bounded
        .apply_addressed(&received)
        .expect("bounded accepted event");
    assert_eq!(
        plan_addressed(&def, &full, &path(0)).expect("paused"),
        AddressedPlan::Idle
    );
    assert_eq!(
        plan_path(&def, &bounded).expect("bounded paused"),
        AddressedPlan::Idle
    );
    assert!(!full.instance(&wait(0)).expect("completed wait").routed());
    full.apply(&RunEvent::RunResumed).expect("resume");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("resumed path");
    let route = plan_addressed(&def, &full, &path(0)).expect("resumed route");
    assert_eq!(
        plan_path(&def, &bounded).expect("bounded resumed route"),
        route
    );
    let AddressedPlan::Emit(route) = route else {
        panic!("completed event must route after resume")
    };
    full.apply_addressed(&route).expect("route");
    bounded.apply_addressed(&route).expect("bounded route");
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("oracle")
    );
}

proptest::proptest! {
    #[test]
    fn every_recorded_outcome_preserves_qualified_identity_correlation_and_deadline(
        index in 0_u32..2,
        offset in -1_000_000_i64..1_000_000,
        correlation in 0_u32..1_000_000,
        outcome in 0_u8..3,
    ) {
        let mut full=state();let sibling=full.instance(&wait(1-index)).expect("sibling").clone();
        let mut bounded=PathExecutionState::from_run(&full,path(index)).expect("path");
        let deadline=OffsetDateTime::UNIX_EPOCH+time::Duration::seconds(offset);
        let key=CorrelationKey::parse(&format!("payment-{correlation}")).expect("key");
        let start=AddressedRunEvent::in_path(path(index),RunEvent::WaitStarted {node:id("notified"),correlation_key:key.clone(),deadline}).expect("address");
        full.apply_addressed(&start).expect("full start");bounded.apply_addressed(&start).expect("bounded start");
        proptest::prop_assert_eq!(full.instance(&wait(index)).expect("wait").waiting_on(),Some(&WaitingOn::Event {correlation_key:key,deadline}));
        let end=if outcome==0 {RunEvent::WaitSatisfied {node:id("notified")}}else{RunEvent::WaitTimedOut {node:id("notified"),handled:outcome==1}};
        let end=AddressedRunEvent::in_path(path(index),end).expect("address");
        full.apply_addressed(&end).expect("full outcome");bounded.apply_addressed(&end).expect("bounded outcome");
        proptest::prop_assert_eq!(&bounded,&PathExecutionState::from_run(&full,path(index)).expect("oracle"));
        proptest::prop_assert_eq!(full.instance(&wait(1-index)),Some(&sibling));
        let before=full.clone();let before_bounded=bounded.clone();
        proptest::prop_assert!(full.apply_addressed(&end).is_err());proptest::prop_assert!(bounded.apply_addressed(&end).is_err());
        proptest::prop_assert_eq!(full,before);proptest::prop_assert_eq!(bounded,before_bounded);
    }
}

#[test]
fn invalid_wait_source_states_and_duplicate_cancellation_are_inert_in_both_models() {
    let cases = vec![
        (vec![started()], vec![started()]),
        // Deliberately seed a different recorded wait kind; this is a reducer type guard,
        // not permission to publish this mismatched definition/lifecycle combination.
        (
            vec![event(RunEvent::TimerStarted {
                node: id("notified"),
                due_at: OffsetDateTime::UNIX_EPOCH,
            })],
            vec![
                event(RunEvent::WaitSatisfied {
                    node: id("notified"),
                }),
                event(RunEvent::WaitTimedOut {
                    node: id("notified"),
                    handled: true,
                }),
                event(RunEvent::WaitTimedOut {
                    node: id("notified"),
                    handled: false,
                }),
            ],
        ),
        (
            vec![
                started(),
                event(RunEvent::StepCancelled {
                    node: id("notified"),
                }),
            ],
            vec![
                started(),
                event(RunEvent::WaitSatisfied {
                    node: id("notified"),
                }),
                event(RunEvent::WaitTimedOut {
                    node: id("notified"),
                    handled: true,
                }),
                event(RunEvent::StepCancelled {
                    node: id("notified"),
                }),
            ],
        ),
        (
            vec![],
            vec![
                event(RunEvent::WaitSatisfied {
                    node: id("notified"),
                }),
                event(RunEvent::WaitTimedOut {
                    node: id("notified"),
                    handled: false,
                }),
            ],
        ),
    ];
    for (prefix, refused) in cases {
        let mut full = state();
        for recorded in prefix {
            full.apply_addressed(&recorded)
                .expect("negative source fixture");
        }
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
        for recorded in refused {
            let before = full.clone();
            let before_bounded = bounded.clone();
            assert!(
                full.apply_addressed(&recorded).is_err(),
                "full rejects {recorded:?}"
            );
            assert!(
                bounded.apply_addressed(&recorded).is_err(),
                "bounded rejects {recorded:?}"
            );
            assert_eq!(full, before);
            assert_eq!(bounded, before_bounded);
        }
    }
}

#[test]
fn explicit_clock_start_records_only_its_member_key_and_deadline_in_both_models() {
    let def = definition();
    let mut full = state();
    let sibling = full.instance(&wait(1)).expect("sibling").clone();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let now = OffsetDateTime::UNIX_EPOCH;
    let key = CorrelationKey::parse("order-explicit-clock").expect("key");
    let progress = start_event_wait_instance(&def, &mut full, &wait(0), Ok(key.clone()), now)
        .expect("explicit full start");
    let path_progress =
        start_event_wait_path_instance(&def, &mut bounded, &wait(0), Ok(key.clone()), now)
            .expect("explicit bounded start");
    assert_eq!(progress, path_progress);
    assert_eq!(progress.events.len(), 1);
    assert_eq!(
        full.instance(&wait(0)).expect("wait").waiting_on(),
        Some(&WaitingOn::Event {
            correlation_key: key,
            deadline: now + time::Duration::seconds(60)
        })
    );
    assert_eq!(full.instance(&wait(1)), Some(&sibling));
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("oracle")
    );
}

#[test]
fn correlation_failure_records_one_local_expression_failure_without_starting_a_wait() {
    let def = definition();
    let mut full = state();
    let sibling = full.instance(&wait(1)).expect("sibling").clone();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let now = OffsetDateTime::UNIX_EPOCH;
    let progress = start_event_wait_instance(&def, &mut full, &wait(0), Err(()), now)
        .expect("record correlation failure");
    assert_eq!(
        progress,
        start_event_wait_path_instance(&def, &mut bounded, &wait(0), Err(()), now)
            .expect("bounded failure")
    );
    assert_eq!(
        progress.events,
        vec![
            event(RunEvent::StepFailed {
                node: id("notified"),
                code: FailureCode::expression_error(),
                fence: None,
            }),
            event(RunEvent::StepRouted {
                node: id("notified"),
                activated: vec![]
            }),
        ]
    );
    assert_eq!(
        full.instance(&wait(0)).expect("failed").failure(),
        Some(&FailureCode::expression_error())
    );
    assert_eq!(full.instance(&wait(0)).expect("failed").waiting_on(), None);
    assert_eq!(full.instance(&wait(1)), Some(&sibling));
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("oracle")
    );
}

#[test]
fn start_refuses_duplicate_paused_stopped_or_draining_sources_without_mutation() {
    for prefix in [
        vec![started()],
        vec![AddressedRunEvent::root(RunEvent::RunPaused)],
        vec![AddressedRunEvent::root(RunEvent::RunStopRequested {
            reason: sagat_domain::execution::StopReason::Cancelled,
            code: None,
        })],
        vec![AddressedRunEvent::root(RunEvent::CollectionDrainStarted {
            node: id("lines"),
            reason: sagat_domain::execution::CollectionDrainReason::Cancelled,
        })],
        vec![event(RunEvent::StepCancelled {
            node: id("notified"),
        })],
    ] {
        let def = definition();
        let mut full = state();
        for recorded in prefix {
            full.apply_addressed(&recorded).expect("guard fixture");
        }
        let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("guard path");
        let before = full.clone();
        let bounded_before = bounded.clone();
        let key = CorrelationKey::parse("replacement").expect("key");
        for answer in [Ok(key), Err(())] {
            assert!(
                start_event_wait_instance(
                    &def,
                    &mut full,
                    &wait(0),
                    answer.clone(),
                    OffsetDateTime::UNIX_EPOCH
                )
                .is_err()
            );
            assert!(
                start_event_wait_path_instance(
                    &def,
                    &mut bounded,
                    &wait(0),
                    answer,
                    OffsetDateTime::UNIX_EPOCH
                )
                .is_err()
            );
            assert_eq!(full, before);
            assert_eq!(bounded, bounded_before);
        }
    }
}

#[test]
fn overflowing_deadline_root_and_foreign_path_are_refused_before_recording() {
    let def = definition();
    let mut full = state();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let before = full.clone();
    let bounded_before = bounded.clone();
    let key = CorrelationKey::parse("key").expect("key");
    assert!(
        start_event_wait_instance(
            &def,
            &mut full,
            &wait(0),
            Ok(key.clone()),
            time::PrimitiveDateTime::MAX.assume_utc()
        )
        .is_err()
    );
    assert!(
        start_event_wait_path_instance(
            &def,
            &mut bounded,
            &wait(0),
            Ok(key.clone()),
            time::PrimitiveDateTime::MAX.assume_utc()
        )
        .is_err()
    );
    for candidate in [ExecutionInstanceId::root(id("notified")), wait(2)] {
        assert!(
            start_event_wait_instance(
                &def,
                &mut full,
                &candidate,
                Ok(key.clone()),
                OffsetDateTime::UNIX_EPOCH
            )
            .is_err()
        );
        assert!(
            start_event_wait_path_instance(
                &def,
                &mut bounded,
                &candidate,
                Ok(key.clone()),
                OffsetDateTime::UNIX_EPOCH
            )
            .is_err()
        );
    }
    assert_eq!(full, before);
    assert_eq!(bounded, bounded_before);
}

// Restores this fixture's open two-member collection while changing exactly one local row.
fn restored_member_projection(
    source: &RunState,
    damaged: sagat_domain::execution::StepState,
) -> RunState {
    restored_local_projection(source, wait(0), damaged)
}

fn restored_local_projection(
    source: &RunState,
    instance: ExecutionInstanceId,
    damaged: sagat_domain::execution::StepState,
) -> RunState {
    use sagat_domain::execution::{CollectionIterationRestore, CollectionPhase};
    let mut steps = source
        .instances()
        .map(|(id, step)| (id.clone(), step.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    steps.insert(instance, damaged);
    let mut full = RunState::restore_instances(
        source.run_id(),
        source.status(),
        None,
        false,
        steps,
        std::collections::BTreeMap::new(),
        source.version(),
    );
    full.restore_collection(
        ExecutionInstanceId::root(id("lines")),
        CollectionPhase::Open,
        2,
        2,
        2,
        Digest::new([1; 32]),
        None,
        (0..2)
            .map(|index| {
                CollectionIterationRestore::admitted(index, wait(index), Digest::new([2; 32]))
            })
            .collect(),
    )
    .expect("preserved genuine collection membership");
    full
}

#[test]
fn malformed_pending_event_projection_cannot_authorize_success_or_failure_start() {
    use sagat_domain::execution::{ExecutionError, StepState, SwitchSelection};
    for field in 0..10 {
        let source = state();
        let now = OffsetDateTime::UNIX_EPOCH;
        let damaged = StepState::restore(
            StepStatus::Pending,
            u32::from(field == 0),
            u64::from(field == 1),
            field == 2,
            field == 3,
            (field == 4).then_some(now),
            (field == 5).then_some(WaitingOn::Timer { due_at: now }),
            (field == 6).then(FailureCode::expression_error),
            (field == 7).then_some(false),
            field == 8,
        )
        .with_restored_switch_selection((field == 9).then_some(SwitchSelection::Default));
        let mut full = restored_member_projection(&source, damaged);
        let mut bounded =
            PathExecutionState::from_run(&full, path(0)).expect("malformed local projection");
        let before = full.clone();
        let bounded_before = bounded.clone();
        let def = definition();
        for answer in [Ok(CorrelationKey::parse("key").expect("key")), Err(())] {
            assert!(
                matches!(
                    start_event_wait_instance(&def, &mut full, &wait(0), answer.clone(), now),
                    Err(ExecutionError::Invariant(
                        "invalid pending scoped event state"
                    ))
                ),
                "field {field}"
            );
            assert!(
                matches!(
                    start_event_wait_path_instance(&def, &mut bounded, &wait(0), answer, now),
                    Err(ExecutionError::Invariant(
                        "invalid pending scoped event state"
                    ))
                ),
                "bounded field {field}"
            );
            assert_eq!(full, before);
            assert_eq!(bounded, bounded_before);
        }
    }
}

#[test]
fn correlation_failure_takes_only_the_local_error_edge_and_cannot_start_a_timer_as_a_wait() {
    let def = definition_with_error_route(true);
    let mut full = state();
    let sibling = full.instance(&wait(1)).expect("sibling").clone();
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("path");
    let now = OffsetDateTime::UNIX_EPOCH;
    let progress = start_event_wait_instance(&def, &mut full, &wait(0), Err(()), now)
        .expect("handled failure");
    assert_eq!(
        progress,
        start_event_wait_path_instance(&def, &mut bounded, &wait(0), Err(()), now)
            .expect("bounded handled failure")
    );
    assert_eq!(
        progress.events,
        vec![
            event(RunEvent::StepFailed {
                node: id("notified"),
                code: FailureCode::expression_error(),
                fence: None
            }),
            event(RunEvent::StepRouted {
                node: id("notified"),
                activated: vec![id("fallback")]
            }),
        ]
    );
    let timer = ExecutionInstanceId::in_path(path(0), id("fallback"));
    assert_eq!(
        full.instance(&timer).expect("local error target").status(),
        StepStatus::Pending
    );
    assert_eq!(full.instance(&wait(1)), Some(&sibling));
    assert_eq!(
        bounded,
        PathExecutionState::from_run(&full, path(0)).expect("oracle")
    );
    let before = full.clone();
    let bounded_before = bounded.clone();
    assert!(matches!(
        start_event_wait_instance(
            &def,
            &mut full,
            &timer,
            Ok(CorrelationKey::parse("key").expect("key")),
            now
        ),
        Err(sagat_domain::execution::ExecutionError::Invariant(
            "event start targeted another kind"
        ))
    ));
    assert!(matches!(
        start_event_wait_path_instance(&def, &mut bounded, &timer, Err(()), now),
        Err(sagat_domain::execution::ExecutionError::Invariant(
            "event start targeted another kind"
        ))
    ));
    assert_eq!(full, before);
    assert_eq!(bounded, bounded_before);
}

proptest::proptest! {
    #[test]
    fn explicit_start_matches_journal_and_bounded_replay_for_varied_keys_and_clocks(
        index in 0_u32..2,
        offset in -1_000_000_i64..1_000_000,
        key in 0_u32..1_000_000,
    ) {
        let def = definition(); let mut full = state();
        let sibling = full.instance(&wait(1-index)).expect("sibling").clone();
        let mut bounded = PathExecutionState::from_run(&full,path(index)).expect("path");
        let now = OffsetDateTime::UNIX_EPOCH + time::Duration::microseconds(offset);
        let key = CorrelationKey::parse(&format!("order-{key}")).expect("key");
        let progress = start_event_wait_instance(&def,&mut full,&wait(index),Ok(key.clone()),now).expect("start");
        proptest::prop_assert_eq!(&progress,&start_event_wait_path_instance(&def,&mut bounded,&wait(index),Ok(key.clone()),now).expect("bounded start"));
        proptest::prop_assert_eq!(progress.events,vec![AddressedRunEvent::in_path(path(index),RunEvent::WaitStarted {node:id("notified"),correlation_key:key,deadline:now+time::Duration::seconds(60)}).expect("event")]);
        proptest::prop_assert_eq!(full.instance(&wait(1-index)),Some(&sibling));
        proptest::prop_assert_eq!(bounded,PathExecutionState::from_run(&full,path(index)).expect("oracle"));
    }
}

#[path = "scoped_event_wait_lifecycle/cancellation.rs"]
mod cancellation;
#[path = "scoped_event_wait_lifecycle/satisfaction.rs"]
mod satisfaction;
#[path = "scoped_event_wait_lifecycle/timeouts.rs"]
mod timeouts;

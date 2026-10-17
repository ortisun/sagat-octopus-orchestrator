//! Bounded path planning is equivalent to the full run oracle without retaining unrelated paths.
#![allow(clippy::expect_used)]

use sagat_domain::execution::{
    AddressedPlan, AddressedRunEvent, CollectionPhase, ExecutionInstanceId, Fence, InstancePath,
    PathExecutionState, RunEvent, RunId, RunState, StepStatus, complete_instance,
    complete_path_instance, heartbeat_instance, heartbeat_path_instance, lease_instance,
    lease_path_instance, plan_addressed, plan_path,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    ActivityName, BodyDefinition, Expression, Node, NodeId, NodeKind, RetryPolicy,
    WorkflowDefinition,
};

fn id(value: &str) -> NodeId {
    NodeId::parse(value).expect("node")
}

fn definition() -> WorkflowDefinition {
    definition_with_policy(RetryPolicy::no_retry(), false, false)
}

fn definition_with_policy(retry: RetryPolicy, high_risk: bool, lookup: bool) -> WorkflowDefinition {
    let body = BodyDefinition::new(
        id("body"),
        id("work"),
        vec![
            Node::new(
                id("work"),
                NodeKind::Activity {
                    activity: ActivityName::parse("act.work").expect("activity"),
                    retry,
                    compensate_with: None,
                    reconcile_with: lookup
                        .then(|| ActivityName::parse("lookup.work").expect("lookup")),
                    idempotency_template: None,
                    secrets: Vec::new(),
                },
            )
            .with_effect(sagat_domain::saga::EffectPolicy {
                high_risk,
                ..sagat_domain::saga::EffectPolicy::NONE
            })
            .with_next(id("return")),
            Node::new(
                id("return"),
                NodeKind::Return {
                    result: Expression::parse("item").expect("result"),
                },
            ),
        ],
    )
    .expect("body");
    WorkflowDefinition::new_structured(
        Slug::parse("bounded-path").expect("workflow"),
        id("items"),
        vec![
            Node::new(
                id("items"),
                NodeKind::ForEach {
                    items: Expression::parse("input.items").expect("items"),
                    body: id("body"),
                    result: Expression::parse("bodyResult").expect("aggregate"),
                    max_items: 10_000,
                    max_concurrency: 256,
                },
            )
            .with_next(id("done")),
            Node::new(id("done"), NodeKind::End { output: None }),
        ],
        vec![body],
        sagat_domain::workflow::WorkflowLimits::default(),
    )
    .expect("definition")
}

fn path(index: u32) -> InstancePath {
    InstancePath::root()
        .child(id("items"), index)
        .expect("path")
}

fn member(index: u32, node: &str) -> ExecutionInstanceId {
    ExecutionInstanceId::in_path(path(index), id(node))
}

fn state(iterations: u32) -> RunState {
    let mut state = RunState::new(RunId::generate());
    for event in [
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: id("items"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: id("items"),
            item_count: iterations,
            max_items: 10_000,
            max_concurrency: 256,
            snapshot_digest: sagat_domain::workflow::Digest::new([1; 32]),
        }),
    ] {
        state.apply_addressed(&event).expect("scope event");
    }
    for index in 0..iterations {
        state
            .apply_addressed(&AddressedRunEvent::root(
                RunEvent::CollectionIterationAdmitted {
                    node: id("items"),
                    index,
                    entry: member(index, "work"),
                    item_digest: sagat_domain::workflow::Digest::new([2; 32]),
                },
            ))
            .expect("iteration admitted");
    }
    state
}

fn state_with_local_collection(iterations: u32) -> RunState {
    let mut state = RunState::new(RunId::generate());
    let owner_path = path(0);
    for event in [
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: id("items"),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: id("items"),
            item_count: 1,
            max_items: 10_000,
            max_concurrency: 256,
            snapshot_digest: sagat_domain::workflow::Digest::new([1; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationAdmitted {
            node: id("items"),
            index: 0,
            entry: ExecutionInstanceId::in_path(owner_path.clone(), id("inner")),
            item_digest: sagat_domain::workflow::Digest::new([2; 32]),
        }),
    ] {
        state.apply_addressed(&event).expect("outer scope event");
    }
    state
        .apply_addressed(
            &AddressedRunEvent::in_path(
                owner_path.clone(),
                RunEvent::CollectionEntered {
                    node: id("inner"),
                    item_count: iterations,
                    max_items: 10_000,
                    max_concurrency: 256,
                    snapshot_digest: sagat_domain::workflow::Digest::new([3; 32]),
                },
            )
            .expect("nested collection address"),
        )
        .expect("nested collection entered");
    for index in 0..iterations {
        let nested_path = owner_path
            .child(id("inner"), index)
            .expect("nested member path");
        state
            .apply_addressed(
                &AddressedRunEvent::in_path(
                    owner_path.clone(),
                    RunEvent::CollectionIterationAdmitted {
                        node: id("inner"),
                        index,
                        entry: ExecutionInstanceId::in_path(nested_path, id("work")),
                        item_digest: sagat_domain::workflow::Digest::new([4; 32]),
                    },
                )
                .expect("nested iteration address"),
            )
            .expect("nested iteration admitted");
    }
    state
}

#[test]
fn one_path_snapshot_excludes_every_unrelated_iteration() {
    let full = state(256);
    let local = PathExecutionState::from_run(&full, path(255)).expect("complete local snapshot");

    assert_eq!(local.local_instances().count(), 1);
    assert_eq!(
        local.local_instance(&id("work")),
        full.instance(&member(255, "work"))
    );
    assert_eq!(local.ancestors().len(), 1);
    assert_eq!(
        local.ancestors()[0].collection_phase(),
        Some(CollectionPhase::Open)
    );
    assert_eq!(local.ancestors()[0].iteration_index(), 255);
    assert_eq!(local.global().version(), full.version());
}

#[test]
fn local_collection_is_a_scalar_projection_without_member_history() {
    let full = state_with_local_collection(256);
    let local = PathExecutionState::from_run(&full, path(0)).expect("complete local snapshot");
    let collection = local.local_collection(&id("inner")).expect("local scope");

    assert_eq!(collection.admitted_count(), 256);
    assert_eq!(collection.completed_count(), 0);
    assert_eq!(collection.active_count(), 256);
    assert_eq!(local.local_instances().count(), 1);
}

#[test]
fn bounded_planner_matches_the_full_run_oracle() {
    let definition = definition();
    let full = state(3);
    let chosen = path(1);
    let local = PathExecutionState::from_run(&full, chosen.clone()).expect("local state");

    assert_eq!(
        plan_path(&definition, &local).expect("bounded plan"),
        plan_addressed(&definition, &full, &chosen).expect("oracle plan")
    );
    assert!(matches!(
        plan_path(&definition, &local),
        Ok(AddressedPlan::Emit(_))
    ));
}

#[test]
fn global_pause_fences_both_bounded_and_oracle_planners() {
    let definition = definition();
    let mut full = state(1);
    full.apply(&RunEvent::RunPaused).expect("pause");
    let chosen = path(0);
    let local = PathExecutionState::from_run(&full, chosen.clone()).expect("local state");

    assert_eq!(plan_path(&definition, &local), Ok(AddressedPlan::Idle));
    assert_eq!(
        plan_addressed(&definition, &full, &chosen),
        Ok(AddressedPlan::Idle)
    );
}

#[test]
fn bounded_worker_lifecycle_matches_the_full_run_oracle() {
    let definition = definition();
    let mut full = state(3);
    let chosen = path(1);
    let work = member(1, "work");
    let mut local =
        PathExecutionState::from_run(&full, chosen.clone()).expect("complete local snapshot");

    let release = match plan_path(&definition, &local).expect("bounded release") {
        AddressedPlan::Emit(event) => event,
        AddressedPlan::Idle => panic!("pending activity must be released"),
    };
    local
        .apply_addressed(&release)
        .expect("bounded release applies");
    full.apply_addressed(&release)
        .expect("oracle release applies");

    let (full_fence, full_lease) = lease_instance(&mut full, &work).expect("oracle lease");
    let (local_fence, local_lease) = lease_path_instance(&mut local, &work).expect("bounded lease");
    assert_eq!((local_fence, &local_lease), (full_fence, &full_lease));

    let before_stale = local.clone();
    assert!(matches!(
        heartbeat_path_instance(
            &mut local,
            &work,
            Fence {
                attempt: local_fence.attempt,
                generation: local_fence.generation + 1,
            },
        ),
        Err(sagat_domain::execution::ExecutionError::StaleInstanceAttempt { .. })
    ));
    assert_eq!(local, before_stale);

    let full_heartbeat =
        heartbeat_instance(&mut full, &work, full_fence).expect("oracle heartbeat");
    let local_heartbeat =
        heartbeat_path_instance(&mut local, &work, local_fence).expect("bounded heartbeat");
    assert_eq!(local_heartbeat, full_heartbeat);

    let full_progress =
        complete_instance(&definition, &mut full, &work, full_fence).expect("oracle completion");
    let local_progress = complete_path_instance(&definition, &mut local, &work, local_fence)
        .expect("bounded completion");
    assert_eq!(local_progress, full_progress);
    assert_eq!(
        local.local_instance(&id("work")).map(|step| step.status()),
        Some(StepStatus::Succeeded)
    );
    assert_eq!(
        local
            .local_instance(&id("return"))
            .map(|step| step.status()),
        Some(StepStatus::Pending)
    );
    assert_eq!(local.global().version(), full.version());
}

#[test]
fn bounded_reconciliation_and_review_reduce_like_the_full_run() {
    use sagat_domain::execution::{FailureCode, ReconcileResult, ReviewOutcome};
    use sagat_domain::saga::ReconcileVerdict;
    use time::OffsetDateTime;

    for decision in 0..6 {
        let mut full = state(3);
        let work = member(1, "work");
        let lookup = work.node().reconciliation();
        let release = AddressedRunEvent::in_path(
            path(1),
            RunEvent::StepReleased {
                node: work.node().clone(),
            },
        )
        .expect("release address");
        full.apply_addressed(&release).expect("release");
        let (fence, _) = lease_instance(&mut full, &work).expect("lease forward");
        let mut bounded = PathExecutionState::from_run(&full, path(1)).expect("bounded state");
        let (verdict, result) = match decision {
            0 => (ReconcileVerdict::Applied, ReconcileResult::Succeeded),
            1 => (
                ReconcileVerdict::NotApplied,
                ReconcileResult::RetryAt(OffsetDateTime::UNIX_EPOCH),
            ),
            2 => (
                ReconcileVerdict::NotApplied,
                ReconcileResult::Failed(FailureCode::reconciled_not_applied()),
            ),
            _ => (
                ReconcileVerdict::Unknown,
                ReconcileResult::Review(FailureCode::reconciliation_unknown()),
            ),
        };
        let mut events = vec![
            RunEvent::ReconciliationRequested {
                node: work.node().clone(),
                fence,
                cause: FailureCode::outcome_unknown(),
            },
            RunEvent::StepReleased {
                node: lookup.clone(),
            },
            RunEvent::StepLeased {
                node: lookup.clone(),
                attempt: 1,
                generation: 1,
            },
            RunEvent::StepCompleted {
                node: lookup,
                attempt: 1,
                generation: 1,
            },
            RunEvent::ReconciliationConcluded {
                node: work.node().clone(),
                verdict,
                result,
            },
        ];
        let outcome = match decision {
            3 => Some(ReviewOutcome::Succeeded),
            4 => Some(ReviewOutcome::Failed(
                FailureCode::parse("DECLINED").expect("code"),
            )),
            _ => None,
        };
        if let Some(outcome) = outcome {
            events.push(RunEvent::StepResolved {
                node: work.node().clone(),
                outcome,
            });
        }
        if decision == 1 || decision == 5 {
            events.push(RunEvent::StepRetryRequested {
                node: work.node().clone(),
            });
        }
        for event in events {
            let addressed = AddressedRunEvent::in_path(path(1), event).expect("effect address");
            full.apply_addressed(&addressed).expect("full reducer");
            bounded
                .apply_addressed(&addressed)
                .expect("bounded effect reducer");
            assert_eq!(
                bounded,
                PathExecutionState::from_run(&full, path(1)).expect("oracle"),
                "decision {decision}"
            );
        }
    }
}

#[test]
fn bounded_effect_events_preserve_stale_fences_and_rejected_state() {
    use sagat_domain::execution::{ExecutionError, FailureCode};
    let mut full = state(2);
    let work = member(0, "work");
    full.apply_addressed(
        &AddressedRunEvent::in_path(
            path(0),
            RunEvent::StepReleased {
                node: work.node().clone(),
            },
        )
        .expect("address"),
    )
    .expect("release");
    let (fence, _) = lease_instance(&mut full, &work).expect("lease");
    let mut bounded = PathExecutionState::from_run(&full, path(0)).expect("bounded");
    for review in [false, true] {
        let stale = Fence {
            generation: fence.generation + 1,
            ..fence
        };
        let event = if review {
            RunEvent::StepNeedsReview {
                node: work.node().clone(),
                code: FailureCode::outcome_unknown(),
                fence: Some(stale),
            }
        } else {
            RunEvent::ReconciliationRequested {
                node: work.node().clone(),
                fence: stale,
                cause: FailureCode::outcome_unknown(),
            }
        };
        let addressed = AddressedRunEvent::in_path(path(0), event).expect("address");
        let before = bounded.clone();
        assert_eq!(
            bounded.apply_addressed(&addressed),
            Err(ExecutionError::StaleInstanceAttempt {
                instance: work.clone()
            })
        );
        assert_eq!(bounded, before);
        assert!(
            full.instance(&ExecutionInstanceId::in_path(
                path(0),
                work.node().reconciliation()
            ))
            .is_none()
        );
    }
}

#[test]
fn bounded_status_keeps_reviews_and_live_work_in_other_paths() {
    use sagat_domain::execution::{FailureCode, RunStatus};
    for sibling_live in [false, true] {
        let mut full = state(3);
        for index in 0..2 {
            let work = member(index, "work");
            full.apply_addressed(
                &AddressedRunEvent::in_path(
                    path(index),
                    RunEvent::StepReleased {
                        node: work.node().clone(),
                    },
                )
                .expect("release address"),
            )
            .expect("release");
            lease_instance(&mut full, &work).expect("lease");
        }
        if sibling_live {
            full.apply_addressed(
                &AddressedRunEvent::in_path(path(2), RunEvent::StepReleased { node: id("work") })
                    .expect("address"),
            )
            .expect("live sibling");
        }
        full.apply_addressed(
            &AddressedRunEvent::in_path(
                path(0),
                RunEvent::StepNeedsReview {
                    node: id("work"),
                    code: FailureCode::outcome_unknown(),
                    fence: Some(Fence {
                        attempt: 1,
                        generation: 1,
                    }),
                },
            )
            .expect("review address"),
        )
        .expect("other path review");
        let mut bounded = PathExecutionState::from_run(&full, path(1)).expect("bounded snapshot");
        let completion = AddressedRunEvent::in_path(
            path(1),
            RunEvent::StepCompleted {
                node: id("work"),
                attempt: 1,
                generation: 1,
            },
        )
        .expect("completion address");
        full.apply_addressed(&completion).expect("full completion");
        bounded
            .apply_addressed(&completion)
            .expect("bounded completion");
        assert_eq!(
            bounded,
            PathExecutionState::from_run(&full, path(1)).expect("full oracle")
        );
        assert_eq!(
            bounded.global().status(),
            if sibling_live {
                RunStatus::Running
            } else {
                RunStatus::ManualReview
            }
        );
    }
}

#[test]
fn global_work_summary_rejects_contradictory_facts() {
    use sagat_domain::execution::RunWorkSummary;
    assert!(RunWorkSummary::restore(false, true, false, false).is_err());
    assert!(RunWorkSummary::restore(false, false, false, true).is_err());
    assert!(RunWorkSummary::restore(true, true, true, true).is_ok());
}

#[test]
fn bounded_failed_attempt_commands_match_full_replay_for_retry_cancel_and_unknown_effects() {
    use sagat_domain::execution::{Failure, FailureCode, fail_instance, fail_path_instance};
    use time::OffsetDateTime;
    for case in 0..6 {
        let def = definition_with_policy(RetryPolicy::DEFAULT, case == 4, case == 3);
        let mut full = state(3);
        let work = member(1, "work");
        full.apply_addressed(
            &AddressedRunEvent::in_path(path(1), RunEvent::StepReleased { node: id("work") })
                .expect("address"),
        )
        .expect("release");
        let (fence, _) = lease_instance(&mut full, &work).expect("lease");
        if case == 2 {
            full.apply_addressed(
                &AddressedRunEvent::in_path(
                    path(1),
                    RunEvent::StepCancelRequested { node: id("work") },
                )
                .expect("address"),
            )
            .expect("request cancellation");
        }
        let mut bounded = PathExecutionState::from_run(&full, path(1)).expect("bounded");
        let failure = Failure {
            fence,
            code: if case >= 3 {
                FailureCode::outcome_unknown()
            } else {
                FailureCode::parse("DECLINED").expect("code")
            },
            retryable: case != 0,
            jitter_sample: 0.0,
        };
        let expected = fail_instance(
            &def,
            &mut full,
            &work,
            failure.clone(),
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("full failed attempt");
        let actual = fail_path_instance(
            &def,
            &mut bounded,
            &work,
            failure,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("bounded failed attempt");
        assert_eq!(actual, expected, "case {case}");
        assert_eq!(
            bounded,
            PathExecutionState::from_run(&full, path(1)).expect("oracle"),
            "case {case}"
        );
    }
}

#[test]
fn bounded_lookup_verdict_and_operator_commands_match_full_replay() {
    use sagat_domain::execution::{
        Failure, FailureCode, ReviewOutcome, complete_reconciliation_instance,
        complete_reconciliation_path_instance, fail_instance, fail_path_instance,
        resolve_review_instance, resolve_review_path_instance, retry_review_instance,
        retry_review_path_instance,
    };
    use sagat_domain::saga::ReconcileVerdict;
    use time::OffsetDateTime;
    let now = OffsetDateTime::UNIX_EPOCH;
    for case in 0..5 {
        let def = definition_with_policy(RetryPolicy::DEFAULT, true, true);
        let mut full = state(3);
        let work = member(1, "work");
        full.apply_addressed(
            &AddressedRunEvent::in_path(path(1), RunEvent::StepReleased { node: id("work") })
                .expect("address"),
        )
        .expect("release");
        let (fence, _) = lease_instance(&mut full, &work).expect("lease");
        let mut bounded = PathExecutionState::from_run(&full, path(1)).expect("bounded");
        let failure = Failure {
            fence,
            code: FailureCode::outcome_unknown(),
            retryable: true,
            jitter_sample: 0.0,
        };
        let expected = fail_instance(&def, &mut full, &work, failure.clone(), now)
            .expect("full unknown effect");
        let actual = fail_path_instance(&def, &mut bounded, &work, failure, now)
            .expect("bounded unknown effect");
        assert_eq!(actual, expected);
        let lookup = ExecutionInstanceId::in_path(path(1), work.node().reconciliation());
        let (expected_fence, expected_event) =
            lease_instance(&mut full, &lookup).expect("full lookup lease");
        let (actual_fence, actual_event) =
            lease_path_instance(&mut bounded, &lookup).expect("bounded authorized lookup lease");
        assert_eq!(
            (actual_fence, actual_event),
            (expected_fence, expected_event)
        );
        let verdict = match case {
            0 => ReconcileVerdict::Applied,
            1 => ReconcileVerdict::NotApplied,
            _ => ReconcileVerdict::Unknown,
        };
        let expected = complete_reconciliation_instance(
            &def,
            &mut full,
            &lookup,
            expected_fence,
            verdict,
            0.0,
            now,
        )
        .expect("full lookup verdict");
        let actual = complete_reconciliation_path_instance(
            &def,
            &mut bounded,
            &lookup,
            actual_fence,
            verdict,
            0.0,
            now,
        )
        .expect("bounded lookup verdict");
        assert_eq!(actual, expected, "verdict case {case}");
        assert_eq!(
            bounded,
            PathExecutionState::from_run(&full, path(1)).expect("oracle")
        );
        if case >= 2 {
            let (expected, actual) = if case == 2 {
                (
                    retry_review_instance(&def, &mut full, &work, now),
                    retry_review_path_instance(&def, &mut bounded, &work, now),
                )
            } else {
                let outcome = if case == 3 {
                    ReviewOutcome::Succeeded
                } else {
                    ReviewOutcome::Failed(FailureCode::parse("DECLINED").expect("code"))
                };
                (
                    resolve_review_instance(&def, &mut full, &work, outcome.clone(), now),
                    resolve_review_path_instance(&def, &mut bounded, &work, outcome, now),
                )
            };
            assert_eq!(actual, expected, "decision case {case}");
            assert_eq!(
                bounded,
                PathExecutionState::from_run(&full, path(1)).expect("decision oracle")
            );
        }
    }
}

#[test]
fn bounded_expiry_matches_full_policy_for_ambiguity_cancellation_and_stale_fences() {
    use sagat_domain::execution::{Expiry, expire_instance, expire_path_instance};
    use time::OffsetDateTime;
    for cause in [Expiry::LeaseExpired, Expiry::AttemptTimeout] {
        for policy in 0..4 {
            for cancelled in [false, true] {
                for stale in [false, true] {
                    let def = definition_with_policy(
                        RetryPolicy::DEFAULT,
                        policy & 1 != 0,
                        policy & 2 != 0,
                    );
                    let mut full = state(3);
                    let work = member(1, "work");
                    full.apply_addressed(
                        &AddressedRunEvent::in_path(
                            path(1),
                            RunEvent::StepReleased { node: id("work") },
                        )
                        .expect("address"),
                    )
                    .expect("release");
                    let (mut fence, _) = lease_instance(&mut full, &work).expect("lease");
                    if cancelled {
                        full.apply_addressed(
                            &AddressedRunEvent::in_path(
                                path(1),
                                RunEvent::StepCancelRequested { node: id("work") },
                            )
                            .expect("cancel address"),
                        )
                        .expect("cancel request");
                    }
                    let mut bounded =
                        PathExecutionState::from_run(&full, path(1)).expect("bounded snapshot");
                    let before_full = full.clone();
                    let before_bounded = bounded.clone();
                    if stale {
                        fence.generation += 1;
                    }
                    let expected = expire_instance(
                        &def,
                        &mut full,
                        &work,
                        fence,
                        cause,
                        0.0,
                        OffsetDateTime::UNIX_EPOCH,
                    );
                    let actual = expire_path_instance(
                        &def,
                        &mut bounded,
                        &work,
                        fence,
                        cause,
                        0.0,
                        OffsetDateTime::UNIX_EPOCH,
                    );
                    assert_eq!(
                        actual, expected,
                        "{cause:?}, policy {policy}, cancelled {cancelled}, stale {stale}"
                    );
                    assert_eq!(
                        bounded,
                        PathExecutionState::from_run(&full, path(1)).expect("oracle")
                    );
                    if stale {
                        assert!(actual.is_err());
                        assert_eq!(full, before_full);
                        assert_eq!(bounded, before_bounded);
                    } else {
                        assert!(actual.is_ok());
                        let step = bounded.local_instance(work.node()).expect("expired step");
                        let expected_status = if policy & 2 != 0 || policy & 1 != 0 {
                            StepStatus::Waiting
                        } else if cancelled {
                            StepStatus::Cancelled
                        } else {
                            StepStatus::RetryWait
                        };
                        assert_eq!(step.status(), expected_status);
                        if policy & 2 != 0 {
                            assert_eq!(
                                step.waiting_on(),
                                Some(&sagat_domain::execution::WaitingOn::Reconciliation)
                            );
                        } else if policy & 1 != 0 {
                            assert!(matches!(
                                step.waiting_on(),
                                Some(sagat_domain::execution::WaitingOn::Operator { .. })
                            ));
                        }
                    }
                }
            }
        }
    }
}

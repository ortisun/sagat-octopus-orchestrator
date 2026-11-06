//! Reset to a point (`T018-052`, `FR-018-035`, `FR-018-036`, D1022, D1605): the registry has a rule for every
//! journal event type, and the eligible points and the plan of a seeded corpus are what the registry and the
//! replayed state say.
#![allow(clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;

use common::{
    Harness, activity, build, child, effect, id, parallel, reconciled, saga, timer, wait,
    with_effect,
};
use sagat_domain::execution::{
    AddressedRunEvent, ChildOutcome, CollectionDrainReason, CollectionOutcome, EffectMarkers,
    ExecutionInstanceId, Expiry, InstancePath, RESET_RULES, ResetBlock, ResetPoints, ResetReapply,
    ResetRefusal, ResetRule, RunEvent, RunId, RunState, RunStatus, StopReason, reset_plan,
    reset_points, rule_by_name, rule_of,
};
use sagat_domain::saga::{EffectPolicy, ReconcileVerdict};
use sagat_domain::workflow::{CancelPolicy, Digest, JoinPolicy, NodeId};

fn addressed(harness: &Harness) -> Vec<AddressedRunEvent> {
    harness
        .journal
        .iter()
        .cloned()
        .map(AddressedRunEvent::root)
        .collect()
}

fn points(harness: &Harness) -> ResetPoints {
    reset_points(harness.state.run_id(), &addressed(harness)).expect("a journal that replays")
}

/// The sequence of every event of a type, 1-based.
fn sequences(harness: &Harness, name: &str) -> Vec<u64> {
    harness
        .journal
        .iter()
        .enumerate()
        .filter(|(_, event)| event.name() == name)
        .map(|(index, _)| u64::try_from(index + 1).expect("small"))
        .collect()
}

fn eligible(points: &ResetPoints) -> Vec<u64> {
    points.eligible.iter().map(|point| point.sequence).collect()
}

fn blocked(points: &ResetPoints, sequence: u64) -> Option<&ResetBlock> {
    points
        .blocked
        .iter()
        .find(|(point, _)| point.sequence == sequence)
        .map(|(_, block)| block)
}

// ------------------------------------------------------------------------------------------ the registry

/// The event types the journal names, read from the source of `RunEvent::name()`: a type added there without
/// a row in the registry fails here (D1605).
fn names_of_the_journal() -> BTreeSet<String> {
    let source = include_str!("../src/execution/event.rs");
    let start = source
        .find("pub const fn name(&self) -> &'static str")
        .expect("the name function");
    let body = &source[start..start + source[start..].find("\n    }\n").expect("its end")];
    body.split("=> \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_event_type_of_the_journal_has_exactly_one_rule() {
    let named = names_of_the_journal();
    let ruled: Vec<&str> = RESET_RULES.iter().map(|(name, _)| *name).collect();
    let ruled_set: BTreeSet<String> = ruled.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(ruled.len(), ruled_set.len(), "a type has two rules");
    assert_eq!(
        named, ruled_set,
        "the journal names a type the registry has no rule for, or the registry names one the journal does not"
    );
    assert!(
        named.len() >= 66,
        "the scan read the table: {}",
        named.len()
    );
}

#[test]
fn the_rules_are_the_ones_of_the_contract() {
    // typed from `contracts/journal-and-events.md`, independent of the table under test
    let exclude = [
        "saga.aborting",
        "compensation.planned",
        "saga.compensated",
        "scope.compensation_planned",
        "scope.compensated",
        "run.stop_requested",
        "end.reached",
        "run.finished",
        "execution_chain.started",
        "execution_chain.successor_linked",
        "run.continued",
        "continuation.selected",
    ];
    let refuse = [
        ("reconciliation.requested", "reconciliation.concluded"),
        ("run.start_delayed", "run.start_released"),
    ];
    for (name, rule) in RESET_RULES {
        let expected = if exclude.contains(name) {
            ResetRule::Exclude
        } else if let Some((_, until)) = refuse.iter().find(|(opened, _)| opened == name) {
            ResetRule::Refuse { until: Some(until) }
        } else {
            ResetRule::Copy
        };
        assert_eq!(*rule, expected, "{name}");
    }
    assert_eq!(
        RESET_RULES
            .iter()
            .filter(|(_, rule)| *rule == ResetRule::Exclude)
            .count(),
        exclude.len()
    );
}

#[test]
fn a_type_this_version_does_not_know_has_no_rule() {
    assert_eq!(rule_by_name("agent.started"), None);
    assert_eq!(rule_by_name("run.reset"), Some(ResetRule::Copy));
}

// -------------------------------------------------------------------------------------------- the corpus

#[test]
fn a_sequential_run_can_be_reset_to_its_start_and_after_every_step() {
    let mut run = Harness::start(build(
        "a",
        vec![
            activity("a").with_next(id("b")),
            activity("b").with_next(id("c")),
            activity("c"),
        ],
    ));
    for node in ["a", "b", "c"] {
        let fence = run.lease(node);
        run.complete(node, fence).expect("complete");
    }
    let found = points(&run);
    let mut expected = vec![1];
    expected.extend(sequences(&run, "step.completed"));
    assert_eq!(eligible(&found), expected);
    assert!(found.blocked.is_empty());
    assert_eq!(
        found
            .eligible
            .iter()
            .map(|point| point
                .completes
                .as_ref()
                .map(|node| node.as_str().to_owned()))
            .collect::<Vec<_>>(),
        [None, Some("a".into()), Some("b".into()), Some("c".into())]
    );
    // every event of the journal has a rule, and the table by name is the same as the match
    for event in &run.journal {
        assert_eq!(rule_by_name(event.name()), Some(rule_of(event)));
    }
}

#[test]
fn a_point_while_a_branch_is_still_open_is_not_offered() {
    let mut run = Harness::start(build(
        "fork",
        vec![
            parallel("fork", &["l", "r"], JoinPolicy::All).with_next(id("after")),
            activity("l"),
            activity("r"),
            activity("after"),
        ],
    ));
    let l = run.lease("l");
    run.complete("l", l).expect("l");
    let r = run.lease("r");
    run.complete("r", r).expect("r");
    let after = run.lease("after");
    run.complete("after", after).expect("after");

    let found = points(&run);
    let completed = sequences(&run, "step.completed");
    // l completed while the fork was parked on its branches and r had not run: work is open
    assert!(matches!(
        blocked(&found, completed[0]),
        Some(ResetBlock::OpenWork(_))
    ));
    // so is r's completion, where both branches are done but the fork has not joined: nothing is runnable.
    // `after` is the first clean point after the fork
    assert_eq!(
        blocked(&found, completed[1]),
        Some(&ResetBlock::NotRunning(RunStatus::Waiting))
    );
    assert!(eligible(&found).contains(&completed[2]), "{found:?}");
    assert!(eligible(&found).contains(&1));
    assert!(!eligible(&found).contains(&completed[0]));
}

#[test]
fn a_wait_and_a_timer_are_open_work_until_they_end() {
    let mut run = Harness::start(build(
        "t",
        vec![
            timer("t", 60).with_next(id("w")),
            wait("w", 300, None).with_next(id("a")),
            activity("a"),
        ],
    ));
    run.advance(std::time::Duration::from_secs(60));
    run.fire_timer("t").expect("timer");
    run.answer(true, "k");
    run.satisfy_wait("w").expect("wait");
    let fence = run.lease("a");
    run.complete("a", fence).expect("a");

    let found = points(&run);
    // nothing is offered between the start of a timer or a wait and its end; the points are the completions
    for sequence in sequences(&run, "timer.started")
        .into_iter()
        .chain(sequences(&run, "wait.started"))
    {
        assert!(!eligible(&found).contains(&sequence));
    }
    assert!(eligible(&found).contains(&1));
    let last = *sequences(&run, "step.completed")
        .last()
        .expect("a completion");
    assert!(eligible(&found).contains(&last), "{found:?}");
}

#[test]
fn no_point_inside_a_saga_that_later_aborts_is_offered() {
    let mut run = Harness::start(build(
        "s",
        vec![
            saga("s", "reserve"),
            effect("reserve", false).with_next(id("charge")),
            effect("charge", false),
        ],
    ));
    let reserve = run.lease("reserve");
    run.complete("reserve", reserve).expect("reserve");
    let charge = run.lease("charge");
    run.fail("charge", charge, false).expect("charge fails");
    // the saga undoes `reserve`
    while run.state.status() == RunStatus::Compensating {
        let step = run
            .state
            .steps()
            .find(|(_, step)| {
                matches!(
                    step.status(),
                    sagat_domain::execution::StepStatus::Ready
                        | sagat_domain::execution::StepStatus::Pending
                )
            })
            .map(|(node, _)| node.as_str().to_owned())
            .expect("a compensation to run");
        let fence = run.lease(&step);
        run.complete(&step, fence).expect("compensation completes");
    }

    let found = points(&run);
    assert!(run.names().contains(&"compensation.planned"));
    // only the start is offered. The completion of `reserve` is inside the saga, whose node is parked while its
    // body runs, so the run is not plainly running there; the completion of its compensation comes after the
    // saga began to abort, an event type a prefix may not hold.
    assert_eq!(eligible(&found), [1], "{found:?}");
    let reserved = sequences(&run, "step.completed")[0];
    assert!(matches!(
        blocked(&found, reserved),
        Some(ResetBlock::NotRunning(RunStatus::Waiting))
    ));
    let undone = *sequences(&run, "step.completed")
        .last()
        .expect("a compensation");
    assert!(matches!(
        blocked(&found, undone),
        Some(ResetBlock::ExcludedEvent("saga.aborting"))
    ));
}

#[test]
fn a_point_before_a_step_that_later_fails_and_is_resolved_is_still_offered() {
    // the failure and the operator's decision after the point concern a step the point does not hold
    let mut run = Harness::start(build(
        "a",
        vec![activity("a").with_next(id("b")), activity("b")],
    ));
    let a = run.lease("a");
    run.complete("a", a).expect("a");
    let b = run.lease("b");
    run.fail("b", b, false).expect("b fails");
    let found = points(&run);
    assert!(run.names().contains(&"run.finished") || run.state.status() != RunStatus::Running);
    let completed = sequences(&run, "step.completed");
    assert_eq!(eligible(&found), [1, completed[0]], "{found:?}");
}

#[test]
fn a_point_inside_the_window_of_an_unknown_outcome_is_refused_and_one_after_it_is_not() {
    // `pay` is leased, its lease is lost (an unknown outcome, reconciled by a lookup), while `other` runs
    // beside it; `after` follows the join
    let mut run = Harness::start(build(
        "fork",
        vec![
            parallel("fork", &["pay", "other"], JoinPolicy::All).with_next(id("after")),
            reconciled("pay"),
            activity("other"),
            activity("after"),
        ],
    ));
    let pay = run.lease("pay");
    run.expire("pay", pay, Expiry::LeaseExpired)
        .expect("the lost lease records the unknown outcome");
    let other = run.lease("other");
    run.complete("other", other).expect("other completes");
    let lookup = run.lease("pay~r");
    run.reconcile("pay~r", lookup, ReconcileVerdict::Applied)
        .expect("the lookup finds the effect");
    let after = run.lease("after");
    run.complete("after", after).expect("after");

    let found = points(&run);
    let names = run.names();
    let requested = sequences(&run, "reconciliation.requested")[0];
    let concluded = sequences(&run, "reconciliation.concluded")[0];
    assert!(requested < concluded, "{names:?}");
    // `other` completed while the reconciliation was open: the window is open at that point
    let inside = sequences(&run, "step.completed")
        .into_iter()
        .find(|sequence| *sequence > requested && *sequence < concluded)
        .expect("a completion inside the window");
    assert_eq!(
        blocked(&found, inside),
        Some(&ResetBlock::OpenWindow("reconciliation.requested"))
    );
    // the window closes with the conclusion: the last completion is not blocked by it
    let last = *sequences(&run, "step.completed").last().expect("after");
    assert!(eligible(&found).contains(&last), "{names:?} {found:?}");
}

#[test]
fn a_completion_while_the_run_is_paused_is_not_offered() {
    let mut run = Harness::start(build(
        "a",
        vec![activity("a").with_next(id("b")), activity("b")],
    ));
    let a = run.lease("a");
    run.pause().expect("pause");
    run.complete("a", a)
        .expect("the attempt in flight completes");
    let found = points(&run);
    let completed = sequences(&run, "step.completed")[0];
    assert_eq!(
        blocked(&found, completed),
        Some(&ResetBlock::NotRunning(RunStatus::Paused))
    );
    assert_eq!(eligible(&found), [1]);
}

#[test]
fn a_child_node_is_a_point_when_its_child_has_finished() {
    let mut run = Harness::start(build(
        "c",
        vec![
            child("c", CancelPolicy::Propagate).with_next(id("after")),
            activity("after"),
        ],
    ));
    run.answer(true, "");
    let kid = run.child_of("c");
    run.finish_child("c", kid, ChildOutcome::Succeeded)
        .expect("the child is done");
    let after = run.lease("after");
    run.complete("after", after).expect("after");
    let found = points(&run);
    // `child.finished` and the completion of `after` are the points of this run, besides its start
    let finished = sequences(&run, "child.finished")[0];
    assert!(eligible(&found).contains(&finished), "{found:?}");
    assert!(eligible(&found).contains(&1));
    assert!(found.eligible.iter().any(|point| {
        point
            .completes
            .as_ref()
            .is_some_and(|node| node.as_str() == "after")
    }));
}

#[test]
fn the_plan_lists_what_is_restored_and_what_runs_again_with_its_effects() {
    let mut run = Harness::start(build(
        "a",
        vec![
            activity("a").with_next(id("b")),
            // an effect that cannot be undone and whose outcome may be unknown: it declares a reconciliation and,
            // being irreversible, no compensation
            with_effect(
                reconciled("b"),
                EffectPolicy {
                    irreversible: true,
                    high_risk: true,
                    ..EffectPolicy::default()
                },
            )
            .with_next(id("c")),
            // an effect that can be undone
            effect("c", false),
        ],
    ));
    for node in ["a", "b", "c"] {
        let fence = run.lease(node);
        run.complete(node, fence).expect("complete");
    }
    let def = run.def.clone();
    let journal = addressed(&run);
    let after_a = sequences(&run, "step.completed")[0];
    let plan = reset_plan(&def, run.state.run_id(), &journal, after_a).expect("a plan");

    assert_eq!(
        plan.restored
            .iter()
            .map(|step| step.node.as_str())
            .collect::<Vec<_>>(),
        ["a"]
    );
    assert_eq!(
        plan.reexecuted
            .iter()
            .map(|step| (step.node.as_str(), step.completed_in_source))
            .collect::<Vec<_>>(),
        [("b", true), ("c", true)]
    );
    assert!(plan.needs_acknowledgement());
    assert_eq!(
        plan.reexecuted[0].effects,
        EffectMarkers {
            compensate_with: false,
            reconcile_with: true,
            irreversible: true,
            high_risk: true,
        }
    );
    assert_eq!(
        plan.reexecuted[1].effects,
        EffectMarkers {
            compensate_with: true,
            ..EffectMarkers::default()
        }
    );

    // a sequence that is not a point is refused, and so is one that is blocked
    assert_eq!(
        reset_plan(&def, run.state.run_id(), &journal, 2),
        Err(ResetRefusal::NotAPoint)
    );
    let start = reset_plan(&def, run.state.run_id(), &journal, 1).expect("the start");
    assert!(start.restored.iter().all(|step| step.node.as_str() == "a"));
}

fn reset_event(source_sequence: u64) -> RunEvent {
    RunEvent::RunReset {
        source_run_id: RunId::generate(),
        source_sequence,
        prefix_digest: Digest::new([0x77; 32]),
        reapply: ResetReapply::Signals,
    }
}

#[test]
fn a_reset_event_follows_the_restored_prefix_of_a_run_that_has_started_and_is_open() {
    let entry = NodeId::parse("a").expect("a node");
    // before the run has started there is no prefix to follow
    let mut fresh = RunState::new(RunId::generate());
    assert!(fresh.apply(&reset_event(1)).is_err());

    let mut state = RunState::new(RunId::generate());
    state
        .apply(&RunEvent::RunStarted { entrypoint: entry })
        .expect("the run starts");
    // a prefix has at least its `run.started`
    assert!(state.apply(&reset_event(0)).is_err());
    state
        .apply(&reset_event(1))
        .expect("the reset follows the prefix");

    // not once the run is stopping
    state
        .apply(&RunEvent::RunStopRequested {
            reason: StopReason::Cancelled,
            code: None,
        })
        .expect("stop");
    assert!(state.apply(&reset_event(1)).is_err());
}

#[test]
fn the_reapply_modes_name_what_is_delivered_again() {
    for (mode, name, signals, updates) in [
        (ResetReapply::None, "NONE", false, false),
        (ResetReapply::Signals, "SIGNALS", true, false),
        (ResetReapply::Updates, "UPDATES", false, true),
        (ResetReapply::All, "ALL", true, true),
    ] {
        assert_eq!(mode.as_str(), name);
        assert_eq!(ResetReapply::parse(name), Some(mode));
        assert_eq!((mode.signals(), mode.updates()), (signals, updates));
    }
    assert_eq!(ResetReapply::parse("EVERYTHING"), None);
    assert_eq!(ResetReapply::default(), ResetReapply::None);
}

#[test]
fn a_reset_run_goes_on_from_the_point_as_the_source_did_and_replays_to_its_state() {
    let mut source = Harness::start(build(
        "a",
        vec![
            activity("a").with_next(id("b")),
            activity("b").with_next(id("c")),
            activity("c"),
        ],
    ));
    for node in ["a", "b", "c"] {
        let fence = source.lease(node);
        source.complete(node, fence).expect("complete");
    }
    let after_a = sequences(&source, "step.completed")[0];
    let prefix: Vec<AddressedRunEvent> = addressed(&source)
        .into_iter()
        .take(usize::try_from(after_a).expect("small"))
        .collect();

    // the new run: the prefix applied, then the reset and the settle reduction
    let reset_run = RunId::generate();
    let mut state = RunState::replay_addressed(reset_run, &prefix).expect("the prefix replays");
    let progress = sagat_domain::execution::record_reset(
        &source.def,
        &mut state,
        source.now,
        source.state.run_id(),
        after_a,
        Digest::new([0x42; 32]),
        ResetReapply::None,
    )
    .expect("the reset applies");
    let names: Vec<_> = progress.events.iter().map(|event| event.name()).collect();
    // the successor of the point is routed and released, as it was when `a` completed in the source
    assert_eq!(
        names,
        ["run.reset", "step.routed", "step.released"],
        "{names:?}"
    );
    assert_eq!(
        state.step(&id("b")).map(|step| step.status()),
        Some(sagat_domain::execution::StepStatus::Ready)
    );

    // the journal alone rebuilds the state
    let mut journal: Vec<RunEvent> = prefix.iter().map(|event| event.event().clone()).collect();
    journal.extend(progress.events);
    let replayed = RunState::replay(reset_run, &journal).expect("the journal replays");
    assert_eq!(replayed, state);
}

fn items() -> NodeId {
    NodeId::parse("items").expect("a node")
}

/// The journal of a collection of one item that runs to success: entered, the item admitted, returned and
/// checkpointed, and the collection finished.
fn collection_journal(finished: bool) -> Vec<AddressedRunEvent> {
    let body = ExecutionInstanceId::in_path(
        InstancePath::root().child(items(), 0).expect("a path"),
        NodeId::parse("work").expect("a node"),
    );
    let mut events = vec![
        RunEvent::RunStarted {
            entrypoint: items(),
        },
        RunEvent::CollectionEntered {
            node: items(),
            item_count: 1,
            max_items: 1,
            max_concurrency: 1,
            snapshot_digest: Digest::new([0x51; 32]),
        },
        RunEvent::CollectionIterationAdmitted {
            node: items(),
            index: 0,
            entry: body.clone(),
            item_digest: Digest::new([0; 32]),
        },
        RunEvent::CollectionIterationReturned {
            node: items(),
            index: 0,
            return_instance: body.clone(),
        },
        RunEvent::CollectionIterationCheckpointed {
            node: items(),
            index: 0,
            return_instance: body,
            result_digest: Digest::new([0x80; 32]),
        },
    ];
    if finished {
        events.push(RunEvent::CollectionFinished {
            node: items(),
            outcome: CollectionOutcome::Succeeded,
            result_digest: Some(Digest::new([0x90; 32])),
        });
    }
    events.into_iter().map(AddressedRunEvent::root).collect()
}

#[test]
fn a_collection_is_a_point_when_it_has_finished_and_never_while_it_is_open() {
    let run = RunId::generate();
    // finished: the start and the finish of the collection are the points
    let done = reset_points(run, &collection_journal(true)).expect("a journal that replays");
    assert_eq!(eligible(&done), [1, 6], "{done:?}");
    assert_eq!(
        done.eligible[1]
            .completes
            .as_ref()
            .map(|node| node.as_str().to_owned()),
        Some("items".to_owned())
    );
    // not finished: only the start; nothing in the middle of a collection is a point
    let open = reset_points(run, &collection_journal(false)).expect("a journal that replays");
    assert_eq!(eligible(&open), [1], "{open:?}");
    assert!(open.blocked.is_empty());
}

#[test]
fn a_prefix_with_an_event_of_an_iteration_is_not_offered_in_this_version() {
    // the body step of the one item was released at the path of its iteration, below the root, and the
    // collection was then cancelled and drained
    let body = ExecutionInstanceId::in_path(
        InstancePath::root().child(items(), 0).expect("a path"),
        NodeId::parse("work").expect("a node"),
    );
    let journal: Vec<AddressedRunEvent> = vec![
        AddressedRunEvent::root(RunEvent::RunStarted {
            entrypoint: items(),
        }),
        AddressedRunEvent::root(RunEvent::CollectionEntered {
            node: items(),
            item_count: 1,
            max_items: 1,
            max_concurrency: 1,
            snapshot_digest: Digest::new([0x51; 32]),
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationAdmitted {
            node: items(),
            index: 0,
            entry: body.clone(),
            item_digest: Digest::new([0; 32]),
        }),
        AddressedRunEvent::in_path(
            body.path().clone(),
            RunEvent::StepReleased {
                node: NodeId::parse("work").expect("a node"),
            },
        )
        .expect("an address"),
        AddressedRunEvent::root(RunEvent::CollectionDrainStarted {
            node: items(),
            reason: CollectionDrainReason::Cancelled,
        }),
        AddressedRunEvent::root(RunEvent::CollectionIterationDrained {
            node: items(),
            index: 0,
        }),
        AddressedRunEvent::root(RunEvent::CollectionFinished {
            node: items(),
            outcome: CollectionOutcome::Cancelled,
            result_digest: None,
        }),
    ];
    let found = reset_points(RunId::generate(), &journal).expect("a journal that replays");
    // the start has no scoped event before it; the finish of the collection has one
    assert_eq!(eligible(&found), [1], "{found:?}");
    assert_eq!(blocked(&found, 7), Some(&ResetBlock::ScopedEvents));
}

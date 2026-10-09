//! Scenario tests for sagas, compensations and reconciliation (T003-004, T003-009, T003-014,
//! T003-019, T003-028; FR-003-001..007).
//!
//! Each scenario drives a run exactly as the application does, checks the journal, and checks that
//! the journal alone rebuilds the state.
#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{
    Harness, activity, build, condition, effect, effect_with, flaky, id, ms, parallel, saga,
    saga_with, sid, with_effect,
};
use sagat_domain::execution::{
    ExecutionError, Expiry, ReviewOutcome, RunStatus, SagaPhase, StepStatus, StopReason, WaitingOn,
};
use sagat_domain::identity::Slug;
use sagat_domain::saga::{
    EffectPolicy, OnCompensationFailure, ReconcileVerdict, SagaCancelPolicy, SagaOutcome,
    compensation_plans,
};
use sagat_domain::workflow::{
    ActivityName, BackoffStrategy, JoinPolicy, Node, NodeId, NodeKind, RetryPolicy, Scope,
    WorkflowDefinition, WorkflowError,
};

/// `settlement`: reserve → submit → (after the saga) notify.
fn settlement() -> WorkflowDefinition {
    build(
        "settlement",
        vec![
            saga("settlement", "reserve").with_next(id("notify")),
            effect("reserve", false).with_next(id("submit")),
            effect("submit", true),
            activity("notify"),
        ],
    )
}

fn names(run: &Harness) -> Vec<&'static str> {
    run.names()
}

fn comp(run: &Harness, saga: &str) -> sagat_domain::execution::SagaState {
    run.state.saga(&id(saga)).expect("saga").clone()
}

// ------------------------------------------------------------------ the definition

#[test]
fn a_compensation_and_a_reconciliation_are_derived_nodes_that_nothing_routes_to() {
    let def = settlement();
    assert_eq!(def.len(), 4, "the author wrote four nodes");
    assert_eq!(def.total_len(), 7, "and the engine can run three more");

    let undo = def.node(&sid("reserve~c")).expect("the compensation");
    assert_eq!(
        undo.kind(),
        &NodeKind::Activity {
            activity: ActivityName::parse("undo.reserve").expect("name"),
            retry: RetryPolicy::COMPENSATION,
            compensate_with: None,
            reconcile_with: None,
            idempotency_template: None,
            secrets: Vec::new(),
        }
    );
    assert!(def.node(&sid("submit~r")).is_some(), "the reconciliation");
    assert!(def.node(&sid("submit~c")).is_some());
    assert!(
        def.nodes().all(|n| !n.id().is_derived()),
        "the nodes the author sees are the ones the author wrote"
    );
    assert!(
        def.all_nodes()
            .all(|n| n.successors().is_empty() || !n.id().is_derived()),
        "a derived node has no successors"
    );
    assert!(
        def.warnings().is_empty(),
        "inside a saga the compensation will run"
    );
}

#[test]
fn the_author_cannot_write_an_id_that_collides_with_a_derived_one() {
    assert!(NodeId::parse("reserve~c").is_err());
    assert!(NodeId::parse("~r").is_err());
    // Read back from a journal, only the two derived forms of a valid id are accepted.
    assert!(NodeId::parse_stored("reserve~c").is_ok());
    assert!(NodeId::parse_stored("reserve~r").is_ok());
    assert!(NodeId::parse_stored("reserve~x").is_err());
    assert!(NodeId::parse_stored("reserve~c~c").is_err());
    assert!(NodeId::parse_stored("~c").is_err());
    assert_eq!(sid("reserve~c").forward(), Some(id("reserve")));
    assert_eq!(id("reserve").forward(), None);
}

#[test]
fn the_plan_is_the_reverse_of_the_forward_order() {
    let def = build(
        "settlement",
        vec![
            saga("settlement", "a"),
            effect("a", false).with_next(id("b")),
            effect("b", false).with_next(id("c")),
            effect("c", false),
        ],
    );
    let plans = compensation_plans(&def);
    assert_eq!(plans.len(), 1);
    let order: Vec<_> = plans[0].entries.iter().map(|e| e.node.as_str()).collect();
    assert_eq!(order, ["c", "b", "a"], "the last effect is undone first");
    let waits: Vec<_> = plans[0]
        .entries
        .iter()
        .map(|e| {
            (
                e.node.as_str(),
                e.after.iter().map(NodeId::as_str).collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        waits,
        [("c", vec![]), ("b", vec!["c"]), ("a", vec!["b"])],
        "each one waits for the nearest one after it, which in turn waits for the rest"
    );
    assert_eq!(plans[0].entries[0].compensation, sid("c~c"));
}

#[test]
fn branches_of_a_parallel_node_are_independent_in_the_plan() {
    let def = build(
        "settlement",
        vec![
            saga("settlement", "start"),
            effect("start", false).with_next(id("fan")),
            parallel("fan", &["a1", "b1"], JoinPolicy::All).with_next(id("after")),
            effect("a1", false).with_next(id("a2")),
            effect("a2", false),
            effect("b1", false),
            effect("after", false),
        ],
    );
    let plan = &compensation_plans(&def)[0];
    let after = |node: &str| -> Vec<&str> {
        plan.entries
            .iter()
            .find(|e| e.node.as_str() == node)
            .expect("entry")
            .after
            .iter()
            .map(NodeId::as_str)
            .collect()
    };
    assert_eq!(
        after("after"),
        Vec::<&str>::new(),
        "after the join: undone first"
    );
    assert_eq!(
        after("a2"),
        ["after"],
        "the end of a branch waits for what came after the join"
    );
    assert_eq!(after("b1"), ["after"]);
    assert_eq!(after("a1"), ["a2"], "a branch is undone in reverse");
    assert_eq!(
        after("start"),
        ["a1", "after", "b1"],
        "what ran before the fork waits for every branch and for what came after the join"
    );
    // The two branches never wait for each other.
    assert!(!after("a1").contains(&"b1") && !after("b1").contains(&"a1"));
}

#[test]
fn a_saga_body_is_a_closed_region_without_an_end() {
    let leaks = WorkflowDefinition::new(
        Slug::parse("demo-flow").expect("slug"),
        id("outside"),
        vec![
            activity("outside").with_next(id("inside")),
            saga("s", "inside"),
            effect("inside", false),
        ],
    );
    assert!(leaks.is_err(), "{leaks:?}");

    let with_end = WorkflowDefinition::new(
        Slug::parse("demo-flow").expect("slug"),
        id("s"),
        vec![
            saga("s", "a"),
            effect("a", false).with_next(id("done")),
            common::end("done"),
        ],
    );
    assert!(matches!(with_end, Err(WorkflowError::EndInsideSaga { .. })));
}

#[test]
fn an_effect_cannot_be_both_irreversible_and_compensated_and_only_an_activity_has_one() {
    let both = WorkflowDefinition::new(
        Slug::parse("demo-flow").expect("slug"),
        id("a"),
        vec![with_effect(
            effect("a", false),
            EffectPolicy {
                irreversible: true,
                ..EffectPolicy::NONE
            },
        )],
    );
    assert!(matches!(
        both,
        Err(WorkflowError::IrreversibleWithCompensation(_))
    ));
    let on_timer = WorkflowDefinition::new(
        Slug::parse("demo-flow").expect("slug"),
        id("t"),
        vec![with_effect(
            common::timer("t", 1),
            EffectPolicy {
                high_risk: true,
                ..EffectPolicy::NONE
            },
        )],
    );
    assert!(matches!(
        on_timer,
        Err(WorkflowError::EffectOnNonActivity(_))
    ));
}

#[test]
fn scopes_nest_and_the_innermost_one_decides() {
    let def = build(
        "outer",
        vec![
            saga("outer", "fan"),
            parallel("fan", &["inner", "other"], JoinPolicy::All),
            saga("inner", "a").with_next(id("tail")),
            effect("a", false),
            activity("tail"),
            activity("other"),
        ],
    );
    assert_eq!(
        def.enclosing_scope(&id("a")),
        Some(&Scope::Saga(id("inner")))
    );
    assert_eq!(
        def.enclosing_scope(&id("inner")),
        Some(&Scope::Parallel(id("fan")))
    );
    assert_eq!(
        def.enclosing_scope(&id("fan")),
        Some(&Scope::Saga(id("outer")))
    );
    let chain: Vec<_> = def
        .scope_chain(&id("a"))
        .map(|s| s.owner().as_str())
        .collect();
    assert_eq!(chain, ["inner", "fan", "outer"]);
    assert_eq!(def.enclosing_scope(&id("outer")), None);
}

// ------------------------------------------------------------------ US02: compensate after failure

#[test]
fn a_saga_that_completes_just_continues() {
    let mut run = Harness::start(settlement());
    assert_eq!(
        names(&run),
        ["run.started", "saga.entered", "step.released"],
        "the saga step waits on its body, the body starts"
    );
    assert_eq!(run.status("settlement"), StepStatus::Waiting);
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    run.complete("submit", f).expect("submit");
    assert_eq!(run.status("settlement"), StepStatus::Succeeded);
    assert_eq!(comp(&run, "settlement").phase(), SagaPhase::Done);
    assert_eq!(comp(&run, "settlement").outcome(), None);
    let f = run.lease("notify");
    run.complete("notify", f).expect("notify");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    run.assert_replays();
}

#[test]
fn reserve_completed_and_submit_failed_for_good_compensates_the_reservation() {
    let mut run = Harness::start(settlement());
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    run.fail_with("submit", f, "PAYMENT_DECLINED", false)
        .expect("submit fails for good");

    // The coordinator took over: the plan was recorded and the compensation is ready.
    assert_eq!(comp(&run, "settlement").phase(), SagaPhase::Compensating);
    assert_eq!(run.state.status(), RunStatus::Compensating);
    assert_eq!(
        comp(&run, "settlement")
            .cause()
            .map(|c| c.as_str().to_owned()),
        Some("PAYMENT_DECLINED".to_owned())
    );
    let saga_state = comp(&run, "settlement");
    assert_eq!(
        saga_state
            .entries()
            .keys()
            .map(NodeId::as_str)
            .collect::<Vec<_>>(),
        ["reserve"],
        "only what completed is compensated"
    );
    assert_eq!(run.status("reserve~c"), StepStatus::Ready);
    assert!(
        !run.has_step("submit~c"),
        "submit never completed, nothing to undo"
    );

    // The compensation is an ordinary activity attempt.
    let f = run.lease("reserve~c");
    run.complete("reserve~c", f).expect("release");
    assert_eq!(run.state.status(), RunStatus::Compensated);
    assert_eq!(
        comp(&run, "settlement").outcome(),
        Some(SagaOutcome::Compensated)
    );
    assert_eq!(run.status("settlement"), StepStatus::Failed);
    assert!(!run.has_step("notify"), "nothing after the saga ran");

    let names = names(&run);
    let at = |name: &str| names.iter().position(|n| *n == name).expect("event");
    assert!(
        at("step.failed") < at("saga.aborting")
            && at("saga.aborting") < at("compensation.planned")
            && at("compensation.planned") < at("saga.compensated")
            && at("saga.compensated") < at("run.stop_requested")
            && at("run.stop_requested") < at("run.finished"),
        "{names:?}"
    );
    run.assert_replays();
}

#[test]
fn compensations_run_in_reverse_order_and_each_waits_for_the_one_after_it() {
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga("settlement", "a"),
            effect("a", false).with_next(id("b")),
            effect("b", false).with_next(id("c")),
            effect("c", false).with_next(id("d")),
            activity("d"),
        ],
    ));
    for node in ["a", "b", "c"] {
        let f = run.lease(node);
        run.complete(node, f).expect("forward");
    }
    let f = run.lease("d");
    run.fail_with("d", f, "BOOM", false).expect("d fails");

    // Only the last effect can be undone first.
    assert_eq!(run.status("c~c"), StepStatus::Ready);
    assert_eq!(run.status("b~c"), StepStatus::Pending);
    assert_eq!(run.status("a~c"), StepStatus::Pending);
    let f = run.lease("c~c");
    run.complete("c~c", f).expect("undo c");
    assert_eq!(run.status("b~c"), StepStatus::Ready);
    assert_eq!(run.status("a~c"), StepStatus::Pending);
    let f = run.lease("b~c");
    run.complete("b~c", f).expect("undo b");
    assert_eq!(run.status("a~c"), StepStatus::Ready);
    let f = run.lease("a~c");
    run.complete("a~c", f).expect("undo a");
    assert_eq!(run.state.status(), RunStatus::Compensated);
    run.assert_replays();
}

#[test]
fn independent_branches_are_compensated_at_the_same_time() {
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga("settlement", "fan"),
            parallel("fan", &["a", "b"], JoinPolicy::All),
            effect("a", false),
            effect("b", false),
        ],
    ));
    let fa = run.lease("a");
    let fb = run.lease("b");
    run.complete("a", fa).expect("a");
    run.fail_with("b", fb, "BOOM", false).expect("b fails");

    // The branch that completed is compensated; the one that failed has nothing to undo.
    assert_eq!(run.status("a~c"), StepStatus::Ready);
    assert!(!run.has_step("b~c"));
    let f = run.lease("a~c");
    run.complete("a~c", f).expect("undo a");
    assert_eq!(run.state.status(), RunStatus::Compensated);

    // With both completed, both compensations are released together.
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga("settlement", "fan"),
            parallel("fan", &["a", "b"], JoinPolicy::All).with_next(id("after")),
            effect("a", false),
            effect("b", false),
            activity("after"),
        ],
    ));
    let fa = run.lease("a");
    let fb = run.lease("b");
    run.complete("a", fa).expect("a");
    run.complete("b", fb).expect("b");
    let f = run.lease("after");
    run.fail_with("after", f, "BOOM", false)
        .expect("after fails");
    assert_eq!(run.status("a~c"), StepStatus::Ready);
    assert_eq!(
        run.status("b~c"),
        StepStatus::Ready,
        "independent: no waiting"
    );
    run.assert_replays();
}

#[test]
fn a_failure_that_something_inside_handles_does_not_start_a_compensation() {
    let mut run = Harness::start(build(
        "s",
        vec![
            saga("s", "a"),
            effect("a", false).with_next(id("b")),
            activity("b").with_on_error(id("recover")),
            activity("recover"),
        ],
    ));
    let f = run.lease("a");
    run.complete("a", f).expect("a");
    let f = run.lease("b");
    run.fail_with("b", f, "BOOM", false).expect("b fails");
    assert_eq!(comp(&run, "s").phase(), SagaPhase::Forward);
    let f = run.lease("recover");
    run.complete("recover", f).expect("recover");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    assert!(!run.has_step("a~c"));
}

#[test]
fn an_unhandled_failure_of_a_saga_goes_to_the_error_route_of_the_saga() {
    let mut run = Harness::start(build(
        "s",
        vec![
            saga("s", "a").with_on_error(id("notify_failure")),
            effect("a", false).with_next(id("b")),
            activity("b"),
            activity("notify_failure"),
        ],
    ));
    let f = run.lease("a");
    run.complete("a", f).expect("a");
    let f = run.lease("b");
    run.fail_with("b", f, "BOOM", false).expect("b fails");
    let f = run.lease("a~c");
    run.complete("a~c", f).expect("undo");
    assert_eq!(comp(&run, "s").outcome(), Some(SagaOutcome::Compensated));
    // The author handled the compensated failure: the run goes on and can still succeed.
    assert_eq!(run.status("notify_failure"), StepStatus::Ready);
    let f = run.lease("notify_failure");
    run.complete("notify_failure", f).expect("notify");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    run.assert_replays();
}

// ------------------------------------------------------------------ US04: a compensation fails

fn short_comp(attempts: u32) -> EffectPolicy {
    EffectPolicy {
        compensation_retry: Some(
            RetryPolicy::new(attempts, ms(100), ms(100), BackoffStrategy::Fixed, false)
                .expect("retry"),
        ),
        ..EffectPolicy::NONE
    }
}

/// `reserve` (compensation allowed `attempts` attempts) → `submit`, which fails.
fn failing_settlement(attempts: u32, on_failure: OnCompensationFailure) -> Harness {
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga_with(
                "settlement",
                "reserve",
                on_failure,
                SagaCancelPolicy::CompensateCompleted,
            ),
            with_effect(effect("reserve", false), short_comp(attempts)).with_next(id("submit")),
            activity("submit"),
        ],
    ));
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    run.fail_with("submit", f, "PAYMENT_DECLINED", false)
        .expect("fails");
    run
}

#[test]
fn a_compensation_that_fails_is_retried_with_its_own_budget() {
    let mut run = failing_settlement(3, OnCompensationFailure::ManualReview);
    let f = run.lease("reserve~c");
    run.fail_with("reserve~c", f, "PROVIDER_DOWN", true)
        .expect("fails once");
    assert_eq!(run.status("reserve~c"), StepStatus::RetryWait);
    run.advance(Duration::from_secs(1));
    run.retry_due("reserve~c").expect("due");
    let f = run.lease("reserve~c");
    assert_eq!(f.attempt, 2, "the attempts of the compensation are its own");
    run.complete("reserve~c", f).expect("works now");
    assert_eq!(run.state.status(), RunStatus::Compensated);
    run.assert_replays();
}

#[test]
fn a_compensation_that_exhausts_its_retries_waits_for_an_operator() {
    let mut run = failing_settlement(2, OnCompensationFailure::ManualReview);
    for attempt in 1..=2 {
        let f = run.lease("reserve~c");
        run.fail_with("reserve~c", f, "PROVIDER_DOWN", true)
            .expect("fails");
        if attempt == 1 {
            run.advance(Duration::from_secs(1));
            run.retry_due("reserve~c").expect("due");
        }
    }
    // Retries spent: the run is not failed, it is waiting for a human.
    assert_eq!(run.status("reserve~c"), StepStatus::Waiting);
    assert!(matches!(
        run.state
            .step(&sid("reserve~c"))
            .expect("step")
            .waiting_on(),
        Some(WaitingOn::Operator { .. })
    ));
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    assert!(!run.state.is_finished());
    assert_eq!(comp(&run, "settlement").phase(), SagaPhase::Compensating);

    // The operator asks for another attempt, which works.
    run.operator_retry("reserve~c").expect("retry");
    assert_eq!(run.status("reserve~c"), StepStatus::Ready);
    let f = run.lease("reserve~c");
    assert_eq!(
        f.attempt, 3,
        "a new attempt of the same compensation, the old ones stay in the journal"
    );
    run.complete("reserve~c", f).expect("release");
    assert_eq!(run.state.status(), RunStatus::Compensated);
    // Nothing was erased.
    let n = names(&run);
    assert!(n.contains(&"step.needs_review") && n.contains(&"step.retry_requested"));
    run.assert_replays();
}

#[test]
fn skipping_a_compensation_ends_the_saga_as_partial() {
    let mut run = failing_settlement(1, OnCompensationFailure::ManualReview);
    let f = run.lease("reserve~c");
    run.fail_with("reserve~c", f, "PROVIDER_DOWN", true)
        .expect("fails");
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    run.operator_skip("reserve~c").expect("skip");
    assert_eq!(run.status("reserve~c"), StepStatus::Skipped);
    assert_eq!(
        comp(&run, "settlement").outcome(),
        Some(SagaOutcome::Partial)
    );
    assert_eq!(run.state.status(), RunStatus::Partial);
    run.assert_replays();
}

#[test]
fn an_operator_can_declare_a_compensation_done_outside_the_platform() {
    let mut run = failing_settlement(1, OnCompensationFailure::ManualReview);
    let f = run.lease("reserve~c");
    run.fail_with("reserve~c", f, "PROVIDER_DOWN", false)
        .expect("fails");
    run.operator_resolve("reserve~c", ReviewOutcome::Succeeded)
        .expect("resolve");
    assert_eq!(run.state.status(), RunStatus::Compensated);
    run.assert_replays();
}

#[test]
fn a_double_manual_command_is_refused_and_changes_nothing() {
    let mut run = failing_settlement(1, OnCompensationFailure::ManualReview);
    let f = run.lease("reserve~c");
    run.fail_with("reserve~c", f, "PROVIDER_DOWN", true)
        .expect("fails");
    run.operator_retry("reserve~c").expect("first retry");
    let journal = run.journal.clone();
    let state = run.state.clone();
    assert!(matches!(
        run.operator_retry("reserve~c"),
        Err(ExecutionError::IllegalTransition { .. })
    ));
    assert!(matches!(
        run.operator_skip("reserve~c"),
        Err(ExecutionError::IllegalTransition { .. })
    ));
    assert!(
        run.operator_resolve("reserve~c", ReviewOutcome::Succeeded)
            .is_err()
    );
    assert_eq!(run.journal, journal);
    assert_eq!(run.state, state);
}

#[test]
fn only_a_compensation_can_be_skipped() {
    let mut run = Harness::start(build(
        "a",
        vec![with_effect(
            activity("a"),
            EffectPolicy {
                high_risk: true,
                ..EffectPolicy::NONE
            },
        )],
    ));
    let f = run.lease("a");
    run.expire("a", f, Expiry::LeaseExpired)
        .expect("unknown outcome");
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    assert!(matches!(
        run.operator_skip("a"),
        Err(ExecutionError::Invariant(_))
    ));
}

#[test]
fn when_the_saga_does_not_wait_for_a_human_a_failed_compensation_ends_it_and_the_run() {
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga_with(
                "settlement",
                "a",
                OnCompensationFailure::Fail,
                SagaCancelPolicy::CompensateCompleted,
            ),
            with_effect(effect("a", false), short_comp(1)).with_next(id("b")),
            with_effect(effect("b", false), short_comp(1)).with_next(id("c")),
            activity("c"),
        ],
    ));
    for node in ["a", "b"] {
        let f = run.lease(node);
        run.complete(node, f).expect("forward");
    }
    let f = run.lease("c");
    run.fail_with("c", f, "BOOM", false).expect("c fails");
    // b is undone first; it fails, so a (which waits for b) is abandoned.
    let f = run.lease("b~c");
    run.fail_with("b~c", f, "PROVIDER_DOWN", false)
        .expect("b fails");
    assert_eq!(run.status("b~c"), StepStatus::Failed);
    assert_eq!(
        run.status("a~c"),
        StepStatus::Cancelled,
        "a would be undone out of order"
    );
    assert_eq!(
        comp(&run, "settlement").outcome(),
        Some(SagaOutcome::CompensationFailed)
    );
    assert_eq!(run.state.status(), RunStatus::CompensationFailed);
    run.assert_replays();
}

#[test]
fn a_failed_compensation_does_not_hold_back_an_independent_one() {
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga_with(
                "settlement",
                "fan",
                OnCompensationFailure::Fail,
                SagaCancelPolicy::CompensateCompleted,
            ),
            parallel("fan", &["a", "b"], JoinPolicy::All).with_next(id("after")),
            with_effect(effect("a", false), short_comp(1)),
            with_effect(effect("b", false), short_comp(1)),
            activity("after"),
        ],
    ));
    let fa = run.lease("a");
    let fb = run.lease("b");
    run.complete("a", fa).expect("a");
    run.complete("b", fb).expect("b");
    let f = run.lease("after");
    run.fail_with("after", f, "BOOM", false)
        .expect("after fails");
    let f = run.lease("a~c");
    run.fail_with("a~c", f, "PROVIDER_DOWN", false)
        .expect("a fails");
    // b does not depend on a: it still runs, and it succeeds.
    assert_eq!(run.status("b~c"), StepStatus::Ready);
    let f = run.lease("b~c");
    run.complete("b~c", f).expect("undo b");
    assert_eq!(run.state.status(), RunStatus::CompensationFailed);
    run.assert_replays();
}

// ------------------------------------------------------------------ irreversible effects

#[test]
fn an_irreversible_effect_makes_the_outcome_partial_even_when_everything_else_is_undone() {
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga("settlement", "reserve"),
            effect("reserve", false).with_next(id("settle")),
            with_effect(
                activity("settle"),
                EffectPolicy {
                    irreversible: true,
                    ..EffectPolicy::NONE
                },
            )
            .with_next(id("notify")),
            activity("notify"),
        ],
    ));
    for node in ["reserve", "settle"] {
        let f = run.lease(node);
        run.complete(node, f).expect("forward");
    }
    let f = run.lease("notify");
    run.fail_with("notify", f, "BOOM", false)
        .expect("notify fails");
    assert_eq!(
        comp(&run, "settlement")
            .irreversible()
            .iter()
            .map(NodeId::as_str)
            .collect::<Vec<_>>(),
        ["settle"],
        "the plan says which effect cannot be undone"
    );
    let f = run.lease("reserve~c");
    run.complete("reserve~c", f).expect("undo reserve");
    assert_eq!(
        comp(&run, "settlement").outcome(),
        Some(SagaOutcome::Partial)
    );
    assert_eq!(run.state.status(), RunStatus::Partial);
    run.assert_replays();
}

// ------------------------------------------------------------------ US03: reconciliation

fn reconciled_submit() -> Harness {
    Harness::start(build(
        "settlement",
        vec![
            saga("settlement", "reserve"),
            effect("reserve", false).with_next(id("submit")),
            effect("submit", true).with_next(id("notify")),
            activity("notify"),
        ],
    ))
}

fn lose_the_response(run: &mut Harness) {
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    // The provider accepted the payment, but the worker never heard back.
    run.expire("submit", f, Expiry::LeaseExpired)
        .expect("lease lost");
}

#[test]
fn an_attempt_with_an_unknown_outcome_is_reconciled_before_anything_else() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    assert_eq!(run.status("submit"), StepStatus::Waiting);
    assert!(matches!(
        run.state.step(&id("submit")).expect("step").waiting_on(),
        Some(WaitingOn::Reconciliation)
    ));
    assert_eq!(
        run.status("submit~r"),
        StepStatus::Ready,
        "the lookup is scheduled at once"
    );
    assert_eq!(
        comp(&run, "settlement").phase(),
        SagaPhase::Forward,
        "no retry, no compensation yet"
    );
    assert!(names(&run).contains(&"reconciliation.requested"));
    run.assert_replays();
}

#[test]
fn the_provider_says_the_effect_happened_so_the_step_succeeds_without_a_second_attempt() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    let f = run.lease("submit~r");
    run.reconcile("submit~r", f, ReconcileVerdict::Applied)
        .expect("applied");
    assert_eq!(run.status("submit"), StepStatus::Succeeded);
    assert_eq!(
        run.state.step(&id("submit")).expect("step").attempt(),
        1,
        "the effect was not executed again"
    );
    let f = run.lease("notify");
    run.complete("notify", f).expect("notify");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    run.assert_replays();
}

#[test]
fn the_provider_says_the_effect_did_not_happen_so_the_step_is_tried_again() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    let f = run.lease("submit~r");
    run.reconcile("submit~r", f, ReconcileVerdict::NotApplied)
        .expect("not applied");
    assert_eq!(run.status("submit"), StepStatus::RetryWait);
    run.advance(Duration::from_secs(120));
    run.retry_due("submit").expect("due");
    let f = run.lease("submit");
    assert_eq!(f.attempt, 2);
    run.complete("submit", f).expect("works");
    assert_eq!(run.status("submit"), StepStatus::Succeeded);
    run.assert_replays();
}

#[test]
fn when_the_effect_did_not_happen_and_no_attempt_is_left_the_saga_compensates() {
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga("settlement", "reserve"),
            effect("reserve", false).with_next(id("submit")),
            effect_with("submit", true, RetryPolicy::no_retry()),
        ],
    ));
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    run.expire("submit", f, Expiry::LeaseExpired).expect("lost");
    let f = run.lease("submit~r");
    run.reconcile("submit~r", f, ReconcileVerdict::NotApplied)
        .expect("not applied");
    assert_eq!(run.status("submit"), StepStatus::Failed);
    assert_eq!(comp(&run, "settlement").phase(), SagaPhase::Compensating);
    assert_eq!(run.status("reserve~c"), StepStatus::Ready);
    run.assert_replays();
}

#[test]
fn a_provider_that_cannot_say_is_never_assumed_either_way() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    let f = run.lease("submit~r");
    run.reconcile("submit~r", f, ReconcileVerdict::Unknown)
        .expect("unknown");
    assert!(matches!(
        run.state.step(&id("submit")).expect("step").waiting_on(),
        Some(WaitingOn::Operator { .. })
    ));
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    // The operator checks with the provider and says it happened.
    run.operator_resolve("submit", ReviewOutcome::Succeeded)
        .expect("resolve");
    assert_eq!(run.status("submit"), StepStatus::Succeeded);
    let f = run.lease("notify");
    run.complete("notify", f).expect("notify");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    run.assert_replays();
}

#[test]
fn a_reconciliation_that_cannot_be_completed_sends_the_step_to_an_operator() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    // The lookup itself keeps failing until its (shared) budget is spent.
    for _ in 0..10 {
        let f = run.lease("submit~r");
        run.fail_with("submit~r", f, "LOOKUP_DOWN", true)
            .expect("fails");
        if run.status("submit~r") == StepStatus::RetryWait {
            run.advance(Duration::from_secs(120));
            run.retry_due("submit~r").expect("due");
        }
    }
    assert_eq!(run.status("submit~r"), StepStatus::Failed);
    assert!(matches!(
        run.state.step(&id("submit")).expect("step").waiting_on(),
        Some(WaitingOn::Operator { .. })
    ));
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    run.assert_replays();
}

#[test]
fn a_worker_can_say_it_does_not_know_what_happened() {
    let mut run = reconciled_submit();
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    // Not retryable, not a plain failure: the worker reports an unknown outcome.
    run.fail_with("submit", f, "OUTCOME_UNKNOWN", false)
        .expect("unknown");
    assert_eq!(run.status("submit~r"), StepStatus::Ready);
}

#[test]
fn an_ordinary_failure_is_not_reconciled() {
    let mut run = reconciled_submit();
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    run.fail_with("submit", f, "PAYMENT_DECLINED", false)
        .expect("declined");
    assert!(
        !run.has_step("submit~r"),
        "the worker knows it did not happen"
    );
    assert_eq!(comp(&run, "settlement").phase(), SagaPhase::Compensating);
}

#[test]
fn a_reconciled_step_can_be_reconciled_again_after_a_retry() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    let f = run.lease("submit~r");
    run.reconcile("submit~r", f, ReconcileVerdict::NotApplied)
        .expect("not applied");
    run.advance(Duration::from_secs(120));
    run.retry_due("submit").expect("due");
    let f = run.lease("submit");
    run.expire("submit", f, Expiry::AttemptTimeout)
        .expect("lost again");
    // A second round reopens the same reconciliation.
    assert_eq!(run.status("submit~r"), StepStatus::Ready);
    let f = run.lease("submit~r");
    assert_eq!(f.attempt, 2, "the attempts of the reconciliation go on");
    run.reconcile("submit~r", f, ReconcileVerdict::Applied)
        .expect("applied");
    assert_eq!(run.status("submit"), StepStatus::Succeeded);
    run.assert_replays();
}

#[test]
fn a_reconciliation_is_completed_with_its_verdict_and_nothing_else() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    let f = run.lease("submit~r");
    assert_eq!(
        run.complete("submit~r", f),
        Err(ExecutionError::NeedsVerdict(sid("submit~r")))
    );
}

// ------------------------------------------------------------------ high risk without a lookup

#[test]
fn a_high_risk_step_with_an_unknown_outcome_waits_for_an_operator_instead_of_retrying() {
    let mut run = Harness::start(build(
        "pay",
        vec![with_effect(
            flaky("pay", 5),
            EffectPolicy {
                high_risk: true,
                ..EffectPolicy::NONE
            },
        )],
    ));
    let f = run.lease("pay");
    run.expire("pay", f, Expiry::LeaseExpired).expect("lost");
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    assert_eq!(
        run.status("pay"),
        StepStatus::Waiting,
        "not RETRY_WAIT: four attempts are left"
    );
    run.operator_resolve("pay", ReviewOutcome::Succeeded)
        .expect("it happened");
    assert_eq!(run.state.status(), RunStatus::Succeeded);
    run.assert_replays();

    // An ordinary step with the same loss is retried blindly, as before.
    let mut ordinary = Harness::start(build("pay", vec![flaky("pay", 5)]));
    let f = ordinary.lease("pay");
    ordinary
        .expire("pay", f, Expiry::LeaseExpired)
        .expect("lost");
    assert_eq!(ordinary.status("pay"), StepStatus::RetryWait);
}

#[test]
fn an_operator_can_have_a_step_that_waited_tried_again_or_failed() {
    let high_risk = || {
        Harness::start(build(
            "pay",
            vec![with_effect(
                activity("pay"),
                EffectPolicy {
                    high_risk: true,
                    ..EffectPolicy::NONE
                },
            )],
        ))
    };
    let mut run = high_risk();
    let f = run.lease("pay");
    run.expire("pay", f, Expiry::AttemptTimeout).expect("lost");
    run.operator_retry("pay").expect("retry");
    let f = run.lease("pay");
    assert_eq!(f.attempt, 2);
    run.complete("pay", f).expect("ok");
    assert_eq!(run.state.status(), RunStatus::Succeeded);

    let mut run = high_risk();
    let f = run.lease("pay");
    run.expire("pay", f, Expiry::AttemptTimeout).expect("lost");
    run.operator_resolve(
        "pay",
        ReviewOutcome::Failed(sagat_domain::execution::FailureCode::parse("NOT_PAID").expect("c")),
    )
    .expect("failed");
    assert_eq!(run.state.status(), RunStatus::Failed);
    run.assert_replays();
}

#[test]
fn a_decision_cannot_be_forced_on_a_step_that_waits_for_something_other_than_a_person() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    // `submit` waits for its lookup and `settlement` for its body: neither is anyone's to decide.
    for node in ["submit", "settlement"] {
        let before = run.state.version();
        assert!(
            matches!(
                run.operator_retry(node),
                Err(ExecutionError::IllegalTransition { .. })
            ),
            "retry {node}"
        );
        assert!(
            matches!(
                run.operator_resolve(node, ReviewOutcome::Succeeded),
                Err(ExecutionError::IllegalTransition { .. })
            ),
            "resolve {node}"
        );
        assert_eq!(run.state.version(), before, "{node}: nothing was recorded");
    }
    assert_eq!(
        run.status("submit~r"),
        StepStatus::Ready,
        "the lookup goes on"
    );
    run.assert_replays();
}

// ------------------------------------------------------------------ cancellation and deadlines

/// `reserve` completed, `submit` leased and running.
fn mid_flight(policy: SagaCancelPolicy) -> (Harness, sagat_domain::execution::Fence) {
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga_with(
                "settlement",
                "reserve",
                OnCompensationFailure::ManualReview,
                policy,
            ),
            effect("reserve", false).with_next(id("submit")),
            effect("submit", false),
        ],
    ));
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    run.heartbeat("submit", f).expect("started");
    (run, f)
}

#[test]
fn cancelling_compensates_what_completed_and_ends_cancelled() {
    let (mut run, submit) = mid_flight(SagaCancelPolicy::CompensateCompleted);
    run.stop(StopReason::Cancelled).expect("cancel");
    // The attempt in flight is asked to stop; what completed is compensated once it has.
    assert!(
        run.state
            .step(&id("submit"))
            .expect("step")
            .cancel_requested()
    );
    assert_eq!(run.state.status(), RunStatus::Compensating);
    run.fail_with("submit", submit, "CANCELLED", true)
        .expect("the worker stops");
    assert_eq!(run.status("submit"), StepStatus::Cancelled);
    assert_eq!(
        run.status("reserve~c"),
        StepStatus::Ready,
        "a compensation runs although the run is being stopped"
    );
    // The attempt that was cancelled might have had an effect: its outcome was not unknown (the
    // worker answered), so only the completed step is compensated.
    assert!(!run.has_step("submit~c"));
    let f = run.lease("reserve~c");
    run.complete("reserve~c", f).expect("undo");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    assert_eq!(
        comp(&run, "settlement").outcome(),
        Some(SagaOutcome::Compensated)
    );
    assert!(comp(&run, "settlement").during_stop());
    run.assert_replays();
}

#[test]
fn waiting_for_what_is_in_flight_then_compensating_everything_that_completed() {
    let (mut run, submit) = mid_flight(SagaCancelPolicy::WaitInFlightThenCompensate);
    run.stop(StopReason::Cancelled).expect("cancel");
    assert!(
        !run.state
            .step(&id("submit"))
            .expect("step")
            .cancel_requested(),
        "the attempt in flight is not asked to stop"
    );
    assert_eq!(run.status("submit"), StepStatus::Running);
    // Its effect may happen, so it is compensated like the rest.
    run.complete("submit", submit).expect("finishes");
    assert_eq!(
        run.status("reserve~c"),
        StepStatus::Pending,
        "waits for submit~c"
    );
    assert_eq!(run.status("submit~c"), StepStatus::Ready);
    let f = run.lease("submit~c");
    run.complete("submit~c", f).expect("undo submit");
    let f = run.lease("reserve~c");
    run.complete("reserve~c", f).expect("undo reserve");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    run.assert_replays();
}

#[test]
fn aborting_without_compensation_just_stops() {
    let (mut run, _) = mid_flight(SagaCancelPolicy::AbortNoCompensation);
    run.stop(StopReason::Cancelled).expect("cancel");
    assert!(
        run.state
            .step(&id("submit"))
            .expect("step")
            .cancel_requested()
    );
    let f = sagat_domain::execution::Fence {
        attempt: 1,
        generation: 1,
    };
    run.fail_with("submit", f, "CANCELLED", true)
        .expect("stops");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    assert!(!run.has_step("reserve~c"));
    assert_eq!(comp(&run, "settlement").phase(), SagaPhase::Forward);
    run.assert_replays();
}

#[test]
fn a_deadline_compensates_and_the_run_times_out_only_when_that_is_done() {
    let (mut run, submit) = mid_flight(SagaCancelPolicy::CompensateCompleted);
    run.stop(StopReason::TimedOut).expect("deadline");
    run.fail_with("submit", submit, "CANCELLED", true)
        .expect("stops");
    assert_eq!(
        run.state.status(),
        RunStatus::Compensating,
        "not TIMED_OUT yet: compensations are active"
    );
    let f = run.lease("reserve~c");
    run.complete("reserve~c", f).expect("undo");
    assert_eq!(run.state.status(), RunStatus::TimedOut);
}

#[test]
fn a_cancellation_whose_compensation_fails_is_not_a_clean_cancellation() {
    let (mut run, submit) = mid_flight(SagaCancelPolicy::CompensateCompleted);
    run.stop(StopReason::Cancelled).expect("cancel");
    run.fail_with("submit", submit, "CANCELLED", true)
        .expect("stops");
    // Retries of the compensation are spent, and the operator gives up by cancelling nothing more:
    // the default budget is five attempts.
    for round in 0..5 {
        let f = run.lease("reserve~c");
        run.fail_with("reserve~c", f, "PROVIDER_DOWN", true)
            .expect("fails");
        if round < 4 {
            run.advance(Duration::from_secs(600));
            run.retry_due("reserve~c").expect("due");
        }
    }
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    // A skip is the operator's decision that the reservation will not be released.
    run.operator_skip("reserve~c").expect("skip");
    assert_eq!(run.state.status(), RunStatus::Partial);
    run.assert_replays();
}

#[test]
fn cancelling_a_run_that_waits_for_an_operator_gives_up_on_the_review() {
    let mut run = failing_settlement(1, OnCompensationFailure::ManualReview);
    let f = run.lease("reserve~c");
    run.fail_with("reserve~c", f, "PROVIDER_DOWN", true)
        .expect("fails");
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    run.stop(StopReason::Cancelled).expect("cancel");
    assert_eq!(run.status("reserve~c"), StepStatus::Cancelled);
    assert_eq!(
        run.state.status(),
        RunStatus::CompensationFailed,
        "an effect was left in place"
    );
    run.assert_replays();
}

#[test]
fn cancelling_a_run_whose_forward_step_waits_for_an_operator_gives_up_and_compensates_defensively()
{
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    let f = run.lease("submit~r");
    run.reconcile("submit~r", f, ReconcileVerdict::Unknown)
        .expect("unknown");
    assert_eq!(run.state.status(), RunStatus::ManualReview);

    run.stop(StopReason::Cancelled).expect("cancel");
    // Nobody is going to decide it: the step is given up on, and because its outcome was unknown
    // it is undone like what completed.
    assert_eq!(run.status("submit"), StepStatus::Cancelled);
    assert_eq!(run.status("submit~c"), StepStatus::Ready);
    assert_eq!(run.status("reserve~c"), StepStatus::Pending);
    assert_ne!(run.state.status(), RunStatus::ManualReview);
    let f = run.lease("submit~c");
    run.complete("submit~c", f).expect("undo submit");
    let f = run.lease("reserve~c");
    run.complete("reserve~c", f).expect("undo reserve");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    run.assert_replays();
}

#[test]
fn an_unknown_outcome_is_reconciled_before_a_saga_that_is_being_stopped_compensates() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    run.stop(StopReason::Cancelled).expect("cancel");
    // The saga waits for the answer: whether `submit` happened decides what is compensated.
    assert_eq!(run.status("submit"), StepStatus::Waiting);
    assert_eq!(run.status("submit~r"), StepStatus::Ready);
    let f = run.lease("submit~r");
    run.reconcile("submit~r", f, ReconcileVerdict::Applied)
        .expect("applied");
    assert_eq!(run.status("submit"), StepStatus::Succeeded);
    assert_eq!(
        run.status("submit~c"),
        StepStatus::Ready,
        "it happened, so it is undone first"
    );
    let f = run.lease("submit~c");
    run.complete("submit~c", f).expect("undo submit");
    let f = run.lease("reserve~c");
    run.complete("reserve~c", f).expect("undo reserve");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    run.assert_replays();
}

#[test]
fn neither_a_lookup_nor_a_compensation_in_flight_is_asked_to_stop_when_the_run_is_cancelled() {
    // A lookup that is running when the run is cancelled is what decides what gets undone.
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    let f = run.lease("submit~r");
    run.heartbeat("submit~r", f).expect("the lookup is running");
    run.stop(StopReason::Cancelled).expect("cancel");
    assert!(
        !run.state
            .step(&sid("submit~r"))
            .expect("step")
            .cancel_requested(),
        "the lookup is left to answer"
    );
    run.reconcile("submit~r", f, ReconcileVerdict::Applied)
        .expect("applied");

    // And an undo that runs while the run is being cancelled is not abandoned either.
    let f = run.lease("submit~c");
    run.heartbeat("submit~c", f).expect("the undo is running");
    assert!(
        !run.state
            .step(&sid("submit~c"))
            .expect("step")
            .cancel_requested(),
        "undoing what was done is not work to abandon"
    );
    run.complete("submit~c", f).expect("undone");
    let f = run.lease("reserve~c");
    run.complete("reserve~c", f).expect("undone");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    run.assert_replays();
}

#[test]
fn an_effect_that_may_have_happened_is_compensated_defensively() {
    // `submit` times out with no reconciliation: its outcome is unknown, the retry is cancelled,
    // and the compensation is asked to undo an effect that may never have happened.
    let mut run = Harness::start(build(
        "settlement",
        vec![
            saga("settlement", "reserve"),
            effect("reserve", false).with_next(id("submit")),
            effect("submit", false),
        ],
    ));
    let f = run.lease("reserve");
    run.complete("reserve", f).expect("reserve");
    let f = run.lease("submit");
    run.expire("submit", f, Expiry::LeaseExpired).expect("lost");
    assert_eq!(run.status("submit"), StepStatus::RetryWait);
    run.stop(StopReason::Cancelled).expect("cancel");
    assert_eq!(run.status("submit"), StepStatus::Cancelled);
    assert_eq!(run.status("submit~c"), StepStatus::Ready);
    assert_eq!(run.status("reserve~c"), StepStatus::Pending);
}

#[test]
fn every_way_an_outcome_can_be_unknown_makes_the_effect_compensated_defensively() {
    // A lost lease, an attempt that ran out of time and a worker that says it does not know: none of
    // them tells whether the effect happened, so none of them may leave it out of the plan.
    for cause in ["lease", "timeout", "worker"] {
        let mut run = Harness::start(build(
            "settlement",
            vec![
                saga("settlement", "reserve"),
                effect("reserve", false).with_next(id("submit")),
                effect_with("submit", false, RetryPolicy::no_retry()),
            ],
        ));
        let f = run.lease("reserve");
        run.complete("reserve", f).expect("reserve");
        let f = run.lease("submit");
        match cause {
            "lease" => run.expire("submit", f, Expiry::LeaseExpired).expect("lost"),
            "timeout" => run
                .expire("submit", f, Expiry::AttemptTimeout)
                .expect("late"),
            _ => run
                .fail_with("submit", f, "OUTCOME_UNKNOWN", false)
                .expect("unknown"),
        }
        assert_eq!(
            comp(&run, "settlement").phase(),
            SagaPhase::Compensating,
            "{cause}"
        );
        assert_eq!(
            run.status("submit~c"),
            StepStatus::Ready,
            "{cause}: the effect may have happened, so it is undone first"
        );
        assert_eq!(run.status("reserve~c"), StepStatus::Pending, "{cause}");
        run.assert_replays();
    }
}

#[test]
fn skipping_a_compensation_lets_the_ones_that_waited_for_it_go_on() {
    let mut run = Harness::start(build(
        "tx",
        vec![
            saga("tx", "a"),
            effect("a", false).with_next(id("b")),
            with_effect(effect("b", false), short_comp(1)).with_next(id("c")),
            activity("c"),
        ],
    ));
    for step in ["a", "b"] {
        let f = run.lease(step);
        run.complete(step, f).expect("step");
    }
    let f = run.lease("c");
    run.fail_with("c", f, "BOOM", false).expect("c fails");
    // `b` is undone first and `a` waits for it; the only attempt at undoing `b` fails.
    assert_eq!(run.status("a~c"), StepStatus::Pending);
    let f = run.lease("b~c");
    run.fail_with("b~c", f, "PROVIDER_DOWN", true)
        .expect("fails");
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    assert_eq!(
        run.status("a~c"),
        StepStatus::Pending,
        "still waiting for the decision"
    );

    // The operator accepts that `b` stays done: `a` is still undone.
    run.operator_skip("b~c").expect("skip");
    assert_eq!(
        run.status("a~c"),
        StepStatus::Ready,
        "a skipped compensation does not block the others"
    );
    let f = run.lease("a~c");
    run.complete("a~c", f).expect("undo a");
    assert_eq!(comp(&run, "tx").outcome(), Some(SagaOutcome::Partial));
    assert_eq!(run.state.status(), RunStatus::Partial);
    run.assert_replays();
}

#[test]
fn an_unknown_outcome_only_makes_the_saga_partial_when_nothing_can_undo_what_may_have_happened() {
    let unknown_then_cancel = |submit: Node| {
        let mut run = Harness::start(build(
            "settlement",
            vec![
                saga("settlement", "reserve"),
                effect("reserve", false).with_next(id("submit")),
                submit,
            ],
        ));
        let f = run.lease("reserve");
        run.complete("reserve", f).expect("reserve");
        let f = run.lease("submit");
        run.expire("submit", f, Expiry::LeaseExpired).expect("lost");
        run.stop(StopReason::Cancelled).expect("cancel");
        run
    };
    let finish = |run: &mut Harness| {
        while let Some(node) = ["submit~c", "reserve~c"]
            .into_iter()
            .find(|n| run.has_step(n) && run.status(n) == StepStatus::Ready)
        {
            let f = run.lease(node);
            run.complete(node, f).expect("undo");
        }
    };
    let high_risk = EffectPolicy {
        high_risk: true,
        ..EffectPolicy::NONE
    };

    // High risk and it can be undone: it is undone, and the saga is compensated, not partial.
    let mut run = unknown_then_cancel(with_effect(effect("submit", false), high_risk));
    assert_eq!(run.status("submit~c"), StepStatus::Ready);
    finish(&mut run);
    assert_eq!(
        comp(&run, "settlement").outcome(),
        Some(SagaOutcome::Compensated)
    );
    assert!(comp(&run, "settlement").irreversible().is_empty());

    // An ordinary step with nothing to undo it, and no promise that it is safe to repeat: nothing
    // is claimed about it.
    let mut run = unknown_then_cancel(activity("submit"));
    assert!(!run.has_step("submit~c"));
    finish(&mut run);
    assert_eq!(
        comp(&run, "settlement").outcome(),
        Some(SagaOutcome::Compensated)
    );

    // High risk and nothing to undo it with: the saga cannot claim to have undone everything.
    let mut run = unknown_then_cancel(with_effect(activity("submit"), high_risk));
    finish(&mut run);
    assert_eq!(
        comp(&run, "settlement").outcome(),
        Some(SagaOutcome::Partial)
    );
    assert_eq!(comp(&run, "settlement").irreversible(), [id("submit")]);
}

// ------------------------------------------------------------------ findings of the review

/// `root` forks two sagas, each with a step that completes and a step after it.
fn two_sibling_sagas() -> Harness {
    Harness::start(build(
        "root",
        vec![
            parallel("root", &["s1", "s2"], JoinPolicy::All),
            saga("s1", "a1"),
            effect("a1", false).with_next(id("b1")),
            activity("b1"),
            saga("s2", "a2"),
            with_effect(effect("a2", false), short_comp(1)).with_next(id("b2")),
            activity("b2"),
        ],
    ))
}

#[test]
fn a_sibling_saga_whose_compensation_failed_is_not_hidden_by_the_one_that_ended_the_run() {
    let mut run = two_sibling_sagas();
    for step in ["a1", "a2"] {
        let f = run.lease(step);
        run.complete(step, f).expect("step");
    }
    let f = run.lease("b2");
    run.heartbeat("b2", f).expect("b2 runs");
    // `s1` fails and is undone: that is what the join decides the run with.
    let f1 = run.lease("b1");
    run.fail_with("b1", f1, "BOOM", false).expect("b1 fails");
    let f = run.lease("a1~c");
    run.complete("a1~c", f).expect("undo a1");
    assert_eq!(comp(&run, "s1").outcome(), Some(SagaOutcome::Compensated));

    // `s2` lost its join and compensates too. Its only attempt at undoing fails.
    run.fail_with("b2", f, "CANCELLED", true).ok();
    let f2 = run.lease("a2~c");
    run.fail_with("a2~c", f2, "PROVIDER_DOWN", true)
        .expect("fails");

    // Nobody decided this: it is a person's to decide, and the run is not over.
    assert_eq!(run.state.status(), RunStatus::ManualReview);
    assert_eq!(
        run.status("a2~c"),
        StepStatus::Waiting,
        "the stop that another branch caused does not give up on the review"
    );
    run.operator_skip("a2~c").expect("skip");
    assert_eq!(comp(&run, "s2").outcome(), Some(SagaOutcome::Partial));
    assert_eq!(
        run.state.status(),
        RunStatus::Partial,
        "the run says the worst thing that happened to it"
    );
    run.assert_replays();
}

#[test]
fn a_saga_that_could_not_undo_everything_does_not_let_its_error_route_end_the_run_successfully() {
    let mut run = Harness::start(build(
        "s",
        vec![
            saga("s", "a").with_on_error(id("handled")),
            with_effect(effect("a", false), short_comp(1)).with_next(id("b")),
            activity("b"),
            activity("handled"),
        ],
    ));
    let f = run.lease("a");
    run.complete("a", f).expect("a");
    let f = run.lease("b");
    run.fail_with("b", f, "BOOM", false).expect("b fails");
    let f = run.lease("a~c");
    run.fail_with("a~c", f, "PROVIDER_DOWN", true)
        .expect("fails");
    run.operator_skip("a~c").expect("skip");

    // The handler is for a failure that was undone. This one was not.
    assert_eq!(comp(&run, "s").outcome(), Some(SagaOutcome::Partial));
    assert!(!run.has_step("handled"), "the error route did not run");
    assert_eq!(run.state.status(), RunStatus::Partial);
    run.assert_replays();
}

#[test]
fn cancelling_a_paused_run_still_undoes_what_it_did() {
    let (mut run, submit) = mid_flight(SagaCancelPolicy::CompensateCompleted);
    run.pause().expect("pause");
    run.stop(StopReason::Cancelled).expect("cancel");
    run.fail_with("submit", submit, "CANCELLED", true)
        .expect("stops");

    // Stopping supersedes the pause: the compensation can be leased.
    assert!(!run.state.is_paused());
    let f = run
        .try_lease("reserve~c")
        .expect("a compensation runs although the run was paused");
    run.complete("reserve~c", f).expect("undo");
    assert_eq!(run.state.status(), RunStatus::Cancelled);
    run.assert_replays();
}

#[test]
fn what_ran_after_a_join_is_undone_before_the_branches_that_fed_it() {
    let def = build(
        "settlement",
        vec![
            saga("settlement", "start"),
            effect("start", false).with_next(id("fan")),
            parallel("fan", &["a1", "b1"], JoinPolicy::All).with_next(id("after")),
            effect("a1", false).with_next(id("a2")),
            effect("a2", false),
            effect("b1", false),
            effect("after", false),
        ],
    );
    let plan = &compensation_plans(&def)[0];
    let after = |node: &str| -> Vec<&str> {
        plan.entries
            .iter()
            .find(|e| e.node.as_str() == node)
            .expect("entry")
            .after
            .iter()
            .map(NodeId::as_str)
            .collect()
    };
    assert_eq!(
        after("after"),
        Vec::<&str>::new(),
        "it ran last, so it is undone first"
    );
    assert_eq!(
        after("a2"),
        ["after"],
        "the end of a branch waits for what came after the join"
    );
    assert_eq!(after("b1"), ["after"]);
    assert_eq!(after("a1"), ["a2"]);
}

/// The plan worked out the long way: for every entry, a walk of everything downstream of it.
fn plan_the_long_way(def: &WorkflowDefinition, saga: &str) -> Vec<(String, Vec<String>)> {
    let region = def.saga_region(&id(saga)).expect("region").clone();
    let entries: std::collections::BTreeSet<NodeId> = region
        .iter()
        .filter(|n| def.node(&n.compensation()).is_some())
        .cloned()
        .collect();
    let following = |node: &NodeId| -> Vec<NodeId> {
        let found = def.node(node).expect("node");
        let mut out: Vec<NodeId> = found.successors().into_iter().cloned().collect();
        if found.next().is_none() {
            for scope in def.scope_chain(node) {
                match scope {
                    Scope::Parallel(p) => {
                        if let Some(next) = def.node(p).and_then(|n| n.next()) {
                            out.push(next.clone());
                            break;
                        }
                    }
                    Scope::Saga(_) => break,
                }
            }
        }
        out
    };
    entries
        .iter()
        .map(|entry| {
            let mut after = std::collections::BTreeSet::new();
            let mut seen = std::collections::BTreeSet::new();
            let mut queue = following(entry);
            while let Some(node) = queue.pop() {
                if !seen.insert(node.clone()) {
                    continue;
                }
                if entries.contains(&node) {
                    after.insert(node.as_str().to_owned());
                    continue;
                }
                queue.extend(following(&node));
            }
            (entry.as_str().to_owned(), after.into_iter().collect())
        })
        .collect()
}

#[test]
fn the_plan_is_the_same_as_the_one_worked_out_the_long_way() {
    let shapes: Vec<(&str, WorkflowDefinition)> = vec![
        (
            "a chain with steps that have nothing to undo in between",
            build(
                "s",
                vec![
                    saga("s", "a"),
                    effect("a", false).with_next(id("x")),
                    activity("x").with_next(id("y")),
                    activity("y").with_next(id("b")),
                    effect("b", false),
                ],
            ),
        ),
        (
            "branches joined into a shared tail",
            build(
                "s",
                vec![
                    saga("s", "start"),
                    effect("start", false).with_next(id("fan")),
                    parallel("fan", &["a1", "b1", "c1"], JoinPolicy::All).with_next(id("t1")),
                    effect("a1", false).with_next(id("a2")),
                    effect("a2", false),
                    effect("b1", false),
                    activity("c1"),
                    activity("t1").with_next(id("t2")),
                    activity("t2").with_next(id("end")),
                    effect("end", false),
                ],
            ),
        ),
        (
            "a parallel inside a branch of another",
            build(
                "s",
                vec![
                    saga("s", "outer"),
                    parallel("outer", &["left", "right"], JoinPolicy::All).with_next(id("last")),
                    parallel("left", &["l1", "l2"], JoinPolicy::All),
                    effect("l1", false),
                    effect("l2", false),
                    effect("right", false),
                    effect("last", false),
                ],
            ),
        ),
        (
            "a condition that forks and joins again",
            build(
                "s",
                vec![
                    saga("s", "first"),
                    effect("first", false).with_next(id("gate")),
                    condition("gate", "yes", "no"),
                    effect("yes", false).with_next(id("join")),
                    activity("no").with_next(id("join")),
                    effect("join", false),
                ],
            ),
        ),
    ];
    for (name, def) in shapes {
        let plan = &compensation_plans(&def)[0];
        let mut got: Vec<(String, Vec<String>)> = plan
            .entries
            .iter()
            .map(|e| {
                (
                    e.node.as_str().to_owned(),
                    e.after.iter().map(|n| n.as_str().to_owned()).collect(),
                )
            })
            .collect();
        got.sort();
        let mut want = plan_the_long_way(&def, "s");
        want.sort();
        assert_eq!(got, want, "{name}");
    }
}

#[test]
fn an_attempt_whose_outcome_became_unknown_while_the_run_was_being_cancelled_is_still_undone() {
    // The run is cancelled while `submit` runs and its worker is asked to stop; then the worker
    // dies and the lease runs out. Whether the effect happened is unknown, whichever came first.
    let (mut run, submit) = mid_flight(SagaCancelPolicy::CompensateCompleted);
    run.stop(StopReason::Cancelled).expect("cancel");
    assert!(
        run.state
            .step(&id("submit"))
            .expect("step")
            .cancel_requested()
    );
    run.expire("submit", submit, Expiry::LeaseExpired)
        .expect("lost");

    assert_eq!(run.status("submit"), StepStatus::Cancelled);
    assert_eq!(
        run.status("submit~c"),
        StepStatus::Ready,
        "the effect may have happened, so it is undone like when the lease ran out first"
    );
    run.assert_replays();
}

#[test]
fn a_lookup_that_found_nothing_leaves_the_step_known_not_to_have_happened() {
    let mut run = reconciled_submit();
    lose_the_response(&mut run);
    let f = run.lease("submit~r");
    run.reconcile("submit~r", f, ReconcileVerdict::NotApplied)
        .expect("not applied");
    assert_eq!(run.status("submit"), StepStatus::RetryWait);
    // The saga is stopped while the step backs off: there is nothing to undo for `submit`.
    run.stop(StopReason::Cancelled).expect("cancel");
    assert_eq!(run.status("submit"), StepStatus::Cancelled);
    assert!(
        !run.has_step("submit~c"),
        "the lookup said it did not happen"
    );
    assert!(comp(&run, "settlement").irreversible().is_empty());
}

#[test]
fn a_retry_is_refused_on_a_step_of_a_saga_that_stopped_going_forward() {
    // `s2` waits for a person to say whether `x2` happened when `s1` fails and the join is lost.
    let mut run = Harness::start(build(
        "root",
        vec![
            parallel("root", &["s1", "s2"], JoinPolicy::All),
            saga("s1", "a1"),
            effect("a1", false).with_next(id("b1")),
            activity("b1"),
            saga("s2", "x2"),
            effect("x2", true),
        ],
    ));
    let f = run.lease("x2");
    run.expire("x2", f, Expiry::LeaseExpired).expect("lost");
    let f = run.lease("x2~r");
    run.reconcile("x2~r", f, ReconcileVerdict::Unknown)
        .expect("unknown");
    let f = run.lease("a1");
    run.complete("a1", f).expect("a1");
    let f = run.lease("b1");
    run.fail_with("b1", f, "BOOM", false).expect("b1 fails");
    let f = run.lease("a1~c");
    run.complete("a1~c", f).expect("undo a1");

    // `s2` lost its join: its body is winding down, and `x2` still waits for a person.
    assert_eq!(comp(&run, "s2").phase(), SagaPhase::Aborting);
    assert_eq!(run.status("x2"), StepStatus::Waiting);
    assert!(
        matches!(
            run.operator_retry("x2"),
            Err(ExecutionError::IllegalTransition { .. })
        ),
        "a retry would be cancelled the moment it was granted"
    );
    // What the person can still do is say whether it happened: the saga undoes it, or not.
    run.operator_resolve("x2", ReviewOutcome::Succeeded)
        .expect("it happened");
    assert_eq!(run.status("x2~c"), StepStatus::Ready);
    run.assert_replays();
}

// ------------------------------------------------------------------ nesting

#[test]
fn a_saga_in_a_branch_that_loses_the_join_compensates_what_it_did() {
    let mut run = Harness::start(build(
        "root",
        vec![
            parallel("root", &["inner", "other"], JoinPolicy::All),
            saga("inner", "a"),
            effect("a", false).with_next(id("b")),
            activity("b"),
            activity("other"),
        ],
    ));
    let fa = run.lease("a");
    run.complete("a", fa).expect("a");
    let fb = run.lease("b");
    run.heartbeat("b", fb).expect("b runs");
    let fo = run.lease("other");
    run.fail_with("other", fo, "BOOM", false)
        .expect("other fails");

    // The join failed while the saga was still running: the saga does not just vanish, it stops
    // its own body and undoes `a`.
    assert_eq!(comp(&run, "inner").phase(), SagaPhase::Aborting);
    assert!(run.state.step(&id("b")).expect("step").cancel_requested());
    run.fail_with("b", fb, "CANCELLED", true).expect("b stops");
    assert_eq!(run.status("a~c"), StepStatus::Ready);
    let f = run.lease("a~c");
    run.complete("a~c", f).expect("undo a");
    assert_eq!(
        comp(&run, "inner").outcome(),
        Some(SagaOutcome::Compensated)
    );
    // The run failed because the other branch did; the saga undid its own work cleanly, and that
    // does not turn the failure into a compensated one.
    assert_eq!(run.state.status(), RunStatus::Failed);
    run.assert_replays();
}

#[test]
fn a_saga_that_completed_before_the_join_failed_is_not_compensated_by_it() {
    // Compensation is the business of a saga that is still running; one that finished committed.
    let mut run = Harness::start(build(
        "root",
        vec![
            parallel("root", &["inner", "other"], JoinPolicy::All),
            saga("inner", "a"),
            effect("a", false),
            activity("other"),
        ],
    ));
    let fa = run.lease("a");
    run.complete("a", fa).expect("a");
    assert_eq!(run.status("inner"), StepStatus::Succeeded);
    let fo = run.lease("other");
    run.fail_with("other", fo, "BOOM", false)
        .expect("other fails");
    assert!(!run.has_step("a~c"));
    assert_eq!(run.state.status(), RunStatus::Failed);
}

#[test]
fn an_inner_saga_that_fails_is_compensated_and_then_the_outer_one_compensates_the_rest() {
    let mut run = Harness::start(build(
        "outer",
        vec![
            saga("outer", "first"),
            effect("first", false).with_next(id("inner")),
            saga("inner", "a").with_next(id("last")),
            effect("a", false).with_next(id("b")),
            activity("b"),
            effect("last", false),
        ],
    ));
    let f = run.lease("first");
    run.complete("first", f).expect("first");
    let f = run.lease("a");
    run.complete("a", f).expect("a");
    let f = run.lease("b");
    run.fail_with("b", f, "BOOM", false).expect("b fails");

    // The inner saga compensates its own work first.
    assert_eq!(comp(&run, "inner").phase(), SagaPhase::Compensating);
    assert_eq!(comp(&run, "outer").phase(), SagaPhase::Forward);
    let f = run.lease("a~c");
    run.complete("a~c", f).expect("undo a");

    // Its failure is unhandled, so the outer saga compensates what it did; `a` is not undone twice.
    assert_eq!(comp(&run, "outer").phase(), SagaPhase::Compensating);
    assert_eq!(
        comp(&run, "outer")
            .entries()
            .keys()
            .map(NodeId::as_str)
            .collect::<Vec<_>>(),
        ["first"]
    );
    let f = run.lease("first~c");
    run.complete("first~c", f).expect("undo first");
    assert_eq!(run.state.status(), RunStatus::Compensated);
    run.assert_replays();
}

#[test]
fn nested_outcomes_are_not_hidden_by_the_outer_saga() {
    let mut run = Harness::start(build(
        "outer",
        vec![
            saga("outer", "inner"),
            saga("inner", "a"),
            with_effect(effect("a", false), short_comp(1)).with_next(id("b")),
            activity("b"),
        ],
    ));
    let f = run.lease("a");
    run.complete("a", f).expect("a");
    let f = run.lease("b");
    run.fail_with("b", f, "BOOM", false).expect("b fails");
    let f = run.lease("a~c");
    run.fail_with("a~c", f, "PROVIDER_DOWN", true)
        .expect("fails");
    run.operator_skip("a~c").expect("skip");
    // Inner: partial. Outer has nothing of its own to undo (a was dealt with), but the run says
    // what really happened.
    assert_eq!(comp(&run, "inner").outcome(), Some(SagaOutcome::Partial));
    assert_eq!(run.state.status(), RunStatus::Partial);
}

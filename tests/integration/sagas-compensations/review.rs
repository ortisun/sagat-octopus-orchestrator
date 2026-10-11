//! US04: a step that waits for a decision is decided by an operator, with a justification, and the
//! decision is audited (FR-003-005, FR-003-007).

use sagat_application::error::AppError;
use sagat_application::runs::ReviewAction;
use sagat_domain::execution::{FailureCode, ReviewOutcome, RunId, RunStatus, StepStatus};
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::engine::{
    Session, World, activity, document, effect_node, saga_node, with, world,
};
use crate::{settlement, worker};

/// `settlement` whose reservation is undone in one attempt, so the first failure of the
/// compensation sends the run to an operator.
pub fn one_shot(on_failure: Option<&str>) -> Value {
    let mut saga = saga_node("settlement", "reserve", Some("notify"));
    if let Some(policy) = on_failure {
        saga["onCompensationFailure"] = json!(policy);
    }
    document(
        "settlement",
        "settlement",
        vec![
            saga,
            with(
                effect_node("reserve", Some("submit"), false),
                "compensationRetry",
                json!({ "maxAttempts": 1, "initialDelayMs": 100, "maxDelayMs": 100,
                        "strategy": "fixed", "jitter": false }),
            ),
            effect_node("submit", None, true),
            activity("notify"),
        ],
    )
}

/// A run whose compensation of `reserve` failed and waits for an operator.
async fn stuck(w: &World, doc: Value) -> (Session, RunId) {
    let (workflow, _) = w.publish(doc).await;
    let s = worker(w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("submit fails");
    let undo = w.one_task(&s).await;
    w.fail_with(&s, &undo, "PROVIDER_DOWN", true)
        .await
        .expect("the provider is down");
    (s, run)
}

async fn audit_actions(w: &World, run: RunId) -> Vec<(String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT action, actor, reason FROM audit_records WHERE resource_type = 'run' \
         AND resource_id LIKE $1 AND action LIKE 'saga.%' ORDER BY created_at, id",
    )
    .bind(format!("{run}/%"))
    .fetch_all(&w.db.pool)
    .await
    .expect("audit")
}

#[tokio::test]
async fn a_compensation_that_ran_out_of_attempts_waits_for_an_operator_and_nothing_else_moves() {
    let w = world().await;
    let (s, run) = stuck(&w, one_shot(None)).await;
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        Some(StepStatus::Waiting)
    );
    assert!(
        w.poll(&s, 8).await.is_empty(),
        "no worker gets anything to do"
    );
    let waiting: (String, Option<String>) = sqlx::query_as(
        "SELECT waiting_kind, failure_code FROM steps WHERE run_id = $1 AND node = 'reserve~c'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("step");
    assert_eq!(
        waiting,
        ("OPERATOR".to_owned(), Some("PROVIDER_DOWN".to_owned()))
    );
    // The run is listed where an operator looks for it.
    let to_review: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM runs WHERE status = 'MANUAL_REVIEW' AND tenant_id = $1")
            .bind(w.admin.tenant_id().as_uuid())
            .fetch_all(&w.db.pool)
            .await
            .expect("runs");
    assert_eq!(to_review, vec![run.as_uuid()]);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn the_operator_retries_the_compensation_and_the_old_attempt_stays_in_the_journal() {
    let w = world().await;
    let (s, run) = stuck(&w, one_shot(None)).await;
    w.review(
        run,
        "reserve~c",
        ReviewAction::Retry,
        "the provider is back",
    )
    .await
    .expect("retry");
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
    let again = w.one_task(&s).await;
    assert_eq!(again.activity.as_str(), "undo.reserve");
    assert_eq!(
        again.fence.attempt, 2,
        "a new attempt of the same compensation"
    );
    w.complete(&s, &again, &json!({})).await.expect("undo");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);

    let names = w.journal(run).await;
    for kept in ["step.needs_review", "step.retry_requested"] {
        assert!(names.iter().any(|n| n == kept), "{kept} in {names:?}");
    }
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn skipping_a_compensation_ends_the_saga_partial_and_is_audited_with_its_reason() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    w.review(
        run,
        "reserve~c",
        ReviewAction::Skip,
        "released by hand in the provider console",
    )
    .await
    .expect("skip");
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        Some(StepStatus::Skipped)
    );
    assert_eq!(w.run_status(run).await, RunStatus::Partial);

    let audit = audit_actions(&w, run).await;
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0].0, "saga.step_skipped");
    assert_eq!(
        audit[0].2.as_deref(),
        Some("released by hand in the provider console")
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn declaring_a_compensation_done_outside_the_platform_ends_the_saga_compensated() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    w.review(
        run,
        "reserve~c",
        ReviewAction::Resolve(ReviewOutcome::Succeeded),
        "undone by hand",
    )
    .await
    .expect("resolve");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    let audit = audit_actions(&w, run).await;
    assert_eq!(
        audit.iter().map(|a| a.0.as_str()).collect::<Vec<_>>(),
        ["saga.step_resolved"]
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn with_the_fail_policy_a_failed_compensation_ends_the_run_without_waiting_for_anyone() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(Some("fail"))).await;
    assert_eq!(w.run_status(run).await, RunStatus::CompensationFailed);
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        Some(StepStatus::Failed)
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_operator_may_retry_but_only_an_administrator_may_skip_or_resolve() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    let operator = w.member("olivia", Role::Operator).await;

    for action in [
        ReviewAction::Skip,
        ReviewAction::Resolve(ReviewOutcome::Succeeded),
    ] {
        let refused = w
            .review_as(&operator, run, "reserve~c", action, "no right to")
            .await;
        assert!(
            matches!(refused, Err(AppError::Forbidden { .. })),
            "{refused:?}"
        );
    }
    assert_eq!(
        w.run_status(run).await,
        RunStatus::ManualReview,
        "nothing changed"
    );
    assert!(audit_actions(&w, run).await.is_empty());

    w.review_as(
        &operator,
        run,
        "reserve~c",
        ReviewAction::Retry,
        "provider recovered",
    )
    .await
    .expect("an operator may ask for another attempt");
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
    let audit = audit_actions(&w, run).await;
    assert_eq!(audit.len(), 1);
    assert_eq!(
        audit[0].1, "olivia",
        "the audit names the person, not a role"
    );
}

#[tokio::test]
async fn a_decision_without_a_justification_is_refused() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    for reason in ["", "   ", "line\nbreak"] {
        let refused = w
            .review(run, "reserve~c", ReviewAction::Retry, reason)
            .await;
        assert!(
            matches!(refused, Err(AppError::NotAllowed(_))),
            "{reason:?}: {refused:?}"
        );
    }
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
}

#[tokio::test]
async fn the_second_of_two_commands_on_the_same_step_is_refused_and_changes_nothing() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    w.review(run, "reserve~c", ReviewAction::Skip, "first decision")
        .await
        .expect("skip");
    let journal = w.journal(run).await;

    let second = w
        .review(
            run,
            "reserve~c",
            ReviewAction::Resolve(ReviewOutcome::Succeeded),
            "second decision",
        )
        .await;
    assert!(
        matches!(second, Err(AppError::StateConflict(_))),
        "{second:?}"
    );
    assert_eq!(w.journal(run).await, journal, "the loser wrote nothing");
    assert_eq!(w.run_status(run).await, RunStatus::Partial);
    assert_eq!(
        audit_actions(&w, run).await.len(),
        1,
        "and left no audit row of a decision not taken"
    );
}

#[tokio::test]
async fn two_operators_deciding_at_once_leave_exactly_one_decision() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    let (a, b) = tokio::join!(
        w.review(run, "reserve~c", ReviewAction::Skip, "operator A"),
        w.review(
            run,
            "reserve~c",
            ReviewAction::Resolve(ReviewOutcome::Succeeded),
            "operator B"
        ),
    );
    assert!(a.is_ok() ^ b.is_ok(), "exactly one wins: {a:?} / {b:?}");
    assert_eq!(audit_actions(&w, run).await.len(), 1);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn only_a_compensation_can_be_skipped_and_only_a_step_that_waits_can_be_decided() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    let not_a_compensation = w
        .review(run, "reserve", ReviewAction::Skip, "skip a forward step")
        .await;
    assert!(
        matches!(not_a_compensation, Err(AppError::NotAllowed(_))),
        "{not_a_compensation:?}"
    );
    // `reserve` completed long ago; nobody can retry what is not waiting.
    let not_waiting = w
        .review(run, "reserve", ReviewAction::Retry, "it already worked")
        .await;
    assert!(
        matches!(not_waiting, Err(AppError::StateConflict(_))),
        "{not_waiting:?}"
    );
    let unknown = w
        .review(run, "ghost", ReviewAction::Retry, "no such step")
        .await;
    assert!(matches!(unknown, Err(AppError::NotFound)), "{unknown:?}");
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
}

#[tokio::test]
async fn another_tenant_cannot_decide_on_a_run_it_cannot_see() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    let refused = w
        .review_as(
            &w.other_admin,
            run,
            "reserve~c",
            ReviewAction::Skip,
            "not mine",
        )
        .await;
    assert!(matches!(refused, Err(AppError::NotFound)), "{refused:?}");
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    assert!(audit_actions(&w, run).await.is_empty());
}

#[tokio::test]
async fn an_operator_settles_an_unknown_outcome_the_platform_could_not() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let _submit = w.one_task(&s).await;
    w.age_lease(run, "submit").await;
    w.scheduler.tick().await;
    let lookup = w.one_task(&s).await;
    w.answer_reconciliation(&s, &lookup, "unknown", None)
        .await
        .expect("unknown");
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);

    // The operator checks with the provider, finds the payment, and says it happened.
    w.review(
        run,
        "submit",
        ReviewAction::Resolve(ReviewOutcome::Succeeded),
        "payment p-9 confirmed with the provider",
    )
    .await
    .expect("resolve");
    assert_eq!(
        w.step_status(run, "submit").await,
        Some(StepStatus::Succeeded)
    );
    let notify = w.one_task(&s).await;
    assert_eq!(notify.activity.as_str(), "act.notify");
    w.complete(&s, &notify, &json!({})).await.expect("notify");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn an_operator_can_declare_that_the_effect_did_not_happen_and_the_saga_compensates() {
    let w = world().await;
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let _submit = w.one_task(&s).await;
    w.age_lease(run, "submit").await;
    w.scheduler.tick().await;
    let lookup = w.one_task(&s).await;
    w.answer_reconciliation(&s, &lookup, "unknown", None)
        .await
        .expect("unknown");

    w.review(
        run,
        "submit",
        ReviewAction::Resolve(ReviewOutcome::Failed(
            FailureCode::parse("NOT_PAID").expect("code"),
        )),
        "the provider has no such payment",
    )
    .await
    .expect("resolve");
    assert_eq!(w.run_status(run).await, RunStatus::Compensating);
    let undo = w.one_task(&s).await;
    assert_eq!(undo.activity.as_str(), "undo.reserve");
    w.complete(&s, &undo, &json!({})).await.expect("undo");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn the_same_idempotency_key_replays_a_decision_instead_of_repeating_it() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    let key = "review-fixed-key-0001";
    let node = sagat_domain::workflow::NodeId::parse_stored("reserve~c").expect("node");
    for _ in 0..2 {
        w.runs
            .review(
                &w.admin,
                run,
                &node,
                &ReviewAction::Skip,
                "same request twice",
                None,
                key,
                &crate::support::engine::ctx(),
            )
            .await
            .expect("skip, then its replay");
    }
    assert_eq!(audit_actions(&w, run).await.len(), 1);
    // The same key for a different decision is a mistake, not a replay.
    let reused = w
        .runs
        .review(
            &w.admin,
            run,
            &node,
            &ReviewAction::Resolve(ReviewOutcome::Succeeded),
            "other decision",
            None,
            key,
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(
        matches!(reused, Err(AppError::IdempotencyKeyReused)),
        "{reused:?}"
    );
}

/// `submit` is looked up and the provider cannot say: it waits for a person.
async fn unknown_outcome(w: &World) -> (Session, RunId) {
    let (workflow, _) = w.publish(settlement()).await;
    let s = worker(w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let _lost = w.one_task(&s).await;
    w.age_lease(run, "submit").await;
    w.scheduler.tick().await;
    let lookup = w.one_task(&s).await;
    w.answer_reconciliation(&s, &lookup, "unknown", None)
        .await
        .expect("unknown");
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    (s, run)
}

#[tokio::test]
async fn asking_again_for_an_effect_that_may_have_happened_is_an_administrators_decision() {
    let w = world().await;
    let (s, run) = unknown_outcome(&w).await;
    let operator = w.member("olivia", Role::Operator).await;

    let refused = w
        .review_as(
            &operator,
            run,
            "submit",
            ReviewAction::Retry,
            "try the payment again",
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::Forbidden { .. })),
        "{refused:?}"
    );
    assert_eq!(
        w.run_status(run).await,
        RunStatus::ManualReview,
        "nothing changed"
    );
    assert!(audit_actions(&w, run).await.is_empty());

    // An administrator, who may declare the payment done or not done, may also ask again.
    w.review(
        run,
        "submit",
        ReviewAction::Retry,
        "provider confirmed it did not go through",
    )
    .await
    .expect("an administrator may");
    let again = w.one_task(&s).await;
    assert_eq!(again.activity.as_str(), "act.submit");
    assert_eq!(again.fence.attempt, 2);
}

#[tokio::test]
async fn a_justification_is_counted_after_it_is_trimmed() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    for reason in ["x       ", "        ok        ", &"y".repeat(2001)] {
        let refused = w
            .review(run, "reserve~c", ReviewAction::Retry, reason)
            .await;
        assert!(
            matches!(refused, Err(AppError::NotAllowed(_))),
            "{reason:?}: {refused:?}"
        );
    }
    // Exactly the bounds are accepted, and what is audited is all of it.
    let longest = "z".repeat(2000);
    w.review(run, "reserve~c", ReviewAction::Skip, &longest)
        .await
        .expect("2000 characters");
    let audit = audit_actions(&w, run).await;
    assert_eq!(
        audit[0].2.as_deref(),
        Some(longest.as_str()),
        "nothing was cut"
    );
}

#[tokio::test]
async fn the_audit_of_a_declared_outcome_says_which_outcome_was_declared() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    w.review(
        run,
        "reserve~c",
        ReviewAction::Resolve(ReviewOutcome::Succeeded),
        "released by hand",
    )
    .await
    .expect("resolve");
    let audit = audit_actions(&w, run).await;
    assert_eq!(
        audit[0].2.as_deref(),
        Some("declared SUCCEEDED: released by hand")
    );

    let (_, other) = stuck(&w, {
        let mut doc = one_shot(None);
        doc["metadata"]["name"] = json!("settlement-b");
        doc
    })
    .await;
    let code = FailureCode::parse("NOT_RELEASED").expect("code");
    w.review(
        other,
        "reserve~c",
        ReviewAction::Resolve(ReviewOutcome::Failed(code)),
        "the provider refused it",
    )
    .await
    .expect("resolve");
    let audit = audit_actions(&w, other).await;
    assert_eq!(
        audit[0].2.as_deref(),
        Some("declared FAILED(NOT_RELEASED): the provider refused it")
    );
}

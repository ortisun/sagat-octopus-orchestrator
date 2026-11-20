//! US04: a step that waits for a decision is decided by an operator, with a justification, and the
//! decision is audited (FR-003-005, FR-003-007).

use sagat_application::error::AppError;
use std::time::{Duration, Instant};

use sagat_application::identity::PageRequest;
use sagat_application::runs::{ReviewAction, RunFilter, RunOperation, RunStore};
use sagat_domain::execution::{FailureCode, ReviewOutcome, RunId, RunStatus, StepStatus};
use sagat_domain::identity::Role;
use sagat_domain::quota::QuotaResource;
use sagat_infra_postgres::PgRunStore;
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

#[derive(Debug, PartialEq, Eq)]
struct ReviewEffects {
    run_version: i64,
    events: i64,
    outbox: i64,
    audits: i64,
    idempotency: i64,
    step_status: String,
    step_attempt: i32,
    failure_code: Option<String>,
    waiting_kind: Option<String>,
}

/// Durable effects a refused review command must leave exactly unchanged.
async fn review_effects(w: &World, run: RunId, step: &str, key: &str) -> ReviewEffects {
    let row: (i64, i64, i64, i64, i64, String, i32, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT r.run_version, \
             (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id), \
             (SELECT count(*) FROM event_outbox o WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id), \
             (SELECT count(*) FROM audit_records a WHERE a.tenant_id=r.tenant_id \
                AND a.resource_type='run' AND a.resource_id LIKE r.id::text || '/%' \
                AND a.action LIKE 'saga.%'), \
             (SELECT count(*) FROM command_dedupe d WHERE d.tenant_id=r.tenant_id \
                AND d.scope='run.review:' || r.id::text AND d.idempotency_key=$3), \
             s.status,s.attempt,s.failure_code,s.waiting_kind \
             FROM runs r JOIN steps s ON s.tenant_id=r.tenant_id AND s.run_id=r.id \
             WHERE r.tenant_id=$1 AND r.id=$2 AND s.node=$4",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .bind(key)
        .bind(step)
        .fetch_one(&w.db.pool)
        .await
        .expect("review effects");
    ReviewEffects {
        run_version: row.0,
        events: row.1,
        outbox: row.2,
        audits: row.3,
        idempotency: row.4,
        step_status: row.5,
        step_attempt: row.6,
        failure_code: row.7,
        waiting_kind: row.8,
    }
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
async fn unknown_effect_retry_requires_resolution_for_compensation_even_for_admin() {
    let w = world().await;
    let (workflow, _) = w.publish(one_shot(None)).await;
    let s = worker(&w).await;
    let run = w.start(workflow, &json!({})).await;
    let reserve = w.one_task(&s).await;
    w.complete(&s, &reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task(&s).await;
    w.fail(&s, &submit, false).await.expect("submit fails");
    let undo = w.one_task(&s).await;
    w.fail_with(&s, &undo, "OUTCOME_UNKNOWN", false)
        .await
        .expect("compensation outcome is unknown");
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);

    let operator = w.member("compensation-operator", Role::Operator).await;
    let node = sagat_domain::workflow::NodeId::parse_stored("reserve~c").expect("node");
    let operator_key = "review-unknown-compensation-operator";
    let before_operator = review_effects(&w, run, "reserve~c", operator_key).await;
    let operator_refused = w
        .runs
        .review(
            &operator,
            run,
            &node,
            &ReviewAction::Retry,
            "operator cannot repeat uncertain undo",
            None,
            operator_key,
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(
        matches!(operator_refused, Err(AppError::OutcomeUnresolved)),
        "run:operate reaches the source-aware state refusal: {operator_refused:?}"
    );
    assert_eq!(
        review_effects(&w, run, "reserve~c", operator_key).await,
        before_operator,
        "operator refusal writes no state, outbox, success audit, or idempotency record"
    );

    let key = "review-unknown-compensation-admin";
    let before = review_effects(&w, run, "reserve~c", key).await;
    let refused = w
        .runs
        .review(
            &w.admin,
            run,
            &node,
            &ReviewAction::Retry,
            "repeat the uncertain undo",
            None,
            key,
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::OutcomeUnresolved)),
        "{refused:?}"
    );
    assert_eq!(
        review_effects(&w, run, "reserve~c", key).await,
        before,
        "refusal writes no state, outbox, success audit, or idempotency record"
    );
    assert!(w.poll(&s, 8).await.is_empty(), "no repeat was scheduled");

    w.review(
        run,
        "reserve~c",
        ReviewAction::Resolve(ReviewOutcome::Succeeded),
        "operator confirmed the undo completed",
    )
    .await
    .expect("outcome-bearing resolution remains available");
    assert_eq!(w.run_status(run).await, RunStatus::Compensated);
    let audit = audit_actions(&w, run).await;
    assert_eq!(audit.len(), 1, "the refusal did not add a success audit");
    assert_eq!(audit[0].0, "saga.step_resolved");
    assert_eq!(audit[0].1, w.admin.subject().as_str());
    assert_eq!(
        audit[0].2.as_deref(),
        Some("declared SUCCEEDED: operator confirmed the undo completed")
    );
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
async fn held_run_lock_makes_two_decisions_and_cancel_leave_one_durable_human_outcome() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    let tenant = w.admin.tenant_id().as_uuid();
    let mut guard = w.db.pool.begin().await.expect("run row guard");
    let guard_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *guard)
        .await
        .expect("guard backend");
    sqlx::query("SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(tenant)
        .bind(run.as_uuid())
        .fetch_one(&mut *guard)
        .await
        .expect("hold run row");

    let node = sagat_domain::workflow::NodeId::parse_stored("reserve~c").expect("node");
    let ctx_a = crate::support::engine::ctx();
    let ctx_b = crate::support::engine::ctx();
    let ctx_cancel = crate::support::engine::ctx();
    let mut race = Box::pin(async {
        tokio::join!(
            w.runs.review(
                &w.admin,
                run,
                &node,
                &ReviewAction::Skip,
                "operator A chose partial",
                None,
                "held-review-operator-a",
                &ctx_a,
            ),
            w.runs.review(
                &w.admin,
                run,
                &node,
                &ReviewAction::Resolve(ReviewOutcome::Succeeded),
                "operator B confirmed undo",
                None,
                "held-review-operator-b",
                &ctx_b,
            ),
            w.runs.operate(
                &w.admin,
                run,
                RunOperation::Cancel,
                "cancel raced with review",
                None,
                "held-review-cancel",
                &ctx_cancel,
            ),
        )
    });
    // Release the row only once all three commands wait on it (directly behind the guard, or
    // behind each other in its lock queue), so they really contend instead of running in turn.
    let blocked = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity waiter \
                 WHERE waiter.datname=current_database() AND waiter.pid<>$1 \
                   AND cardinality(pg_blocking_pids(waiter.pid))>0",
            )
            .bind(guard_pid)
            .fetch_one(&w.db.pool)
            .await
            .expect("observe blocked commands");
            if waiting >= 3 {
                break;
            }
            tokio::task::yield_now().await;
        }
    });
    tokio::select! {
        result = &mut race => panic!("a command bypassed the held run row: {result:?}"),
        result = blocked => result.expect("all three commands reach the held run row"),
    }
    guard.commit().await.expect("release run row");

    let (a, b, cancel) = race.await;
    assert!(a.is_ok() ^ b.is_ok(), "exactly one wins: {a:?} / {b:?}");
    assert!(
        cancel.is_ok() || matches!(cancel, Err(AppError::StateConflict(_))),
        "cancel either preserves the review or observes its terminal decision: {cancel:?}"
    );
    let audit = audit_actions(&w, run).await;
    assert_eq!(audit.len(), 1, "only the winning human decision is audited");
    assert_eq!(audit[0].1, w.admin.subject().as_str());
    assert!(
        matches!(
            w.step_status(run, "reserve~c").await,
            Some(StepStatus::Succeeded | StepStatus::Skipped)
        ),
        "cancel did not erase or overwrite the winning decision"
    );

    // A cold store reconstructs the same accepted outcome from PostgreSQL after the services that
    // raced are no longer relevant; replay and the SQL projection must agree.
    let restarted = PgRunStore::new(w.execution_pool.clone(), w.cache.clone());
    let restarted_record = restarted
        .get(w.admin.tenant_id(), run)
        .await
        .expect("cold read")
        .expect("run survives restart");
    assert_eq!(restarted_record.status, w.run_status(run).await);
    restarted
        .verify_run(w.admin.tenant_id(), run)
        .await
        .expect("cold replay agrees with the projection");
}

/// The deterministic order the race above cannot force: cancellation commits first, then both
/// decisions. Cancellation must keep the compensation review, so a decision still wins.
#[tokio::test]
async fn a_cancel_committed_before_both_decisions_keeps_the_review_for_them() {
    let w = world().await;
    let (_, run) = stuck(&w, one_shot(None)).await;
    let node = sagat_domain::workflow::NodeId::parse_stored("reserve~c").expect("node");
    let cancel = w
        .runs
        .operate(
            &w.admin,
            run,
            RunOperation::Cancel,
            "cancel before anyone decides",
            None,
            "cancel-first",
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(cancel.is_ok(), "{cancel:?}");
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        Some(StepStatus::Waiting),
        "the cancellation kept the review"
    );
    let a = w
        .runs
        .review(
            &w.admin,
            run,
            &node,
            &ReviewAction::Skip,
            "operator A chose partial",
            None,
            "cancel-first-a",
            &crate::support::engine::ctx(),
        )
        .await;
    let b = w
        .runs
        .review(
            &w.admin,
            run,
            &node,
            &ReviewAction::Resolve(ReviewOutcome::Succeeded),
            "operator B confirmed undo",
            None,
            "cancel-first-b",
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(a.is_ok(), "the first decision after the cancel wins: {a:?}");
    assert!(
        matches!(b, Err(AppError::StateConflict(_))),
        "the second finds it decided: {b:?}"
    );
    assert_eq!(
        w.step_status(run, "reserve~c").await,
        Some(StepStatus::Skipped)
    );
    assert_eq!(audit_actions(&w, run).await.len(), 1);
    w.store
        .verify_run(w.admin.tenant_id(), run)
        .await
        .expect("replay agrees with the projection");
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
async fn unknown_effect_retry_requires_resolution_for_original_even_for_admin() {
    let w = world().await;
    let (s, run) = unknown_outcome(&w).await;
    let operator = w.member("olivia", Role::Operator).await;
    let viewer = w.member("victor", Role::Viewer).await;
    let node = sagat_domain::workflow::NodeId::parse_stored("submit").expect("node");

    let viewer_key = "review-unknown-original-viewer";
    let before_viewer = review_effects(&w, run, "submit", viewer_key).await;
    let viewer_refused = w
        .runs
        .review(
            &viewer,
            run,
            &node,
            &ReviewAction::Retry,
            "try the payment again",
            None,
            viewer_key,
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(
        matches!(viewer_refused, Err(AppError::Forbidden { .. })),
        "missing run:operate stays an authorization denial: {viewer_refused:?}"
    );
    assert_eq!(
        review_effects(&w, run, "submit", viewer_key).await,
        before_viewer,
        "authorization denial has no execution success effects"
    );

    let operator_key = "review-unknown-original-operator";
    let before_operator = review_effects(&w, run, "submit", operator_key).await;
    let operator_refused = w
        .runs
        .review(
            &operator,
            run,
            &node,
            &ReviewAction::Retry,
            "operator cannot repeat uncertain payment",
            None,
            operator_key,
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(
        matches!(operator_refused, Err(AppError::OutcomeUnresolved)),
        "run:operate reaches the source-aware state refusal: {operator_refused:?}"
    );
    assert_eq!(
        review_effects(&w, run, "submit", operator_key).await,
        before_operator,
        "operator refusal writes no state, outbox, success audit, or idempotency record"
    );

    let key = "review-unknown-original-admin";
    let before = review_effects(&w, run, "submit", key).await;
    let refused = w
        .runs
        .review(
            &w.admin,
            run,
            &node,
            &ReviewAction::Retry,
            "provider answer is still uncertain",
            None,
            key,
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::OutcomeUnresolved)),
        "SagaOverride does not establish the original outcome: {refused:?}"
    );
    assert_eq!(
        review_effects(&w, run, "submit", key).await,
        before,
        "refusal writes no state, outbox, success audit, or idempotency record"
    );
    assert!(w.poll(&s, 8).await.is_empty(), "no repeat was scheduled");

    w.review(
        run,
        "submit",
        ReviewAction::Resolve(ReviewOutcome::Succeeded),
        "provider confirmed payment p-9 settled",
    )
    .await
    .expect("the administrator records the observed outcome instead");
    let notify = w.one_task(&s).await;
    assert_eq!(notify.activity.as_str(), "act.notify");
    w.complete(&s, &notify, &json!({})).await.expect("notify");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    let audit = audit_actions(&w, run).await;
    assert_eq!(audit.len(), 1, "the refusal did not add a success audit");
    assert_eq!(audit[0].0, "saga.step_resolved");
    assert_eq!(audit[0].1, w.admin.subject().as_str());
    assert_eq!(
        audit[0].2.as_deref(),
        Some("declared SUCCEEDED: provider confirmed payment p-9 settled")
    );
    w.assert_consistent(run).await;
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ReviewStatementWork {
    calls: i64,
    rows: i64,
    buffers: i64,
}

impl std::ops::Sub for ReviewStatementWork {
    type Output = Self;

    fn sub(self, before: Self) -> Self {
        Self {
            calls: self.calls - before.calls,
            rows: self.rows - before.rows,
            buffers: self.buffers - before.buffers,
        }
    }
}

async fn review_statement_work(w: &World) -> ReviewStatementWork {
    let (calls, rows, buffers): (i64, i64, i64) = sqlx::query_as(
        "SELECT coalesce(sum(calls),0)::bigint, coalesce(sum(rows),0)::bigint, \
                coalesce(sum(shared_blks_hit+shared_blks_read),0)::bigint \
         FROM pg_stat_statements \
         WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) \
           AND query NOT LIKE '%pg_stat_statements%'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("statement accounting for this test database");
    ReviewStatementWork {
        calls,
        rows,
        buffers,
    }
}

fn statement_json(work: ReviewStatementWork) -> Value {
    json!({
        "calls": work.calls,
        "rows": work.rows,
        "buffers": work.buffers,
    })
}

async fn review_relation_bytes(w: &World) -> (i64, i64) {
    let relations = [
        "runs",
        "run_active_counts",
        "steps",
        "run_events",
        "event_outbox",
        "command_dedupe",
    ];
    sqlx::query_as(
        "SELECT coalesce(sum(pg_table_size(c.oid)),0)::bigint, \
                coalesce(sum(pg_indexes_size(c.oid)),0)::bigint \
         FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
         WHERE n.nspname=current_schema() AND c.relname=ANY($1)",
    )
    .bind(relations.as_slice())
    .fetch_one(&w.db.pool)
    .await
    .expect("review table and index bytes")
}

/// Counts only this benchmark's ordinary forward reviews plus the obligations an idle pass must
/// not create. Hints and journal rows are compared before/after the observation rather than assumed
/// absent: each run legitimately wrote both while it was started and leased.
async fn review_population(w: &World) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM runs r JOIN steps s \
              ON s.tenant_id=r.tenant_id AND s.run_id=r.id \
            WHERE r.tenant_id=$1 AND r.namespace_id=$2 AND r.status='MANUAL_REVIEW' \
              AND s.node='charge' AND s.kind='FORWARD' AND s.status='WAITING' \
              AND s.waiting_kind='OPERATOR' AND s.failure_code='OUTCOME_UNKNOWN' \
              AND s.lease_token_hash IS NULL AND s.lease_expires_at IS NULL), \
           (SELECT count(*) FROM steps \
            WHERE tenant_id=$1 AND namespace_id=$2 \
              AND (status IN ('LEASED','RUNNING') OR lease_token_hash IS NOT NULL)), \
           ((SELECT count(*) FROM steps \
             WHERE tenant_id=$1 AND namespace_id=$2 AND status='RETRY_WAIT') + \
            (SELECT count(*) FROM run_events e JOIN runs r \
               ON r.tenant_id=e.tenant_id AND r.id=e.run_id \
             WHERE r.tenant_id=$1 AND r.namespace_id=$2 \
               AND e.event_type IN ('step.retry_scheduled','step.retry_due','step.retry_requested'))), \
           (SELECT count(*) FROM event_outbox o \
            WHERE o.tenant_id=$1 AND o.event_type='task.ready'), \
           (SELECT count(*) FROM run_events e JOIN runs r \
              ON r.tenant_id=e.tenant_id AND r.id=e.run_id \
            WHERE r.tenant_id=$1 AND r.namespace_id=$2)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("ordinary unknown review population")
}

#[tokio::test]
#[ignore = "measurement: run explicitly with --ignored --nocapture"]
async fn ordinary_unknown_reviews_remain_idle_at_one_one_hundred_and_one_thousand() {
    const SCALES: [usize; 3] = [1, 100, 1_000];
    const POLL_BATCH: u32 = 100;
    const SETUP_LIMIT: Duration = Duration::from_secs(600);
    const OBSERVATION_LIMIT: Duration = Duration::from_secs(60);

    assert_eq!(QuotaResource::ActiveRuns.default_limit(), 10_000);
    assert_eq!(QuotaResource::TenantActiveRuns.default_limit(), 50_000);
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    let postgres_version: String = sqlx::query_scalar("SELECT current_setting('server_version')")
        .fetch_one(&w.db.pool)
        .await
        .expect("PostgreSQL version");
    let (workflow, _) = w
        .publish(document(
            "ordinary-unknown-review-scale",
            "charge",
            vec![effect_node("charge", None, false)],
        ))
        .await;
    let session = w.session(&["act.charge"]).await;
    let mut loaded = 0_usize;
    let mut measurements = Vec::new();

    for scale in SCALES {
        let prior_scale = loaded;
        let setup_sql_before = review_statement_work(&w).await;
        let setup_started = Instant::now();
        tokio::time::timeout(SETUP_LIMIT, async {
            while loaded < scale {
                let batch = (scale - loaded).min(POLL_BATCH as usize);
                for _ in 0..batch {
                    w.start(workflow, &json!({})).await;
                }
                let mut reported = 0_usize;
                while reported < batch {
                    let tasks = w.poll(&session, POLL_BATCH).await;
                    assert!(
                        !tasks.is_empty(),
                        "the normal worker poll must make progress"
                    );
                    for task in tasks {
                        let ack = w
                            .fail_with(&session, &task, "OUTCOME_UNKNOWN", false)
                            .await
                            .expect("the engine records an ordinary unknown outcome");
                        assert_eq!(ack.step_status, StepStatus::Waiting);
                        reported += 1;
                    }
                }
                loaded += batch;
            }
        })
        .await
        .expect("review setup exceeded its 600-second resource guard");
        let setup_elapsed = setup_started.elapsed();
        let setup_sql = review_statement_work(&w).await - setup_sql_before;

        let population = review_population(&w).await;
        assert_eq!(
            population.0, scale as i64,
            "all unknown effects remain reviews"
        );
        assert_eq!(population.1, 0, "a review holds no worker lease");
        assert_eq!(
            population.2, 0,
            "unknown effects acquired no retry obligation"
        );
        let (table_bytes, index_bytes) = review_relation_bytes(&w).await;
        assert!(
            table_bytes + index_bytes <= 1_073_741_824,
            "review relations exceeded the 1 GiB exam guard"
        );

        let list_sql_before = review_statement_work(&w).await;
        let list_started = Instant::now();
        let page = tokio::time::timeout(
            OBSERVATION_LIMIT,
            w.runs.list(
                &w.admin,
                "payments",
                &RunFilter {
                    workflow: Some(workflow),
                    status: Some(RunStatus::ManualReview),
                    origin: None,
                },
                &PageRequest {
                    limit: Some(100),
                    cursor: None,
                },
            ),
        )
        .await
        .expect("review listing exceeded its 60-second observation guard")
        .expect("list the first review page");
        let list_elapsed = list_started.elapsed();
        let list_sql = review_statement_work(&w).await - list_sql_before;
        assert_eq!(page.items.len(), scale.min(100));
        assert!(
            page.items
                .iter()
                .all(|run| run.status == RunStatus::ManualReview)
        );

        // Explain the exact statement observed above, including its lateral segment lookup.
        // This fixture contains synthetic identifiers only; no production data is sampled.
        let mut list_statements: Vec<String> = sqlx::query_scalar(
            "SELECT query FROM pg_stat_statements \
             WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) \
               AND query LIKE 'SELECT id, tenant_id, namespace_id, workflow_id,%' \
               AND query LIKE '%FROM runs LEFT JOIN LATERAL%' \
               AND query LIKE '%ORDER BY runs.id DESC LIMIT $6%'",
        )
        .fetch_all(&w.db.pool)
        .await
        .expect("the actual review listing was observed");
        assert_eq!(
            list_statements.len(),
            1,
            "select only the production list statement"
        );
        let list_statement = list_statements.pop().expect("one review list statement");
        let explain = format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {list_statement}");
        let list_plan: sqlx::types::Json<Value> = tokio::time::timeout(
            OBSERVATION_LIMIT,
            sqlx::query_scalar(sqlx::AssertSqlSafe(explain))
                .bind(w.admin.tenant_id().as_uuid())
                .bind(w.namespace.id().as_uuid())
                .bind(Option::<uuid::Uuid>::None)
                .bind(workflow.as_uuid())
                .bind("MANUAL_REVIEW")
                .bind(101_i64)
                .bind(Option::<String>::None)
                .bind(true) // pg_stat_statements normalizes the lateral join's ON true.
                .fetch_one(&w.execution_pool),
        )
        .await
        .expect("review EXPLAIN exceeded its 60-second observation guard")
        .expect("explain the measured review listing");

        let before_idle = review_population(&w).await;
        let idle_sql_before = review_statement_work(&w).await;
        let idle_started = Instant::now();
        let (scheduler_report, polled) = tokio::time::timeout(OBSERVATION_LIMIT, async {
            let report = w.scheduler.tick().await;
            let polled = w.poll(&session, POLL_BATCH).await;
            (report, polled)
        })
        .await
        .expect("idle observation exceeded its 60-second resource guard");
        let idle_elapsed = idle_started.elapsed();
        let idle_sql = review_statement_work(&w).await - idle_sql_before;
        let after_idle = review_population(&w).await;
        assert_eq!(scheduler_report, sagat_scheduler::Report::default());
        assert!(polled.is_empty(), "ordinary reviews are not worker work");
        assert_eq!(
            after_idle, before_idle,
            "idle observation changed durable state"
        );
        assert_eq!(
            after_idle.0, scale as i64,
            "idle passes retain every review"
        );
        assert_eq!(after_idle.1, 0, "idle passes create no leases");
        assert_eq!(after_idle.2, 0, "idle passes create no retries");

        measurements.push(json!({
            "scale": scale,
            "setup_increment": {
                "reviews_added": scale - prior_scale,
                "elapsed_ms": setup_elapsed.as_secs_f64() * 1_000.0,
                "statement_delta": statement_json(setup_sql),
            },
            "storage": {
                "table_bytes": table_bytes,
                "index_bytes": index_bytes,
            },
            "list_first_100": {
                "returned": page.items.len(),
                "query_plan": list_plan.0,
                "elapsed_ms": list_elapsed.as_secs_f64() * 1_000.0,
                "statement_delta": statement_json(list_sql),
            },
            "idle_tick_and_poll": {
                "elapsed_ms": idle_elapsed.as_secs_f64() * 1_000.0,
                "statement_delta": statement_json(idle_sql),
                "scheduler_report": {
                    "did_work": scheduler_report.did_work(),
                    "collections_advanced": scheduler_report.collections_advanced,
                    "loops_advanced": scheduler_report.loops_advanced,
                    "continuations_advanced": scheduler_report.continuations_advanced,
                    "continuations_waiting": scheduler_report.continuations_waiting,
                    "approvals_opened": scheduler_report.approvals_opened,
                    "approvals_expired": scheduler_report.approvals_expired,
                    "approval_routes_released": scheduler_report.approval_routes_released,
                    "stopped_runs_settled": scheduler_report.stopped_runs_settled,
                    "timers_fired": scheduler_report.timers_fired,
                    "waits_timed_out": scheduler_report.waits_timed_out,
                    "skipped": scheduler_report.skipped,
                    "failed": scheduler_report.failed,
                    "fenced": scheduler_report.fenced,
                    "retries_released": scheduler_report.retries_released,
                    "leases_expired": scheduler_report.leases_expired,
                    "deadlines_enforced": scheduler_report.deadlines_enforced,
                    "hints_renewed": scheduler_report.hints_renewed,
                    "children_started": scheduler_report.children_started,
                    "children_finished": scheduler_report.children_finished,
                    "children_cancelled": scheduler_report.children_cancelled,
                    "quota_stops": scheduler_report.quota_stops,
                },
                "polled_tasks": polled.len(),
                "review_rows": after_idle.0,
                "leases": after_idle.1,
                "retry_obligations": after_idle.2,
                "task_ready_hints": after_idle.3,
                "journal_events": after_idle.4,
            },
        }));
    }

    println!(
        "{}",
        serde_json::to_string(&json!({
            "t037029_unknown_review_scale": {
                "environment": {
                    "postgres_version": postgres_version,
                    "scales": SCALES,
                },
                "resource_guards": {
                    "max_reviews": SCALES[SCALES.len() - 1],
                    "poll_batch": POLL_BATCH,
                    "setup_seconds_per_scale": SETUP_LIMIT.as_secs(),
                    "observation_seconds": OBSERVATION_LIMIT.as_secs(),
                    "namespace_active_run_default": QuotaResource::ActiveRuns.default_limit(),
                    "tenant_active_run_default": QuotaResource::TenantActiveRuns.default_limit(),
                },
                "measurements": measurements,
                "interpretation": "observations on this fixture only; no latency or capacity SLO is asserted",
            }
        }))
        .expect("measurement JSON")
    );
}

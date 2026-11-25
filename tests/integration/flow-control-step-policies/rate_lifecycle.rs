//! US07 lifecycle of a rate-limited run (task T022-075; FR-022-044, FR-022-046; SC-022-022): one test per item of the
//! task that no earlier test proves. The others are proved where they were built:
//!
//! - the cancellation of a waiter: `fault_flow_control_step_policies::rate_limits::a_grant_and_a_cancellation_of_a_buffered_start_are_decided_by_the_first_commit`
//!   (a buffered start cancelled journals `rate_limit.dequeued` and leaves no row) and
//!   `readiness::a_cancelled_run_takes_its_queued_step_out_of_every_queue` (a step waiter);
//! - a lowered policy ceiling: `fault_flow_control_step_policies::rate_limits::a_lowered_policy_ceiling_slows_later_grants_and_revokes_none`;
//! - a granted step cancelled before its lease: `returns::a_cancellation_gives_back_a_granted_unleased_step`
//!   (`rate_limit.returned` with `concurrency.released`);
//! - node-level `concurrency` with node-level `rateLimit`: `queue::a_dual_gated_head_is_not_starved_by_a_younger_waiter`
//!   and `queue::a_dual_gated_waiter_times_out_on_the_gate_that_refuses_it_rate_first`;
//! - a body `child` node starting a rate-limited workflow: a `child` node inside a collection body is refused at
//!   publication on this tree (`child_starts::a_child_node_inside_a_collection_body_is_refused_at_publication`); a root
//!   one is `rate_creators::a_child_run_of_a_rate_limited_definition_is_buffered_and_granted_from_the_queue`.
//!
//! Windows of an hour keep every start queued whatever the runner's speed; where a window must age, the holder's counted
//! instants are moved ahead while the starts behind it are admitted (decision D022-072) and the window then ages on the
//! database clock, a bounded wait that decides nothing.

use sagat_application::clock::SystemClock;
use sagat_application::definitions::DefinitionService;
use sagat_application::runs::RunOperation;
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{RunId, RunStatus};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::rate_gate::move_grant_instants;
use crate::run_admission::count;
use crate::support::engine::{World, activity, ctx, document};
use crate::support::flow_control::{gated_world, publish, switches, until_past};

const SECOND_US: i64 = 1_000_000;
/// How far a holder's counted instants are moved ahead while the starts behind it are admitted (decision D022-072).
const MARGIN_US: i64 = 60 * SECOND_US;

/// A one-step workflow `name` whose start takes a slot of `input.accountId`, `limit` per hour.
fn hourly(name: &str, limit: u32) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": limit, "per": "hour"});
    doc
}

/// The run's own grant and waiter rows: (grants, waiters).
async fn slot_rows(w: &World, run: RunId) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN'), \
                (SELECT count(*) FROM rate_limit_waiters WHERE run_id = $1 AND limit_scope = 'RUN')",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows")
}

/// Makes every rate key with waiters due now, as the owner (a hint only says when a pass may look).
async fn keys_due(w: &World) {
    sqlx::query("UPDATE rate_limit_windows SET next_eligible_at = now() WHERE waiters > 0")
        .execute(&w.db.pool)
        .await
        .expect("due");
}

/// FR-022-044: a buffered start whose run deadline passes is dequeued (`rate_limit.dequeued`) and the run ends at its
/// deadline without a slot; the holder keeps its slot.
#[tokio::test]
async fn a_run_deadline_dequeues_a_buffered_start() {
    let w = gated_world().await;
    let workflow = publish(&w, hourly("rate-lifecycle-deadline", 1)).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(slot_rows(&w, waiter).await, (0, 1));
    w.age_run_deadline(waiter).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.deadlines_enforced, 1, "{report:?}");
    assert!(w.run_status(waiter).await.is_terminal());
    assert_eq!(count(&w, waiter, "rate_limit.dequeued").await, 1);
    assert_eq!(count(&w, waiter, "rate_limit.granted").await, 0);
    assert_eq!(slot_rows(&w, waiter).await, (0, 0));
    assert_eq!(slot_rows(&w, holder).await, (1, 0));
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// FR-022-046, ADR-0089 decision 4: a paused waiter is skipped by the pass and does not hold back the waiter behind it;
/// after its resume a pass grants it. Behind a holder of `limit: 1, per: second`, `p` and `q` are buffered (the holder's
/// counted instants are moved ahead while they are admitted, decision D022-072, so they queue whatever the runner's
/// speed) and `p` is paused; once the holder's slot has aged out on the database clock, the pass grants `q`, not `p`;
/// once `q`'s has too, a pass still grants nothing while `p` is paused, and the pass after `p`'s resume grants it. The
/// windows age on the database clock (a bounded wait that decides nothing), so the journals the consistency check reads
/// never hold two grants in one window.
#[tokio::test]
async fn a_paused_waiter_is_skipped_and_its_resume_lets_the_pass_grant_it() {
    let w = gated_world().await;
    let mut doc = document("rate-lifecycle-pause", "g", vec![activity("g")]);
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": 1, "per": "second"});
    let workflow = publish(&w, doc).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    move_grant_instants(&w, holder, MARGIN_US).await;
    let p = w.start(workflow, &input).await;
    let q = w.start(workflow, &input).await;
    for run in [p, q] {
        assert_eq!(slot_rows(&w, run).await, (0, 1), "buffered");
    }
    move_grant_instants(&w, holder, -MARGIN_US).await;
    w.operate_run(p, RunOperation::Pause).await;
    until_past(&w.db.pool, grant_us(&w, holder).await + SECOND_US).await;
    keys_due(&w).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(
        slot_rows(&w, q).await,
        (1, 0),
        "the waiter behind is granted"
    );
    assert_eq!(slot_rows(&w, p).await, (0, 1), "the paused one is skipped");
    assert_eq!(count(&w, p, "rate_limit.granted").await, 0);

    until_past(&w.db.pool, grant_us(&w, q).await + SECOND_US).await;
    keys_due(&w).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 0, "still paused: {report:?}");
    assert_eq!(slot_rows(&w, p).await, (0, 1));
    w.operate_run(p, RunOperation::Resume).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 1,
        "the resume made the key due: {report:?}"
    );
    assert_eq!(slot_rows(&w, p).await, (1, 0));
    for run in [holder, p, q] {
        w.assert_consistent(run).await;
    }
}

/// The grant instant of `run`'s start slot, in microseconds.
async fn grant_us(w: &World, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT (extract(epoch FROM granted_at) * 1000000)::bigint FROM rate_limit_grants \
          WHERE run_id = $1 AND limit_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the run's grant")
}

/// FR-022-044 (US07 edge cases: a new version's lower limit applies to the next decision): version 1 declares `limit: 3,
/// per: hour` and two of its starts hold slots; version 2 lowers it to 2. Nothing is revoked, and a start of version 2
/// counts the slots version 1 took on the shared key and is buffered at its own lower limit (version 1's three would
/// have had room for it).
#[tokio::test]
async fn a_lowered_declaration_revokes_nothing_and_applies_to_the_next_start() {
    let w = gated_world().await;
    let workflow = publish(&w, hourly("rate-lifecycle-lowered", 3)).await;
    let input = json!({"accountId": "acct-1"});
    let first = w.start(workflow, &input).await;
    let second = w.start(workflow, &input).await;
    for run in [first, second] {
        assert_eq!(slot_rows(&w, run).await, (1, 0));
    }
    publish_version(&w, workflow, 2, hourly("rate-lifecycle-lowered", 2)).await;
    for run in [first, second] {
        assert_eq!(slot_rows(&w, run).await, (1, 0), "nothing is revoked");
        assert_eq!(w.run_status(run).await, RunStatus::Running);
    }
    let third = w.start(workflow, &input).await;
    assert_eq!(
        slot_rows(&w, third).await,
        (0, 1),
        "the lowered declaration decides the next start"
    );
    let limit: i32 = sqlx::query_scalar(
        "SELECT declared_limit FROM rate_limit_waiters WHERE run_id = $1 AND limit_scope = 'RUN'",
    )
    .bind(third.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the waiter");
    assert_eq!(limit, 2, "filed under the version's own limit");
    for run in [first, second, third] {
        w.assert_consistent(run).await;
    }
}

/// Publishes `doc` as the version `revision` of `workflow`, with both switches on.
async fn publish_version(w: &World, workflow: WorkflowId, revision: u64, doc: Value) {
    let defs = DefinitionService::new(w.store.clone(), SystemClock, JsonIrCodec)
        .with_flow_control(switches(true));
    defs.update_draft(
        &w.admin,
        workflow,
        revision - 1,
        &serde_json::to_vec(&doc).expect("json"),
        &ctx(),
    )
    .await
    .expect("draft");
    defs.publish(
        &w.admin,
        workflow,
        revision,
        None,
        &format!("publish-{}", Uuid::now_v7()),
        &ctx(),
    )
    .await
    .expect("published");
}

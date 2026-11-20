//! The rate gate against PostgreSQL (T022-072, T022-074, T022-077; transaction T19, transaction T23 and transaction T25 of the plan; FR-022-040..044,
//! FR-022-047; D022-005, D022-006, D022-029, D022-064): a run's start that declares `rateLimit` is granted a slot or
//! buffered in its admission transaction through the one decision of both levels (G022-012), a run's slot counts from its
//! grant, a step's from the lease the scheduler copies from the journal, and the purge keeps a grant for its window's
//! longest period and one minute more. Every instant is the database clock's; a window that must pass is waited for on
//! that clock with a bound (`until_past`), never by a sleep.

use sagat_application::error::AppError;
use sagat_application::flow_control::FlowControlRefusal;
use sagat_domain::execution::{RunId, RunStatus};
use sagat_domain::quota::QuotaResource;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::run_admission::{count, set};
use crate::support::engine::{World, activity, document, with};
use crate::support::flow_control::{
    age_start_deadline, database_now_us, gated_world, publish, until_past,
};

const HOUR_US: i64 = 3_600_000_000;
const SECOND_US: i64 = 1_000_000;

/// A one-step workflow `name` whose start takes a slot of `input.accountId`, `limit` per `per`.
fn rated_start(name: &str, limit: u32, per: &str) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": limit, "per": per});
    doc
}

/// A one-step workflow `name` whose step `g` takes a slot of group `group` on `input.accountId`, `limit` per `per`.
fn rated_step(name: &str, group: &str, limit: u32, per: &str) -> Value {
    document(
        name,
        "g",
        vec![with(
            activity("g"),
            "rateLimit",
            json!({"group": group, "key": "input.accountId", "limit": limit, "per": per}),
        )],
    )
}

fn us(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos() / 1_000).expect("an instant")
}

/// The run's own grant and waiter rows: (grants, waiters, granted_at, leased_at).
async fn start_slot(w: &World, run: RunId) -> (i64, i64, Option<i64>, Option<i64>) {
    let (grants, waiters, granted, leased): (i64, i64, Option<OffsetDateTime>, Option<OffsetDateTime>) =
        sqlx::query_as(
            "SELECT (SELECT count(*) FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN'), \
                    (SELECT count(*) FROM rate_limit_waiters WHERE run_id = $1 AND limit_scope = 'RUN'), \
                    (SELECT granted_at FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN'), \
                    (SELECT leased_at FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN')",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("rows");
    (grants, waiters, granted.map(us), leased.map(us))
}

/// The `granted_at_us` of the last `rate_limit.granted` of `run` (its node, or the run's start for `None`).
async fn journal_grant_us(w: &World, run: RunId, node: Option<&str>) -> i64 {
    sqlx::query_scalar(
        "SELECT (detail->>'granted_at_us')::bigint FROM run_events \
          WHERE run_id = $1 AND event_type = 'rate_limit.granted' AND node IS NOT DISTINCT FROM $2 \
          ORDER BY seq DESC LIMIT 1",
    )
    .bind(run.as_uuid())
    .bind(node)
    .fetch_one(&w.db.pool)
    .await
    .expect("a grant event")
}

/// The next-eligible hint of the window `waiter` waits in (the key of its rate waiter row), in microseconds.
async fn hint_us(w: &World, waiter: RunId) -> Option<i64> {
    let hint: Option<OffsetDateTime> = sqlx::query_scalar(
        "SELECT k.next_eligible_at FROM rate_limit_windows k JOIN rate_limit_waiters x \
             ON x.tenant_id = k.tenant_id AND x.namespace_id = k.namespace_id AND x.pool = k.pool \
            AND x.limit_scope = k.limit_scope AND x.group_name = k.group_name AND x.key_digest = k.key_digest \
          WHERE x.run_id = $1",
    )
    .bind(waiter.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the waiter's window");
    hint.map(us)
}

/// The step `g` grant of `run`: (leased_at, the occurred_at of its attempt's `step.leased`), in microseconds.
async fn step_lease(w: &World, run: RunId) -> (Option<i64>, Option<i64>) {
    let (leased, journal): (Option<OffsetDateTime>, Option<OffsetDateTime>) = sqlx::query_as(
        "SELECT (SELECT leased_at FROM rate_limit_grants WHERE run_id = $1 AND node = 'g'), \
                (SELECT e.occurred_at FROM run_events e JOIN rate_limit_grants g \
                     ON g.run_id = e.run_id AND g.node = e.node AND g.attempt = e.attempt \
                  WHERE e.run_id = $1 AND e.event_type = 'step.leased' ORDER BY e.seq DESC LIMIT 1)",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows");
    (leased.map(us), journal.map(us))
}

fn refused(result: Result<RunId, AppError>) -> FlowControlRefusal {
    match result {
        Err(AppError::FlowControl(refusal)) => refusal,
        other => panic!("refused expected: {other:?}"),
    }
}

// ------------------------------------------------------------------------------------------ the run's start (transaction T19)

/// A rate-limited start with room is granted in its admission transaction; its slot counts from its grant, which is its
/// lease instant (the row's `leased_at` equals its `granted_at`, and both the journal's `granted_at_us`), and outlives
/// the run: a run-level slot is never given back, it stops counting by time. The next start of the key waits, and the
/// pass that cannot grant it rests the key until exactly the grant's instant plus the window.
#[tokio::test]
async fn a_rate_limited_start_is_granted_its_slot_from_its_grant_and_the_next_waits_until_the_window_ends()
 {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-start", 1, "hour")).await;
    let session = w.session(&["act.g"]).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let journal = w.journal(first).await;
    assert_eq!(
        &journal[..3],
        ["run.started", "rate_limit.granted", "step.released"],
        "{journal:?}"
    );
    let (grants, waiters, granted, leased) = start_slot(&w, first).await;
    let event = journal_grant_us(&w, first, None).await;
    assert_eq!((grants, waiters), (1, 0));
    assert_eq!(granted, Some(event), "the row's instant is the journal's");
    assert_eq!(leased, granted, "a run's slot counts from its grant");
    w.assert_consistent(first).await;

    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(
        w.journal(second).await,
        ["run.started", "rate_limit.queued"]
    );
    assert_eq!(w.run_status(second).await, RunStatus::Waiting);
    assert_eq!(start_slot(&w, second).await.1, 1);
    let other = w.start(workflow, &json!({"accountId": "acct-2"})).await;
    assert_eq!(start_slot(&w, other).await.0, 1, "another key has room");

    let tasks = w.poll(&session, 8).await;
    assert_eq!(tasks.len(), 2, "the two runs that hold a slot run");
    let task = tasks
        .iter()
        .find(|t| t.run_id == first)
        .expect("the first's task");
    w.complete(&session, task, &json!({})).await.expect("done");
    assert_eq!(w.run_status(first).await, RunStatus::Succeeded);
    assert_eq!(
        start_slot(&w, first).await.0,
        1,
        "the slot outlives the run: it counts by time"
    );
    assert_eq!(count(&w, first, "rate_limit.returned").await, 0);
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 0, "{report:?}");
    // the key rests until the first instant the grant stops counting for the waiter: its instant plus one hour
    assert_eq!(hint_us(&w, second).await, Some(event + HOUR_US));
    for run in [first, second, other] {
        w.assert_consistent(run).await;
    }
}

/// A buffered start is granted by the first pass after the slot ahead of it stops counting (`leased_at + per` is past on
/// the database clock), not before; the one behind it waits for its own window.
#[tokio::test]
async fn a_buffered_start_is_granted_once_the_window_of_the_slot_ahead_has_passed() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-window", 1, "second")).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let third = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let first_at = journal_grant_us(&w, first, None).await;
    until_past(&w.db.pool, first_at + SECOND_US).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(count(&w, second, "rate_limit.granted").await, 1);
    let second_at = journal_grant_us(&w, second, None).await;
    let (grants, waiters, _, leased) = start_slot(&w, second).await;
    assert_eq!((grants, waiters, leased), (1, 0, Some(second_at)));
    assert_eq!(
        start_slot(&w, third).await.1,
        1,
        "the third waits on: {:?} {:?}",
        w.journal(third).await,
        start_slot(&w, third).await
    );
    assert_eq!(w.run_status(second).await, RunStatus::Running);
    for run in [first, second, third] {
        w.assert_consistent(run).await;
    }
}

/// D022-006, FR-022-044: a start that declares both gates gets its permit and its slot in one commit, or waits for both
/// with one deadline; a run never commits with a permit and no slot, or the reverse.
#[tokio::test]
async fn a_dual_gated_start_takes_its_permit_and_slot_together_or_waits_for_both() {
    let w = gated_world().await;
    let mut doc = rated_start("rate-dual", 1, "hour");
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 2});
    let workflow = publish(&w, doc).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let journal = w.journal(first).await;
    assert_eq!(
        &journal[..3],
        ["run.started", "rate_limit.granted", "concurrency.acquired"],
        "{journal:?}"
    );
    assert_eq!(crate::run_admission::start_rows(&w, first).await, (1, 0));
    assert_eq!(start_slot(&w, first).await.0, 1);

    // the permit has room (one holder of two), the slot does not: the run takes neither
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(
        w.journal(second).await,
        ["run.started", "rate_limit.queued", "concurrency.queued"]
    );
    assert_eq!(crate::run_admission::start_rows(&w, second).await, (0, 1));
    assert_eq!(start_slot(&w, second).await.1, 1);
    let deadlines: Vec<OffsetDateTime> = sqlx::query_scalar(
        "SELECT deadline_at FROM concurrency_waiters WHERE run_id = $1 \
         UNION ALL SELECT deadline_at FROM rate_limit_waiters WHERE run_id = $1",
    )
    .bind(second.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("deadlines");
    assert_eq!(deadlines.len(), 2);
    assert_eq!(deadlines[0], deadlines[1], "one deadline for both gates");
    for run in [first, second] {
        w.assert_consistent(run).await;
    }
}

/// `contracts/errors.md` (US07), rate first (FR-022-052): a start that would be buffered beyond its key's waiter ceiling
/// or the namespace's `queued_runs` is refused `RATE_LIMIT_QUEUE_FULL` (a dual-gated one too), and one whose key would be
/// a new window beyond `rate_limit_live_keys` `FLOW_CONTROL_LIMIT`; nothing of a refused start exists.
#[tokio::test]
async fn a_start_beyond_a_rate_ceiling_is_refused_rate_first_and_nothing_of_it_exists() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-ceilings", 1, "hour")).await;
    let mut dual = rated_start("rate-ceilings-dual", 1, "hour");
    dual["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 5});
    let dual = publish(&w, dual).await;
    set(&w, QuotaResource::FlowControlKeyWaiters, 1).await;
    let _holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let _waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let runs_before: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&w.db.pool)
        .await
        .expect("runs");
    let full = w
        .start_with(
            workflow,
            &json!({"accountId": "acct-1"}),
            "rate-full-key",
            None,
        )
        .await;
    assert_eq!(refused(full), FlowControlRefusal::RateLimitQueueFull);
    set(&w, QuotaResource::FlowControlKeyWaiters, 10).await;
    set(&w, QuotaResource::QueuedRuns, 1).await;
    let namespace_full = w
        .start_with(
            workflow,
            &json!({"accountId": "acct-1"}),
            "rate-full-ns",
            None,
        )
        .await;
    assert_eq!(
        refused(namespace_full),
        FlowControlRefusal::RateLimitQueueFull
    );
    // a dual-gated start whose slot is full meets the full namespace rate first
    let _dual_holder = w.start(dual, &json!({"accountId": "acct-9"})).await;
    let dual_full = w
        .start_with(
            dual,
            &json!({"accountId": "acct-9"}),
            "rate-full-dual",
            None,
        )
        .await;
    assert_eq!(refused(dual_full), FlowControlRefusal::RateLimitQueueFull);
    // two windows exist (acct-1 and acct-9, at run scope): at a ceiling of two a third key is refused
    set(&w, QuotaResource::RateLimitLiveKeys, 2).await;
    let new_key = w
        .start_with(
            workflow,
            &json!({"accountId": "acct-2"}),
            "rate-new-key",
            None,
        )
        .await;
    assert_eq!(
        refused(new_key),
        FlowControlRefusal::Limit {
            limit: "rate_limit_live_keys"
        }
    );
    let runs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&w.db.pool)
        .await
        .expect("runs");
    assert_eq!(
        runs_after,
        runs_before + 1,
        "only the dual holder was created; a refused start leaves no run"
    );
}

/// FR-022-047: the effective limit is the lowest of the declaration and the namespace's ceiling of the window's length;
/// the ceiling of another length does not clamp it. At the bound one more waits; zero is not a value (refused when set).
#[tokio::test]
async fn the_namespace_ceiling_of_a_window_length_clamps_a_declared_rate_limit() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-clamp", 3, "hour")).await;
    set(&w, QuotaResource::RateLimitPerSecond, 1).await;
    set(&w, QuotaResource::RateLimitPerMinute, 1).await;
    let a = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let b = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(
        start_slot(&w, b).await.0,
        1,
        "the hour's ceiling is not lowered"
    );
    set(&w, QuotaResource::RateLimitPerHour, 2).await;
    let c = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(
        w.journal(c).await,
        ["run.started", "rate_limit.queued"],
        "two grants count against an effective limit of two"
    );
    let queued_limit: i32 =
        sqlx::query_scalar("SELECT declared_limit FROM rate_limit_waiters WHERE run_id = $1")
            .bind(c.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("waiter");
    assert_eq!(
        queued_limit, 2,
        "the waiter is filed with its effective limit"
    );
    let zero = w
        .quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::RateLimitPerHour,
            0,
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(zero.is_err(), "a policy of zero is not a value: {zero:?}");
    for run in [a, b, c] {
        w.assert_consistent(run).await;
    }
}

/// A buffered start whose deadline passed fails `RATE_LIMIT_WAIT_TIMEOUT` directly, never having run.
#[tokio::test]
async fn a_buffered_start_past_its_deadline_fails_rate_limit_wait_timeout() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-timeout", 1, "hour")).await;
    let _holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(w.scheduler.tick().await.gate_waits_timed_out, 0);
    age_start_deadline(&w, waiter).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.gate_waits_timed_out, 1, "{report:?}");
    assert_eq!(w.run_status(waiter).await, RunStatus::Failed);
    let (code,): (Option<String>,) = sqlx::query_as("SELECT stop_code FROM runs WHERE id = $1")
        .bind(waiter.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("run");
    assert_eq!(code.as_deref(), Some("RATE_LIMIT_WAIT_TIMEOUT"));
    assert_eq!(count(&w, waiter, "rate_limit.wait_timed_out").await, 1);
    assert_eq!(start_slot(&w, waiter).await, (0, 0, None, None));
    w.assert_consistent(waiter).await;
}

// --------------------------------------------------------------------------------------- a step's lease (transaction T23)

/// D022-005, D022-029: a step's slot counts unleased, then for its window from its attempt's lease. The pass copies the
/// instant of the granted attempt's `step.leased` exactly, records nothing before the lease and nothing twice, and the
/// key then rests until exactly that instant plus the window.
#[tokio::test]
async fn a_step_slot_counts_from_the_lease_the_pass_copies_from_the_journal() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_step("rate-lease", "provider", 1, "hour")).await;
    let session = w.session(&["act.g"]).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(count(&w, first, "rate_limit.granted").await, 1);
    assert_eq!(count(&w, second, "rate_limit.queued").await, 1);
    assert_eq!(step_lease(&w, first).await, (None, None));
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        0,
        "nothing is leased yet"
    );
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, first);
    let report = w.scheduler.record_rate_leases().await;
    assert_eq!(report.rate_leases_recorded, 1, "{report:?}");
    let (leased, journal) = step_lease(&w, first).await;
    assert!(journal.is_some());
    assert_eq!(leased, journal, "the lease instant is the journal's");
    let granted = journal_grant_us(&w, first, Some("g")).await;
    assert!(leased >= Some(granted), "never before the grant");
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        0,
        "a lease is recorded once"
    );
    assert_eq!(w.scheduler.tick().await.gates_granted, 0);
    assert_eq!(
        hint_us(&w, second).await,
        leased.map(|at| at + HOUR_US),
        "the key rests until the lease plus the window"
    );
    w.complete(&session, &task, &json!({})).await.expect("done");
    for run in [first, second] {
        w.assert_consistent(run).await;
    }
}

/// An unleased slot frees by a return (which sets the hint) or by a lease, whose recording lowers the hint, so the key
/// rests until the pass's start plus the waiter's window: the hint the pass was selected by plus a minute, exactly
/// (early, never late; D022-021, D022-064, correctness review Major 3 of 2026-11-20).
#[tokio::test]
async fn an_unleased_slot_rests_its_key_until_the_pass_start_plus_the_window() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_step("rate-unleased", "provider", 1, "minute")).await;
    let _first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let selected_by = hint_us(&w, second)
        .await
        .expect("a new waiter makes its key due");
    assert!(selected_by <= database_now_us(&w.db.pool).await);
    assert_eq!(w.scheduler.tick().await.gates_granted, 0);
    assert_eq!(
        hint_us(&w, second).await,
        Some(selected_by + 60 * SECOND_US)
    );
    w.assert_consistent(second).await;
}

/// Correctness review, Major 4, of 2026-11-20: a grant made to a window through another key's pass (a dual-gated waiter served
/// when its permit frees) wakes the window's other waiters when that slot may first stop counting: its instant plus the
/// shortest window they wait over. Before, the window rested with no hint (nothing counted when its pass ran), and the
/// waiter behind could only time out.
#[tokio::test]
async fn a_grant_through_another_keys_pass_wakes_the_waiters_of_its_window() {
    let w = gated_world().await;
    let holder_doc = document(
        "rate-holder",
        "g",
        vec![with(
            activity("g"),
            "concurrency",
            json!({"group": "conn", "key": "input.accountId", "limit": 1}),
        )],
    );
    let mut dual_doc = rated_step("rate-dual-head", "shared", 1, "minute");
    dual_doc["nodes"][0]["concurrency"] =
        json!({"group": "conn", "key": "input.accountId", "limit": 1});
    let holder = publish(&w, holder_doc).await;
    let dual = publish(&w, dual_doc).await;
    let rate_only = publish(&w, rated_step("rate-behind", "shared", 1, "minute")).await;
    let session = w.session(&["act.g"]).await;
    let held = w.start(holder, &json!({"accountId": "acct-1"})).await;
    let head = w.start(dual, &json!({"accountId": "acct-1"})).await;
    let behind = w.start(rate_only, &json!({"accountId": "acct-1"})).await;
    assert_eq!(count(&w, head, "rate_limit.queued").await, 1);
    assert_eq!(count(&w, behind, "rate_limit.queued").await, 1);
    // nothing counts on the window: its pass refuses the head (its permit) and the one behind (the head's reservation)
    assert_eq!(w.scheduler.tick().await.gates_granted, 0);
    assert_eq!(
        hint_us(&w, behind).await,
        None,
        "the window rests: nothing counts"
    );
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, held);
    w.complete(&session, &task, &json!({})).await.expect("done");
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    let granted = journal_grant_us(&w, head, Some("g")).await;
    assert_eq!(
        hint_us(&w, behind).await,
        Some(granted + 60 * SECOND_US),
        "the window wakes when the head's slot may first stop counting"
    );
    for run in [held, head, behind] {
        w.assert_consistent(run).await;
    }
}

/// T022-079: `verify_run` holds invariants 8 and 9 for the start (a run holds a grant row with its journal's instant
/// while its journal holds the slot and the slot can still count): a grant row gone too early is a breach.
#[tokio::test]
async fn verify_run_refuses_a_start_whose_grant_row_is_gone_while_it_still_counts() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-verify", 1, "hour")).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    w.assert_consistent(run).await;
    sqlx::query("DELETE FROM rate_limit_grants WHERE run_id = $1")
        .bind(run.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("seeded breach");
    let verdict = w
        .store
        .verify_run(w.admin.tenant_id(), run)
        .await
        .expect_err("a breach");
    assert!(verdict.contains("invariants 8 and 9"), "{verdict}");
}

/// D022-009: each acquirer counts the key's grants over its own window. A waiter whose window is shorter than a grant's
/// is woken when the grant stops counting for it (its lease plus the waiter's window, here a minute), not when it stops
/// counting for the grant's own declaration (an hour: red before D022-064).
#[tokio::test]
async fn a_waiter_with_a_shorter_window_is_woken_when_the_grant_ages_out_for_it() {
    let w = gated_world().await;
    let hourly = publish(&w, rated_step("rate-hourly", "shared", 1, "hour")).await;
    let minutely = publish(&w, rated_step("rate-minutely", "shared", 1, "minute")).await;
    let session = w.session(&["act.g"]).await;
    let first = w.start(hourly, &json!({"accountId": "acct-1"})).await;
    let second = w.start(minutely, &json!({"accountId": "acct-1"})).await;
    assert_eq!(count(&w, second, "rate_limit.queued").await, 1);
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, first);
    w.scheduler.record_rate_leases().await;
    let leased = step_lease(&w, first).await.0.expect("a lease instant");
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 0, "{report:?}");
    assert_eq!(
        hint_us(&w, second).await,
        Some(leased + 60 * SECOND_US),
        "the waiter's window, not the grant's"
    );
    w.complete(&session, &task, &json!({})).await.expect("done");
    for run in [first, second] {
        w.assert_consistent(run).await;
    }
}

/// SC-022-021: with no worker for three windows at most `limit` granted slots are unleased, and a returning worker
/// leases at most `limit` in its first window (a slot counts while unleased, then from its lease).
#[tokio::test]
async fn without_workers_at_most_limit_slots_wait_unleased_and_a_returning_worker_leases_at_most_limit()
 {
    let w = gated_world().await;
    let workflow = publish(&w, rated_step("rate-no-worker", "provider", 2, "second")).await;
    let mut runs = Vec::new();
    for _ in 0..6 {
        runs.push(w.start(workflow, &json!({"accountId": "acct-1"})).await);
    }
    let unleased = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM rate_limit_grants WHERE limit_scope = 'STEP' AND leased_at IS NULL",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("grants")
    };
    let start = database_now_us(&w.db.pool).await;
    for window in 1..=3 {
        until_past(&w.db.pool, start + window * SECOND_US).await;
        w.scheduler.tick().await;
        assert_eq!(
            unleased().await,
            2,
            "window {window}: at most the limit wait unleased"
        );
    }
    let session = w.session(&["act.g"]).await;
    let tasks = w.poll(&session, 8).await;
    assert_eq!(
        tasks.len(),
        2,
        "the returning worker leases at most the limit"
    );
    assert_eq!(
        w.scheduler.record_rate_leases().await.rate_leases_recorded,
        2
    );
    let granted_total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE event_type = 'rate_limit.granted' AND run_id = ANY($1)",
    )
    .bind(runs.iter().map(|run| run.as_uuid()).collect::<Vec<_>>())
    .fetch_one(&w.db.pool)
    .await
    .expect("grants");
    assert_eq!(
        granted_total, 2,
        "nothing was granted while no worker leased"
    );
    for run in runs {
        w.assert_consistent(run).await;
    }
}

// ----------------------------------------------------------------------------------------------- the purge (transaction T25)

/// FR-022-056: a grant is kept for its window row's longest period and one minute after its lease; exactly at that
/// instant it is kept, one microsecond later it is deleted, the window's count follows, and the window that no grant and
/// no waiter uses any more goes with it.
#[tokio::test]
async fn the_purge_keeps_a_grant_for_the_longest_period_and_a_minute_and_not_a_microsecond_more() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-purge", 1, "hour")).await;
    let session = w.session(&["act.g"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({})).await.expect("done");
    let (_, _, _, leased) = start_slot(&w, run).await;
    let leased = leased.expect("a lease instant");
    let kept_until = leased + HOUR_US + 60 * SECOND_US;
    let at = |micros: i64| {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000).expect("an instant")
    };
    assert_eq!(
        w.store
            .purge_rate_grants_as_of(at(kept_until), 64, 100)
            .await
            .expect("purge"),
        0,
        "exactly at the boundary the grant is kept"
    );
    assert_eq!(start_slot(&w, run).await.0, 1);
    assert_eq!(
        w.store
            .purge_rate_grants_as_of(at(kept_until + 1), 64, 100)
            .await
            .expect("purge"),
        1,
        "one microsecond later it is deleted"
    );
    let windows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_limit_windows WHERE group_name = 'accounts'")
            .fetch_one(&w.db.pool)
            .await
            .expect("windows");
    assert_eq!(
        windows, 0,
        "the empty window is deleted with its last grant"
    );
}

/// The purge keeps a grant as long as the longest period its window row has seen, not its own: a per-second grant of a
/// key an hourly acquirer has waited on is kept an hour and a minute (FR-022-056, D022-009).
#[tokio::test]
async fn the_purge_keeps_a_short_grant_for_the_longest_period_its_window_has_seen() {
    let w = gated_world().await;
    let secondly = publish(&w, rated_step("rate-purge-s", "shared", 1, "second")).await;
    let hourly = publish(&w, rated_step("rate-purge-h", "shared", 1, "hour")).await;
    let session = w.session(&["act.g"]).await;
    let first = w.start(secondly, &json!({"accountId": "acct-1"})).await;
    let _hourly_waiter = w.start(hourly, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, first);
    w.scheduler.record_rate_leases().await;
    let leased = step_lease(&w, first).await.0.expect("a lease instant");
    let at = |micros: i64| {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000).expect("an instant")
    };
    let max_period: i32 = sqlx::query_scalar(
        "SELECT max_period_ms FROM rate_limit_windows WHERE group_name = 'shared'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("window");
    assert_eq!(max_period, 3_600_000);
    assert_eq!(
        w.store
            .purge_rate_grants_as_of(at(leased + SECOND_US + 60 * SECOND_US + 1), 64, 100)
            .await
            .expect("purge"),
        0,
        "its own window and a minute are not enough"
    );
    assert_eq!(
        w.store
            .purge_rate_grants_as_of(at(leased + HOUR_US + 60 * SECOND_US + 1), 64, 100)
            .await
            .expect("purge"),
        1
    );
    w.complete(&session, &task, &json!({})).await.expect("done");
}

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
    // the window's oldest wait moved to the waiter that remains, exactly (decision D022-068 item 5; correctness Major 3
    // of the review of task T022-078)
    let (oldest, third_enqueued): (Option<OffsetDateTime>, OffsetDateTime) = sqlx::query_as(
        "SELECT k.oldest_wait_at, x.enqueued_at FROM rate_limit_windows k JOIN rate_limit_waiters x \
             ON x.tenant_id = k.tenant_id AND x.namespace_id = k.namespace_id AND x.pool = k.pool \
            AND x.limit_scope = k.limit_scope AND x.group_name = k.group_name AND x.key_digest = k.key_digest \
          WHERE x.run_id = $1",
    )
    .bind(third.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the third's window");
    assert_eq!(
        oldest,
        Some(third_enqueued),
        "the remaining waiter's instant"
    );
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
/// rests until the pass's start plus the waiter's window: the instant of the statement that selected the key plus a
/// minute (early, never late; D022-021, D022-064, correctness review Major 3 of 2026-11-20). That statement runs within
/// the tick, so the hint lies between the database clock read before the tick and the one read after it, each plus a
/// minute, and after the hint the key was selected by plus a minute (decision D022-066 item 3: the selection's instant,
/// not the older hint, which fell behind real time after a lag).
#[tokio::test]
async fn an_unleased_slot_rests_its_key_until_the_pass_start_plus_the_window() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_step("rate-unleased", "provider", 1, "minute")).await;
    let _first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let selected_by = hint_us(&w, second)
        .await
        .expect("a new waiter makes its key due");
    let before = database_now_us(&w.db.pool).await;
    assert!(selected_by < before);
    assert_eq!(w.scheduler.tick().await.gates_granted, 0);
    let after = database_now_us(&w.db.pool).await;
    let rested = hint_us(&w, second).await.expect("a hint");
    assert!(
        (before + 60 * SECOND_US..=after + 60 * SECOND_US).contains(&rested),
        "the selection's instant plus the window: {before} <= {rested} - 60 s <= {after}"
    );
    assert!(
        rested > selected_by + 60 * SECOND_US,
        "not the older hint's"
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
    // nothing counts on the window: its pass refuses the head (its permit) and the one behind (the head's reservation).
    // One key at a time: with both keys' passes at once, the window's pass may find the head's run held by the permit's
    // pass and skip it, which keeps the window's hint by design (`Busy`; a loaded runner did so on 2026-11-25)
    let one_at_a_time = sagat_scheduler::SchedulerSettings {
        concurrency: 1,
        ..sagat_scheduler::SchedulerSettings::default()
    };
    assert_eq!(
        crate::support::scale::scheduler(&w, one_at_a_time, None)
            .tick()
            .await
            .gates_granted,
        0
    );
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

/// Moves the instants of `run`'s grant rows by `by_us` microseconds, as the owner, with the guard that forbids it
/// switched off for this transaction only (decision D022-072: counted instants are moved, the clock is not).
pub(crate) async fn move_grant_instants(w: &World, run: RunId, by_us: i64) {
    let mut tx = w.db.pool.begin().await.expect("tx");
    sqlx::query("ALTER TABLE rate_limit_grants DISABLE TRIGGER rate_limit_grants_guard")
        .execute(&mut *tx)
        .await
        .expect("the guard off in this transaction");
    sqlx::query(
        "UPDATE rate_limit_grants SET granted_at = granted_at + $2 * interval '1 microsecond', \
                leased_at = leased_at + $2 * interval '1 microsecond' WHERE run_id = $1",
    )
    .bind(run.as_uuid())
    .bind(by_us)
    .execute(&mut *tx)
    .await
    .expect("the grant's instants moved");
    sqlx::query("ALTER TABLE rate_limit_grants ENABLE ALWAYS TRIGGER rate_limit_grants_guard")
        .execute(&mut *tx)
        .await
        .expect("the guard on again");
    tx.commit().await.expect("moved");
}

/// Task T022-103 (FR-022-056), mixed periods on one key: two per-second slots and one hourly slot of `acct-1`. The window
/// row keeps the longest period (an hour), so a per-second grant is kept an hour and a minute after its lease, not its
/// own second and a minute; the purge deletes only what is past that, at most `limit` rows a key, and the window's
/// `live_grants` goes down by exactly the rows each purge deleted. The hourly grant, leased ten minutes later (its
/// instants moved, decision D022-072), stays.
#[tokio::test]
async fn the_purge_of_a_key_with_mixed_periods_counts_down_the_rows_it_deleted() {
    const LATER_US: i64 = 600 * SECOND_US;
    let w = gated_world().await;
    let secondly = publish(&w, rated_start("rate-purge-mixed-s", 5, "second")).await;
    let hourly = publish(&w, rated_start("rate-purge-mixed-h", 5, "hour")).await;
    let input = json!({"accountId": "acct-1"});
    let short = [
        w.start(secondly, &input).await,
        w.start(secondly, &input).await,
    ];
    let long = w.start(hourly, &input).await;
    for run in short.iter().chain([&long]) {
        assert_eq!(start_slot(&w, *run).await.0, 1, "granted at once");
    }
    // the runs end: a grant counts until it ages out, whatever its run does since
    let session = w.session(&["act.g"]).await;
    for task in w.poll(&session, 8).await {
        w.complete(&session, &task, &json!({})).await.expect("done");
    }
    move_grant_instants(&w, long, LATER_US).await;
    let window = || async {
        sqlx::query_as::<_, (i32, i32, i64)>(
            "SELECT w.max_period_ms, w.live_grants, \
                    (SELECT count(*) FROM rate_limit_grants g WHERE g.group_name = w.group_name) \
               FROM rate_limit_windows w WHERE w.group_name = 'accounts'",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("the window")
    };
    assert_eq!(window().await, (3_600_000, 3, 3));
    let mut latest = 0;
    for run in short {
        latest = latest.max(start_slot(&w, run).await.3.expect("a lease instant"));
    }
    let at = |micros: i64| {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000).expect("an instant")
    };
    assert_eq!(
        w.store
            .purge_rate_grants_as_of(at(latest + SECOND_US + 60 * SECOND_US + 1), 64, 100)
            .await
            .expect("purge"),
        0,
        "a per-second grant is kept for the window's longest period"
    );
    let past_short = at(latest + HOUR_US + 60 * SECOND_US + 1);
    assert_eq!(
        w.store
            .purge_rate_grants_as_of(past_short, 64, 1)
            .await
            .expect("purge"),
        1,
        "at most `limit` rows of a key"
    );
    assert_eq!(
        window().await,
        (3_600_000, 2, 2),
        "down by the one row deleted"
    );
    assert_eq!(
        w.store
            .purge_rate_grants_as_of(past_short, 64, 100)
            .await
            .expect("purge"),
        1,
        "the other per-second grant; the hourly one is ten minutes younger"
    );
    assert_eq!(
        window().await,
        (3_600_000, 1, 1),
        "down by the one row deleted"
    );
    assert_eq!(start_slot(&w, long).await.0, 1, "the hourly grant stays");
    // (the consistency checks are not run: they compare the journal with the true clock, under which the grants this
    // purge deleted as of an hour from now still count; the window's own count is checked above)
    move_grant_instants(&w, long, -LATER_US).await;
}

// ------------------------------------------------------------------------------- the re-reviews of 2026-11-20 (D022-066)

/// The namespace's rate-limit window rows (both scopes, every version).
async fn window_rows(w: &World) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM rate_limit_windows WHERE namespace_id = $1")
        .bind(w.namespace.id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("windows")
}

/// Architecture Blocker 1 and security Major 1 of the re-reviews (decision D022-066 item 1): a window row counts against
/// `rate_limit_live_keys` until it goes, whether or not its grants still count, so a key whose slot aged out keeps its
/// place: at the ceiling a new key is refused, and the aged key is decided as before on the row that exists (no row is
/// added). Before, an aged window did not count, so a new key passed the ceiling and the aged one could be revived beyond it.
#[tokio::test]
async fn an_aged_window_counts_against_the_live_keys_ceiling_until_it_goes() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-aged", 1, "second")).await;
    set(&w, QuotaResource::RateLimitLiveKeys, 1).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let leased = start_slot(&w, first)
        .await
        .3
        .expect("a run's slot counts from its grant");
    // the slot no longer counts; the row is not one the purge may delete before its window and one minute more
    until_past(&w.db.pool, leased + SECOND_US).await;
    let new_key = w
        .start_with(
            workflow,
            &json!({"accountId": "acct-2"}),
            "rate-aged-new",
            None,
        )
        .await;
    assert_eq!(
        refused(new_key),
        FlowControlRefusal::Limit {
            limit: "rate_limit_live_keys"
        },
        "the aged row still counts"
    );
    let again = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let (grants, waiters, _, _) = start_slot(&w, again).await;
    assert_eq!((grants, waiters), (1, 0), "granted on the row that exists");
    assert_eq!(window_rows(&w).await, 1, "no row beyond the ceiling");
    let usage = w
        .quotas
        .list(
            &w.admin,
            Some(w.namespace.id()),
            &crate::support::engine::ctx(),
        )
        .await
        .expect("usage");
    let counted = usage
        .iter()
        .find(|row| row.resource == QuotaResource::RateLimitLiveKeys)
        .expect("the resource's usage");
    assert_eq!(
        counted.used,
        Some(1),
        "the usage counts the rows the gate counts"
    );
    for run in [first, again] {
        w.assert_consistent(run).await;
    }
}

/// Decision D022-066 item 1: a decision that would be refused by `rate_limit_live_keys` first reclaims a window of its own
/// namespace that no waiter and only grants the purge may delete keep (FR-022-056: its longest period and one minute
/// after the lease, the purge's own predicate), and decides with the room that frees; the reclaimed window goes at the
/// commit with its grant. The namespace never waits for the deployment-wide purge.
#[tokio::test]
async fn a_new_key_at_the_ceiling_reclaims_a_window_only_purgeable_grants_keep() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-reclaim", 1, "second")).await;
    set(&w, QuotaResource::RateLimitLiveKeys, 1).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let leased = start_slot(&w, first).await.3.expect("a lease instant");
    until_past(&w.db.pool, leased + SECOND_US + 60 * SECOND_US).await;
    let second = w.start(workflow, &json!({"accountId": "acct-2"})).await;
    assert_eq!(
        start_slot(&w, second).await.0,
        1,
        "granted: the aged window was reclaimed"
    );
    assert_eq!(
        start_slot(&w, first).await.0,
        0,
        "the first's grant is gone"
    );
    assert_eq!(window_rows(&w).await, 1, "one row: the new key's");
    for run in [first, second] {
        w.assert_consistent(run).await;
    }
}

/// OD-2026-11-20-11 (the owner, "Yes, extend it"): the rate live-keys count reads the committed window rows without a
/// namespace lock, as the concurrency count does, so it misses only the keys other transactions of the same moment have
/// created and not committed. With the ceiling at two, one key committed and one created by a transaction still in
/// flight, a readiness round creates a second key (it counts one); once both commit the ceiling is passed by exactly the
/// other transaction's key, and the next new key, which counts every committed row, is refused.
#[tokio::test]
async fn the_rate_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_step("rate-overshoot", "provider", 1, "hour")).await;
    set(&w, QuotaResource::RateLimitLiveKeys, 2).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(window_rows(&w).await, 1);
    // another transaction of the same moment has created a key of the namespace and not committed
    let mut other = w.db.pool.begin().await.expect("a transaction");
    sqlx::query(
        "INSERT INTO rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, key_version, \
                                        key_shard, updated_at) \
         SELECT $1, $2, 'ORDINARY', 'STEP', 'provider', $3, active, 0, clock_timestamp() \
           FROM flow_control_key_versions_in_force()",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(vec![0x42_u8; 32])
    .execute(&mut *other)
    .await
    .expect("a key in flight");
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        w.start(workflow, &json!({"accountId": "acct-2"})),
    )
    .await
    .expect("the round takes no namespace lock");
    other.commit().await.expect("the other key commits");
    assert_eq!(
        window_rows(&w).await,
        3,
        "the ceiling of two passed by exactly the other transaction's one key"
    );
    let third = w.start(workflow, &json!({"accountId": "acct-3"})).await;
    let (status, code, attempt): (String, Option<String>, i32) = sqlx::query_as(
        "SELECT status, failure_code, attempt FROM steps WHERE run_id = $1 AND node = 'g'",
    )
    .bind(third.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the step");
    assert_eq!(
        (status.as_str(), code.as_deref(), attempt),
        ("FAILED", Some("FLOW_CONTROL_LIMIT"), 0),
        "every committed row is counted"
    );
    assert_eq!(window_rows(&w).await, 3, "no row for the refused key");
    // the stand-in key has no grant and no waiter: it goes before the consistency check, which refuses an empty window
    sqlx::query("DELETE FROM rate_limit_windows WHERE key_digest = $1")
        .bind(vec![0x42_u8; 32])
        .execute(&w.db.pool)
        .await
        .expect("the stand-in key goes");
    for run in [first, second, third] {
        w.assert_consistent(run).await;
    }
}

/// Correctness Major 1 of the re-review (decision D022-066 item 2): a rest that waits for the window a grant in flight
/// holds computes its hint after that grant commits. The grant's `least` leaves an already-due hint unchanged, so the
/// rest's compare-and-set still matches; a single statement then counted from the snapshot it took before the grant
/// committed and rested the window with no hint, and the waiter could only time out.
#[tokio::test]
async fn a_rest_that_waits_for_a_grant_in_flight_counts_that_grant() {
    use sagat_application::maintenance::MaintenanceStore;

    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-rest-race", 1, "second")).await;
    let plain = publish(&w, document("rate-rest-plain", "g", vec![activity("g")])).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(start_slot(&w, waiter).await.1, 1, "it waits");
    let leased = start_slot(&w, first).await.3.expect("a lease instant");
    until_past(&w.db.pool, leased + SECOND_US).await;
    // the key is due and nothing counts on it: the pass selects it (its hint, its instant)
    let (shard, digest, hint): (i16, Vec<u8>, OffsetDateTime) = sqlx::query_as(
        "UPDATE rate_limit_windows SET next_eligible_at = clock_timestamp() \
          WHERE namespace_id = $1 AND group_name = 'accounts' RETURNING key_shard, key_digest, next_eligible_at",
    )
    .bind(w.namespace.id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the key made due");
    let key = sagat_application::flow_control::GateKey {
        tenant_id: w.admin.tenant_id(),
        namespace_id: w.namespace.id(),
        kind: sagat_domain::workflow::GateKind::RateLimit,
        level: sagat_domain::workflow::GateLevel::Run,
        group: sagat_domain::workflow::ResolvedGroup::parse("accounts").expect("a group"),
        digest: digest.clone().try_into().expect("a digest"),
        shard: u8::try_from(shard)
            .ok()
            .and_then(sagat_domain::sharding::Shard::new)
            .expect("a shard"),
        hint,
        selected_at: hint,
    };
    // a grant in flight: the window locked, a slot granted to another run, the hint moved with `least` (unchanged)
    let other = w.start(plain, &json!({})).await;
    let mut grant = w.db.pool.begin().await.expect("a transaction");
    sqlx::query(
        "SELECT 1 FROM rate_limit_windows WHERE namespace_id = $1 AND group_name = 'accounts' FOR UPDATE",
    )
    .bind(w.namespace.id().as_uuid())
    .execute(&mut *grant)
    .await
    .expect("the window held");
    let granted: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *grant)
        .await
        .expect("an instant");
    sqlx::query(
        "INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, group_name, \
                                       key_digest, declared_limit, period_ms, granted_at, leased_at, shard) \
         VALUES ($1, $2, $3, '', 0, 'ORDINARY', 'RUN', 'accounts', $4, 1, 1000, $5, $5, 0)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(other.as_uuid())
    .bind(&digest)
    .bind(granted)
    .execute(&mut *grant)
    .await
    .expect("a grant");
    sqlx::query(
        "UPDATE rate_limit_windows SET next_eligible_at = least(next_eligible_at, $2 + interval '1 second') \
          WHERE namespace_id = $1 AND group_name = 'accounts'",
    )
    .bind(w.namespace.id().as_uuid())
    .bind(granted)
    .execute(&mut *grant)
    .await
    .expect("the hint moved");
    let rest = tokio::spawn({
        let store = w.store.clone();
        async move { store.rest_gate_key(&key, None).await }
    });
    crate::support::bounded::within(
        "the rest waiting for the window".to_owned(),
        std::time::Duration::from_secs(30),
        async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                     AND wait_event_type = 'Lock' AND state = 'active' AND query LIKE '%rate_limit_windows%' \
                     AND pid <> pg_backend_pid()",
                )
                .fetch_one(&w.db.pool)
                .await
                .expect("activity");
                if waiting > 0 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    grant.commit().await.expect("the grant commits");
    rest.await.expect("joined").expect("the rest");
    assert_eq!(
        hint_us(&w, waiter).await,
        Some(us(granted) + SECOND_US),
        "the window wakes when the grant in flight may first stop counting"
    );
    sqlx::query("DELETE FROM rate_limit_grants WHERE run_id = $1")
        .bind(other.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("the stand-in grant goes");
}

/// Decision D022-073 items 1 and 4 (strict admission order; the hint of a start queued with room): one window shared by a
/// start that also needs a permit (`limit: 1`) and one that does not, `limit: 3, per: hour`. The holder takes a slot
/// and the permit; the dual-gated waiter `w2` finds room in the window but no permit, so it waits for both and keeps its
/// room; a pass refuses it (its permit) and rests the window until the holder's slot ages out, an hour ahead. A start
/// `a` of the rate-only definition then finds room in the window and an eligible waiter ahead: it queues, the window's
/// hint drops to the instant of its queueing statement, before its commit (a later hint would hold it for the hour), and the next pass grants it while `w2`
/// still waits for its permit (the reservation rule keeps `w2`'s room: two counted of three).
#[tokio::test]
async fn a_start_behind_an_eligible_waiter_queues_although_the_window_has_room_and_the_next_pass_grants_it()
 {
    let w = gated_world().await;
    let mut dual = rated_start("rate-strict-dual", 3, "hour");
    dual["concurrency"] = json!({"group": "serial", "key": "input.accountId", "limit": 1});
    let dual = publish(&w, dual).await;
    let rated = publish(&w, rated_start("rate-strict-rated", 3, "hour")).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(dual, &input).await;
    let w2 = w.start(dual, &input).await;
    assert_eq!(
        w.journal(w2).await,
        ["run.started", "rate_limit.queued", "concurrency.queued"]
    );
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 0, "no permit for w2: {report:?}");
    let rested = hint_us(&w, w2).await.expect("a rested window");
    assert!(
        rested > database_now_us(&w.db.pool).await,
        "the window rests until the holder's slot ages out: {rested}"
    );
    let a = w.start(rated, &input).await;
    let committed = database_now_us(&w.db.pool).await;
    let (grants, waiters, ..) = start_slot(&w, a).await;
    assert_eq!(
        (grants, waiters),
        (0, 1),
        "room in the window, an eligible waiter ahead: it queues: {:?}",
        w.journal(a).await
    );
    let hint = hint_us(&w, a).await.expect("a due window");
    assert!(
        hint <= committed,
        "the window is due once the start has committed: {hint} > {committed}"
    );
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(start_slot(&w, a).await.0, 1, "the start is granted");
    assert_eq!(start_slot(&w, w2).await.1, 1, "w2 waits for its permit");
    for run in [holder, w2, a] {
        w.assert_consistent(run).await;
    }
    w.assert_keys_consistent().await;
}

/// Decision D022-076 item 7 (re-review minor 7; spec US07 edge cases, "a lowered policy applies to waiters at once", and
/// its raised twin): a raised namespace ceiling applies to the starts already buffered, up to their declaration. Declared
/// 3 an hour, the namespace's ceiling of an hour 1: one holder, two starts buffered under it (filed with 1, their
/// declaration of 3 kept beside it). The ceiling raised to 3, one pass grants both (bounded by the filed 1, neither
/// would be granted for the hour), each grant recording the limit of 3 it was decided with; a fourth start then waits.
#[tokio::test]
async fn a_raised_ceiling_applies_to_the_buffered_starts_up_to_their_declaration() {
    let w = gated_world().await;
    let workflow = publish(&w, rated_start("rate-raised", 3, "hour")).await;
    set(&w, QuotaResource::RateLimitPerHour, 1).await;
    let input = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &input).await;
    let buffered = [
        w.start(workflow, &input).await,
        w.start(workflow, &input).await,
    ];
    for run in buffered {
        assert_eq!(start_slot(&w, run).await.1, 1, "a ceiling of one an hour");
    }
    let (filed, pinned): (i32, Option<i32>) = sqlx::query_as(
        "SELECT declared_limit, pinned_limit FROM rate_limit_waiters WHERE run_id = $1",
    )
    .bind(buffered[0].as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the waiter");
    assert_eq!(
        (filed, pinned),
        (1, Some(3)),
        "filed under the ceiling, its declaration kept"
    );
    set(&w, QuotaResource::RateLimitPerHour, 3).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 2,
        "the raised ceiling applies to the buffered starts: {report:?}"
    );
    for run in buffered {
        assert_eq!(start_slot(&w, run).await.0, 1);
        let decided: (i32, i64) = sqlx::query_as(
            "SELECT (SELECT declared_limit FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN'), \
                    (SELECT (detail->>'limit')::bigint FROM run_events \
                      WHERE run_id = $1 AND event_type = 'rate_limit.granted' AND node IS NULL)",
        )
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the grant");
        assert_eq!(decided, (3, 3), "decided with the raised ceiling");
    }
    let fourth = w.start(workflow, &input).await;
    assert_eq!(
        start_slot(&w, fourth).await.1,
        1,
        "never above the declaration"
    );
    for run in [holder, buffered[0], buffered[1], fourth] {
        w.assert_consistent(run).await;
    }
}

// ------------------------------------------------------------------------------- no re-authorization at grant (T022-106)

/// Task T022-106 (FR-022-060, decision D022-034): a grant from the buffer does not re-authorize. `carol`, a developer,
/// starts a run that waits behind a holder for its permit and its slot; she then loses `run:start` (her membership is
/// demoted to viewer, and a start of hers is refused). Once the holder ends and its slot ages out, the next pass grants
/// her buffered run both gates all the same, and the run keeps her as its initiator: `runs.started_by`, the actor of its
/// `run.started` and its one `run.started` audit record; the pass adds no start record of its own.
#[tokio::test]
async fn a_grant_from_the_buffer_does_not_reauthorize_and_keeps_the_initiator() {
    use sagat_application::runs::StartRun;
    use sagat_domain::identity::{Permission, Principal, Role};

    let w = gated_world().await;
    let mut doc = rated_start("rate-initiator", 1, "second");
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": 1});
    let workflow = publish(&w, doc).await;
    let session = w.session(&["act.g"]).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let carol = w.member("carol", Role::Developer).await;
    w.bind_everything(&carol).await;
    let input = serde_json::to_vec(&json!({"accountId": "acct-1"})).expect("json");
    let start = |principal: Principal, key: &'static str| {
        let input = input.clone();
        let w = &w;
        async move {
            w.runs
                .start(
                    &principal,
                    &StartRun {
                        namespace: "payments",
                        workflow,
                        version: None,
                        input: &input,
                        correlation_id: None,
                        deadline: None,
                        key,
                    },
                    &crate::support::engine::ctx(),
                )
                .await
                .map(|record| record.info.run_id)
        }
    };
    let buffered = start(carol.clone(), "initiator-buffered")
        .await
        .expect("admitted");
    assert_eq!(
        w.journal(buffered).await,
        ["run.started", "rate_limit.queued", "concurrency.queued"]
    );

    sqlx::query(
        "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = 'carol'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("demoted");
    let demoted = Principal::new(w.admin.tenant_id(), carol.subject().clone(), Role::Viewer);
    assert!(
        matches!(
            start(demoted, "initiator-refused").await,
            Err(AppError::Forbidden {
                permission: Permission::RunStart
            })
        ),
        "carol may no longer start a run"
    );

    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder);
    w.complete(&session, &task, &json!({})).await.expect("done");
    until_past(
        &w.db.pool,
        start_slot(&w, holder).await.2.expect("granted") + SECOND_US,
    )
    .await;
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    assert_eq!(
        start_slot(&w, buffered).await.0,
        1,
        "granted from the buffer"
    );
    assert_eq!(count(&w, buffered, "concurrency.acquired").await, 1);

    let (started_by, journal_actor): (String, String) = sqlx::query_as(
        "SELECT r.started_by, e.actor FROM runs r JOIN run_events e ON e.run_id = r.id \
          WHERE r.id = $1 AND e.event_type = 'run.started'",
    )
    .bind(buffered.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the run");
    // the run names its initiator as a subject, the journal as an actor
    assert_eq!(started_by, carol.subject().as_str());
    assert_eq!(journal_actor, format!("user:{}", carol.subject().as_str()));
    let audit: Vec<String> = sqlx::query_scalar(
        "SELECT actor FROM audit_records WHERE action = 'run.started' AND resource_id = $1",
    )
    .bind(buffered.to_string())
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        audit,
        [carol.subject().as_str().to_owned()],
        "one start record, hers"
    );
    for run in [holder, buffered] {
        w.assert_consistent(run).await;
    }
}

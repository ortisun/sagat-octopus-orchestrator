//! What a lease and a completion cost the database, as counts that a change must not raise
//! unnoticed (FR-038-001). Counts, not times: they do not depend on how loaded the machine is.
//!
//! One worker, one run at a time, no scheduler tick and no other session: nothing races, so the
//! counts are the same on every run and the budget is an equality. A change that makes the path
//! cheaper lowers the numbers here in the same change; one that makes it dearer says why.

use crate::support::engine::{World, activity, document, with, world};
use serde_json::json;
use std::collections::BTreeMap;

/// The statements, the transactions and the rows written by what ran between two readings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Work {
    statements: i64,
    transactions: i64,
    rows_written: i64,
}

impl std::ops::Sub for Work {
    type Output = Self;

    fn sub(self, before: Self) -> Self {
        Self {
            statements: self.statements - before.statements,
            transactions: self.transactions - before.transactions,
            rows_written: self.rows_written - before.rows_written,
        }
    }
}

/// Everything this test database has executed so far, but for the readings themselves. `BEGIN`
/// counts the transactions and is left out of the statements. Only top-level statements: what a trigger
/// runs inside one is counted or not by the server's `pg_stat_statements.track`, and the budget must
/// not depend on it.
async fn work(w: &World) -> Work {
    let (statements, transactions, rows_written): (i64, i64, i64) = sqlx::query_as(
        "SELECT coalesce(sum(calls) FILTER (WHERE query NOT IN ('BEGIN', 'COMMIT', 'ROLLBACK')), 0)::bigint, \
                coalesce(sum(calls) FILTER (WHERE query = 'BEGIN'), 0)::bigint, \
                coalesce(sum(rows) FILTER (WHERE query ~* '^\\s*(INSERT|UPDATE|DELETE)'), 0)::bigint \
         FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
           AND toplevel AND query NOT LIKE '%pg_stat_statements%'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("statement accounting for this test database");
    Work {
        statements,
        transactions,
        rows_written,
    }
}

/// `pg_stat_statements` keeps a bounded number of entries for the whole server and throws the
/// least used away when it is full, which restarts their counts. Every test database adds entries,
/// so in a suite that happens all the time. A window in which it happened measured nothing.
#[derive(Debug)]
struct Evicted;

async fn evictions(w: &World) -> i64 {
    sqlx::query_scalar("SELECT dealloc FROM pg_stat_statements_info")
        .fetch_one(&w.db.pool)
        .await
        .expect("eviction counter")
}

/// A reading that opens a window.
struct Reading {
    work: Work,
    evictions: i64,
}

async fn reading(w: &World) -> Reading {
    let evictions = evictions(w).await;
    Reading {
        work: work(w).await,
        evictions,
    }
}

/// What ran since `before`, unless an eviction fell in the window.
async fn spent(w: &World, before: Reading) -> Result<Work, Evicted> {
    let after = work(w).await;
    if evictions(w).await == before.evictions {
        Ok(after - before.work)
    } else {
        Err(Evicted)
    }
}

/// Runs a measured scenario again, in a world of its own, while an eviction spoils it.
macro_rules! measured {
    ($(#[$doc:meta])* $name:ident, $once:ident) => {
        $(#[$doc])*
        #[tokio::test]
        async fn $name() {
            for _ in 0..8 {
                if $once().await.is_ok() {
                    return;
                }
            }
            panic!("pg_stat_statements evicted entries during every one of eight attempts");
        }
    };
}

/// Calls per statement so far, for the message of a failed budget: it names what was added.
async fn calls(w: &World) -> BTreeMap<String, i64> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT left(regexp_replace(query, '\\s+', ' ', 'g'), 100), sum(calls)::bigint \
         FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
           AND toplevel AND query NOT LIKE '%pg_stat_statements%' GROUP BY 1",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("statements of this test database");
    rows.into_iter().collect()
}

/// One more run, with the statements of the lease and of the completion of its second step listed.
async fn itemised(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    session: &crate::support::engine::Session,
) -> String {
    w.start(workflow, &json!({})).await;
    let first = w.poll(session, 1).await.remove(0);
    w.complete(session, &first, &json!({})).await.expect("a");
    let mut text = String::new();
    // The second step (a completion with a next step) and the third (the completion that ends the run).
    for (step, ends) in [("b", false), ("c", true)] {
        let mut before = calls(w).await;
        let task = w.poll(session, 1).await.remove(0);
        for (title, done) in [("lease", false), ("completion", true)] {
            if done {
                w.complete(session, &task, &json!({})).await.expect(step);
            }
            let after = calls(w).await;
            let what = if ends && done {
                "completion that ends the run"
            } else {
                title
            };
            text.push_str(&format!("\n{step} {what}:\n"));
            for (query, count) in &after {
                let delta = count - before.get(query).copied().unwrap_or(0);
                if delta > 0 {
                    text.push_str(&format!("  {delta:>3}  {query}\n"));
                }
            }
            before = after;
        }
    }
    text
}

/// The statements of one more start of `workflow`, for a failing budget's message.
async fn itemised_start(w: &World, workflow: sagat_domain::definition::WorkflowId) -> String {
    let before = calls(w).await;
    w.start(workflow, &json!({})).await;
    let mut text = String::from("\nadmission:\n");
    for (query, count) in &calls(w).await {
        let delta = count - before.get(query).copied().unwrap_or(0);
        if delta > 0 {
            text.push_str(&format!("  {delta:>3}  {query}\n"));
        }
    }
    text
}

/// The cost of each of the six commands of one run of three chained activities, in order: lease
/// and completion of `a`, of `b` and of `c` (whose completion ends the run).
async fn one_run(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    session: &crate::support::engine::Session,
) -> Result<[Work; 6], Evicted> {
    let run = w.start(workflow, &json!({})).await;
    let mut costs = Vec::with_capacity(6);
    // The run is taken to its end whatever a window says: one left half done would hand its next
    // step, not this run's first, to the next measured run.
    let mut evicted = false;
    for _ in 0..3 {
        let before = reading(w).await;
        let mut tasks = w.poll(session, 1).await;
        match spent(w, before).await {
            Ok(cost) => costs.push(cost),
            Err(Evicted) => evicted = true,
        }
        assert_eq!(tasks.len(), 1, "the next step of the chain is leased");
        let task = tasks.remove(0);
        assert_eq!(
            task.run_id, run,
            "a step of this run, not one left by another"
        );
        let before = reading(w).await;
        w.complete(session, &task, &json!({}))
            .await
            .expect("completed");
        match spent(w, before).await {
            Ok(cost) => costs.push(cost),
            Err(Evicted) => evicted = true,
        }
    }
    if evicted {
        return Err(Evicted);
    }
    Ok(costs.try_into().expect("six commands"))
}

/// [`one_run`], on a new run each time an eviction spoils the last: a run is cheap and its windows
/// are short, where the whole scenario is long enough for an eviction to fall in it every time.
async fn one_run_measured(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    session: &crate::support::engine::Session,
) -> Result<[Work; 6], Evicted> {
    for _ in 0..40 {
        if let Ok(costs) = one_run(w, workflow, session).await {
            return Ok(costs);
        }
    }
    Err(Evicted)
}

const fn cost(statements: i64, transactions: i64, rows_written: i64) -> Work {
    Work {
        statements,
        transactions,
        rows_written,
    }
}

/// The budget. Lower a line in the change that earns it; raise one only with the reason beside it.
const LEASE: Work = cost(38, 2, 5);
const COMPLETION_WITH_A_NEXT_STEP: Work = cost(50, 1, 11);
const COMPLETION_THAT_ENDS_THE_RUN: Work = cost(47, 1, 9);

async fn a_lease_and_a_completion_cost_what_the_budget_says_once() -> Result<(), Evicted> {
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    let (workflow, _) = w
        .publish(document(
            "budget-chain",
            "a",
            vec![
                with(activity("a"), "next", json!("b")),
                with(activity("b"), "next", json!("c")),
                activity("c"),
            ],
        ))
        .await;
    let session = w.session(&["act.a", "act.b", "act.c"]).await;
    // The first run pays for what is cached afterwards (the definition, prepared statements).
    one_run_measured(&w, workflow, &session).await?;
    let first = one_run_measured(&w, workflow, &session).await?;
    let second = one_run_measured(&w, workflow, &session).await?;
    assert_eq!(first, second, "the counts of a quiet database do not vary");
    let [lease_a, done_a, lease_b, done_b, lease_c, done_c] = first;
    if (lease_a, lease_b, lease_c) != (LEASE, LEASE, LEASE)
        || (done_a, done_b) != (COMPLETION_WITH_A_NEXT_STEP, COMPLETION_WITH_A_NEXT_STEP)
        || done_c != COMPLETION_THAT_ENDS_THE_RUN
    {
        panic!(
            "the work of a lease or of a completion left its budget (lower the budget with the \
             change that earns it; raise it only with the reason beside it).\n\
             lease: {lease_a:?} {lease_b:?} {lease_c:?}, budget {LEASE:?}\n\
             completion with a next step: {done_a:?} {done_b:?}, budget {COMPLETION_WITH_A_NEXT_STEP:?}\n\
             completion that ends the run: {done_c:?}, budget {COMPLETION_THAT_ENDS_THE_RUN:?}\n\
             statements of the second and third steps of one more run:{}",
            itemised(&w, workflow, &session).await
        );
    }
    Ok(())
}

measured!(
    a_lease_and_a_completion_cost_what_the_budget_says,
    a_lease_and_a_completion_cost_what_the_budget_says_once
);

/// What admitting a run costs, for a definition that declares no gate (feature 022, NFR-022-007, T022-091): the line
/// every admission change is measured against. A change to `PgRunStore::create` or to the start path that adds a
/// statement, a transaction or a row for a definition without `concurrency` or `rateLimit` moves it.
const PLAIN_ADMISSION: Work = cost(33, 1, 11);

async fn a_plain_admission_costs_what_the_budget_says_once() -> Result<(), Evicted> {
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    let (workflow, _) = w
        .publish(document(
            "budget-admission",
            "a",
            vec![with(activity("a"), "next", json!("b")), activity("b")],
        ))
        .await;
    // The first starts pay for what is cached afterwards (the definition, prepared statements).
    for _ in 0..2 {
        w.start(workflow, &json!({})).await;
    }
    let mut costs = Vec::new();
    for _ in 0..2 {
        let before = reading(&w).await;
        w.start(workflow, &json!({})).await;
        costs.push(spent(&w, before).await?);
    }
    assert_eq!(
        costs[0], costs[1],
        "the counts of a quiet database do not vary"
    );
    if costs[0] != PLAIN_ADMISSION {
        let before = calls(&w).await;
        w.start(workflow, &json!({})).await;
        let after = calls(&w).await;
        let mut text = String::new();
        for (query, count) in &after {
            let delta = count - before.get(query).copied().unwrap_or(0);
            if delta > 0 {
                text.push_str(&format!("  {delta:>3}  {query}\n"));
            }
        }
        panic!(
            "the work of a plain admission left its budget (lower the budget with the change that earns it; raise it only \
             with the reason beside it): {:?}, budget {PLAIN_ADMISSION:?}\nstatements of one more admission:\n{text}",
            costs[0]
        );
    }
    Ok(())
}

measured!(
    a_plain_admission_costs_what_the_budget_says,
    a_plain_admission_costs_what_the_budget_says_once
);

/// Feature 022 (T022-076, NFR-022-007, FR-038-001): the hot path of a definition whose middle step declares `rateLimit`.
/// The claim of the rate-limited step is the plain claim (a gated step is granted before it is `READY`, ADR-0089, and its
/// lease instant is recorded by a pass, T022-074); the completion whose next step declares the gate pays for the
/// decision (the key versions, the window filed and locked, its count, the grant row); the other commands are the plain
/// lines. Measured before any call site of the gate in a completion (T022-096).
/// Measured 2026-11-19: the plain completion (50, 1, 11) and eight statements (the key versions in force, the window
/// filed and locked, the database clock after the lock, the window's count, the grant row, its journal event and the
/// extra quota check of the second event) and two rows (the grant and `rate_limit.granted`).
/// Raised 2026-11-19 by one statement (58 to 59): the namespace ceilings the decision applies, read once per round
/// (`concurrency_limit`, `flow_control_queue_timeout_ms`, `concurrency_live_keys`; G1199, FR-022-047). A rate limit
/// alone files no semaphore, so the live-keys check costs nothing here.
/// Raised 2026-11-20 by one statement (59 to 60): whether the step's window exists, so that a new window meets the
/// namespace's `rate_limit_live_keys` before it is created (T022-077, G1199's rate half, D022-064); the live count itself
/// is read only when the window would be new.
const COMPLETION_WITH_A_RATE_LIMITED_NEXT_STEP: Work = cost(60, 1, 13);
/// An admission whose entry step declares `rateLimit`: the plain admission and the decision of its entry.
/// Measured 2026-11-19: the plain admission (33, 1, 11) and the same eight statements and two rows; raised to nine
/// statements (41 to 42) the same day for the ceilings read (G1199).
/// Raised 2026-11-20 by two statements (42 to 44): the fleet activation read of the one admission point (as the gated
/// start's line) and the window's existence (`rate_limit_live_keys`, task T022-077, decision D022-064); plan, "Hot-path
/// budget of a gated definition".
const RATE_LIMITED_ENTRY_ADMISSION: Work = cost(44, 1, 13);

async fn a_rate_limited_step_costs_what_the_budget_says_once() -> Result<(), Evicted> {
    let w = crate::support::flow_control::gated_world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    // no key: the single key of the group, so that every run of the measure counts on one existing row
    let gate = json!({"limit": 10_000, "per": "hour"});
    let workflow = crate::support::flow_control::publish(
        &w,
        document(
            "budget-gated-chain",
            "a",
            vec![
                with(activity("a"), "next", json!("b")),
                with(
                    with(activity("b"), "rateLimit", gate.clone()),
                    "next",
                    json!("c"),
                ),
                activity("c"),
            ],
        ),
    )
    .await;
    let session = w.session(&["act.a", "act.b", "act.c"]).await;
    one_run_measured(&w, workflow, &session).await?;
    let first = one_run_measured(&w, workflow, &session).await?;
    let second = one_run_measured(&w, workflow, &session).await?;
    assert_eq!(first, second, "the counts of a quiet database do not vary");
    let [lease_a, done_a, lease_b, done_b, lease_c, done_c] = first;
    let entry = crate::support::flow_control::publish(
        &w,
        document(
            "budget-gated-entry",
            "a",
            vec![
                with(with(activity("a"), "rateLimit", gate), "next", json!("b")),
                activity("b"),
            ],
        ),
    )
    .await;
    for _ in 0..2 {
        w.start(entry, &json!({})).await;
    }
    let mut admissions = Vec::new();
    for _ in 0..2 {
        let before = reading(&w).await;
        w.start(entry, &json!({})).await;
        admissions.push(spent(&w, before).await?);
    }
    assert_eq!(
        admissions[0], admissions[1],
        "the counts of a quiet database do not vary"
    );
    if (lease_a, lease_b, lease_c) != (LEASE, LEASE, LEASE)
        || done_a != COMPLETION_WITH_A_RATE_LIMITED_NEXT_STEP
        || done_b != COMPLETION_WITH_A_NEXT_STEP
        || done_c != COMPLETION_THAT_ENDS_THE_RUN
        || admissions[0] != RATE_LIMITED_ENTRY_ADMISSION
    {
        panic!(
            "the work of a rate-limited path left its budget (lower the budget with the change that earns it; raise it \
             only with the reason beside it).\n\
             lease (a, the rate-limited b, c): {lease_a:?} {lease_b:?} {lease_c:?}, budget {LEASE:?} each\n\
             completion whose next step is rate-limited: {done_a:?}, budget {COMPLETION_WITH_A_RATE_LIMITED_NEXT_STEP:?}\n\
             completion of the rate-limited step: {done_b:?}, budget {COMPLETION_WITH_A_NEXT_STEP:?}\n\
             completion that ends the run: {done_c:?}, budget {COMPLETION_THAT_ENDS_THE_RUN:?}\n\
             admission of a rate-limited entry: {:?}, budget {RATE_LIMITED_ENTRY_ADMISSION:?}\n\
             statements of the second and third steps of one more run:{}",
            admissions[0],
            itemised(&w, workflow, &session).await
        );
    }
    Ok(())
}

measured!(
    a_rate_limited_step_costs_what_the_budget_says,
    a_rate_limited_step_costs_what_the_budget_says_once
);

/// An admission whose start declares `concurrency` and is granted at once (feature 022, T022-022; NFR-022-007: what a
/// declaring definition adds to its own admission is a budget line of the change that adds it). Measured 2026-11-20 on a
/// key that its last holder emptied (an empty key row is pruned, so each start files it again): the plain admission
/// (33, 1, 11) and ten statements, itemised against a plain start: the digest key versions in force, the ceilings, the
/// key's existence, the namespace's live keys (a key filed again), the key's insert and its lock, the decision's clock,
/// the key's count, the holder row and the `concurrency.acquired` journal row; and four for the projection: its own
/// `UPDATE runs` (the run's UPDATE does not name the column, so a process runs on a schema that predates it) and the
/// three statements of the content accounting that a write of a run row runs. Four rows more: holder, key, journal row,
/// projection.
/// Raised 2026-11-20 by one statement (47 to 48): the fleet activation read of the one admission point (`SELECT
/// capability FROM platform_requirements`, decision D022-057, commit 295b158b), found by diffing the statement lists of
/// commits 64b6f2d9 (47) and 34b8be41 (48); plan, "Hot-path budget of a gated definition".
const GATED_START_ADMISSION: Work = cost(48, 1, 15);
/// The same admission when the key is full and the run queues: the namespace's queued runs, the key's hint and the
/// waiter row in place of the holder row, and no entry released (no ready step, task hint or outbox row).
/// Raised 2026-11-20 by one statement (47 to 48), the same activation read.
const QUEUED_START_ADMISSION: Work = cost(48, 1, 12);
/// The completion that ends a run holding a start permit: the plain one (47, 1, 9) and eleven statements: seven, among
/// them the counter-slot lock (D022-050 item 8), the return of the permit under its key's lock (the holder's key, the
/// lock, the delete, the key's hint) and the `concurrency.released` journal row, and the projection's four as at
/// admission; four rows more.
const COMPLETION_THAT_ENDS_A_GATED_START: Work = cost(58, 1, 13);

async fn a_gated_start_costs_what_the_budget_says_once() -> Result<(), Evicted> {
    let w = crate::support::flow_control::gated_world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    let gated = |name: &str, limit: u32| {
        let mut doc = document(name, "a", vec![activity("a")]);
        // no key: the single key of the group, so that every run of the measure counts on one existing row
        doc["concurrency"] = json!({"limit": limit});
        doc
    };
    let roomy =
        crate::support::flow_control::publish(&w, gated("budget-gated-start", 10_000)).await;
    let session = w.session(&["act.a"]).await;
    let mut admissions = Vec::new();
    let mut completions = Vec::new();
    for _ in 0..3 {
        let before = reading(&w).await;
        w.start(roomy, &json!({})).await;
        admissions.push(spent(&w, before).await?);
        let task = w.one_task(&session).await;
        let before = reading(&w).await;
        w.complete(&session, &task, &json!({})).await.expect("done");
        completions.push(spent(&w, before).await?);
    }
    let full = crate::support::flow_control::publish(&w, gated("budget-queued-start", 1)).await;
    w.start(full, &json!({})).await;
    let mut queued = Vec::new();
    for _ in 0..3 {
        let before = reading(&w).await;
        w.start(full, &json!({})).await;
        queued.push(spent(&w, before).await?);
    }
    assert_eq!(
        (admissions[1], completions[1], queued[1]),
        (admissions[2], completions[2], queued[2]),
        "the counts of a quiet database do not vary"
    );
    if admissions[1] != GATED_START_ADMISSION
        || queued[1] != QUEUED_START_ADMISSION
        || completions[1] != COMPLETION_THAT_ENDS_A_GATED_START
    {
        panic!(
            "the work of a gated start left its budget (lower the budget with the change that earns it; raise it only with \
             the reason beside it).\n\
             granted admission: {:?}, budget {GATED_START_ADMISSION:?}\n\
             queued admission: {:?}, budget {QUEUED_START_ADMISSION:?}\n\
             completion that ends the run: {:?}, budget {COMPLETION_THAT_ENDS_A_GATED_START:?}\n\
             statements of one more queued start:{}",
            admissions[1],
            queued[1],
            completions[1],
            itemised_start(&w, full).await
        );
    }
    Ok(())
}

measured!(
    a_gated_start_costs_what_the_budget_says,
    a_gated_start_costs_what_the_budget_says_once
);

/// Sets the stored kind of the one step row of `run` at `node`, as the owner.
async fn store_kind(w: &World, run: sagat_domain::execution::RunId, node: &str, kind: &str) {
    let changed = sqlx::query(
        "UPDATE steps SET node_kind = $3 WHERE tenant_id = $1 AND run_id = $2 AND node = $4",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(kind)
    .bind(node)
    .execute(&w.db.pool)
    .await
    .expect("rewrite the stored kind")
    .rows_affected();
    assert_eq!(changed, 1, "the step row exists");
}

async fn stored_kind(w: &World, run: sagat_domain::execution::RunId, node: &str) -> String {
    sqlx::query_scalar(
        "SELECT node_kind FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(node)
    .fetch_one(&w.db.pool)
    .await
    .expect("stored kind")
}

async fn one_activity(w: &World, name: &str) -> sagat_domain::definition::WorkflowId {
    w.publish(document(name, "a", vec![activity("a")])).await.0
}

/// The kind of a step is written once. A row that holds another kind than its definition's is
/// refused when the step is next written, whether or not the write of the kind is skipped.
#[tokio::test]
async fn a_step_whose_stored_kind_is_not_its_definitions_is_refused() {
    let w = world().await;
    let workflow = one_activity(&w, "kind-mismatch").await;
    let run = w.start(workflow, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    store_kind(&w, run, "a", "timer").await;
    let leased = w
        .workers
        .poll(
            &session.principal,
            session.worker_id,
            &session.token,
            1,
            None,
        )
        .await;
    assert!(
        leased.as_ref().map_or(true, Vec::is_empty),
        "a step whose kind was changed is not leased: {leased:?}"
    );
    assert_eq!(
        stored_kind(&w, run, "a").await,
        "timer",
        "and is not repaired"
    );
}

/// A row written before kinds were stored is upgraded by the first write, once.
#[tokio::test]
async fn a_legacy_step_is_given_its_kind_by_the_first_write() {
    let w = world().await;
    let workflow = one_activity(&w, "kind-legacy").await;
    let run = w.start(workflow, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    store_kind(&w, run, "a", "legacyControl").await;
    let mut tasks = w.poll(&session, 1).await;
    assert_eq!(tasks.len(), 1, "a legacy step is leased");
    assert_eq!(stored_kind(&w, run, "a").await, "activity");
    w.complete(&session, &tasks.remove(0), &json!({}))
        .await
        .expect("completed");
}

/// A session that holds all it declared can take nothing more, whatever is ready: its poll ends
/// at the first candidate that shows it, and costs the same with one run waiting as with four.
async fn a_poll_of_a_full_session_does_not_walk_its_window_once() -> Result<(), Evicted> {
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    let workflow = one_activity(&w, "full-session").await;
    let session = w.session_as(&w.worker, &["act.a"], 1).await;
    w.start(workflow, &json!({})).await;
    w.start(workflow, &json!({})).await;
    assert_eq!(
        w.poll(&session, 1).await.len(),
        1,
        "the session is now full"
    );
    // Once more, so that both measurements below run with everything cached.
    assert!(w.poll(&session, 8).await.is_empty());
    let before = reading(&w).await;
    assert!(
        w.poll(&session, 8).await.is_empty(),
        "no room, nothing leased"
    );
    let one_waiting = spent(&w, before).await?;
    for _ in 0..3 {
        w.start(workflow, &json!({})).await;
    }
    let before = reading(&w).await;
    assert!(w.poll(&session, 8).await.is_empty(), "still no room");
    let four_waiting = spent(&w, before).await?;
    assert_eq!(
        one_waiting, four_waiting,
        "a full session pays for one candidate, not for every run that is ready"
    );
    Ok(())
}

measured!(
    a_poll_of_a_full_session_does_not_walk_its_window,
    a_poll_of_a_full_session_does_not_walk_its_window_once
);

/// A candidate is read by a poll before it is claimed. When its step has been taken in between,
/// opening it for a claim costs the chain lookup and one read of the run row, and loads nothing.
async fn a_candidate_with_no_ready_step_is_dropped_before_its_run_is_loaded_once()
-> Result<(), Evicted> {
    use sagat_application::runs::{Lock, RunStore};
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("the test PostgreSQL preloads pg_stat_statements");
    let workflow = one_activity(&w, "stale-candidate").await;
    let session = w.session(&["act.a"]).await;
    let taken = w.start(workflow, &json!({})).await;
    assert_eq!(
        w.poll(&session, 1).await.len(),
        1,
        "its only step is leased"
    );
    let ready = w.start(workflow, &json!({})).await;
    let tenant = w.admin.tenant_id();
    // Once before measuring, so that the statements are prepared.
    assert!(
        w.store
            .begin_claim(tenant, taken, &["act.a".to_owned()])
            .await
            .expect("opened")
            .is_none()
    );
    let before = reading(&w).await;
    let opened = w
        .store
        .begin_claim(tenant, taken, &["act.a".to_owned()])
        .await
        .expect("opened");
    let cost = spent(&w, before).await?;
    assert!(
        opened.is_none(),
        "a run with nothing ready is not opened for a claim"
    );
    assert_eq!(
        cost,
        cost_of(2, 1, 0),
        "the chain lookup and the run row, in one transaction"
    );
    assert!(
        w.store
            .begin_claim(tenant, ready, &["act.a".to_owned()])
            .await
            .expect("opened")
            .is_some(),
        "a run with a ready step is opened"
    );
    assert!(
        w.store
            .begin(tenant, taken, Lock::SkipLocked)
            .await
            .expect("opened")
            .is_some(),
        "and any run still opens for a command that is not a claim"
    );
    Ok(())
}

measured!(
    a_candidate_with_no_ready_step_is_dropped_before_its_run_is_loaded,
    a_candidate_with_no_ready_step_is_dropped_before_its_run_is_loaded_once
);

const fn cost_of(statements: i64, transactions: i64, rows_written: i64) -> Work {
    cost(statements, transactions, rows_written)
}

/// The same through two polls. Worker A reads two candidates and is held while it leases the
/// first; worker B leases the second meanwhile. When A goes on, its second candidate is stale:
/// it drops it without opening the run. Three runs opened would mean the claim loaded it as
/// before. Read from the claim's own counters, which no eviction of statement statistics touches.
#[test]
fn a_poll_does_not_load_a_run_another_worker_emptied_meanwhile() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (telemetry, recorder) =
        sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        runtime.block_on(emptied_meanwhile(&telemetry));
    });
}

/// `worker_claim_candidates_total{stage}` so far, zero before its first increment.
fn claim_stage(telemetry: &sagat_telemetry::Metrics, name: &str) -> u64 {
    telemetry
        .render()
        .lines()
        .find(|line| {
            line.starts_with("worker_claim_candidates_total{")
                && line.contains(&format!("stage=\"{name}\""))
        })
        .and_then(|line| line.rsplit(' ').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

async fn emptied_meanwhile(telemetry: &sagat_telemetry::Metrics) {
    let w = world().await;
    let workflow = one_activity(&w, "emptied-meanwhile").await;
    let a = w.session(&["act.a"]).await;
    let b = w.session(&["act.a"]).await;
    // Both polls once on a run of their own, so that nothing below is a first execution.
    for session in [&a, &b] {
        w.start(workflow, &json!({})).await;
        assert_eq!(w.poll(session, 1).await.len(), 1);
    }
    // The barrier: A's lease, and only A's, waits for this lock as it is written.
    sqlx::query("CREATE TABLE held_worker (worker text NOT NULL)")
        .execute(&w.db.pool)
        .await
        .expect("barrier target");
    sqlx::query("INSERT INTO held_worker VALUES ($1)")
        .bind(a.worker_id.to_string())
        .execute(&w.db.pool)
        .await
        .expect("A is the one held");
    sqlx::query("GRANT SELECT ON held_worker TO PUBLIC")
        .execute(&w.db.pool)
        .await
        .expect("readable by the runtime role");
    sqlx::query(
        "CREATE FUNCTION hold_a_lease() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
           IF OLD.status = 'READY' AND NEW.status = 'LEASED' \
              AND NEW.leased_by = (SELECT worker FROM held_worker) THEN \
             PERFORM pg_advisory_xact_lock_shared(910380006); END IF; RETURN NEW; END $$",
    )
    .execute(&w.db.pool)
    .await
    .expect("barrier function");
    sqlx::query("CREATE TRIGGER hold_a_lease BEFORE UPDATE ON steps FOR EACH ROW EXECUTE FUNCTION hold_a_lease()")
        .execute(&w.db.pool)
        .await
        .expect("barrier trigger");
    let mut barrier = w.db.pool.begin().await.expect("barrier");
    sqlx::query("SELECT pg_advisory_xact_lock(910380006)")
        .execute(&mut *barrier)
        .await
        .expect("hold the barrier");
    w.start(workflow, &json!({})).await;
    w.start(workflow, &json!({})).await;
    let (probed, opened) = (
        claim_stage(telemetry, "probed"),
        claim_stage(telemetry, "opened"),
    );
    let poll_a = w.workers.poll(&a.principal, a.worker_id, &a.token, 2, None);
    tokio::pin!(poll_a);
    let held =
        crate::support::eventually("A is held while it leases its first candidate", || async {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND NOT granted \
               AND objid = 910380006 AND database = (SELECT oid FROM pg_database \
               WHERE datname = current_database()))",
            )
            .fetch_one(&w.db.pool)
            .await
            .expect("observe the barrier");
            waiting.then_some(())
        });
    tokio::select! {
        result = &mut poll_a => panic!("A passed the barrier: {result:?}"),
        () = held => {}
    }
    let taken_by_b = w.poll(&b, 1).await;
    assert_eq!(
        taken_by_b.len(),
        1,
        "B skips the run A holds and leases the other"
    );
    barrier.commit().await.expect("release A");
    let taken_by_a = poll_a.await.expect("A's poll");
    assert_eq!(
        taken_by_a.len(),
        1,
        "A leases the run it held and nothing else"
    );
    assert_ne!(taken_by_a[0].run_id, taken_by_b[0].run_id);
    // A probes its two candidates. B, in flight with A, starts past what A asked for and probes the
    // other run only (`window_order.rs`), where tried in A's order it would probe A's held run first.
    assert_eq!(claim_stage(telemetry, "probed") - probed, 3);
    assert_eq!(
        claim_stage(telemetry, "opened") - opened,
        2,
        "one run opened for each lease; the candidate B emptied is dropped unopened"
    );
}

/// A run whose only ready step is for an activity the session does not serve is dropped for that
/// session's claim as unloaded as one with nothing ready, and still opens for a session that serves it.
#[tokio::test]
async fn a_run_whose_ready_step_is_for_another_activity_is_dropped_for_this_session() {
    use sagat_application::runs::RunStore;
    let w = world().await;
    let (workflow, _) = w
        .publish(document(
            "other-activity",
            "fork",
            vec![
                json!({"id":"fork","type":"parallel","branches":["a","b"],"join":"all"}),
                activity("a"),
                activity("b"),
            ],
        ))
        .await;
    let run = w.start(workflow, &json!({})).await;
    let only_a = w.session(&["act.a"]).await;
    assert_eq!(
        w.poll(&only_a, 2).await.len(),
        1,
        "a is leased, b stays ready"
    );
    assert!(
        w.store
            .begin_claim(w.admin.tenant_id(), run, &["act.a".to_owned()])
            .await
            .expect("opened")
            .is_none(),
        "a ready step of another activity does not open the run for this session"
    );
    assert!(
        w.store
            .begin_claim(w.admin.tenant_id(), run, &["act.b".to_owned()])
            .await
            .expect("opened")
            .is_some(),
        "and does for a session that serves it"
    );
    assert!(
        w.poll(&only_a, 2).await.is_empty(),
        "and nothing of it is leased to this worker"
    );
}

/// A step made ready by a transaction that has not committed is not there yet for a claim, and
/// is there once it has.
#[tokio::test]
async fn a_claim_sees_a_ready_step_only_once_it_is_committed() {
    use sagat_application::runs::RunStore;
    let w = world().await;
    let workflow = one_activity(&w, "ready-uncommitted").await;
    let session = w.session(&["act.a"]).await;
    let run = w.start(workflow, &json!({})).await;
    assert_eq!(
        w.poll(&session, 1).await.len(),
        1,
        "its only step is leased"
    );
    let tenant = w.admin.tenant_id();
    let mut writer = w.db.pool.begin().await.expect("writer");
    sqlx::query("UPDATE steps SET status = 'READY', lease_token_hash = NULL, lease_expires_at = NULL, leased_by = NULL, ready_at = clock_timestamp() WHERE tenant_id = $1 AND run_id = $2")
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .execute(&mut *writer)
        .await
        .expect("make the step ready, uncommitted");
    assert!(
        w.store
            .begin_claim(tenant, run, &["act.a".to_owned()])
            .await
            .expect("opened")
            .is_none(),
        "not ready for anyone else yet"
    );
    writer.commit().await.expect("committed");
    assert!(
        w.store
            .begin_claim(tenant, run, &["act.a".to_owned()])
            .await
            .expect("opened")
            .is_some(),
        "ready once committed"
    );
}

/// The loads a command skips are skipped because the definition excludes their state. With the
/// check on, a run that has a row of an excluded kind is refused instead of read past.
#[tokio::test]
async fn a_run_with_state_its_definition_excludes_is_refused_when_the_check_is_on() {
    use sagat_application::runs::{Lock, RunStore};
    let w = world().await;
    let workflow = one_activity(&w, "excluded-state").await;
    let run = w.start(workflow, &json!({})).await;
    let tenant = w.admin.tenant_id();
    let checking = sagat_infra_postgres::PgRunStore::new(
        w.execution_pool.clone(),
        sagat_application::execution::DefinitionCache::new(
            std::sync::Arc::new(sagat_infra_ir::JsonIrCodec),
            8,
        ),
    )
    .with_skipped_load_verification(true);
    assert!(
        checking
            .begin(tenant, run, Lock::Wait)
            .await
            .expect("opened")
            .is_some(),
        "a run with nothing excluded passes the check"
    );
    sqlx::query(
        "INSERT INTO run_sagas (tenant_id, run_id, node, phase, updated_at) \
         VALUES ($1, $2, 'ghost', 'FORWARD', clock_timestamp())",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("a saga row no node of the definition could have made");
    let refused = checking
        .begin(tenant, run, Lock::Wait)
        .await
        .expect_err("the check refuses the run");
    assert!(
        format!("{refused:?}").contains("a projection its definition excludes"),
        "refused for the excluded projection, not for something else: {refused:?}"
    );
    assert!(
        w.store.begin(tenant, run, Lock::Wait).await.is_ok(),
        "a store that does not check reads past it"
    );
    let verified = w.store.verify_run(tenant, run).await;
    assert!(
        verified
            .as_ref()
            .is_err_and(|why| why.contains("a projection its definition excludes")),
        "and the integrity check still reports it: {verified:?}"
    );
}

/// The counters of a poll say what it did: three runs are ready, a worker asks for two tasks, so
/// three candidates are fetched and two are tried, opened and leased (FR-038-007).
#[test]
fn a_poll_counts_what_it_fetched_probed_opened_and_leased() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (telemetry, recorder) =
        sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
    metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let w = world().await;
            let workflow = one_activity(&w, "counted-poll").await;
            let session = w.session(&["act.a"]).await;
            for _ in 0..3 {
                w.start(workflow, &json!({})).await;
            }
            assert_eq!(w.poll(&session, 2).await.len(), 2);
        });
    });
    let rendered = telemetry.render();
    let stage = |name: &str| -> Option<u64> {
        rendered
            .lines()
            .find(|line| {
                line.starts_with("worker_claim_candidates_total{")
                    && line.contains(&format!("stage=\"{name}\""))
            })
            .and_then(|line| line.rsplit(' ').next())
            .and_then(|value| value.parse().ok())
    };
    assert_eq!(
        (
            stage("fetched"),
            stage("probed"),
            stage("opened"),
            stage("leased")
        ),
        (Some(3), Some(2), Some(2), Some(2)),
        "{rendered}"
    );
    assert!(
        rendered
            .lines()
            .filter(|line| line.starts_with("worker_claim_candidates_total{"))
            .all(|line| line.matches('=').count() == 1),
        "the stage is the only label: {rendered}"
    );
}

/// What one start of `workflow` with `input` costs, after a first start warmed the definition and the compiled schema.
async fn a_start_once(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    input: &serde_json::Value,
) -> Result<Work, Evicted> {
    w.start(workflow, input).await;
    let before = reading(w).await;
    w.start(workflow, input).await;
    spent(w, before).await
}

/// SC-043-003, design 043 in `enforce` mode: a start of a version with an `inputSchema` costs exactly the statements,
/// the transactions and the rows written of the same shape without one (NFR-043-PERF): the schema travels with the
/// parsed definition and the check runs before the creation transaction, writing nothing.
async fn a_checked_start_costs_what_an_unchecked_one_does_once() -> Result<(), Evicted> {
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("statement accounting");
    let (plain, _) = w
        .publish(document("budget-plain", "a", vec![activity("a")]))
        .await;
    let mut checked = document("budget-checked", "a", vec![activity("a")]);
    checked["inputSchema"] = json!({
        "type": "object",
        "required": ["orderId"],
        "properties": {"orderId": {"type": "string", "maxLength": 64}}
    });
    let (checked, _) = w.publish(checked).await;
    let input = json!({"orderId": "o-1"});
    let without = a_start_once(&w, plain, &input).await?;
    let with = a_start_once(&w, checked, &input).await?;
    assert_eq!(
        with, without,
        "a start with an inputSchema costs what one without does"
    );
    Ok(())
}

measured!(
    /// SC-043-003 for a start.
    a_checked_start_costs_what_an_unchecked_one_does,
    a_checked_start_costs_what_an_unchecked_one_does_once
);

/// The calls each statement made between two [`calls`] listings, one line per statement that ran, for the message of a
/// failed comparison: it names the statement one window ran and the other did not.
fn listed(before: &BTreeMap<String, i64>, after: &BTreeMap<String, i64>) -> String {
    let mut text = String::new();
    for (query, count) in after {
        let delta = count - before.get(query).copied().unwrap_or(0);
        if delta > 0 {
            text.push_str(&format!("  {delta:>3}  {query}\n"));
        }
    }
    text
}

/// What one triggered start costs: an event published, then one dispatch round that starts the run of its trigger.
async fn a_triggered_start_once(
    w: &World,
    dispatcher: &crate::support::engine::EventLoop,
    kind: &str,
) -> Result<(Work, String), Evicted> {
    let publish = |id: String| async move {
        w.events
            .publish(
                &w.admin,
                sagat_application::events::PublishRequest {
                    namespace: "payments".to_owned(),
                    event_id: Some(sagat_domain::events::EventId::parse(&id).expect("id")),
                    event_type: sagat_domain::workflow::EventType::parse(kind).expect("type"),
                    subject: None,
                    correlation_key: None,
                    occurred_at: None,
                    data: br#"{"orderId":"o-1"}"#.to_vec(),
                    dedupe_ttl: None,
                },
                &crate::support::engine::ctx(),
            )
            .await
            .expect("published");
    };
    publish(format!("{kind}-warm")).await;
    dispatcher.tick().await;
    publish(format!("{kind}-measured")).await;
    let listed_before = calls(w).await;
    let before = reading(w).await;
    let round = dispatcher.tick().await;
    assert_eq!(round.runs_started, 1, "{kind}");
    let cost = spent(w, before).await?;
    Ok((cost, listed(&listed_before, &calls(w).await)))
}

/// SC-043-003 for a triggered start: the envelope checked against an `inputSchema` costs nothing more.
async fn a_checked_triggered_start_costs_what_an_unchecked_one_does_once() -> Result<(), Evicted> {
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("statement accounting");
    let mut plain = document("budget-trigger-plain", "a", vec![activity("a")]);
    plain["triggers"] = json!([{"kind": "event", "eventType": "budget.plain"}]);
    w.publish(plain).await;
    let mut checked = document("budget-trigger-checked", "a", vec![activity("a")]);
    checked["triggers"] = json!([{"kind": "event", "eventType": "budget.checked"}]);
    checked["inputSchema"] = json!({
        "type": "object",
        "required": ["data"],
        "properties": {"data": {"type": "object", "required": ["orderId"]}}
    });
    w.publish(checked).await;
    // A round reads whether the cron triggers were handed over to the schedule pass (`SELECT state FROM
    // schedule_trigger_handover`) only when what it read last is two seconds old (`HandoverCache::LEGACY_BOUND`, D2072),
    // so whether a window holds that read depends on how long the publications and the rounds before it took: on a loaded
    // machine one window counted 52 statements and the other 51. With a zero bound every round reads it, once, and the
    // two windows are compared statement for statement.
    let dispatcher = &w
        .dispatcher_with(sagat_scheduler::EventDispatchSettings::default())
        .with_handover_bound(std::time::Duration::ZERO);
    let (without, without_listed) = a_triggered_start_once(&w, dispatcher, "budget.plain").await?;
    let (with, with_listed) = a_triggered_start_once(&w, dispatcher, "budget.checked").await?;
    assert_eq!(
        with, without,
        "a triggered start with an inputSchema costs what one without does\n\
         statements of the round with one:\n{with_listed}statements of the round without:\n{without_listed}"
    );
    Ok(())
}

measured!(
    /// SC-043-003 for a triggered start.
    a_checked_triggered_start_costs_what_an_unchecked_one_does,
    a_checked_triggered_start_costs_what_an_unchecked_one_does_once
);

/// What starting one child costs: a parent started, then one scheduler pass that creates its child.
async fn a_child_start_once(
    w: &World,
    parent: sagat_domain::definition::WorkflowId,
) -> Result<Work, Evicted> {
    w.start(parent, &json!({"orderId": "o-1"})).await;
    w.scheduler.tick().await; // warm: the definitions and the compiled schema
    w.start(parent, &json!({"orderId": "o-1"})).await;
    let before = reading(w).await;
    let report = w.scheduler.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    spent(w, before).await
}

/// SC-043-003 for a child start: a child whose version has an `inputSchema` costs what one without does.
async fn a_checked_child_start_costs_what_an_unchecked_one_does_once() -> Result<(), Evicted> {
    use crate::support::engine::child_node;

    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("statement accounting");
    w.publish(document("budget-kid-plain", "a", vec![activity("a")]))
        .await;
    let mut checked = document("budget-kid-checked", "a", vec![activity("a")]);
    checked["inputSchema"] = json!({"type": "object", "required": ["orderId"], "properties": {"orderId": {"type": "string"}}});
    w.publish(checked).await;
    let (plain, _) = w
        .publish(document(
            "budget-mother-plain",
            "c",
            vec![child_node("c", "budget-kid-plain", None)],
        ))
        .await;
    let (with_schema, _) = w
        .publish(document(
            "budget-mother-checked",
            "c",
            vec![child_node("c", "budget-kid-checked", None)],
        ))
        .await;
    let without = a_child_start_once(&w, plain).await?;
    let with = a_child_start_once(&w, with_schema).await?;
    assert_eq!(
        with, without,
        "a child start with an inputSchema costs what one without does"
    );
    Ok(())
}

measured!(
    /// SC-043-003 for a child start.
    a_checked_child_start_costs_what_an_unchecked_one_does,
    a_checked_child_start_costs_what_an_unchecked_one_does_once
);

/// A scheduler that holds every shard, so that its pass hands continuations off, and checks their input in `enforce`.
async fn a_chain_scheduler(w: &World) -> crate::support::engine::Sched {
    let placement = sagat_scheduler::Placement::new(format!("budget-{}", uuid::Uuid::now_v7()));
    sagat_scheduler::Keeper::new(
        w.store.clone(),
        placement.clone(),
        sagat_scheduler::KeeperSettings {
            ttl: std::time::Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    sagat_scheduler::Scheduler::new(
        w.store.clone(),
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_scheduler::SchedulerSettings::default(),
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        sagat_application::run_input::InputValidationMode::Enforce,
    ))
    .with_children(w.cache.clone())
    .with_placement(placement)
}

/// A chain of `name` whose activity is `name.work`, and whose continuation starts the next segment with `input.next`.
fn a_rolling_chain(name: &str, schema: Option<serde_json::Value>) -> serde_json::Value {
    let mut doc = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": format!("{name}.work"), "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    });
    if let Some(schema) = schema {
        doc["inputSchema"] = schema;
    }
    doc
}

/// What one continuation costs: a chain whose activity is done, then one scheduler pass that hands it off. A first chain
/// of the same workflow warms the definition and the compiled schema; its successor's task is left leased, unfinished.
async fn a_continuation_once(
    w: &World,
    scheduler: &crate::support::engine::Sched,
    workflow: sagat_domain::definition::WorkflowId,
    name: &str,
) -> Result<Work, Evicted> {
    let activity = format!("{name}.work");
    let session = w.session(&[activity.as_str()]).await;
    let input = json!({"segment": 1, "next": {"segment": 2}});
    let finish = |chain: sagat_domain::execution::RunId| {
        let session = &session;
        async move {
            let mut polls = 0;
            let task = loop {
                let tasks = w.poll(session, 8).await;
                if let Some(task) = tasks.into_iter().find(|task| task.run_id == chain) {
                    break task;
                }
                polls += 1;
                assert!(polls < 100, "the chain's task never came");
            };
            w.complete(session, &task, &json!({}))
                .await
                .expect("the activity completes");
        }
    };
    let warm = w.start(workflow, &input).await;
    finish(warm).await;
    assert_eq!(
        scheduler.tick().await.continuations_advanced,
        1,
        "{name}: warm"
    );
    let measured = w.start(workflow, &input).await;
    finish(measured).await;
    let before = reading(w).await;
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 1, "{name}: {report:?}");
    spent(w, before).await
}

/// SC-043-003 for a continuation (T043-18): a successor whose version has an `inputSchema` costs, in statements,
/// transactions and rows written, what one without does; the check runs between the two transactions of the hand-off
/// and writes nothing.
async fn a_checked_continuation_costs_what_an_unchecked_one_does_once() -> Result<(), Evicted> {
    let w = world().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&w.db.pool)
        .await
        .expect("statement accounting");
    let (plain, _) = w.publish(a_rolling_chain("budgetplain", None)).await;
    let (checked, _) = w
        .publish(a_rolling_chain(
            "budgetchecked",
            Some(json!({
                "type": "object",
                "required": ["segment"],
                "properties": {"segment": {"type": "integer"}}
            })),
        ))
        .await;
    let scheduler = a_chain_scheduler(&w).await;
    let without = a_continuation_once(&w, &scheduler, plain, "budgetplain").await?;
    let with = a_continuation_once(&w, &scheduler, checked, "budgetchecked").await?;
    assert_eq!(
        with, without,
        "a continuation with an inputSchema costs what one without does"
    );
    Ok(())
}

measured!(
    /// SC-043-003 for a continuation.
    a_checked_continuation_costs_what_an_unchecked_one_does,
    a_checked_continuation_costs_what_an_unchecked_one_does_once
);

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
    let mut before = calls(w).await;
    let task = w.poll(session, 1).await.remove(0);
    for (title, done) in [("lease", false), ("completion", true)] {
        if done {
            w.complete(session, &task, &json!({})).await.expect("b");
        }
        let after = calls(w).await;
        text.push_str(&format!("\n{title}:\n"));
        for (query, count) in &after {
            let delta = count - before.get(query).copied().unwrap_or(0);
            if delta > 0 {
                text.push_str(&format!("  {delta:>3}  {query}\n"));
            }
        }
        before = after;
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
             statements of the second step of one more run:{}",
            itemised(&w, workflow, &session).await
        );
    }
    Ok(())
}

measured!(
    a_lease_and_a_completion_cost_what_the_budget_says,
    a_lease_and_a_completion_cost_what_the_budget_says_once
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
            .begin(tenant, taken, Lock::Claim)
            .await
            .expect("opened")
            .is_none()
    );
    let before = reading(&w).await;
    let opened = w
        .store
        .begin(tenant, taken, Lock::Claim)
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
            .begin(tenant, ready, Lock::Claim)
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
    // A probes its two candidates and B its two (the run A holds, then the other).
    assert_eq!(claim_stage(telemetry, "probed") - probed, 4);
    assert_eq!(
        claim_stage(telemetry, "opened") - opened,
        2,
        "one run opened for each lease; the candidate B emptied is dropped unopened"
    );
}

/// What the drop does not cover: any ready step keeps the run open, also one that is not for
/// this worker. The run is loaded and nothing is leased, as before.
#[tokio::test]
async fn a_run_whose_ready_step_is_for_another_activity_is_still_opened() {
    use sagat_application::runs::{Lock, RunStore};
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
            .begin(w.admin.tenant_id(), run, Lock::Claim)
            .await
            .expect("opened")
            .is_some(),
        "a ready step of another activity keeps the run open for a claim"
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
    use sagat_application::runs::{Lock, RunStore};
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
            .begin(tenant, run, Lock::Claim)
            .await
            .expect("opened")
            .is_none(),
        "not ready for anyone else yet"
    );
    writer.commit().await.expect("committed");
    assert!(
        w.store
            .begin(tenant, run, Lock::Claim)
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

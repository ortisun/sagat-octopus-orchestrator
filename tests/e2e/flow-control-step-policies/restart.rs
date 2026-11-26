//! T022-059, narrowed for the release 0.3.0 slice to queued runs and waiting steps (NFR-022-001): the real control plane
//! and engine executables are killed with `SIGKILL` while a decision of flow control is in flight, and started again.
//! Runs queued on a concurrency key and on a rate-limit key, and steps waiting for a step permit or a rate slot, survive;
//! after the restart they are granted in admission order (D022-073), none twice and none lost; a waiter whose deadline
//! passed during the outage times out exactly once; and the keys' counters, rows and journals agree
//! (`support::flow_control::key_consistency`, invariants 1, 2, 8, 9 and 12 of `data-model.md`) for every run, each of
//! which `verify_run` replays.
//!
//! A kill lands inside a decision deterministically: the test holds a row lock (a key's row, or a run's row) that the
//! decision needs, waits until a backend of the process waits for it (`pg_locks`), kills the process, then lets the lock
//! go and waits until every backend of the killed process has ended. The killed decision had begun its transaction and
//! could not commit it; what the test then reads is what a restart finds. No wait of these tests stands in for an order:
//! each polls a state the database shows, with a bound that names it.

use std::time::Duration;

use reqwest::Method;
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_domain::execution::RunId;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::processes::{Process, STATE_LIMIT, until};
use crate::support::api::Options;
use crate::support::engine::{activity, document};
use crate::support::flow_control::{
    key_consistency, set_gate_deadline, set_start_deadline, until_past,
};
use crate::support::worker::{HUMAN, NAMESPACE, Platform, TENANT, Worker, platform_with};

/// The deadline a waiter is given during an outage: it passes while no engine runs.
const OUTAGE_DEADLINE_US: i64 = 300_000;
const SECOND_US: i64 = 1_000_000;

// --- what the tests are made of -----------------------------------------------------------------------------------

/// A platform with flow control on (both switches, the key ring, both capabilities activated for the fleet) whose REST
/// calls go to a real control plane, and the directory of the processes' logs.
async fn stack() -> (Platform, Process, tempfile::TempDir) {
    let mut p = platform_with(Options {
        flow_control: true,
        ..Options::default()
    })
    .await;
    let dir = tempfile::tempdir().expect("a directory for the processes' logs");
    let control_plane = Process::control_plane(&p, dir.path(), "control-plane-1").await;
    p.api.base.clone_from(&control_plane.url);
    (p, control_plane, dir)
}

/// A one-step workflow `name` (activity `act.<node>`) whose start is gated by `field` (`concurrency` or `rateLimit`).
fn start_gated(name: &str, node: &str, field: &str, policy: Value) -> Value {
    let mut doc = document(name, node, vec![activity(node)]);
    doc[field] = policy;
    doc
}

/// A one-step workflow `name` whose only step is gated by `field`.
fn step_gated(name: &str, node: &str, field: &str, policy: Value) -> Value {
    let mut step = activity(node);
    step[field] = policy;
    document(name, node, vec![step])
}

fn run_id(run: &str) -> RunId {
    RunId::new(Uuid::parse_str(run).expect("a run id"))
}

/// `POST /runs` with the idempotency key `key`, as `alice`, to the control plane the platform speaks to now.
fn start_request(
    p: &Platform,
    workflow: &str,
    input: &Value,
    key: &str,
) -> reqwest::RequestBuilder {
    p.api
        .as_user(Method::POST, "/runs", HUMAN, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", key)
        .json(&json!({"workflowId": workflow, "input": input}))
}

/// `POST /runs/{run}/operations` `CANCEL` with the idempotency key `key`.
fn cancel_request(p: &Platform, run: &str, key: &str) -> reqwest::RequestBuilder {
    p.api
        .as_user(
            Method::POST,
            &format!("/runs/{run}/operations"),
            HUMAN,
            TENANT,
        )
        .header("x-octopus-namespace", NAMESPACE)
        .header("idempotency-key", key)
        .json(&json!({"action": "CANCEL", "reason": "the restart test cancels it"}))
}

/// Sends `request` and returns its run id, which it must have accepted (`202`).
async fn accepted(request: reqwest::RequestBuilder) -> String {
    let response = request.send().await.expect("the control plane answers");
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    assert_eq!(status, 202, "{body}");
    body["runId"].as_str().expect("a run id").to_owned()
}

/// Sends the operation `request`, which must have been accepted (`202`).
async fn operated(request: reqwest::RequestBuilder) {
    let response = request.send().await.expect("the control plane answers");
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    assert_eq!(status, 202, "{body}");
}

async fn start(p: &Platform, workflow: &str, input: Value) -> String {
    accepted(start_request(
        p,
        workflow,
        &input,
        &format!("start-{}", Uuid::now_v7()),
    ))
    .await
}

/// The status and stop code of `run`, read from the database (a control plane may be down).
async fn state(p: &Platform, run: &str) -> (String, Option<String>) {
    sqlx::query_as("SELECT status, stop_code FROM runs WHERE id = $1")
        .bind(Uuid::parse_str(run).expect("a run id"))
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the run")
}

/// How many `event`s the journal of `run` holds.
async fn count(p: &Platform, run: &str, event: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id = $1 AND event_type = $2")
        .bind(Uuid::parse_str(run).expect("a run id"))
        .bind(event)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the journal")
}

/// The journal position of `run`'s first `event` on the database's order of commits: the instant it was journaled, in
/// microseconds.
async fn journaled_us(p: &Platform, run: &str, event: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT (extract(epoch FROM min(occurred_at)) * 1000000)::bigint FROM run_events \
          WHERE run_id = $1 AND event_type = $2",
    )
    .bind(Uuid::parse_str(run).expect("a run id"))
    .bind(event)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the journal")
}

/// The instant of `run`'s one `rate_limit.granted`, as the grant recorded it (`granted_at_us`).
async fn granted_us(p: &Platform, run: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT (detail ->> 'granted_at_us')::bigint FROM run_events \
          WHERE run_id = $1 AND event_type = 'rate_limit.granted'",
    )
    .bind(Uuid::parse_str(run).expect("a run id"))
    .fetch_one(&p.api.db.pool)
    .await
    .expect("one grant")
}

/// The statement that locks the row of the key a run holds, was granted by or waits on: a rate window when `windows`,
/// else a concurrency key. A statement of constants; `$1` is the run.
fn key_row(windows: bool) -> &'static str {
    if windows {
        "SELECT 1 FROM rate_limit_windows k \
          WHERE (k.tenant_id, k.namespace_id, k.pool, k.limit_scope, k.group_name, k.key_digest) IN ( \
                SELECT tenant_id, namespace_id, pool, limit_scope, group_name, key_digest FROM rate_limit_waiters \
                 WHERE run_id = $1 \
                UNION SELECT tenant_id, namespace_id, pool, limit_scope, group_name, key_digest FROM rate_limit_grants \
                 WHERE run_id = $1) \
          FOR UPDATE OF k"
    } else {
        "SELECT 1 FROM concurrency_semaphores k \
          WHERE (k.tenant_id, k.namespace_id, k.pool, k.permit_scope, k.group_name, k.key_digest) IN ( \
                SELECT tenant_id, namespace_id, pool, permit_scope, group_name, key_digest FROM concurrency_waiters \
                 WHERE run_id = $1 \
                UNION SELECT tenant_id, namespace_id, pool, holder_scope, group_name, key_digest FROM concurrency_holders \
                 WHERE run_id = $1) \
          FOR UPDATE OF k"
    }
}

const RUN_ROW: &str = "SELECT 1 FROM runs WHERE id = $1 FOR UPDATE";

/// Takes, in a transaction of the test's own, the one row each `(statement, run)` of `rows` locks, and keeps them until
/// the transaction is rolled back: the barrier a decision of a process waits behind.
async fn hold(
    p: &Platform,
    rows: &[(&'static str, &str)],
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = p.api.db.pool.begin().await.expect("a transaction");
    for (statement, run) in rows {
        let locked = sqlx::query(*statement)
            .bind(Uuid::parse_str(run).expect("a run id"))
            .fetch_all(&mut *held)
            .await
            .expect("the row locked");
        assert_eq!(locked.len(), 1, "one row locked for {run}: {statement}");
    }
    held
}

/// Every run's journal replays to its projections (`verify_run`), and the keys of the tenant agree with their rows and
/// with the gates of the runs that hold or wait on them; no key row is left with no holder, grant or waiter.
async fn assert_consistent(p: &Platform, runs: &[&String]) {
    let named = key_consistency(&p.api.db.pool, p.tenant)
        .await
        .unwrap_or_else(|why| {
            panic!("the flow-control keys are inconsistent after the restarts: {why}")
        });
    for run in runs.iter().map(|run| run_id(run)).chain(named) {
        p.api
            .executions
            .verify_run(p.tenant, run)
            .await
            .unwrap_or_else(|why| panic!("run {run} is inconsistent after the restarts: {why}"));
    }
}

/// Points `worker` at the worker API of `engine`, keeping its session (a lease survives a restart of the engine).
async fn reconnect(worker: &mut Worker<'_>, engine: &Process) {
    worker.client = WorkerServiceClient::connect(engine.url.clone())
        .await
        .expect("connect to the restarted engine")
        .max_decoding_message_size(16 * 1024 * 1024);
}

/// Polls `worker` and completes every task it is given until every run of `runs` has ended; returns the runs whose tasks
/// it completed, in the order it was given them.
async fn work_until_ended(p: &Platform, worker: &mut Worker<'_>, runs: &[&String]) -> Vec<String> {
    let mut done = Vec::new();
    let ids: Vec<Uuid> = runs
        .iter()
        .map(|run| Uuid::parse_str(run).expect("a run id"))
        .collect();
    let deadline = std::time::Instant::now() + STATE_LIMIT;
    loop {
        for task in worker.poll(8).await {
            worker
                .complete(&task, &json!({"done": true}))
                .await
                .expect("complete");
            done.push(task.run_id.clone());
        }
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM runs WHERE id = ANY($1) \
               AND status NOT IN ('SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT')",
        )
        .bind(&ids)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the runs");
        if open == 0 {
            return done;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{open} runs still open after {STATE_LIMIT:?}; completed so far: {done:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The runs in the order their first `event` was journaled.
async fn in_journal_order(p: &Platform, runs: &[&String], event: &str) -> Vec<String> {
    let mut at = Vec::new();
    for run in runs {
        at.push((journaled_us(p, run, event).await, (*run).clone()));
    }
    at.sort();
    at.into_iter().map(|(_, run)| run).collect()
}

// --- the engine is killed ------------------------------------------------------------------------------------------

/// Runs queued on a concurrency key and on a rate-limit key survive two kills of the engine: one inside a grant (behind
/// the key rows the test holds) and one inside a timeout (behind the run rows the test holds). The killed engines
/// granted and timed out nothing; the third grants the queued runs in their admission order, each once, at most one per
/// window of the rate key, and times out each waiter whose deadline passed during the outage exactly once.
#[tokio::test]
async fn queued_runs_survive_a_killed_engine_and_are_granted_in_admission_order() {
    let (p, _control_plane, dir) = stack().await;
    let long = 3_600_000;
    let accounts = p
        .publish(
            "restart-accounts",
            &start_gated(
                "restart-accounts",
                "c",
                "concurrency",
                json!({"group": "accounts", "key": "input.accountId", "limit": 1, "queueTimeoutMs": long}),
            ),
        )
        .await;
    let partners = p
        .publish(
            "restart-partners",
            &start_gated(
                "restart-partners",
                "r",
                "rateLimit",
                json!({"group": "partners", "key": "input.partner", "limit": 1, "per": "second",
                       "queueTimeoutMs": long}),
            ),
        )
        .await;
    // a window an hour long: its waiter can only time out within the test
    let hourly = p
        .publish(
            "restart-hourly",
            &start_gated(
                "restart-hourly",
                "h",
                "rateLimit",
                json!({"group": "hourly", "key": "input.partner", "limit": 1, "per": "hour",
                       "queueTimeoutMs": long}),
            ),
        )
        .await;

    // the control plane decides every start in its creation transaction; no engine runs yet
    let holder = start(&p, &accounts, json!({"accountId": "a-1"})).await;
    let mut queued = Vec::new();
    for _ in 0..3 {
        queued.push(start(&p, &accounts, json!({"accountId": "a-1"})).await);
    }
    let other_holder = start(&p, &accounts, json!({"accountId": "a-2"})).await;
    let times_out = start(&p, &accounts, json!({"accountId": "a-2"})).await;
    let first_slot = start(&p, &partners, json!({"partner": "p-1"})).await;
    let mut buffered = Vec::new();
    for _ in 0..3 {
        buffered.push(start(&p, &partners, json!({"partner": "p-1"})).await);
    }
    let hourly_slot = start(&p, &hourly, json!({"partner": "p-2"})).await;
    let rate_times_out = start(&p, &hourly, json!({"partner": "p-2"})).await;
    for run in queued
        .iter()
        .chain(&buffered)
        .chain([&times_out, &rate_times_out])
    {
        assert_eq!(
            state(&p, run).await.0,
            "WAITING",
            "{run} waits at its start"
        );
    }
    assert_eq!(count(&p, &holder, "concurrency.acquired").await, 1);
    assert_eq!(count(&p, &first_slot, "rate_limit.granted").await, 1);

    // the holder ends: its key has room for the oldest waiter, whose grant is the engine's
    operated(cancel_request(&p, &holder, "cancel-holder")).await;
    assert_eq!(state(&p, &holder).await.0, "CANCELLED");

    // Kill 1: inside a grant. The test holds both keys' rows; the engine's grant pass waits behind them.
    let keys = hold(
        &p,
        &[(key_row(false), &queued[0]), (key_row(true), &buffered[0])],
    )
    .await;
    let mut engine = Process::engine(&p, dir.path(), "engine-1").await;
    engine.until_blocked(&p).await;
    engine.kill().await;
    // the deadlines of two waiters pass while no engine runs
    let mut deadlines = Vec::new();
    for run in [&times_out, &rate_times_out] {
        deadlines.push(set_start_deadline(&p.api.db.pool, run_id(run), OUTAGE_DEADLINE_US).await);
    }
    keys.rollback().await.expect("the keys let go");
    engine.until_gone(&p).await;
    for deadline in deadlines {
        until_past(&p.api.db.pool, deadline).await;
    }
    assert_eq!(
        count(&p, &queued[0], "concurrency.acquired").await,
        0,
        "the killed engine's grant did not commit"
    );
    assert_eq!(count(&p, &buffered[0], "rate_limit.granted").await, 0);

    // Kill 2: inside a timeout. The test holds the rows of the runs past their deadline; the restarted engine's timeout
    // pass, which comes before its grants, waits behind them.
    let runs = hold(&p, &[(RUN_ROW, &times_out), (RUN_ROW, &rate_times_out)]).await;
    let mut engine = Process::engine(&p, dir.path(), "engine-2").await;
    engine.until_blocked(&p).await;
    engine.kill().await;
    runs.rollback().await.expect("the runs let go");
    engine.until_gone(&p).await;
    for run in [&times_out, &rate_times_out] {
        assert_eq!(
            state(&p, run).await.0,
            "WAITING",
            "the killed timeout did not commit"
        );
        assert_eq!(count(&p, run, "concurrency.wait_timed_out").await, 0);
        assert_eq!(count(&p, run, "rate_limit.wait_timed_out").await, 0);
    }
    assert_eq!(count(&p, &queued[0], "concurrency.acquired").await, 0);

    // The third engine times out the two waiters, then grants the queues in order as their holders end.
    let engine = Process::engine(&p, dir.path(), "engine-3").await;
    until(
        "the waiters past their deadline timed out",
        STATE_LIMIT,
        || async {
            let ended = state(&p, &times_out).await.0 == "FAILED"
                && state(&p, &rate_times_out).await.0 == "FAILED";
            ended.then_some(())
        },
    )
    .await;
    let mut worker = p.worker_at(&engine.url, "restart-worker").await;
    worker.register(&["act.c", "act.r", "act.h"]).await;
    let all: Vec<&String> = queued
        .iter()
        .chain(&buffered)
        .chain([&other_holder, &first_slot, &hourly_slot])
        .collect();
    let completed = work_until_ended(&p, &mut worker, &all).await;

    for run in &all {
        assert_eq!(state(&p, run).await.0, "SUCCEEDED", "{run} was not lost");
        assert_eq!(
            completed.iter().filter(|done| done == run).count(),
            1,
            "{run}'s task was given once"
        );
        assert_eq!(count(&p, run, "step.leased").await, 1, "{run} ran once");
    }
    // the concurrency queue: each granted once, oldest first (D022-073)
    let queue: Vec<&String> = queued.iter().collect();
    for run in &queue {
        assert_eq!(
            count(&p, run, "concurrency.acquired").await,
            1,
            "{run} granted once"
        );
    }
    assert_eq!(
        in_journal_order(&p, &queue, "concurrency.acquired").await,
        queued,
        "the concurrency queue is granted in admission order"
    );
    // the rate queue: each granted once, in admission order, never two in one window of a second
    let mut previous = granted_us(&p, &first_slot).await;
    for run in &buffered {
        assert_eq!(
            count(&p, run, "rate_limit.granted").await,
            1,
            "{run} granted once"
        );
        let at = granted_us(&p, run).await;
        assert!(
            at >= previous + SECOND_US,
            "{run} was granted {}us after the slot before it: in admission order and one per second",
            at - previous
        );
        previous = at;
    }
    // the waiters whose deadline passed during the outage: timed out once, never granted
    let (status, code) = state(&p, &times_out).await;
    assert_eq!(
        (status.as_str(), code.as_deref()),
        ("FAILED", Some("CONCURRENCY_WAIT_TIMEOUT"))
    );
    assert_eq!(count(&p, &times_out, "concurrency.wait_timed_out").await, 1);
    assert_eq!(count(&p, &times_out, "concurrency.acquired").await, 0);
    let (status, code) = state(&p, &rate_times_out).await;
    assert_eq!(
        (status.as_str(), code.as_deref()),
        ("FAILED", Some("RATE_LIMIT_WAIT_TIMEOUT"))
    );
    assert_eq!(
        count(&p, &rate_times_out, "rate_limit.wait_timed_out").await,
        1
    );
    assert_eq!(count(&p, &rate_times_out, "rate_limit.granted").await, 0);
    for run in [&times_out, &rate_times_out] {
        assert_eq!(count(&p, run, "step.leased").await, 0, "{run} never ran");
    }
    assert_eq!(count(&p, &holder, "concurrency.released").await, 1);
    let mut every = all.clone();
    every.extend([&holder, &times_out, &rate_times_out]);
    assert_consistent(&p, &every).await;
    let (holders, waiters): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM concurrency_holders) + (SELECT count(*) FROM concurrency_waiters), \
                (SELECT count(*) FROM rate_limit_waiters)",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the rows");
    assert_eq!(
        (holders, waiters),
        (0, 0),
        "every run ended: no permit held, no waiter left"
    );
}

/// Steps waiting for a step permit and for a rate slot survive a kill of the engine inside a grant (behind the rate
/// window the test holds) while the steps that hold the permit and the slot are leased to a worker: the worker completes
/// them through the restarted engine, which gives the permit back once and grants the waiting steps in admission order,
/// each once, at most one rate slot per window counted from the lease; a waiting step whose deadline passed during the
/// outage times out exactly once.
#[tokio::test]
async fn waiting_steps_survive_a_killed_engine_and_are_granted_in_admission_order() {
    let (p, _control_plane, dir) = stack().await;
    let long = 3_600_000;
    let jobs = p
        .publish(
            "restart-jobs",
            &step_gated(
                "restart-jobs",
                "p",
                "concurrency",
                json!({"group": "jobs", "key": "input.job", "limit": 1, "queueTimeoutMs": long}),
            ),
        )
        .await;
    let calls = p
        .publish(
            "restart-calls",
            &step_gated(
                "restart-calls",
                "t",
                "rateLimit",
                json!({"group": "calls", "key": "input.partner", "limit": 1, "per": "second",
                       "queueTimeoutMs": long}),
            ),
        )
        .await;

    // each run's step is decided ready by the control plane: a permit or a slot, or a wait
    let permit_holder = start(&p, &jobs, json!({"job": "j-1"})).await;
    let mut permit_waiters = Vec::new();
    for _ in 0..3 {
        permit_waiters.push(start(&p, &jobs, json!({"job": "j-1"})).await);
    }
    let other_holder = start(&p, &jobs, json!({"job": "j-2"})).await;
    let times_out = start(&p, &jobs, json!({"job": "j-2"})).await;
    let slot_holder = start(&p, &calls, json!({"partner": "c-1"})).await;
    let mut slot_waiters = Vec::new();
    for _ in 0..3 {
        slot_waiters.push(start(&p, &calls, json!({"partner": "c-1"})).await);
    }
    for run in permit_waiters
        .iter()
        .chain(&slot_waiters)
        .chain([&times_out])
    {
        assert_eq!(state(&p, run).await.0, "WAITING", "{run}'s step waits");
    }
    assert_eq!(count(&p, &permit_holder, "concurrency.acquired").await, 1);
    assert_eq!(count(&p, &slot_holder, "rate_limit.granted").await, 1);

    // The window is due (its hint, early, has come); the test holds its row before any engine runs, so the first grant
    // pass of the engine waits behind it inside the grant of the oldest waiting step.
    let hint: i64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM next_eligible_at) * 1000000)::bigint FROM rate_limit_windows WHERE waiters > 0",
    )
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the window's hint");
    until_past(&p.api.db.pool, hint).await;
    let window = hold(&p, &[(key_row(true), &slot_waiters[0])]).await;
    let mut engine = Process::engine(&p, dir.path(), "engine-1").await;
    engine.until_blocked(&p).await;
    // while its scheduler waits, its worker API leases the steps that hold the permit and the slot: handlers in progress
    let mut worker = p.worker_at(&engine.url, "restart-worker").await;
    worker.register(&["act.p", "act.t"]).await;
    let mut in_progress = Vec::new();
    let leasing = std::time::Instant::now() + STATE_LIMIT;
    while in_progress.len() < 3 {
        in_progress.extend(worker.poll(8).await);
        assert!(
            std::time::Instant::now() < leasing,
            "the holders' steps were not all leased: {in_progress:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut leased: Vec<String> = in_progress.iter().map(|task| task.run_id.clone()).collect();
    leased.sort();
    let mut holders = vec![
        permit_holder.clone(),
        other_holder.clone(),
        slot_holder.clone(),
    ];
    holders.sort();
    assert_eq!(leased, holders, "only the holders' steps run");
    engine.kill().await;
    let deadline =
        set_gate_deadline(&p.api.db.pool, run_id(&times_out), "p", OUTAGE_DEADLINE_US).await;
    window.rollback().await.expect("the window lets go");
    engine.until_gone(&p).await;
    until_past(&p.api.db.pool, deadline).await;
    assert_eq!(
        count(&p, &slot_waiters[0], "rate_limit.granted").await,
        0,
        "the killed engine's grant did not commit"
    );

    // The restarted engine accepts the leases the killed one gave, gives the permit back, and grants in order.
    let engine = Process::engine(&p, dir.path(), "engine-2").await;
    reconnect(&mut worker, &engine).await;
    until(
        "the step past its deadline timed out",
        STATE_LIMIT,
        || async { (count(&p, &times_out, "concurrency.wait_timed_out").await > 0).then_some(()) },
    )
    .await;
    for task in &in_progress {
        worker
            .complete(task, &json!({"done": true}))
            .await
            .expect("a lease given before the kill completes after it");
    }
    let all: Vec<&String> = permit_waiters.iter().chain(&slot_waiters).collect();
    let completed = work_until_ended(&p, &mut worker, &all).await;

    let holders = [&permit_holder, &other_holder, &slot_holder];
    for run in all.iter().copied().chain(holders) {
        assert_eq!(state(&p, run).await.0, "SUCCEEDED", "{run} was not lost");
        assert_eq!(
            count(&p, run, "step.leased").await,
            1,
            "{run}'s step ran once"
        );
    }
    for run in &all {
        assert_eq!(
            completed.iter().filter(|done| done == run).count(),
            1,
            "{run}'s task was given once"
        );
    }
    for run in [&permit_holder, &other_holder] {
        assert_eq!(
            count(&p, run, "concurrency.released").await,
            1,
            "{run} gave its permit back once"
        );
    }
    let queue: Vec<&String> = permit_waiters.iter().collect();
    for run in &queue {
        assert_eq!(
            count(&p, run, "concurrency.acquired").await,
            1,
            "{run} granted once"
        );
    }
    assert_eq!(
        in_journal_order(&p, &queue, "concurrency.acquired").await,
        permit_waiters,
        "the waiting steps are granted their permits in admission order"
    );
    // a step's slot counts from its lease (data-model, Rate limits): the first waiter is granted a window after the
    // holder's lease, and each next a window after the one before it, in admission order
    let mut previous = journaled_us(&p, &slot_holder, "step.leased").await;
    for run in &slot_waiters {
        assert_eq!(
            count(&p, run, "rate_limit.granted").await,
            1,
            "{run} granted once"
        );
        let at = granted_us(&p, run).await;
        assert!(
            at >= previous + SECOND_US,
            "{run} was granted {}us after the slot before it counted: in admission order and one per second",
            at - previous
        );
        previous = journaled_us(&p, run, "step.leased").await;
    }
    assert_eq!(state(&p, &times_out).await.0, "FAILED");
    assert_eq!(
        count(&p, &times_out, "concurrency.wait_timed_out").await,
        1,
        "timed out once"
    );
    assert_eq!(count(&p, &times_out, "concurrency.acquired").await, 0);
    assert_eq!(count(&p, &times_out, "step.leased").await, 0);
    let mut every = all.clone();
    every.extend(holders);
    every.push(&times_out);
    assert_consistent(&p, &every).await;
}

// --- the control plane is killed -----------------------------------------------------------------------------------

/// Sends `request` while the test holds the row a decision of `control_plane` needs, kills the control plane once its
/// decision waits behind it, lets the row go and returns once every backend of the killed process has ended: the
/// request had no answer and its transaction did not commit.
async fn kill_while_deciding(
    p: &Platform,
    control_plane: &mut Process,
    row: (&'static str, &str),
    request: reqwest::RequestBuilder,
) {
    let held = hold(p, &[row]).await;
    let sent = tokio::spawn(request.send());
    control_plane.until_blocked(p).await;
    control_plane.kill().await;
    let answer = sent.await.expect("the request's task");
    assert!(
        answer.is_err(),
        "a killed control plane answered: {answer:?}"
    );
    held.rollback().await.expect("the row lets go");
    control_plane.until_gone(p).await;
}

/// The control plane decides a run's start (its permit, its slot or its place in a queue) and a cancellation (a dequeue)
/// in one transaction. Killed inside each of them, it leaves nothing: no run, no waiter, no dequeue. The caller sends the
/// same request with the same idempotency key to the restarted control plane, which decides it once (a further send
/// replays the same run); the queued runs are then granted in admission order by the engine, which ran throughout.
#[tokio::test]
async fn a_killed_control_plane_leaves_no_half_decided_start_or_cancellation() {
    let (mut p, mut control_plane, dir) = stack().await;
    let engine = Process::engine(&p, dir.path(), "engine").await;
    let accounts = p
        .publish(
            "restart-cp-accounts",
            &start_gated(
                "restart-cp-accounts",
                "c",
                "concurrency",
                json!({"group": "accounts", "key": "input.accountId", "limit": 1, "queueTimeoutMs": 3_600_000}),
            ),
        )
        .await;
    let hourly = p
        .publish(
            "restart-cp-hourly",
            &start_gated(
                "restart-cp-hourly",
                "h",
                "rateLimit",
                json!({"group": "hourly", "key": "input.partner", "limit": 1, "per": "hour",
                       "queueTimeoutMs": 3_600_000}),
            ),
        )
        .await;
    let holder = start(&p, &accounts, json!({"accountId": "a-1"})).await;
    let first = start(&p, &accounts, json!({"accountId": "a-1"})).await;
    let slot = start(&p, &hourly, json!({"partner": "p-1"})).await;
    let runs_of = |workflow: String| {
        let pool = p.api.db.pool.clone();
        async move {
            let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE workflow_id = $1")
                .bind(Uuid::parse_str(&workflow).expect("a workflow id"))
                .fetch_one(&pool)
                .await
                .expect("the runs");
            runs
        }
    };

    // A start queued on a concurrency key, killed inside its admission (behind the key's row).
    let input = json!({"accountId": "a-1"});
    kill_while_deciding(
        &p,
        &mut control_plane,
        (key_row(false), &first),
        start_request(&p, &accounts, &input, "start-second"),
    )
    .await;
    assert_eq!(
        runs_of(accounts.clone()).await,
        2,
        "the killed start created no run"
    );
    let mut control_plane = Process::control_plane(&p, dir.path(), "control-plane-2").await;
    p.api.base.clone_from(&control_plane.url);
    let second = accepted(start_request(&p, &accounts, &input, "start-second")).await;
    assert_eq!(
        accepted(start_request(&p, &accounts, &input, "start-second")).await,
        second,
        "the same key replays the one run"
    );
    assert_eq!(runs_of(accounts.clone()).await, 3);
    assert_eq!(
        count(&p, &second, "concurrency.queued").await,
        1,
        "queued once"
    );

    // A start buffered on a rate key, killed inside its admission (behind the window's row).
    let input = json!({"partner": "p-1"});
    kill_while_deciding(
        &p,
        &mut control_plane,
        (key_row(true), &slot),
        start_request(&p, &hourly, &input, "start-buffered"),
    )
    .await;
    assert_eq!(
        runs_of(hourly.clone()).await,
        1,
        "the killed start created no run"
    );
    let mut control_plane = Process::control_plane(&p, dir.path(), "control-plane-3").await;
    p.api.base.clone_from(&control_plane.url);
    let buffered = accepted(start_request(&p, &hourly, &input, "start-buffered")).await;
    assert_eq!(
        accepted(start_request(&p, &hourly, &input, "start-buffered")).await,
        buffered
    );
    assert_eq!(runs_of(hourly.clone()).await, 2);
    assert_eq!(
        count(&p, &buffered, "rate_limit.queued").await,
        1,
        "buffered once"
    );
    assert_eq!(state(&p, &buffered).await.0, "WAITING");

    // A cancellation of the oldest waiter, killed inside its dequeue (behind the key's row).
    kill_while_deciding(
        &p,
        &mut control_plane,
        (key_row(false), &first),
        cancel_request(&p, &first, "cancel-first"),
    )
    .await;
    assert_eq!(
        state(&p, &first).await.0,
        "WAITING",
        "the killed cancellation left the run waiting"
    );
    assert_eq!(count(&p, &first, "concurrency.dequeued").await, 0);
    let control_plane = Process::control_plane(&p, dir.path(), "control-plane-4").await;
    p.api.base.clone_from(&control_plane.url);
    operated(cancel_request(&p, &first, "cancel-first")).await;
    operated(cancel_request(&p, &first, "cancel-first")).await;
    assert_eq!(state(&p, &first).await.0, "CANCELLED");
    assert_eq!(
        count(&p, &first, "concurrency.dequeued").await,
        1,
        "dequeued once"
    );
    operated(cancel_request(&p, &buffered, "cancel-buffered")).await;
    assert_eq!(count(&p, &buffered, "rate_limit.dequeued").await, 1);

    // The engine, which ran throughout, grants the start decided after the restart once its holder ends.
    let mut worker = p.worker_at(&engine.url, "restart-worker").await;
    worker.register(&["act.c", "act.h"]).await;
    let completed = work_until_ended(&p, &mut worker, &[&holder, &second, &slot]).await;
    assert_eq!(completed.iter().filter(|run| **run == second).count(), 1);
    assert_eq!(
        count(&p, &second, "concurrency.acquired").await,
        1,
        "granted once"
    );
    assert!(
        journaled_us(&p, &second, "concurrency.acquired").await
            >= journaled_us(&p, &holder, "concurrency.released").await,
        "granted after its holder gave the permit back"
    );
    for run in [&holder, &second, &slot] {
        assert_eq!(state(&p, run).await.0, "SUCCEEDED");
    }
    assert_consistent(&p, &[&holder, &first, &second, &slot, &buffered]).await;
}

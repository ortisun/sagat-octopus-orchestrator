//! The gate of a run's start against PostgreSQL (T10 of the plan, T022-022; FR-022-018, FR-022-021; D022-031, D1180,
//! D1182): a definition with a top-level `concurrency` is admitted through its key in the run's creation transaction (a
//! permit held, or a wait in the key's queue), its permit is given back by its last commit, and the grant and timeout
//! routines serve its queue (T26, T27). Every run is checked by `verify_run`, which holds invariants 1 and 2 of
//! `data-model.md` for the run's start.

use sagat_application::error::AppError;
use sagat_application::flow_control::FlowControlRefusal;
use sagat_application::runs::RunOperation;
use sagat_domain::execution::{RunId, RunStatus};
use sagat_domain::quota::QuotaResource;
use serde_json::{Value, json};

use crate::support::engine::{World, activity, child_node, ctx, document};
use crate::support::flow_control::{gated_world, publish};

/// A one-step workflow `name` whose start takes a permit of `input.accountId` with `limit`.
pub(crate) fn serial_start(name: &str, limit: u32) -> Value {
    let mut doc = document(name, "g", vec![activity("g")]);
    doc["concurrency"] = json!({"group": "accounts", "key": "input.accountId", "limit": limit});
    doc
}

/// The run's own holder and waiter rows (holders, waiters).
pub(crate) async fn start_rows(w: &World, run: RunId) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM concurrency_holders WHERE run_id = $1 AND holder_scope = 'RUN'), \
                (SELECT count(*) FROM concurrency_waiters WHERE run_id = $1 AND permit_scope = 'RUN')",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("rows")
}

pub(crate) async fn count(w: &World, run: RunId, event: &str) -> usize {
    w.journal(run).await.iter().filter(|e| *e == event).count()
}

pub(crate) async fn set(w: &World, resource: QuotaResource, value: u64) {
    w.quotas
        .set(&w.admin, Some(w.namespace.id()), resource, value, &ctx())
        .await
        .unwrap_or_else(|error| panic!("{resource}: {error:?}"));
}

/// Replaces a queued start's waiter row by one past its deadline, and its projection with it (a waiter row is never
/// updated in place).
pub(crate) async fn age_start_deadline(w: &World, run: RunId) {
    crate::support::flow_control::age_start_deadline(w, run).await;
}

/// A start with room holds a permit from its creation transaction and runs; its last commit gives the permit back,
/// before `run.finished`, and leaves no row.
#[tokio::test]
async fn a_start_with_room_holds_a_permit_runs_and_gives_it_back_with_its_last_event() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-room", 1)).await;
    let session = w.session(&["act.g"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let journal = w.journal(run).await;
    assert_eq!(
        &journal[..3],
        ["run.started", "concurrency.acquired", "step.released"],
        "{journal:?}"
    );
    assert_eq!(start_rows(&w, run).await, (1, 0));
    w.assert_consistent(run).await;

    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, run);
    w.complete(&session, &task, &json!({})).await.expect("done");
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    let journal = w.journal(run).await;
    let tail: Vec<_> = journal.iter().rev().take(2).map(String::as_str).collect();
    assert_eq!(
        tail,
        ["run.finished", "concurrency.released"],
        "{journal:?}"
    );
    assert_eq!(
        start_rows(&w, run).await,
        (0, 0),
        "the permit is given back"
    );
    w.assert_consistent(run).await;
}

/// A start without room waits in the key's queue (`WAITING`, nothing released, no task) and holds nothing; the end of
/// the holder makes the key due and the next pass grants the oldest waiter, which then runs. Another key is not held
/// back.
#[tokio::test]
async fn a_start_without_room_waits_and_is_granted_when_the_holder_ends() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-queue", 1)).await;
    let session = w.session(&["act.g"]).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let older = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let younger = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let other = w.start(workflow, &json!({"accountId": "acct-2"})).await;
    for waiter in [older, younger] {
        assert_eq!(
            w.journal(waiter).await,
            ["run.started", "concurrency.queued"]
        );
        assert_eq!(w.run_status(waiter).await, RunStatus::Waiting);
        assert_eq!(start_rows(&w, waiter).await, (0, 1));
        w.assert_consistent(waiter).await;
    }
    assert_eq!(start_rows(&w, other).await, (1, 0), "another key has room");
    assert_eq!(w.scheduler.tick().await.gates_granted, 0, "nothing is free");

    let tasks = w.poll(&session, 8).await;
    let mut ran: Vec<RunId> = tasks.iter().map(|t| t.run_id).collect();
    ran.sort();
    let mut expected = vec![holder, other];
    expected.sort();
    assert_eq!(ran, expected, "only the holders run");
    let task = tasks
        .iter()
        .find(|t| t.run_id == holder)
        .expect("holder's task");
    w.complete(&session, task, &json!({})).await.expect("done");
    let report = w.scheduler.tick().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(count(&w, older, "concurrency.acquired").await, 1);
    assert_eq!(w.run_status(older).await, RunStatus::Running);
    assert_eq!(start_rows(&w, older).await, (1, 0));
    assert_eq!(
        start_rows(&w, younger).await,
        (0, 1),
        "the younger waits on"
    );
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, older, "the granted run's entry is released");
    for run in [holder, older, younger, other] {
        w.assert_consistent(run).await;
    }
}

/// A queued start whose deadline passed fails `CONCURRENCY_WAIT_TIMEOUT` directly, never having run, and leaves the
/// queue.
#[tokio::test]
async fn a_queued_start_past_its_deadline_fails_concurrency_wait_timeout() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-timeout", 1)).await;
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
    assert_eq!(code.as_deref(), Some("CONCURRENCY_WAIT_TIMEOUT"));
    assert_eq!(count(&w, waiter, "concurrency.wait_timed_out").await, 1);
    assert_eq!(count(&w, waiter, "step.leased").await, 0);
    assert_eq!(start_rows(&w, waiter).await, (0, 0));
    w.assert_consistent(waiter).await;
}

/// A cancelled queued start leaves the queue (`concurrency.dequeued`) and ends `CANCELLED`; a cancelled holder gives
/// its permit back, and the waiter behind it is granted by the next pass.
#[tokio::test]
async fn a_cancellation_dequeues_a_queued_start_and_frees_a_held_permit() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-cancel", 1)).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    w.operate_run(first, RunOperation::Cancel).await;
    assert_eq!(w.run_status(first).await, RunStatus::Cancelled);
    assert_eq!(count(&w, first, "concurrency.dequeued").await, 1);
    assert_eq!(start_rows(&w, first).await, (0, 0));
    w.operate_run(holder, RunOperation::Cancel).await;
    assert_eq!(count(&w, holder, "concurrency.released").await, 1);
    assert_eq!(start_rows(&w, holder).await, (0, 0));
    // the report names why a tick granted nothing (a busy run, a failed grant, a fenced or empty selection), so an
    // intermittent failure on a loaded runner says its cause (batch 3 work item 2)
    let report = w.scheduler.tick().await;
    assert_eq!(
        report.gates_granted, 1,
        "the tick did not grant exactly one: {report:?}"
    );
    assert_eq!(start_rows(&w, second).await, (1, 0));
    for run in [holder, first, second] {
        w.assert_consistent(run).await;
    }
}

/// D1180: a start key that does not evaluate creates the run and ends it `FAILED`/`EXPRESSION_ERROR` in its admission
/// transaction; it never waited for or held a permit.
#[tokio::test]
async fn a_start_key_that_does_not_evaluate_ends_the_run_with_expression_error() {
    let w = gated_world().await;
    let mut doc = document("start-key-error", "g", vec![activity("g")]);
    doc["concurrency"] = json!({"key": "input.missing.deeper", "limit": 1});
    let workflow = publish(&w, doc).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    let (code,): (Option<String>,) = sqlx::query_as("SELECT stop_code FROM runs WHERE id = $1")
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("run");
    assert_eq!(code.as_deref(), Some("EXPRESSION_ERROR"));
    assert_eq!(start_rows(&w, run).await, (0, 0));
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM concurrency_semaphores")
        .fetch_one(&w.db.pool)
        .await
        .expect("keys");
    assert_eq!(keys, 0, "no key was filed");
    w.assert_consistent(run).await;
}

/// D1182, `contracts/errors.md`: a synchronous start that would wait beyond the key's waiter ceiling is refused
/// `CONCURRENCY_QUEUE_FULL`, one beyond the namespace's `queued_runs` `FLOW_CONTROL_LIMIT`, and one whose key would be
/// new beyond the live keys `FLOW_CONTROL_LIMIT`; nothing of a refused run exists.
#[tokio::test]
async fn a_start_beyond_a_queue_ceiling_is_refused_and_nothing_of_it_exists() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-ceilings", 1)).await;
    let refused = |result: Result<RunId, AppError>| match result {
        Err(AppError::FlowControl(refusal)) => refusal,
        other => panic!("refused expected: {other:?}"),
    };
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
            "start-full-key",
            None,
        )
        .await;
    assert_eq!(refused(full), FlowControlRefusal::ConcurrencyQueueFull);
    set(&w, QuotaResource::FlowControlKeyWaiters, 10).await;
    set(&w, QuotaResource::QueuedRuns, 1).await;
    let namespace_full = w
        .start_with(
            workflow,
            &json!({"accountId": "acct-1"}),
            "start-full-namespace",
            None,
        )
        .await;
    assert_eq!(
        refused(namespace_full),
        FlowControlRefusal::Limit {
            limit: "queued_runs"
        }
    );
    set(&w, QuotaResource::ConcurrencyLiveKeys, 1).await;
    let new_key = w
        .start_with(
            workflow,
            &json!({"accountId": "acct-2"}),
            "start-new-key",
            None,
        )
        .await;
    assert_eq!(
        refused(new_key),
        FlowControlRefusal::Limit {
            limit: "concurrency_live_keys"
        }
    );
    let runs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&w.db.pool)
        .await
        .expect("runs");
    assert_eq!(runs_after, runs_before, "a refused start leaves no run");
}

/// Every creator of the tree passes the run's start through its gate (D022-031): a child node's run is admitted
/// through its key like a start of the API, and waits in the same queue.
#[tokio::test]
async fn a_child_run_of_a_gated_definition_is_admitted_through_its_key() {
    let w = gated_world().await;
    let gated = publish(&w, serial_start("start-child-target", 1)).await;
    let session = w.session(&["act.g"]).await;
    let holder = w.start(gated, &json!({"accountId": "acct-1"})).await;
    // the child's input is its parent's (the child node carries no input of its own)
    let parent_doc = document(
        "start-child-parent",
        "c",
        vec![child_node("c", "start-child-target", None)],
    );
    let parent = publish(&w, parent_doc).await;
    let parent_run = w.start(parent, &json!({"accountId": "acct-1"})).await;
    w.scheduler.tick().await;
    let child: (uuid::Uuid,) = sqlx::query_as("SELECT id FROM runs WHERE parent_run_id = $1")
        .bind(parent_run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("the child run exists");
    let child = RunId::new(child.0);
    assert_eq!(
        w.journal(child).await,
        ["run.started", "concurrency.queued"]
    );
    assert_eq!(start_rows(&w, child).await, (0, 1));
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder);
    w.complete(&session, &task, &json!({})).await.expect("done");
    w.scheduler.tick().await;
    assert_eq!(
        start_rows(&w, child).await,
        (1, 0),
        "granted from the queue"
    );
    for run in [holder, child, parent_run] {
        w.assert_consistent(run).await;
    }
}

/// D022-021, the reservation rule at admission: once a permit is free but a run still waits for it, a new start does not
/// take it ahead of the waiter: it queues behind, and the pass grants the older one.
#[tokio::test]
async fn a_new_start_never_takes_a_free_permit_ahead_of_a_queued_run() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-reserve", 1)).await;
    let session = w.session(&["act.g"]).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let older = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder);
    w.complete(&session, &task, &json!({})).await.expect("done");
    assert_eq!(start_rows(&w, holder).await, (0, 0), "the permit is free");
    let newer = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(
        w.journal(newer).await,
        ["run.started", "concurrency.queued"],
        "the free permit is the queued run's"
    );
    assert_eq!(w.scheduler.tick().await.gates_granted, 1);
    assert_eq!(
        start_rows(&w, older).await,
        (1, 0),
        "the oldest waiter is granted"
    );
    assert_eq!(start_rows(&w, newer).await, (0, 1));
    for run in [holder, older, newer] {
        w.assert_consistent(run).await;
    }
}

/// Invariants 1 and 2 of `data-model.md` for a run's start: `verify_run` fails on a holder row the journal does not
/// hold, on a waiter row gone while the journal waits, and on a key counter that disagrees with its rows.
#[tokio::test]
async fn verify_run_fails_on_a_seeded_start_row_inconsistency() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-invariants", 1)).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let tenant = w.admin.tenant_id();
    w.assert_consistent(holder).await;
    w.assert_consistent(waiter).await;
    let mut tx = w.db.pool.begin().await.expect("tx");
    sqlx::query("DELETE FROM concurrency_waiters WHERE run_id = $1")
        .bind(waiter.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("waiter seeded away");
    sqlx::query("DELETE FROM concurrency_holders WHERE run_id = $1")
        .bind(holder.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("holder seeded away");
    tx.commit().await.expect("seeded");
    for run in [holder, waiter] {
        let why = w
            .store
            .verify_run(tenant, run)
            .await
            .expect_err("a missing row is an inconsistency");
        assert!(why.contains("invariants 1 and 2"), "{why}");
    }
}

/// G1197, security and correctness reviews of 2026-11-20 (B1): the finish paths of a `forEach` and of a `while` end the
/// run without the reducer, so they would never give a start's permit back. Until they are integrated, a definition with
/// a start gate and such a node is refused at admission (`BINARY_CAPABILITY_MISSING`) and nothing of the run exists.
#[tokio::test]
async fn a_start_gate_with_a_collection_or_a_loop_is_refused_until_their_finish_gives_the_permit_back()
 {
    let w = gated_world().await;
    let body = json!([
        {"id": "g", "type": "activity", "activity": "act.g", "next": "returned"},
        {"id": "returned", "type": "return", "result": "1"}
    ]);
    let mut collection = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "start-collection"}, "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": "g", "nodes": body.clone()}},
        "nodes": [
            {"id": "lines", "type": "forEach", "items": "input.lines", "body": "line",
             "result": "bodyResult", "maxItems": 8, "maxConcurrency": 1, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    });
    collection["concurrency"] = json!({"key": "input.accountId", "limit": 1});
    let mut looping = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "start-loop"}, "entrypoint": "poll",
        "bodies": {"pass": {"entrypoint": "g", "nodes": body}},
        "nodes": [
            {"id": "poll", "type": "while", "condition": "carry < 1", "body": "pass", "initial": "0",
             "carry": "bodyResult", "maxIterations": 2, "timeoutMs": 600000, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    });
    looping["concurrency"] = json!({"key": "input.accountId", "limit": 1});
    for doc in [collection, looping] {
        let name = doc["metadata"]["name"].as_str().expect("name").to_owned();
        let workflow = publish(&w, doc).await;
        let refused = w
            .start_with(
                workflow,
                &json!({"accountId": "acct-1", "lines": [1]}),
                &format!("start-{name}"),
                None,
            )
            .await;
        assert!(
            matches!(
                refused,
                Err(AppError::FlowControl(
                    FlowControlRefusal::BinaryCapabilityMissing
                ))
            ),
            "{name}: {refused:?}"
        );
    }
    let (runs, keys): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM runs), (SELECT count(*) FROM concurrency_semaphores)",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("counts");
    assert_eq!((runs, keys), (0, 0), "nothing of a refused start exists");
}

/// ADR-0050's merged order (correctness review M1 of 2026-11-20): a commit that ends a run holding a start permit
/// locks the run's active-run counter row (slot 6) before the permit's semaphore (slot 7), as an admission does. With the
/// counter row held elsewhere, the completion waits on it while the semaphore is still free.
#[tokio::test]
async fn a_run_that_ends_locks_its_active_run_counter_before_its_permits_key() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-counter-order", 1)).await;
    let session = w.session(&["act.g"]).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let task = w.one_task(&session).await;
    let mut holder = w.db.pool.begin().await.expect("a transaction");
    sqlx::query(
        "SELECT 1 FROM run_active_counts c JOIN runs r ON r.tenant_id = c.tenant_id AND r.namespace_id = c.namespace_id \
          WHERE r.id = $1 AND c.slot = (get_byte(uuid_send($1::uuid), 15) & 15)::smallint FOR UPDATE OF c",
    )
    .bind(run.as_uuid())
    .execute(&mut *holder)
    .await
    .expect("the counter row held");
    let output = json!({});
    let completing = w.complete(&session, &task, &output);
    let watch = async {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                     AND wait_event_type = 'Lock' AND state = 'active' AND pid <> pg_backend_pid()",
                )
                .fetch_one(&w.db.pool)
                .await
                .expect("activity");
                if waiting > 0 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the completion waits for the counter row");
        let mut probe = w.db.pool.begin().await.expect("a probe");
        let free = sqlx::query(
            "SELECT 1 FROM concurrency_semaphores WHERE permit_scope = 'RUN' FOR UPDATE NOWAIT",
        )
        .execute(&mut *probe)
        .await;
        probe.rollback().await.expect("probe ends");
        holder.rollback().await.expect("the counter row freed");
        free.is_ok()
    };
    let (completed, semaphore_free) = tokio::join!(completing, watch);
    completed.expect("completed");
    assert!(
        semaphore_free,
        "the semaphore is locked only after the counter row"
    );
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    assert_eq!(start_rows(&w, run).await, (0, 0));
    w.assert_consistent(run).await;
}

/// Invariants 1 and 2 (correctness review m1 of 2026-11-20): `verify_run` also fails on a key counter that disagrees
/// with its rows and on a waiter whose deadline is not the journal's.
#[tokio::test]
async fn verify_run_fails_on_a_seeded_counter_or_deadline_disagreement() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-invariant-counts", 1)).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let tenant = w.admin.tenant_id();
    sqlx::query(
        "UPDATE concurrency_semaphores SET holders = holders + 1 WHERE permit_scope = 'RUN'",
    )
    .execute(&w.db.pool)
    .await
    .expect("counter seeded");
    let why = w
        .store
        .verify_run(tenant, holder)
        .await
        .expect_err("a counter that disagrees is an inconsistency");
    assert!(why.contains("counters agree: false"), "{why}");
    sqlx::query(
        "UPDATE concurrency_semaphores SET holders = holders - 1 WHERE permit_scope = 'RUN'",
    )
    .execute(&w.db.pool)
    .await
    .expect("counter restored");
    w.assert_consistent(holder).await;
    sqlx::query(
        "WITH gone AS (DELETE FROM concurrency_waiters WHERE run_id = $1 RETURNING *) \
         INSERT INTO concurrency_waiters (tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, \
                group_name, key_digest, declared_limit, enqueued_at, deadline_at, shard) \
         SELECT tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, group_name, key_digest, \
                declared_limit, enqueued_at, deadline_at + interval '1 second', shard FROM gone",
    )
    .bind(waiter.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("deadline seeded");
    let why = w
        .store
        .verify_run(tenant, waiter)
        .await
        .expect_err("a deadline that is not the journal's is an inconsistency");
    assert!(why.contains("invariants 1 and 2"), "{why}");
}

/// G022-011, T022-105 (the owner's condition on D022-050 item 3): the namespace's `queued_runs` ceiling is exact under
/// concurrency. Every admission takes the namespace's active-run quota lock (`ensure_room_for_run`) before its runs row
/// and holds it to its commit, and the start's gate counts the queued runs after that lock in a fresh statement (READ
/// COMMITTED), so two starts at the boundary are decided one after the other. Proved with the lock held by the test: both
/// starts wait on it (observed in `pg_stat_activity`, no sleep), and once it is released exactly one queues and the other
/// is refused `FLOW_CONTROL_LIMIT` (`queued_runs`).
#[tokio::test]
async fn two_starts_at_the_queued_runs_ceiling_are_decided_one_after_the_other() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-queued-race", 1)).await;
    for account in ["a", "b", "c"] {
        w.start(workflow, &json!({"accountId": account})).await;
    }
    let first = w.start(workflow, &json!({"accountId": "a"})).await;
    assert_eq!(start_rows(&w, first).await, (0, 1), "one run queued");
    set(&w, QuotaResource::QueuedRuns, 2).await;

    let lock_key = format!(
        "quota:{}:{}:{}",
        QuotaResource::ActiveRuns,
        w.admin.tenant_id(),
        w.namespace.id()
    );
    let mut held = w.db.pool.begin().await.expect("a transaction");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *held)
        .await
        .expect("the namespace's admission lock held");
    let (input_b, input_c) = (json!({"accountId": "b"}), json!({"accountId": "c"}));
    let start_b = w.start_with(workflow, &input_b, "race-start-b", None);
    let start_c = w.start_with(workflow, &input_c, "race-start-c", None);
    let release = async {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                     AND wait_event_type = 'Lock' AND wait_event = 'advisory' AND pid <> pg_backend_pid()",
                )
                .fetch_one(&w.db.pool)
                .await
                .expect("activity");
                if waiting >= 2 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both starts wait on the namespace's admission lock");
        held.rollback().await.expect("the lock released");
    };
    let (b, c, ()) = tokio::join!(start_b, start_c, release);
    let outcomes = [b, c];
    let queued: Vec<RunId> = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().ok().copied())
        .collect();
    let refused: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().err())
        .collect();
    assert_eq!(queued.len(), 1, "exactly one start queues: {outcomes:?}");
    assert!(
        matches!(
            refused.as_slice(),
            [AppError::FlowControl(FlowControlRefusal::Limit {
                limit: "queued_runs"
            })]
        ),
        "the other is refused at the ceiling: {outcomes:?}"
    );
    assert_eq!(start_rows(&w, queued[0]).await, (0, 1));
    let waiting: i64 =
        sqlx::query_scalar("SELECT count(*) FROM concurrency_waiters WHERE permit_scope = 'RUN'")
            .fetch_one(&w.db.pool)
            .await
            .expect("waiters");
    assert_eq!(
        waiting, 2,
        "the namespace holds exactly its ceiling of queued runs"
    );
    for run in [first, queued[0]] {
        w.assert_consistent(run).await;
    }
}

/// FR-022-031 (re-review of 34b8be41, Major 2): turning the switch off stops new gated starts while what was admitted
/// drains. A process with the switch off that holds the digest keys (its new starts are refused `CAPABILITY_DISABLED`,
/// `admission::a_process_with_the_switch_the_activation_and_the_keys_admits_a_concurrency_gated_run`) still grants the
/// queued start once the holder ends.
#[tokio::test]
async fn with_the_switch_off_a_process_holding_the_keys_drains_the_queue() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("switched-off-drain", 1)).await;
    let session = w.session(&["act.g"]).await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let waiter = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    assert_eq!(start_rows(&w, waiter).await, (0, 1));

    // the engine's own configuration with the switches off and the key file named, and its store built as `serve` builds
    // it (red when the configuration drops the file of a switched-off process, 34b8be41)
    let file = crate::support::flow_control::process_env()
        .into_iter()
        .find(|(name, _)| *name == "SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE")
        .expect("the key file")
        .1;
    let lookup = |name: &str| match name {
        "SAGAT_OIDC_ISSUER" => Some("https://idp.example/".to_owned()),
        "SAGAT_OIDC_AUDIENCE" => Some("sagat".to_owned()),
        "SAGAT_OIDC_JWKS_URI" => Some("https://idp.example/jwks.json".to_owned()),
        "SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE" => Some(file.clone()),
        _ => None,
    };
    let engine = sagat_execution_engine::EngineConfig::from_lookup(
        &lookup,
        sagat_control_plane::config::Environment::Dev,
    )
    .expect("a switched-off engine with the key file");
    assert!(!engine.flow_control.concurrency_keys);
    let switched_off =
        sagat_infra_postgres::PgRunStore::new(w.execution_pool.clone(), w.cache.clone())
            .with_flow_control(engine.flow_control)
            .with_flow_control_keys(engine.flow_control_keys.expect("the ring is held to drain"));
    let scheduler = sagat_scheduler::Scheduler::new(
        switched_off,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_scheduler::SchedulerSettings::default(),
    )
    .with_children(w.cache.clone());
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder);
    w.complete(&session, &task, &json!({})).await.expect("done");
    let report = scheduler.tick().await;
    assert_eq!(report.gates_granted, 1, "{report:?}");
    assert_eq!(start_rows(&w, waiter).await, (1, 0), "the queue drained");
    assert_eq!(w.run_status(waiter).await, RunStatus::Running);
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// The next-eligible hint of the key `run`'s start is queued for, in microseconds.
async fn key_hint_us(w: &World, run: RunId) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT (extract(epoch FROM k.next_eligible_at) * 1000000)::bigint \
           FROM concurrency_waiters x JOIN concurrency_semaphores k \
             ON k.tenant_id = x.tenant_id AND k.namespace_id = x.namespace_id AND k.pool = x.pool \
            AND k.permit_scope = x.permit_scope AND k.group_name = x.group_name AND k.key_digest = x.key_digest \
          WHERE x.run_id = $1 AND x.permit_scope = 'RUN'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the start's waiter and its key")
}

/// Decision D022-073 (strict admission order; SC-022-004, FR-022-042): with `limit: 3`, three holders and one start
/// queued behind them, two holders end, so the key has room for two; a new start finds an eligible waiter ahead and
/// queues instead of taking a permit before it. Its key is due once it has committed (so the next pass visits it: plan
/// "The grant pass, by key", the delay strict order costs), and one pass grants the waiter and then the new start, in
/// admission order.
#[tokio::test]
async fn a_start_behind_an_eligible_waiter_queues_although_the_key_has_room_and_one_pass_grants_both_in_order()
 {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-strict-order", 3)).await;
    let session = w.session(&["act.g"]).await;
    let input = json!({"accountId": "acct-1"});
    let mut holders = Vec::new();
    for _ in 0..3 {
        holders.push(w.start(workflow, &input).await);
    }
    let waiter = w.start(workflow, &input).await;
    assert_eq!(start_rows(&w, waiter).await, (0, 1), "the key is full");
    for task in w.poll(&session, 8).await {
        if task.run_id != holders[2] {
            w.complete(&session, &task, &json!({})).await.expect("done");
        }
    }
    for run in &holders[..2] {
        assert_eq!(start_rows(&w, *run).await, (0, 0), "its permit given back");
    }
    // the releases above set the key's hint already: the start must write its own instant (re-review minor 5)
    let before = crate::support::flow_control::database_now_us(&w.db.pool).await;
    let late = w.start(workflow, &input).await;
    let committed = crate::support::flow_control::database_now_us(&w.db.pool).await;
    assert_eq!(
        start_rows(&w, late).await,
        (0, 1),
        "room for two, an eligible waiter ahead: it queues: {:?}",
        w.journal(late).await
    );
    assert_eq!(count(&w, late, "concurrency.queued").await, 1);
    let hint = key_hint_us(&w, late).await;
    assert!(
        hint.is_some_and(|hint| (before..=committed).contains(&hint)),
        "the start writes the key's hint to its own instant, whatever it was (decision D022-076 item 1): {hint:?} \
         outside [{before}, {committed}]"
    );
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 2, "{report:?}");
    let acquired: Vec<(uuid::Uuid, time::OffsetDateTime)> = sqlx::query_as(
        "SELECT run_id, acquired_at FROM concurrency_holders WHERE run_id = ANY($1) AND holder_scope = 'RUN' \
          ORDER BY acquired_at, run_id",
    )
    .bind(vec![waiter.as_uuid(), late.as_uuid()])
    .fetch_all(&w.db.pool)
    .await
    .expect("holders");
    let order: Vec<uuid::Uuid> = acquired.iter().map(|(run, _)| *run).collect();
    assert_eq!(
        order,
        [waiter.as_uuid(), late.as_uuid()],
        "admission order: {acquired:?}"
    );
    assert!(
        acquired[0].1 < acquired[1].1,
        "two grants, one after the other: {acquired:?}"
    );
    for run in holders.iter().copied().chain([waiter, late]) {
        w.assert_consistent(run).await;
    }
}

/// Item 2 of the batch 3 work (the intermittent `rate_limits::a_flood_of_a_thousand_starts_on_one_key_never_holds_more_than_its_limit_in_a_window`,
/// "grants follow admission order"): the grant routine takes a waiter's run with `SKIP LOCKED`, and a run another
/// transaction held (a pass that had not yet released it, a statement in flight) was skipped while the pass went on: on
/// a window with room for both, the younger waiter behind it was granted first, out of admission order (decision
/// D022-073). Two buffered starts of `limit: 2, per: hour` behind two holders whose slots have aged out; the older
/// waiter's run held by a transaction of this test: the pass grants nobody and leaves the key due; released, the next
/// pass grants both, the older first.
#[tokio::test]
async fn a_waiter_whose_run_is_busy_holds_back_the_younger_waiters_of_its_key() {
    let w = gated_world().await;
    let mut doc = document("start-busy-order", "g", vec![activity("g")]);
    doc["rateLimit"] =
        json!({"group": "accounts", "key": "input.accountId", "limit": 2, "per": "hour"});
    let workflow = publish(&w, doc).await;
    let input = json!({"accountId": "acct-1"});
    let _holders = [
        w.start(workflow, &input).await,
        w.start(workflow, &input).await,
    ];
    let older = w.start(workflow, &input).await;
    let younger = w.start(workflow, &input).await;
    // the holders' slots two hours old, out of the window (the guard of a grant row off in this one transaction)
    let mut tx = w.db.pool.begin().await.expect("tx");
    for statement in [
        "ALTER TABLE rate_limit_grants DISABLE TRIGGER USER",
        "UPDATE rate_limit_grants SET granted_at = granted_at - interval '2 hours', \
                leased_at = leased_at - interval '2 hours'",
        "ALTER TABLE rate_limit_grants ENABLE TRIGGER USER",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect(statement);
    }
    tx.commit().await.expect("aged");
    let granted = |run: RunId| {
        let w = &w;
        async move {
            sqlx::query_scalar::<_, time::OffsetDateTime>(
                "SELECT granted_at FROM rate_limit_grants WHERE run_id = $1 AND limit_scope = 'RUN'",
            )
            .bind(run.as_uuid())
            .fetch_optional(&w.db.pool)
            .await
            .expect("a grant")
        }
    };
    let mut busy = w.db.pool.begin().await.expect("a transaction");
    sqlx::query("SELECT 1 FROM runs WHERE id = $1 FOR UPDATE")
        .bind(older.as_uuid())
        .execute(&mut *busy)
        .await
        .expect("the older waiter's run held");
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        (report.gates_granted, granted(younger).await),
        (0, None),
        "the younger waiter is not granted past the busy older one: {report:?}"
    );
    busy.rollback().await.expect("released");
    let report = w.scheduler.grant_gates().await;
    assert_eq!(report.gates_granted, 2, "{report:?}");
    let (first, second) = (granted(older).await, granted(younger).await);
    assert!(
        first.is_some() && second.is_some() && first < second,
        "the older waiter first: {first:?} {second:?}"
    );
}
/// The failure rows of `run`'s waiters (node, attempt, class), decision D022-124.
async fn grant_failures(w: &World, run: RunId) -> Vec<(String, i32, String)> {
    sqlx::query_as(
        "SELECT node, attempt, failure FROM flow_control_grant_failures WHERE run_id = $1 ORDER BY node, attempt",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("failure rows")
}

/// Makes every insert of a holder row of `run` raise SQLSTATE `code` (a trigger the test drops with [`unrefuse`]).
async fn refuse_holder(w: &World, run: RunId, code: &str) {
    for statement in [
        format!(
            "CREATE FUNCTION test_refuse_holder() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
               IF NEW.run_id = '{}' THEN RAISE EXCEPTION 'the test refuses this holder' USING ERRCODE = '{code}'; \
               END IF; RETURN NEW; END $$",
            run.as_uuid()
        ),
        "CREATE TRIGGER test_refuse_holder BEFORE INSERT ON concurrency_holders FOR EACH ROW \
           EXECUTE FUNCTION test_refuse_holder()"
            .to_owned(),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&w.db.pool)
            .await
            .expect("the refusing trigger");
    }
}

async fn unrefuse(w: &World) {
    for statement in [
        "DROP TRIGGER test_refuse_holder ON concurrency_holders",
        "DROP FUNCTION test_refuse_holder()",
    ] {
        sqlx::query(statement)
            .execute(&w.db.pool)
            .await
            .expect("the trigger dropped");
    }
}

/// Batch 5 correctness Major 1 (decision D022-121) and its re-read's new finding 1 (decision D022-124): a waiter whose
/// grant always fails (here a trigger refuses its holder row, as a constraint would) no longer ends its key's pass, and
/// no longer counts ahead of the younger waiters, on a key of limit 1: it is skipped, counted `failed`, recorded with
/// its class, and stays queued, and the younger waiter behind it is granted in the same pass. Once its cause is gone, the
/// first try that decides it (here refused for room, the younger holding the permit) deletes its row, and it is granted
/// when the permit is free.
#[tokio::test]
async fn a_waiter_whose_grant_always_fails_does_not_hold_back_the_younger_waiters() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-poisoned", 1)).await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &account).await;
    let older = w.start(workflow, &account).await;
    let younger = w.start(workflow, &account).await;
    assert_eq!(start_rows(&w, older).await, (0, 1));
    assert_eq!(start_rows(&w, younger).await, (0, 1));
    refuse_holder(&w, older, "P0001").await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder, "only the holder runs");
    w.complete(&session, &task, &json!({})).await.expect("done");
    assert_eq!(start_rows(&w, holder).await, (0, 0), "the holder ended");
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        (report.gates_granted, report.failed),
        (1, 1),
        "the failing waiter is skipped and not counted ahead, the younger granted: {report:?}"
    );
    assert_eq!(
        start_rows(&w, younger).await,
        (1, 0),
        "the younger is granted"
    );
    assert_eq!(
        start_rows(&w, older).await,
        (0, 1),
        "the failing waiter stays queued"
    );
    assert_eq!(
        grant_failures(&w, older).await,
        [(String::new(), 0, "INTERNAL".to_owned())],
        "its failure is recorded with its class"
    );
    unrefuse(&w).await;
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 0,
        "the younger holds the permit: {report:?}"
    );
    assert!(
        grant_failures(&w, older).await.is_empty(),
        "a try that decides it deletes its failure row"
    );
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, younger);
    w.complete(&session, &task, &json!({})).await.expect("done");
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 1,
        "the older is granted once it can be: {report:?}"
    );
    assert_eq!(start_rows(&w, older).await, (1, 0));
    for run in [holder, older, younger] {
        w.assert_consistent(run).await;
    }
}

/// The re-read's finding 3 (decision D022-130 item 1): a failure that clears on its own (here a trigger raising
/// SQLSTATE 40001, a serialization failure, on the older waiter's holder row) ends the key's pass, as a busy run does:
/// nothing is granted, the younger waiter is not tried, and no failure row is written.
#[tokio::test]
async fn a_transient_grant_failure_ends_the_keys_pass() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-transient", 1)).await;
    let session = w.session(&["act.g"]).await;
    let account = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &account).await;
    let older = w.start(workflow, &account).await;
    let younger = w.start(workflow, &account).await;
    refuse_holder(&w, older, "40001").await;
    let task = w.one_task(&session).await;
    assert_eq!(task.run_id, holder);
    w.complete(&session, &task, &json!({})).await.expect("done");
    let report = w.scheduler.grant_gates().await;
    unrefuse(&w).await;
    assert_eq!(
        (report.gates_granted, report.failed),
        (0, 1),
        "a transient failure ends the pass: {report:?}"
    );
    assert_eq!(start_rows(&w, younger).await, (0, 1), "the younger waits");
    assert!(
        grant_failures(&w, older).await.is_empty(),
        "a failure that clears on its own is not recorded"
    );
    let report = w.scheduler.grant_gates().await;
    assert_eq!(
        report.gates_granted, 1,
        "the next pass grants the older: {report:?}"
    );
    assert_eq!(start_rows(&w, older).await, (1, 0));
    for run in [holder, older, younger] {
        w.assert_consistent(run).await;
    }
}

/// Batch 5 correctness minor 2 (decision D022-126 item 2): the grant of a queued start locks its run's active-run
/// counter row only when the definition's entry path can end the run in the grant's transaction. A start whose entry is
/// an activity cannot end there: with its counter row held by another transaction, its grant refused for room is
/// decided at once, never waiting for that row. A start whose entry is an `end` node still takes the row first (the
/// lock-order case of `concurrency_locks`).
#[tokio::test]
async fn a_held_counter_row_does_not_block_the_refused_grant_of_a_start_that_cannot_end() {
    let w = gated_world().await;
    let workflow = publish(&w, serial_start("start-cannot-end", 1)).await;
    let account = json!({"accountId": "acct-1"});
    let holder = w.start(workflow, &account).await;
    let waiter = w.start(workflow, &account).await;
    assert_eq!(start_rows(&w, waiter).await, (0, 1));
    // the key due, so the pass tries the waiter, which the holder's permit refuses
    sqlx::query(
        "UPDATE concurrency_semaphores SET next_eligible_at = clock_timestamp() WHERE permit_scope = 'RUN'",
    )
    .execute(&w.db.pool)
    .await
    .expect("the key due");
    let mut counter = w.db.pool.begin().await.expect("a transaction");
    sqlx::query(
        "SELECT 1 FROM run_active_counts c JOIN runs r ON r.tenant_id = c.tenant_id AND r.namespace_id = c.namespace_id \
          WHERE r.id = $1 AND c.slot = (get_byte(uuid_send($1::uuid), 15) & 15)::smallint FOR UPDATE OF c",
    )
    .bind(waiter.as_uuid())
    .execute(&mut *counter)
    .await
    .expect("the counter row held");
    // a bound on a hang, not a timing of the race: the refused grant takes no lock the test holds
    let report = crate::support::bounded::within(
        "the refused grant of a start that cannot end".to_owned(),
        std::time::Duration::from_secs(60),
        w.scheduler.grant_gates(),
    )
    .await;
    counter.rollback().await.expect("the counter row freed");
    assert_eq!(report.gates_granted, 0, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(start_rows(&w, waiter).await, (0, 1), "still queued");
    for run in [holder, waiter] {
        w.assert_consistent(run).await;
    }
}

/// Batch 5 correctness Major 2 (decision D022-122): two waiters of one run on one key, adjacent, the first refused by its
/// other gate (a rate-limit window with no slot left). The refusal's transaction is rolled back, and the rollback awaited,
/// before the second waiter is tried, so the second finds its run free and is granted, and the younger waiter of another
/// run behind them is granted in the same pass. Before, the dropped transaction kept the run locked until the pool rolled
/// it back in a task of its own: the second waiter found its run busy, which ends the key's pass (D022-088). A connection
/// released while the run is still locked fails the test whatever the timing (decision D022-130 item 3).
#[tokio::test]
async fn a_refused_waiter_frees_its_run_before_the_next_waiter_of_the_same_run_is_tried() {
    let w = gated_world().await;
    let shared = json!({"group": "shared", "key": "input.accountId", "limit": 3});
    let window = json!({"group": "window", "key": "input.accountId", "limit": 1, "per": "hour"});
    let slot_taker = publish(
        &w,
        crate::support::engine::document(
            "same-run-slot-taker",
            "t",
            vec![crate::support::engine::with(
                activity("t"),
                "rateLimit",
                window.clone(),
            )],
        ),
    )
    .await;
    let twice = publish(
        &w,
        crate::support::engine::document(
            "same-run-twice",
            "p",
            vec![
                crate::support::engine::parallel_node("p", &["a", "b"], "all"),
                crate::support::engine::with(
                    crate::support::engine::with(activity("a"), "concurrency", shared.clone()),
                    "rateLimit",
                    window,
                ),
                crate::support::engine::with(activity("b"), "concurrency", shared.clone()),
            ],
        ),
    )
    .await;
    let behind = publish(
        &w,
        crate::support::engine::document(
            "same-run-behind",
            "c",
            vec![crate::support::engine::with(
                activity("c"),
                "concurrency",
                shared,
            )],
        ),
    )
    .await;
    let account = json!({"accountId": "acct-1"});
    let _slot = w.start(slot_taker, &account).await;
    let run = w.start(twice, &account).await;
    let other = w.start(behind, &account).await;
    let waiting = |run: RunId| {
        let w = &w;
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT node FROM concurrency_waiters WHERE run_id = $1 AND permit_scope = 'STEP' ORDER BY node",
            )
            .bind(run.as_uuid())
            .fetch_all(&w.db.pool)
            .await
            .expect("waiters")
        }
    };
    assert_eq!(
        waiting(run).await,
        ["a", "b"],
        "both steps of the run wait on the shared key"
    );
    assert_eq!(
        waiting(other).await,
        ["c"],
        "the other run waits behind them"
    );
    // the scheduler's store over a pool of one connection whose `after_release` hook, which sqlx runs before it sends
    // the rollback of a dropped transaction (the ping that follows flushes it), asks a second connection for the run's
    // lock with `FOR UPDATE NOWAIT`: a refusal that merely drops its transaction still holds the run then, and the
    // check is refused. With one connection the next try cannot begin before the hook has run, so the check reads a
    // held lock, never a timing (re-read correctness finding 5, decision D022-130 item 3)
    let url = if crate::support::as_runtime_role() {
        w.db.url_for(
            crate::support::RUNTIME_ROLE,
            crate::support::RUNTIME_ROLE_PASSWORD,
        )
    } else {
        w.db.url.clone()
    };
    let lingering = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (probe, seen) = (w.db.pool.clone(), std::sync::Arc::clone(&lingering));
    let late = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_release(move |_, _| {
            let (probe, seen) = (probe.clone(), std::sync::Arc::clone(&seen));
            Box::pin(async move {
                let mut check = probe.begin().await?;
                let locked = sqlx::query("SELECT 1 FROM runs WHERE id = $1 FOR UPDATE NOWAIT")
                    .bind(run.as_uuid())
                    .execute(&mut *check)
                    .await;
                check.rollback().await?;
                if let Err(error) = locked {
                    seen.lock().expect("the record").push(error.to_string());
                }
                Ok(true)
            })
        })
        .connect(&url)
        .await
        .expect("a pool of one connection that checks the run's lock on release");
    let store = crate::support::flow_control::gate(sagat_infra_postgres::PgRunStore::new(
        late,
        w.cache.clone(),
    ));
    // one key at a time, so that only the shared key's pass takes the run
    let one_at_a_time = sagat_scheduler::SchedulerSettings {
        concurrency: 1,
        ..sagat_scheduler::SchedulerSettings::default()
    };
    let report = sagat_scheduler::Scheduler::new(
        store,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        one_at_a_time,
    )
    .with_children(w.cache.clone())
    .grant_gates()
    .await;
    assert_eq!(
        (waiting(run).await, waiting(other).await),
        (vec!["a".to_owned()], vec![]),
        "the run's second waiter and the other run are granted in the first pass: {report:?}"
    );
    assert!(
        lingering.lock().expect("the record").is_empty(),
        "a released connection still held the run: {:?}",
        lingering.lock().expect("the record")
    );
    for run in [run, other] {
        w.assert_consistent(run).await;
    }
}

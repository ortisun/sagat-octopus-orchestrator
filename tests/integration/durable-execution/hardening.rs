//! Bounds and isolation added by the security review of feature 002 (T002-035): what one tenant, one
//! worker or one run can make the platform do is limited, and each limit is exercised here.
//!
//! The assertions name the *behaviour* (a refusal with a typed error, a run that still ends, a query
//! that stays cheap), never an implementation detail, so a refactor that keeps the guarantee keeps
//! the test green and one that drops it does not.

use std::sync::Arc;
use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::execution::{MAX_PAYLOAD_BYTES, MAX_RUN_OUTPUT_BYTES};
use sagat_application::worker::{
    Capabilities, MAX_LABELS, MAX_POLL_BYTES, MAX_SESSIONS_PER_IDENTITY,
};
use sagat_domain::execution::{RunStatus, StepStatus};
use sagat_domain::identity::{Principal, Role};
use serde_json::{Value, json};

use crate::support::engine::{
    World, activity, document, parallel_node, subject_of, task_ref, wait_node, with, world,
};

/// A JSON object of about `bytes` bytes once serialised.
fn blob(bytes: usize) -> Value {
    json!({ "p": "x".repeat(bytes) })
}

fn len_of(value: &Value) -> usize {
    serde_json::to_vec(value).expect("json").len()
}

/// `s0 -> s1 -> ... -> s{n-1}`, every node an activity `act.s<i>`.
fn long_chain(name: &str, nodes: usize) -> (Value, Vec<String>) {
    let ids: Vec<String> = (0..nodes).map(|i| format!("s{i}")).collect();
    let all = ids
        .iter()
        .enumerate()
        .map(|(i, id)| match ids.get(i + 1) {
            Some(next) => with(activity(id), "next", json!(next)),
            None => activity(id),
        })
        .collect();
    let activities = ids.iter().map(|id| format!("act.{id}")).collect();
    (document(name, "s0", all), activities)
}

fn refs(names: &[String]) -> Vec<&str> {
    names.iter().map(String::as_str).collect()
}

// ------------------------------------------------------------- what a run may retain

#[tokio::test]
async fn a_run_cannot_retain_more_output_than_its_budget_and_the_step_fails_for_good() {
    let w = world().await;
    let (doc, activities) = long_chain("hoarder", 20);
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&refs(&activities)).await;

    let output = blob(240_000);
    let fits = MAX_RUN_OUTPUT_BYTES / len_of(&output);
    assert!(
        fits + 2 < 20,
        "the chain is long enough to cross the budget"
    );

    for n in 1..=fits {
        let task = w.one_task(&session).await;
        w.complete(&session, &task, &output)
            .await
            .unwrap_or_else(|e| panic!("completion {n} is within the budget: {e}"));
        assert_ne!(w.run_status(run).await, RunStatus::Failed, "after {n}");
    }

    // The next one would push what the run retains over the budget: it is accepted as a *failure*
    // of the step, with a code of its own and no retry (a retry would only exceed it again).
    let task = w.one_task(&session).await;
    let node = task.node.as_str().to_owned();
    w.complete(&session, &task, &output)
        .await
        .expect("the report itself is accepted");
    assert_eq!(w.step_status(run, &node).await, Some(StepStatus::Failed));
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    let (code, attempts): (Option<String>, i32) =
        sqlx::query_as("SELECT failure_code, attempt FROM steps WHERE run_id = $1 AND node = $2")
            .bind(run.as_uuid())
            .bind(&node)
            .fetch_one(&w.db.pool)
            .await
            .expect("step");
    assert_eq!(code.as_deref(), Some("RUN_DATA_LIMIT"));
    assert_eq!(attempts, 1, "the step was not retried");
    assert!(
        w.poll(&session, 4).await.is_empty(),
        "nothing is left to hand out"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_signal_that_would_exceed_the_budget_of_the_run_is_refused_and_changes_nothing() {
    let w = world().await;
    let ids: Vec<String> = (0..20).map(|i| format!("w{i}")).collect();
    let nodes = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let wait = wait_node(id, "e", "input.k", 300_000, None);
            match ids.get(i + 1) {
                Some(next) => with(wait, "next", json!(next)),
                None => wait,
            }
        })
        .collect();
    let (wf, _) = w.publish(document("signal-bomb", "w0", nodes)).await;
    let run = w.start(wf, &json!({"k": "x"})).await;

    let payload = serde_json::to_vec(&blob(240_000)).expect("json");
    let fits = MAX_RUN_OUTPUT_BYTES / payload.len();
    for n in 0..fits {
        w.runs
            .signal(
                &w.admin,
                run,
                "e",
                &payload,
                &format!("signal-key-{n:04}"),
                &crate::support::engine::ctx(),
            )
            .await
            .unwrap_or_else(|e| panic!("signal {n} is within the budget: {e}"));
    }
    let before = w.journal(run).await;
    let refused = w
        .runs
        .signal(
            &w.admin,
            run,
            "e",
            &payload,
            "signal-key-over",
            &crate::support::engine::ctx(),
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::QuotaExceeded(_))),
        "{refused:?}"
    );
    assert_eq!(w.journal(run).await, before, "the journal did not move");
    assert_eq!(w.run_status(run).await, RunStatus::Waiting);
    w.assert_consistent(run).await;
}

// ------------------------------------------------------------- what a poll may return

#[tokio::test]
async fn a_poll_is_bounded_in_bytes_and_the_rest_waits_for_the_next_one() {
    let w = world().await;
    let (wf, _) = w.publish(document("bulk", "a", vec![activity("a")])).await;
    let input = blob(MAX_PAYLOAD_BYTES - 1024);
    let total = 44;
    let mut runs = Vec::new();
    for _ in 0..total {
        runs.push(w.start(wf, &input).await);
    }
    let session = w.session(&["act.a"]).await;

    let first = w.poll(&session, 64).await;
    let bytes: usize = first.iter().map(|t| t.input.len()).sum();
    assert!(
        first.len() < total,
        "{} tasks in one response, {bytes} bytes",
        first.len()
    );
    assert!(
        bytes <= MAX_POLL_BYTES,
        "the serialized inputs fit the budget without a last-task overshoot: {bytes}"
    );

    // Nothing was lost: what the first poll left is handed out by the next ones.
    let mut seen: std::collections::BTreeSet<_> = first.iter().map(|t| t.run_id).collect();
    for _ in 0..4 {
        for task in w.poll(&session, 64).await {
            assert!(seen.insert(task.run_id), "a task was handed out twice");
        }
    }
    assert_eq!(
        seen.len(),
        total,
        "every task reached a worker exactly once"
    );
}

/// Compact numeric JSON expands when the task context is parsed and serialized: each `1e15`
/// becomes `1000000000000000.0`. Stored byte limits alone do not bound a worker response.
fn expanding_output() -> Vec<u8> {
    format!("[{}0]", "1e15,".repeat(48_000)).into_bytes()
}

#[tokio::test]
async fn a_serialized_task_over_the_poll_budget_fails_instead_of_staying_ready() {
    let w = world().await;
    let output = expanding_output();
    let expanded =
        serde_json::to_vec(&serde_json::from_slice::<Value>(&output).expect("valid numeric JSON"))
            .expect("canonical JSON");
    assert!(output.len() < MAX_PAYLOAD_BYTES);
    assert!(expanded.len() > output.len() * 3);
    let completed = MAX_POLL_BYTES / expanded.len() + 1;
    assert!(completed * output.len() < MAX_RUN_OUTPUT_BYTES);
    let (doc, activities) = long_chain("expanded-context", completed + 1);
    let (wf, _) = w.publish(doc).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&refs(&activities)).await;
    for n in 0..completed {
        let task = w.one_task(&session).await;
        assert!(task.input.len() <= MAX_POLL_BYTES);
        w.workers
            .complete(
                &session.principal,
                &task_ref(&task),
                &output,
                &format!("expanded-output-{n}"),
            )
            .await
            .expect("compact output is within the storage limit");
    }

    assert!(
        w.poll(&session, 64).await.is_empty(),
        "no oversized lease escapes"
    );
    let (status, code, attempt, token): (String, Option<String>, i32, Option<Vec<u8>>) =
        sqlx::query_as(
            "SELECT status, failure_code, attempt, lease_token_hash FROM steps \
             WHERE run_id = $1 AND node = $2",
        )
        .bind(run.as_uuid())
        .bind(format!("s{completed}"))
        .fetch_one(&w.db.pool)
        .await
        .expect("failed preflight");
    assert_eq!(status, "FAILED");
    assert_eq!(code.as_deref(), Some("RUN_DATA_LIMIT"));
    assert_eq!(
        attempt, 1,
        "a deterministic preflight failure is not retried"
    );
    assert!(token.is_none(), "there is no stranded live lease");
    assert_eq!(w.run_status(run).await, RunStatus::Failed);
    assert!(w.poll(&session, 64).await.is_empty());
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn large_sibling_tasks_are_budgeted_before_their_leases_commit() {
    let w = world().await;
    let branches = ["a", "b", "c", "d"];
    let mut nodes = vec![
        with(activity("s0"), "next", json!("s1")),
        with(activity("s1"), "next", json!("s2")),
        with(activity("s2"), "next", json!("fanout")),
        parallel_node("fanout", &branches, "all"),
    ];
    nodes.extend(branches.iter().map(|id| activity(id)));
    let (wf, _) = w.publish(document("large-siblings", "s0", nodes)).await;
    let run = w.start(wf, &json!({})).await;
    let session = w
        .session(&[
            "act.s0", "act.s1", "act.s2", "act.a", "act.b", "act.c", "act.d",
        ])
        .await;
    let output = expanding_output();
    for n in 0..3 {
        let task = w.one_task(&session).await;
        w.workers
            .complete(
                &session.principal,
                &task_ref(&task),
                &output,
                &format!("sibling-output-{n}"),
            )
            .await
            .expect("store output");
    }
    let first = w.poll(&session, 64).await;
    assert_eq!(first.len(), 3, "only three expanded contexts fit");
    assert!(first.iter().map(|task| task.input.len()).sum::<usize>() <= MAX_POLL_BYTES);
    let leased: i64 =
        sqlx::query_scalar("SELECT count(*) FROM steps WHERE run_id = $1 AND status = 'LEASED'")
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("leased count");
    assert_eq!(leased, 3, "every committed lease was returned");
    let last = w.one_task(&session).await;
    assert_eq!(
        last.fence.attempt, 1,
        "waiting for response space spends no attempt"
    );
    assert_eq!(last.fence.generation, 1);
    assert!(first.iter().all(|task| task.node != last.node));
    for task in first.iter().chain(std::iter::once(&last)) {
        w.complete(&session, task, &json!({}))
            .await
            .expect("finish branch");
    }
    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    w.assert_consistent(run).await;
}

// ------------------------------------------------------------- registration

fn capabilities(languages: Vec<String>) -> Capabilities {
    Capabilities {
        activities: vec!["act.a".into()],
        languages,
        pools: vec![],
        max_inflight: 10,
        protocol_version: "1".into(),
    }
}

#[tokio::test]
async fn what_a_worker_advertises_is_validated_and_bounded() {
    let w = world().await;
    let register = |languages: Vec<String>| {
        let caps = capabilities(languages);
        let w = &w;
        async move {
            w.workers
                .register(&w.worker, "payments", "validated", &caps)
                .await
        }
    };
    assert!(register(vec!["rust".into(), "c++".into()]).await.is_ok());
    for bad in [
        vec!["has space".to_owned()],
        vec![String::new()],
        vec!["x".repeat(65)],
        vec!["rust\n".to_owned()],
        (0..=MAX_LABELS).map(|i| format!("l{i}")).collect(),
    ] {
        let refused = register(bad.clone()).await;
        assert!(
            matches!(refused, Err(AppError::NotAllowed(_))),
            "{bad:?} -> {refused:?}"
        );
    }
    for bad in [
        Capabilities {
            activities: vec!["a".repeat(129)],
            ..capabilities(vec![])
        },
        Capabilities {
            activities: (0..=256).map(|i| format!("act.{i}")).collect(),
            ..capabilities(vec![])
        },
        Capabilities {
            pools: vec!["x".repeat(65)],
            ..capabilities(vec![])
        },
        Capabilities {
            pools: (0..=MAX_LABELS).map(|i| format!("p{i}")).collect(),
            ..capabilities(vec![])
        },
    ] {
        let refused = w
            .workers
            .register(&w.worker, "payments", "validated-bounds", &bad)
            .await;
        assert!(matches!(refused, Err(AppError::NotAllowed(_))), "{bad:?}");
    }
    for bad_name in ["x".repeat(121), "line\nbreak".to_owned()] {
        assert!(matches!(
            w.workers
                .register(&w.worker, "payments", &bad_name, &capabilities(vec![]))
                .await,
            Err(AppError::NotAllowed(_))
        ));
    }
}

#[tokio::test]
async fn an_identity_holds_a_bounded_number_of_sessions_and_the_same_name_replaces_its_session() {
    let w = world().await;
    let caps = capabilities(vec![]);
    let cap = usize::try_from(MAX_SESSIONS_PER_IDENTITY).expect("fits");
    let mut first = None;
    for i in 0..cap {
        let registered = w
            .workers
            .register(&w.worker, "payments", &format!("pool-{i}"), &caps)
            .await
            .unwrap_or_else(|e| panic!("worker {i} is within the cap: {e}"));
        if i == 0 {
            first = Some(registered);
        }
    }
    let first = first.expect("first");

    // One more *name* is refused with a typed error...
    let refused = w
        .workers
        .register(&w.worker, "payments", "one-too-many", &caps)
        .await;
    assert!(
        matches!(refused, Err(AppError::QuotaExceeded(_))),
        "{refused:?}"
    );

    // ...but a worker that reconnects under its own name is not "one more": it replaces its session
    // (the id stays, the token changes, the old token stops working), even at the cap.
    let again = w
        .workers
        .register(&w.worker, "payments", "pool-0", &caps)
        .await
        .expect("a reconnect at the cap");
    assert_eq!(again.worker_id, first.worker_id);
    assert_ne!(
        again.session_token.expose(),
        first.session_token.expose(),
        "a new token"
    );
    let stale = w
        .workers
        .poll(
            &w.worker,
            first.worker_id,
            first.session_token.expose(),
            1,
            None,
        )
        .await;
    assert_eq!(stale.map(|_| ()), Err(AppError::Unauthenticated));
    assert!(
        w.workers
            .poll(
                &w.worker,
                again.worker_id,
                again.session_token.expose(),
                1,
                None
            )
            .await
            .is_ok()
    );
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM worker_sessions")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(live, i64::from(MAX_SESSIONS_PER_IDENTITY));
}

// ------------------------------------------------------------- the claim query

#[tokio::test]
async fn the_claim_query_walks_the_ready_index_and_never_aggregates_the_backlog() {
    let w = world().await;
    // A plan is a property of the query and the schema, not of the data: with sequential scans
    // discouraged the planner shows the path it *can* take, on any database.
    let mut conn = w.db.pool.acquire().await.expect("connection");
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *conn)
        .await
        .expect("set");
    let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN {}",
        sagat_infra_postgres::CLAIMABLE_SQL
    )))
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(vec!["act.a".to_owned(), "act.b".to_owned()])
    .bind(8_i64)
    .bind(3_i32) // current negotiated worker minor; legacy compatibility is tested separately
    .fetch_all(&mut *conn)
    .await
    .expect("explain");
    let plan = plan.join("\n");
    assert!(plan.contains("steps_ready_idx"), "{plan}");
    assert!(
        !plan.contains("Aggregate"),
        "an aggregate reads every ready step of the tenant: {plan}"
    );
    assert!(plan.contains("Limit"), "each activity stops early: {plan}");
}

// ------------------------------------------------------------- a run that cannot be leased

async fn poison_leasing_of(w: &World, node: &str) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION poisoned() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'poisoned'; END $$; \
         CREATE TRIGGER poisoned BEFORE UPDATE ON steps FOR EACH ROW \
         WHEN (NEW.node = '{node}' AND NEW.status = 'LEASED') EXECUTE FUNCTION poisoned();"
    )))
    .execute(&w.db.pool)
    .await
    .expect("trigger");
}

#[tokio::test]
async fn a_run_that_cannot_be_leased_does_not_keep_the_others_from_a_worker() {
    let w = world().await;
    poison_leasing_of(&w, "poison").await;
    let (bad, _) = w
        .publish(document("bad", "poison", vec![activity("poison")]))
        .await;
    let (good, _) = w.publish(document("good", "a", vec![activity("a")])).await;
    // The poisoned run is the oldest, so it is the first candidate of every poll.
    let _ = w.start(bad, &json!({})).await;
    let healthy = [
        w.start(good, &json!({})).await,
        w.start(good, &json!({})).await,
    ];
    let session = w.session(&["act.poison", "act.a"]).await;

    let tasks = w.poll(&session, 8).await;
    let mut got: Vec<_> = tasks.iter().map(|t| t.run_id).collect();
    got.sort();
    let mut want = healthy.to_vec();
    want.sort();
    assert_eq!(got, want, "the healthy runs are served despite the bad one");

    // When *only* the bad run is left the worker is told (a typed error), not given an empty answer
    // that hides a stuck run.
    for task in &tasks {
        w.complete(&session, task, &json!({})).await.expect("done");
    }
    let alone = w
        .workers
        .poll(
            &session.principal,
            session.worker_id,
            &session.token,
            8,
            None,
        )
        .await;
    assert!(alone.is_err(), "{alone:?}");
}

#[tokio::test]
async fn a_task_leased_before_another_run_failed_is_still_delivered() {
    let w = world().await;
    poison_leasing_of(&w, "poison").await;
    let (good, _) = w.publish(document("good", "a", vec![activity("a")])).await;
    let (bad, _) = w
        .publish(document("bad", "poison", vec![activity("poison")]))
        .await;
    // Now the healthy run is first: it is leased and committed before the bad one is tried.
    let healthy = w.start(good, &json!({})).await;
    let _ = w.start(bad, &json!({})).await;
    let session = w.session(&["act.a", "act.poison"]).await;

    let tasks = w
        .workers
        .poll(
            &session.principal,
            session.worker_id,
            &session.token,
            8,
            None,
        )
        .await
        .expect("the lease that was committed is not thrown away with the error");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].run_id, healthy);
    assert_eq!(
        w.step_status(healthy, "a").await,
        Some(StepStatus::Leased),
        "and it is leased to this worker, whose token it now holds"
    );
}

// ------------------------------------------------------------- hints and the outbox

#[tokio::test]
async fn a_task_nobody_takes_is_announced_a_bounded_number_of_times() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("orphan", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    assert_eq!(
        w.outbox("task.ready").await,
        1,
        "announced when it became ready"
    );

    for _ in 0..12 {
        w.age_hint(run, "a").await;
        w.scheduler.tick().await;
    }
    assert_eq!(
        w.outbox("task.ready").await,
        1 + 5,
        "re-announced five times and then left to the pollers"
    );

    // A step that becomes ready *again* (a retry) starts a fresh allowance.
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.fail(&session, &task, true).await.expect("fail");
    w.age_retry(run, "a").await;
    w.scheduler.tick().await;
    assert_eq!(w.outbox("task.ready").await, 1 + 5 + 1);
    let hinted: i32 =
        sqlx::query_scalar("SELECT hint_count FROM steps WHERE run_id = $1 AND node = 'a'")
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(hinted, 0);
}

#[tokio::test]
async fn housekeeping_purges_a_backlog_larger_than_one_batch() {
    let w = world().await;
    let rows: i64 = 12_000;
    sqlx::query(
        "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload, delivered_at) \
         SELECT gen_random_uuid(), $1, gen_random_uuid(), 'task.ready', '{}', \
                clock_timestamp() - interval '30 days' FROM generate_series(1, $2)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(rows)
    .execute(&w.db.pool)
    .await
    .expect("delivered backlog");
    sqlx::query(
        "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
         VALUES (gen_random_uuid(), $1, gen_random_uuid(), 'task.ready', '{}')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("owed");

    w.scheduler.housekeeping().await;
    let left: Vec<(bool, i64)> = sqlx::query_as(
        "SELECT delivered_at IS NOT NULL, count(*) FROM event_outbox GROUP BY 1 ORDER BY 1",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(
        left,
        vec![(false, 1)],
        "one call cleared more than one batch, and kept what is still owed"
    );
}

// ------------------------------------------------------------- replays and leases

#[tokio::test]
async fn a_key_is_no_oracle_for_a_worker_that_does_not_hold_the_lease() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("oracle", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    let key = format!("done-{}", task.task_id());
    w.complete(&session, &task, &json!({"secret": 1}))
        .await
        .expect("complete");

    // A peer that knows the run, the node and the (guessable) key but not the token.
    let intruder = w.session(&["act.a"]).await;
    let mut forged = task_ref(&task);
    forged.lease_token = "not-the-token-of-this-lease";
    for (what, output, key) in [
        (
            "same key, same result",
            br#"{"secret":1}"#.as_slice(),
            key.as_str(),
        ),
        (
            "same key, other result",
            br#"{"secret":2}"#.as_slice(),
            key.as_str(),
        ),
        (
            "unused key",
            br#"{"secret":1}"#.as_slice(),
            "never-used-key",
        ),
    ] {
        let answer = w
            .workers
            .complete(&intruder.principal, &forged, output, key)
            .await;
        assert_eq!(
            answer.map(|_| ()),
            Err(AppError::StaleAttempt),
            "{what}: the same answer whether or not the key exists"
        );
    }

    // The holder still gets the precise answers.
    let replay = w.complete(&session, &task, &json!({"secret": 1})).await;
    assert!(replay.expect("replay").already_applied);
    let conflicting = w
        .workers
        .complete(
            &session.principal,
            &task_ref(&task),
            br#"{"secret":2}"#,
            &key,
        )
        .await;
    assert_eq!(conflicting.map(|_| ()), Err(AppError::IdempotencyKeyReused));
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_lease_renewed_after_the_candidate_was_read_is_not_reclaimed() {
    let w = Arc::new(world().await);
    let (wf, _) = w
        .publish(document("renewed", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let _task = w.one_task(&session).await;
    w.age_lease(run, "a").await;
    let before = w.journal(run).await;

    // Hold the run's lock so the scheduler reads its candidates (the lease looks expired) and then
    // waits for the lock; renew the lease in the meantime, as a late heartbeat would.
    let mut holder = w.db.pool.begin().await.expect("tx");
    sqlx::query("SELECT 1 FROM runs WHERE id = $1 FOR UPDATE")
        .bind(run.as_uuid())
        .execute(&mut *holder)
        .await
        .expect("lock");
    let tick = tokio::spawn({
        let w = Arc::clone(&w);
        async move { w.scheduler.tick().await }
    });
    let mut waiting = false;
    for _ in 0..200 {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("activity");
        if blocked > 0 {
            waiting = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(waiting, "the scheduler never reached the run's lock");
    sqlx::query(
        "UPDATE steps SET lease_expires_at = clock_timestamp() + interval '1 hour' \
         WHERE run_id = $1 AND node = 'a'",
    )
    .bind(run.as_uuid())
    .execute(&mut *holder)
    .await
    .expect("renew");
    holder.commit().await.expect("commit");

    let report = tick.await.expect("tick");
    assert_eq!(report.leases_expired, 0, "{report:?}");
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Leased));
    assert_eq!(w.journal(run).await, before, "nothing was written");

    // Control: once it really is over, the same pass reclaims it.
    w.age_lease(run, "a").await;
    assert_eq!(w.scheduler.tick().await.leases_expired, 1);
    w.assert_consistent(run).await;
}

// ------------------------------------------------------------- other tenants and secrets

#[tokio::test]
async fn a_worker_of_another_tenant_is_refused_and_the_attempt_is_audited() {
    let w = world().await;
    let (wf, _) = w.publish(document("mine", "a", vec![activity("a")])).await;
    let run = w.start(wf, &json!({})).await;
    let mine = w.session(&["act.a"]).await;
    let task = w.one_task(&mine).await;

    let foreign = Principal::new(
        w.other_admin.tenant_id(),
        subject_of("worker-b"),
        Role::Worker,
    );
    let _theirs = w.session_as(&foreign, &["act.a"], 10).await;
    let heartbeat = w.workers.heartbeat(&foreign, &task_ref(&task)).await;
    assert_eq!(heartbeat.map(|_| ()), Err(AppError::StaleAttempt));
    let completed = w
        .workers
        .complete(&foreign, &task_ref(&task), b"{}", "cross-tenant-key")
        .await;
    assert_eq!(completed.map(|_| ()), Err(AppError::StaleAttempt));

    // Denials are written off the response path, and the same denial (tenant, actor, resource) is
    // recorded once per window, so two attempts on one run leave one row, against the tenant that
    // made them and never against the owner.
    let audited = |tenant: sagat_domain::identity::TenantId| {
        let pool = w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM audit_records WHERE tenant_id = $1 \
                 AND action = 'access.denied.cross_tenant' AND resource_id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(run.to_string())
            .fetch_one(&pool)
            .await
            .expect("count")
        }
    };
    let foreign_tenant = w.other_admin.tenant_id();
    let rows = crate::support::eventually("the denial is audited", || async {
        let rows = audited(foreign_tenant).await;
        (rows > 0).then_some(rows)
    })
    .await;
    assert_eq!(rows, 1);
    assert_eq!(audited(w.admin.tenant_id()).await, 0);
    assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Leased));
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_lease_token_is_never_printed() {
    let w = world().await;
    let (wf, _) = w.publish(document("quiet", "a", vec![activity("a")])).await;
    let _ = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    let token = task.lease_token.expose().to_owned();
    for printed in [format!("{task:?}"), format!("{:?}", task_ref(&task))] {
        assert!(!printed.contains(&token), "{printed}");
    }
}

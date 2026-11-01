//! Crashes at every point between claim, effect, commit and publish, with an intact journal and no
//! double transition (SC-002-001, T002-017, T002-027, T002-031, NFR-002-DUR).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sagat_application::execution::{Actor, Effects, RunTx};
use sagat_application::runs::{Lock, ReviewAction, RunStore};
use sagat_domain::execution::{self, FailureCode, ReviewOutcome, RunStatus, StepStatus};
use serde_json::json;

use sagat_application::worker::WorkerSettings;

use crate::support::engine::{
    Session, World, activity, chain, document, flaky, with, world, world_with,
};

/// A stand-in for an external system that deduplicates by idempotency key, as a payment API does.
#[derive(Default, Clone)]
struct ExternalSystem {
    performed: Arc<Mutex<BTreeMap<String, u32>>>,
}

impl ExternalSystem {
    /// Performs the side effect unless this key already did; returns whether it ran.
    fn charge(&self, key: &str) -> bool {
        let mut performed = self.performed.lock().expect("lock");
        let count = performed.entry(key.to_owned()).or_insert(0);
        *count += 1;
        *count == 1
    }

    fn total_effects(&self) -> usize {
        self.performed.lock().expect("lock").len()
    }

    fn calls(&self, key: &str) -> u32 {
        self.performed
            .lock()
            .expect("lock")
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    fn duplicate_calls(&self) -> usize {
        self.performed
            .lock()
            .expect("lock")
            .values()
            .filter(|calls| **calls > 1)
            .count()
    }

    fn snapshot(&self) -> BTreeMap<String, u32> {
        self.performed.lock().expect("lock").clone()
    }
}

#[tokio::test]
async fn a_crash_after_the_external_effect_and_before_the_checkpoint_does_not_charge_twice() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("charge-once", "charge", vec![flaky("charge", 3)]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let bank = ExternalSystem::default();

    // Worker 1 performs the effect... and dies before reporting it.
    let first = w.session(&["act.charge"]).await;
    let attempt_one = w.one_task(&first).await;
    assert!(
        bank.charge(&attempt_one.idempotency_key),
        "the first attempt really charged"
    );

    w.age_lease(run, "charge").await;
    w.scheduler.tick().await;
    assert_eq!(w.run_status(run).await, RunStatus::ManualReview);
    assert!(
        w.poll(&first, 8).await.is_empty(),
        "the unknown charge is not leased a second time"
    );
    assert_eq!(
        w.complete(&first, &attempt_one, &json!({"charged": true}))
            .await
            .map(|_| ()),
        Err(sagat_application::error::AppError::StaleAttempt)
    );

    // The synthetic provider is authoritative for this fixture: it confirms the charge happened.
    w.review(
        run,
        "charge",
        ReviewAction::Resolve(ReviewOutcome::Succeeded),
        "the bank ledger confirms the charge completed",
    )
    .await
    .expect("resolve the uncertain charge");

    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    assert_eq!(bank.total_effects(), 1, "exactly one charge in the world");
    assert_eq!(
        bank.calls(&attempt_one.idempotency_key),
        1,
        "the platform did not repeat the unknown effect"
    );
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "step.resolved")
            .count(),
        1,
        "and journaled one outcome-bearing resolution"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_transaction_that_never_commits_leaves_no_trace() {
    let w = world().await;
    let (wf, _) = w.publish(chain("uncommitted")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;
    let before = snapshot(&w, run).await;

    // A command is fully executed and persisted inside a transaction... which is then abandoned,
    // exactly as when the process dies between the last statement and COMMIT.
    let mut tx = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("exists");
    let engine = sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator);
    let node = task.instance.node().clone();
    let fence = task.fence;
    let mut effects = Effects::by(Actor::Worker("doomed".into()), None);
    effects.outputs = vec![(task.instance.node().clone(), b"{}".to_vec())];
    engine
        .run(&mut tx, effects, move |def, state, now| {
            Ok(((), execution::complete_step(def, state, &node, fence, now)?))
        })
        .await
        .expect("the command itself succeeded");
    drop(tx); // no COMMIT

    assert_eq!(snapshot(&w, run).await, before, "nothing was written");
    w.assert_consistent(run).await;
    // And the worker's retry of the same report works as if the first never happened.
    let ack = w
        .complete(&session, &task, &json!({}))
        .await
        .expect("retry");
    assert!(!ack.already_applied);
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_connection_killed_in_the_middle_of_a_command_rolls_everything_back() {
    let w = world().await;
    let (wf, _) = w.publish(chain("killed-connection")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;
    let before = snapshot(&w, run).await;

    let mut tx = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("exists");
    let engine = sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator);
    let (node, fence) = (task.instance.node().clone(), task.fence);
    engine
        .run(
            &mut tx,
            Effects::by(Actor::Worker("doomed".into()), None),
            move |def, state, now| {
                Ok(((), execution::complete_step(def, state, &node, fence, now)?))
            },
        )
        .await
        .expect("persisted inside the transaction");

    // The database connection dies before COMMIT.
    let killed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = current_database() AND pid <> pg_backend_pid() AND state = 'idle in transaction') t",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("terminate");
    assert_eq!(killed, 1, "exactly the open transaction was killed");
    assert!(
        tx.commit().await.is_err(),
        "the commit cannot succeed on a dead connection"
    );

    assert_eq!(snapshot(&w, run).await, before);
    w.assert_consistent(run).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("the worker simply retries");
}

#[tokio::test]
async fn a_scheduler_interrupted_at_any_moment_never_applies_a_transition_twice() {
    let w = world().await;
    let doc = document(
        "interrupted",
        "pause",
        vec![
            with(
                json!({"id": "pause", "type": "timer", "durationMs": 1000}),
                "next",
                json!("after"),
            ),
            activity("after"),
        ],
    );
    let (wf, _) = w.publish(doc).await;
    let mut runs = Vec::new();
    for i in 0..12 {
        let run = w.start(wf, &json!({"i": i})).await;
        w.age_due(run, "pause").await;
        runs.push(run);
    }
    // Start a tick and abandon it after a random-ish few milliseconds, many times over.
    for ms in [0u64, 1, 2, 3, 5, 8, 13, 21, 34] {
        let tick = w.scheduler.tick();
        let _ = tokio::time::timeout(Duration::from_millis(ms), tick).await;
    }
    // A full tick completes whatever is left.
    while w.scheduler.tick().await.did_work() {}

    for run in runs {
        let fired = w
            .journal(run)
            .await
            .iter()
            .filter(|e| *e == "timer.fired")
            .count();
        assert_eq!(fired, 1, "the timer fired exactly once");
        assert_eq!(w.step_status(run, "after").await, Some(StepStatus::Ready));
        w.assert_consistent(run).await;
    }
}

#[tokio::test]
async fn killing_database_connections_while_workers_run_loses_nothing_and_duplicates_nothing() {
    // Short leases, so a task whose grant was committed but never delivered is recovered quickly.
    let w = world_with(WorkerSettings {
        lease_ttl: Duration::from_secs(1),
        ..WorkerSettings::default()
    })
    .await;
    let patient = |id: &str, next: Option<&str>| {
        let mut node = flaky(id, 30);
        if let Some(next) = next {
            node = with(node, "next", json!(next));
        }
        node
    };
    let (wf, _) = w
        .publish(document(
            "chaos",
            "a",
            vec![
                patient("a", Some("b")),
                patient("b", Some("c")),
                patient("c", None),
            ],
        ))
        .await;
    let runs_total = 24;
    let mut runs = Vec::new();
    for i in 0..runs_total {
        runs.push(w.start(wf, &json!({"i": i})).await);
    }
    let bank = ExternalSystem::default();

    // The storm: for three seconds, terminate every other backend over and over.
    let storm = {
        let pool = w.db.pool.clone();
        tokio::spawn(async move {
            let until = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < until {
                let _ = sqlx::query(
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                     WHERE datname = current_database() AND pid <> pg_backend_pid()",
                )
                .execute(&pool)
                .await;
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        })
    };

    // Four workers retry every operation until it goes through, as a real SDK would.
    let mut sessions: Vec<Session> = Vec::new();
    for i in 0..4 {
        sessions.push(register_with_retry(&w, i).await);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    let workers = futures_util::future::join_all(
        sessions.iter().map(|s| worker_loop(&w, s, &bank, deadline)),
    );
    let operator = resolve_unknown_effects(&w, &runs, &bank, deadline);
    let (done, operator_done) = tokio::join!(workers, operator);
    storm.await.expect("storm");
    assert!(
        done.iter().all(|d| *d),
        "a worker gave up before the work was done"
    );
    assert!(
        operator_done,
        "the operator gave up before the work was done"
    );
    // Every worker future has returned, so this is the final externally observable ledger. The
    // state/journal oracle below must agree with it exactly; no provider call can still arrive.
    let final_ledger = bank.snapshot();

    let mut unfinished = 0;
    for run in &runs {
        if !matches!(
            w.run_status(*run).await,
            RunStatus::Succeeded | RunStatus::Failed
        ) {
            unfinished += 1;
        }
    }
    assert_eq!(
        unfinished, 0,
        "every run reached a truthful terminal outcome"
    );

    let mut known_successes = 0usize;
    for run in &runs {
        // The journal is complete and contiguous, the projections equal it, and every retained
        // step outcome agrees with both its one settlement event and the provider's final ledger.
        w.assert_consistent(*run).await;
        let journal = w.journal(*run).await;
        assert_eq!(
            journal.iter().filter(|e| *e == "run.finished").count(),
            1,
            "{run}"
        );
        let mut failed_steps = 0usize;
        for node in ["a", "b", "c"] {
            let status = w
                .step_status(*run, node)
                .await
                .unwrap_or_else(|| panic!("run {run} retained no {node} step"));
            let key = format!("{run}:{node}");
            let provider_calls = final_ledger.get(&key).copied().unwrap_or(0);
            let settlements: Vec<(String, serde_json::Value)> = sqlx::query_as(
                "SELECT event_type, detail FROM run_events WHERE run_id = $1 AND node = $2 \
                 AND event_type IN ('step.completed', 'step.resolved') ORDER BY seq",
            )
            .bind(run.as_uuid())
            .bind(node)
            .fetch_all(&w.db.pool)
            .await
            .expect("settlement events");

            match status {
                StepStatus::Succeeded => {
                    known_successes += 1;
                    assert_eq!(provider_calls, 1, "run {run} node {node} succeeded");
                    assert_eq!(settlements.len(), 1, "run {run} node {node}");
                    let (event_type, detail) = &settlements[0];
                    assert!(
                        event_type == "step.completed"
                            || (event_type == "step.resolved" && detail["outcome"] == "succeeded"),
                        "run {run} node {node} has invalid success settlement {settlements:?}"
                    );
                }
                StepStatus::Failed => {
                    failed_steps += 1;
                    assert_eq!(provider_calls, 0, "run {run} node {node} failed");
                    assert_eq!(settlements.len(), 1, "run {run} node {node}");
                    let (event_type, detail) = &settlements[0];
                    assert_eq!(event_type, "step.resolved", "run {run} node {node}");
                    assert_eq!(detail["outcome"], "failed", "run {run} node {node}");
                    assert_eq!(
                        detail["code"], "OPERATOR_CONFIRMED_NOT_APPLIED",
                        "run {run} node {node}"
                    );
                }
                StepStatus::Cancelled | StepStatus::Pending => {
                    assert_eq!(provider_calls, 0, "run {run} node {node} did not run");
                    assert!(
                        settlements.is_empty(),
                        "run {run} node {node} settled despite status {status}"
                    );
                }
                other => panic!("run {run} node {node} retained non-terminal status {other}"),
            }
        }
        match w.run_status(*run).await {
            RunStatus::Succeeded => assert_eq!(failed_steps, 0, "successful run {run}"),
            RunStatus::Failed => assert_eq!(
                failed_steps, 1,
                "failed run {run} needs one explicit negative resolution"
            ),
            other => panic!("run {run} retained non-terminal status {other}"),
        }
    }
    assert_eq!(
        known_successes,
        bank.total_effects(),
        "successful retained steps and provider effects agree exactly"
    );
    assert_eq!(bank.duplicate_calls(), 0, "no provider effect was repeated");
    assert_eq!(
        bank.snapshot(),
        final_ledger,
        "the provider ledger changed after all worker loops finished"
    );
    let ids: BTreeSet<_> = runs.iter().collect();
    assert_eq!(ids.len(), runs_total);
}

/// Resolves only the outcomes this fixture's synthetic provider can answer. A lease committed but
/// never delivered has no provider call and is resolved failed; a call observed by the provider is
/// resolved succeeded. Database faults may abort a decision, so it is retried with fresh identity.
async fn resolve_unknown_effects(
    w: &World,
    runs: &[sagat_domain::execution::RunId],
    bank: &ExternalSystem,
    deadline: std::time::Instant,
) -> bool {
    // The database is being killed under this loop, so a review can fail transiently; the last
    // refusal is kept and printed when the deadline passes, instead of being discarded.
    let mut last_refusal: Option<String> = None;
    loop {
        if std::time::Instant::now() > deadline {
            eprintln!("unknown effects still open at the deadline; last refusal: {last_refusal:?}");
            return false;
        }
        for run in runs {
            let waiting: Result<Option<String>, _> = sqlx::query_scalar(
                "SELECT node FROM steps WHERE run_id = $1 AND status = 'WAITING' \
                 AND waiting_kind = 'OPERATOR' LIMIT 1",
            )
            .bind(run.as_uuid())
            .fetch_optional(&w.db.pool)
            .await;
            let Ok(Some(node)) = waiting else {
                continue;
            };
            let key = format!("{run}:{node}");
            let outcome = if bank.calls(&key) > 0 {
                ReviewOutcome::Succeeded
            } else {
                ReviewOutcome::Failed(
                    FailureCode::parse("OPERATOR_CONFIRMED_NOT_APPLIED")
                        .expect("valid review failure code"),
                )
            };
            if let Err(error) = w
                .review(
                    *run,
                    &node,
                    ReviewAction::Resolve(outcome),
                    "the synthetic provider ledger supplies the final outcome",
                )
                .await
            {
                last_refusal = Some(format!("{run}/{node}: {error:?}"));
            }
        }
        let open: Result<i64, _> =
            sqlx::query_scalar("SELECT count(*) FROM runs WHERE finished_at IS NULL")
                .fetch_one(&w.db.pool)
                .await;
        if matches!(open, Ok(0)) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// Performs the work for `session` until no ready step remains anywhere, retrying every call that
/// fails because the database was killed under it. Returns false if the deadline passes first.
async fn worker_loop(
    w: &World,
    session: &Session,
    bank: &ExternalSystem,
    deadline: std::time::Instant,
) -> bool {
    loop {
        if std::time::Instant::now() > deadline {
            return false;
        }
        w.scheduler.tick().await;
        let tasks = match w
            .workers
            .poll(
                &session.principal,
                session.worker_id,
                &session.token,
                4,
                None,
            )
            .await
        {
            Ok(tasks) => tasks,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
        };
        if tasks.is_empty() {
            let open: Result<i64, _> =
                sqlx::query_scalar("SELECT count(*) FROM runs WHERE finished_at IS NULL")
                    .fetch_one(&w.db.pool)
                    .await;
            if matches!(open, Ok(0)) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
            continue;
        }
        for task in tasks {
            bank.charge(&task.idempotency_key);
            // Report until the platform acknowledges (a replay counts as acknowledgement).
            for _ in 0..200 {
                match w.complete(session, &task, &json!({"ok": true})).await {
                    Ok(_) | Err(sagat_application::error::AppError::StaleAttempt) => break,
                    Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        }
    }
}

async fn register_with_retry(w: &World, index: u32) -> Session {
    for _ in 0..100 {
        let caps = sagat_application::worker::Capabilities {
            activities: vec!["act.a".into(), "act.b".into(), "act.c".into()],
            languages: vec![],
            pools: vec![],
            max_inflight: 50,
            protocol_version: "1".into(),
        };
        if let Ok(registered) = w
            .workers
            .register(
                &w.worker,
                "payments",
                &format!("chaos-worker-{index}"),
                &caps,
            )
            .await
        {
            return Session {
                principal: w.worker.clone(),
                worker_id: registered.worker_id,
                token: registered.session_token.expose().to_owned(),
            };
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("could not register a worker");
}

/// Everything observable about a run's rows, to compare before and after.
async fn snapshot(
    w: &World,
    run: sagat_domain::execution::RunId,
) -> (Vec<String>, Vec<(String, String, i32, i64)>, i64, i64) {
    let journal = w.journal(run).await;
    let steps: Vec<(String, String, i32, i64)> = sqlx::query_as(
        "SELECT node, status, attempt, generation FROM steps WHERE run_id = $1 ORDER BY node",
    )
    .bind(run.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("steps");
    let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM event_outbox")
        .fetch_one(&w.db.pool)
        .await
        .expect("outbox");
    let dedupe: i64 = sqlx::query_scalar("SELECT count(*) FROM command_dedupe")
        .fetch_one(&w.db.pool)
        .await
        .expect("dedupe");
    (journal, steps, outbox, dedupe)
}

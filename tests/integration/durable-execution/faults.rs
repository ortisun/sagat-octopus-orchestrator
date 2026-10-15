//! Crashes at every point between claim, effect, commit and publish, with an intact journal and no
//! double transition (SC-002-001, T002-017, T002-027, T002-031, NFR-002-DUR).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sagat_application::execution::{Actor, Effects, RunTx};
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::execution::{self, RunStatus, StepStatus};
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
    w.age_retry(run, "charge").await;
    w.scheduler.tick().await;

    // Worker 2 gets the same task again, with the SAME idempotency key, so the bank recognises it.
    let second = w.session(&["act.charge"]).await;
    let attempt_two = w.one_task(&second).await;
    assert_eq!(attempt_two.fence.attempt, 2);
    assert_eq!(attempt_two.idempotency_key, attempt_one.idempotency_key);
    assert!(
        !bank.charge(&attempt_two.idempotency_key),
        "the retry was recognised as a duplicate"
    );
    w.complete(&second, &attempt_two, &json!({"charged": true}))
        .await
        .expect("complete");

    assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
    assert_eq!(bank.total_effects(), 1, "exactly one charge in the world");
    assert_eq!(
        bank.calls(&attempt_one.idempotency_key),
        2,
        "the platform tried twice (at-least-once)"
    );
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "step.completed")
            .count(),
        1,
        "and journaled one completion"
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
    let done = futures_util::future::join_all(
        sessions.iter().map(|s| worker_loop(&w, s, &bank, deadline)),
    )
    .await;
    storm.await.expect("storm");
    assert!(
        done.iter().all(|d| *d),
        "a worker gave up before the work was done"
    );

    let mut unfinished = 0;
    for run in &runs {
        if w.run_status(*run).await != RunStatus::Succeeded {
            unfinished += 1;
        }
    }
    assert_eq!(unfinished, 0, "every run finished");

    for run in &runs {
        // The journal is complete and contiguous, the projections equal it, and each node
        // completed exactly once, whatever the storm did to individual attempts.
        w.assert_consistent(*run).await;
        let journal = w.journal(*run).await;
        assert_eq!(
            journal.iter().filter(|e| *e == "step.completed").count(),
            3,
            "{run}"
        );
        assert_eq!(
            journal.iter().filter(|e| *e == "run.finished").count(),
            1,
            "{run}"
        );
    }
    // At-least-once: attempts may have repeated, but every effect is keyed, so none was duplicated.
    assert_eq!(
        bank.total_effects(),
        runs_total * 3,
        "one effect per (run, node)"
    );
    let ids: BTreeSet<_> = runs.iter().collect();
    assert_eq!(ids.len(), runs_total);
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

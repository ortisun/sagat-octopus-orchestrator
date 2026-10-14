//! Races: who gets a task, who wins a completion, what happens when a lease expires mid-report
//! (T002-023, T002-027; US04; FR-002-004, NFR-002-DUR).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use sagat_application::error::AppError;
use sagat_application::runs::{Lock, RunStore};
use sagat_application::worker::Capabilities;
use sagat_domain::execution::{RunStatus, StepStatus};
use sagat_domain::quota::QuotaResource;
use serde_json::json;

use crate::support::engine::{activity, chain, document, task_ref, with, world};

async fn activity_limit(w: &crate::support::engine::World, activity: &str, limit: i64) {
    sqlx::query(
        "INSERT INTO activity_quota_limits \
         (tenant_id, namespace_id, activity, max_inflight, updated_by) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(activity)
    .bind(limit)
    .bind(w.admin.subject().as_str())
    .execute(&w.db.pool)
    .await
    .expect("activity quota");
}

#[tokio::test]
async fn sixteen_workers_racing_for_one_task_produce_exactly_one_lease() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("one-task", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;

    let mut sessions = Vec::new();
    for _ in 0..16 {
        sessions.push(w.session(&["act.a"]).await);
    }
    let polls = futures_util::future::join_all(sessions.iter().map(|s| w.poll(s, 4))).await;
    let granted: Vec<_> = polls.iter().flatten().collect();
    assert_eq!(granted.len(), 1, "one step, one lease: {}", granted.len());
    assert_eq!(
        (granted[0].fence.attempt, granted[0].fence.generation),
        (1, 1)
    );
    let leased: i64 = sqlx::query_scalar("SELECT count(*) FROM steps WHERE status = 'LEASED'")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(leased, 1);
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "step.leased")
            .count(),
        1,
        "the lease is journaled once"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn many_workers_and_many_tasks_never_assign_a_step_twice() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("fanout", "a", vec![activity("a")]))
        .await;
    let mut runs = Vec::new();
    for i in 0..30 {
        runs.push(w.start(wf, &json!({"i": i})).await);
    }
    let mut sessions = Vec::new();
    for _ in 0..8 {
        sessions.push(w.session(&["act.a"]).await);
    }
    // Everyone polls repeatedly until the queue is empty.
    let claimed: Vec<Vec<_>> = futures_util::future::join_all(sessions.iter().map(|s| async {
        let mut mine = Vec::new();
        for _ in 0..12 {
            let got = w.poll(s, 4).await;
            if got.is_empty() {
                break;
            }
            mine.extend(got);
        }
        mine
    }))
    .await;
    let all: Vec<_> = claimed.iter().flatten().collect();
    let unique: BTreeSet<_> = all
        .iter()
        .map(|t| (t.run_id, t.node.clone(), t.fence.generation))
        .collect();
    assert_eq!(all.len(), unique.len(), "a step was handed out twice");
    assert_eq!(
        unique.len(),
        30,
        "every ready step was handed out exactly once"
    );
    for run in runs {
        w.assert_consistent(run).await;
    }
}

#[tokio::test]
async fn concurrent_pollers_never_overshoot_an_activity_quota() {
    let w = world().await;
    activity_limit(&w, "act.a", 3).await;
    let (wf, _) = w
        .publish(document("quota-race", "a", vec![activity("a")]))
        .await;
    for i in 0..24 {
        w.start(wf, &json!({"i": i})).await;
    }
    let mut sessions = Vec::new();
    for _ in 0..12 {
        sessions.push(w.session(&["act.a"]).await);
    }
    let polls = futures_util::future::join_all(sessions.iter().map(|s| w.poll(s, 8))).await;
    assert_eq!(polls.iter().map(Vec::len).sum::<usize>(), 3);
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id = $1 AND namespace_id = $2 \
           AND activity = 'act.a' AND status IN ('LEASED', 'RUNNING') \
           AND lease_expires_at > clock_timestamp()",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("held leases");
    assert_eq!(held, 3, "the exact quota is never exceeded under a race");
}

#[tokio::test]
async fn concurrent_claims_on_one_session_are_exact_across_service_instances() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("session-cap-race", "a", vec![activity("a")]))
        .await;
    for i in 0..24 {
        w.start(wf, &json!({"i": i})).await;
    }
    let session = w.session_as(&w.worker, &["act.a"], 2).await;
    let first = w.workers.clone();
    let second = w.workers.clone();
    let polls = futures_util::future::join_all((0..16).map(|index| {
        let workers = if index % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        let principal = session.principal.clone();
        let token = session.token.clone();
        async move {
            workers
                .poll(&principal, session.worker_id, &token, 8, None)
                .await
                .expect("concurrent claim")
        }
    }))
    .await;
    assert_eq!(
        polls.iter().map(Vec::len).sum::<usize>(),
        2,
        "the durable session fence must admit exactly max_inflight leases"
    );
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id = $1 \
         AND status IN ('LEASED', 'RUNNING') AND leased_by = $2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(session.worker_id.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("active session leases");
    assert_eq!(active, 2);
}

#[tokio::test]
async fn session_replacement_revalidates_the_token_and_lowered_limit_at_the_claim_fence() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("session-replacement", "a", vec![activity("a")]))
        .await;
    for i in 0..8 {
        w.start(wf, &json!({"i": i})).await;
    }
    w.bind_everything(&w.worker).await;
    let capabilities = |max_inflight| Capabilities {
        activities: vec!["act.a".into()],
        languages: vec!["rust".into()],
        pools: Vec::new(),
        max_inflight,
        protocol_version: "1.4".into(),
    };
    let original = w
        .workers
        .register(&w.worker, "payments", "replacement-fence", &capabilities(8))
        .await
        .expect("original session");

    // Hold the drain fence after the old poll has selected and locked its run, but before it can
    // lock and revalidate the worker session. This makes the stale-snapshot race deterministic.
    let drain_key = format!(
        "drain:{}:{}:{}",
        w.admin.tenant_id(),
        w.namespace.id(),
        w.worker.subject()
    );
    let mut fence = w.db.pool.begin().await.expect("begin drain fence");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&drain_key)
        .execute(&mut *fence)
        .await
        .expect("hold drain fence");
    let fence_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *fence)
        .await
        .expect("fence backend");
    let workers = w.workers.clone();
    let principal = w.worker.clone();
    let old_worker_id = original.worker_id;
    let old_token = original.session_token.expose().to_owned();
    let old_claim = tokio::spawn(async move {
        workers
            .poll(&principal, old_worker_id, &old_token, 8, None)
            .await
    });
    let waiting = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity waiter \
                 WHERE waiter.datname = current_database() \
                   AND $1 = ANY(pg_blocking_pids(waiter.pid)))",
            )
            .bind(fence_pid)
            .fetch_one(&w.db.pool)
            .await
            .expect("observe blocked claim");
            if blocked {
                break;
            }
            assert!(
                !old_claim.is_finished(),
                "old claim did not reach the fence"
            );
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(waiting.is_ok(), "old claim reached the drain fence");

    let replacement = w
        .workers
        .register(&w.worker, "payments", "replacement-fence", &capabilities(1))
        .await
        .expect("replacement session");
    assert_eq!(original.worker_id, replacement.worker_id);
    fence.commit().await.expect("release drain fence");
    let stale = old_claim
        .await
        .expect("old claim task")
        .expect("the prevalidated request completes without a lease");
    assert!(
        stale.is_empty(),
        "the transactional recheck rejects authority replaced after the initial validation"
    );
    let claimed = w
        .workers
        .poll(
            &w.worker,
            replacement.worker_id,
            replacement.session_token.expose(),
            8,
            None,
        )
        .await
        .expect("replacement claim");
    assert_eq!(
        claimed.len(),
        1,
        "the stored lowered limit is authoritative"
    );
}

#[tokio::test]
async fn a_lowered_platform_ceiling_applies_to_an_existing_activity_override() {
    let w = world().await;
    activity_limit(&w, "act.a", 3).await;
    w.quotas
        .grant_ceiling(
            w.admin.tenant_id(),
            QuotaResource::ActivityConcurrency,
            Some(1),
        )
        .await
        .expect("lower activity ceiling");
    let (wf, _) = w
        .publish(document("quota-ceiling", "a", vec![activity("a")]))
        .await;
    for i in 0..3 {
        w.start(wf, &json!({"i": i})).await;
    }
    let session = w.session(&["act.a"]).await;
    assert_eq!(
        w.poll(&session, 8).await.len(),
        1,
        "lowering the operator ceiling applies to an override stored earlier"
    );
}

#[tokio::test]
async fn a_full_activity_is_skipped_and_an_expired_lease_returns_its_place() {
    let w = world().await;
    activity_limit(&w, "act.a", 1).await;
    let (a, _) = w
        .publish(document("quota-a", "a", vec![activity("a")]))
        .await;
    let (b, _) = w
        .publish(document("quota-b", "b", vec![activity("b")]))
        .await;
    for i in 0..3 {
        w.start(a, &json!({"i": i})).await;
    }
    w.start(b, &json!({})).await;
    let session = w.session_as(&w.worker, &["act.a", "act.b"], 8).await;
    let tasks = w.poll(&session, 8).await;
    assert_eq!(
        tasks.len(),
        2,
        "one capped activity and the uncapped activity are served"
    );
    assert!(tasks.iter().any(|task| task.activity.as_str() == "act.a"));
    assert!(tasks.iter().any(|task| task.activity.as_str() == "act.b"));

    sqlx::query(
        "UPDATE steps SET lease_expires_at = clock_timestamp() - interval '1 second' \
         WHERE tenant_id = $1 AND namespace_id = $2 AND activity = 'act.a' AND status = 'LEASED'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("expire lease");
    let next = w.poll(&session, 8).await;
    assert_eq!(
        next.iter()
            .filter(|task| task.activity.as_str() == "act.a")
            .count(),
        1
    );
}

#[tokio::test]
async fn a_poll_never_waits_for_a_run_that_another_transaction_holds() {
    let w = world().await;
    let (wf, _) = w.publish(chain("locked")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;

    // Something else (an operator command, a timer) holds the run's row lock.
    let held = w
        .store
        .begin(w.admin.tenant_id(), run, Lock::Wait)
        .await
        .expect("begin")
        .expect("exists");
    let started = Instant::now();
    let tasks = w.poll(&session, 4).await;
    assert!(tasks.is_empty(), "the locked run is skipped");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "polling blocked for {:?}",
        started.elapsed()
    );
    drop(held); // rolls back

    assert_eq!(
        w.poll(&session, 4).await.len(),
        1,
        "and picked up as soon as it is free"
    );
}

#[tokio::test]
async fn eight_simultaneous_reports_of_one_completion_apply_it_once() {
    let w = world().await;
    let (wf, _) = w.publish(chain("storm")).await;
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;

    let output = json!({"ok": true});
    let acks =
        futures_util::future::join_all((0..8).map(|_| w.complete(&session, &task, &output))).await;
    let applied = acks
        .iter()
        .filter(|a| matches!(a, Ok(ack) if !ack.already_applied))
        .count();
    let replays = acks
        .iter()
        .filter(|a| matches!(a, Ok(ack) if ack.already_applied))
        .count();
    assert_eq!((applied, replays), (1, 7), "{acks:?}");
    assert_eq!(
        w.journal(run)
            .await
            .iter()
            .filter(|e| *e == "step.completed")
            .count(),
        1
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_completion_racing_a_lease_expiry_has_exactly_one_winner() {
    // Whichever commits first decides; the loser is told so, and the run stays consistent.
    let w = world().await;
    let (wf, _) = w
        .publish(document(
            "photo-finish",
            "a",
            vec![with(activity("a"), "next", json!("b")), activity("b")],
        ))
        .await;
    let session = w.session(&["act.a", "act.b"]).await;
    let mut completions_won = 0;
    for round in 0..8 {
        let run = w.start(wf, &json!({})).await;
        let task = w.one_task(&session).await;
        w.age_lease(run, "a").await;

        let output = json!({});
        let (completion, tick) =
            tokio::join!(w.complete(&session, &task, &output), w.scheduler.tick());
        let completed = completion.is_ok();
        let expired = tick.leases_expired == 1;
        assert!(
            completed ^ expired,
            "round {round}: completed={completed} expired={expired} ({completion:?}, {tick:?})"
        );
        if completed {
            completions_won += 1;
            assert_eq!(w.step_status(run, "a").await, Some(StepStatus::Succeeded));
        } else {
            assert_eq!(completion.map(|_| ()), Err(AppError::StaleAttempt));
            assert_eq!(w.step_status(run, "a").await, Some(StepStatus::RetryWait));
        }
        w.assert_consistent(run).await;
        // Leave nothing ready behind for the next round to trip over.
        w.operate(run, sagat_application::runs::RunOperation::Cancel)
            .await
            .expect("cancel");
        w.assert_consistent(run).await;
    }
    let _ = completions_won; // either side may win a round; both outcomes are valid
}

#[tokio::test]
async fn a_worker_never_holds_more_tasks_than_it_declared() {
    let w = world().await;
    let (wf, _) = w
        .publish(document("limited", "a", vec![activity("a")]))
        .await;
    for i in 0..5 {
        w.start(wf, &json!({"i": i})).await;
    }
    let session = w.session_as(&w.worker, &["act.a"], 2).await;
    assert_eq!(
        w.poll(&session, 10).await.len(),
        2,
        "capped by max_inflight"
    );
    assert!(w.poll(&session, 10).await.is_empty(), "already at capacity");

    let another = w.session_as(&w.worker, &["act.a"], 10).await;
    assert_eq!(
        w.poll(&another, 10).await.len(),
        3,
        "the rest goes to someone with room"
    );
}

#[tokio::test]
async fn a_worker_only_receives_the_activities_it_advertised() {
    let w = world().await;
    let (wf, _) = w.publish(chain("capabilities")).await;
    let run = w.start(wf, &json!({})).await;
    let wrong = w.session(&["act.other"]).await;
    assert!(w.poll(&wrong, 10).await.is_empty());
    let right = w.session(&["act.a"]).await;
    assert_eq!(w.one_task(&right).await.node.as_str(), "a");
    assert_eq!(w.run_status(run).await, RunStatus::Running);
}

#[tokio::test]
async fn heartbeats_from_a_worker_that_lost_its_lease_do_not_resurrect_it() {
    let w = world().await;
    let (wf, _) = w.publish(chain("zombie")).await;
    let run = w.start(wf, &json!({})).await;
    let zombie = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&zombie).await;
    w.age_lease(run, "a").await;
    w.scheduler.tick().await;

    assert_eq!(
        w.workers
            .heartbeat(&zombie.principal, &task_ref(&task))
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    assert_eq!(
        w.step_status(run, "a").await,
        Some(StepStatus::RetryWait),
        "a late heartbeat changed nothing"
    );
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn a_capped_backlog_cannot_hide_another_activity_past_the_candidate_limit() {
    let w = world().await;
    activity_limit(&w, "act.a", 0).await;
    let (blocked, _) = w
        .publish(document("capped-backlog", "a", vec![activity("a")]))
        .await;
    for i in 0..65 {
        w.start(blocked, &json!({"i":i})).await;
    }
    let (available, _) = w
        .publish(document("available", "b", vec![activity("b")]))
        .await;
    let run = w.start(available, &json!({})).await;
    let session = w.session_as(&w.worker, &["act.a", "act.b"], 8).await;
    let tasks = w.poll(&session, 1).await;
    assert_eq!(
        tasks.len(),
        1,
        "a capped oldest backlog cannot monopolize candidate selection"
    );
    assert_eq!(tasks[0].run_id, run);
    assert_eq!(tasks[0].activity.as_str(), "act.b");
}

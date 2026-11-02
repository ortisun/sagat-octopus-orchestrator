//! Workers lost with their leases: BL05 retains uncertain effects for review while the
//! surviving workers finish independent work. No lost original is silently repeated (037).

use std::time::Duration;

use sagat_application::error::AppError;
use sagat_domain::execution::RunStatus;
use sagat_scheduler::SchedulerSettings;
use serde_json::json;

use crate::support::engine::{chain, world};
use crate::support::scale::{TTL, keeper, scheduler};

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn when_three_workers_in_ten_die_their_unknown_work_is_retained_without_a_second_attempt() {
    let w = world().await;
    let (wf, _) = w.publish(chain("chaos-workers")).await;
    let mut runs = Vec::new();
    for i in 0..60 {
        runs.push(w.start(wf, &json!({ "i": i })).await);
    }

    // Two engine replicas share the shards.
    let (alpha, beta) = (keeper(&w, "alpha", TTL), keeper(&w, "beta", TTL));
    for _ in 0..3 {
        alpha.round().await.expect("alpha");
        beta.round().await.expect("beta");
    }
    let schedulers = [
        scheduler(
            &w,
            SchedulerSettings::default(),
            Some(alpha.placement().clone()),
        ),
        scheduler(
            &w,
            SchedulerSettings::default(),
            Some(beta.placement().clone()),
        ),
    ];

    // Ten workers each take what they can; three of them then die holding their tasks.
    let mut sessions = Vec::new();
    for _ in 0..10 {
        sessions.push(w.session(&["act.a", "act.b"]).await);
    }
    let mut held = Vec::new();
    for session in &sessions {
        held.push(w.poll(session, 6).await);
    }
    let dead: Vec<usize> = vec![0, 1, 2];
    let lost: usize = dead.iter().map(|i| held[*i].len()).sum();
    assert!(
        lost > 0,
        "the dead workers held nothing: nothing was tested"
    );

    let lost_runs: std::collections::BTreeSet<_> = dead
        .iter()
        .flat_map(|i| held[*i].iter().map(|task| task.run_id))
        .collect();
    assert_eq!(lost_runs.len(), lost, "one original lease per affected run");

    // The survivors finish what they hold; the dead ones never answer.
    for (i, session) in sessions.iter().enumerate() {
        if dead.contains(&i) {
            continue;
        }
        for task in &held[i] {
            w.complete(session, task, &json!({}))
                .await
                .expect("complete");
        }
    }
    // Their leases run out; replicas fence them and retain the unknown effects.
    for i in &dead {
        for task in &held[*i] {
            w.age_lease(task.run_id, task.instance.node().as_str())
                .await;
        }
    }
    let mut expired = 0;
    for scheduler in &schedulers {
        expired += scheduler.tick().await.leases_expired;
    }
    assert_eq!(
        usize::try_from(expired).expect("fits"),
        lost,
        "every lost attempt was fenced once"
    );
    for scheduler in &schedulers {
        scheduler.tick().await;
    }

    // The survivors finish only independent runs; held reviews release no new tasks.
    let survivors: Vec<_> = sessions
        .iter()
        .enumerate()
        .filter(|(i, _)| !dead.contains(i))
        .map(|(_, s)| s)
        .collect();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let mut worked = false;
        for session in &survivors {
            for task in w.poll(session, 16).await {
                assert!(
                    !lost_runs.contains(&task.run_id),
                    "an uncertain run dispatched work"
                );
                w.complete(session, &task, &json!({}))
                    .await
                    .expect("complete");
                worked = true;
            }
        }
        for scheduler in &schedulers {
            worked |= scheduler.tick().await.did_work();
        }
        if !worked {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the work did not finish"
        );
    }
    for run in &runs {
        let expected = if lost_runs.contains(run) {
            RunStatus::ManualReview
        } else {
            RunStatus::Succeeded
        };
        assert_eq!(w.run_status(*run).await, expected, "run {run}");
        if lost_runs.contains(run) {
            let (reviews, leases, retries, held): (i64, i64, i64, i64) = sqlx::query_as(
                "SELECT \
                 (SELECT count(*) FROM steps WHERE run_id=$1 AND status='WAITING' \
                    AND waiting_kind='OPERATOR' AND failure_code='LEASE_EXPIRED'), \
                 (SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='step.leased'), \
                 (SELECT count(*) FROM run_events WHERE run_id=$1 \
                    AND event_type IN ('step.retry_scheduled','step.retry_due','step.retry_requested')), \
                 (SELECT count(*) FROM steps WHERE run_id=$1 \
                    AND (status IN ('READY','LEASED','RUNNING','RETRY_WAIT') OR lease_token_hash IS NOT NULL))",
            )
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("review retains evidence without worker capacity or repeated work");
            assert_eq!((reviews, leases, retries, held), (1, 1, 0, 0));
        }
        w.assert_consistent(*run).await;
    }

    // A late report cannot overwrite the retained review.
    for i in &dead {
        for task in &held[*i] {
            assert_eq!(
                w.complete(&sessions[*i], task, &json!({}))
                    .await
                    .map(|_| ()),
                Err(AppError::StaleAttempt)
            );
        }
    }
}

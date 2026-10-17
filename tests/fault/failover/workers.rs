//! Workers lost with their leases: BL05, a share of the workers dies and the platform takes their
//! work back and finishes it.

use std::time::Duration;

use sagat_application::error::AppError;
use sagat_domain::execution::RunStatus;
use sagat_scheduler::SchedulerSettings;
use serde_json::json;

use crate::support::engine::{chain, world};
use crate::support::scale::{TTL, keeper, scheduler};

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn when_three_workers_in_ten_die_their_work_is_taken_back_and_nothing_is_lost_or_done_twice()
{
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
    // Their leases run out and the replicas take the attempts back.
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
        "every lost attempt was taken back, once"
    );
    for i in &dead {
        for task in &held[*i] {
            w.age_retry(task.run_id, task.instance.node().as_str())
                .await;
        }
    }
    for scheduler in &schedulers {
        scheduler.tick().await;
    }

    // The survivors do the rest: the retried attempts and every next step.
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
        assert_eq!(w.run_status(*run).await, RunStatus::Succeeded, "run {run}");
        w.assert_consistent(*run).await;
    }

    // A dead worker that comes back with its old lease is refused: the attempt is somebody else's.
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

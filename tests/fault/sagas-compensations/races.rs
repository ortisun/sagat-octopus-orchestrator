//! Races the saga machinery must settle exactly one way: two schedulers noticing the same lost
//! lease, a lease that runs out while the answer is arriving, and a timeout against a signal.

use futures_util::future::join_all;
use sagat_domain::execution::{RunStatus, StepStatus};
use serde_json::json;

use crate::common::{quick, settlement, worker};
use crate::support::engine::{
    activity, document, effect_activities, effect_node, saga_node, wait_node, world,
};

#[tokio::test]
async fn schedulers_that_notice_the_same_lost_lease_at_once_request_one_reconciliation() {
    let w = world().await;
    let (wf, _) = w.publish(settlement()).await;
    let s = worker(&w).await;
    let mut runs = Vec::new();
    for _ in 0..6 {
        let run = w.start(wf, &json!({})).await;
        runs.push(run);
    }
    // Every run reserved and has `submit` leased, then loses it.
    for _ in 0..6 {
        for task in w.poll(&s, 1).await {
            w.complete(&s, &task, &json!({})).await.expect("reserve");
        }
    }
    for task in w.poll(&s, 8).await {
        assert_eq!(task.activity.as_str(), "act.submit");
    }
    for run in &runs {
        w.age_lease(*run, "submit").await;
    }

    // Eight schedulers look at the same six lost leases at the same instant.
    let reports = join_all((0..8).map(|_| w.scheduler.tick())).await;
    let expired: u32 = reports.iter().map(|r| r.leases_expired).sum();
    assert_eq!(
        expired, 6,
        "each lost lease is handled by exactly one of them"
    );
    for run in &runs {
        let journal = w.journal(*run).await;
        let requested = journal
            .iter()
            .filter(|e| *e == "reconciliation.requested")
            .count();
        assert_eq!(requested, 1, "one lookup per lost attempt: {journal:?}");
        assert_eq!(
            w.step_status(*run, "submit~r").await,
            Some(StepStatus::Ready)
        );
        w.assert_consistent(*run).await;
    }
}

#[tokio::test]
async fn a_lease_that_runs_out_while_the_answer_arrives_is_decided_once() {
    let w = world().await;
    let (wf, _) = w.publish(quick(settlement())).await;
    let s = worker(&w).await;
    let mut both = 0;
    let mut neither = 0;
    for _ in 0..8 {
        let run = w.start(wf, &json!({})).await;
        let reserve = w.one_task(&s).await;
        w.complete(&s, &reserve, &json!({})).await.expect("reserve");
        let _lost = w.one_task(&s).await;
        w.age_lease(run, "submit").await;
        w.scheduler.tick().await;
        let lookup = w.one_task(&s).await;
        w.age_lease(run, "submit~r").await;

        let output = json!({"effect": "applied"});
        let (answer, tick) = tokio::join!(w.complete(&s, &lookup, &output), w.scheduler.tick());
        let answered = answer.is_ok();
        let expired = tick.leases_expired == 1;
        if answered && expired {
            both += 1;
        }
        if !answered && !expired {
            neither += 1;
        }
        // Whoever won, the journal holds exactly one outcome of this attempt and replays.
        w.assert_consistent(run).await;
        let journal = w.journal(run).await;
        let concluded = journal
            .iter()
            .filter(|e| *e == "reconciliation.concluded")
            .count();
        assert!(concluded <= 1, "{journal:?}");
        if answered {
            assert_eq!(concluded, 1);
            assert_eq!(
                w.step_status(run, "submit").await,
                Some(StepStatus::Succeeded)
            );
        }
        // Drain whatever is left so the next round starts clean.
        for task in w.poll(&s, 8).await {
            let _ = w.answer_reconciliation(&s, &task, "applied", None).await;
        }
    }
    assert_eq!(both, 0, "an answer and an expiry never both take effect");
    assert_eq!(neither, 0, "one of them always does");
}

/// A saga whose body waits for a confirmation that may never come: reserve → wait → (after).
fn confirmed() -> serde_json::Value {
    document(
        "confirmed",
        "tx",
        vec![
            saga_node("tx", "reserve", Some("after")),
            effect_node("reserve", Some("confirm"), false),
            wait_node("confirm", "confirmed", "input.k", 60_000, None),
            activity("after"),
        ],
    )
}

#[tokio::test]
async fn a_signal_and_a_timeout_arriving_together_decide_the_saga_exactly_one_way() {
    let w = world().await;
    let (wf, _) = w.publish(confirmed()).await;
    let mut names = effect_activities(&["reserve"]);
    names.push("act.after".to_owned());
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = w.session(&names).await;
    let mut signalled = 0;
    let mut timed_out = 0;
    for round in 0..12 {
        let run = w.start(wf, &json!({"k": round})).await;
        let reserve = w.one_task(&s).await;
        w.complete(&s, &reserve, &json!({})).await.expect("reserve");
        assert_eq!(w.run_status(run).await, RunStatus::Waiting);
        w.age_due(run, "confirm").await;

        let key = format!("signal-{round}-0000001");
        let ctx = crate::support::engine::ctx();
        let (signal, _tick) = tokio::join!(
            w.runs
                .signal(&w.admin, run, "confirmed", br#"{"ok":true}"#, &key, &ctx),
            w.scheduler.tick()
        );
        let journal = w.journal(run).await;
        let satisfied = journal.iter().filter(|e| *e == "wait.satisfied").count();
        let expired = journal.iter().filter(|e| *e == "wait.timed_out").count();
        assert_eq!(satisfied + expired, 1, "exactly one decides: {journal:?}");
        if satisfied == 1 {
            // The signal won: the saga goes on and the run succeeds.
            signalled += 1;
            assert!(signal.is_ok());
            let after = w.one_task(&s).await;
            assert_eq!(after.activity.as_str(), "act.after");
            w.complete(&s, &after, &json!({})).await.expect("after");
            assert_eq!(w.run_status(run).await, RunStatus::Succeeded);
        } else {
            // The timeout won: the body failed, so the saga undoes the reservation.
            timed_out += 1;
            assert!(signal.is_err(), "nobody is waiting for the signal any more");
            assert_eq!(w.run_status(run).await, RunStatus::Compensating);
            let undo = w.one_task(&s).await;
            assert_eq!(undo.activity.as_str(), "undo.reserve");
            w.complete(&s, &undo, &json!({})).await.expect("undo");
            assert_eq!(w.run_status(run).await, RunStatus::Compensated);
        }
        w.assert_consistent(run).await;
    }
    assert_eq!(signalled + timed_out, 12);
}

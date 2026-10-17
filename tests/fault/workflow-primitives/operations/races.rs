//! An Update against the other writers of a run: an activity completion, a timer and a wait
//! timeout. Both commit orders are forced through the database's lock queue (see `engine::race`),
//! and in each the Update and the other writer both commit, the journal says which came first, and
//! the projections equal a replay of the journal.

use sagat_domain::execution::{StepStatus, UpdateReceiptStatus};
use serde_json::json;

use super::engine::{
    Declared, declared, input, journal, position, race, replace, start, update_evidence,
};
use crate::support::engine::{activity, timer_node, wait_node, with};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    UpdateFirst,
    UpdateSecond,
}

const ORDERS: [Order; 2] = [Order::UpdateFirst, Order::UpdateSecond];

fn after_node() -> serde_json::Value {
    activity("after")
}

/// `work -> hold`: the run stays alive after the completion, so an Update can still follow it.
fn worker_document(name: &str) -> serde_json::Value {
    declared(
        name,
        "work",
        vec![
            with(activity("work"), "next", json!("hold")),
            activity("hold"),
        ],
    )
}

fn timer_document(name: &str) -> serde_json::Value {
    declared(
        name,
        "pause",
        vec![
            with(timer_node("pause", 3_600_000), "next", json!("after")),
            after_node(),
        ],
    )
}

fn wait_document(name: &str) -> serde_json::Value {
    declared(
        name,
        "gate",
        vec![
            wait_node(
                "gate",
                "account.release",
                "input.key",
                3_600_000,
                Some("after"),
            ),
            after_node(),
        ],
    )
}

/// What every race leaves behind, whatever the order: one accepted Update, one receipt, one
/// `state.updated` event and outbox row, and projections equal to a replay of the journal.
async fn assert_update_committed_once(fixture: &Declared, expected_revision: i64) {
    assert_eq!(
        update_evidence(&fixture.w, fixture.run).await,
        (expected_revision, 1, 1, 1)
    );
    fixture.w.assert_consistent(fixture.run).await;
}

#[tokio::test]
async fn an_update_and_a_worker_completion_both_commit_in_either_order() {
    for order in ORDERS {
        let fixture = start(worker_document("operations-worker-race"), &input(10)).await;
        let session = fixture.w.session(&["act.work", "act.hold"]).await;
        let task = fixture.w.one_task(&session).await;
        let run = fixture.run;
        let update_args = replace(41);
        let update =
            fixture
                .ops
                .update(run, "balance.observe", "worker-race", &update_args, Some(0));
        let output = json!({"amount": 7});
        let completion = fixture.w.complete(&session, &task, &output);

        let (answer, completed) = match order {
            Order::UpdateFirst => {
                let (answer, completed) = race(&fixture.w, run, update, completion).await;
                (answer, completed)
            }
            Order::UpdateSecond => {
                let (completed, answer) = race(&fixture.w, run, completion, update).await;
                (answer, completed)
            }
        };
        let answer = answer.expect("the update commits");
        completed.expect("the completion commits");

        assert_eq!(answer.status(), UpdateReceiptStatus::Applied, "{order:?}");
        // The Update's result is what the run had completed when it ran: the order is observable.
        let saw = match order {
            Order::UpdateFirst => 0,
            Order::UpdateSecond => 7,
        };
        assert_eq!(answer.result, Some(json!(saw)), "{order:?}");
        assert_eq!(answer.resolution.receipt().after_state_revision(), 1);

        let events = journal(&fixture.w, run).await;
        let update_at = position(&events, "state.updated");
        let completed_at = position(&events, "step.completed");
        match order {
            Order::UpdateFirst => assert!(update_at < completed_at, "{events:?}"),
            Order::UpdateSecond => assert!(completed_at < update_at, "{events:?}"),
        }
        assert_eq!(
            events
                .iter()
                .filter(|(_, event)| event == "step.completed")
                .count(),
            1,
            "{order:?}: the completion is journaled once"
        );
        assert_eq!(
            fixture.w.step_status(run, "work").await,
            Some(StepStatus::Succeeded)
        );
        assert!(
            fixture.w.step_status(run, "hold").await.is_some(),
            "{order:?}: the successor step was scheduled"
        );
        assert_update_committed_once(&fixture, 1).await;
    }
}

/// A timer or a wait timeout that is due at the same moment as an Update: the scheduler fires it
/// once, the Update commits once, and the journal order is the lock order.
async fn timer_race(document: serde_json::Value, due_node: &str, wait: bool) {
    for order in ORDERS {
        let fixture = start(document.clone(), &input(10)).await;
        let run = fixture.run;
        fixture.w.age_due(run, due_node).await;
        let update_args = replace(55);
        let update =
            fixture
                .ops
                .update(run, "balance.replace", "timer-race", &update_args, Some(0));
        let tick = fixture.w.scheduler.tick();

        let (answer, report) = match order {
            Order::UpdateFirst => race(&fixture.w, run, update, tick).await,
            Order::UpdateSecond => {
                let (report, answer) = race(&fixture.w, run, tick, update).await;
                (answer, report)
            }
        };
        let answer = answer.expect("the update commits");
        assert_eq!(answer.status(), UpdateReceiptStatus::Applied, "{order:?}");
        assert_eq!(answer.result, Some(json!(55)), "{order:?}");
        let (fired, kind) = if wait {
            (report.waits_timed_out, "wait.timed_out")
        } else {
            (report.timers_fired, "timer.fired")
        };
        assert_eq!(fired, 1, "{order:?}: {report:?}");
        assert_eq!(
            report.failed + report.fenced + report.skipped,
            0,
            "{report:?}"
        );

        let events = journal(&fixture.w, run).await;
        let update_at = position(&events, "state.updated");
        let fired_at = position(&events, kind);
        match order {
            Order::UpdateFirst => assert!(update_at < fired_at, "{events:?}"),
            Order::UpdateSecond => assert!(fired_at < update_at, "{events:?}"),
        }
        assert_eq!(
            events.iter().filter(|(_, event)| event == kind).count(),
            1,
            "{order:?}: the firing is journaled once"
        );
        assert!(
            fixture.w.step_status(run, "after").await.is_some(),
            "{order:?}: the timer's successor was scheduled"
        );
        // Nothing is left due: a second pass finds no timer to fire again.
        assert!(!fixture.w.scheduler.tick().await.did_work(), "{order:?}");
        assert_update_committed_once(&fixture, 1).await;
    }
}

#[tokio::test]
async fn an_update_and_a_firing_timer_both_commit_in_either_order() {
    timer_race(timer_document("operations-timer-race"), "pause", false).await;
}

#[tokio::test]
async fn an_update_and_a_wait_timeout_both_commit_in_either_order() {
    timer_race(wait_document("operations-wait-race"), "gate", true).await;
}

/// The same request delivered twice at once (a client that retries before the first answer): the
/// second waits for the first and replays its receipt, so the state moves once.
#[tokio::test]
async fn two_deliveries_of_one_update_key_apply_once_and_replay_once() {
    let fixture = start(worker_document("operations-duplicate-delivery"), &input(10)).await;
    let run = fixture.run;
    let args = replace(41);
    let (first, second) = race(
        &fixture.w,
        run,
        fixture
            .ops
            .update(run, "balance.replace", "duplicate", &args, Some(0)),
        fixture
            .ops
            .update(run, "balance.replace", "duplicate", &args, Some(0)),
    )
    .await;
    let first = first.expect("first delivery");
    let second = second.expect("second delivery");
    assert!(!first.resolution.replayed());
    assert!(second.resolution.replayed());
    assert_eq!(first.resolution.receipt(), second.resolution.receipt());
    assert_eq!(
        (first.result, second.result),
        (Some(json!(41)), Some(json!(41)))
    );
    assert_update_committed_once(&fixture, 1).await;
}

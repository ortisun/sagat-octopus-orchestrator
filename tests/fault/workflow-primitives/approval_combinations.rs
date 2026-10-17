//! Races between an approval decision and the other things that can happen to its run: a declared
//! Update, a second decider, a cancel, the chain's cancel latch, the chain's original deadline, the
//! approval's own expiry and the continuation hand-off (decision D295). The decision and every
//! contender go through the public HTTP routes and the real scheduler; the contenders are released
//! together by a barrier and every round checks the same invariants, whichever commits first:
//! one outcome for the request, one sequence for the journal, no second successor.

use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use sagat_domain::execution::RunId;
use serde_json::{Value, json};
use tokio::sync::Barrier;
use uuid::Uuid;

use crate::support::approval_flow::{
    ApprovalFlow, OPERATOR, OTHER_OPERATOR, approval_chain, approval_chain_with_state,
    approval_with_state, reason_of,
};

const ROUNDS: usize = 6;

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::now_v7().simple().to_string()[20..])
}

async fn count(flow: &ApprovalFlow, run: RunId, kind: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type=$2")
        .bind(run.as_uuid())
        .bind(kind)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("count journal entries")
}

async fn request_status(flow: &ApprovalFlow, approval: &str) -> String {
    sqlx::query_scalar("SELECT status FROM approval_requests WHERE request_id=$1::uuid")
        .bind(approval)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("request status")
}

async fn receipts(flow: &ApprovalFlow, approval: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM approval_decision_receipts WHERE request_id=$1::uuid")
        .bind(approval)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("receipts")
}

/// The journal is one gap-free sequence and rebuilds the projection.
async fn assert_one_journal(flow: &ApprovalFlow, run: RunId) {
    let sequences: Vec<i64> =
        sqlx::query_scalar("SELECT seq FROM run_events WHERE run_id=$1 ORDER BY seq")
            .bind(run.as_uuid())
            .fetch_all(&flow.api.db.pool)
            .await
            .expect("sequences");
    let expected: Vec<i64> = (1..=i64::try_from(sequences.len()).expect("length")).collect();
    assert_eq!(
        sequences, expected,
        "a gap or a duplicate in the journal of {run}"
    );
    flow.verify(run).await;
}

/// Releases `a` and `b` together.
async fn together<A, B>(a: A, b: B) -> (A::Output, B::Output)
where
    A: std::future::Future + Send + 'static,
    B: std::future::Future + Send + 'static,
    A::Output: Send + 'static,
    B::Output: Send + 'static,
{
    let gate = Arc::new(Barrier::new(3));
    let (first_gate, second_gate) = (gate.clone(), gate.clone());
    let first = tokio::spawn(async move {
        first_gate.wait().await;
        a.await
    });
    let second = tokio::spawn(async move {
        second_gate.wait().await;
        b.await
    });
    gate.wait().await;
    let (first, second) = tokio::join!(first, second);
    (
        first.expect("first contender"),
        second.expect("second contender"),
    )
}

#[tokio::test]
async fn a_decision_and_a_declared_update_racing_both_commit_in_one_journal() {
    let flow = Arc::new(ApprovalFlow::boot(&[]).await);
    let name = unique("race-state");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("publishes");
    for round in 0..ROUNDS {
        let run = flow
            .start(&workflow, json!({"account": {"balance": 1}}))
            .await;
        let approval = flow.the_approval(run).await;
        let (decide_flow, update_flow) = (flow.clone(), flow.clone());
        let decided_approval = approval.clone();
        let ((decided, _), (updated, applied)) = together(
            async move {
                decide_flow
                    .decide(
                        &decided_approval,
                        OPERATOR,
                        &format!("race-{round}"),
                        "APPROVE",
                    )
                    .await
            },
            async move {
                update_flow
                    .update_expecting(run, &format!("race-update-{round}"), 99, Some(0))
                    .await
            },
        )
        .await;
        // The decision leaves the run mutable until the scheduler ends it, so neither contender can
        // lose to the other: both commit, in whichever order the row lock gave.
        assert_eq!(decided, StatusCode::OK, "round {round}");
        assert_eq!(updated, StatusCode::OK, "round {round}: {applied}");
        flow.drive().await;
        assert_eq!(flow.run_status(run).await, "SUCCEEDED", "round {round}");
        assert_eq!(
            count(&flow, run, "approval.decided").await,
            1,
            "round {round}"
        );
        assert_eq!(count(&flow, run, "state.updated").await, 1, "round {round}");
        assert_eq!(flow.query(run, 0).await.1["result"], 99, "round {round}");
        assert_one_journal(&flow, run).await;
    }
}

#[tokio::test]
async fn two_deciders_at_once_leave_one_outcome_and_one_receipt() {
    let flow = Arc::new(ApprovalFlow::boot(&[]).await);
    let name = unique("race-deciders");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("publishes");
    for round in 0..ROUNDS {
        let run = flow
            .start(&workflow, json!({"account": {"balance": 1}}))
            .await;
        let approval = flow.the_approval(run).await;
        let (first_flow, second_flow) = (flow.clone(), flow.clone());
        let (first_approval, second_approval) = (approval.clone(), approval.clone());
        let ((first, first_body), (second, second_body)) = together(
            async move {
                first_flow
                    .decide(&first_approval, OPERATOR, &format!("a-{round}"), "APPROVE")
                    .await
            },
            async move {
                second_flow
                    .decide(
                        &second_approval,
                        OTHER_OPERATOR,
                        &format!("b-{round}"),
                        "REJECT",
                    )
                    .await
            },
        )
        .await;
        let wins = [first, second]
            .iter()
            .filter(|s| **s == StatusCode::OK)
            .count();
        assert_eq!(
            wins, 1,
            "round {round}: {first} {first_body} / {second} {second_body}"
        );
        let loser = if first == StatusCode::OK {
            &second_body
        } else {
            &first_body
        };
        assert_eq!(
            reason_of(loser),
            "ALREADY_DECIDED",
            "round {round}: {loser}"
        );
        assert_eq!(receipts(&flow, &approval).await, 1, "round {round}");
        flow.drive().await;
        let approved = flow.step_status(run, "approved").await.is_some();
        let rejected = flow.step_status(run, "rejected").await.is_some();
        assert!(approved ^ rejected, "round {round}: exactly one route ran");
        assert_eq!(
            request_status(&flow, &approval).await,
            if approved { "APPROVED" } else { "REJECTED" }
        );
        assert_eq!(count(&flow, run, "approval.decided").await, 1);
        assert_one_journal(&flow, run).await;
    }
}

#[tokio::test]
async fn the_same_decision_sent_twice_at_once_commits_once_and_replays_the_receipt() {
    let flow = Arc::new(ApprovalFlow::boot(&[]).await);
    let name = unique("race-duplicate");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("publishes");
    for round in 0..ROUNDS {
        let run = flow
            .start(&workflow, json!({"account": {"balance": 1}}))
            .await;
        let approval = flow.the_approval(run).await;
        let key = format!("same-key-{round}");
        let (first_flow, second_flow) = (flow.clone(), flow.clone());
        let (first_approval, second_approval) = (approval.clone(), approval.clone());
        let (first_key, second_key) = (key.clone(), key.clone());
        let ((first, first_body), (second, second_body)) = together(
            async move {
                first_flow
                    .decide(&first_approval, OPERATOR, &first_key, "APPROVE")
                    .await
            },
            async move {
                second_flow
                    .decide(&second_approval, OPERATOR, &second_key, "APPROVE")
                    .await
            },
        )
        .await;
        assert_eq!(
            (first, second),
            (StatusCode::OK, StatusCode::OK),
            "round {round}"
        );
        assert_eq!(
            first_body["receiptId"], second_body["receiptId"],
            "round {round}"
        );
        let replays = [&first_body, &second_body]
            .iter()
            .filter(|body| body["replayed"] == true)
            .count();
        assert_eq!(replays, 1, "round {round}: one commits, the other replays");
        assert_eq!(receipts(&flow, &approval).await, 1, "round {round}");
        flow.drive().await;
        assert_eq!(
            count(&flow, run, "approval.decided").await,
            1,
            "round {round}"
        );
        assert_one_journal(&flow, run).await;
    }
}

#[tokio::test]
async fn a_decision_and_a_run_cancel_racing_leave_one_consistent_outcome() {
    let flow = Arc::new(ApprovalFlow::boot(&[]).await);
    let name = unique("race-cancel");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("publishes");
    for round in 0..ROUNDS {
        let run = flow
            .start(&workflow, json!({"account": {"balance": 1}}))
            .await;
        let approval = flow.the_approval(run).await;
        let (decide_flow, cancel_flow) = (flow.clone(), flow.clone());
        let decided_approval = approval.clone();
        let ((decided, decided_body), (cancelled, _)) = together(
            async move {
                decide_flow
                    .decide(
                        &decided_approval,
                        OPERATOR,
                        &format!("cancel-race-{round}"),
                        "APPROVE",
                    )
                    .await
            },
            async move { cancel_flow.cancel(run).await },
        )
        .await;
        flow.drive().await;
        let status = request_status(&flow, &approval).await;
        match status.as_str() {
            // The decision committed first: the run was routed or cancelled after it.
            "APPROVED" => assert_eq!(decided, StatusCode::OK, "round {round}"),
            // The cancel committed first: the decision found a settled request.
            "CANCELLED" => {
                assert!(
                    matches!(cancelled, StatusCode::OK | StatusCode::ACCEPTED),
                    "round {round}: {cancelled}"
                );
                assert_eq!(
                    decided,
                    StatusCode::CONFLICT,
                    "round {round}: {decided_body}"
                );
                assert_eq!(reason_of(&decided_body), "ALREADY_DECIDED", "round {round}");
                assert_eq!(flow.run_status(run).await, "CANCELLED", "round {round}");
            }
            other => panic!("round {round}: unexpected request status {other}"),
        }
        let outcomes = count(&flow, run, "approval.decided").await;
        assert_eq!(outcomes, i64::from(status == "APPROVED"), "round {round}");
        assert_one_journal(&flow, run).await;
    }
}

#[tokio::test]
async fn a_decision_and_the_chain_cancel_latch_racing_never_leave_a_successor() {
    let flow = Arc::new(ApprovalFlow::boot(&["segment.work"]).await);
    let name = unique("race-latch");
    let workflow = flow
        .publish(&name, approval_chain(&name, 600_000, 3))
        .await
        .expect("publishes");
    for round in 0..ROUNDS {
        let chain = flow
            .start(&workflow, json!({"segment": 1, "next": {"segment": 2}}))
            .await;
        flow.drive().await;
        flow.work_on(chain, json!({"done": round})).await;
        let approval = flow.the_approval(chain).await;
        let (decide_flow, cancel_flow) = (flow.clone(), flow.clone());
        let decided_approval = approval.clone();
        let ((decided, decided_body), (cancelled, cancelled_body)) = together(
            async move {
                decide_flow
                    .decide(
                        &decided_approval,
                        OPERATOR,
                        &format!("latch-{round}"),
                        "APPROVE",
                    )
                    .await
            },
            async move { cancel_flow.cancel_chain(chain, chain).await },
        )
        .await;
        assert_eq!(
            cancelled,
            StatusCode::ACCEPTED,
            "round {round}: {cancelled_body}"
        );
        for _ in 0..3 {
            flow.drive().await;
        }
        let inspected = flow.chain(chain).await;
        assert_eq!(
            inspected["finalOutcome"], "CANCELLED",
            "round {round}: {inspected}"
        );
        assert_eq!(
            inspected["segments"].as_array().map(Vec::len),
            Some(1),
            "round {round}: a latched chain never hands off: {inspected}"
        );
        let status = request_status(&flow, &approval).await;
        match status.as_str() {
            "APPROVED" => assert_eq!(decided, StatusCode::OK, "round {round}"),
            "CANCELLED" => {
                assert_eq!(
                    decided,
                    StatusCode::CONFLICT,
                    "round {round}: {decided_body}"
                );
            }
            other => panic!("round {round}: unexpected request status {other}"),
        }
        assert_one_journal(&flow, chain).await;
    }
}

#[tokio::test]
async fn a_decision_racing_the_scheduler_hand_off_creates_exactly_one_successor() {
    let flow = Arc::new(ApprovalFlow::boot(&["segment.work"]).await);
    let name = unique("race-handoff");
    let workflow = flow
        .publish(&name, approval_chain_with_state(&name, 600_000, 3))
        .await
        .expect("publishes");
    for round in 0..ROUNDS {
        let chain = flow
            .start(
                &workflow,
                json!({"account": {"balance": 5}, "next": {"account": {"balance": 0}}}),
            )
            .await;
        flow.drive().await;
        flow.work_on(chain, json!({"done": round})).await;
        let approval = flow.the_approval(chain).await;
        let (decide_flow, drive_flow) = (flow.clone(), flow.clone());
        let decided_approval = approval.clone();
        let ((decided, _), ()) = together(
            async move {
                decide_flow
                    .decide(
                        &decided_approval,
                        OPERATOR,
                        &format!("hand-off-{round}"),
                        "APPROVE",
                    )
                    .await
            },
            async move {
                for _ in 0..6 {
                    drive_flow.drive().await;
                    tokio::time::sleep(Duration::from_millis(15)).await;
                }
            },
        )
        .await;
        assert_eq!(decided, StatusCode::OK, "round {round}");
        for _ in 0..3 {
            flow.drive().await;
        }
        let inspected = flow.chain(chain).await;
        assert_eq!(
            inspected["segments"].as_array().map(Vec::len),
            Some(2),
            "round {round}: one successor, not none and not two: {inspected}"
        );
        assert_eq!(flow.run_status(chain).await, "SUCCEEDED", "round {round}");
        assert_eq!(count(&flow, chain, "approval.decided").await, 1);
        assert_one_journal(&flow, chain).await;
        let head: RunId = inspected["currentRunId"]
            .as_str()
            .expect("head")
            .parse()
            .expect("id");
        assert_one_journal(&flow, head).await;
    }
}

#[tokio::test]
async fn a_decision_racing_the_chains_original_deadline_has_one_outcome_and_no_open_request() {
    let flow = Arc::new(ApprovalFlow::boot(&["segment.work"]).await);
    let name = unique("race-deadline");
    let mut document = approval_chain(&name, 600_000, 3);
    document["chainPolicy"]["timeoutMs"] = json!(3_000);
    let workflow = flow.publish(&name, document).await.expect("publishes");
    // The decision lands before, at and after the chain's deadline; the chain identity is immutable,
    // so the deadline is the real one and only the offset of the decision moves.
    for (round, offset_ms) in [-400_i64, -150, -60, 0, 60, 200].into_iter().enumerate() {
        let chain = flow
            .start(&workflow, json!({"segment": 1, "next": {"segment": 2}}))
            .await;
        flow.drive().await;
        flow.work_on(chain, json!({"done": round})).await;
        let approval = flow.the_approval(chain).await;
        let deadline: time::OffsetDateTime = sqlx::query_scalar(
            "SELECT original_deadline_at FROM execution_chains WHERE chain_id = $1",
        )
        .bind(chain.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("original deadline");
        let target = deadline + time::Duration::milliseconds(offset_ms);
        let wait = (target - time::OffsetDateTime::now_utc()).max(time::Duration::ZERO);
        tokio::time::sleep(Duration::from_millis(
            u64::try_from(wait.whole_milliseconds()).expect("wait"),
        ))
        .await;
        let (decide_flow, drive_flow) = (flow.clone(), flow.clone());
        let decided_approval = approval.clone();
        let ((decided, _), ()) = together(
            async move {
                decide_flow
                    .decide(
                        &decided_approval,
                        OPERATOR,
                        &format!("deadline-{round}"),
                        "APPROVE",
                    )
                    .await
            },
            async move {
                for _ in 0..8 {
                    drive_flow.drive().await;
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            },
        )
        .await;
        // Let the deadline pass for whichever segment is the head, then settle.
        tokio::time::sleep(Duration::from_millis(
            u64::try_from(
                ((deadline - time::OffsetDateTime::now_utc()).max(time::Duration::ZERO)
                    + time::Duration::milliseconds(300))
                .whole_milliseconds(),
            )
            .expect("wait"),
        ))
        .await;
        for _ in 0..6 {
            flow.drive().await;
        }
        let status = request_status(&flow, &approval).await;
        assert!(
            matches!(status.as_str(), "APPROVED" | "CANCELLED"),
            "round {round} (offset {offset_ms} ms): an ended chain leaves no open request, got {status}"
        );
        assert_eq!(
            decided == StatusCode::OK,
            status == "APPROVED",
            "round {round}: the receipt and the request agree"
        );
        let inspected = flow.chain(chain).await;
        assert_eq!(
            inspected["finalOutcome"], "TIMED_OUT",
            "round {round}: {inspected}"
        );
        let segments = inspected["segments"].as_array().map_or(0, Vec::len);
        assert!(
            segments == 1 || (segments == 2 && status == "APPROVED"),
            "round {round}: a successor only follows a decision that won: {inspected}"
        );
        assert_one_journal(&flow, chain).await;
    }
}

#[tokio::test]
async fn a_decision_racing_the_approvals_own_expiry_routes_exactly_one_branch() {
    let flow = Arc::new(ApprovalFlow::boot(&[]).await);
    let name = unique("race-expiry");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 1_200))
        .await
        .expect("publishes");
    for round in 0..3 {
        let run = flow
            .start(&workflow, json!({"account": {"balance": 1}}))
            .await;
        let approval = flow.the_approval(run).await;
        // Decide in the window around the deadline while the scheduler runs.
        tokio::time::sleep(Duration::from_millis(900)).await;
        let (decide_flow, drive_flow) = (flow.clone(), flow.clone());
        let decided_approval = approval.clone();
        let ((decided, decided_body), ()) = together(
            async move {
                let mut last = (StatusCode::NOT_FOUND, Value::Null);
                for attempt in 0..12 {
                    last = decide_flow
                        .decide(
                            &decided_approval,
                            OPERATOR,
                            &format!("expiry-{round}"),
                            "APPROVE",
                        )
                        .await;
                    if attempt >= 3 || last.0 != StatusCode::OK {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(60)).await;
                }
                last
            },
            async move {
                for _ in 0..30 {
                    drive_flow.drive().await;
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            },
        )
        .await;
        flow.drive().await;
        let status = request_status(&flow, &approval).await;
        let approved = flow.step_status(run, "approved").await.is_some();
        let expired = flow.step_status(run, "expired").await.is_some();
        assert!(approved ^ expired, "round {round}: exactly one route ran");
        match status.as_str() {
            "APPROVED" => assert!(approved && decided == StatusCode::OK, "round {round}"),
            "EXPIRED" => {
                assert!(expired, "round {round}");
                assert!(
                    decided == StatusCode::CONFLICT
                        && reason_of(&decided_body) == "ALREADY_DECIDED"
                        || decided == StatusCode::OK,
                    "round {round}: {decided} {decided_body}"
                );
            }
            other => panic!("round {round}: unexpected status {other}"),
        }
        assert_eq!(flow.run_status(run).await, "SUCCEEDED", "round {round}");
        assert_one_journal(&flow, run).await;
    }
}

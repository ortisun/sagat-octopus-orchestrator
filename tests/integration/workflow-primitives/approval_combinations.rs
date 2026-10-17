//! Approval combined with declared run state, with continuation chains, and refused inside
//! collection and loop bodies (decision D295), over the real HTTP, scheduler and PostgreSQL
//! boundaries. Every request is opened and expired by the scheduler and settled through the public
//! decision route; nothing is seeded.
//!
//! The order this file protects: the decision, the Update, the hand-off and the cancel latch each
//! commit under the run's row lock, so the journal stays one sequence and a run keeps exactly one
//! outcome of its approval.

use reqwest::StatusCode;
use sagat_domain::execution::RunId;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::approval_flow::{
    ApprovalFlow, INITIATOR, OPERATOR, VIEWER, approval_chain, approval_chain_with_state,
    approval_with_state, reason_of,
};

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::now_v7().simple().to_string()[20..])
}

#[tokio::test]
async fn an_approval_with_declared_state_publishes_and_the_decision_routes_the_run() {
    let flow = ApprovalFlow::boot(&[]).await;
    let name = unique("appr-state");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("an approval and a declared state publish together (D295)");
    let run = flow
        .start(&workflow, json!({"account": {"balance": 100}}))
        .await;

    let approval = flow.the_approval(run).await;
    assert_eq!(flow.run_status(run).await, "WAITING");

    // The state is readable and mutable while the request is open.
    let (status, read) = flow.query(run, 5).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["result"], 105, "{read}");
    assert_eq!(read["stateRevision"], 0, "{read}");
    let (status, applied) = flow.update(run, "raise-to-250", 250).await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert_eq!(applied["afterStateRevision"], 1, "{applied}");
    assert_eq!(
        flow.run_status(run).await,
        "WAITING",
        "an Update does not settle the approval"
    );

    let (status, decided) = flow
        .decide(&approval, OPERATOR, "approve-1", "APPROVE")
        .await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    flow.drive().await;
    assert_eq!(flow.run_status(run).await, "SUCCEEDED");
    assert_eq!(
        flow.step_status(run, "approved").await.as_deref(),
        Some("SUCCEEDED")
    );
    assert_eq!(flow.step_status(run, "rejected").await, None);

    // The state outlives the approval: it is readable on the ended run and no longer mutable.
    let (status, read) = flow.query(run, 0).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["result"], 250, "{read}");
    assert_eq!(read["stateRevision"], 1, "{read}");
    let (status, refused) = flow.update(run, "too-late", 1).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(reason_of(&refused), "RUN_NOT_MUTABLE", "{refused}");
    flow.verify(run).await;
}

#[tokio::test]
async fn an_update_after_the_decision_but_before_the_route_runs_is_one_journal() {
    let flow = ApprovalFlow::boot(&[]).await;
    let name = unique("appr-state-order");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("publishes");
    let run = flow
        .start(&workflow, json!({"account": {"balance": 1}}))
        .await;
    let approval = flow.the_approval(run).await;

    // The decision commits and the scheduler has not released the route yet.
    let (status, _) = flow
        .decide(&approval, OPERATOR, "approve-2", "APPROVE")
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, applied) = flow.update(run, "between", 7).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the run is still mutable: {applied}"
    );
    flow.drive().await;
    assert_eq!(flow.run_status(run).await, "SUCCEEDED");
    let journal = flow.journal(run).await;
    let decided = journal.iter().position(|kind| kind == "approval.decided");
    let updated = journal.iter().position(|kind| kind.contains("update"));
    assert!(decided.is_some() && updated.is_some(), "{journal:?}");
    flow.verify(run).await;
}

#[tokio::test]
async fn a_stale_expected_revision_conflicts_while_the_approval_stays_open() {
    let flow = ApprovalFlow::boot(&[]).await;
    let name = unique("appr-state-rev");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("publishes");
    let run = flow
        .start(&workflow, json!({"account": {"balance": 1}}))
        .await;
    let approval = flow.the_approval(run).await;

    let (status, _) = flow.update_expecting(run, "first", 10, Some(0)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, stale) = flow.update_expecting(run, "second", 20, Some(0)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(reason_of(&stale), "REVISION_CONFLICT", "{stale}");
    // The refusal changed nothing about the request: it is still pending and still decidable.
    let items = flow.approvals(run).await;
    assert_eq!(items[0]["status"], "PENDING", "{items:?}");
    let (status, _) = flow.decide(&approval, OPERATOR, "reject-1", "REJECT").await;
    assert_eq!(status, StatusCode::OK);
    flow.drive().await;
    assert_eq!(
        flow.step_status(run, "rejected").await.as_deref(),
        Some("SUCCEEDED")
    );
    let (_, read) = flow.query(run, 0).await;
    assert_eq!(read["result"], 10, "{read}");
    flow.verify(run).await;
}

#[tokio::test]
async fn an_expired_approval_keeps_the_state_and_takes_the_expire_route() {
    let flow = ApprovalFlow::boot(&[]).await;
    let name = unique("appr-state-exp");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 1_500))
        .await
        .expect("publishes");
    let run = flow
        .start(&workflow, json!({"account": {"balance": 3}}))
        .await;
    let approval = flow.the_approval(run).await;
    let (status, _) = flow.update(run, "before-expiry", 30).await;
    assert_eq!(status, StatusCode::OK);

    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        flow.drive().await;
        if flow.run_status(run).await == "SUCCEEDED" {
            break;
        }
    }
    assert_eq!(flow.run_status(run).await, "SUCCEEDED");
    assert_eq!(
        flow.step_status(run, "expired").await.as_deref(),
        Some("SUCCEEDED")
    );
    let (status, late) = flow
        .decide(&approval, OPERATOR, "too-late", "APPROVE")
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{late}");
    assert_eq!(reason_of(&late), "ALREADY_DECIDED", "{late}");
    let (_, read) = flow.query(run, 0).await;
    assert_eq!(read["result"], 30, "{read}");
    flow.verify(run).await;
}

#[tokio::test]
async fn a_viewer_reads_the_state_but_cannot_decide_a_stateful_approval() {
    let flow = ApprovalFlow::boot(&[]).await;
    let name = unique("appr-state-viewer");
    let workflow = flow
        .publish(&name, approval_with_state(&name, 600_000))
        .await
        .expect("publishes");
    let run = flow
        .start(&workflow, json!({"account": {"balance": 1}}))
        .await;
    let approval = flow.the_approval(run).await;
    let (status, refused) = flow.decide(&approval, VIEWER, "viewer-1", "APPROVE").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(reason_of(&refused), "NOT_ELIGIBLE", "{refused}");
    let (status, refused) = flow
        .decide(&approval, INITIATOR, "alice-1", "APPROVE")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(reason_of(&refused), "INITIATOR_CANNOT_DECIDE", "{refused}");
    assert_eq!(flow.approvals(run).await[0]["status"], "PENDING");
}

fn body_workflow(name: &str, owner: Value, body_nodes: Vec<Value>) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "owner",
        "bodies": {"iteration": {"entrypoint": "gate", "nodes": body_nodes}},
        "nodes": [owner, {"id": "done", "type": "end"}]
    })
}

fn gated_body() -> Vec<Value> {
    vec![
        json!({
            "id": "gate",
            "type": "approval",
            "timeoutMs": 600_000,
            "policy": {"roles": ["operator"]},
            "onApprove": "work",
            "onReject": "work",
            "onExpire": "work"
        }),
        json!({"id": "work", "type": "activity", "activity": "gate.work", "next": "out"}),
        json!({"id": "out", "type": "return", "result": "steps.work.output"}),
    ]
}

#[tokio::test]
async fn an_approval_inside_a_collection_body_is_refused_naming_scoped_compensation() {
    let flow = ApprovalFlow::boot(&["gate.work"]).await;
    let name = unique("appr-in-map");
    let owner = json!({
        "id": "owner", "type": "forEach", "items": "input.items", "body": "iteration",
        "result": "bodyResult", "maxItems": 4, "maxConcurrency": 1, "next": "done"
    });
    let refused = flow
        .publish(&name, body_workflow(&name, owner, gated_body()))
        .await
        .expect_err("an approval in a collection body is not publishable (D295)");
    assert_eq!(reason_of(&refused), "UNSUPPORTED_NODE", "{refused}");
    assert_eq!(refused["details"]["kind"], "approval", "{refused}");
    assert_eq!(refused["details"]["node"], "gate", "{refused}");
    assert_eq!(
        refused["details"]["dependency"], "SCOPED_COMPENSATION",
        "{refused}"
    );
}

#[tokio::test]
async fn an_approval_inside_a_loop_body_is_refused_naming_scoped_compensation() {
    let flow = ApprovalFlow::boot(&["gate.work"]).await;
    let name = unique("appr-in-loop");
    let owner = json!({
        "id": "owner", "type": "while", "condition": "carry < 1", "body": "iteration",
        "initial": "0", "carry": "bodyResult", "maxIterations": 3, "timeoutMs": 60000,
        "next": "done"
    });
    let refused = flow
        .publish(&name, body_workflow(&name, owner, gated_body()))
        .await
        .expect_err("an approval in a loop body is not publishable (D295)");
    assert_eq!(reason_of(&refused), "UNSUPPORTED_NODE", "{refused}");
    assert_eq!(refused["details"]["kind"], "approval", "{refused}");
    assert_eq!(refused["details"]["node"], "gate", "{refused}");
    assert_eq!(
        refused["details"]["dependency"], "SCOPED_COMPENSATION",
        "{refused}"
    );
}

#[tokio::test]
async fn a_chain_segment_waits_for_its_approval_and_the_decision_rolls_it_over() {
    let flow = ApprovalFlow::boot(&["segment.work"]).await;
    let name = unique("appr-chain");
    let workflow = flow
        .publish(&name, approval_chain(&name, 600_000, 3))
        .await
        .expect("an approval and a continuation publish together (D295)");
    let chain = flow
        .start(
            &workflow,
            json!({"segment": 1, "next": {"segment": 2, "next": {"segment": 3}}}),
        )
        .await;

    // Segment 1 works, then waits for a person.
    flow.drive().await;
    flow.work(json!({"done": 1})).await;
    let first_approval = flow.the_approval(chain).await;
    assert_eq!(flow.run_status(chain).await, "WAITING");
    assert_eq!(
        flow.head(chain).await,
        chain,
        "the head does not move before the decision"
    );

    let (status, decided) = flow
        .decide(&first_approval, OPERATOR, "round-1", "APPROVE")
        .await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    flow.drive().await;
    let head = flow.head(chain).await;
    assert_ne!(
        head, chain,
        "the approved boundary handed the chain to a successor"
    );
    assert_eq!(flow.run_status(chain).await, "SUCCEEDED");

    // Segment 2 has an approval of its own, decided in the successor segment.
    flow.work(json!({"done": 2})).await;
    let second_approval = flow.the_approval(head).await;
    assert_ne!(first_approval, second_approval);
    let (status, refused) = flow
        .decide(&second_approval, OPERATOR, "round-2", "REJECT")
        .await;
    assert_eq!(status, StatusCode::OK, "{refused}");
    flow.drive().await;
    assert_eq!(flow.run_status(head).await, "SUCCEEDED");
    assert_eq!(flow.chain(chain).await["finalOutcome"], "SUCCEEDED");
    assert_eq!(
        flow.chain(chain).await["segments"].as_array().map(Vec::len),
        Some(2)
    );
    flow.verify(chain).await;
    flow.verify(head).await;
}

fn short_chain(name: &str, chain_ms: u64) -> Value {
    let mut document = approval_chain(name, 600_000, 3);
    document["chainPolicy"]["timeoutMs"] = json!(chain_ms);
    document
}

async fn request_rows(flow: &ApprovalFlow, run: RunId) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT request.status,timer.state FROM approval_requests request \
         JOIN approval_timers timer ON timer.tenant_id=request.tenant_id \
          AND timer.request_id=request.request_id \
         WHERE request.tenant_id=$1 AND request.run_id=$2 ORDER BY request.opened_sequence",
    )
    .bind(flow.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("approval rows")
}

#[tokio::test]
async fn the_chains_original_deadline_settles_an_open_approval() {
    let flow = ApprovalFlow::boot(&["segment.work"]).await;
    let name = unique("appr-chain-deadline");
    let workflow = flow
        .publish(&name, short_chain(&name, 3_000))
        .await
        .expect("publishes");
    let chain = flow
        .start(&workflow, json!({"segment": 1, "next": {"segment": 2}}))
        .await;
    flow.drive().await;
    flow.work(json!({"done": 1})).await;
    let approval = flow.the_approval(chain).await;
    assert_eq!(
        request_rows(&flow, chain).await,
        [("PENDING".into(), "PENDING".into())]
    );

    // The approval's own deadline is ten minutes away; the chain's is three seconds.
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        flow.drive().await;
        if flow.run_status(chain).await == "TIMED_OUT" {
            break;
        }
    }
    assert_eq!(flow.run_status(chain).await, "TIMED_OUT");
    assert_eq!(flow.chain(chain).await["finalOutcome"], "TIMED_OUT");
    let rows = request_rows(&flow, chain).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_ne!(
        rows[0].0, "PENDING",
        "an ended run leaves no open request: {rows:?}"
    );
    assert_ne!(
        rows[0].1, "PENDING",
        "nor a running approval timer: {rows:?}"
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
         AND action='approval.cancelled'",
    )
    .bind(flow.tenant.as_uuid())
    .bind(&approval)
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("approval audit");
    assert_eq!(
        audited, 1,
        "the platform's own stop is audited with the request it settled"
    );
    let (status, late) = flow
        .decide(&approval, OPERATOR, "after-the-deadline", "APPROVE")
        .await;
    assert!(
        status == StatusCode::CONFLICT && reason_of(&late) == "ALREADY_DECIDED",
        "a decision after the chain ended names the settled outcome: {status} {late}"
    );
    assert_eq!(
        flow.chain(chain).await["segments"].as_array().map(Vec::len),
        Some(1)
    );
    flow.verify(chain).await;
}

#[tokio::test]
async fn cancelling_the_chain_settles_the_open_approval_and_no_decision_can_revive_it() {
    let flow = ApprovalFlow::boot(&["segment.work"]).await;
    let name = unique("appr-chain-cancel");
    let workflow = flow
        .publish(&name, approval_chain(&name, 600_000, 3))
        .await
        .expect("publishes");
    let chain = flow
        .start(&workflow, json!({"segment": 1, "next": {"segment": 2}}))
        .await;
    flow.drive().await;
    flow.work(json!({"done": 1})).await;
    let approval = flow.the_approval(chain).await;

    let (status, body) = flow.cancel_chain(chain, chain).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;
    assert_eq!(flow.run_status(chain).await, "CANCELLED");
    assert_eq!(flow.chain(chain).await["finalOutcome"], "CANCELLED");
    let rows = request_rows(&flow, chain).await;
    assert_eq!(rows, [("CANCELLED".into(), "CANCELLED".into())]);

    let (status, late) = flow
        .decide(&approval, OPERATOR, "after-cancel", "APPROVE")
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{late}");
    assert_eq!(reason_of(&late), "ALREADY_DECIDED", "{late}");
    flow.drive().await;
    assert_eq!(
        flow.chain(chain).await["segments"].as_array().map(Vec::len),
        Some(1),
        "no successor"
    );
    flow.verify(chain).await;
}

#[tokio::test]
async fn a_chain_cancelled_between_the_decision_and_the_hand_off_never_hands_off() {
    let flow = ApprovalFlow::boot(&["segment.work"]).await;
    let name = unique("appr-chain-latch");
    let workflow = flow
        .publish(&name, approval_chain(&name, 600_000, 3))
        .await
        .expect("publishes");
    let chain = flow
        .start(&workflow, json!({"segment": 1, "next": {"segment": 2}}))
        .await;
    flow.drive().await;
    flow.work(json!({"done": 1})).await;
    let approval = flow.the_approval(chain).await;

    // The decision commits; the scheduler has not run the boundary yet.
    let (status, _) = flow
        .decide(&approval, OPERATOR, "approve-then-cancel", "APPROVE")
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = flow.cancel_chain(chain, chain).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;

    let inspected = flow.chain(chain).await;
    assert_eq!(inspected["finalOutcome"], "CANCELLED", "{inspected}");
    assert_eq!(
        inspected["segments"].as_array().map(Vec::len),
        Some(1),
        "{inspected}"
    );
    assert_eq!(flow.run_status(chain).await, "CANCELLED");
    flow.verify(chain).await;
}

#[tokio::test]
async fn the_state_an_update_wrote_during_the_approval_is_what_the_next_segment_starts_with() {
    let flow = ApprovalFlow::boot(&["segment.work"]).await;
    let name = unique("appr-chain-state");
    let workflow = flow
        .publish(&name, approval_chain_with_state(&name, 600_000, 3))
        .await
        .expect("an approval, a declared state and a continuation publish together (D295)");
    let chain = flow
        .start(
            &workflow,
            json!({"account": {"balance": 100}, "next": {"account": {"balance": 0}}}),
        )
        .await;
    flow.drive().await;
    flow.work(json!({"done": 1})).await;
    let approval = flow.the_approval(chain).await;

    // A person changes the state while the approval is open; the decision then rolls the chain over.
    let (status, applied) = flow
        .update_expecting(chain, "during-approval", 250, Some(0))
        .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    let (status, _) = flow.decide(&approval, OPERATOR, "round-1", "APPROVE").await;
    assert_eq!(status, StatusCode::OK);
    flow.drive().await;
    let head = flow.head(chain).await;
    assert_ne!(head, chain);

    // The successor starts from the transferred state (250), not from the input it was given (0).
    let (status, read) = flow.chain_query(chain, 0).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["result"], 250, "{read}");
    // The predecessor is a closed segment: it can no longer be updated.
    let (status, refused) = flow.update(chain, "old-segment", 1).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(reason_of(&refused), "RUN_CONTINUED", "{refused}");
    // The successor's approval opens normally and an Update still works while it is open.
    flow.work(json!({"done": 2})).await;
    let second = flow.the_approval(head).await;
    let (status, applied) = flow
        .chain_update(chain, "second-segment", 300, Some(head))
        .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    let (status, _) = flow.decide(&second, OPERATOR, "round-2", "REJECT").await;
    assert_eq!(status, StatusCode::OK);
    flow.drive().await;
    assert_eq!(flow.chain(chain).await["finalOutcome"], "SUCCEEDED");
    let (_, read) = flow.chain_query(chain, 0).await;
    assert_eq!(read["result"], 300, "{read}");
    flow.verify(chain).await;
    flow.verify(head).await;
}

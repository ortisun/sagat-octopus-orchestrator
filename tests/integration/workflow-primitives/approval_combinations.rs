//! Approval combined with declared run state, with continuation chains, and refused inside
//! collection and loop bodies (decision D295), over the real HTTP, scheduler and PostgreSQL
//! boundaries. Every request is opened and expired by the scheduler and settled through the public
//! decision route; nothing is seeded.
//!
//! The order this file protects: the decision, the Update, the hand-off and the cancel latch each
//! commit under the run's row lock, so the journal stays one sequence and a run keeps exactly one
//! outcome of its approval.

use reqwest::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::approval_flow::{
    ApprovalFlow, INITIATOR, OPERATOR, VIEWER, approval_with_state, reason_of,
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

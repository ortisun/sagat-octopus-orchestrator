//! Switch decisions at process and transaction failure boundaries.

use sagat_application::runs::RunStore;
use sagat_domain::execution::SwitchSelection;
use sagat_domain::workflow::NodeId;
use serde_json::{Value, json};

use crate::support::engine::{activity, document, world};

fn switch(name: &str) -> Value {
    document(
        name,
        "choose",
        vec![
            json!({
                "id": "choose",
                "type": "switch",
                "cases": [
                    {"id": "first", "when": "input.amount > 0", "target": "accepted"},
                    {"id": "second", "when": "input.amount >= 0", "target": "review"}
                ],
                "default": "rejected"
            }),
            activity("accepted"),
            activity("review"),
            activity("rejected"),
        ],
    )
}

async fn switch_counts(
    w: &crate::support::engine::World,
    run: sagat_domain::execution::RunId,
) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM run_events \
            WHERE tenant_id = $1 AND run_id = $2 AND event_type = 'switch.evaluated'), \
           (SELECT count(*) FROM steps \
            WHERE tenant_id = $1 AND run_id = $2 AND node = 'accepted'), \
           (SELECT count(*) FROM event_outbox \
            WHERE tenant_id = $1 AND aggregate_id = $2 AND event_type = 'task.ready')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("durable switch counts")
}

#[tokio::test]
async fn lost_start_response_and_duplicate_retry_commit_one_switch_route() {
    let w = world().await;
    let (workflow, _) = w.publish(switch("switch-lost-response")).await;

    // The first call committed, but its response is discarded as if the connection died after COMMIT.
    let committed = w
        .start_with(
            workflow,
            &json!({"amount": 5}),
            "switch-lost-response-once",
            None,
        )
        .await
        .expect("the request committed before its response was lost");
    let replayed = w
        .start_with(
            workflow,
            &json!({"amount": 5}),
            "switch-lost-response-once",
            None,
        )
        .await
        .expect("the duplicate request replays the committed response");

    assert_eq!(replayed, committed);
    assert_eq!(switch_counts(&w, committed).await, (1, 1, 1));
    w.assert_consistent(committed).await;
}

#[tokio::test]
async fn restart_recovers_the_recorded_choice_when_the_broker_hint_is_lost() {
    let w = world().await;
    let session = w
        .session(&["act.accepted", "act.review", "act.rejected"])
        .await;
    let (workflow, _) = w.publish(switch("switch-restart")).await;
    let run = w.start(workflow, &json!({"amount": 5})).await;

    let pending_hint: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE delivered_at IS NULL) \
         FROM event_outbox WHERE tenant_id = $1 AND aggregate_id = $2 \
           AND event_type = 'task.ready'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("pending task hint");
    assert_eq!(
        pending_hint,
        (1, 1),
        "the only broker hint remains undelivered"
    );

    // A fresh control-plane/worker-service replica has no in-memory decision or broker delivery.
    // It must reconstruct the selected route and lease the durable successor from PostgreSQL.
    let restarted = w.replica().await;
    let graph = restarted
        .store
        .graph(w.admin.tenant_id(), run)
        .await
        .expect("reload graph after restart")
        .expect("run still exists");
    let choice = graph
        .steps
        .iter()
        .find(|step| step.node.as_str() == "choose")
        .and_then(|step| step.switch_selection.clone());
    assert_eq!(
        choice,
        Some(SwitchSelection::Case(
            NodeId::parse("first").expect("case id")
        ))
    );

    let tasks = restarted
        .workers
        .poll(
            &session.principal,
            session.worker_id,
            &session.token,
            8,
            None,
        )
        .await
        .expect("fresh replica polls PostgreSQL");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].run_id, run);
    assert_eq!(tasks[0].node.as_str(), "accepted");
    assert_eq!(tasks[0].activity.as_str(), "act.accepted");
    assert_eq!(switch_counts(&w, run).await, (1, 1, 1));
    w.assert_consistent(run).await;
}

#[tokio::test]
async fn successor_outbox_failure_rolls_back_the_choice_and_retry_commits_once() {
    let w = world().await;
    let (workflow, _) = w.publish(switch("switch-fault-rollback")).await;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_switch_fault_hint() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected switch task hint failure'; END $$; \
         CREATE TRIGGER reject_switch_fault_hint BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type = 'task.ready') \
         EXECUTE FUNCTION reject_switch_fault_hint()",
    )
    .execute(&w.db.pool)
    .await
    .expect("install transaction fault");

    w.start_with(
        workflow,
        &json!({"amount": 5}),
        "switch-fault-rollback-once",
        None,
    )
    .await
    .expect_err("outbox failure aborts the whole switch transition");

    let partial: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM runs WHERE workflow_id = $1), \
           (SELECT count(*) FROM run_events e JOIN runs r ON r.id = e.run_id \
            WHERE r.workflow_id = $1 AND e.event_type = 'switch.evaluated'), \
           (SELECT count(*) FROM steps s JOIN runs r ON r.id = s.run_id \
            WHERE r.workflow_id = $1 AND s.node = 'accepted')",
    )
    .bind(workflow.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("inspect aborted transaction");
    assert_eq!(partial, (0, 0, 0), "no partial route survived the abort");

    sqlx::raw_sql(
        "DROP TRIGGER reject_switch_fault_hint ON event_outbox; \
         DROP FUNCTION reject_switch_fault_hint()",
    )
    .execute(&w.db.pool)
    .await
    .expect("remove transaction fault");

    let run = w
        .start_with(
            workflow,
            &json!({"amount": 5}),
            "switch-fault-rollback-once",
            None,
        )
        .await
        .expect("retry after rollback");
    assert_eq!(switch_counts(&w, run).await, (1, 1, 1));
    w.assert_consistent(run).await;
}

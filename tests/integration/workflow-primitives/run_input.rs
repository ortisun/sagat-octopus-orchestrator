//! Design 043 against a real PostgreSQL, rows 11 and 12 of the creators: a child run and a continue-as-new, whose parent
//! or predecessor may have been created before the check was on (T043-8, T043-9, FR-043-008, FR-043-009,
//! OD-2026-11-14-9).

use sagat_application::definitions::InputValidationMode;
use sagat_application::execution::child_run_id;
use sagat_domain::execution::{RunStatus, StepStatus};
use sagat_domain::workflow::NodeId;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{World, activity, child_node, document, world};
use crate::support::run_input::{order_schema, publish_in, unusable_schema, with_schema};

async fn step_code(w: &World, run: sagat_domain::execution::RunId, node: &str) -> Option<String> {
    sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = $2")
        .bind(run.as_uuid())
        .bind(node)
        .fetch_one(&w.db.pool)
        .await
        .expect("code")
}

/// Row 11: a child whose version refuses the input the parent's node evaluated is not created, and the parent step fails
/// `CHILD_INPUT_INVALID`; a schema outside the subset fails it `RUN_INPUT_SCHEMA_UNUSABLE`; a matching input starts it.
#[tokio::test]
async fn row_11_a_refused_child_is_not_created_and_its_parent_step_fails_with_the_code() {
    let w = world().await;
    let kid = format!("row-11-kid-{}", Uuid::now_v7());
    w.publish(with_schema(
        document(&kid, "a", vec![activity("a")]),
        order_schema(),
    ))
    .await;
    let unusable = format!("row-11-unusable-{}", Uuid::now_v7());
    publish_in(
        &w,
        InputValidationMode::Report,
        &with_schema(
            document(&unusable, "a", vec![activity("a")]),
            unusable_schema(),
        ),
    )
    .await;
    for (label, child, input, expected) in [
        (
            "refused",
            kid.as_str(),
            json!({"orderId": 42}),
            Some("CHILD_INPUT_INVALID"),
        ),
        (
            "unusable",
            unusable.as_str(),
            json!({"orderId": "o"}),
            Some("RUN_INPUT_SCHEMA_UNUSABLE"),
        ),
        ("matching", kid.as_str(), json!({"orderId": "o-1"}), None),
    ] {
        let (parent_workflow, _) = w
            .publish(document(
                &format!("row-11-parent-{label}-{}", Uuid::now_v7()),
                "c",
                vec![child_node("c", child, None)],
            ))
            .await;
        // the parent is a run like one created before the upgrade: it has no schema, its child is checked all the same
        let parent = w.start(parent_workflow, &input).await;
        w.scheduler.tick().await;
        let child_run = child_run_id(parent, &NodeId::parse("c").expect("node"));
        match expected {
            Some(code) => {
                assert!(w.run_record(child_run).await.is_none(), "{label}: no child");
                assert_eq!(
                    w.step_status(parent, "c").await,
                    Some(StepStatus::Failed),
                    "{label}"
                );
                assert_eq!(
                    step_code(&w, parent, "c").await.as_deref(),
                    Some(code),
                    "{label}"
                );
                assert_eq!(w.run_status(parent).await, RunStatus::Failed, "{label}");
            }
            None => assert!(
                w.run_record(child_run).await.is_some(),
                "{label}: the child exists"
            ),
        }
    }
}

fn rolling(name: &str, schema: Value) -> Value {
    with_schema(
        json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": name},
            "entrypoint": "work",
            "nodes": [
                {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
                {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
            ],
            "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
        }),
        schema,
    )
}

async fn stop_code(w: &World, run: sagat_domain::execution::RunId) -> Value {
    let stop: Value = sqlx::query_scalar(
        "SELECT detail FROM run_events WHERE run_id = $1 AND event_type = 'run.stop_requested'",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("the stop that ended the run");
    stop["code"].clone()
}

async fn head(w: &World, chain: sagat_domain::execution::RunId) -> Uuid {
    sqlx::query_scalar("SELECT current_run_id FROM execution_chains WHERE chain_id = $1")
        .bind(chain.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("head")
}

/// Row 12: a continuation whose evaluated input the run's version refuses creates no successor, and the predecessor ends
/// `FAILED` with `CONTINUATION_INPUT_INVALID`; a schema outside the subset ends it `RUN_INPUT_SCHEMA_UNUSABLE`.
#[tokio::test]
async fn row_12_a_refused_continuation_fails_its_run_and_creates_no_successor() {
    for (label, unusable) in [("refused", false), ("unusable", true)] {
        let w = world().await;
        let name = format!("row-12-{label}-{}", Uuid::now_v7());
        let schema = json!({
            "type": "object",
            "required": ["segment"],
            "properties": {"segment": {"type": "integer"}}
        });
        let (workflow, _) = if unusable {
            // stored as a version of 0.2.x would be: the chain started before the check, its continuation is checked
            publish_in(
                &w,
                InputValidationMode::Report,
                &rolling(&name, unusable_schema()),
            )
            .await
        } else {
            w.publish(rolling(&name, schema)).await
        };
        let chain = if unusable {
            // the first segment is started in report mode, as one started before the upgrade was
            let reported = sagat_application::runs::RunService::new(
                w.store.clone(),
                sagat_application::clock::SystemClock,
                sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
                w.cache.clone(),
            )
            .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
                InputValidationMode::Report,
            ));
            let input =
                serde_json::to_vec(&json!({"segment": 1, "next": {"segment": 2}})).expect("json");
            reported
                .start(
                    &w.admin,
                    &sagat_application::runs::StartRun {
                        namespace: "payments",
                        workflow,
                        version: None,
                        input: &input,
                        correlation_id: None,
                        deadline: None,
                        key: &format!("start-{}", Uuid::now_v7()),
                    },
                    &crate::support::engine::ctx(),
                )
                .await
                .expect("started in report mode")
                .info
                .run_id
        } else {
            w.start(workflow, &json!({"segment": 1, "next": {"segment": "two"}}))
                .await
        };
        let scheduler = crate::chains::scheduler(&w, &format!("row-12-{label}")).await;
        let session = w.session(&["segment.work"]).await;
        let task = w.one_task(&session).await;
        w.complete(&session, &task, &json!({}))
            .await
            .expect("complete");
        scheduler.tick().await;
        assert_eq!(w.run_status(chain).await, RunStatus::Failed, "{label}");
        let expected = if unusable {
            "RUN_INPUT_SCHEMA_UNUSABLE"
        } else {
            "CONTINUATION_INPUT_INVALID"
        };
        assert_eq!(stop_code(&w, chain).await, json!(expected), "{label}");
        assert_eq!(
            head(&w, chain).await,
            chain.as_uuid(),
            "{label}: no successor"
        );
    }
}

/// Both reviewers of phase 2: in `report` mode a continuation whose input is refused hands off, and the refusal is
/// audited once, in the hand-off transaction (D043-039).
#[tokio::test]
async fn report_mode_audits_a_refused_continuation_with_its_hand_off() {
    let w = world().await;
    let name = format!("row-12-report-{}", Uuid::now_v7());
    let (workflow, _) = w
        .publish(rolling(
            &name,
            json!({"type": "object", "required": ["segment"], "properties": {"segment": {"type": "integer"}}}),
        ))
        .await;
    let chain = w
        .start(workflow, &json!({"segment": 1, "next": {"segment": "two"}}))
        .await;
    let scheduler =
        crate::chains::scheduler_in(&w, "row-12-report", InputValidationMode::Report).await;
    let session = w.session(&["segment.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("complete");
    scheduler.tick().await;
    assert_ne!(
        head(&w, chain).await,
        chain.as_uuid(),
        "the successor was created"
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.input_refused' AND reason LIKE 'reported: RUN_INPUT_INVALID%'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audits");
    assert_eq!(audits, 1);
}

/// Row 11 with a parent created before the check was on (`report` mode, an input its own schema refuses): its child is
/// checked all the same (OD-2026-11-14-9).
#[tokio::test]
async fn row_11_the_child_of_a_parent_admitted_before_the_check_is_checked() {
    let w = world().await;
    let kid = format!("row-11-old-kid-{}", Uuid::now_v7());
    w.publish(with_schema(
        document(&kid, "a", vec![activity("a")]),
        order_schema(),
    ))
    .await;
    let (parent_workflow, _) = w
        .publish(with_schema(
            document(&format!("row-11-old-parent-{}", Uuid::now_v7()), "c", vec![child_node("c", &kid, None)]),
            json!({"type": "object", "required": ["orderId"], "properties": {"orderId": {"type": "string"}}}),
        ))
        .await;
    let old = sagat_application::runs::RunService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(sagat_infra_schema::run_input::RunInputValidator::gate(
        InputValidationMode::Report,
    ));
    let input = serde_json::to_vec(&json!({"orderId": 42})).expect("json");
    let parent = old
        .start(
            &w.admin,
            &sagat_application::runs::StartRun {
                namespace: "payments",
                workflow: parent_workflow,
                version: None,
                input: &input,
                correlation_id: None,
                deadline: None,
                key: &format!("start-{}", Uuid::now_v7()),
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("admitted before the check")
        .info
        .run_id;
    w.scheduler.tick().await;
    assert!(
        w.run_record(child_run_id(parent, &NodeId::parse("c").expect("node")))
            .await
            .is_none()
    );
    assert_eq!(
        step_code(&w, parent, "c").await.as_deref(),
        Some("CHILD_INPUT_INVALID")
    );
}

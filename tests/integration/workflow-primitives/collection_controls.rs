//! Scoped control nodes select branches from their own committed iteration data.
use crate::support::scope_flow::{Flow, activity, collection_document, iteration, node, retry};
use serde_json::{Value, json};

pub(super) fn document(name: &str, control: Value) -> Value {
    collection_document(
        name,
        vec![
            activity(
                "reserve",
                "line.reserve",
                "choose",
                json!({"compensateWith":"line.release", "compensationRetry":retry()}),
            ),
            control,
            activity("accept", "line.charge", "returned", json!({})),
            activity("reject", "line.notify", "returned", json!({})),
            json!({"id":"returned", "type":"return", "result":"item"}),
        ],
        2,
    )
}

pub(super) async fn at_control(name: &str, control: Value, items: Value) -> Flow {
    let count = items.as_array().expect("items").len();
    let flow = Flow::start(name, document(name, control), items).await;
    flow.drive().await;
    let initial = flow.poll().await;
    assert_eq!(initial.len(), count);
    for task in &initial {
        assert_eq!(node(task), "reserve");
        flow.complete(task, json!({"itemIndex":iteration(task)}))
            .await;
    }
    flow
}

async fn distinct_branches(name: &str, control: Value) {
    let flow = at_control(name, control, json!([1, -1])).await;
    flow.drive().await;
    let branches = flow.poll().await;
    assert_eq!(branches.len(), 2);
    for task in &branches {
        assert_eq!(
            node(task),
            if iteration(task) == 0 {
                "accept"
            } else {
                "reject"
            }
        );
        let input: Value = serde_json::from_str(&task.input_json).expect("local context");
        assert_eq!(input["index"], iteration(task));
        assert_eq!(
            input["steps"]["reserve"]["output"]["itemIndex"],
            iteration(task)
        );
        flow.complete(task, json!({"ok":true})).await;
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    assert!(flow.poll().await.is_empty());
    let choices: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
          AND event_type IN ('condition.evaluated','switch.evaluated') \
          AND instance_path_nodes=ARRAY['lines']::text[]",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("qualified recorded decisions");
    assert_eq!(choices, 2);
    let usage: (i32, i32, i32) = sqlx::query_as(
        "SELECT admitted_instances,reserved_instances,retired_reservations \
           FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("branch lifetime accounting");
    assert_eq!(
        usage,
        (10, 2, 2),
        "one unselected branch retires per member"
    );
    flow.verify().await;
}

#[tokio::test]
async fn a_control_transaction_trace_correlates_the_run_without_logging_its_item() {
    use sagat_domain::{execution::ExecutionInstanceId, workflow::NodeId};
    let marker = "sk-live-control-canary-1234567890abcdef";
    let flow = at_control("collection-control-trace", json!({
        "id":"choose", "type":"condition", "expression":"input.absent", "ifTrue":"accept", "ifFalse":"reject"
    }), json!([{"sensitive":marker}])).await;
    let captured = crate::support::logs::Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
    assert_eq!(
        flow.scheduler
            .advance_collection(flow.tenant, flow.run, &owner)
            .await
            .expect("record typed expression failure"),
        sagat_scheduler::CollectionAdvance::BodyProgress
    );
    let text = captured.text();
    assert!(
        text.contains("scheduler.action") && text.contains("collection_control"),
        "control transaction span"
    );
    assert!(
        text.contains(&flow.run.to_string()) && text.contains(&flow.tenant.to_string()),
        "qualified correlation"
    );
    assert!(
        !text.contains(marker) && !text.contains("sensitive") && !text.contains("input.absent"),
        "no item or expression content in control telemetry"
    );
    flow.verify().await;
}

#[tokio::test]
async fn local_control_success_and_failure_match_the_full_run_oracle() {
    use sagat_domain::{execution::ExecutionInstanceId, workflow::NodeId};
    use sagat_scheduler::CollectionAdvance;
    async fn run_case(
        whole: bool,
        switch: bool,
        fails: bool,
    ) -> (Vec<CollectionAdvance>, Vec<Value>, String) {
        let expression = if fails { "input.absent" } else { "item > 0" };
        let control = if switch {
            json!({"id":"choose", "type":"switch", "cases":[{"id":"positive", "when":expression, "target":"accept"}], "default":"reject"})
        } else {
            json!({"id":"choose", "type":"condition", "expression":expression, "ifTrue":"accept", "ifFalse":"reject"})
        };
        let name = format!("control-oracle-{whole}-{switch}-{fails}");
        let flow = Flow::start(&name, document(&name, control), json!([1, -1])).await;
        let owner = ExecutionInstanceId::root(NodeId::parse("lines").expect("owner"));
        let mut transitions = Vec::new();
        for _ in 0..80 {
            let progress = if whole {
                flow.scheduler
                    .advance_collection_from_run(flow.tenant, flow.run, &owner)
                    .await
            } else {
                flow.scheduler
                    .advance_collection(flow.tenant, flow.run, &owner)
                    .await
            }
            .expect("one fenced transition");
            transitions.push(progress);
            if matches!(
                progress,
                CollectionAdvance::Finished | CollectionAdvance::DrainFinished
            ) {
                break;
            }
            if progress == CollectionAdvance::Waiting {
                let tasks = flow.poll().await;
                assert!(
                    !tasks.is_empty(),
                    "no external work can advance {transitions:?}"
                );
                for task in tasks {
                    flow.complete(&task, json!({"node":node(&task), "index":iteration(&task)}))
                        .await;
                }
            }
        }
        let status = flow.run_status().await;
        assert_eq!(status, if fails { "COMPENSATED" } else { "SUCCEEDED" });
        flow.verify().await;
        let rows: Vec<Value> = sqlx::query_scalar(
            "SELECT jsonb_build_object('seq',seq,'kind',event_type,'node',node, \
                'nodes',instance_path_nodes,'indices',instance_path_indices,'detail',detail) \
             FROM run_events WHERE tenant_id=$1 AND run_id=$2 ORDER BY seq",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_all(&flow.api.db.pool)
        .await
        .expect("complete qualified journal");
        let uuid =
            regex::Regex::new("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
                .expect("UUID pattern");
        let journal = rows
            .into_iter()
            .map(|row| {
                serde_json::from_str(&uuid.replace_all(&row.to_string(), "<id>"))
                    .expect("UUID-normalized event")
            })
            .collect();
        (transitions, journal, status)
    }
    let _server = crate::support::hold_server();
    for switch in [false, true] {
        for fails in [false, true] {
            assert_eq!(
                run_case(false, switch, fails).await,
                run_case(true, switch, fails).await,
                "switch={switch}, fails={fails}: exact transitions, journal and terminal status"
            );
        }
    }
}

#[tokio::test]
async fn a_control_expression_failure_drains_and_compensates_its_real_prefix() {
    for (name, control) in [
        (
            "collection-condition-error",
            json!({
                "id":"choose", "type":"condition", "expression":"input.absent",
                "ifTrue":"accept", "ifFalse":"reject"
            }),
        ),
        (
            "collection-switch-error",
            json!({
                "id":"choose", "type":"switch", "cases":[
                    {"id":"invalid", "when":"input.absent", "target":"accept"}
                ], "default":"reject"
            }),
        ),
    ] {
        let flow = at_control(name, control, json!([1])).await;
        flow.drive().await;
        let undo = flow.poll().await;
        assert_eq!(
            undo.len(),
            1,
            "only the actual prefix effect is compensated"
        );
        assert_eq!(node(&undo[0]), "reserve~c");
        assert_eq!(undo[0].task_kind, "COMPENSATION");
        flow.complete(&undo[0], json!({"undone":true})).await;
        flow.drive().await;
        assert_eq!(flow.run_status().await, "COMPENSATED");
        let cause: String = sqlx::query_scalar(
            "SELECT drain_failure_code FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("typed failure");
        assert_eq!(cause, "EXPRESSION_ERROR");
        flow.verify().await;
    }
}

#[tokio::test]
async fn a_failed_condition_takes_its_declared_local_recovery_without_stopping_the_run() {
    let flow = at_control(
        "collection-condition-on-error",
        json!({
            "id":"choose", "type":"condition", "expression":"input.absent",
            "ifTrue":"accept", "ifFalse":"reject", "onError":"reject"
        }),
        json!([1]),
    )
    .await;
    flow.drive().await;
    let recovery = flow.poll().await;
    assert_eq!(recovery.len(), 1);
    assert_eq!(node(&recovery[0]), "reject");
    flow.complete(&recovery[0], json!({"recovered":true})).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;
}

#[tokio::test]
async fn a_selected_switch_case_does_not_evaluate_a_later_invalid_predicate() {
    let flow = at_control(
        "collection-switch-first-case",
        json!({
            "id":"choose", "type":"switch", "cases":[
                {"id":"first", "when":"true", "target":"accept"},
                {"id":"later", "when":"input.absent", "target":"reject"}
            ], "default":"reject"
        }),
        json!([1]),
    )
    .await;
    flow.drive().await;
    let selected = flow.poll().await;
    assert_eq!(selected.len(), 1);
    assert_eq!(node(&selected[0]), "accept");
    flow.complete(&selected[0], json!({"ok":true})).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    flow.verify().await;
}

#[tokio::test]
async fn each_member_evaluates_a_condition_in_its_own_committed_context() {
    distinct_branches("collection-local-condition", json!({
        "id":"choose", "type":"condition", "expression":"item > 0 && steps.reserve.output.itemIndex == index",
        "ifTrue":"accept", "ifFalse":"reject"
    })).await;
}

#[tokio::test]
async fn each_member_evaluates_ordered_switch_cases_or_its_default() {
    distinct_branches("collection-local-switch", json!({
        "id":"choose", "type":"switch", "cases":[
            {"id":"positive", "when":"item > 0 && steps.reserve.output.itemIndex == index", "target":"accept"}
        ], "default":"reject"
    })).await;
}

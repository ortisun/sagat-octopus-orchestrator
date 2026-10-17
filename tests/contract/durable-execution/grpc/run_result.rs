//! A run's result (G21, owner decision 2026-10-16): the `outputExpr` of the `end` that finishes a
//! run is evaluated in the transaction that reaches it, kept as that step's output and handed to
//! the parent of a child run as `steps.<child>.output`. Driven through the worker protocol, the
//! scheduler and real PostgreSQL.

use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::worker::{Platform, Worker, platform};

fn kid(output: Option<&str>) -> Value {
    let mut end = json!({"id": "done", "type": "end"});
    if let Some(output) = output {
        end["outputExpr"] = json!(output);
    }
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "kid-flow"}, "entrypoint": "kid",
        "nodes": [
            {"id": "kid", "type": "activity", "activity": "act.kid", "next": "more"},
            {"id": "more", "type": "activity", "activity": "act.more", "next": "done"},
            end
        ]
    })
}

fn mother(before: usize) -> Value {
    let mut nodes: Vec<Value> = (0..before)
        .map(|i| {
            let next = if i + 1 == before {
                "c".to_owned()
            } else {
                format!("fill{}", i + 1)
            };
            json!({"id": format!("fill{i}"), "type": "activity", "activity": "act.fill", "next": next})
        })
        .collect();
    nodes.push(json!({"id": "c", "type": "child", "workflowId": "kid-flow", "next": "after"}));
    nodes.push(json!({"id": "after", "type": "activity", "activity": "act.after"}));
    let entry = if before == 0 {
        "c".to_owned()
    } else {
        "fill0".to_owned()
    };
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "mother-flow"}, "entrypoint": entry, "nodes": nodes
    })
}

const ACTIVITIES: &[&str] = &["act.kid", "act.more", "act.after", "act.fill"];

/// Ticks the scheduler until the worker is handed a task, or fails.
async fn next_task(p: &Platform, w: &mut Worker<'_>) -> sagat_api_grpc::pb::TaskLease {
    for _ in 0..20 {
        if let Some(task) = w.poll(1).await.pop() {
            return task;
        }
        p.tick().await;
    }
    panic!("no task became ready");
}

/// Ticks until the run is in `status`.
async fn settle(p: &Platform, run: &str, status: &str) {
    for _ in 0..20 {
        if p.status(run).await == status {
            return;
        }
        p.tick().await;
    }
    assert_eq!(p.status(run).await, status, "run {run}");
}

async fn result_of(p: &Platform, run: &str) -> Option<Value> {
    let bytes: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT p.content FROM runs r JOIN payloads p \
           ON p.tenant_id = r.tenant_id AND p.digest = r.result_digest WHERE r.id = $1",
    )
    .bind(Uuid::parse_str(run).expect("run id"))
    .fetch_optional(&p.api.db.pool)
    .await
    .expect("result");
    bytes.map(|b| serde_json::from_slice(&b).expect("a result is JSON"))
}

async fn stop_code(p: &Platform, run: &str) -> Option<String> {
    sqlx::query_scalar("SELECT stop_code FROM runs WHERE id = $1")
        .bind(Uuid::parse_str(run).expect("run id"))
        .fetch_one(&p.api.db.pool)
        .await
        .expect("stop code")
}

/// Runs the child's two activities with the given outputs and returns the child's run id.
async fn run_child(p: &Platform, w: &mut Worker<'_>, kid: &Value, more: &Value) -> String {
    let first = next_task(p, w).await;
    assert_eq!(first.activity_name, "act.kid");
    w.complete(&first, kid).await.expect("kid");
    let second = next_task(p, w).await;
    assert_eq!(second.activity_name, "act.more");
    w.complete(&second, more).await.expect("more");
    first.run_id
}

#[tokio::test]
async fn a_child_result_becomes_the_output_of_the_parent_step() {
    let p = platform().await;
    p.publish(
        "kid-flow",
        &kid(Some("steps.kid.output.total + steps.more.output.extra")),
    )
    .await;
    let wf = p.publish("mother-flow", &mother(0)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;

    let child = run_child(&p, &mut w, &json!({"total": 40}), &json!({"extra": 2})).await;
    settle(&p, &child, "SUCCEEDED").await;
    assert_eq!(result_of(&p, &child).await, Some(json!(42)));

    let after = next_task(&p, &mut w).await;
    assert_eq!(after.activity_name, "act.after");
    assert_eq!(after.run_id, run);
    let input: Value = serde_json::from_str(&after.input_json).expect("task input");
    assert_eq!(input["steps"]["c"]["output"], json!(42));
    w.complete(&after, &json!({})).await.expect("after");
    settle(&p, &run, "SUCCEEDED").await;
    assert_eq!(
        result_of(&p, &run).await,
        None,
        "the mother's end declares no result"
    );
}

#[tokio::test]
async fn an_end_without_an_output_expression_leaves_the_parent_step_without_output() {
    let p = platform().await;
    p.publish("kid-flow", &kid(None)).await;
    let wf = p.publish("mother-flow", &mother(0)).await;
    p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;

    let child = run_child(&p, &mut w, &json!({"total": 1}), &json!({})).await;
    settle(&p, &child, "SUCCEEDED").await;
    assert_eq!(result_of(&p, &child).await, None);
    let after = next_task(&p, &mut w).await;
    let input: Value = serde_json::from_str(&after.input_json).expect("task input");
    assert!(input["steps"].get("c").is_none(), "{input}");
}

#[tokio::test]
async fn a_result_expression_that_fails_fails_the_child_and_the_parent_step() {
    let p = platform().await;
    p.publish("kid-flow", &kid(Some("steps.kid.output.total + 'text'")))
        .await;
    let wf = p.publish("mother-flow", &mother(0)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;

    let child = run_child(&p, &mut w, &json!({"total": 1}), &json!({})).await;
    settle(&p, &child, "FAILED").await;
    assert_eq!(
        stop_code(&p, &child).await.as_deref(),
        Some("EXPRESSION_ERROR")
    );
    assert_eq!(result_of(&p, &child).await, None);
    settle(&p, &run, "FAILED").await;
    assert_eq!(stop_code(&p, &run).await.as_deref(), Some("CHILD_FAILED"));
}

#[tokio::test]
async fn a_result_larger_than_one_payload_fails_the_child_with_run_data_limit() {
    let p = platform().await;
    p.publish(
        "kid-flow",
        &kid(Some("[steps.kid.output, steps.more.output]")),
    )
    .await;
    let wf = p.publish("mother-flow", &mother(0)).await;
    p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;

    let big = json!({"blob": "x".repeat(200 * 1024)});
    let child = run_child(&p, &mut w, &big, &big).await;
    settle(&p, &child, "FAILED").await;
    assert_eq!(
        stop_code(&p, &child).await.as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(result_of(&p, &child).await, None);
}

#[tokio::test]
async fn a_parent_that_cannot_keep_the_result_fails_its_step_with_run_data_limit() {
    let p = platform().await;
    p.publish("kid-flow", &kid(Some("steps.kid.output"))).await;
    // 16 outputs of 250 KiB leave the mother less room than the child's 200 KiB result needs
    // within the 4 MiB a run may retain.
    let wf = p.publish("mother-flow", &mother(16)).await;
    let run = p.start(&wf, &json!({})).await;
    let mut w = p.worker("wendy").await;
    w.register(ACTIVITIES).await;
    let filler = json!({"fill": "y".repeat(250 * 1024)});
    for _ in 0..16 {
        let task = next_task(&p, &mut w).await;
        assert_eq!(task.activity_name, "act.fill");
        w.complete(&task, &filler).await.expect("filler");
    }

    let result = json!({"blob": "z".repeat(200 * 1024)});
    let child = run_child(&p, &mut w, &result, &json!({})).await;
    settle(&p, &child, "SUCCEEDED").await;
    assert!(
        result_of(&p, &child).await.is_some(),
        "the child keeps its result"
    );
    settle(&p, &run, "FAILED").await;
    let code: Option<String> =
        sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = 'c'")
            .bind(Uuid::parse_str(&run).expect("run id"))
            .fetch_one(&p.api.db.pool)
            .await
            .expect("parent step");
    assert_eq!(code.as_deref(), Some("RUN_DATA_LIMIT"));
}

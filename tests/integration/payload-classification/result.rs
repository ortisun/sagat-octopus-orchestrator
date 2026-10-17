//! Real PostgreSQL result selection tests.
#![allow(clippy::expect_used)]

use sagat_application::payload_disclosure::{DisclosureSelector, PayloadDisclosureResult};
use serde_json::json;

use super::disclosure::{disclose, disclosed_string};
use super::support::engine::{activity, document, end_node, with, world};

#[tokio::test]
async fn result_uses_only_runs_result_digest_and_the_workflow_output_schema() {
    let world = world().await;
    let work = with(activity("work"), "next", json!("done"));
    let mut source = document(
        "classified-result",
        "work",
        vec![
            work,
            with(end_node("done"), "outputExpr", json!("steps.work.output")),
        ],
    );
    source["outputSchema"] = json!({
        "type":"object", "additionalProperties":false,
        "x-sagat-classification":"public",
        "properties":{"answer":{"type":"string"}}
    });
    let (workflow, _) = world.publish(source).await;
    let run = world.start(workflow, &json!({})).await;

    let before = disclose(&world, &world.admin, run, DisclosureSelector::Result)
        .await
        .expect("no result is a closed success");
    assert!(matches!(
        before.result,
        PayloadDisclosureResult::Unavailable { .. }
    ));

    let session = world.session(&["act.work"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"answer":"from-run-result"}))
        .await
        .expect("complete workflow");
    let after = disclose(&world, &world.admin, run, DisclosureSelector::Result)
        .await
        .expect("stored result");
    let PayloadDisclosureResult::Disclosed { projection, .. } = after.result else {
        panic!("result uses its schema")
    };
    assert_eq!(
        disclosed_string(&projection.tree, "answer"),
        Some("from-run-result")
    );
}

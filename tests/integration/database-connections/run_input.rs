//! Design 043, row 13: an ad hoc database execution checks its input against the `inputSchema` its document declares,
//! under the same permits as every start and never cached; a refused start replays a run its key holds; a private rerun
//! answers `/` and `other` (review of phase 2, D043-148). Runs after `ad_hoc::run`, whose connection it uses.

use std::sync::Arc;

use crate::support::engine::{World, ctx, document};
use sagat_application::{
    clock::SystemClock,
    definitions::InputValidationMode,
    error::AppError,
    execution::Engine,
    runs::{AdHocMode, RunOperation, RunService, StartDatabaseExecution},
};
use serde_json::json;

pub async fn run(w: &World) {
    let mut doc = document(
        "database-ad-hoc-checked",
        "query",
        vec![json!({
            "id":"query", "type":"database", "connectionRef":{"name":"ad-hoc-db"},
            "operation":"query", "statement":"SELECT 1::bigint"
        })],
    );
    doc["inputSchema"] = json!({
        "type": "object",
        "required": ["orderId"],
        "properties": {"orderId": {"type": "string"}}
    });
    let bytes = serde_json::to_vec(&doc).unwrap();
    let request = |input: &'static [u8], key: &'static str| StartDatabaseExecution {
        namespace: w.namespace.id(),
        mode: AdHocMode::Step,
        document: &bytes,
        input,
        timeout_ms: None,
        key,
    };

    // refused, with the field and the rule
    let refused = w
        .runs
        .start_database(
            &w.admin,
            &request(br#"{"orderId": 42}"#, "ad-hoc-riv-0001"),
            &ctx(),
        )
        .await
        .expect_err("refused");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "RUN_INPUT_INVALID",
            field: Some("/orderId".to_owned()),
            rule: Some("type"),
        }
    );
    w.runs
        .start_database(
            &w.admin,
            &request(br#"{"orderId": "o-1"}"#, "ad-hoc-riv-0002"),
            &ctx(),
        )
        .await
        .expect("a matching input is admitted");

    // a run admitted before the check (report mode) is replayed by a refused retry of its key
    let reported = RunService::new(
        w.store.clone(),
        SystemClock,
        Engine::new(sagat_infra_ir::JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(Arc::new(sagat_application::run_input::RunInputGate::new(
        sagat_infra_schema::run_input::RunInputValidator::process(),
        InputValidationMode::Report,
    )));
    let admitted = reported
        .start_database(
            &w.admin,
            &request(br#"{"orderId": 7}"#, "ad-hoc-riv-0003"),
            &ctx(),
        )
        .await
        .expect("admitted in report mode");
    let replayed = w
        .runs
        .start_database(
            &w.admin,
            &request(br#"{"orderId": 7}"#, "ad-hoc-riv-0003"),
            &ctx(),
        )
        .await
        .expect("replayed, not refused");
    assert_eq!(replayed.info.run_id, admitted.info.run_id);

    // its private rerun says nothing of the source's input
    w.operate(admitted.info.run_id, RunOperation::Cancel)
        .await
        .unwrap();
    let rerun = w
        .runs
        .rerun(
            &w.admin,
            admitted.info.run_id,
            "repeat it after the fix",
            "ad-hoc-riv-rerun-1",
            &ctx(),
        )
        .await
        .expect_err("refused");
    assert_eq!(
        rerun,
        AppError::InvalidRunInput {
            reason: "RUN_INPUT_INVALID",
            field: Some("/".to_owned()),
            rule: Some("other"),
        }
    );
}

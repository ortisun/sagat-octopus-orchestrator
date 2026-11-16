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

/// The metrics of the lifecycle this scenario runs in, recorded on its thread (set by the test before it starts).
pub static METRICS: std::sync::OnceLock<sagat_telemetry::Metrics> = std::sync::OnceLock::new();

/// `database_executions_total{origin="AD_HOC_STEP",outcome}` so far: what the run service counts per ad hoc start.
fn executions(outcome: &str) -> u64 {
    let Some(metrics) = METRICS.get() else {
        panic!("the lifecycle records its metrics");
    };
    metrics
        .render()
        .lines()
        .filter(|line| {
            line.starts_with("database_executions_total{")
                && line.contains("origin=\"AD_HOC_STEP\"")
                && line.contains(&format!("outcome=\"{outcome}\""))
        })
        .filter_map(|line| line.rsplit(' ').next()?.parse::<u64>().ok())
        .sum()
}

/// What one call added to the two outcomes of `database_executions_total`: `(admitted, refused)`.
struct Counted {
    admitted: u64,
    refused: u64,
}

impl Counted {
    fn now() -> Self {
        Self {
            admitted: executions("admitted"),
            refused: executions("refused"),
        }
    }

    fn since(before: &Self) -> (u64, u64) {
        let now = Self::now();
        (now.admitted - before.admitted, now.refused - before.refused)
    }
}

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

    // refused, with the field and the rule; counted once, `refused`
    let before = Counted::now();
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
    assert_eq!(Counted::since(&before), (0, 1), "a refusal is counted once");
    let before = Counted::now();
    w.runs
        .start_database(
            &w.admin,
            &request(br#"{"orderId": "o-1"}"#, "ad-hoc-riv-0002"),
            &ctx(),
        )
        .await
        .expect("a matching input is admitted");
    assert_eq!(
        Counted::since(&before),
        (1, 0),
        "an admission is counted once"
    );

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
    let before = Counted::now();
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
    assert_eq!(
        Counted::since(&before),
        (1, 0),
        "a refused retry answered with its run is counted once, as the store's replay is"
    );

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

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

    // Architecture review of 2026-11-16, Major 2: a document whose `inputSchema` is outside the subset is parsed in the
    // process's mode, as publication is. In `enforce` it is refused as a publication would be, naming the keyword; in
    // `report` it is parsed with a warning, the gate reports `RUN_INPUT_SCHEMA_UNUSABLE`, and the run is created.
    let mut outside = document(
        "database-ad-hoc-outside",
        "query",
        vec![json!({
            "id":"query", "type":"database", "connectionRef":{"name":"ad-hoc-db"},
            "operation":"query", "statement":"SELECT 1::bigint"
        })],
    );
    outside["inputSchema"] =
        json!({"type": "object", "patternProperties": {"^a": {"type": "string"}}});
    let outside = serde_json::to_vec(&outside).unwrap();
    let unusable = |key: &'static str| StartDatabaseExecution {
        namespace: w.namespace.id(),
        mode: AdHocMode::Step,
        document: &outside,
        input: b"{}",
        timeout_ms: None,
        key,
    };
    let refused = w
        .runs
        .start_database(&w.admin, &unusable("ad-hoc-riv-0004"), &ctx())
        .await
        .expect_err("refused in enforce");
    assert!(
        matches!(&refused, AppError::InvalidDefinition(error) if error.to_string().contains("patternProperties")),
        "refused as at publication, naming the keyword: {refused:?}"
    );
    let reported_run = reported
        .start_database(&w.admin, &unusable("ad-hoc-riv-0005"), &ctx())
        .await
        .expect("created in report mode");
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'run.input_refused' AND resource_id = $1 \
         AND reason = 'reported: RUN_INPUT_SCHEMA_UNUSABLE'",
    )
    .bind(reported_run.info.run_id.to_string())
    .fetch_one(&w.db.pool)
    .await
    .unwrap();
    assert_eq!(audits, 1, "the report is audited with the run");

    sentinel(w).await;
}

/// Row 13 of T043-17 (FR-043-014, SC-043-002; round 2 of 2026-11-16, implementation readiness 3 and 13): a sentinel value,
/// and a sentinel key under `additionalProperties`, in ad hoc inputs that fail at them, appear in none of the answers, the
/// logs and spans captured at TRACE, the audit rows and the metrics text.
async fn sentinel(w: &World) {
    const VALUE: &str = "sentinel-043-ad-hoc-value-3f9b";
    const KEY: &str = "sentinel-043-ad-hoc-key-c41e";
    let mut doc = document(
        "database-ad-hoc-sentinel",
        "query",
        vec![json!({
            "id":"query", "type":"database", "connectionRef":{"name":"ad-hoc-db"},
            "operation":"query", "statement":"SELECT 1::bigint"
        })],
    );
    doc["inputSchema"] = json!({
        "type": "object",
        "required": ["orderId"],
        "properties": {"orderId": {"type": "string", "maxLength": 8}},
        "additionalProperties": {"type": "integer"}
    });
    let bytes = serde_json::to_vec(&doc).unwrap();
    let (_guard, captured) = crate::support::logs::capture_with_spans(tracing::Level::TRACE);
    let mut answers = String::new();
    for (n, input) in [
        json!({"orderId": VALUE}),
        json!({"orderId": "o-1", KEY: VALUE}),
    ]
    .into_iter()
    .enumerate()
    {
        let input = serde_json::to_vec(&input).unwrap();
        let key = format!("ad-hoc-sentinel-{n}");
        let refused = w
            .runs
            .start_database(
                &w.admin,
                &StartDatabaseExecution {
                    namespace: w.namespace.id(),
                    mode: AdHocMode::Step,
                    document: &bytes,
                    input: &input,
                    timeout_ms: None,
                    key: &key,
                },
                &ctx(),
            )
            .await
            .expect_err("refused");
        assert!(
            matches!(refused, AppError::InvalidRunInput { .. }),
            "{refused:?}"
        );
        answers.push_str(&format!("{refused:?} {refused}\n"));
    }
    let audits: Vec<Option<String>> = sqlx::query_scalar("SELECT reason FROM audit_records")
        .fetch_all(&w.db.pool)
        .await
        .unwrap();
    let audits = audits.into_iter().flatten().collect::<Vec<_>>().join("\n");
    let Some(metrics) = METRICS.get() else {
        panic!("the lifecycle records its metrics");
    };
    let rendered = metrics.render();
    assert!(
        rendered.contains("run_input_validations_total"),
        "the refusals were counted: {rendered}"
    );
    let logs = captured.text();
    assert!(
        logs.contains("run_input.check") && logs.contains("refused"),
        "the capture saw the gate refuse both starts: {logs}"
    );
    for (label, text) in [
        ("the answers", answers),
        ("the logs and spans", logs),
        ("the audit rows", audits),
        ("the metrics", rendered),
    ] {
        assert!(
            !text.contains(VALUE),
            "{label}: the sentinel value leaked: {text}"
        );
        assert!(
            !text.contains(KEY),
            "{label}: the sentinel key leaked: {text}"
        );
    }
}

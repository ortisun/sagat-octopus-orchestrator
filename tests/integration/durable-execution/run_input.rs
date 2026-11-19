//! Design 043 against a real PostgreSQL, rows 1 and 2 of the creators (a start through the API), with the replay of a
//! refused start, the version check of the store and `report` mode in the control plane (T043-5, T043-21, T043-22,
//! FR-043-001, FR-043-003, FR-043-004, FR-043-011, FR-043-013, FR-043-017).

use std::sync::Arc;

use sagat_application::clock::SystemClock;
use sagat_application::definitions::InputValidationMode;
use sagat_application::error::{AppError, StoreError};
use sagat_application::execution::{Engine, RunInfo};
use sagat_application::quotas::RunLimits;
use sagat_application::run_input::test_support::Scripted;
use sagat_application::run_input::{CheckedRunInput, RunInputGate, SchemaRefusal};
use sagat_application::runs::{NewRun, RunService, RunStore, StartRun};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::RunId;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_schema::run_input::RunInputValidator;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{World, activity, ctx, document, world};
use crate::support::run_input::{
    SENTINEL, audits, order_schema, publish_in, runs_of, unusable_schema, with_schema,
};

fn flow(name: &str) -> Value {
    document(name, "a", vec![activity("a")])
}

async fn start(
    w: &World,
    workflow: sagat_domain::definition::WorkflowId,
    input: &Value,
    key: &str,
) -> Result<RunId, AppError> {
    w.start_with(workflow, input, key, None).await
}

fn key() -> String {
    format!("start-{}", Uuid::now_v7())
}

/// Row 1: an input the schema refuses is answered with the reason, the pointer and the rule, never the value, and no run
/// exists; a matching input starts the run.
#[tokio::test]
async fn row_1_a_refused_start_names_the_field_and_the_rule_and_creates_nothing() {
    let w = world().await;
    let (workflow, _) = w
        .publish(with_schema(flow("row-one"), order_schema()))
        .await;
    let refused = start(
        &w,
        workflow,
        &json!({"orderId": 42, "note": SENTINEL}),
        &key(),
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
    assert!(!format!("{refused:?}{refused}").contains("42"));
    assert_eq!(runs_of(&w, workflow).await, 0);
    let missing = start(&w, workflow, &json!({}), &key())
        .await
        .expect_err("refused");
    assert_eq!(
        missing,
        AppError::InvalidRunInput {
            reason: "RUN_INPUT_INVALID",
            field: Some("/orderId".to_owned()),
            rule: Some("required"),
        }
    );
    start(&w, workflow, &json!({"orderId": "o-1"}), &key())
        .await
        .expect("a matching input starts");
    assert_eq!(runs_of(&w, workflow).await, 1);
}

/// Row 1, a schema outside the subset stored before publication refused it: every start of its version is refused
/// `RUN_INPUT_SCHEMA_UNUSABLE` (FR-043-011), and a version without a schema takes any object (FR-043-002).
#[tokio::test]
async fn row_1_an_unusable_schema_refuses_every_start_and_no_schema_takes_any_object() {
    let w = world().await;
    let (workflow, _) = publish_in(
        &w,
        InputValidationMode::Report,
        &with_schema(flow("row-one-unusable"), unusable_schema()),
    )
    .await;
    for input in [json!({}), json!({"x-a": "b"})] {
        let refused = start(&w, workflow, &input, &key())
            .await
            .expect_err("refused");
        assert_eq!(
            refused,
            AppError::InvalidRunInput {
                reason: "RUN_INPUT_SCHEMA_UNUSABLE",
                field: None,
                rule: None,
            }
        );
    }
    assert_eq!(runs_of(&w, workflow).await, 0);
    let (free, _) = w.publish(flow("row-one-free")).await;
    start(&w, free, &json!({"anything": [1, 2, 3]}), &key())
        .await
        .expect("no schema, any object");
}

/// FR-043-004: a refused start whose key already holds a run of the same fingerprint replays that run (created before
/// the check was on, here by a gate in `report` mode); a retry after another version was promoted is another request
/// and gets the `409` of a reused key.
#[tokio::test]
async fn a_refused_start_replays_the_run_of_its_key_and_a_promotion_makes_it_another_request() {
    let w = world().await;
    let (workflow, _) = w.publish(with_schema(flow("replay"), order_schema())).await;
    let reported = RunService::new(
        w.store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(RunInputValidator::gate(InputValidationMode::Report));
    let key = key();
    let input = serde_json::to_vec(&json!({"orderId": 7})).expect("json");
    let request = StartRun {
        namespace: "payments",
        workflow,
        version: None,
        input: &input,
        correlation_id: None,
        deadline: None,
        key: &key,
    };
    let first = reported
        .start(&w.admin, &request, &ctx())
        .await
        .expect("admitted in report mode");
    let replayed = w
        .runs
        .start(&w.admin, &request, &ctx())
        .await
        .expect("replayed, not refused");
    assert_eq!(replayed.info.run_id, first.info.run_id);
    assert_eq!(runs_of(&w, workflow).await, 1);

    w.publish_new_version(
        workflow,
        with_schema(
            flow("replay"),
            json!({"type": "object", "required": ["orderId", "other"]}),
        ),
    )
    .await;
    let after = w
        .runs
        .start(&w.admin, &request, &ctx())
        .await
        .expect_err("another request");
    assert_eq!(after, AppError::IdempotencyKeyReused);
}

/// T043-22: in `report` mode the run is created, and the refusal is audited once, in the transaction that creates it,
/// across a retry and a replay of the same start.
#[tokio::test]
async fn report_mode_creates_the_run_and_audits_the_refusal_once_across_a_replay() {
    let w = world().await;
    let (workflow, _) = w.publish(with_schema(flow("report"), order_schema())).await;
    let reported = RunService::new(
        w.store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(RunInputValidator::gate(InputValidationMode::Report));
    let key = key();
    let input = serde_json::to_vec(&json!({"orderId": SENTINEL.len()})).expect("json");
    let request = StartRun {
        namespace: "payments",
        workflow,
        version: None,
        input: &input,
        correlation_id: None,
        deadline: None,
        key: &key,
    };
    let run = reported
        .start(&w.admin, &request, &ctx())
        .await
        .expect("created");
    reported
        .start(&w.admin, &request, &ctx())
        .await
        .expect("replayed");
    assert_eq!(runs_of(&w, workflow).await, 1);
    assert_eq!(
        audits(
            &w,
            "run.input_refused",
            "reported: RUN_INPUT_INVALID rule=type"
        )
        .await,
        1,
        "audited once with the run {}",
        run.info.run_id
    );
}

/// T043-21: an input past the work budget is final the first time (`workLimit` at the root), and a validator with no room
/// is transient (`503`), remembered nowhere: the same start passes once there is room.
#[tokio::test]
async fn the_budget_is_final_and_no_room_is_transient() {
    let w = world().await;
    let costly = json!({
        "type": "object",
        "properties": {"items": {"type": "array", "items": {"anyOf": [
            {"type": "string", "pattern": "^(a|b)*c$"},
            {"type": "string", "pattern": "^(a|b)*d$"}
        ]}}}
    });
    let (workflow, _) = w.publish(with_schema(flow("budget"), costly)).await;
    // every item matches its second branch, so nothing fails before the budget does
    let long = format!("{}d", "ab".repeat(1_000));
    let input = json!({"items": vec![long; 100]});
    let refused = start(&w, workflow, &input, &key())
        .await
        .expect_err("past the budget");
    assert_eq!(
        refused,
        AppError::InvalidRunInput {
            reason: "RUN_INPUT_INVALID",
            field: Some("/".to_owned()),
            rule: Some("workLimit"),
        }
    );

    let (simple, _) = w.publish(with_schema(flow("busy"), order_schema())).await;
    let busy = Arc::new(Scripted::answering([
        Err(SchemaRefusal::Busy),
        Err(SchemaRefusal::Backstop),
    ]));
    let runs = RunService::new(
        w.store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(Arc::new(RunInputGate::new(
        busy.clone(),
        InputValidationMode::Enforce,
    )));
    let input = serde_json::to_vec(&json!({"orderId": "o-1"})).expect("json");
    let key = key();
    let request = StartRun {
        namespace: "payments",
        workflow: simple,
        version: None,
        input: &input,
        correlation_id: None,
        deadline: None,
        key: &key,
    };
    for _ in 0..2 {
        let error = runs
            .start(&w.admin, &request, &ctx())
            .await
            .expect_err("no room");
        assert_eq!(error, AppError::Unavailable);
        assert!(error.is_retryable());
    }
    runs.start(&w.admin, &request, &ctx())
        .await
        .expect("room again: admitted");
    assert_eq!(busy.calls().len(), 3);
}

/// FR-043-017: the store refuses an input checked against another version than the run's, and writes nothing.
#[tokio::test]
async fn the_store_refuses_an_input_checked_against_another_version() {
    let w = world().await;
    let (workflow, version) = w.publish(flow("mismatch")).await;
    let template = w.start(workflow, &json!({})).await;
    let mut info: RunInfo = w.run_record(template).await.expect("run").info;
    info.run_id = RunId::generate();
    info.root = info.run_id;
    let run = info.run_id;
    let new = NewRun {
        business: None,
        schedule: None,
        retained: None,
        limits: RunLimits::of(&w.store, info.tenant_id, info.namespace_id)
            .await
            .expect("limits"),
        definition: w.cache.get(version).expect("parsed"),
        input: CheckedRunInput::unchecked(WorkflowVersionId::generate(), b"{}".to_vec()),
        input_blob: None,
        audit: AuditRecord::by(
            &w.admin,
            AuditAction::RunStarted,
            ResourceType::Run,
            run.to_string(),
            None,
            info.created_at,
        ),
        idempotency: sagat_application::definitions::Idempotency {
            scope: format!("run.start:{}", info.namespace_id),
            key: key(),
            fingerprint: vec![1],
        },
        reset_sequence: None,
        pending_reset: None,
        info,
    };
    let Err(refused) = RunStore::create(&w.store, new).await else {
        panic!("a run whose input was checked against another version was created");
    };
    assert!(
        matches!(refused, StoreError::Internal { .. }),
        "{refused:?}"
    );
    assert_eq!(runs_of(&w, workflow).await, 1);
}

/// Re-review item 4 (D043-151): a retry refused because the validator had no room, whose key already holds a run, is
/// answered with that run, as it was before design 043. The pool's one permit is held by the test (no sleep).
#[tokio::test]
async fn a_retry_with_no_room_replays_the_run_its_key_holds() {
    use sagat_infra_schema::run_input::permits::Permits;

    let w = world().await;
    let (workflow, _) = w
        .publish(with_schema(flow("replay-busy"), order_schema()))
        .await;
    let key = key();
    let input = serde_json::to_vec(&json!({"orderId": "o-1"})).expect("json");
    let request = StartRun {
        namespace: "payments",
        workflow,
        version: None,
        input: &input,
        correlation_id: None,
        deadline: None,
        key: &key,
    };
    let first = w
        .runs
        .start(&w.admin, &request, &ctx())
        .await
        .expect("started");
    let validator = Arc::new(RunInputValidator::with_permits(Permits::new(
        1,
        std::time::Duration::from_millis(20),
    )));
    let held = validator
        .permits()
        .acquire(w.admin.tenant_id())
        .await
        .expect("the only permit");
    let saturated = RunService::new(
        w.store.clone(),
        SystemClock,
        Engine::new(JsonEvaluator),
        w.cache.clone(),
    )
    .with_run_inputs(Arc::new(RunInputGate::new(
        validator.clone(),
        InputValidationMode::Enforce,
    )));
    let again = saturated
        .start(&w.admin, &request, &ctx())
        .await
        .expect("the run of the key, not a 503");
    assert_eq!(again.info.run_id, first.info.run_id);
    drop(held);
}

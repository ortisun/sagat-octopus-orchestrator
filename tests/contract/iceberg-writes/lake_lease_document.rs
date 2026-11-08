//! The lease document of a lake step is made from the run's own data (feature 035, T035-056, D035-138).
//!
//! `lake_lease_document` evaluates the step's record source over the run's data, takes the strictest limits of the step and of the
//! connection, and builds the attempt input the runner reads. It is pure, so it is held here, where the server's end and the
//! runner's reader meet: what it makes the reader accepts and reads back, and what it cannot make it refuses with a closed code that
//! is permanent (the data of a run does not change, so a retry would fail the same way).

use sagat_application::{
    execution::{ExpressionEvaluator, RunData},
    worker::{lake_input::LakeFacts, lake_lease::lake_lease_document},
};
use sagat_domain::database::{
    ColumnMapping, ConnectionRef, DatabaseFailure, IcebergWrite, IcebergWriteParts, LakeLimits,
    RecordSource, TableIdentifier, WriteIntent,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{Expression, NodeId};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_lake::protocol::{AttemptInput, Records};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn write(records: RecordSource, limits: LakeLimits) -> IcebergWrite {
    IcebergWrite::new(IcebergWriteParts {
        connection: ConnectionRef::new(Slug::parse("lake-main").expect("slug"), None).expect("ref"),
        table: TableIdentifier::new(vec!["sales".to_owned()], "orders").expect("table"),
        reference: "main".to_owned(),
        intent: WriteIntent::Append,
        records,
        mapping: vec![ColumnMapping::new("/amount", "amount").expect("mapping")],
        ignore_unknown_fields: false,
        limits,
        revert: false,
    })
    .expect("a write")
}

fn inline(expression: &str) -> RecordSource {
    RecordSource::Inline(Expression::parse(expression).expect("expression"))
}

fn data(input: &Value) -> RunData {
    RunData {
        run_id: None,
        input: serde_json::to_vec(input).expect("input"),
        input_blob: None,
        state: None,
        state_revision: None,
        outputs: BTreeMap::new(),
        scope_inputs: BTreeMap::new(),
    }
}

fn facts() -> LakeFacts<'static> {
    LakeFacts {
        effect_key: "run-1:load",
        first_attempt_started_at: time::macros::datetime!(2026-11-08 10:00:00 UTC),
        run_id: "run-1",
        step_id: "step-1",
        workflow: "orders-sync",
        workflow_version: "3",
        prior_attempt_may_have_left_files: false,
        allow_empty: false,
    }
}

fn lease(
    source: RecordSource,
    step_limits: LakeLimits,
    connection_limits: LakeLimits,
    input: &Value,
) -> Result<Vec<u8>, DatabaseFailure> {
    lake_lease_document(
        &JsonEvaluator,
        &write(source, step_limits),
        &data(input),
        &facts(),
        connection_limits,
    )
}

#[test]
fn the_records_are_the_values_the_expression_selects_from_the_run_input_and_the_reader_reads_them_back()
 {
    let rows = json!([{"amount": "12.50", "sku": "A-1"}, {"amount": "3"}]);
    let built = lease(
        inline("input.rows"),
        LakeLimits::default(),
        LakeLimits::default(),
        &json!({"rows": rows}),
    )
    .expect("a document");
    let input = AttemptInput::read(&built).expect("the runner's reader accepts it");
    assert_eq!(input.effect_key(), "run-1:load");
    assert!(
        matches!(input.records(), Records::Inline(items) if items.len() == 2),
        "{:?}",
        String::from_utf8_lossy(&built)
    );
    let document: Value = serde_json::from_slice(&built).expect("JSON");
    assert_eq!(
        document["records"]["inline"], rows,
        "the records are exactly the selected values"
    );
    assert_eq!(document["priorAttemptMayHaveLeftFiles"], false);
    assert_eq!(document["allowEmpty"], false);
}

#[test]
fn an_empty_selection_is_a_document_with_no_record_and_the_write_decides_what_that_means() {
    let built = lease(
        inline("input.rows"),
        LakeLimits::default(),
        LakeLimits::default(),
        &json!({"rows": []}),
    )
    .expect("a document");
    assert!(matches!(
        AttemptInput::read(&built).expect("accepted").records(),
        Records::Inline(items) if items.is_empty()
    ));
}

#[test]
fn the_limits_are_the_strictest_of_the_step_and_of_the_connection() {
    let step = LakeLimits {
        max_records: Some(500),
        ..LakeLimits::default()
    };
    let connection = LakeLimits {
        max_records: Some(1_000),
        timeout_ms: Some(120_000),
        ..LakeLimits::default()
    };
    let built =
        lease(inline("input.rows"), step, connection, &json!({"rows": []})).expect("a document");
    let input = AttemptInput::read(&built).expect("accepted");
    assert_eq!(
        input.limits().max_records,
        500,
        "the step asked for less than the connection allows"
    );
    assert_eq!(
        input.limits().timeout_ms,
        120_000,
        "the connection is stricter than the default"
    );
}

#[test]
fn what_is_not_a_list_of_objects_is_a_configuration_failure() {
    for (name, input) in [
        ("an object", json!({"rows": {"amount": 1}})),
        ("a number", json!({"rows": 7})),
        ("a missing value", json!({})),
        ("a list of scalars", json!({"rows": [1, 2]})),
        (
            "a list with a scalar among objects",
            json!({"rows": [{"amount": "1"}, "x"]}),
        ),
    ] {
        let refused = lease(
            inline("input.rows"),
            LakeLimits::default(),
            LakeLimits::default(),
            &input,
        )
        .expect_err(name);
        assert_eq!(refused, DatabaseFailure::Configuration, "{name}");
        assert_eq!(refused.code(), "DATABASE_CONFIGURATION_INVALID");
    }
}

#[test]
fn a_source_the_lease_cannot_serve_yet_is_refused_by_name_and_not_left_to_hang() {
    for source in [
        RecordSource::Blobs(Expression::parse("input.parts").expect("expression")),
        RecordSource::DatabaseResult(NodeId::parse("read").expect("node")),
    ] {
        let refused = lease(
            source,
            LakeLimits::default(),
            LakeLimits::default(),
            &json!({"parts": []}),
        )
        .expect_err("not served");
        assert_eq!(refused, DatabaseFailure::Capability);
        assert_eq!(refused.code(), "DATABASE_CAPABILITY_UNAVAILABLE");
    }
}

#[test]
fn a_document_over_the_bound_the_runner_reads_is_a_limit_failure() {
    // 40 records of 128 KiB each is over 4 MiB.
    let big = "x".repeat(128 * 1024);
    let rows: Vec<Value> = (0..40).map(|i| json!({"amount": big, "n": i})).collect();
    let refused = lease(
        inline("input.rows"),
        LakeLimits::default(),
        LakeLimits::default(),
        &json!({"rows": rows}),
    );
    // The evaluator may refuse the selection before the builder sees it; either way it is a closed, permanent code and not a lease.
    let refused = refused.expect_err("over the bound");
    assert_eq!(refused, DatabaseFailure::IcebergLimitExceeded);
}

#[test]
fn the_evaluator_is_the_one_the_workflow_uses() {
    // The record source is judged by the same expression semantics as every other expression of the workflow.
    let selected = JsonEvaluator
        .run_result(
            &Expression::parse("input.rows").expect("expression"),
            &data(&json!({"rows": [{"a": 1}]})),
        )
        .expect("evaluated");
    assert_eq!(
        serde_json::from_slice::<Value>(&selected).expect("JSON"),
        json!([{"a": 1}])
    );
}

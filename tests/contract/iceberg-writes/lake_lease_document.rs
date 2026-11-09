//! The lease document of a lake step is made from the run's own data (feature 035, T035-056, D035-138).
//!
//! `lake_lease_document` evaluates the step's record source over the run's data, takes the strictest limits of the step and of the
//! connection, and builds the attempt input the runner reads. It is pure, so it is held here, where the server's end and the
//! runner's reader meet: what it makes the reader accepts and reads back, and what it cannot make it refuses with a closed code that
//! is permanent (the data of a run does not change, so a retry would fail the same way).

use sagat_application::{
    execution::{ExpressionEvaluator, RunData},
    worker::{
        lake_input::LakeFacts,
        lake_lease::{lake_lease_document, named_records},
    },
};
use sagat_domain::blobs::{BlobDescriptor, BlobId, ContentType};
use sagat_domain::database::{
    ColumnMapping, ConnectionRef, DatabaseFailure, IcebergWrite, IcebergWriteParts, LakeLimits,
    RecordSource, TableIdentifier, WriteIntent,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{Digest, Expression};
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
    let write = write(source, step_limits);
    let named = named_records(&JsonEvaluator, &write, &data(input))?;
    lake_lease_document(&write, &facts(), named, &[], connection_limits)
}

const NDJSON: &str = "application/x-ndjson";
const PARQUET: &str = "application/vnd.apache.parquet";

fn blob(size: u64, content_type: &str) -> BlobDescriptor {
    BlobDescriptor::new(
        BlobId::generate(),
        size,
        Digest::new([7; 32]),
        ContentType::parse(content_type).expect("a media type"),
    )
}

/// The descriptor as a workflow value carries it.
fn value_of(descriptor: &BlobDescriptor) -> Value {
    json!({"$blob": {
        "id": descriptor.id().to_string(),
        "size": descriptor.size(),
        "sha256": descriptor.digest().to_hex(),
        "contentType": descriptor.content_type().as_str(),
    }})
}

/// The lease of a blob source `input.parts`, given what the run's data holds and what the catalog says about the blobs.
fn lease_blobs(named: &Value, found: &[BlobDescriptor]) -> Result<Vec<u8>, DatabaseFailure> {
    let write = write(
        RecordSource::Blobs(Expression::parse("input.parts").expect("expression")),
        LakeLimits::default(),
    );
    let named = named_records(&JsonEvaluator, &write, &data(&json!({"parts": named})))?;
    lake_lease_document(&write, &facts(), named, found, LakeLimits::default())
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
fn the_blobs_a_step_names_are_the_blobs_of_the_document_in_the_order_it_names_them_with_their_formats()
 {
    let (first, second, third) = (blob(10, PARQUET), blob(20, NDJSON), blob(30, PARQUET));
    // The catalog answers in its own order and may know more than the step named.
    let extra = blob(5, NDJSON);
    let built = lease_blobs(
        &json!([value_of(&second), value_of(&first), value_of(&third)]),
        &[third.clone(), extra, first.clone(), second.clone()],
    )
    .expect("a document");
    let input = AttemptInput::read(&built).expect("the runner's reader accepts it");
    let Records::Blobs(blobs) = input.records() else {
        panic!("blobs expected: {}", String::from_utf8_lossy(&built));
    };
    let named: Vec<(String, String)> = blobs
        .iter()
        .map(|blob| (blob.id.to_string(), format!("{:?}", blob.format)))
        .collect();
    assert_eq!(
        named,
        vec![
            (second.id().to_string(), "Ndjson".to_owned()),
            (first.id().to_string(), "Parquet".to_owned()),
            (third.id().to_string(), "Parquet".to_owned()),
        ],
        "the step's order, the format of each blob's own media type, and nothing the step did not name"
    );
}

#[test]
fn a_lone_descriptor_is_a_list_of_one() {
    let only = blob(1, NDJSON);
    let built = lease_blobs(&value_of(&only), std::slice::from_ref(&only)).expect("a document");
    assert!(matches!(
        AttemptInput::read(&built).expect("accepted").records(),
        Records::Blobs(blobs) if blobs.len() == 1 && blobs[0].id == only.id()
    ));
}

#[test]
fn thirty_two_blobs_are_the_most_a_step_may_name() {
    let many: Vec<BlobDescriptor> = (0..33).map(|_| blob(1, NDJSON)).collect();
    let values = |count: usize| Value::Array(many[..count].iter().map(value_of).collect());
    let built = lease_blobs(&values(32), &many).expect("32 is the ceiling");
    assert!(matches!(
        AttemptInput::read(&built).expect("accepted").records(),
        Records::Blobs(blobs) if blobs.len() == 32
    ));
    assert_eq!(
        lease_blobs(&values(33), &many).expect_err("33 is over it"),
        DatabaseFailure::Configuration
    );
}

#[test]
fn the_catalog_is_never_asked_about_more_than_thirty_two_blobs_none_or_one_twice() {
    // `named_records` is what the lease hands the catalog: its own refusal is what keeps a query from carrying such a list, and the
    // document builder's is a second line, not the first.
    let write = write(
        RecordSource::Blobs(Expression::parse("input.parts").expect("expression")),
        LakeLimits::default(),
    );
    let many: Vec<BlobDescriptor> = (0..33).map(|_| blob(1, NDJSON)).collect();
    let named = |parts: Vec<&BlobDescriptor>| {
        let parts: Vec<Value> = parts.into_iter().map(value_of).collect();
        named_records(&JsonEvaluator, &write, &data(&json!({"parts": parts})))
    };
    assert!(named(many.iter().take(32).collect()).is_ok());
    for (name, parts) in [
        ("33", many.iter().collect()),
        ("none", vec![]),
        ("one twice", vec![&many[0], &many[0]]),
    ] {
        assert_eq!(named(parts), Err(DatabaseFailure::Configuration), "{name}");
    }
}

#[test]
fn what_does_not_name_a_list_of_blobs_is_a_configuration_failure_before_the_catalog_is_asked() {
    let one = blob(1, NDJSON);
    let found = std::slice::from_ref(&one);
    for (name, named) in [
        ("none", json!([])),
        ("a number", json!(7)),
        ("an object that is not a descriptor", json!({"id": "x"})),
        ("a scalar in the list", json!([value_of(&one), 1])),
        ("a bare id", json!([one.id().to_string()])),
        (
            "the same blob twice",
            json!([value_of(&one), value_of(&one)]),
        ),
        (
            "a descriptor with a member more",
            json!([{"$blob": {
                "id": one.id().to_string(), "size": 1, "sha256": one.digest().to_hex(),
                "contentType": NDJSON, "extra": true,
            }}]),
        ),
    ] {
        assert_eq!(
            lease_blobs(&named, found).expect_err(name),
            DatabaseFailure::Configuration,
            "{name}"
        );
    }
}

#[test]
fn a_blob_that_is_not_a_file_of_records_is_a_configuration_failure() {
    for content_type in [
        "application/json",
        "application/octet-stream",
        "text/csv",
        "application/x-ndjson-not",
    ] {
        let wrong = blob(1, content_type);
        assert_eq!(
            lease_blobs(&json!([value_of(&wrong)]), std::slice::from_ref(&wrong))
                .expect_err(content_type),
            DatabaseFailure::Configuration,
            "{content_type}"
        );
    }
}

#[test]
fn a_blob_the_catalog_does_not_vouch_for_is_unavailable_and_says_nothing_of_why() {
    let named = blob(100, NDJSON);
    let with = |changed: BlobDescriptor| {
        lease_blobs(&json!([value_of(&named)]), &[changed]).expect_err("not vouched for")
    };
    // Not in the catalog for this run (another run's, another tenant's, not committed, deleted): the answer is the same.
    assert_eq!(
        lease_blobs(&json!([value_of(&named)]), &[]).expect_err("absent"),
        DatabaseFailure::ResultUnavailable
    );
    assert_eq!(
        lease_blobs(&json!([value_of(&named)]), &[blob(100, NDJSON)]).expect_err("another blob"),
        DatabaseFailure::ResultUnavailable
    );
    // A descriptor in a JSON value proves nothing: size, digest and media type are the catalog's.
    let same = |size, digest: [u8; 32], content_type: &str| {
        BlobDescriptor::new(
            named.id(),
            size,
            Digest::new(digest),
            ContentType::parse(content_type).expect("media type"),
        )
    };
    for (name, changed) in [
        ("another size", same(101, [7; 32], NDJSON)),
        ("another digest", same(100, [8; 32], NDJSON)),
        ("another media type", same(100, [7; 32], PARQUET)),
    ] {
        assert_eq!(with(changed), DatabaseFailure::ResultUnavailable, "{name}");
    }
}

#[test]
fn the_blobs_of_a_step_are_at_most_two_gibibytes_together() {
    const GIB: u64 = 1024 * 1024 * 1024;
    let (first, second) = (blob(GIB, PARQUET), blob(GIB, PARQUET));
    let both = [first.clone(), second.clone()];
    lease_blobs(&json!([value_of(&first), value_of(&second)]), &both)
        .expect("exactly 2 GiB is the ceiling");
    let over = blob(1, NDJSON);
    let refused = lease_blobs(
        &json!([value_of(&first), value_of(&second), value_of(&over)]),
        &[first, second, over],
    )
    .expect_err("one byte over");
    assert_eq!(refused, DatabaseFailure::IcebergLimitExceeded);
}

#[test]
fn a_sum_that_overflows_is_a_limit_failure_and_not_a_wrap() {
    // Two halves of the range of a u64 add up to 0 when the sum wraps, which is under every ceiling.
    let half = |_| blob(1 << 63, NDJSON);
    let two: Vec<BlobDescriptor> = (0..2).map(half).collect();
    let named = Value::Array(two.iter().map(value_of).collect());
    assert_eq!(
        lease_blobs(&named, &two).expect_err("overflow"),
        DatabaseFailure::IcebergLimitExceeded
    );
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

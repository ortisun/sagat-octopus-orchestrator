//! The result a SQL step retained is the records of a lake write (feature 035, D035-145, T035-056, G035-067).
//!
//! The step names the node whose result it takes. The retained result is the typed document of feature 017 (columns, and rows of typed
//! cells); the lease turns it into the records the host reads: one object a row, keyed by the column, each value as the cell holds it, and
//! a `binary` cell kept as the typed cell, which is how the host reads a binary column. Whatever cannot be turned into records fails the
//! step for good with a closed code (the result of a run does not change, so a retry would fail the same way); a result above the inline
//! bound is a blob, and is refused by name until it is read.
use sagat_application::{
    database::DatabaseResultValidator,
    execution::RunData,
    worker::{
        lake_input::LakeRecords,
        lake_lease::{NamedRecords, named_records},
    },
};
use sagat_domain::blobs::BlobId;
use sagat_domain::database::{
    ColumnMapping, ConnectionRef, DatabaseFailure, DatabaseLimits, IcebergWrite, IcebergWriteParts,
    LakeLimits, RecordSource, TableIdentifier, WriteIntent,
};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{Digest, NodeId};
use sagat_infra_ir::{JsonDatabaseResultValidator, JsonEvaluator};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn write() -> IcebergWrite {
    IcebergWrite::new(IcebergWriteParts {
        connection: ConnectionRef::new(Slug::parse("lake-main").expect("slug"), None).expect("ref"),
        table: TableIdentifier::new(vec!["sales".to_owned()], "orders").expect("table"),
        reference: "main".to_owned(),
        intent: WriteIntent::Append,
        records: RecordSource::DatabaseResult(NodeId::parse("read").expect("node")),
        mapping: vec![ColumnMapping::new("/id", "id").expect("mapping")],
        ignore_unknown_fields: false,
        limits: LakeLimits::default(),
        revert: false,
    })
    .expect("a write")
}

/// The run's data once the SQL step `read` succeeded with `output`, or with no output at all.
fn data(output: Option<&[u8]>) -> RunData {
    let mut outputs = BTreeMap::new();
    let mut data = RunData {
        run_id: None,
        input: b"{}".to_vec(),
        input_blob: None,
        state: None,
        state_revision: None,
        outputs: std::mem::take(&mut outputs),
        scope_inputs: BTreeMap::new(),
    };
    if let Some(output) = output {
        data.insert_root_output(NodeId::parse("read").expect("node"), output.to_vec());
    }
    data
}

fn records_of(result: &Value) -> Result<Vec<Value>, DatabaseFailure> {
    let bytes = serde_json::to_vec(result).expect("JSON");
    match named_records(&JsonEvaluator, &write(), &data(Some(&bytes)))? {
        NamedRecords::Inline(records) => Ok(records),
        other => panic!("inline records expected: {other:?}"),
    }
}

fn result(columns: &Value, rows: &Value) -> Value {
    json!({"columns": columns, "rows": rows, "affectedRows": "0", "truncated": false})
}

#[test]
fn the_rows_of_a_result_are_objects_keyed_by_the_column_with_each_value_as_the_cell_holds_it() {
    let columns = json!([
        {"name": "id", "type": "text"},
        {"name": "total", "type": "integer"},
        {"name": "price", "type": "decimal"},
        {"name": "ratio", "type": "float"},
        {"name": "paid", "type": "boolean"},
        {"name": "at", "type": "timestamp"},
        {"name": "day", "type": "date"},
        {"name": "meta", "type": "json"},
        {"name": "note", "type": "text"},
    ]);
    let rows = json!([
        [
            {"type": "text", "value": "a-1"},
            {"type": "integer", "value": "9007199254740993"},
            {"type": "decimal", "value": "12.50"},
            {"type": "float", "value": 0.5},
            {"type": "boolean", "value": true},
            {"type": "timestamp", "value": "2026-11-08T10:00:00Z"},
            {"type": "date", "value": "2026-11-08"},
            {"type": "json", "value": {"k": [1, 2]}},
            {"type": "null", "value": null},
        ],
        [
            {"type": "text", "value": "a-2"},
            {"type": "integer", "value": "-3"},
            {"type": "decimal", "value": "0.01"},
            {"type": "float", "value": 1.0},
            {"type": "boolean", "value": false},
            {"type": "timestamp", "value": "2026-11-08T10:00:01+01:00"},
            {"type": "date", "value": "2026-11-09"},
            {"type": "json", "value": null},
            {"type": "text", "value": "x"},
        ],
    ]);
    assert_eq!(
        records_of(&result(&columns, &rows)).expect("records"),
        vec![
            json!({
                "id": "a-1", "total": "9007199254740993", "price": "12.50", "ratio": 0.5, "paid": true,
                "at": "2026-11-08T10:00:00Z", "day": "2026-11-08", "meta": {"k": [1, 2]}, "note": null,
            }),
            json!({
                "id": "a-2", "total": "-3", "price": "0.01", "ratio": 1.0, "paid": false,
                "at": "2026-11-08T10:00:01+01:00", "day": "2026-11-09", "meta": null, "note": "x",
            }),
        ],
        "an integer or a decimal stays the string the cell holds: the digits of a value a JSON number cannot hold survive"
    );
}

#[test]
fn a_binary_cell_stays_the_typed_cell_the_host_reads() {
    let records = records_of(&result(
        &json!([{"name": "id", "type": "text"}, {"name": "blob", "type": "binary"}]),
        &json!([[
            {"type": "text", "value": "a-1"},
            {"type": "binary", "value": "AAEC"}
        ]]),
    ))
    .expect("records");
    assert_eq!(
        records,
        vec![json!({"id": "a-1", "blob": {"type": "binary", "value": "AAEC"}})]
    );
}

#[test]
fn a_json_value_is_a_value_unless_it_is_spelled_as_a_typed_binary_cell() {
    let spelled = |value: Value| {
        records_of(&result(
            &json!([{"name": "j", "type": "json"}]),
            &json!([[{"type": "json", "value": value.clone()}]]),
        ))
        .map(|mut records| records.remove(0)["j"].clone())
    };
    for value in [
        json!({"type": "binary"}),
        json!({"type": "text", "value": "x"}),
        json!({"type": "binary", "value": "AAEC", "more": 1}),
        json!({"kind": "binary", "value": "AAEC"}),
        json!(["binary", "AAEC"]),
        json!("binary"),
    ] {
        assert_eq!(spelled(value.clone()), Ok(value.clone()), "{value}");
    }
}

/// The other half of `runners/lake/tests/test_result_records.py`: each cell of the one list both are held to becomes the value the host is
/// tested with, and is a cell the platform keeps (a list of cells the validator would refuse would hold the host to nothing).
#[test]
fn every_cell_of_the_shared_fixture_becomes_the_value_the_host_is_held_to() {
    let document = super::read_json(&super::fixtures().join("result-records/cells.json"));
    let cases = document["cells"].as_array().expect("the cells");
    assert!(
        cases.len() >= 14,
        "the fixture is the list of every kind of cell"
    );
    for case in cases {
        let name = case["name"].as_str().expect("a name");
        let cell = &case["cell"];
        let kind = cell["type"].as_str().expect("a type");
        // A null cell sits in a column of some type: the value of the cell says it is null.
        let column = if kind == "null" { "text" } else { kind };
        let retained = result(&json!([{"name": "x", "type": column}]), &json!([[cell]]));
        let bytes = serde_json::to_vec(&retained).expect("JSON");
        assert!(
            JsonDatabaseResultValidator
                .validate(&bytes, DatabaseLimits::MAX)
                .is_ok(),
            "{name}: the platform keeps this cell"
        );
        assert_eq!(
            records_of(&retained),
            Ok(vec![json!({"x": case["record"]})]),
            "{name}"
        );
    }
}

/// The rows of a result are values of the customer: whatever prints the records of a write (a `{:?}` in a log, a panic, a test that fails)
/// prints how many there are, and never what they say.
#[test]
fn the_records_of_a_write_print_how_many_they_are_and_never_what_they_say() {
    let records = records_of(&result(
        &json!([{"name": "id", "type": "text"}]),
        &json!([[{"type": "text", "value": "CANARY-ROW"}]]),
    ))
    .expect("records");
    let named = NamedRecords::Inline(records.clone());
    let leased = LakeRecords::Inline(records);
    for (what, printed) in [
        ("the named records", format!("{named:?}")),
        ("the records of the document", format!("{leased:?}")),
    ] {
        assert!(!printed.contains("CANARY"), "{what}: {printed}");
        assert!(printed.contains('1'), "{what} say how many: {printed}");
    }
}

#[test]
fn a_result_of_no_row_is_a_selection_of_no_record_and_the_write_decides_what_that_means() {
    let records = records_of(&result(
        &json!([{"name": "id", "type": "text"}]),
        &json!([]),
    ))
    .expect("records");
    assert!(records.is_empty());
}

#[test]
fn what_cannot_be_records_fails_for_good_with_the_code_that_says_why() {
    let id = json!([{"name": "id", "type": "text"}]);
    let row = json!([[{"type": "text", "value": "a"}]]);
    for (name, document, expected) in [
        (
            "a statement that selects no column",
            result(&json!([]), &json!([[]])),
            DatabaseFailure::Configuration,
        ),
        (
            "two columns of one name",
            result(
                &json!([{"name": "id", "type": "text"}, {"name": "id", "type": "text"}]),
                &json!([[{"type": "text", "value": "a"}, {"type": "text", "value": "b"}]]),
            ),
            DatabaseFailure::Configuration,
        ),
        (
            "a result the step cut short",
            json!({"columns": id, "rows": row, "affectedRows": "0", "truncated": true}),
            DatabaseFailure::ResultUnavailable,
        ),
        (
            "a row of the wrong width",
            result(
                &id,
                &json!([[{"type": "text", "value": "a"}, {"type": "text", "value": "b"}]]),
            ),
            DatabaseFailure::ResultUnavailable,
        ),
        (
            "a member the document does not have",
            json!({"columns": id, "rows": row, "affectedRows": "0", "truncated": false, "extra": 1}),
            DatabaseFailure::ResultUnavailable,
        ),
        (
            "a cell that is not a typed cell",
            result(&id, &json!([["a"]])),
            DatabaseFailure::ResultUnavailable,
        ),
        (
            "not the document of a result at all",
            json!({"rows": 3}),
            DatabaseFailure::ResultUnavailable,
        ),
        (
            "a column with no name",
            result(&json!([{"name": "", "type": "text"}]), &json!([])),
            DatabaseFailure::Configuration,
        ),
        (
            "a count of affected rows that is not text",
            json!({"columns": id, "rows": row, "affectedRows": 0, "truncated": false}),
            DatabaseFailure::ResultUnavailable,
        ),
        (
            "a flag that is not a boolean",
            json!({"columns": id, "rows": row, "affectedRows": "0", "truncated": "no"}),
            DatabaseFailure::ResultUnavailable,
        ),
        (
            // The host reads this very shape as bytes for a binary column: a value of the database that spells it would be written as such.
            "a json value that spells a typed binary cell",
            result(
                &json!([{"name": "j", "type": "json"}]),
                &json!([[{"type": "json", "value": {"type": "binary", "value": "AAEC"}}]]),
            ),
            DatabaseFailure::ResultUnavailable,
        ),
        (
            "a column that names no type",
            result(&json!([{"name": "id", "type": 7}]), &json!([])),
            DatabaseFailure::ResultUnavailable,
        ),
    ] {
        assert_eq!(records_of(&document), Err(expected), "{name}");
    }
}

#[test]
fn the_output_of_a_step_that_did_not_succeed_is_an_unavailable_result() {
    assert_eq!(
        named_records(&JsonEvaluator, &write(), &data(None)),
        Err(DatabaseFailure::ResultUnavailable),
        "the node produced no output: it failed, or was skipped by an error edge"
    );
}

#[test]
fn a_result_above_the_inline_bound_is_a_blob_and_is_refused_by_name_until_it_is_read() {
    let blob = json!({"$blob": {
        "id": BlobId::generate().to_string(),
        "size": 300_000,
        "sha256": Digest::new([1; 32]).to_hex(),
        "contentType": "application/json",
    }});
    let bytes = serde_json::to_vec(&blob).expect("JSON");
    let refused = named_records(&JsonEvaluator, &write(), &data(Some(&bytes))).expect_err("a blob");
    assert_eq!(refused, DatabaseFailure::Capability);
    assert_eq!(refused.code(), "DATABASE_CAPABILITY_UNAVAILABLE");
}

/// What the platform keeps of a result is what its own validator accepted: every document it accepts is turned into records, or
/// refused for what it says (no column, a column twice), never refused as a document the converter cannot read.
#[test]
fn whatever_the_validator_of_the_result_accepts_is_a_result_the_lease_can_read() {
    let validator = JsonDatabaseResultValidator;
    let types = [
        ("null", json!(null)),
        ("boolean", json!(true)),
        ("integer", json!("42")),
        ("decimal", json!("1.50")),
        ("float", json!(2.5)),
        ("text", json!("t")),
        ("timestamp", json!("2026-11-08T10:00:00Z")),
        ("date", json!("2026-11-08")),
        ("binary", json!("AAEC")),
        ("json", json!({"a": [1]})),
    ];
    for (kind, value) in &types {
        let document = result(
            &json!([{"name": "c", "type": kind}]),
            &json!([[{"type": kind, "value": value}], [{"type": "null", "value": null}]]),
        );
        let bytes = serde_json::to_vec(&document).expect("JSON");
        // Every kind of the list is one the validator keeps: were one dropped from it the loop would skip it, and say nothing.
        assert!(
            validator.validate(&bytes, DatabaseLimits::MAX).is_ok(),
            "the validator keeps a {kind} cell"
        );
        let records = records_of(&document).unwrap_or_else(|error| panic!("{kind}: {error:?}"));
        assert_eq!(records.len(), 2, "{kind}");
        assert_eq!(records[1]["c"], Value::Null, "a null cell is null: {kind}");
    }
    // The shapes the validator accepts and the converter names: no column, a column twice.
    for document in [
        result(&json!([]), &json!([])),
        result(
            &json!([{"name": "c", "type": "text"}, {"name": "c", "type": "text"}]),
            &json!([]),
        ),
    ] {
        let bytes = serde_json::to_vec(&document).expect("JSON");
        assert!(
            validator.validate(&bytes, DatabaseLimits::MAX).is_ok(),
            "{document}"
        );
        assert_eq!(records_of(&document), Err(DatabaseFailure::Configuration));
    }
}

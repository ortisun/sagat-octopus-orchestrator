//! `sagat.lake.v1` (feature 035, T035-017, D035-062, D035-067): one request line, progress lines at most once a second,
//! exactly one result line, and anything else is `LAKE_HOST_PROTOCOL`. The runner rebuilds the receipt from typed
//! fields, so a field the contract does not name never reaches the journal, and the host's standard error is reduced
//! to a closed exception class and a line count.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Duration;

use jsonschema::Validator;
use sagat_domain::database::DatabaseFailure;
use sagat_infra_lake::protocol::{
    Decoder, EncodeError, Event, ExceptionClass, MAX_LINE_BYTES, Operation, Outcome, PROTOCOL,
    ProtocolFailure, Receipt, RelayAccess, RelayToken, Request, StderrSummary, TokenError,
    encode_request, summarize_stderr,
};
use serde_json::{Map, Value, json};

const TOKEN: &str = "relay-token-canary-0123456789abcdef";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn receipts_schema() -> Validator {
    let path = root().join("specs/035-iceberg-write-steps/contracts/receipts.schema.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    jsonschema::options()
        .build(&schema)
        .expect("the receipt schema compiles")
}

fn sample(name: &str) -> Value {
    let path = root().join(format!(
        "tests/contract/iceberg-writes/fixtures/receipts/{name}.json"
    ));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn line(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

fn result_line(receipt: &Value) -> Vec<u8> {
    line(&json!({"receipt": receipt, "relayDenials": 0}))
}

fn body(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

fn request(extra: Value) -> Request {
    Request::new(
        Operation::IcebergWrite,
        "run-7f3a/load/1",
        "2026-11-05T10:00:00.000Z",
        RelayAccess::new("/work/a1/relay.sock", RelayToken::new(TOKEN).unwrap()),
        body(extra),
    )
}

// -- the request ------------------------------------------------------------------------------------------------------

#[test]
fn the_request_is_one_line_that_ends_with_a_newline() {
    let encoded = encode_request(&request(json!({
        "table": {"namespace": ["sales"], "name": "orders"},
        "mode": "append",
        "note": "a value with a\nnewline and a \u{2028} separator",
    })))
    .unwrap();

    assert_eq!(encoded.iter().filter(|byte| **byte == b'\n').count(), 1);
    assert_eq!(encoded.last(), Some(&b'\n'));
    let parsed: Value = serde_json::from_slice(&encoded[..encoded.len() - 1]).unwrap();
    assert_eq!(parsed["protocol"], PROTOCOL);
    assert_eq!(PROTOCOL, "sagat.lake.v1");
    assert_eq!(parsed["operation"], "icebergWrite");
    assert_eq!(parsed["effectKey"], "run-7f3a/load/1");
    assert_eq!(parsed["firstAttemptStartedAt"], "2026-11-05T10:00:00.000Z");
    assert_eq!(
        parsed["relay"],
        json!({"socket": "/work/a1/relay.sock", "token": TOKEN})
    );
    assert_eq!(parsed["table"]["name"], "orders");
    assert_eq!(
        parsed["note"],
        "a value with a\nnewline and a \u{2028} separator"
    );
}

#[test]
fn every_operation_has_the_name_the_contract_gives_it() {
    let names: Vec<(Operation, &str)> = vec![
        (Operation::IcebergWrite, "icebergWrite"),
        (Operation::IcebergRevert, "icebergRevert"),
        (Operation::ObjectWrite, "objectWrite"),
        (Operation::Probe, "probe"),
        (Operation::Browse, "browse"),
    ];
    for (operation, name) in names {
        let encoded = encode_request(&Request::new(
            operation,
            "k",
            "2026-11-05T10:00:00Z",
            RelayAccess::new("/work/a/relay.sock", RelayToken::new(TOKEN).unwrap()),
            Map::new(),
        ))
        .unwrap();
        let parsed: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(parsed["operation"], name);
    }
}

#[test]
fn a_body_cannot_replace_a_member_of_the_envelope() {
    for reserved in [
        "protocol",
        "operation",
        "effectKey",
        "firstAttemptStartedAt",
        "relay",
    ] {
        let mut members = Map::new();
        members.insert(reserved.to_owned(), json!("x"));
        let outcome = encode_request(&request(Value::Object(members)));
        assert_eq!(outcome, Err(EncodeError::ReservedMember), "{reserved}");
    }
}

#[test]
fn the_debug_form_of_a_request_shows_neither_the_token_nor_the_credentials() {
    let request = request(
        json!({"connection": {"credentials": {"s3.secret-access-key": "credential-canary"}}}),
    );

    let shown = format!("{request:?} {:?}", RelayToken::new(TOKEN).unwrap());

    assert!(!shown.contains("relay-token-canary"), "{shown}");
    assert!(!shown.contains("credential-canary"), "{shown}");
    assert!(
        shown.contains("icebergWrite") || shown.contains("IcebergWrite"),
        "{shown}"
    );
}

// -- progress lines ---------------------------------------------------------------------------------------------------

#[test]
fn a_heartbeat_is_accepted_and_carries_nothing() {
    let mut decoder = Decoder::new();

    assert_eq!(
        decoder.line(br#"{"heartbeat":true}"#, ms(0)),
        Ok(Event::Heartbeat)
    );
}

#[test]
fn heartbeats_at_most_once_a_second_are_accepted_for_as_long_as_the_host_runs() {
    let mut decoder = Decoder::new();

    for second in 0..120 {
        assert_eq!(
            decoder.line(br#"{"heartbeat":true}"#, ms(second * 1000)),
            Ok(Event::Heartbeat),
            "{second}"
        );
    }
}

/// The window starts `HEARTBEAT_WINDOW` before a beat, and before the first second of an attempt that moment does not
/// exist: a beat at exactly zero must not be treated as already out of the window (the floor saturated to zero and
/// `beat <= floor` dropped it, so any number of beats at the same first instant passed).
#[test]
fn a_flood_in_the_first_second_is_a_flood_even_at_the_very_first_instant() {
    let beat = br#"{"heartbeat":true}"#;
    for instant in [ms(0), ms(1), ms(999)] {
        let mut decoder = Decoder::new();
        assert_eq!(decoder.line(beat, instant), Ok(Event::Heartbeat));
        assert_eq!(decoder.line(beat, instant), Ok(Event::Heartbeat));
        assert_eq!(
            decoder.line(beat, instant),
            Err(ProtocolFailure::HeartbeatFlood),
            "{instant:?}"
        );
    }
}

#[test]
fn a_beat_leaves_the_window_exactly_one_second_after_it_arrived() {
    let beat = br#"{"heartbeat":true}"#;
    let mut decoder = Decoder::new();
    assert_eq!(decoder.line(beat, ms(0)), Ok(Event::Heartbeat));
    assert_eq!(decoder.line(beat, ms(500)), Ok(Event::Heartbeat));
    // the first beat is a second old now and out; the second is still in, so one more fits and the next does not
    assert_eq!(decoder.line(beat, ms(1000)), Ok(Event::Heartbeat));
    assert_eq!(
        decoder.line(beat, ms(1000)),
        Err(ProtocolFailure::HeartbeatFlood)
    );
}

#[test]
fn one_extra_heartbeat_inside_a_second_is_tolerated_and_a_flood_is_not() {
    let mut decoder = Decoder::new();
    let beat = br#"{"heartbeat":true}"#;

    assert_eq!(decoder.line(beat, ms(10)), Ok(Event::Heartbeat));
    assert_eq!(decoder.line(beat, ms(400)), Ok(Event::Heartbeat));
    assert_eq!(
        decoder.line(beat, ms(800)),
        Err(ProtocolFailure::HeartbeatFlood)
    );
}

#[test]
fn the_window_slides_so_a_slow_host_is_never_refused_by_an_old_heartbeat() {
    let mut decoder = Decoder::new();
    let beat = br#"{"heartbeat":true}"#;

    assert_eq!(decoder.line(beat, ms(0)), Ok(Event::Heartbeat));
    assert_eq!(decoder.line(beat, ms(500)), Ok(Event::Heartbeat));
    // the first one is a full second old now
    assert_eq!(decoder.line(beat, ms(1000)), Ok(Event::Heartbeat));
}

#[test]
fn a_heartbeat_must_be_exactly_true_and_nothing_else() {
    for bad in [
        r#"{"heartbeat":false}"#,
        r#"{"heartbeat":1}"#,
        r#"{"heartbeat":"true"}"#,
        r#"{"heartbeat":null}"#,
        r#"{"heartbeat":true,"extra":1}"#,
        r#"{"heartbeat":true,"relayDenials":0}"#,
    ] {
        let mut decoder = Decoder::new();
        assert!(decoder.line(bad.as_bytes(), ms(0)).is_err(), "{bad}");
    }
}

// -- the result line --------------------------------------------------------------------------------------------------

#[test]
fn one_result_line_ends_the_conversation() {
    let mut decoder = Decoder::new();
    decoder.line(br#"{"heartbeat":true}"#, ms(0)).unwrap();

    let event = decoder
        .line(&result_line(&sample("iceberg-committed")), ms(1500))
        .unwrap();

    let Event::Result(result) = event else {
        panic!("not a result: {event:?}")
    };
    assert_eq!(result.relay_denials, 0);
    assert!(matches!(
        result.outcome,
        Outcome::Receipt(Receipt::Iceberg(_))
    ));
    assert_eq!(decoder.finish().unwrap(), result);
}

#[test]
fn a_second_result_or_any_line_after_the_result_is_refused() {
    let first = result_line(&sample("iceberg-committed"));
    for next in [
        first.clone(),
        br#"{"heartbeat":true}"#.to_vec(),
        b"{}".to_vec(),
        b"garbage".to_vec(),
    ] {
        let mut decoder = Decoder::new();
        decoder.line(&first, ms(0)).unwrap();
        assert_eq!(
            decoder.line(&next, ms(2000)),
            Err(ProtocolFailure::LineAfterResult)
        );
    }
}

#[test]
fn a_host_that_exits_without_a_result_line_is_a_protocol_failure() {
    let mut decoder = Decoder::new();
    decoder.line(br#"{"heartbeat":true}"#, ms(0)).unwrap();

    assert_eq!(decoder.finish(), Err(ProtocolFailure::NoResult));
    assert_eq!(Decoder::new().finish(), Err(ProtocolFailure::NoResult));
}

#[test]
fn every_protocol_failure_is_lake_host_protocol() {
    for failure in [
        ProtocolFailure::NoResult,
        ProtocolFailure::Malformed,
        ProtocolFailure::UnknownMember,
        ProtocolFailure::LineTooLong,
        ProtocolFailure::HeartbeatFlood,
        ProtocolFailure::LineAfterResult,
        ProtocolFailure::InvalidReceipt,
        ProtocolFailure::InvalidFailure,
    ] {
        assert_eq!(failure.code(), DatabaseFailure::LakeHostProtocol);
        assert_eq!(failure.code().code(), "LAKE_HOST_PROTOCOL");
        assert!(!failure.to_string().is_empty());
    }
}

#[test]
fn a_line_that_is_not_json_of_the_protocol_is_malformed() {
    let valid = result_line(&sample("iceberg-committed"));
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", b"".to_vec()),
        ("spaces", b"   ".to_vec()),
        ("text", b"Traceback (most recent call last):".to_vec()),
        ("a number", b"7".to_vec()),
        ("an array", b"[]".to_vec()),
        ("null", b"null".to_vec()),
        ("a string", br#""receipt""#.to_vec()),
        ("truncated", valid[..valid.len() - 5].to_vec()),
        ("not UTF-8", vec![0xff, 0xfe, b'{', b'}']),
        ("two objects", [valid.clone(), valid.clone()].concat()),
    ];
    for (name, bytes) in cases {
        let mut decoder = Decoder::new();
        assert_eq!(
            decoder.line(&bytes, ms(0)),
            Err(ProtocolFailure::Malformed),
            "{name}"
        );
    }
}

#[test]
fn a_member_the_protocol_does_not_define_is_refused() {
    let receipt = sample("iceberg-committed");
    let cases = [
        json!({"receipt": receipt, "relayDenials": 0, "extra": 1}),
        json!({"receipt": receipt}),
        json!({"relayDenials": 0}),
        json!({}),
        json!({"receipt": receipt, "failure": {"code": "ICEBERG_TABLE_NOT_FOUND"}, "relayDenials": 0}),
        json!({"heartbeat": true, "receipt": receipt, "relayDenials": 0}),
        json!({"result": receipt, "relayDenials": 0}),
    ];
    for case in cases {
        let mut decoder = Decoder::new();
        assert_eq!(
            decoder.line(&line(&case), ms(0)),
            Err(ProtocolFailure::UnknownMember),
            "{case}"
        );
    }
}

#[test]
fn the_count_of_relay_denials_is_a_small_non_negative_integer() {
    let receipt = sample("iceberg-committed");
    for good in [json!(0), json!(7), json!(4_294_967_295_u64)] {
        let mut decoder = Decoder::new();
        let parsed = decoder
            .line(
                &line(&json!({"receipt": receipt, "relayDenials": good})),
                ms(0),
            )
            .unwrap();
        let Event::Result(result) = parsed else {
            panic!("not a result")
        };
        assert_eq!(Value::from(result.relay_denials), good);
    }
    for bad in [
        json!(-1),
        json!(1.5),
        json!("0"),
        json!(null),
        json!(4_294_967_296_u64),
        json!([0]),
    ] {
        let mut decoder = Decoder::new();
        let outcome = decoder.line(
            &line(&json!({"receipt": receipt, "relayDenials": bad})),
            ms(0),
        );
        assert_eq!(outcome, Err(ProtocolFailure::Malformed), "{bad}");
    }
}

#[test]
fn a_line_above_the_limit_is_refused_before_it_is_parsed() {
    let mut decoder = Decoder::new();
    let mut huge = br#"{"heartbeat":true}"#.to_vec();
    huge.resize(MAX_LINE_BYTES + 1, b' ');

    assert_eq!(
        decoder.line(&huge, ms(0)),
        Err(ProtocolFailure::LineTooLong)
    );
}

#[test]
fn a_line_of_exactly_the_limit_is_still_read() {
    let mut decoder = Decoder::new();
    let mut exact = br#"{"heartbeat":true}"#.to_vec();
    exact.resize(MAX_LINE_BYTES, b' ');

    assert_eq!(decoder.line(&exact, ms(0)), Ok(Event::Heartbeat));
}

// -- the receipt is rebuilt from typed fields ------------------------------------------------------------------------

#[test]
fn the_receipts_a_host_can_produce_are_rebuilt_unchanged_and_validate_against_the_schema() {
    let schema = receipts_schema();
    for name in ["iceberg-committed", "object-storage-already-present"] {
        let original = sample(name);
        let mut decoder = Decoder::new();
        let Event::Result(result) = decoder.line(&result_line(&original), ms(0)).unwrap() else {
            panic!("{name}: not a result")
        };
        let Outcome::Receipt(receipt) = result.outcome else {
            panic!("{name}: not a receipt")
        };

        let rebuilt = serde_json::to_value(&receipt).unwrap();

        assert_eq!(rebuilt, original, "{name}");
        assert!(schema.is_valid(&rebuilt), "{name}: {rebuilt}");
    }
}

#[test]
fn a_receipt_of_a_kind_the_host_never_produces_is_refused() {
    for name in [
        "postgresql-upsert",
        "clickhouse-inserted",
        "broker-stream-delivered",
        "failed-continued",
    ] {
        let mut decoder = Decoder::new();
        let outcome = decoder.line(&result_line(&sample(name)), ms(0));
        assert_eq!(outcome, Err(ProtocolFailure::InvalidReceipt), "{name}");
    }
}

#[test]
fn a_receipt_with_a_field_the_contract_does_not_name_is_refused() {
    for (key, value) in [
        ("rows", json!([{"id": 1}])),
        (
            "dataFileLocation",
            json!("s3://lake/warehouse/data/00000.parquet"),
        ),
        ("objectKey", json!("exports/2026/11/05.parquet")),
        ("droppedFieldNames", json!(["ssn"])),
        ("message", json!("Traceback: password=hunter2")),
    ] {
        for name in ["iceberg-committed", "object-storage-already-present"] {
            let mut receipt = sample(name);
            receipt[key] = value.clone();
            let mut decoder = Decoder::new();
            assert_eq!(
                decoder.line(&result_line(&receipt), ms(0)),
                Err(ProtocolFailure::InvalidReceipt),
                "{name}: {key}"
            );
        }
    }
}

#[test]
fn an_object_storage_receipt_whose_typed_fields_do_not_hold_is_refused() {
    let edits: Vec<(&str, &str, Value)> = vec![
        ("an outcome of another kind", "outcome", json!("COMMITTED")),
        ("an outcome outside the enum", "outcome", json!("DONE")),
        ("a format outside the enum", "format", json!("csv")),
        ("a count with a leading zero", "records", json!("01")),
        ("a count as a number", "bytes", json!(9120)),
        ("a flag as a string", "conditionalCreate", json!("true")),
        ("an empty effect key", "effectKey", json!("")),
    ];
    for (name, key, value) in edits {
        let mut receipt = sample("object-storage-already-present");
        receipt[key] = value;
        let mut decoder = Decoder::new();
        assert_eq!(
            decoder.line(&result_line(&receipt), ms(0)),
            Err(ProtocolFailure::InvalidReceipt),
            "{name}"
        );
    }
    for key in ["kind", "outcome", "effectKey", "foundByLookup", "format"] {
        let mut receipt = sample("object-storage-already-present");
        receipt.as_object_mut().unwrap().remove(key);
        let mut decoder = Decoder::new();
        assert_eq!(
            decoder.line(&result_line(&receipt), ms(0)),
            Err(ProtocolFailure::InvalidReceipt),
            "missing {key}"
        );
    }
}

#[test]
fn a_receipt_whose_typed_fields_do_not_hold_is_refused() {
    let edits: Vec<(&str, &str, Value)> = vec![
        ("an outcome outside the enum", "outcome", json!("DONE")),
        ("an outcome of another kind", "outcome", json!("CREATED")),
        ("a mode outside the enum", "mode", json!("merge")),
        ("a count with a leading zero", "rowsAdded", json!("01")),
        (
            "a count above 19 digits",
            "rowsAdded",
            json!("12345678901234567890"),
        ),
        (
            "a count above u64",
            "rowsAdded",
            json!("18446744073709551616"),
        ),
        ("a count as a number", "rowsAdded", json!(1000)),
        ("a negative count", "rowsAdded", json!("-1")),
        ("a count that is empty", "rowsAdded", json!("")),
        (
            "a snapshot id as a number",
            "snapshotId",
            json!(8744736658442914487_i64),
        ),
        (
            "a snapshot id that is not an integer",
            "snapshotId",
            json!("12a"),
        ),
        (
            "a snapshot id above i64",
            "snapshotId",
            json!("9223372036854775808"),
        ),
        (
            "a snapshot id with a leading zero",
            "snapshotId",
            json!("007"),
        ),
        ("a negative zero snapshot id", "snapshotId", json!("-0")),
        ("a snapshot id with a plus sign", "snapshotId", json!("+5")),
        (
            "a snapshot id that is only a sign",
            "snapshotId",
            json!("-"),
        ),
        ("an empty snapshot id", "snapshotId", json!("")),
        ("a format version of 3", "formatVersion", json!(3)),
        (
            "a row level mode outside the enum",
            "rowLevelMode",
            json!("MERGE_ON_READ"),
        ),
        ("an empty effect key", "effectKey", json!("")),
        (
            "an effect key above 512 characters",
            "effectKey",
            json!("k".repeat(513)),
        ),
        (
            "an effect key with a control character",
            "effectKey",
            json!("run\n7f3a"),
        ),
        (
            "found-by-lookup as a string",
            "foundByLookup",
            json!("false"),
        ),
        ("an empty ref", "ref", json!("")),
        ("a schema id that is negative", "schemaId", json!(-3)),
    ];
    for (name, key, value) in edits {
        let mut receipt = sample("iceberg-committed");
        receipt[key] = value;
        let mut decoder = Decoder::new();
        assert_eq!(
            decoder.line(&result_line(&receipt), ms(0)),
            Err(ProtocolFailure::InvalidReceipt),
            "{name}"
        );
    }
    for key in [
        "kind",
        "outcome",
        "effectKey",
        "foundByLookup",
        "mode",
        "ref",
    ] {
        let mut receipt = sample("iceberg-committed");
        receipt.as_object_mut().unwrap().remove(key);
        let mut decoder = Decoder::new();
        assert_eq!(
            decoder.line(&result_line(&receipt), ms(0)),
            Err(ProtocolFailure::InvalidReceipt),
            "missing {key}"
        );
    }
}

#[test]
fn an_object_storage_receipt_holds_hashes_as_64_lowercase_hex_digits() {
    for bad in [
        json!("9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08"),
        json!("9f86d081"),
        json!("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08a"),
        json!("zz86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"),
        json!(5),
    ] {
        for key in ["objectNameSha256", "recordsSha256"] {
            let mut receipt = sample("object-storage-already-present");
            receipt[key] = bad.clone();
            let mut decoder = Decoder::new();
            assert_eq!(
                decoder.line(&result_line(&receipt), ms(0)),
                Err(ProtocolFailure::InvalidReceipt),
                "{key}"
            );
        }
    }
}

#[test]
fn an_iceberg_receipt_without_the_optional_fields_is_valid_and_rebuilt_without_them() {
    let minimal = json!({
        "kind": "iceberg", "outcome": "EMPTY_NO_COMMIT", "effectKey": "k/1", "foundByLookup": false,
        "mode": "append", "ref": "main"
    });
    let mut decoder = Decoder::new();
    let Event::Result(result) = decoder.line(&result_line(&minimal), ms(0)).unwrap() else {
        panic!("not a result")
    };
    let Outcome::Receipt(receipt) = result.outcome else {
        panic!("not a receipt")
    };

    assert_eq!(serde_json::to_value(&receipt).unwrap(), minimal);
    assert!(receipts_schema().is_valid(&minimal));
}

// -- failures ---------------------------------------------------------------------------------------------------------

#[test]
fn a_failure_names_a_code_of_the_closed_set_and_nothing_free() {
    let mut decoder = Decoder::new();
    let failure =
        json!({"code": "ICEBERG_SCHEMA_MISMATCH", "column": "amount", "reason": "SCALE_OVERFLOW"});

    let Event::Result(result) = decoder
        .line(
            &line(&json!({"failure": failure, "relayDenials": 3})),
            ms(0),
        )
        .unwrap()
    else {
        panic!("not a result")
    };

    assert_eq!(result.relay_denials, 3);
    let Outcome::Failure(failure) = result.outcome else {
        panic!("not a failure")
    };
    assert_eq!(failure.code, DatabaseFailure::IcebergSchemaMismatch);
    assert_eq!(failure.column.as_deref(), Some("amount"));
    assert_eq!(failure.reason.as_deref(), Some("SCALE_OVERFLOW"));
}

#[test]
fn a_failure_with_only_a_code_is_valid() {
    let mut decoder = Decoder::new();

    let Event::Result(result) = decoder
        .line(
            &line(&json!({"failure": {"code": "STORAGE_UNAVAILABLE"}, "relayDenials": 0})),
            ms(0),
        )
        .unwrap()
    else {
        panic!("not a result")
    };

    let Outcome::Failure(failure) = result.outcome else {
        panic!("not a failure")
    };
    assert_eq!(failure.code.code(), "STORAGE_UNAVAILABLE");
    assert_eq!((failure.column, failure.reason), (None, None));
}

#[test]
fn a_failure_outside_the_closed_set_or_with_free_text_is_refused() {
    let cases = [
        json!({"code": "NOT_A_CODE"}),
        json!({"code": "iceberg_table_not_found"}),
        json!({"code": ""}),
        json!({"code": 7}),
        json!({}),
        json!({"code": "ICEBERG_TABLE_NOT_FOUND", "message": "boom"}),
        json!({"code": "ICEBERG_TABLE_NOT_FOUND", "column": "a\nb"}),
        json!({"code": "ICEBERG_TABLE_NOT_FOUND", "column": ""}),
        json!({"code": "ICEBERG_TABLE_NOT_FOUND", "column": "c".repeat(129)}),
        json!({"code": "ICEBERG_SCHEMA_MISMATCH", "reason": "scale overflow"}),
        json!({"code": "ICEBERG_SCHEMA_MISMATCH", "reason": "scale_overflow"}),
        json!({"code": "ICEBERG_SCHEMA_MISMATCH", "reason": ""}),
        json!({"code": "ICEBERG_SCHEMA_MISMATCH", "reason": "password=hunter2"}),
        json!({"code": "ICEBERG_SCHEMA_MISMATCH", "reason": "R".repeat(49)}),
        json!({"code": "ICEBERG_SCHEMA_MISMATCH", "column": 3}),
    ];
    for failure in cases {
        let mut decoder = Decoder::new();
        let outcome = decoder.line(
            &line(&json!({"failure": failure, "relayDenials": 0})),
            ms(0),
        );
        assert_eq!(outcome, Err(ProtocolFailure::InvalidFailure), "{failure}");
    }
}

// -- standard error ---------------------------------------------------------------------------------------------------

#[test]
fn standard_error_is_reduced_to_a_closed_class_and_a_line_count() {
    let stderr = b"Traceback (most recent call last):\n  File \"host.py\", line 9, in <module>\nValueError: password=hunter2 for s3://secret-bucket\n";

    let summary = summarize_stderr(stderr);

    assert_eq!(
        summary,
        StderrSummary {
            exception: ExceptionClass::ValueError,
            lines: 3
        }
    );
    let shown = format!("{summary:?}");
    assert!(
        !shown.contains("hunter2") && !shown.contains("secret-bucket"),
        "{shown}"
    );
}

#[test]
fn the_exception_class_is_one_of_a_closed_set() {
    let cases: Vec<(&str, ExceptionClass)> = vec![
        ("MemoryError\n", ExceptionClass::MemoryError),
        (
            "OSError: [Errno 28] No space left on device\n",
            ExceptionClass::OsError,
        ),
        ("ConnectionResetError: x\n", ExceptionClass::ConnectionError),
        ("builtins.TimeoutError: x\n", ExceptionClass::TimeoutError),
        ("KeyError: 'warehouse'\n", ExceptionClass::KeyError),
        ("TypeError: x\n", ExceptionClass::TypeError),
        ("RuntimeError: x\n", ExceptionClass::RuntimeError),
        (
            "pyiceberg.exceptions.CommitFailedException: x\n",
            ExceptionClass::Other,
        ),
        ("hunter2\n", ExceptionClass::None),
        ("Traceback (most recent call last):\n", ExceptionClass::None),
        ("something went wrong here\n", ExceptionClass::None),
        ("password: hunter2 is wrong\n", ExceptionClass::None),
        ("", ExceptionClass::None),
        ("\n\n", ExceptionClass::None),
    ];
    for (stderr, expected) in cases {
        assert_eq!(
            summarize_stderr(stderr.as_bytes()).exception,
            expected,
            "{stderr:?}"
        );
    }
}

#[test]
fn the_line_count_counts_lines_and_saturates() {
    assert_eq!(summarize_stderr(b"").lines, 0);
    assert_eq!(summarize_stderr(b"one").lines, 1);
    assert_eq!(summarize_stderr(b"one\n").lines, 1);
    assert_eq!(summarize_stderr(b"one\ntwo\n").lines, 2);
    assert_eq!(summarize_stderr(b"one\n\nthree").lines, 3);
    assert_eq!(summarize_stderr(&[0xff, b'\n', 0xfe]).lines, 2);
}

// -- the relay token --------------------------------------------------------------------------------------------------

#[test]
fn a_relay_token_is_long_enough_and_has_no_character_that_could_break_the_framing() {
    let long = "a".repeat(129);
    let short = "a".repeat(31);
    for bad in [
        "",
        short.as_str(),
        long.as_str(),
        "relay token with spaces 0123456789abcdef",
        "relay-token-with-a-newline\n0123456789abcdef",
        "relay-token-with-a-return\r0123456789abcdef",
        "relay-token-with-a-nul\u{0}0123456789abcdef",
        "relay-token-with-a-colon:0123456789abcdef",
        "relay-token-with-an-accent-\u{e9}0123456789abcdef",
    ] {
        assert_eq!(
            RelayToken::new(bad).unwrap_err(),
            TokenError::Shape,
            "{bad:?}"
        );
    }
    for good in [
        "a".repeat(32),
        "a".repeat(128),
        "A-b_c-0123456789-0123456789-0123456789".to_owned(),
    ] {
        assert!(RelayToken::new(good).is_ok());
    }
}

#[test]
fn a_generated_token_is_64_hex_digits_valid_and_never_repeats() {
    let first = RelayToken::generate().unwrap();
    let second = RelayToken::generate().unwrap();

    assert_ne!(first, second);
    let encoded = encode_request(&Request::new(
        Operation::Probe,
        "k",
        "2026-11-05T10:00:00Z",
        RelayAccess::new("/work/a/relay.sock", first),
        Map::new(),
    ))
    .unwrap();
    let parsed: Value = serde_json::from_slice(&encoded).unwrap();
    let token = parsed["relay"]["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{token}"
    );
    assert!(RelayToken::new(token).is_ok());
}

//! The attempt input of a lake step, as the lease carries it (feature 035, T035-035, T035-056, D035-121).
//!
//! The reader and the server that will build the document are held to one corpus
//! (`tests/contract/iceberg-writes/fixtures/attempt-input/cases.json`). A document that is not exactly what the contract says
//! is refused whole, by the name of the member, and no error or `Debug` shows a value: the document holds the author's data.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use sagat_infra_lake::protocol::{
    AttemptInput, BlobRef, Connection, InputError, InputFile, InputFormat, Limits, MAX_INPUT_BYTES,
    NodeLimits, Operation, Records, RelayAccess, RelayToken, Request, encode_request,
};
use serde_json::{Value, json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corpus() -> Value {
    let path = root().join("tests/contract/iceberg-writes/fixtures/attempt-input/cases.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn base() -> Value {
    corpus()["base"].clone()
}

fn bytes(document: &Value) -> Vec<u8> {
    serde_json::to_vec(document).unwrap()
}

/// Applies `set` and `remove` edits (JSON pointers) to a copy of `base`.
fn edited(edits: &[Value]) -> Value {
    let mut document = base();
    for edit in edits {
        let (pointer, value) = match (edit.get("set"), edit.get("remove")) {
            (Some(pointer), None) => (pointer.as_str().unwrap(), Some(edit["value"].clone())),
            (None, Some(pointer)) => (pointer.as_str().unwrap(), None),
            _ => panic!("an edit is a set or a remove: {edit}"),
        };
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        let target = if parent.is_empty() {
            &mut document
        } else {
            document.pointer_mut(parent).expect("the parent exists")
        };
        match (target, value) {
            (Value::Object(members), Some(value)) => {
                members.insert(key.to_owned(), value);
            }
            (Value::Object(members), None) => {
                members.remove(key).expect("the member to remove exists");
            }
            _ => panic!("an edit goes into an object: {pointer}"),
        }
    }
    document
}

fn outcome(result: Result<AttemptInput, InputError>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(InputError::Document) => "document".to_owned(),
        Err(InputError::Member(name)) => format!("member:{name}"),
        Err(InputError::Unsupported(name)) => format!("unsupported:{name}"),
    }
}

#[test]
fn the_reader_says_what_the_corpus_says_of_every_document() {
    let all = corpus();
    let cases = all["cases"].as_array().unwrap();
    assert!(
        cases.len() > 80,
        "the corpus is thin: {} cases",
        cases.len()
    );
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let document = match case.get("text") {
            Some(text) => text.as_str().unwrap().as_bytes().to_vec(),
            None => bytes(&edited(
                case.get("edit")
                    .map_or(&[][..], |edits| edits.as_array().unwrap()),
            )),
        };
        assert_eq!(
            outcome(AttemptInput::read(&document)),
            case["expect"].as_str().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn the_document_the_corpus_is_built_on_is_read_into_what_it_says() {
    let input = AttemptInput::read(&bytes(&base())).unwrap();
    assert_eq!(input.effect_key(), "run-7f3a/load/1");
    assert_eq!(input.first_attempt_started_at(), "2026-11-05T10:00:00.000Z");
    assert_eq!(input.table().namespace(), ["sales"]);
    assert_eq!(input.table().name(), "orders");
    assert_eq!(input.reference(), "main");
    assert_eq!(input.mapping().len(), 1);
    assert_eq!(
        input.limits(),
        NodeLimits {
            max_record_bytes: 1_048_576,
            max_records: 1_000_000,
            timeout_ms: 600_000,
        }
    );
    match input.records() {
        Records::Inline(records) => {
            assert_eq!(records, &[json!({"amount": "12.50", "sku": "A-1"})])
        }
        other => panic!("not inline: {other:?}"),
    }
}

#[test]
fn the_records_can_be_taken_out_and_the_rest_of_the_input_is_still_whole() {
    let mut input = AttemptInput::read(&bytes(&base())).unwrap();
    match input.take_records() {
        Records::Inline(records) => assert_eq!(records, [json!({"amount": "12.50", "sku": "A-1"})]),
        other => panic!("not inline: {other:?}"),
    }
    assert!(matches!(input.records(), Records::Inline(left) if left.is_empty()));
    // what the request is made from does not depend on the records
    assert_eq!(input.effect_key(), "run-7f3a/load/1");
    assert_eq!(input.table().name(), "orders");
    assert_eq!(input.mapping().len(), 1);
}

#[test]
fn blobs_are_read_with_their_ids_and_formats_in_the_order_written() {
    let document = edited(&[json!({"set": "/records", "value": {"blobs": [
        {"id": "0192f5aa-0000-7000-8000-000000000002", "format": "parquet"},
        {"id": "0192f5aa-0000-7000-8000-000000000001", "format": "ndjson"}
    ]}})]);
    let input = AttemptInput::read(&bytes(&document)).unwrap();
    let Records::Blobs(blobs) = input.records() else {
        panic!("not blobs");
    };
    let shown: Vec<(String, InputFormat)> = blobs
        .iter()
        .map(|BlobRef { id, format }| (id.to_string(), *format))
        .collect();
    assert_eq!(
        shown,
        [
            (
                "0192f5aa-0000-7000-8000-000000000002".to_owned(),
                InputFormat::Parquet
            ),
            (
                "0192f5aa-0000-7000-8000-000000000001".to_owned(),
                InputFormat::Ndjson
            ),
        ]
    );
}

fn blobs(count: usize) -> Value {
    let list: Vec<Value> = (0..count)
        .map(|index| json!({"id": format!("0192f5aa-0000-7000-8000-{index:012}"), "format": "ndjson"}))
        .collect();
    edited(&[json!({"set": "/records", "value": {"blobs": list}})])
}

#[test]
fn thirty_two_blobs_are_the_most_and_the_next_is_refused() {
    assert!(AttemptInput::read(&bytes(&blobs(32))).is_ok());
    assert_eq!(
        AttemptInput::read(&bytes(&blobs(33))).unwrap_err(),
        InputError::Member("records")
    );
}

#[test]
fn the_longest_effect_key_and_ref_are_taken_and_the_next_byte_is_refused() {
    let with = |pointer: &str, length: usize| {
        AttemptInput::read(&bytes(&edited(&[
            json!({"set": pointer, "value": "k".repeat(length)}),
        ])))
    };
    assert!(with("/effectKey", 512).is_ok());
    assert_eq!(
        with("/effectKey", 513).unwrap_err(),
        InputError::Member("effectKey")
    );
    assert!(with("/ref", 255).is_ok());
    assert_eq!(with("/ref", 256).unwrap_err(), InputError::Member("ref"));
}

#[test]
fn the_longest_document_is_taken_and_one_byte_more_is_not() {
    // one inline record whose value pads the document to exactly the bound
    let padded = |extra: usize| {
        let mut document =
            edited(&[json!({"set": "/records", "value": {"inline": [{"pad": ""}]}})]);
        let size = serde_json::to_vec(&document).unwrap().len();
        let fill = MAX_INPUT_BYTES + extra - size;
        document["records"]["inline"][0]["pad"] = Value::String("x".repeat(fill));
        let text = serde_json::to_vec(&document).unwrap();
        assert_eq!(text.len(), MAX_INPUT_BYTES + extra);
        text
    };
    assert!(AttemptInput::read(&padded(0)).is_ok());
    assert_eq!(
        AttemptInput::read(&padded(1)).unwrap_err(),
        InputError::Document
    );
}

#[test]
fn a_refusal_names_the_member_and_never_shows_a_value() {
    const MARKER: &str = "MARKER-the-authors-data-9f3a";
    // each document is wrong in a member whose value is the marker
    let wrong = [
        edited(&[json!({"set": "/effectKey", "value": format!("{MARKER}\u{7}")})]),
        edited(&[json!({"set": "/firstAttemptStartedAt", "value": MARKER})]),
        edited(&[json!({"set": "/snapshotProperties", "value": {MARKER: "x"}})]),
        edited(&[
            json!({"set": "/table/name", "value": ""}),
            json!({"set": "/table/namespace", "value": [MARKER, ""]}),
        ]),
        edited(&[json!({"set": "/records", "value": {MARKER: []}})]),
        edited(&[json!({"set": "/mapping", "value": [{"field": MARKER, "column": MARKER}]})]),
        edited(&[json!({"set": "/mode", "value": MARKER})]),
        edited(&[json!({"set": "/schema", "value": MARKER})]),
        edited(&[json!({"set": "/operation", "value": MARKER})]),
        edited(&[json!({"set": MARKER_POINTER, "value": MARKER})]),
    ];
    for document in wrong {
        let error = AttemptInput::read(&bytes(&document)).unwrap_err();
        assert!(!format!("{error} {error:?}").contains(MARKER), "{error}");
    }
}

/// A member nobody knows, named by the author's data.
const MARKER_POINTER: &str = "/MARKER-the-authors-data-9f3a";

#[test]
fn what_is_debugged_shows_how_many_records_and_not_the_records() {
    const MARKER: &str = "MARKER-the-authors-data-9f3a";
    let document = edited(&[
        json!({"set": "/records", "value": {"inline": [{"secret": MARKER}, {"secret": MARKER}]}}),
    ]);
    let input = AttemptInput::read(&bytes(&document)).unwrap();
    let shown = format!("{input:?} {:?}", input.records());
    assert!(!shown.contains(MARKER), "{shown}");
    assert!(shown.contains("2 records"), "{shown}");
}

fn secrets(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// The request the host is sent, built from the lease input and the parts the runner owns, is the fixture the host's own
/// tests parse: the two documents of the contract (the lease input and the host's request) cannot drift apart.
#[test]
fn the_request_built_from_the_input_is_the_request_the_host_parses() {
    let request =
        root().join("tests/contract/iceberg-writes/fixtures/requests/iceberg-write-append.json");
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(request).unwrap()).unwrap();

    let input = AttemptInput::read(&bytes(&base())).unwrap();
    let connection = Connection::new(
        fixture["connection"]["catalog"].clone(),
        fixture["connection"]["storage"].clone(),
        vec!["s3://lake/warehouse/".to_owned()],
        secrets(&[
            ("clientId", "client-1"),
            ("clientSecret", "client-secret-canary"),
        ]),
        secrets(&[
            ("accessKeyId", "AKIAEXAMPLE"),
            ("secretAccessKey", "storage-secret-canary"),
        ]),
    );
    let inputs = vec![
        InputFile::new("inline.ndjson", InputFormat::Ndjson).unwrap(),
        InputFile::new("blob-0.parquet", InputFormat::Parquet).unwrap(),
    ];
    let limits = Limits::new(4, 17_179_869_184, 67_108_864, "2026-11-05T10:10:00.000Z").unwrap();
    let body = input
        .write_append(connection, inputs, limits)
        .into_body()
        .unwrap();
    let relay = RelayAccess::new(
        "/work/attempt-1/relay.sock",
        RelayToken::new("relay-token-canary-0123456789abcdef").unwrap(),
    );
    let line = encode_request(&Request::new(
        Operation::IcebergWrite,
        input.effect_key(),
        input.first_attempt_started_at(),
        relay,
        body,
    ))
    .unwrap();
    let built: Value = serde_json::from_slice(&line[..line.len() - 1]).unwrap();
    assert_eq!(built, fixture);
}

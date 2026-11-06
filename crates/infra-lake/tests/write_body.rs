//! The body of an `icebergWrite` request (feature 035, T035-035, D035-109): the part of the request line that the runner
//! builds and the Python host parses. The fixture is the one both read; `runners/lake/tests/test_wire_request.py` parses
//! it with the host's own `parse_write` and `open_connection`, so a member spelled differently on either side fails a
//! test on that side and not in a container.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use sagat_domain::database::{ColumnMapping, MAX_MAPPED_COLUMNS, TableIdentifier};
use sagat_infra_lake::protocol::{
    BodyError, Connection, InputFile, InputFormat, Limits, Operation, RelayAccess, RelayToken,
    Request, WriteAppend, encode_request,
};
use serde_json::{Map, Value, json};

const TOKEN: &str = "relay-token-canary-0123456789abcdef";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture() -> Value {
    let path =
        root().join("tests/contract/iceberg-writes/fixtures/requests/iceberg-write-append.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn secrets(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn connection() -> Connection {
    let stored = fixture();
    Connection::new(
        stored["connection"]["catalog"].clone(),
        stored["connection"]["storage"].clone(),
        vec!["s3://lake/warehouse/".to_owned()],
        secrets(&[
            ("clientId", "client-1"),
            ("clientSecret", "client-secret-canary"),
        ]),
        secrets(&[
            ("accessKeyId", "AKIAEXAMPLE"),
            ("secretAccessKey", "storage-secret-canary"),
        ]),
    )
}

fn inputs() -> Vec<InputFile> {
    vec![
        InputFile::new("inline.ndjson", InputFormat::Ndjson).unwrap(),
        InputFile::new("blob-0.parquet", InputFormat::Parquet).unwrap(),
    ]
}

fn append() -> WriteAppend {
    build("main", connection(), inputs())
}

fn build(reference: &str, connection: Connection, inputs: Vec<InputFile>) -> WriteAppend {
    WriteAppend::new(
        TableIdentifier::new(vec!["sales".to_owned()], "orders").unwrap(),
        reference,
        connection,
        inputs,
        Limits::new(4, 17_179_869_184, 67_108_864, "2026-11-05T10:10:00.000Z").unwrap(),
    )
    .snapshot_property("sagat.effect-key", "run-7f3a/load/1")
    .snapshot_property("sagat.run-id", "run-7f3a")
    .snapshot_property("sagat.step-id", "load")
    .snapshot_property("sagat.workflow", "orders-sync")
    .snapshot_property("sagat.workflow-version", "12")
    .map_column(ColumnMapping::new("/amount", "amount").unwrap())
    .ignore_unknown_fields(true)
}

fn line(write: WriteAppend) -> Value {
    let relay = RelayAccess::new(
        "/work/attempt-1/relay.sock",
        RelayToken::new(TOKEN).unwrap(),
    );
    let request = Request::new(
        Operation::IcebergWrite,
        "run-7f3a/load/1",
        "2026-11-05T10:00:00.000Z",
        relay,
        write.into_body().unwrap(),
    );
    let bytes = encode_request(&request).unwrap();
    assert_eq!(bytes.last(), Some(&b'\n'));
    serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap()
}

#[test]
fn the_request_the_runner_builds_is_the_fixture_the_host_parses() {
    assert_eq!(line(append()), fixture());
}

#[test]
fn a_secret_document_that_is_empty_is_not_sent() {
    let stored = fixture();
    let bare = Connection::new(
        stored["connection"]["catalog"].clone(),
        stored["connection"]["storage"].clone(),
        vec!["s3://lake/warehouse/".to_owned()],
        BTreeMap::new(),
        BTreeMap::new(),
    );
    let write = build("main", bare, inputs());
    assert_eq!(line(write)["connection"]["credentials"], json!({}));
}

#[test]
fn the_flags_and_the_prior_attempt_reach_the_line() {
    let line = line(
        append()
            .ignore_unknown_fields(false)
            .allow_empty(true)
            .prior_attempt_may_have_left_files(true),
    );
    assert_eq!(line["ignoreUnknownFields"], json!(false));
    assert_eq!(line["allowEmpty"], json!(true));
    assert_eq!(line["priorAttemptMayHaveLeftFiles"], json!(true));
}

#[test]
fn the_sections_of_the_modes_not_served_are_null() {
    let line = line(append());
    assert_eq!(line["mode"], json!("append"));
    assert_eq!(line["overwrite"], Value::Null);
    assert_eq!(line["upsert"], Value::Null);
}

#[test]
fn the_limits_cross_as_the_strings_the_host_reads() {
    let limits = &line(append())["limits"];
    assert_eq!(limits["maxCommitRetries"], json!(4));
    assert_eq!(limits["maxDecodedBytes"], json!("17179869184"));
    assert_eq!(limits["maxMetadataBytes"], json!("67108864"));
}

#[test]
fn no_secret_is_shown_by_debug() {
    let shown = format!("{:?} {:?}", append(), connection());
    for secret in [
        "client-secret-canary",
        "storage-secret-canary",
        "AKIAEXAMPLE",
    ] {
        assert!(!shown.contains(secret), "{secret} was shown");
    }
}

#[test]
fn a_body_cannot_name_the_envelope() {
    let mut body = append().into_body().unwrap();
    assert!(!body.contains_key("protocol"));
    assert!(!body.contains_key("relay"));
    assert!(!body.contains_key("effectKey"));
    body.insert("relay".to_owned(), Value::Null);
    let request = Request::new(
        Operation::IcebergWrite,
        "k",
        "2026-11-05T10:00:00.000Z",
        RelayAccess::new("/x", RelayToken::new(TOKEN).unwrap()),
        body,
    );
    assert!(encode_request(&request).is_err());
}

#[test]
fn a_file_name_that_could_leave_the_input_directory_is_refused() {
    let long = "a".repeat(129);
    for name in [
        "", ".hidden", "../x", "a/b", "in/a", "a b", "a\u{0}b", "é", &long,
    ] {
        assert_eq!(
            InputFile::new(name, InputFormat::Ndjson).unwrap_err(),
            BodyError::InputName,
            "{name:?}"
        );
    }
    let longest = "a".repeat(128);
    for name in ["a", "blob-0.parquet", "inline.ndjson", "A_b-1.x", &longest] {
        assert!(
            InputFile::new(name, InputFormat::Ndjson).is_ok(),
            "{name:?}"
        );
    }
}

#[test]
fn limits_outside_what_the_host_accepts_are_refused() {
    let at = "2026-11-05T10:10:00.000Z";
    assert!(Limits::new(16, 1, 1, at).is_ok());
    assert_eq!(Limits::new(17, 1, 1, at).unwrap_err(), BodyError::Limits);
    assert!(Limits::new(0, i64::MAX as u64, i64::MAX as u64, at).is_ok());
    assert_eq!(
        Limits::new(0, i64::MAX as u64 + 1, 1, at).unwrap_err(),
        BodyError::Limits
    );
    assert_eq!(
        Limits::new(0, 1, i64::MAX as u64 + 1, at).unwrap_err(),
        BodyError::Limits
    );
}

#[test]
fn a_deadline_that_is_not_an_instant_in_utc_is_refused() {
    for deadline in [
        "",
        "tomorrow",
        "2026-11-05T10:10:00Z ",
        "2026-11-05 10:10:00Z",
        "2026-11-05T10:10:00+00:00",
        "2026-11-05T10:10:00.1234567Z",
        "2026-11-05T10:10:00.Z",
        "2026-11-05T10:10Z",
        "2026-11-05T10:10:00\nZ",
    ] {
        assert_eq!(
            Limits::new(0, 1, 1, deadline).unwrap_err(),
            BodyError::Limits,
            "{deadline:?}"
        );
    }
    for deadline in [
        "2026-11-05T10:10:00Z",
        "2026-11-05T10:10:00.1Z",
        "2026-11-05T10:10:00.123456Z",
    ] {
        assert!(Limits::new(0, 1, 1, deadline).is_ok(), "{deadline:?}");
    }
}

#[test]
fn a_ref_the_domain_would_refuse_is_refused_when_the_body_is_built() {
    let long = "r".repeat(256);
    for reference in ["", "a\nb", &long] {
        let write = build(reference, connection(), inputs());
        assert_eq!(write.into_body().unwrap_err(), BodyError::Reference);
    }
    let longest = "r".repeat(255);
    assert!(build(&longest, connection(), inputs()).into_body().is_ok());
}

#[test]
fn inputs_that_the_host_would_refuse_are_refused_when_the_body_is_built() {
    let none = build("main", connection(), vec![]);
    assert_eq!(none.into_body().unwrap_err(), BodyError::Inputs);

    let files = |count: usize| -> Vec<InputFile> {
        (0..count)
            .map(|index| InputFile::new(&format!("f{index}"), InputFormat::Ndjson).unwrap())
            .collect()
    };
    let too_many = build("main", connection(), files(33));
    assert_eq!(too_many.into_body().unwrap_err(), BodyError::Inputs);
    assert!(build("main", connection(), files(32)).into_body().is_ok());
}

#[test]
fn a_connection_the_host_would_refuse_is_refused_when_the_body_is_built() {
    let stored = fixture();
    let catalog = || stored["connection"]["catalog"].clone();
    let storage = || stored["connection"]["storage"].clone();
    let with = |locations: Vec<String>, secret: &str| {
        Connection::new(
            catalog(),
            storage(),
            locations,
            secrets(&[("token", secret)]),
            BTreeMap::new(),
        )
    };
    for bad in [
        with(vec![], "ok"),
        with(vec!["s3://x/".to_owned(); 17], "ok"),
        with(vec![String::new()], "ok"),
        with(vec!["s".repeat(2049)], "ok"),
        with(vec!["s3://x/".to_owned()], "line\nbreak"),
        with(vec!["s3://x/".to_owned()], "del\u{7f}"),
        with(vec!["s3://x/".to_owned()], &"s".repeat(16_385)),
    ] {
        let write = build("main", bad, inputs());
        assert_eq!(write.into_body().unwrap_err(), BodyError::Connection);
    }
    for good in [
        with(vec!["s3://x/".to_owned(); 16], "ok"),
        with(vec!["s".repeat(2048)], "ok"),
        with(vec!["s3://x/".to_owned()], &"s".repeat(16_384)),
    ] {
        let write = build("main", good, inputs());
        assert!(write.into_body().is_ok());
    }
}

#[test]
fn a_snapshot_property_the_host_does_not_know_is_refused_when_the_body_is_built() {
    let write = append().snapshot_property("added-records", "1");
    assert_eq!(write.into_body().unwrap_err(), BodyError::SnapshotProperty);
}

#[test]
fn a_mapping_beyond_the_ceiling_of_the_domain_is_refused_when_the_body_is_built() {
    // `append()` already maps `/amount`, so it takes one less than the ceiling to reach it
    let mapped = |total: usize| {
        (0..total - 1).fold(append(), |write, index| {
            write.map_column(
                ColumnMapping::new(&format!("/f{index}"), &format!("c{index}")).unwrap(),
            )
        })
    };
    assert!(mapped(MAX_MAPPED_COLUMNS).into_body().is_ok());
    assert_eq!(
        mapped(MAX_MAPPED_COLUMNS + 1).into_body().unwrap_err(),
        BodyError::Mapping
    );
}

#[test]
fn the_body_is_a_map_with_no_member_of_the_envelope() {
    let body: Map<String, Value> = append().into_body().unwrap();
    for member in [
        "protocol",
        "operation",
        "effectKey",
        "firstAttemptStartedAt",
        "relay",
    ] {
        assert!(!body.contains_key(member), "{member}");
    }
}

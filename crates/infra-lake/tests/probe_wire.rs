//! The probe of an Iceberg connection on the lake runner's wire (feature 035, D035-150, slice S6a).
//!
//! Three things are held here to the corpus that the server and the Python host are held to, so that no end can drift:
//!
//! - **the lease**: `sagat.lake.probe-input.v1`, read strictly (`fixtures/probe-input/probe.json` is what the server's builder
//!   writes byte for byte; `refused.json` is every way a document is not that);
//! - **the request line** the host reads (`fixtures/requests/probe.json`, which the Python host's `parse_probe` reads too):
//!   the lease's members nested in `probe`, next to the pinned connection, and neither an effect key nor an instant, since a
//!   probe writes nothing and the host refuses a line that has them;
//! - **the result line**: `{"receipt": {"kind":"probe", ...}}`, read into typed fields by the rules of the server's
//!   `ProbeResult::new` and written again as the server writes it (`fixtures/probe-results/{accepted,refused}`).
//!
//! And the table of how a probe ends (`dispose_probe`): the deadline and a cancellation are `DATABASE_TIMEOUT` and
//! `DATABASE_CANCELLED` whether or not the request was sent (a probe writes nothing, so nothing is unknown), and a memory kill
//! is not `ICEBERG_LIMIT_EXCEEDED` (no write failed to fit).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use sagat_domain::database::DatabaseFailure;
use sagat_infra_lake::control::ControlFailure;
use sagat_infra_lake::protocol::{
    Body, Connection, Decoder, EncodeError, Event, HostResult, InputError, Operation, Outcome,
    ProbeBody, ProbeInput, ProbeReport, ProtocolFailure, RelayAccess, RelayToken, Request,
    encode_request,
};
use sagat_infra_lake::supervisor::{
    AttemptFacts, Disposition, Ended, ProbeDisposition, dispose, dispose_probe,
};
use serde_json::{Value, json};

const FIXTURES: &str = "../../tests/contract/iceberg-writes/fixtures";

fn path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURES)
        .join(relative)
}

fn text(relative: &str) -> String {
    std::fs::read_to_string(path(relative)).unwrap()
}

fn json_of(relative: &str) -> Value {
    serde_json::from_str(&text(relative)).unwrap()
}

fn lease() -> Vec<u8> {
    std::fs::read(path("probe-input/probe.json")).unwrap()
}

fn input() -> ProbeInput {
    ProbeInput::read(&lease()).unwrap()
}

/// The files of a corpus directory, by name, in order.
fn directory(relative: &str) -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(path(relative))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(entry.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

// ----------------------------------------------------------------------------------------------------------- the lease
#[test]
fn the_lease_of_the_corpus_is_read() {
    let input = input();
    assert_eq!(input.connection(), "lake-main");
    assert_eq!(input.revision(), 3);
    assert_eq!(input.check_timeout_ms(), 7000);
}

#[test]
fn every_document_of_the_refused_corpus_is_refused_and_no_error_repeats_what_it_said() {
    let refused: BTreeMap<String, String> =
        serde_json::from_str(&text("probe-input/refused.json")).unwrap();
    assert!(
        refused.len() >= 40,
        "the corpus is the contract: {}",
        refused.len()
    );
    for (name, document) in &refused {
        let error = ProbeInput::read(document.as_bytes())
            .expect_err(&format!("`{name}` is not a lease of a probe"));
        let said = error.to_string();
        for value in ["lake-main", "other-lake", "x", "https://catalog.example"] {
            // an error names a part of the contract, never a value of the document
            assert!(!said.contains(&format!("\"{value}\"")), "{name}: {said}");
        }
        assert!(
            matches!(error, InputError::Document | InputError::Member(_)),
            "{name}: {error:?}"
        );
    }
}

#[test]
fn a_document_that_is_not_json_or_has_a_name_twice_is_the_document_that_is_wrong() {
    for document in [
        "not json",
        "[]",
        r#"{"schema":"sagat.lake.probe-input.v1","schema":"sagat.lake.probe-input.v1"}"#,
    ] {
        assert_eq!(
            ProbeInput::read(document.as_bytes()),
            Err(InputError::Document),
            "{document}"
        );
    }
}

#[test]
fn a_member_that_is_wrong_is_named() {
    let mut lease = json_of("probe-input/probe.json");
    lease["checkTimeoutMs"] = json!(99);
    assert_eq!(
        ProbeInput::read(&serde_json::to_vec(&lease).unwrap()),
        Err(InputError::Member("checkTimeoutMs"))
    );
}

#[test]
fn the_bounds_of_the_lease_are_served_to_the_edge() {
    let build = |name: &str, revision: u64, timeout: u64| {
        let mut lease = json_of("probe-input/probe.json");
        lease["connection"] = json!({"name": name, "revision": revision});
        lease["checkTimeoutMs"] = json!(timeout);
        serde_json::to_vec(&lease).unwrap()
    };
    let longest = "a".repeat(63);
    for (name, revision, timeout) in [
        ("abc", 1, 100),
        (longest.as_str(), u64::from(u32::MAX), 60_000),
        ("a1-b2-c3", 2, 7000),
    ] {
        let read = ProbeInput::read(&build(name, revision, timeout)).unwrap();
        assert_eq!(read.connection(), name);
        assert_eq!(u64::from(read.revision()), revision);
        assert_eq!(u64::from(read.check_timeout_ms()), timeout);
    }
}

#[test]
fn a_lease_larger_than_any_lease_is_refused_before_it_is_parsed() {
    let mut lease = json_of("probe-input/probe.json");
    lease["padding"] = json!("x".repeat(8192));
    assert_eq!(
        ProbeInput::read(&serde_json::to_vec(&lease).unwrap()),
        Err(InputError::Document)
    );
}

#[test]
fn a_lease_that_names_the_schema_of_the_writes_is_not_a_probe_lease() {
    // the reader of the writes refuses the probe's operation by name as unsupported; this one refuses the write's schema
    let mut lease = json_of("probe-input/probe.json");
    lease["schema"] = json!("sagat.lake.attempt-input.v1");
    assert_eq!(
        ProbeInput::read(&serde_json::to_vec(&lease).unwrap()),
        Err(InputError::Member("schema"))
    );
}

// ------------------------------------------------------------------------------------------------- the request line
fn secrets(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn connection() -> Connection {
    let stored = json_of("requests/probe.json");
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

fn relay() -> RelayAccess {
    RelayAccess::new(
        "/work/attempt-1/relay.sock",
        RelayToken::new("relay-token-canary-0123456789abcdef").unwrap(),
    )
}

fn body() -> Body {
    ProbeBody::new(connection(), &input()).into_body().unwrap()
}

#[test]
fn the_request_line_of_a_probe_is_the_one_the_host_is_held_to() {
    let line = encode_request(&Request::probe(relay(), body())).unwrap();

    assert_eq!(line.last(), Some(&b'\n'));
    let sent: Value = serde_json::from_slice(&line[..line.len() - 1]).unwrap();
    assert_eq!(sent, json_of("requests/probe.json"));
}

#[test]
fn a_probe_line_names_neither_an_effect_key_nor_an_instant() {
    let line = encode_request(&Request::probe(relay(), body())).unwrap();
    let sent: Value = serde_json::from_slice(&line[..line.len() - 1]).unwrap();
    let members: Vec<&str> = sent
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    // the host reads exactly these and refuses any other member
    for member in [
        "effectKey",
        "firstAttemptStartedAt",
        "snapshotProperties",
        "table",
    ] {
        assert!(!members.contains(&member), "{member}");
    }
    let mut sorted = members.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        ["connection", "operation", "probe", "protocol", "relay"]
    );
}

#[test]
fn a_probe_that_is_given_an_effect_key_is_not_a_line_the_host_reads() {
    let request = Request::new(
        Operation::Probe,
        "run-7f3a/probe/1",
        "2026-11-05T10:00:00.000Z",
        relay(),
        body(),
    );
    assert_eq!(encode_request(&request).unwrap_err(), EncodeError::Envelope);
}

#[test]
fn the_constructor_of_a_probe_fixes_the_operation() {
    // whatever the body, the line says `probe`
    let line = encode_request(&Request::probe(relay(), body())).unwrap();
    let sent: Value = serde_json::from_slice(&line[..line.len() - 1]).unwrap();
    assert_eq!(sent["operation"], "probe");
}

#[test]
fn the_body_of_a_probe_cannot_name_a_member_of_the_envelope() {
    let mut members = serde_json::Map::new();
    members.insert("relay".to_owned(), json!({"socket": "/x", "token": "y"}));
    assert_eq!(
        encode_request(&Request::probe(relay(), Body::from(members))).unwrap_err(),
        EncodeError::ReservedMember
    );
}

#[test]
fn nothing_of_a_probe_request_is_shown() {
    let shown = format!(
        "{:?} {:?}",
        ProbeBody::new(connection(), &input()),
        Request::probe(relay(), body())
    );
    for secret in [
        "client-secret-canary",
        "storage-secret-canary",
        "relay-token-canary-0123456789abcdef",
        "AKIAEXAMPLE",
    ] {
        assert!(!shown.contains(secret), "{secret}");
    }
}

#[test]
fn a_connection_the_host_would_refuse_is_refused_here() {
    let bad = Connection::new(
        json!({"type": "rest"}),
        json!({"type": "s3"}),
        vec![],
        secrets(&[]),
        secrets(&[]),
    );
    assert!(ProbeBody::new(bad, &input()).into_body().is_err());
}

// -------------------------------------------------------------------------------------------------- the result line
fn line(receipt: &str) -> Vec<u8> {
    format!(r#"{{"receipt":{receipt},"relayDenials":0}}"#).into_bytes()
}

fn decode(line: &[u8]) -> Result<HostResult, ProtocolFailure> {
    match Decoder::new().line(line, Duration::ZERO)? {
        Event::Result(result) => Ok(result),
        Event::Heartbeat => panic!("a heartbeat is not a result"),
    }
}

fn report_of(receipt: &str) -> ProbeReport {
    match decode(&line(receipt)).unwrap().outcome {
        Outcome::Probe(report) => report,
        other => panic!("not a probe report: {other:?}"),
    }
}

#[test]
fn every_accepted_result_of_the_corpus_is_read_and_written_as_the_server_writes_it() {
    let files = directory("probe-results/accepted");
    assert!(files.len() >= 9, "{}", files.len());
    for (name, receipt) in files {
        if name == "reordered.canonical.json" {
            continue;
        }
        let report = report_of(receipt.trim_end());
        // the canonical bytes are the ones the server stores: the file itself, or its golden when the host reordered it
        let expected = if name == "reordered.json" {
            text("probe-results/accepted/reordered.canonical.json")
        } else {
            receipt.clone()
        };
        assert_eq!(
            String::from_utf8(report.output_json()).unwrap(),
            expected.trim_end(),
            "{name}"
        );
        assert!(report.answers(&input()), "{name}");
    }
}

#[test]
fn every_refused_result_of_the_corpus_is_refused_by_the_runner_too() {
    let files = directory("probe-results/refused");
    assert!(files.len() >= 25, "{}", files.len());
    let lease = input();
    for (name, receipt) in files {
        let decoded = decode(&line(receipt.trim_end()));
        if name.starts_with("mismatch-") {
            // well formed, and the result of another probe: the runner does not report it as this one's
            let Outcome::Probe(report) = decoded.unwrap().outcome else {
                panic!("{name}: not a probe report");
            };
            assert!(!report.answers(&lease), "{name}");
        } else if name == "malformed-not-json.json" {
            assert_eq!(decoded.unwrap_err(), ProtocolFailure::Malformed, "{name}");
        } else {
            assert_eq!(
                decoded.unwrap_err(),
                ProtocolFailure::InvalidReceipt,
                "{name}"
            );
        }
    }
}

#[test]
fn a_report_answers_the_probe_it_names_by_name_and_by_revision() {
    let report = report_of(&text("probe-results/accepted/all-passed.json"));
    assert!(report.answers(&input()));
    assert_eq!(report.connection(), "lake-main");
    assert_eq!(report.revision(), 3);

    let mut other_revision = json_of("probe-input/probe.json");
    other_revision["connection"]["revision"] = json!(4);
    let other_revision = ProbeInput::read(&serde_json::to_vec(&other_revision).unwrap()).unwrap();
    assert!(!report.answers(&other_revision));

    let mut other_name = json_of("probe-input/probe.json");
    other_name["connection"]["name"] = json!("other-lake");
    let other_name = ProbeInput::read(&serde_json::to_vec(&other_name).unwrap()).unwrap();
    assert!(!report.answers(&other_name));
}

#[test]
fn a_result_is_read_into_typed_fields_and_whatever_else_it_said_is_not_kept() {
    // a member nobody named is a break of the protocol, not something the runner drops and forwards the rest of
    let mut result = json_of("probe-results/accepted/all-passed.json");
    result["note"] = json!("the catalog said: password=hunter2");
    let refused = decode(&line(&serde_json::to_string(&result).unwrap())).unwrap_err();
    assert_eq!(refused, ProtocolFailure::InvalidReceipt);
}

#[test]
fn a_result_over_the_bound_the_server_would_refuse_is_refused_here_first() {
    let mut result = json_of("probe-results/accepted/all-passed.json");
    result["connection"]["name"] = json!("a".repeat(5000));
    let line = line(&serde_json::to_string(&result).unwrap());
    assert!(
        line.len() < 16 * 1024,
        "the line itself is within the bound of a line"
    );
    assert_eq!(decode(&line).unwrap_err(), ProtocolFailure::InvalidReceipt);
}

#[test]
fn a_write_receipt_still_decodes_as_one() {
    let result = decode(&line(&text("receipts/iceberg-committed.json"))).unwrap();
    assert!(matches!(result.outcome, Outcome::Receipt(_)));
}

// ---------------------------------------------------------------------------------------- how a probe ends
const SENT: AttemptFacts = AttemptFacts {
    memory_killed: false,
    request_sent: true,
};
const NOT_SENT: AttemptFacts = AttemptFacts {
    memory_killed: false,
    request_sent: false,
};
const KILLED: AttemptFacts = AttemptFacts {
    memory_killed: true,
    request_sent: true,
};

fn probe_result() -> HostResult {
    decode(&line(&text("probe-results/accepted/all-passed.json"))).unwrap()
}

fn failure_result(code: &str) -> HostResult {
    decode(format!(r#"{{"failure":{{"code":"{code}"}},"relayDenials":0}}"#).as_bytes()).unwrap()
}

fn failed_with(disposition: ProbeDisposition) -> DatabaseFailure {
    match disposition {
        ProbeDisposition::Fail(failure) => failure.code,
        other => panic!("not a failure: {other:?}"),
    }
}

#[test]
fn the_result_of_a_probe_stands_even_if_the_container_was_killed_after_it() {
    for facts in [SENT, KILLED] {
        let disposition = dispose_probe(Ended::Conversation(Ok(probe_result())), facts);
        let ProbeDisposition::Complete(report) = disposition else {
            panic!("not reported");
        };
        assert!(report.answers(&input()));
    }
}

#[test]
fn what_the_host_says_it_could_not_do_is_reported_with_its_code() {
    let ended = Ended::Conversation(Ok(failure_result("DATABASE_CREDENTIAL_UNAVAILABLE")));
    assert_eq!(
        failed_with(dispose_probe(ended, SENT)),
        DatabaseFailure::Credential
    );
}

#[test]
fn a_write_receipt_from_a_probe_is_a_break_of_the_protocol() {
    let receipt = decode(&line(&text("receipts/iceberg-committed.json"))).unwrap();
    assert_eq!(
        failed_with(dispose_probe(Ended::Conversation(Ok(receipt)), SENT)),
        DatabaseFailure::LakeHostProtocol
    );
}

#[test]
fn a_probe_result_from_a_write_is_a_break_of_the_protocol() {
    let Disposition::Fail(failure) = dispose(Ended::Conversation(Ok(probe_result())), SENT) else {
        panic!("a write reported a probe");
    };
    assert_eq!(failure.code, DatabaseFailure::LakeHostProtocol);
}

#[test]
fn a_memory_kill_of_a_probe_is_not_a_write_that_does_not_fit() {
    for ending in [
        ControlFailure::Silent,
        ControlFailure::Protocol(ProtocolFailure::NoResult),
    ] {
        assert_eq!(
            failed_with(dispose_probe(Ended::Conversation(Err(ending)), KILLED)),
            DatabaseFailure::WorkerError
        );
    }
}

#[test]
fn a_host_that_breaks_the_protocol_or_never_connects_is_a_protocol_failure() {
    assert_eq!(
        failed_with(dispose_probe(
            Ended::Conversation(Err(ControlFailure::Protocol(ProtocolFailure::Malformed))),
            SENT
        )),
        DatabaseFailure::LakeHostProtocol
    );
    assert_eq!(
        failed_with(dispose_probe(Ended::HostNeverConnected, NOT_SENT)),
        DatabaseFailure::LakeHostProtocol
    );
    assert_eq!(
        failed_with(dispose_probe(
            Ended::Conversation(Err(ControlFailure::Silent)),
            SENT
        )),
        DatabaseFailure::LakeHostProtocol
    );
}

#[test]
fn a_sandbox_that_did_not_start_is_unavailable() {
    assert_eq!(
        failed_with(dispose_probe(Ended::SandboxUnavailable, NOT_SENT)),
        DatabaseFailure::LakeSandboxUnavailable
    );
}

#[test]
fn the_deadline_and_a_cancellation_are_what_they_are_whether_or_not_the_request_was_sent() {
    // a write that was sent may have committed, so its end is unknown; a probe writes nothing, so nothing is
    for facts in [SENT, NOT_SENT, KILLED] {
        assert_eq!(
            failed_with(dispose_probe(Ended::DeadlinePassed, facts)),
            DatabaseFailure::Timeout
        );
        assert_eq!(
            failed_with(dispose_probe(Ended::Cancelled, facts)),
            DatabaseFailure::Cancelled
        );
    }
}

/// What a host that probes may report as a failure of the attempt: what the probe host raises (`probe.py`, `request.py`,
/// `connection.py`) and nothing else. A failed check is a result and never one of these.
const A_PROBE_MAY_FAIL_WITH: [DatabaseFailure; 4] = [
    DatabaseFailure::Credential,
    DatabaseFailure::IcebergLocationNotAllowed,
    DatabaseFailure::LakeHostProtocol,
    DatabaseFailure::WorkerError,
];

#[test]
fn a_probe_is_failed_only_with_a_code_a_probe_can_raise_and_any_other_is_a_protocol_failure() {
    // the host holds the secrets and reads a hostile catalog's answers: a code of a write (a commit conflict, an unknown
    // outcome, a schema mismatch) is not one a probe has, and is not repeated as if it were
    let mut passed_through = Vec::new();
    for code in DatabaseFailure::ALL {
        let line = format!(
            r#"{{"failure":{{"code":"{}"}},"relayDenials":0}}"#,
            code.code()
        );
        // the runner reads only the codes a host may report; the others never reach the table
        let Ok(Event::Result(result)) = Decoder::new().line(line.as_bytes(), Duration::ZERO) else {
            continue;
        };
        let disposition = dispose_probe(Ended::Conversation(Ok(result)), SENT);
        let reported = failed_with(disposition);
        if A_PROBE_MAY_FAIL_WITH.contains(code) {
            assert_eq!(reported, *code, "{code:?}");
            passed_through.push(*code);
        } else {
            assert_eq!(reported, DatabaseFailure::LakeHostProtocol, "{code:?}");
        }
    }
    assert_eq!(passed_through.len(), A_PROBE_MAY_FAIL_WITH.len());
}

#[test]
fn the_host_may_be_given_less_time_for_a_check_than_the_lease_names() {
    // the attempt may have less time than three checks of the lease (a connection that allows less, or a resolve that was
    // slow): the host is told the time that fits, so that its answer is a typed timeout and not an attempt that ends first
    let body = ProbeBody::new(connection(), &input())
        .with_check_timeout_ms(2500)
        .into_body()
        .unwrap();
    let line = encode_request(&Request::probe(relay(), body)).unwrap();
    let sent: Value = serde_json::from_slice(&line[..line.len() - 1]).unwrap();

    assert_eq!(sent["probe"]["checkTimeoutMs"], 2500);
    assert_eq!(
        sent["probe"]["connection"],
        json!({"name": "lake-main", "revision": 3})
    );
}

#[test]
fn a_time_for_a_check_that_the_host_would_refuse_is_refused_here() {
    for milliseconds in [0, 99, 60_001, u32::MAX] {
        let body = ProbeBody::new(connection(), &input()).with_check_timeout_ms(milliseconds);
        assert!(body.into_body().is_err(), "{milliseconds}");
    }
    for milliseconds in [100, 60_000] {
        let body = ProbeBody::new(connection(), &input()).with_check_timeout_ms(milliseconds);
        assert!(body.into_body().is_ok(), "{milliseconds}");
    }
}

#[test]
fn a_lost_lease_reports_nothing() {
    assert!(matches!(
        dispose_probe(Ended::LeaseLost, SENT),
        ProbeDisposition::Abandon
    ));
}

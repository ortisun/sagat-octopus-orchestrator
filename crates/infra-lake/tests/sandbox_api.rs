//! The wire of the lake sandbox API (feature 035, ADR-0079 rules 3 to 5 and 14, G035-055).
//!
//! The request the server accepts and the answers the client reads are held to one corpus
//! (`tests/contract/iceberg-writes/fixtures/sandbox-api/cases.json`) that the sandbox runner's server reads too, so a client
//! and a server that disagree fail one test. The rule they are held to: **an answer that is not recognised exactly is never
//! read as started or as ended**, because a container with the credentials in it may still be up.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use sagat_infra_lake::sandbox_api::{
    Class, FIRST_UID, KillOutcome, MAX_BODY_BYTES, MAX_SLOTS, Plain, Reason, StartAnswer,
    StartBody, StartOutcome, WaitOutcome, WaitState, attempt_id_ok, kill_body, plain_body,
    read_kill, read_start, read_wait, start_body, uid_for_slot, wait_body,
};
use serde_json::Value;

fn corpus() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/sandbox-api/cases.json");
    serde_json::from_str(&std::fs::read_to_string(&path).expect("the corpus is there"))
        .expect("the corpus is JSON")
}

fn cases(name: &str) -> Vec<Value> {
    let all = corpus();
    let found = all[name].as_array().expect("a list of cases").clone();
    assert!(!found.is_empty(), "{name} has no cases");
    found
}

fn text<'a>(case: &'a Value, key: &str) -> &'a str {
    case[key].as_str().expect("a text member")
}

fn start_outcome(outcome: StartOutcome) -> String {
    match outcome {
        StartOutcome::Started => "started".to_owned(),
        StartOutcome::Refused(reason) => format!("refused:{}", reason.as_str()),
        StartOutcome::Unavailable => "unavailable".to_owned(),
    }
}

/// What a server does with the body of a start request: a body it cannot read is rejected (400), and one it can read but
/// must refuse is refused with the reason.
fn server_view(body: &str) -> String {
    match StartBody::parse(body.as_bytes()) {
        Err(_) => "rejected".to_owned(),
        Ok(parsed) => match parsed.validate() {
            Ok(()) => "ok".to_owned(),
            Err(reason) => format!("refused:{}", reason.as_str()),
        },
    }
}

#[test]
fn the_server_reads_the_requests_of_the_corpus_the_way_it_says() {
    for case in cases("start_requests") {
        assert_eq!(
            server_view(text(&case, "body")),
            text(&case, "expect"),
            "{}",
            text(&case, "name")
        );
    }
}

#[test]
fn the_client_reads_the_start_answers_of_the_corpus_the_way_it_says() {
    for case in cases("start_answers") {
        let status = u16::try_from(case["status"].as_u64().expect("a status")).expect("a status");
        assert_eq!(
            start_outcome(read_start(status, text(&case, "body").as_bytes())),
            text(&case, "expect"),
            "{}",
            text(&case, "name")
        );
    }
}

#[test]
fn the_client_reads_the_wait_answers_of_the_corpus_the_way_it_says() {
    for case in cases("wait_answers") {
        let status = u16::try_from(case["status"].as_u64().expect("a status")).expect("a status");
        let got = match read_wait(status, text(&case, "body").as_bytes()) {
            WaitOutcome::Running => "running",
            WaitOutcome::Ended => "ended",
            WaitOutcome::Unavailable => "unavailable",
        };
        assert_eq!(got, text(&case, "expect"), "{}", text(&case, "name"));
    }
}

#[test]
fn the_client_reads_the_kill_answers_of_the_corpus_the_way_it_says() {
    for case in cases("kill_answers") {
        let status = u16::try_from(case["status"].as_u64().expect("a status")).expect("a status");
        let KillOutcome {
            ended,
            memory_killed,
        } = read_kill(status, text(&case, "body").as_bytes());
        assert_eq!(
            format!("ended:{ended},memory:{memory_killed}"),
            text(&case, "expect"),
            "{}",
            text(&case, "name")
        );
    }
}

/// What the server writes is what the client reads: every answer, through the encoder and back.
#[test]
fn every_answer_the_server_writes_is_read_back_by_the_client_as_it_was() {
    for reason in [
        Reason::Ceiling,
        Reason::Slot,
        Reason::Class,
        Reason::Deadline,
        Reason::Directory,
    ] {
        let (status, body) = start_body(StartAnswer::Refused(reason));
        assert_eq!(
            read_start(status, &body),
            StartOutcome::Refused(reason),
            "{reason:?}"
        );
    }
    let (status, body) = start_body(StartAnswer::Started);
    assert_eq!(read_start(status, &body), StartOutcome::Started);
    let (status, body) = start_body(StartAnswer::Unavailable);
    assert_eq!(read_start(status, &body), StartOutcome::Unavailable);

    for (state, expected) in [
        (WaitState::Running, WaitOutcome::Running),
        (WaitState::Ended, WaitOutcome::Ended),
    ] {
        let (status, body) = wait_body(state);
        assert_eq!(read_wait(status, &body), expected, "{state:?}");
    }

    for (ended, memory_killed) in [(true, true), (true, false), (false, true), (false, false)] {
        let (status, body) = kill_body(ended, memory_killed);
        assert_eq!(
            read_kill(status, &body),
            KillOutcome {
                ended,
                memory_killed
            },
            "ended {ended}, memory {memory_killed}"
        );
    }
}

/// Every refusal of the server that is not an answer to the question (a body it cannot read, a route it does not have, a
/// method it does not allow, a body too large, the engine unavailable) is a closed body with a status, and none of them is
/// read by the client as started or as ended.
#[test]
fn the_plain_bodies_are_closed_and_never_read_as_started_or_ended() {
    for plain in [Plain::Rejected, Plain::Unavailable] {
        let (status, body) = plain_body(plain);
        assert!(status == 400 || status == 503, "{plain:?}: {status}");
        let text = String::from_utf8(body.clone()).expect("text");
        assert!(
            text == r#"{"result":"rejected"}"# || text == r#"{"result":"unavailable"}"#,
            "{text}"
        );
        assert_eq!(read_start(status, &body), StartOutcome::Unavailable);
        assert_eq!(read_wait(status, &body), WaitOutcome::Unavailable);
        assert_eq!(
            read_kill(status, &body),
            KillOutcome {
                ended: false,
                memory_killed: false
            }
        );
    }
}

/// The statuses are part of the contract: the same answer with another status is not that answer.
#[test]
fn an_answer_with_the_status_of_another_is_not_that_answer() {
    let (_, started) = start_body(StartAnswer::Started);
    let (_, refused) = start_body(StartAnswer::Refused(Reason::Ceiling));
    let (_, unavailable) = start_body(StartAnswer::Unavailable);
    for status in [100, 201, 202, 204, 301, 400, 404, 409, 500, 502, 503] {
        assert_eq!(
            read_start(status, &started),
            StartOutcome::Unavailable,
            "{status}"
        );
    }
    for status in [200, 201, 400, 500, 503] {
        assert_eq!(
            read_start(status, &refused),
            StartOutcome::Unavailable,
            "{status}"
        );
    }
    for status in [200, 409, 500] {
        assert_eq!(
            read_start(status, &unavailable),
            StartOutcome::Unavailable,
            "{status}"
        );
    }
    let (_, ended) = kill_body(true, false);
    for status in [201, 204, 400, 404, 500, 503] {
        assert_eq!(
            read_kill(status, &ended),
            KillOutcome {
                ended: false,
                memory_killed: false
            },
            "{status}"
        );
    }
}

/// Bodies are small. A body over the bound is never parsed, in either direction.
#[test]
fn a_body_over_the_bound_is_refused_unparsed_in_both_directions() {
    let padding = " ".repeat(MAX_BODY_BYTES);
    let request = format!(r#"{{"slot":3,"class":"small","deadline_ms":60000}}{padding}"#);
    assert!(request.len() > MAX_BODY_BYTES);
    assert!(StartBody::parse(request.as_bytes()).is_err());
    let answer = format!(r#"{{"result":"started"}}{padding}"#);
    assert_eq!(
        read_start(200, answer.as_bytes()),
        StartOutcome::Unavailable
    );
    let killed = format!(r#"{{"ended":true,"memory_killed":false}}{padding}"#);
    assert_eq!(
        read_kill(200, killed.as_bytes()),
        KillOutcome {
            ended: false,
            memory_killed: false
        }
    );
    // and one at the bound is read: the bound is on the length, not a reason to refuse a small body
    let exact = format!(
        r#"{{"slot":3,"class":"small","deadline_ms":60000}}{}"#,
        " ".repeat(MAX_BODY_BYTES - r#"{"slot":3,"class":"small","deadline_ms":60000}"#.len())
    );
    assert_eq!(exact.len(), MAX_BODY_BYTES);
    assert!(StartBody::parse(exact.as_bytes()).is_ok());
}

/// Bytes that are not text are not an answer, and not a request.
#[test]
fn bytes_that_are_not_text_are_neither_a_request_nor_an_answer() {
    for bytes in [&[0xff, 0xfe, 0xfd][..], &[0x00][..], b"\xef\xbb\xbf{}"] {
        assert!(StartBody::parse(bytes).is_err(), "{bytes:?}");
        assert_eq!(read_start(200, bytes), StartOutcome::Unavailable);
        assert_eq!(read_wait(200, bytes), WaitOutcome::Unavailable);
        assert_eq!(
            read_kill(200, bytes),
            KillOutcome {
                ended: false,
                memory_killed: false
            }
        );
    }
}

#[test]
fn a_slot_is_a_user_of_the_lake_range_and_none_past_the_last() {
    assert_eq!(uid_for_slot(0), Some(FIRST_UID));
    assert_eq!(uid_for_slot(MAX_SLOTS - 1), Some(FIRST_UID + MAX_SLOTS - 1));
    assert_eq!(uid_for_slot(MAX_SLOTS), None);
    assert_eq!(uid_for_slot(u32::MAX), None);
    // above the range of system accounts, and no slot is root
    const { assert!(FIRST_UID >= 10_000) };
    assert_ne!(uid_for_slot(0), Some(0));
    // the values the sandbox profile uses (`crates/infra-sandbox/src/lake.rs`); a test in the sandbox runner holds the two
    // functions equal, since neither crate may depend on the other
    assert_eq!((FIRST_UID, MAX_SLOTS), (40_000, 4096));
}

#[test]
fn the_class_is_a_closed_set_of_two_names() {
    for (class, name) in [(Class::Small, "small"), (Class::Large, "large")] {
        assert_eq!(class.as_str(), name);
        assert_eq!(Class::parse(name), Some(class));
    }
    for bad in ["", "Small", "LARGE", "medium", " small", "small ", "smal"] {
        assert_eq!(Class::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn the_reasons_have_one_name_each_and_it_reads_back() {
    for reason in [
        Reason::Ceiling,
        Reason::Slot,
        Reason::Class,
        Reason::Deadline,
        Reason::Directory,
    ] {
        assert_eq!(Reason::parse(reason.as_str()), Some(reason));
    }
    assert_eq!(Reason::parse("unknown"), None);
    assert_eq!(Reason::parse(""), None);
    assert_eq!(Reason::parse("Ceiling"), None);
}

/// The rule of ADR-0079 (rule 10) that the sandbox API and the staging directory share.
#[test]
fn an_attempt_id_is_sixty_hex_digits_and_hyphens_at_most_and_starts_with_a_digit() {
    for ok in [
        "0",
        "a",
        "0198f2c4-7b1e-7c3a-9d52-3f4a6b8c0d11",
        &"f".repeat(60),
    ] {
        assert!(attempt_id_ok(ok), "{ok:?}");
    }
    for bad in [
        "",
        "-",
        "-abc",
        "g",
        "ABC",
        "a b",
        "a/b",
        "../a",
        "a\nb",
        "a=b",
        "a,b",
        &"f".repeat(61),
    ] {
        assert!(!attempt_id_ok(bad), "{bad:?}");
    }
}

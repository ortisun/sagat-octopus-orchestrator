//! What the runner does when an attempt ends (feature 035, T035-035, D035-110, G035-054): one table from how the wait
//! ended, and what the engine said about the container, to what is reported. The function is pure; the driver gathers
//! the facts and acts on the answer (kills the container, then reports).
//!
//! Every row of the table has a test, and the table is exhaustive on purpose: a new way to end is a compile error here
//! until someone decides what it reports.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use sagat_domain::database::DatabaseFailure;
use sagat_infra_lake::control::ControlFailure;
use sagat_infra_lake::protocol::{Decoder, Event, HostResult, Outcome, ProtocolFailure};
use sagat_infra_lake::supervisor::{ContainerFacts, Disposition, Ended, dispose};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../tests/contract/iceberg-writes/fixtures/receipts/{name}.json"
    ));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A result the host sent, read by the same decoder the conversation uses.
fn result(line: &Value) -> HostResult {
    let mut decoder = Decoder::new();
    match decoder
        .line(
            &serde_json::to_vec(line).unwrap(),
            std::time::Duration::ZERO,
        )
        .unwrap()
    {
        Event::Result(result) => result,
        Event::Heartbeat => panic!("a heartbeat is not a result"),
    }
}

fn receipt_result() -> HostResult {
    result(&json!({"receipt": fixture("iceberg-committed"), "relayDenials": 0}))
}

fn failure_result(failure: &Value) -> HostResult {
    result(&json!({"failure": failure, "relayDenials": 0}))
}

const NOTHING: ContainerFacts = ContainerFacts {
    memory_killed: false,
};
const OUT_OF_MEMORY: ContainerFacts = ContainerFacts {
    memory_killed: true,
};

fn failed(disposition: Disposition) -> (DatabaseFailure, Option<String>, Option<String>) {
    match disposition {
        Disposition::Fail(failure) => (failure.code, failure.column, failure.reason),
        other => panic!("not a failure: {other:?}"),
    }
}

fn code_of(ended: Ended, facts: ContainerFacts) -> DatabaseFailure {
    failed(dispose(ended, facts)).0
}

// -- the host answered ------------------------------------------------------------------------------------------------

#[test]
fn a_receipt_completes_the_step_with_that_receipt() {
    let host_result = receipt_result();
    let Outcome::Receipt(expected) = host_result.outcome.clone() else {
        panic!("a receipt");
    };
    match dispose(Ended::Conversation(Ok(host_result)), NOTHING) {
        Disposition::Complete(receipt) => assert_eq!(*receipt, expected),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_receipt_stands_whatever_the_engine_says_about_the_container_afterwards() {
    // the host exited 0 after its result, or was killed for lingering: the work was done
    for facts in [NOTHING, OUT_OF_MEMORY] {
        assert!(matches!(
            dispose(Ended::Conversation(Ok(receipt_result())), facts),
            Disposition::Complete(_)
        ));
    }
}

#[test]
fn a_failure_of_the_host_is_reported_with_its_code_its_column_and_its_reason() {
    let reported = failure_result(
        &json!({"code": "ICEBERG_SCHEMA_MISMATCH", "column": "amount", "reason": "SCALE_OVERFLOW"}),
    );
    assert_eq!(
        failed(dispose(Ended::Conversation(Ok(reported)), NOTHING)),
        (
            DatabaseFailure::IcebergSchemaMismatch,
            Some("amount".to_owned()),
            Some("SCALE_OVERFLOW".to_owned())
        )
    );
}

#[test]
fn a_failure_without_a_column_has_none() {
    let reported = failure_result(&json!({"code": "CATALOG_UNAVAILABLE"}));
    assert_eq!(
        failed(dispose(Ended::Conversation(Ok(reported)), NOTHING)),
        (DatabaseFailure::CatalogUnavailable, None, None)
    );
}

/// The host holds the credentials and the column is text it chooses (G035-057). A name a person would write is kept;
/// text with a character that carries an encoding, a markup or a separator is dropped, and the code is still reported.
/// (A control character never gets here: the decoder refuses a failure that has one.)
#[test]
fn a_column_that_is_not_a_plain_name_is_not_repeated() {
    for kept in [
        "amount",
        "customer name",
        "ünïcode_1",
        "a.b-c",
        "金额",
        &"c".repeat(128),
    ] {
        let reported = failure_result(
            &json!({"code": "ICEBERG_SCHEMA_MISMATCH", "column": kept, "reason": "TYPE"}),
        );
        let (_, column, _) = failed(dispose(Ended::Conversation(Ok(reported)), NOTHING));
        assert_eq!(column.as_deref(), Some(kept), "{kept}");
    }
    for dropped in [
        "YWJjZGVm+/=",
        "x;y",
        "<script>",
        "a/b",
        "col\u{200b}umn",
        "{\"k\":1}",
        "a`b",
    ] {
        let reported = failure_result(
            &json!({"code": "ICEBERG_SCHEMA_MISMATCH", "column": dropped, "reason": "TYPE"}),
        );
        let (code, column, reason) = failed(dispose(Ended::Conversation(Ok(reported)), NOTHING));
        assert_eq!(code, DatabaseFailure::IcebergSchemaMismatch, "{dropped}");
        assert_eq!(column, None, "{dropped:?}");
        assert_eq!(reason.as_deref(), Some("TYPE"));
    }
}

// -- the host broke the protocol or never spoke ------------------------------------------------------------------------

#[test]
fn every_way_the_conversation_breaks_is_a_host_protocol_failure() {
    for broken in [
        ControlFailure::Silent,
        ControlFailure::Protocol(ProtocolFailure::NoResult),
        ControlFailure::Protocol(ProtocolFailure::Malformed),
        ControlFailure::Protocol(ProtocolFailure::UnknownMember),
        ControlFailure::Protocol(ProtocolFailure::LineTooLong),
        ControlFailure::Protocol(ProtocolFailure::HeartbeatFlood),
        ControlFailure::Protocol(ProtocolFailure::LineAfterResult),
        ControlFailure::Protocol(ProtocolFailure::InvalidReceipt),
        ControlFailure::Protocol(ProtocolFailure::InvalidFailure),
    ] {
        assert_eq!(
            code_of(Ended::Conversation(Err(broken)), NOTHING),
            DatabaseFailure::LakeHostProtocol,
            "{broken:?}"
        );
    }
}

#[test]
fn a_host_that_never_opened_the_conversation_gave_no_result() {
    assert_eq!(
        code_of(Ended::HostNeverConnected, NOTHING),
        DatabaseFailure::LakeHostProtocol
    );
}

// -- the memory limit --------------------------------------------------------------------------------------------------

/// A container the memory limit killed ends its host with no word: the conversation reports no result, or silence if it
/// was swapping to its end. What the engine says tells that from a host that merely vanished, and the write does not
/// fit in the limit, so repeating it repeats the kill.
#[test]
fn a_host_the_memory_limit_killed_exceeded_the_limit_and_did_not_break_the_protocol() {
    for ended in [
        Ended::Conversation(Err(ControlFailure::Protocol(ProtocolFailure::NoResult))),
        Ended::Conversation(Err(ControlFailure::Silent)),
        Ended::HostNeverConnected,
    ] {
        assert_eq!(
            code_of(ended, OUT_OF_MEMORY),
            DatabaseFailure::IcebergLimitExceeded
        );
    }
    assert!(!DatabaseFailure::IcebergLimitExceeded.retryable());
}

#[test]
fn a_host_that_broke_the_protocol_before_the_memory_limit_killed_it_still_broke_it() {
    for broken in [
        ProtocolFailure::Malformed,
        ProtocolFailure::UnknownMember,
        ProtocolFailure::LineTooLong,
        ProtocolFailure::HeartbeatFlood,
        ProtocolFailure::LineAfterResult,
    ] {
        assert_eq!(
            code_of(
                Ended::Conversation(Err(ControlFailure::Protocol(broken))),
                OUT_OF_MEMORY
            ),
            DatabaseFailure::LakeHostProtocol,
            "{broken:?}"
        );
    }
}

// -- the sandbox and the clock -----------------------------------------------------------------------------------------

#[test]
fn a_container_that_could_not_be_started_is_a_sandbox_failure_that_can_be_retried() {
    assert_eq!(
        code_of(Ended::SandboxUnavailable, NOTHING),
        DatabaseFailure::LakeSandboxUnavailable
    );
    assert!(DatabaseFailure::LakeSandboxUnavailable.retryable());
}

#[test]
fn the_deadline_is_a_timeout_whatever_the_container_looked_like() {
    for facts in [NOTHING, OUT_OF_MEMORY] {
        assert_eq!(
            code_of(Ended::DeadlinePassed, facts),
            DatabaseFailure::Timeout
        );
    }
}

#[test]
fn a_cancellation_is_reported_as_one() {
    for facts in [NOTHING, OUT_OF_MEMORY] {
        assert_eq!(code_of(Ended::Cancelled, facts), DatabaseFailure::Cancelled);
    }
}

#[test]
fn a_lost_lease_reports_nothing() {
    // the lease is fenced: a report from this runner would be refused, and the next attempt settles what was sent
    for facts in [NOTHING, OUT_OF_MEMORY] {
        assert!(matches!(
            dispose(Ended::LeaseLost, facts),
            Disposition::Abandon
        ));
    }
}

/// An attempt that ended without a word never says an unknown outcome: the next attempt looks the effect key up in the
/// table's history and settles what was committed (D035-030), so no ending here is `OUTCOME_UNKNOWN`.
#[test]
fn no_ending_reports_an_unknown_outcome() {
    let endings = [
        Ended::Conversation(Err(ControlFailure::Silent)),
        Ended::Conversation(Err(ControlFailure::Protocol(ProtocolFailure::NoResult))),
        Ended::HostNeverConnected,
        Ended::SandboxUnavailable,
        Ended::DeadlinePassed,
        Ended::Cancelled,
    ];
    for ended in endings {
        for facts in [NOTHING, OUT_OF_MEMORY] {
            let (code, _, _) = failed(dispose(ended.clone(), facts));
            assert_ne!(code, DatabaseFailure::OutcomeUnknown);
        }
    }
}

// -- two endings at once -----------------------------------------------------------------------------------------------

fn ok_conversation() -> Ended {
    Ended::Conversation(Ok(receipt_result()))
}

fn broken_conversation() -> Ended {
    Ended::Conversation(Err(ControlFailure::Silent))
}

/// The driver waits on several things and two can be ready in the same poll. Which wins is decided here and not by the
/// order a `select!` happens to list them: a lost lease first (nothing can be reported), then a result the host gave (the
/// work was done), then what a person asked for, then the clock, then what the host and the sandbox did.
#[test]
fn when_two_endings_are_ready_together_the_stronger_one_wins() {
    let order = [
        Ended::LeaseLost,
        ok_conversation(),
        Ended::Cancelled,
        Ended::DeadlinePassed,
        broken_conversation(),
        Ended::HostNeverConnected,
        Ended::SandboxUnavailable,
    ];
    for (index, stronger) in order.iter().enumerate() {
        for weaker in &order[index + 1..] {
            for candidates in [
                vec![stronger.clone(), weaker.clone()],
                vec![weaker.clone(), stronger.clone()],
            ] {
                assert_eq!(
                    Ended::strongest(candidates),
                    Some(stronger.clone()),
                    "{stronger:?} against {weaker:?}"
                );
            }
        }
    }
}

#[test]
fn a_result_the_host_gave_beats_the_deadline_that_passed_while_it_was_read() {
    let winner = Ended::strongest([Ended::DeadlinePassed, ok_conversation()]).unwrap();
    assert!(matches!(dispose(winner, NOTHING), Disposition::Complete(_)));
}

#[test]
fn a_lost_lease_beats_a_result_because_nothing_can_be_reported() {
    let winner = Ended::strongest([ok_conversation(), Ended::LeaseLost]).unwrap();
    assert!(matches!(dispose(winner, NOTHING), Disposition::Abandon));
}

#[test]
fn nothing_ready_is_no_ending() {
    assert_eq!(Ended::strongest(Vec::<Ended>::new()), None);
}

#[test]
fn one_ending_alone_is_itself_and_equal_ones_keep_the_first() {
    assert_eq!(Ended::strongest([Ended::Cancelled]), Some(Ended::Cancelled));
    let first = Ended::Conversation(Err(ControlFailure::Silent));
    let second = Ended::Conversation(Err(ControlFailure::Protocol(ProtocolFailure::NoResult)));
    assert_eq!(Ended::strongest([first.clone(), second]), Some(first));
}

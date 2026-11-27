//! A step-log batch carries the text a step's code logged and the worker's tokens (design 041,
//! FR-041-007 (d)): the `Debug` of `AppendStepLogsRequest`, `StepLogRecord` and `StepLogField` prints
//! counts only, so no `tracing` field, panic message or `{:?}` of the server can copy them.
#![allow(clippy::expect_used)]

use sagat_api_grpc::pb;

const MESSAGE: &str = "canary-message-4f1d";
const EXCEPTION: &str = "canary-exception-77ab";
const KEY: &str = "canary-key-90c2";
const VALUE: &str = "canary-value-31e8";
const SESSION_TOKEN: &str = "canary-session-token-c5";
const LEASE_TOKEN: &str = "canary-lease-token-a9";

fn request() -> pb::AppendStepLogsRequest {
    pb::AppendStepLogsRequest {
        worker_id: "w".into(),
        session_token: SESSION_TOKEN.into(),
        lease_token: LEASE_TOKEN.into(),
        records: vec![pb::StepLogRecord {
            sequence: 7,
            level: pb::StepLogLevel::Info as i32,
            message: MESSAGE.into(),
            exception: EXCEPTION.into(),
            fields: vec![pb::StepLogField {
                key: KEY.into(),
                value: Some(pb::step_log_field::Value::StringValue(VALUE.into())),
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The attempt's last records, as its report carries them (design 041 section 2a).
fn carry() -> pb::StepLogCarry {
    pb::StepLogCarry {
        records: request().records,
        highest_sequence: 9,
    }
}

fn assert_clean(text: &str) {
    for canary in [MESSAGE, EXCEPTION, KEY, VALUE, SESSION_TOKEN, LEASE_TOKEN] {
        assert!(!text.contains(canary), "{canary} printed in {text}");
    }
}

#[test]
fn the_debug_of_a_step_log_batch_prints_counts_and_no_text() {
    let text = format!("{:?}", request());
    assert_clean(&text);
    assert!(text.contains("records: 1"), "{text}");
}

#[test]
fn the_debug_of_a_record_a_field_and_a_value_prints_no_text() {
    let request = request();
    let record = &request.records[0];
    assert_clean(&format!("{record:?}"));
    assert_clean(&format!("{:?}", record.fields[0]));
    assert_clean(&format!("{:?}", record.fields[0].value));
    assert_clean(&format!("{request:#?}"));
}

/// A report that holds a carry prints the carry's counts and none of its text (FR-041-007 (d)): the carry prints its
/// number of records and its highest sequence, never a record.
#[test]
fn the_debug_of_a_report_that_holds_a_carry_prints_counts_and_no_text() {
    let complete = pb::CompleteStepRequest {
        lease_token: LEASE_TOKEN.into(),
        step_logs: Some(carry()),
        ..Default::default()
    };
    let fail = pb::FailStepRequest {
        lease_token: LEASE_TOKEN.into(),
        step_logs: Some(carry()),
        ..Default::default()
    };
    for text in [
        format!("{complete:?}"),
        format!("{fail:#?}"),
        format!("{:?}", carry()),
    ] {
        assert_clean(&text.replace(LEASE_TOKEN, ""));
        assert!(text.contains("records: 1"), "{text}");
        assert!(text.contains("highest_sequence: 9"), "{text}");
        assert!(
            !text.contains("sequence: 7"),
            "a record is printed in {text}"
        );
    }
}

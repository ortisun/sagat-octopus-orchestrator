//! W3C trace context handling (NFR-001-OBS).

use std::collections::HashSet;

use sagat_telemetry::{TraceContext, TraceId};

const VALID: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

#[test]
fn a_valid_traceparent_is_continued() {
    let ctx = TraceContext::from_traceparent(Some(VALID));
    assert_eq!(
        ctx.trace_id().to_string(),
        "4bf92f3577b34da6a3ce929d0e0e4736"
    );
    assert!(ctx.is_continuation());
    assert!(ctx.sampled());
}

#[test]
fn the_sampled_flag_follows_the_low_bit() {
    let unsampled = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00";
    assert!(!TraceContext::from_traceparent(Some(unsampled)).sampled());
    let other_flags = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-03";
    assert!(TraceContext::from_traceparent(Some(other_flags)).sampled());
}

#[test]
fn a_missing_header_starts_a_new_trace() {
    let ctx = TraceContext::from_traceparent(None);
    assert!(!ctx.is_continuation());
    assert_eq!(ctx.trace_id().to_string().len(), 32);
}

#[test]
fn malformed_headers_never_fail_and_never_leak_into_the_trace_id() {
    let hostile = [
        "",
        "garbage",
        // wrong version / the reserved ff version
        "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        // all-zero ids are invalid by spec
        "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
        // uppercase hex is invalid
        "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
        // wrong lengths
        "00-4bf92f3577b34da6a3ce929d0e0e473-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e47366-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-1",
        // extra / missing fields
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7",
        // non-hex, injection shapes, multibyte characters
        "00-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-0\n1",
        "00-\u{e9}bf92f3577b34da6a3ce929d0e0e47-00f067aa0ba902b7-01",
        "00-'; DROP TABLE audit_records;--",
    ];
    for header in hostile {
        let ctx = TraceContext::from_traceparent(Some(header));
        assert!(!ctx.is_continuation(), "must not continue {header:?}");
        let id = ctx.trace_id().to_string();
        assert_eq!(id.len(), 32, "{header:?}");
        assert!(
            id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "{header:?} -> {id}"
        );
    }
}

#[test]
fn an_enormous_header_is_handled() {
    let huge = "0".repeat(1_000_000);
    let ctx = TraceContext::from_traceparent(Some(&huge));
    assert!(!ctx.is_continuation());
}

#[test]
fn generated_ids_are_unique_lowercase_hex_that_round_trip() {
    let mut seen = HashSet::new();
    for _ in 0..1_000 {
        let id = TraceId::generate();
        let text = id.to_string();
        assert_eq!(text.len(), 32);
        assert_eq!(TraceId::parse(&text), Some(id), "round trip");
        assert!(seen.insert(text), "collision");
    }
}

#[test]
fn parse_rejects_what_it_should() {
    assert!(TraceId::parse("4bf92f3577b34da6a3ce929d0e0e4736").is_some());
    assert!(TraceId::parse("00000000000000000000000000000000").is_none());
    assert!(TraceId::parse("4BF92F3577B34DA6A3CE929D0E0E4736").is_none());
    assert!(TraceId::parse("").is_none());
}

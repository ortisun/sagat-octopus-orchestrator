//! The trace of work the engine starts (`D2084`, `D2087`, `NFR-018-006`): a context with a trace keeps it, one without is given a
//! fresh random one that is a valid W3C trace id, and the rest of the context is not touched.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::identity::RequestContext;
use sagat_telemetry::TraceId;

#[test]
fn a_trace_the_context_has_is_kept_and_so_is_the_span() {
    let known = TraceId::generate().to_string();
    let ctx = RequestContext {
        trace_id: Some(known.clone()),
        span_id: Some("0123456789abcdef".to_owned()),
    };
    let traced = ctx.traced();
    assert_eq!(traced.trace_id.as_deref(), Some(known.as_str()));
    assert_eq!(traced.span_id.as_deref(), Some("0123456789abcdef"));
}

#[test]
fn a_context_without_a_trace_is_given_32_lowercase_hex_characters_that_are_not_zero_and_never_twice_the_same()
 {
    let ctx = RequestContext {
        trace_id: None,
        span_id: Some("0123456789abcdef".to_owned()),
    };
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..64 {
        let id = ctx.traced().trace_id.expect("a trace");
        assert_eq!(id.len(), 32, "{id}");
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "lowercase hex: {id}"
        );
        assert_ne!(id, "0".repeat(32), "the all-zero trace is invalid");
        assert!(
            TraceId::parse(&id).is_some(),
            "a trace id the exporter accepts: {id}"
        );
        assert!(seen.insert(id), "two contexts got the same trace");
    }
    assert!(ctx.trace_id.is_none(), "the context itself is not changed");
    assert_eq!(
        ctx.traced().span_id.as_deref(),
        Some("0123456789abcdef"),
        "the span is kept"
    );
}

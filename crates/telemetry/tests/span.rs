//! Span ids: the ones derived from identifiers, and the header that carries them (FR-008-001).

use std::collections::HashSet;

use sagat_telemetry::{
    SpanId, TraceContext, TraceId, attempt_span_id, keeps, run_span_id, traceparent,
};
use uuid::Uuid;

#[test]
fn the_span_of_a_run_is_the_same_wherever_it_is_computed() {
    let run = Uuid::now_v7();
    assert_eq!(run_span_id(run), run_span_id(run));
    assert_ne!(run_span_id(run), run_span_id(Uuid::now_v7()));
}

#[test]
fn every_attempt_of_a_step_has_a_span_of_its_own_and_the_run_has_another() {
    let step = Uuid::new_v4();
    let spans: HashSet<SpanId> = (1..=10_000).map(|n| attempt_span_id(step, n)).collect();
    assert_eq!(spans.len(), 10_000, "two attempts of one step share a span");
    assert_eq!(attempt_span_id(step, 3), attempt_span_id(step, 3));
    // A run id and a step id that share their low bytes still give different spans.
    assert_ne!(run_span_id(step), attempt_span_id(step, 1));
}

#[test]
fn no_span_id_is_ever_zero() {
    assert_eq!(SpanId::from_u64(0).to_string(), "0000000000000001");
    assert!(SpanId::parse("0000000000000000").is_none());
    let zero_low = Uuid::from_bytes(
        [7; 8]
            .into_iter()
            .chain([0; 8])
            .collect::<Vec<u8>>()
            .try_into()
            .expect("16 bytes"),
    );
    for span in [run_span_id(zero_low), attempt_span_id(zero_low, 0)] {
        assert_ne!(span.as_bytes(), &[0u8; 8]);
    }
}

#[test]
fn a_traceparent_built_here_is_read_back_by_the_parser() {
    let trace = TraceId::generate();
    let span = attempt_span_id(Uuid::new_v4(), 2);
    for sampled in [true, false] {
        let header = traceparent(trace, span, sampled);
        let context = TraceContext::from_traceparent(Some(&header));
        assert!(context.is_continuation(), "{header}");
        assert_eq!(context.trace_id(), trace);
        assert_eq!(context.parent_span_id(), Some(span));
        assert_eq!(context.sampled(), sampled);
    }
}

#[test]
fn a_context_can_continue_a_stored_trace() {
    let trace = TraceId::generate();
    let span = run_span_id(Uuid::now_v7());
    let context = TraceContext::continuing(trace, Some(span), true);
    assert_eq!(context.trace_id(), trace);
    assert_eq!(context.parent_span_id(), Some(span));
    assert!(context.is_continuation() && context.sampled());
    assert!(!TraceContext::continuing(trace, None, false).is_continuation());
}

#[test]
fn the_decision_to_keep_a_trace_depends_on_its_id_alone() {
    let trace = TraceId::generate();
    assert_eq!(keeps(trace, 0.3), keeps(trace, 0.3));
    assert!(keeps(trace, 1.0) && keeps(trace, 7.0));
    assert!(!keeps(trace, 0.0) && !keeps(trace, -1.0) && !keeps(trace, f64::NAN));
}

#[test]
fn the_share_of_traces_kept_follows_the_ratio() {
    let total = 20_000;
    for ratio in [0.01, 0.1, 0.5, 0.9] {
        let kept = (0..total)
            .filter(|_| keeps(TraceId::generate(), ratio))
            .count();
        let share = kept as f64 / f64::from(total);
        assert!(
            (share - ratio).abs() < 0.03,
            "ratio {ratio}: kept {share} of {total}"
        );
    }
}

#[test]
fn a_greater_ratio_never_keeps_less() {
    for _ in 0..2_000 {
        let trace = TraceId::generate();
        let mut was_kept = false;
        for step in 0..=10 {
            let kept = keeps(trace, f64::from(step) / 10.0);
            assert!(
                kept || !was_kept,
                "kept at a smaller ratio and not at a greater one"
            );
            was_kept = kept;
        }
    }
}

//! Span identity: the ids a trace is made of, and the ones that can be derived instead of drawn.
//!
//! A span id is drawn at random for the work a process does while a request is open (the HTTP and
//! gRPC hops). The two spans the platform derives from its journal, the **run** and each **attempt**
//! of a step, get ids computed from the identifiers the platform already has. That is what lets the
//! lease carry the span of the attempt to a worker before the attempt exists as an exported span,
//! lets a scheduler pass parent its work to the run's span without asking anyone, and makes a span
//! that is exported twice the same span (D162).

use std::fmt;

use uuid::Uuid;

use crate::metrics::{
    WorkflowPrimitiveKind, WorkflowPrimitiveOutcome, WorkflowPrimitiveRejectionCode,
};
use crate::otlp::AttrValue;
use crate::trace::{TraceId, decode_hex};

/// Span names emitted for workflow primitive operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPrimitiveSpan {
    ScopeAdmit,
    IterationCommit,
    ApprovalDecide,
    Query,
    Update,
    Continue,
    SwitchEvaluate,
    /// A journal entry this version of the projector cannot interpret (a newer writer).
    Unsupported,
}

impl WorkflowPrimitiveSpan {
    pub const ALL: [Self; 8] = [
        Self::ScopeAdmit,
        Self::IterationCommit,
        Self::ApprovalDecide,
        Self::Query,
        Self::Update,
        Self::Continue,
        Self::SwitchEvaluate,
        Self::Unsupported,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ScopeAdmit => "workflow.scope.admit",
            Self::IterationCommit => "workflow.iteration.commit",
            Self::ApprovalDecide => "workflow.approval.decide",
            Self::Query => "workflow.query",
            Self::Update => "workflow.update",
            Self::Continue => "workflow.continue",
            Self::SwitchEvaluate => "workflow.switch.evaluate",
            Self::Unsupported => "workflow.unsupported",
        }
    }
}

/// Primitive span attributes whose values cannot contain user-controlled text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPrimitiveSpanField {
    Kind(WorkflowPrimitiveKind),
    Outcome(WorkflowPrimitiveOutcome),
    Rejection(WorkflowPrimitiveRejectionCode),
    ScopeDepth(u8),
    IterationIndex(u32),
    SegmentIndex(u32),
}

impl WorkflowPrimitiveSpanField {
    #[must_use]
    pub fn attribute(self) -> (&'static str, AttrValue) {
        match self {
            Self::Kind(kind) => ("sagat.primitive.kind", kind.as_str().into()),
            Self::Outcome(outcome) => ("sagat.outcome", outcome.as_str().into()),
            Self::Rejection(code) => ("sagat.rejection.code", code.as_str().into()),
            Self::ScopeDepth(depth) => ("sagat.scope.depth", u32::from(depth).into()),
            Self::IterationIndex(index) => ("sagat.iteration.index", index.into()),
            Self::SegmentIndex(index) => ("sagat.continuation.segment_index", index.into()),
        }
    }
}

/// Mixed into the id of a run's span so that it never equals that of an attempt of a step whose id
/// shares its low bytes.
const RUN_DOMAIN: u64 = 0x5241_4E52_5553_4F4E;
/// The same, for an attempt.
const ATTEMPT_DOMAIN: u64 = 0x4154_5445_4D50_5421;
/// Mixed into the id of a span derived from one event of a run's journal.
const EVENT_DOMAIN: u64 = 0x4A4F_5552_4E41_4C21;
/// Odd, so that multiplying by it is a bijection on `u64`: two attempts never share an id.
const ATTEMPT_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// 16 lowercase hexadecimal characters, never all zeroes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpanId(pub(crate) [u8; 8]);

impl SpanId {
    /// A fresh random span id.
    #[must_use]
    pub fn generate() -> Self {
        let bytes = Uuid::new_v4().into_bytes();
        let mut id = [0u8; 8];
        id.copy_from_slice(&bytes[..8]);
        Self::from_u64(u64::from_be_bytes(id))
    }

    /// The id with these bits; zero (which W3C forbids) becomes one.
    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(if value == 0 { 1 } else { value }.to_be_bytes())
    }

    /// Parses 16 lowercase hex characters; rejects the all-zero id.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        decode_hex::<8>(value)
            .filter(|id| *id != [0u8; 8])
            .map(Self)
    }

    /// The 8 bytes of the id.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

impl fmt::Display for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SpanId({self})")
    }
}

fn low_bits(id: Uuid) -> u64 {
    let bytes = id.as_bytes();
    let mut low = [0u8; 8];
    low.copy_from_slice(&bytes[8..]);
    u64::from_be_bytes(low)
}

/// The span of a run: the same for the same run, wherever and whenever it is computed.
#[must_use]
pub fn run_span_id(run: Uuid) -> SpanId {
    SpanId::from_u64(low_bits(run) ^ RUN_DOMAIN)
}

/// The span of one attempt of a step: the same for the same attempt, different for every attempt.
#[must_use]
pub fn attempt_span_id(step: Uuid, attempt: u32) -> SpanId {
    let mix = (u64::from(attempt) + 1).wrapping_mul(ATTEMPT_MIX);
    SpanId::from_u64(low_bits(step) ^ ATTEMPT_DOMAIN ^ mix)
}

/// The span that starts at the event `seq` of a run's journal (a timer, a wait): the same for the
/// same event, different for every event of the run.
#[must_use]
pub fn event_span_id(run: Uuid, seq: u64) -> SpanId {
    let mix = seq.wrapping_add(1).wrapping_mul(ATTEMPT_MIX);
    SpanId::from_u64(low_bits(run) ^ EVENT_DOMAIN ^ mix)
}

/// The `traceparent` header of `span` in `trace`.
#[must_use]
pub fn traceparent(trace: TraceId, span: SpanId, sampled: bool) -> String {
    format!("00-{trace}-{span}-{}", if sampled { "01" } else { "00" })
}

/// Whether the trace is kept, decided from the trace id alone so that every process (and the
/// projector, long after the request) reaches the same answer without being told. `ratio` is the
/// share of traces kept: at or above 1 all of them, at or below 0 none.
#[must_use]
pub fn keeps(trace: TraceId, ratio: f64) -> bool {
    if ratio >= 1.0 {
        return true;
    }
    if ratio.is_nan() || ratio <= 0.0 {
        return false;
    }
    // The last seven bytes of a trace id are the random ones (W3C Trace Context level 2); the
    // first of the last eight is not, in the ids generated here (a UUID's variant bits).
    let bytes = trace.as_bytes();
    let mut random = [0u8; 8];
    random[1..].copy_from_slice(&bytes[9..]);
    let drawn = u64::from_be_bytes(random);
    // 2^56: the range `drawn` falls in. Truncation is the point: the threshold is a share of it.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let threshold = (ratio * (1u64 << 56) as f64) as u64;
    drawn < threshold
}

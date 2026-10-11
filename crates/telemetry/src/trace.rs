//! W3C Trace Context (`traceparent`) handling.

use std::fmt;

use uuid::Uuid;

/// 32 lowercase hexadecimal characters, never all zeroes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceId([u8; 16]);

impl TraceId {
    /// A fresh random trace id.
    #[must_use]
    pub fn generate() -> Self {
        // UUIDv4 has 122 random bits and is never all zeroes (the version nibble is fixed).
        Self(*Uuid::new_v4().as_bytes())
    }

    /// Parses 32 lowercase hex characters; rejects the all-zero id.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        decode_hex::<16>(value)
            .filter(|id| *id != [0u8; 16])
            .map(Self)
    }
}

/// Decodes exactly `2 * N` lowercase hex characters.
fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes = text.as_bytes();
    if bytes.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(bytes.as_chunks::<2>().0) {
        *slot = (hex_value(pair[0])? << 4) | hex_value(pair[1])?;
    }
    Some(out)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None, // uppercase is invalid in traceparent
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TraceId({self})")
    }
}

/// Trace identity of one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceContext {
    trace_id: TraceId,
    /// Span id of the caller (the `parent-id` field), when the request carried a valid header.
    parent_span_id: Option<[u8; 8]>,
    sampled: bool,
}

impl TraceContext {
    /// Continues the trace of a valid `traceparent` header, otherwise starts a new one.
    ///
    /// Only version `00` with the exact four-field layout is accepted; anything malformed is
    /// ignored (a bad header must never fail a request), and a new trace id is generated.
    #[must_use]
    pub fn from_traceparent(header: Option<&str>) -> Self {
        header
            .and_then(Self::parse_traceparent)
            .unwrap_or_else(|| Self {
                trace_id: TraceId::generate(),
                parent_span_id: None,
                sampled: false,
            })
    }

    fn parse_traceparent(value: &str) -> Option<Self> {
        let mut parts = value.split('-');
        let (version, trace, parent, flags) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || version != "00" {
            return None;
        }
        let trace_id = TraceId::parse(trace)?;
        let span = decode_hex::<8>(parent).filter(|span| *span != [0u8; 8])?;
        let [flag_bits] = decode_hex::<1>(flags)?;
        Some(Self {
            trace_id,
            parent_span_id: Some(span),
            sampled: flag_bits & 1 == 1,
        })
    }

    #[must_use]
    pub const fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    /// Whether the caller asked for this trace to be sampled.
    #[must_use]
    pub const fn sampled(&self) -> bool {
        self.sampled
    }

    /// Whether the trace continues an inbound `traceparent`.
    #[must_use]
    pub const fn is_continuation(&self) -> bool {
        self.parent_span_id.is_some()
    }
}

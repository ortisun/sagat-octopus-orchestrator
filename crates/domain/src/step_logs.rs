//! Step logs (feature 041, ADR-0084): the lines an activity writes to the SDK's logger, stored per
//! attempt beside the journal, never execution state.
//!
//! This module holds the closed sets of `specs/041-step-logs/data-model.md` §1, the bounds of a batch
//! (`validate_batch`, design section 2, P4), the canonical form of a line's fields (data-model.md §5)
//! and `plan_batch`, the pure rule that removes duplicates, counts gaps, applies the attempt and
//! tenant allowances and decides the marker and the verdict (T5 and T6). The counting rule it keeps,
//! for every stream over any sequence of plans and refusals while the drop counts are below their
//! saturation at 2^53 (SC-041-002): lines stored (markers excluded) + dropped by the worker + dropped
//! by the server = `last_sequence`.

use std::collections::{HashMap, HashSet};
use std::fmt;

/// At most this many records in one batch.
pub const MAX_BATCH_RECORDS: usize = 500;
/// At most this many encoded bytes in one batch (192 KiB).
pub const MAX_BATCH_BYTES: usize = 192 * 1024;
/// The highest sequence a worker may assign: 2^62 − 1.
pub const MAX_WORKER_SEQUENCE: u64 = (1 << 62) - 1;
/// The first sequence of the server's markers: 2^62.
pub const FIRST_MARKER_SEQUENCE: u64 = 1 << 62;
/// The drop counts saturate here (2^53), so a JavaScript reader keeps them exact.
pub const DROP_SATURATION: u64 = 1 << 53;
/// A message and an exception text are cut at this many bytes, at a character boundary.
pub const MAX_TEXT_BYTES: usize = 8 * 1024;
/// The canonical fields of a line hold at most this many bytes …
pub const MAX_FIELDS_BYTES: usize = 2 * 1024;
/// … and at most this many keys.
pub const MAX_FIELDS: usize = 32;
/// What every stored row is charged on top of its text (OD-2026-11-15-6).
pub const ROW_CHARGE: u64 = 128;
/// The value a redacted field, key or number becomes.
pub const REDACTED: &str = "[REDACTED]";

/// The five levels, in the order `minLevel` compares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StepLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl StepLogLevel {
    pub const ALL: [Self; 5] = [
        Self::Trace,
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// The level of its stored or query name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.as_str() == name)
    }
}

/// What a stored row is (data-model.md §1). Markers are the server's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Line,
    CeilingMarker,
    ScriptMarker,
}

impl LineKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::CeilingMarker => "ceiling_marker",
            Self::ScriptMarker => "script_marker",
        }
    }
}

/// Which attempt ceiling a ceiling marker records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerReason {
    AttemptRecords,
    AttemptBytes,
}

impl MarkerReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AttemptRecords => "attempt_records",
            Self::AttemptBytes => "attempt_bytes",
        }
    }
}

/// The ceiling a stream met first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeilingReason {
    AttemptRecords,
    AttemptBytes,
    TenantQuota,
}

impl CeilingReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AttemptRecords => "attempt_records",
            Self::AttemptBytes => "attempt_bytes",
            Self::TenantQuota => "tenant_quota",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        [Self::AttemptRecords, Self::AttemptBytes, Self::TenantQuota]
            .into_iter()
            .find(|reason| reason.as_str() == name)
    }

    const fn marker(self) -> Option<MarkerReason> {
        match self {
            Self::AttemptRecords => Some(MarkerReason::AttemptRecords),
            Self::AttemptBytes => Some(MarkerReason::AttemptBytes),
            Self::TenantQuota => None,
        }
    }
}

/// The source of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSource {
    Worker,
    Script,
}

impl StreamSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Script => "script",
        }
    }
}

/// Why a batch stops capture for its attempt or its session (the wire's `StepLogStopReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// `STOP_ATTEMPT`: the final batch is already stored.
    FinalRecorded,
    /// `STOP_ATTEMPT`: the lease is not this attempt's, has expired, or a report would be refused.
    Lease,
    /// `STOP_ATTEMPT`: the binding or the membership no longer covers the step.
    Authority,
    /// `STOP_ATTEMPT`: the attempt's log was deleted or its removal began.
    Removed,
    /// `STOP_CAPTURE`: the deployment switch or the namespace turned capture off.
    CaptureOff,
    /// `STOP_CAPTURE`: the session is unknown, expired, of another subject, namespace or token, or
    /// below 1.7.
    Session,
}

impl StopReason {
    /// Whether the worker stops for the session (`STOP_CAPTURE`) rather than the attempt.
    #[must_use]
    pub const fn stops_capture(self) -> bool {
        matches!(self, Self::CaptureOff | Self::Session)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FinalRecorded => "final_recorded",
            Self::Lease => "lease",
            Self::Authority => "authority",
            Self::Removed => "removed",
            Self::CaptureOff => "capture_off",
            Self::Session => "session",
        }
    }
}

/// A batch outside the bounds of P4 (`INVALID_ARGUMENT`). Carries no value of the batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BatchInvalid {
    #[error("a step-log batch holds more than 500 records")]
    TooManyRecords,
    #[error("a step-log batch is larger than 192 KiB")]
    TooLarge,
    #[error("a step-log sequence is outside 1 to 2^62-1")]
    SequenceOutOfRange,
    #[error("step-log sequences are not strictly increasing")]
    NotIncreasing,
    #[error("a step-log record has an unknown level")]
    UnknownLevel,
    #[error("the highest sequence of a final batch is outside its range")]
    HighestOutOfRange,
}

/// What P4 needs of one record. `level` is `None` when the wire value names no level (the adapter maps
/// the protobuf enumeration; an unspecified or unknown value is `None`).
#[derive(Debug, Clone, Copy)]
pub struct RecordShape {
    pub sequence: u64,
    pub level: Option<StepLogLevel>,
}

/// P4: at most 500 records and 192 KiB encoded, sequences strictly increasing in 1 to 2^62 − 1, a
/// known level, and for a final batch a `highest_sequence` in range and at least the last record's.
///
/// # Errors
/// [`BatchInvalid`] naming the first bound that does not hold.
pub fn validate_batch(
    records: &[RecordShape],
    encoded_len: usize,
    final_batch: bool,
    highest_sequence: u64,
) -> Result<(), BatchInvalid> {
    if records.len() > MAX_BATCH_RECORDS {
        return Err(BatchInvalid::TooManyRecords);
    }
    if encoded_len > MAX_BATCH_BYTES {
        return Err(BatchInvalid::TooLarge);
    }
    let mut previous = 0;
    for record in records {
        if record.sequence == 0 || record.sequence > MAX_WORKER_SEQUENCE {
            return Err(BatchInvalid::SequenceOutOfRange);
        }
        if record.sequence <= previous {
            return Err(BatchInvalid::NotIncreasing);
        }
        if record.level.is_none() {
            return Err(BatchInvalid::UnknownLevel);
        }
        previous = record.sequence;
    }
    if final_batch && (highest_sequence > MAX_WORKER_SEQUENCE || highest_sequence < previous) {
        return Err(BatchInvalid::HighestOutOfRange);
    }
    Ok(())
}

/// What the stream row holds when a batch reads it (T4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamCounters {
    pub records: u64,
    pub bytes: u64,
    pub last_sequence: u64,
    pub ceiling_sequence: Option<u64>,
    pub ceiling: Option<CeilingReason>,
    pub dropped_by_worker: u64,
    pub dropped_by_server: u64,
    pub next_marker_sequence: u64,
    pub truncated_records: u64,
}

impl StreamCounters {
    /// A stream no batch has written yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            records: 0,
            bytes: 0,
            last_sequence: 0,
            ceiling_sequence: None,
            ceiling: None,
            dropped_by_worker: 0,
            dropped_by_server: 0,
            next_marker_sequence: FIRST_MARKER_SEQUENCE,
            truncated_records: 0,
        }
    }
}

/// The attempt ceilings in force for a batch (`step_log_attempt_records`, `step_log_attempt_bytes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptCeilings {
    pub records: u64,
    pub bytes: u64,
}

/// One record as the plan sees it: its sequence and its text bytes after scrubbing and cutting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizedRecord {
    pub sequence: u64,
    pub text_bytes: u64,
    pub truncated: bool,
}

/// The tenant's allowance, read without a lock (T6): `None` when the quota is not enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantAllowance {
    pub remaining: Option<u64>,
}

/// What the plan says the batch answers when it stores (T5, T6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanVerdict {
    Continue,
    /// A ceiling was met: send no more records, still send the empty final batch.
    FinalOnly,
}

/// The plan of one batch: what to insert and how the counters move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The indices, in the batch, of the records to insert, in sequence order.
    pub keep: Vec<usize>,
    /// A ceiling marker to insert, at its server sequence.
    pub marker: Option<(u64, MarkerReason)>,
    /// The counters after the batch, assuming every kept record and the marker are inserted.
    pub after: StreamCounters,
    pub verdict: PlanVerdict,
    /// Sequences this batch adds to the drop counts (before saturation), by side.
    pub newly_dropped_by_worker: u64,
    pub newly_dropped_by_server: u64,
    /// Of `newly_dropped_by_server`, those refused by the tenant quota.
    pub dropped_for_tenant: u64,
}

const fn saturating_drop(count: u64, add: u64) -> u64 {
    let sum = count.saturating_add(add);
    if sum > DROP_SATURATION {
        DROP_SATURATION
    } else {
        sum
    }
}

/// T5 and T6: duplicates (records at or below `last_sequence`) are dropped; the sequences between
/// `last_sequence` and the batch's high mark that the batch does not hold are a gap, counted as
/// dropped by the server above a ceiling already met and by the worker otherwise; records are kept
/// in sequence order until the first that does not fit the attempt ceilings, the rest dropped by the
/// server, with one marker when an attempt ceiling is first met; and if what is kept (128 bytes a row,
/// the marker included) does not fit the tenant's allowance, nothing is kept and the records are
/// dropped for the tenant quota. `records` must have passed [`validate_batch`].
#[must_use]
pub fn plan_batch(
    stream: &StreamCounters,
    ceilings: AttemptCeilings,
    tenant: TenantAllowance,
    records: &[SizedRecord],
    final_batch: bool,
    highest_sequence: u64,
) -> Plan {
    let fresh: Vec<(usize, &SizedRecord)> = records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.sequence > stream.last_sequence)
        .collect();
    let batch_mark = fresh.last().map_or(0, |(_, record)| record.sequence);
    let mark = batch_mark
        .max(if final_batch { highest_sequence } else { 0 })
        .max(stream.last_sequence);
    let sent = fresh.len() as u64;
    let gap = mark - stream.last_sequence - sent;
    // The gap below the ceiling the worker skipped; above it, the records it stopped sending after
    // FINAL_ONLY, which the server's ceiling dropped.
    let (gap_worker, gap_server) = stream.ceiling_sequence.map_or((gap, 0), |ceiling| {
        let below = if ceiling > stream.last_sequence {
            // Sequences in (last, ceiling] that the batch does not hold.
            let held = fresh
                .iter()
                .filter(|(_, record)| record.sequence <= ceiling)
                .count() as u64;
            (ceiling.min(mark) - stream.last_sequence).saturating_sub(held)
        } else {
            0
        };
        (below, gap - below)
    });
    let mut keep = Vec::new();
    let (mut records_after, mut bytes_after) = (stream.records, stream.bytes);
    let mut met = stream.ceiling;
    let mut first_met = None;
    let mut kept_bytes = 0;
    let mut truncated = 0;
    for (index, record) in &fresh {
        if met.is_some() {
            break;
        }
        if records_after + 1 > ceilings.records {
            met = Some(CeilingReason::AttemptRecords);
            first_met = met;
            break;
        }
        if bytes_after + record.text_bytes > ceilings.bytes {
            met = Some(CeilingReason::AttemptBytes);
            first_met = met;
            break;
        }
        records_after += 1;
        bytes_after += record.text_bytes;
        kept_bytes += record.text_bytes;
        truncated += u64::from(record.truncated);
        keep.push(*index);
    }
    let mut marker = first_met
        .and_then(CeilingReason::marker)
        .map(|reason| (stream.next_marker_sequence, reason));
    let charge = kept_bytes + ROW_CHARGE * (keep.len() as u64 + u64::from(marker.is_some()));
    let mut dropped_for_tenant = 0;
    let over_tenant = tenant
        .remaining
        .is_some_and(|remaining| charge > 0 && charge > remaining);
    if over_tenant {
        keep.clear();
        marker = None;
        records_after = stream.records;
        bytes_after = stream.bytes;
        truncated = 0;
        dropped_for_tenant = sent;
        if met.is_none() || first_met.is_some() {
            met = Some(CeilingReason::TenantQuota);
            first_met = met;
        }
    }
    let kept = keep.len() as u64;
    let newly_dropped_by_server = (sent - kept) + gap_server;
    let ceiling_sequence = match (stream.ceiling_sequence, first_met) {
        (Some(ceiling), _) => Some(ceiling),
        (None, Some(_)) => Some(mark),
        (None, None) => None,
    };
    let after = StreamCounters {
        records: records_after,
        bytes: bytes_after,
        last_sequence: mark,
        ceiling_sequence,
        ceiling: stream.ceiling.or(met),
        dropped_by_worker: saturating_drop(stream.dropped_by_worker, gap_worker),
        dropped_by_server: saturating_drop(stream.dropped_by_server, newly_dropped_by_server),
        next_marker_sequence: stream.next_marker_sequence + u64::from(marker.is_some()),
        truncated_records: stream.truncated_records + truncated,
    };
    Plan {
        keep,
        marker,
        verdict: if after.ceiling.is_some() {
            PlanVerdict::FinalOnly
        } else {
            PlanVerdict::Continue
        },
        after,
        newly_dropped_by_worker: gap_worker,
        newly_dropped_by_server,
        dropped_for_tenant,
    }
}

// ---------------------------------------------------------------------------------- fields

/// The value of one field of a line (FR-041-003). Its `Debug` names the variant only: the value is text a
/// step's code logged (FR-041-007 (d)).
#[derive(Clone, PartialEq)]
pub enum FieldValue {
    String(String),
    Int(i64),
    /// Finite only: a non-finite double is stored as the string `NaN`, `Infinity` or `-Infinity`.
    Double(f64),
    Bool(bool),
}

impl fmt::Debug for FieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::String(_) => "FieldValue::String(<redacted>)",
            Self::Int(_) => "FieldValue::Int(<redacted>)",
            Self::Double(_) => "FieldValue::Double(<redacted>)",
            Self::Bool(_) => "FieldValue::Bool(<redacted>)",
        })
    }
}

impl FieldValue {
    /// A double as stored: finite as a number, else its name as a string (FR-041-003).
    #[must_use]
    pub fn double(value: f64) -> Self {
        if value.is_nan() {
            Self::String("NaN".into())
        } else if value.is_infinite() {
            Self::String(if value > 0.0 { "Infinity" } else { "-Infinity" }.into())
        } else {
            Self::Double(value)
        }
    }
}

/// A NUL character becomes U+FFFD (FR-041-003).
#[must_use]
pub fn normalise(text: &str) -> String {
    text.replace('\0', "\u{FFFD}")
}

/// `text` cut to at most `max` bytes at a character boundary, and whether it was cut.
#[must_use]
pub fn cut(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// The canonical decimal text of a double (data-model.md §5): ECMAScript `Number::toString`, so
/// integral values below 10^21 print without a fraction or an exponent.
#[must_use]
pub fn canonical_double(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    // The shortest round-trip digits and exponent, from Rust's `{:e}` (d.ddd e N).
    let formatted = format!("{:e}", value.abs());
    let (mantissa, exponent) = formatted.split_once('e').unwrap_or((&formatted, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i64 = exponent.parse().unwrap_or(0);
    let k = digits.len() as i64;
    let n = exponent + 1;
    let sign = if value < 0.0 { "-" } else { "" };
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        let (head, tail) = digits.split_at(n as usize);
        format!("{head}.{tail}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let e = if e >= 0 {
            format!("+{e}")
        } else {
            e.to_string()
        };
        if k == 1 {
            format!("{digits}e{e}")
        } else {
            format!("{}.{}e{e}", &digits[..1], &digits[1..])
        }
    };
    format!("{sign}{body}")
}

/// The canonical decimal text of a number field, as an SDK redacts it (FR-041-007 (a)).
#[must_use]
pub fn canonical_number(value: &FieldValue) -> Option<String> {
    match value {
        FieldValue::Int(n) => Some(n.to_string()),
        FieldValue::Double(d) => Some(canonical_double(*d)),
        _ => None,
    }
}

fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = fmt::Write::write_fmt(out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn entry_json(key: &str, value: &FieldValue) -> String {
    let mut out = String::new();
    push_json_string(&mut out, key);
    out.push(':');
    match value {
        FieldValue::String(text) => push_json_string(&mut out, text),
        FieldValue::Int(n) => out.push_str(&n.to_string()),
        FieldValue::Double(d) => out.push_str(&canonical_double(*d)),
        FieldValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
    }
    out
}

/// The canonical fields of a line (data-model.md §5) and whether any were dropped: keys that collide
/// keep the first in the order sent and get `~2`, `~3` … appended; keys sorted by their UTF-8 bytes;
/// fields kept in key order until the next would pass 32 keys or 2,048 bytes. `None` when no field is
/// kept.
#[must_use]
pub fn canonical_fields(fields: &[(String, FieldValue)]) -> (Option<String>, bool) {
    // The names taken so far, and for each key the next suffix to try: renaming is linear in the number of
    // fields however many collide (a batch of 192 KiB can carry tens of thousands of one key).
    let mut taken: HashSet<String> = HashSet::with_capacity(fields.len());
    let mut next: HashMap<&str, u64> = HashMap::new();
    let mut named: Vec<(String, &FieldValue)> = Vec::with_capacity(fields.len());
    for (key, value) in fields {
        let candidate = if taken.contains(key.as_str()) {
            let n = next.entry(key.as_str()).or_insert(2);
            let mut candidate = format!("{key}~{n}");
            while taken.contains(&candidate) {
                *n += 1;
                candidate = format!("{key}~{n}");
            }
            *n += 1;
            candidate
        } else {
            key.clone()
        };
        taken.insert(candidate.clone());
        named.push((candidate, value));
    }
    named.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut out = String::from("{");
    let mut count = 0;
    let mut dropped = false;
    for (key, value) in &named {
        let entry = entry_json(key, value);
        let extra = entry.len() + usize::from(count > 0);
        if count == MAX_FIELDS || out.len() + extra + 1 > MAX_FIELDS_BYTES {
            dropped = true;
            break;
        }
        if count > 0 {
            out.push(',');
        }
        out.push_str(&entry);
        count += 1;
    }
    out.push('}');
    (if count == 0 { None } else { Some(out) }, dropped)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const BIG: AttemptCeilings = AttemptCeilings {
        records: 10_000,
        bytes: 1 << 20,
    };
    const OPEN: TenantAllowance = TenantAllowance { remaining: None };

    fn sized(sequences: &[u64], bytes: u64) -> Vec<SizedRecord> {
        sequences
            .iter()
            .map(|&sequence| SizedRecord {
                sequence,
                text_bytes: bytes,
                truncated: false,
            })
            .collect()
    }

    fn stored_lines(c: &StreamCounters) -> u64 {
        c.records
    }

    fn holds(c: &StreamCounters) -> bool {
        stored_lines(c) + c.dropped_by_worker + c.dropped_by_server == c.last_sequence
    }

    #[test]
    fn a_batch_is_refused_outside_its_bounds() {
        let ok = |s: u64| RecordShape {
            sequence: s,
            level: Some(StepLogLevel::Info),
        };
        assert_eq!(validate_batch(&[ok(1), ok(2)], 10, false, 0), Ok(()));
        assert_eq!(
            validate_batch(&vec![ok(1); 501], 10, false, 0),
            Err(BatchInvalid::TooManyRecords)
        );
        assert_eq!(
            validate_batch(&[ok(1)], MAX_BATCH_BYTES + 1, false, 0),
            Err(BatchInvalid::TooLarge)
        );
        assert_eq!(
            validate_batch(&[ok(0)], 10, false, 0),
            Err(BatchInvalid::SequenceOutOfRange)
        );
        assert_eq!(
            validate_batch(&[ok(FIRST_MARKER_SEQUENCE)], 10, false, 0),
            Err(BatchInvalid::SequenceOutOfRange)
        );
        assert_eq!(
            validate_batch(&[ok(2), ok(2)], 10, false, 0),
            Err(BatchInvalid::NotIncreasing)
        );
        assert_eq!(
            validate_batch(
                &[RecordShape {
                    sequence: 1,
                    level: None
                }],
                10,
                false,
                0
            ),
            Err(BatchInvalid::UnknownLevel)
        );
        assert_eq!(
            validate_batch(&[ok(5)], 10, true, 4),
            Err(BatchInvalid::HighestOutOfRange)
        );
        assert_eq!(
            validate_batch(&[], 0, true, FIRST_MARKER_SEQUENCE),
            Err(BatchInvalid::HighestOutOfRange)
        );
        assert_eq!(validate_batch(&[ok(5)], 10, true, 9), Ok(()));
    }

    #[test]
    fn a_resend_stores_nothing_and_a_gap_is_the_workers() {
        let first = plan_batch(
            &StreamCounters::new(),
            BIG,
            OPEN,
            &sized(&[1, 2, 3], 10),
            false,
            0,
        );
        assert_eq!(first.keep, vec![0, 1, 2]);
        let again = plan_batch(&first.after, BIG, OPEN, &sized(&[1, 2, 3], 10), false, 0);
        assert!(again.keep.is_empty());
        assert_eq!(again.after, first.after);
        let gap = plan_batch(&first.after, BIG, OPEN, &sized(&[6], 10), true, 8);
        assert_eq!(gap.keep, vec![0]);
        assert_eq!(gap.after.dropped_by_worker, 4, "4, 5, 7 and 8");
        assert_eq!(gap.after.last_sequence, 8);
        assert!(holds(&gap.after));
    }

    #[test]
    fn the_attempt_ceiling_keeps_the_first_lines_and_plans_one_marker() {
        let ceilings = AttemptCeilings {
            records: 2,
            bytes: 1 << 20,
        };
        let plan = plan_batch(
            &StreamCounters::new(),
            ceilings,
            OPEN,
            &sized(&[1, 2, 3, 4], 10),
            false,
            0,
        );
        assert_eq!(plan.keep, vec![0, 1]);
        assert_eq!(
            plan.marker,
            Some((FIRST_MARKER_SEQUENCE, MarkerReason::AttemptRecords))
        );
        assert_eq!(plan.verdict, PlanVerdict::FinalOnly);
        assert_eq!(plan.after.dropped_by_server, 2);
        assert_eq!(plan.after.ceiling_sequence, Some(4));
        // The worker stops sending; its empty final batch counts the rest as the server's.
        let last = plan_batch(&plan.after, ceilings, OPEN, &[], true, 10);
        assert!(last.marker.is_none(), "one marker per stream");
        assert_eq!(last.after.dropped_by_server, 8);
        assert_eq!(last.after.dropped_by_worker, 0);
        assert!(holds(&last.after));
    }

    #[test]
    fn the_byte_ceiling_is_met_by_the_record_that_does_not_fit() {
        let ceilings = AttemptCeilings {
            records: 100,
            bytes: 25,
        };
        let plan = plan_batch(
            &StreamCounters::new(),
            ceilings,
            OPEN,
            &sized(&[1, 2, 3], 10),
            false,
            0,
        );
        assert_eq!(plan.keep, vec![0, 1]);
        assert_eq!(
            plan.marker.map(|(_, reason)| reason),
            Some(MarkerReason::AttemptBytes)
        );
    }

    #[test]
    fn the_tenant_allowance_stores_nothing_and_counts_the_records() {
        let tight = TenantAllowance {
            remaining: Some(ROW_CHARGE * 2 + 19),
        };
        let plan = plan_batch(
            &StreamCounters::new(),
            BIG,
            tight,
            &sized(&[1, 2], 10),
            false,
            0,
        );
        assert!(plan.keep.is_empty());
        assert_eq!(plan.dropped_for_tenant, 2);
        assert_eq!(plan.after.ceiling, Some(CeilingReason::TenantQuota));
        assert_eq!(plan.verdict, PlanVerdict::FinalOnly);
        assert!(holds(&plan.after));
        let fits = TenantAllowance {
            remaining: Some(ROW_CHARGE * 2 + 20),
        };
        assert_eq!(
            plan_batch(
                &StreamCounters::new(),
                BIG,
                fits,
                &sized(&[1, 2], 10),
                false,
                0
            )
            .keep
            .len(),
            2
        );
    }

    #[test]
    fn the_planned_marker_is_charged_to_the_tenant() {
        let ceilings = AttemptCeilings {
            records: 1,
            bytes: 1 << 20,
        };
        // One kept row (10 + 128) and the marker (128): 266 bytes; 265 do not fit.
        let tight = TenantAllowance {
            remaining: Some(265),
        };
        let plan = plan_batch(
            &StreamCounters::new(),
            ceilings,
            tight,
            &sized(&[1, 2], 10),
            false,
            0,
        );
        assert!(plan.keep.is_empty() && plan.marker.is_none());
        assert_eq!(plan.after.ceiling, Some(CeilingReason::TenantQuota));
    }

    #[test]
    fn an_empty_batch_charges_nothing_at_the_tenant_limit() {
        let none_left = TenantAllowance { remaining: Some(0) };
        let plan = plan_batch(&StreamCounters::new(), BIG, none_left, &[], true, 0);
        assert_eq!(plan.after.ceiling, None);
        assert_eq!(plan.verdict, PlanVerdict::Continue);
    }

    #[test]
    fn a_late_batch_never_lowers_the_last_sequence() {
        let first = plan_batch(&StreamCounters::new(), BIG, OPEN, &sized(&[5], 1), false, 0);
        let late = plan_batch(&first.after, BIG, OPEN, &sized(&[2, 3], 1), false, 0);
        assert!(late.keep.is_empty());
        assert_eq!(late.after.last_sequence, 5);
        assert!(holds(&late.after));
    }

    #[test]
    fn drops_saturate_at_two_to_the_fifty_three() {
        let plan = plan_batch(
            &StreamCounters::new(),
            BIG,
            OPEN,
            &[],
            true,
            MAX_WORKER_SEQUENCE,
        );
        assert_eq!(plan.after.dropped_by_worker, DROP_SATURATION);
    }

    #[test]
    fn a_double_prints_as_ecmascript_number_to_string() {
        for (value, text) in [
            (12_345_678.0, "12345678"),
            (0.5, "0.5"),
            (-1.25, "-1.25"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1.5e-7, "1.5e-7"),
            (0.000_001, "0.000001"),
            (123.456, "123.456"),
            (-0.0, "0"),
            (5e-324, "5e-324"),
            (1.797_693_134_862_315_7e308, "1.7976931348623157e+308"),
        ] {
            assert_eq!(canonical_double(value), text, "{value}");
        }
        assert_eq!(
            FieldValue::double(f64::NAN),
            FieldValue::String("NaN".into())
        );
        assert_eq!(
            FieldValue::double(f64::NEG_INFINITY),
            FieldValue::String("-Infinity".into())
        );
    }

    #[test]
    fn fields_are_canonical_and_bounded() {
        let (json, dropped) = canonical_fields(&[
            ("b".into(), FieldValue::Int(-3)),
            ("a".into(), FieldValue::String("x\"\\\u{1}y".into())),
            ("a".into(), FieldValue::Bool(true)),
            ("c".into(), FieldValue::Double(2.0)),
        ]);
        assert_eq!(
            json.as_deref(),
            Some(r#"{"a":"x\"\\\u0001y","a~2":true,"b":-3,"c":2}"#)
        );
        assert!(!dropped);
        let many: Vec<(String, FieldValue)> = (0..40)
            .map(|n| (format!("k{n:02}"), FieldValue::Int(n)))
            .collect();
        let (json, dropped) = canonical_fields(&many);
        assert!(dropped);
        assert_eq!(json.unwrap().matches(':').count(), MAX_FIELDS);
        let wide: Vec<(String, FieldValue)> = (0..4)
            .map(|n| (format!("k{n}"), FieldValue::String("v".repeat(700))))
            .collect();
        let (json, dropped) = canonical_fields(&wide);
        assert!(dropped);
        assert!(json.unwrap().len() <= MAX_FIELDS_BYTES);
        assert_eq!(canonical_fields(&[]), (None, false));
    }

    #[test]
    fn the_debug_of_a_field_value_prints_no_value() {
        for value in [
            FieldValue::String("canary-text".into()),
            FieldValue::Int(918_273_645),
            FieldValue::Double(0.918_273_645),
            FieldValue::Bool(true),
        ] {
            let text = format!("{value:?}");
            assert!(
                !text.contains("canary") && !text.contains("918273645") && !text.contains("true"),
                "{text}"
            );
        }
    }

    #[test]
    fn many_colliding_keys_are_renamed_in_linear_time() {
        // 40,000 fields of one key fit in one 192 KiB batch; renaming them must not take quadratic or cubic time
        // (security review of 2026-11-19, Major 1).
        let fields: Vec<(String, FieldValue)> = vec![("k".into(), FieldValue::Bool(true)); 40_000];
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(canonical_fields(&fields));
        });
        let (json, dropped) = wait
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("canonical_fields finishes within 5 s");
        assert!(dropped);
        let json = json.expect("fields kept");
        assert_eq!(json.matches(':').count(), MAX_FIELDS);
        assert!(json.starts_with(r#"{"k":true,"k~10":true"#), "{json}");
    }

    #[test]
    fn text_is_cut_at_a_character_boundary() {
        // One ASCII byte first, so that byte 8,192 falls inside a two-byte character and the cut must step back.
        let text = format!("a{}", "é".repeat(5000));
        let (kept, cut_it) = cut(&text, MAX_TEXT_BYTES);
        assert!(cut_it);
        assert_eq!(kept.len(), MAX_TEXT_BYTES - 1);
        assert!(kept.ends_with('é'));
        assert_eq!(normalise("a\0b"), "a\u{FFFD}b");
    }

    #[derive(Debug, Clone)]
    enum Step {
        Batch {
            sequences: Vec<u64>,
            bytes: u64,
            final_batch: bool,
            highest_extra: u64,
        },
        Refused,
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            (
                proptest::collection::btree_set(1u64..400, 0..20),
                0u64..300,
                any::<bool>(),
                0u64..30
            )
                .prop_map(|(set, bytes, final_batch, highest_extra)| Step::Batch {
                    sequences: set.into_iter().collect(),
                    bytes,
                    final_batch,
                    highest_extra,
                }),
            Just(Step::Refused),
        ]
    }

    proptest! {
        /// The counting rule of design section 2: over any sequence of plans and refusals, lines
        /// stored (markers excluded) + dropped by the worker + dropped by the server = last_sequence.
        #[test]
        fn stored_plus_dropped_is_the_last_sequence(
            steps in proptest::collection::vec(step(), 1..30),
            records in 1u64..40,
            bytes in 1u64..4_000,
            remaining in proptest::option::of(0u64..6_000),
        ) {
            let ceilings = AttemptCeilings { records, bytes };
            let mut tenant_left = remaining;
            let mut stream = StreamCounters::new();
            let mut markers = 0;
            let mut inserted = 0u64;
            for step in steps {
                let Step::Batch { sequences, bytes, final_batch, highest_extra } = step else {
                    continue;
                };
                let batch = sized(&sequences, bytes);
                let highest = sequences.last().copied().unwrap_or(0) + highest_extra;
                let plan = plan_batch(
                    &stream,
                    ceilings,
                    TenantAllowance { remaining: tenant_left },
                    &batch,
                    final_batch,
                    highest,
                );
                let charged: u64 = plan.keep.iter().map(|&i| batch[i].text_bytes + ROW_CHARGE).sum::<u64>()
                    + ROW_CHARGE * u64::from(plan.marker.is_some());
                tenant_left = tenant_left.map(|left| left - charged);
                markers += u64::from(plan.marker.is_some());
                inserted += plan.keep.len() as u64;
                prop_assert!(plan.after.last_sequence >= stream.last_sequence);
                prop_assert!(plan.after.records <= ceilings.records);
                prop_assert!(plan.after.bytes <= ceilings.bytes);
                stream = plan.after;
                // The rows the insert stores are the ones the counters count.
                prop_assert_eq!(inserted, stream.records);
                prop_assert_eq!(
                    inserted + stream.dropped_by_worker + stream.dropped_by_server,
                    stream.last_sequence
                );
            }
            prop_assert!(markers <= 1);
        }
    }
}

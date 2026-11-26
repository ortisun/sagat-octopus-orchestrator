//! Step logs (feature 041, ADR-0084): the lines an activity writes to the SDK's logger, stored per
//! attempt beside the journal, never execution state.
//!
//! This module holds the closed sets of `specs/041-step-logs/data-model.md` §1, the bounds of a batch
//! (`validate_batch`, design section 2, P4) and of a carry (`validate_carry`, section 2a, C2), the canonical form of
//! a line's fields (data-model.md §5) and `plan_batch`, the pure rule that removes duplicates, counts gaps, applies the
//! attempt and tenant allowances, keeps a carry's order guard and decides the marker, the drops by reason
//! (`DropReason`) and the verdict (T5 and T6), for a batch and for a carry. The counting rule it keeps,
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
    /// Every reason, in the order of the contract's `StepLogStream.ceiling` enum.
    pub const ALL: [Self; 3] = [Self::AttemptRecords, Self::AttemptBytes, Self::TenantQuota];

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
        Self::ALL.into_iter().find(|reason| reason.as_str() == name)
    }

    const fn marker(self) -> Option<MarkerReason> {
        match self {
            Self::AttemptRecords => Some(MarkerReason::AttemptRecords),
            Self::AttemptBytes => Some(MarkerReason::AttemptBytes),
            Self::TenantQuota => None,
        }
    }
}

/// Why the server did not store some of a carry's records for a reason that is not a ceiling (design 041 section 2b,
/// decision D041-129): the stream's `notStoredReason`, never its `ceiling`. Produced only by the carry's SQL, never by
/// `plan_batch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotStoredReason {
    /// The tenant's step-log quota could not be checked within the fallback refill's budget or with no wait slot free:
    /// the log service was busy, not the tenant's quota (gap G041-026).
    ServiceBusy,
}

impl NotStoredReason {
    /// Every reason, in the order of the contract's `StepLogStream.notStoredReason` enum.
    pub const ALL: [Self; 1] = [Self::ServiceBusy];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ServiceBusy => "service_busy",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.as_str() == name)
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

/// Why records of a stream were not stored (decision D041-128): the label of `step_log_lines_dropped_total` and the keys
/// of a carry's drops. `WorkerGap` is the worker's side; every other reason is the server's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropReason {
    /// Sequences the worker assigned and never sent (a batch it gave up on).
    WorkerGap,
    AttemptRecords,
    AttemptBytes,
    TenantQuota,
    /// The fallback refill passed its budget or found no wait slot free (decision D041-129).
    ServiceBusy,
    /// A carry whose transaction began before a batch of its attempt that was stored after it (D041-132).
    OutOfOrder,
    /// A carry that failed C2 but whose `highest_sequence` is valid.
    Invalid,
    /// A report whose output or failure code held a declared secret.
    Secret,
    /// A report recorded by a content-quota fallback.
    ContentFallback,
    /// The re-run of a report after a guard's raise during its carry (ADR-0095 decision 5).
    Failed,
}

impl DropReason {
    /// Every reason, in the order of data-model.md §1.
    pub const ALL: [Self; 10] = [
        Self::WorkerGap,
        Self::AttemptRecords,
        Self::AttemptBytes,
        Self::TenantQuota,
        Self::ServiceBusy,
        Self::OutOfOrder,
        Self::Invalid,
        Self::Secret,
        Self::ContentFallback,
        Self::Failed,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkerGap => "worker_gap",
            Self::AttemptRecords => "attempt_records",
            Self::AttemptBytes => "attempt_bytes",
            Self::TenantQuota => "tenant_quota",
            Self::ServiceBusy => "service_busy",
            Self::OutOfOrder => "out_of_order",
            Self::Invalid => "invalid",
            Self::Secret => "secret",
            Self::ContentFallback => "content_fallback",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.as_str() == name)
    }

    const fn index(self) -> usize {
        self as usize
    }
}

impl From<CeilingReason> for DropReason {
    fn from(reason: CeilingReason) -> Self {
        match reason {
            CeilingReason::AttemptRecords => Self::AttemptRecords,
            CeilingReason::AttemptBytes => Self::AttemptBytes,
            CeilingReason::TenantQuota => Self::TenantQuota,
        }
    }
}

/// Why a carry stores no record (design section 2a): the application plans the first four, the carry's own block
/// answers the last two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkOnlyReason {
    Invalid,
    Secret,
    ContentFallback,
    Failed,
    /// The charge was refused at the tenant's step-log limit.
    TenantQuota,
    /// The fallback refill passed its budget or found no wait slot free.
    ServiceBusy,
}

impl MarkOnlyReason {
    /// The reasons the application may plan, the values `p_mark_reason` may take (data-model.md §8a, item 0).
    pub const PLANNED: [Self; 4] = [
        Self::Invalid,
        Self::Secret,
        Self::ContentFallback,
        Self::Failed,
    ];

    #[must_use]
    pub const fn drop_reason(self) -> DropReason {
        match self {
            Self::Invalid => DropReason::Invalid,
            Self::Secret => DropReason::Secret,
            Self::ContentFallback => DropReason::ContentFallback,
            Self::Failed => DropReason::Failed,
            Self::TenantQuota => DropReason::TenantQuota,
            Self::ServiceBusy => DropReason::ServiceBusy,
        }
    }
}

/// Sequences one plan adds to a stream's drop counts, per reason, before saturation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Drops([u64; DropReason::ALL.len()]);

impl Drops {
    #[must_use]
    pub const fn get(&self, reason: DropReason) -> u64 {
        self.0[reason.index()]
    }

    /// The worker's side: `WorkerGap`.
    #[must_use]
    pub const fn by_worker(&self) -> u64 {
        self.get(DropReason::WorkerGap)
    }

    /// The server's side: every other reason.
    #[must_use]
    pub fn by_server(&self) -> u64 {
        self.0.iter().sum::<u64>() - self.by_worker()
    }

    /// The reasons with a count above zero, in the order of [`DropReason::ALL`].
    pub fn iter(&self) -> impl Iterator<Item = (DropReason, u64)> + '_ {
        DropReason::ALL
            .into_iter()
            .map(|reason| (reason, self.get(reason)))
            .filter(|(_, count)| *count > 0)
    }

    fn add(&mut self, reason: DropReason, count: u64) {
        self.0[reason.index()] += count;
    }
}

/// A batch outside the bounds of P4 (`INVALID_ARGUMENT`). Carries no value of the batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BatchInvalid {
    #[error("a step-log batch holds no record")]
    Empty,
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
}

/// What P4 needs of one record. `level` is `None` when the wire value names no level (the adapter maps
/// the protobuf enumeration; an unspecified or unknown value is `None`).
#[derive(Debug, Clone, Copy)]
pub struct RecordShape {
    pub sequence: u64,
    pub level: Option<StepLogLevel>,
}

fn check_records(records: &[RecordShape]) -> Result<u64, BatchInvalid> {
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
    Ok(previous)
}

/// P4: 1 to 500 records and at most 192 KiB encoded, sequences strictly increasing in 1 to 2^62 − 1 and a known level. A
/// batch carries no final mark: the attempt's report does (decision D041-090).
///
/// # Errors
/// [`BatchInvalid`] naming the first bound that does not hold.
pub fn validate_batch(records: &[RecordShape], encoded_len: usize) -> Result<(), BatchInvalid> {
    if records.is_empty() {
        return Err(BatchInvalid::Empty);
    }
    if records.len() > MAX_BATCH_RECORDS {
        return Err(BatchInvalid::TooManyRecords);
    }
    if encoded_len > MAX_BATCH_BYTES {
        return Err(BatchInvalid::TooLarge);
    }
    check_records(records).map(|_| ())
}

/// At most this many records in one carry (design section 2a, C2).
pub const MAX_CARRY_RECORDS: usize = 64;
/// At most this many encoded bytes in one carry (16 KiB).
pub const MAX_CARRY_BYTES: usize = 16 * 1024;

/// What C2 makes of a carry (design section 2a). A carry never makes a report fail (FR-041-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarryShape {
    /// Within its bounds: planned as a full carry.
    Full,
    /// Outside its bounds, but its `highest_sequence` alone is valid: a mark-only carry whose records are counted
    /// dropped by the server (`invalid`).
    MarkOnly,
    /// Outside its bounds and its `highest_sequence` too: ignored (`invalid`), the report recorded without it.
    Ignored,
}

/// C2: at most 64 records and 16 KiB encoded, sequences strictly increasing in 1 to 2^62 − 1, a known level, and a
/// `highest_sequence` below 2^62 and at least the last record's sequence.
#[must_use]
pub fn validate_carry(
    records: &[RecordShape],
    encoded_len: usize,
    highest_sequence: u64,
) -> CarryShape {
    let highest_valid = highest_sequence <= MAX_WORKER_SEQUENCE;
    let full = records.len() <= MAX_CARRY_RECORDS
        && encoded_len <= MAX_CARRY_BYTES
        && check_records(records).is_ok_and(|last| highest_sequence >= last);
    match (full && highest_valid, highest_valid) {
        (true, _) => CarryShape::Full,
        (false, true) => CarryShape::MarkOnly,
        (false, false) => CarryShape::Ignored,
    }
}

/// What the stream row holds when a batch reads it (T4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamCounters {
    pub records: u64,
    pub bytes: u64,
    pub last_sequence: u64,
    pub ceiling_sequence: Option<u64>,
    pub ceiling: Option<CeilingReason>,
    /// Why some records were not stored for a reason that is not a ceiling (decision D041-129).
    pub not_stored_reason: Option<NotStoredReason>,
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
            not_stored_reason: None,
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
    /// A ceiling was met: send no more records; the report still carries the final mark.
    FinalOnly,
}

/// The final mark a carry brings to the plan (design section 2a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CarryMark {
    /// The highest sequence the attempt's logger assigned.
    pub highest_sequence: u64,
    /// No batch of the attempt stored after this transaction began (`last_stored_xid` null or below its own): when
    /// false, no record of the carry is kept and each fresh one is counted `out_of_order`.
    pub in_order: bool,
    /// A mark-only carry: no record kept, no marker, each sequence it sent counted with this reason.
    pub mark_only: Option<MarkOnlyReason>,
}

/// The plan of one batch or carry: what to insert and how the counters move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The indices, in the batch, of the records to insert, in sequence order.
    pub keep: Vec<usize>,
    /// A ceiling marker to insert, at its server sequence.
    pub marker: Option<(u64, MarkerReason)>,
    /// The counters after the batch, assuming every kept record and the marker are inserted.
    pub after: StreamCounters,
    pub verdict: PlanVerdict,
    /// Sequences this plan adds to the drop counts, per reason, before saturation.
    pub drops: Drops,
}

const fn saturating_drop(count: u64, add: u64) -> u64 {
    let sum = count.saturating_add(add);
    if sum > DROP_SATURATION {
        DROP_SATURATION
    } else {
        sum
    }
}

/// T5 and T6, for a batch (`carry` `None`) and for a carry. Duplicates (records at or below `last_sequence`) are
/// dropped and not counted; the sequences between `last_sequence` and the high mark (a carry's `highest_sequence`
/// included) that it does not hold are a gap, counted as dropped by the server above a ceiling already met, with that
/// ceiling's reason, and by the worker (`worker_gap`) otherwise. A full carry out of order keeps nothing and counts its
/// fresh records `out_of_order`. Otherwise records are kept in sequence order until the first that does not fit the
/// attempt ceilings, the rest dropped by the server with that ceiling's reason, with one marker when an attempt ceiling
/// is first met; and if what is kept (128 bytes a row, the marker included) does not fit the tenant's allowance,
/// nothing is kept and every fresh record is counted `tenant_quota`. A mark-only carry keeps nothing, plans no marker
/// and counts each distinct sequence it sent in (`last_sequence`, `highest_sequence`] with its reason; `tenant_quota`
/// sets the ceiling if none is met, `service_busy` the stream's `not_stored_reason`. A batch's records and a full
/// carry's must have passed [`validate_batch`] or [`validate_carry`]; a mark-only carry's may be in any order.
#[must_use]
pub fn plan_batch(
    stream: &StreamCounters,
    ceilings: AttemptCeilings,
    tenant: TenantAllowance,
    records: &[SizedRecord],
    carry: Option<CarryMark>,
) -> Plan {
    let mark_only = carry.and_then(|carry| carry.mark_only);
    let fresh: Vec<(usize, &SizedRecord)> = if mark_only.is_some() {
        let highest = carry.map_or(0, |carry| carry.highest_sequence);
        let mut seen = HashSet::new();
        let mut distinct: Vec<(usize, &SizedRecord)> = records
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                record.sequence > stream.last_sequence && record.sequence <= highest
            })
            .filter(|(_, record)| seen.insert(record.sequence))
            .collect();
        distinct.sort_by_key(|(_, record)| record.sequence);
        distinct
    } else {
        records
            .iter()
            .enumerate()
            .filter(|(_, record)| record.sequence > stream.last_sequence)
            .collect()
    };
    let batch_mark = fresh.last().map_or(0, |(_, record)| record.sequence);
    let mark = batch_mark
        .max(carry.map_or(0, |carry| carry.highest_sequence))
        .max(stream.last_sequence);
    let sent = fresh.len() as u64;
    // Saturating: the records passed validate_batch or validate_carry, so this never saturates; an unchecked input
    // gives a wrong plan, never a wrapped count.
    let gap = mark
        .saturating_sub(stream.last_sequence)
        .saturating_sub(sent);
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
    let mut drops = Drops::default();
    drops.add(DropReason::WorkerGap, gap_worker);
    if gap_server > 0 {
        // A gap above the ceiling exists only once a ceiling is met.
        drops.add(
            stream
                .ceiling
                .map_or(DropReason::TenantQuota, DropReason::from),
            gap_server,
        );
    }
    let mut keep = Vec::new();
    let (mut records_after, mut bytes_after) = (stream.records, stream.bytes);
    let mut met = stream.ceiling;
    let mut first_met = None;
    let mut not_stored_reason = stream.not_stored_reason;
    let mut marker = None;
    let mut truncated = 0;
    if let Some(reason) = mark_only {
        drops.add(reason.drop_reason(), sent);
        match reason {
            MarkOnlyReason::TenantQuota if met.is_none() => {
                met = Some(CeilingReason::TenantQuota);
                first_met = met;
            }
            MarkOnlyReason::ServiceBusy => not_stored_reason = Some(NotStoredReason::ServiceBusy),
            _ => {}
        }
    } else if carry.is_some_and(|carry| !carry.in_order) {
        drops.add(DropReason::OutOfOrder, sent);
    } else {
        let mut kept_bytes = 0;
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
        marker = first_met
            .and_then(CeilingReason::marker)
            .map(|reason| (stream.next_marker_sequence, reason));
        let charge = kept_bytes + ROW_CHARGE * (keep.len() as u64 + u64::from(marker.is_some()));
        let over_tenant = tenant
            .remaining
            .is_some_and(|remaining| charge > 0 && charge > remaining);
        if over_tenant {
            keep.clear();
            marker = None;
            records_after = stream.records;
            bytes_after = stream.bytes;
            truncated = 0;
            drops.add(DropReason::TenantQuota, sent);
            if met.is_none() || first_met.is_some() {
                met = Some(CeilingReason::TenantQuota);
                first_met = met;
            }
        } else {
            let unkept = sent - keep.len() as u64;
            if unkept > 0 {
                // The records after the first that did not fit: the ceiling met before or by this plan.
                drops.add(
                    met.map_or(DropReason::TenantQuota, DropReason::from),
                    unkept,
                );
            }
        }
    }
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
        not_stored_reason,
        dropped_by_worker: saturating_drop(stream.dropped_by_worker, drops.by_worker()),
        dropped_by_server: saturating_drop(stream.dropped_by_server, drops.by_server()),
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
        drops,
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

    fn fin(highest_sequence: u64) -> CarryMark {
        CarryMark {
            highest_sequence,
            in_order: true,
            mark_only: None,
        }
    }

    fn ok(sequence: u64) -> RecordShape {
        RecordShape {
            sequence,
            level: Some(StepLogLevel::Info),
        }
    }

    #[test]
    fn a_batch_is_refused_outside_its_bounds() {
        assert_eq!(validate_batch(&[ok(1), ok(2)], 10), Ok(()));
        assert_eq!(
            validate_batch(&[], 0),
            Err(BatchInvalid::Empty),
            "a batch holds 1 to 500 records"
        );
        assert_eq!(
            validate_batch(&vec![ok(1); 501], 10),
            Err(BatchInvalid::TooManyRecords)
        );
        let five_hundred: Vec<RecordShape> = (1..=500).map(ok).collect();
        assert_eq!(validate_batch(&five_hundred, MAX_BATCH_BYTES), Ok(()));
        assert_eq!(
            validate_batch(&[ok(1)], MAX_BATCH_BYTES + 1),
            Err(BatchInvalid::TooLarge)
        );
        assert_eq!(
            validate_batch(&[ok(0)], 10),
            Err(BatchInvalid::SequenceOutOfRange)
        );
        assert_eq!(
            validate_batch(&[ok(FIRST_MARKER_SEQUENCE)], 10),
            Err(BatchInvalid::SequenceOutOfRange)
        );
        assert_eq!(validate_batch(&[ok(MAX_WORKER_SEQUENCE)], 10), Ok(()));
        assert_eq!(
            validate_batch(&[ok(2), ok(2)], 10),
            Err(BatchInvalid::NotIncreasing)
        );
        assert_eq!(
            validate_batch(
                &[RecordShape {
                    sequence: 1,
                    level: None
                }],
                10
            ),
            Err(BatchInvalid::UnknownLevel)
        );
    }

    #[test]
    fn a_carry_outside_its_bounds_is_mark_only_or_ignored() {
        let sixty_four: Vec<RecordShape> = (1..=64).map(ok).collect();
        let sixty_five: Vec<RecordShape> = (1..=65).map(ok).collect();
        assert_eq!(
            validate_carry(&sixty_four, MAX_CARRY_BYTES, 64),
            CarryShape::Full
        );
        assert_eq!(validate_carry(&sixty_five, 100, 65), CarryShape::MarkOnly);
        assert_eq!(
            validate_carry(&[ok(1)], MAX_CARRY_BYTES + 1, 1),
            CarryShape::MarkOnly
        );
        assert_eq!(
            validate_carry(&[], 0, 0),
            CarryShape::Full,
            "an attempt that logged nothing"
        );
        assert_eq!(validate_carry(&[ok(5)], 10, 9), CarryShape::Full);
        assert_eq!(
            validate_carry(&[ok(5)], 10, 4),
            CarryShape::MarkOnly,
            "highest below the last record"
        );
        assert_eq!(validate_carry(&[ok(2), ok(2)], 10, 2), CarryShape::MarkOnly);
        assert_eq!(validate_carry(&[ok(0)], 10, 3), CarryShape::MarkOnly);
        assert_eq!(
            validate_carry(
                &[RecordShape {
                    sequence: 1,
                    level: None
                }],
                10,
                1
            ),
            CarryShape::MarkOnly
        );
        assert_eq!(
            validate_carry(&[], 0, MAX_WORKER_SEQUENCE),
            CarryShape::Full
        );
        assert_eq!(
            validate_carry(&[], 0, FIRST_MARKER_SEQUENCE),
            CarryShape::Ignored
        );
        assert_eq!(
            validate_carry(&[ok(1)], 10, FIRST_MARKER_SEQUENCE),
            CarryShape::Ignored
        );
        assert_eq!(
            validate_carry(&sixty_five, 100, FIRST_MARKER_SEQUENCE),
            CarryShape::Ignored
        );
    }

    #[test]
    fn drop_reasons_are_one_closed_set() {
        let names: Vec<&str> = DropReason::ALL
            .iter()
            .map(|reason| reason.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "worker_gap",
                "attempt_records",
                "attempt_bytes",
                "tenant_quota",
                "service_busy",
                "out_of_order",
                "invalid",
                "secret",
                "content_fallback",
                "failed"
            ]
        );
        for reason in DropReason::ALL {
            assert_eq!(DropReason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(DropReason::parse("busy"), None);
        for ceiling in CeilingReason::ALL {
            assert_eq!(DropReason::from(ceiling).as_str(), ceiling.as_str());
        }
        let planned: Vec<&str> = MarkOnlyReason::PLANNED
            .iter()
            .map(|r| r.drop_reason().as_str())
            .collect();
        assert_eq!(planned, ["invalid", "secret", "content_fallback", "failed"]);
        assert_eq!(
            MarkOnlyReason::TenantQuota.drop_reason(),
            DropReason::TenantQuota
        );
        assert_eq!(
            MarkOnlyReason::ServiceBusy.drop_reason(),
            DropReason::ServiceBusy
        );
    }

    #[test]
    fn a_carry_out_of_order_keeps_nothing_and_records_its_mark() {
        let first = plan_batch(&StreamCounters::new(), BIG, OPEN, &sized(&[1, 2], 10), None);
        let carry = CarryMark {
            highest_sequence: 6,
            in_order: false,
            mark_only: None,
        };
        let plan = plan_batch(&first.after, BIG, OPEN, &sized(&[2, 3, 4], 10), Some(carry));
        assert!(plan.keep.is_empty() && plan.marker.is_none());
        assert_eq!(
            plan.drops.get(DropReason::OutOfOrder),
            2,
            "3 and 4; 2 is a duplicate"
        );
        assert_eq!(plan.drops.by_worker(), 2, "5 and 6");
        assert_eq!(plan.after.last_sequence, 6);
        assert_eq!(plan.after.records, 2);
        assert_eq!(plan.after.ceiling, None);
        assert!(holds(&plan.after));
        let in_order = plan_batch(&first.after, BIG, OPEN, &sized(&[3, 4], 10), Some(fin(6)));
        assert_eq!(in_order.keep, vec![0, 1]);
        assert_eq!(in_order.drops.get(DropReason::OutOfOrder), 0);
    }

    #[test]
    fn a_mark_only_carry_counts_each_sent_sequence_once_with_its_reason() {
        let first = plan_batch(&StreamCounters::new(), BIG, OPEN, &sized(&[1, 2], 10), None);
        // An invalid carry: a duplicate (4 twice), one at or below last_sequence (2) and one above highest (9).
        let records = sized(&[4, 2, 4, 3, 9], 10);
        for reason in MarkOnlyReason::PLANNED {
            let carry = CarryMark {
                highest_sequence: 6,
                in_order: true,
                mark_only: Some(reason),
            };
            let plan = plan_batch(&first.after, BIG, OPEN, &records, Some(carry));
            assert!(plan.keep.is_empty() && plan.marker.is_none());
            assert_eq!(plan.drops.get(reason.drop_reason()), 2, "3 and 4");
            assert_eq!(plan.drops.by_worker(), 2, "5 and 6");
            assert_eq!(plan.drops.by_server(), 2);
            assert_eq!(plan.after.last_sequence, 6);
            assert_eq!(plan.after.ceiling, None);
            assert_eq!(plan.after.not_stored_reason, None);
            assert!(holds(&plan.after));
        }
        let quota = CarryMark {
            highest_sequence: 6,
            in_order: true,
            mark_only: Some(MarkOnlyReason::TenantQuota),
        };
        let plan = plan_batch(&first.after, BIG, OPEN, &records, Some(quota));
        assert_eq!(plan.after.ceiling, Some(CeilingReason::TenantQuota));
        assert_eq!(plan.after.ceiling_sequence, Some(6));
        assert_eq!(plan.drops.get(DropReason::TenantQuota), 2);
        let busy = CarryMark {
            highest_sequence: 6,
            in_order: true,
            mark_only: Some(MarkOnlyReason::ServiceBusy),
        };
        let plan = plan_batch(&first.after, BIG, OPEN, &records, Some(busy));
        assert_eq!(
            plan.after.ceiling, None,
            "the busy loss is not a ceiling (D041-129)"
        );
        assert_eq!(plan.after.ceiling_sequence, None);
        assert_eq!(
            plan.after.not_stored_reason,
            Some(NotStoredReason::ServiceBusy)
        );
        assert_eq!(plan.drops.get(DropReason::ServiceBusy), 2);
        assert_eq!(plan.verdict, PlanVerdict::Continue);
        assert!(holds(&plan.after));
    }

    #[test]
    fn a_mark_only_carry_after_a_ceiling_splits_its_gap_as_t5_does() {
        let ceilings = AttemptCeilings {
            records: 2,
            bytes: 1 << 20,
        };
        let met = plan_batch(
            &StreamCounters::new(),
            ceilings,
            OPEN,
            &sized(&[1, 2, 3, 4], 10),
            None,
        );
        assert_eq!(met.after.ceiling_sequence, Some(4));
        let secret = CarryMark {
            highest_sequence: 10,
            in_order: true,
            mark_only: Some(MarkOnlyReason::Secret),
        };
        let plan = plan_batch(&met.after, ceilings, OPEN, &[], Some(secret));
        assert_eq!(
            plan.drops.get(DropReason::AttemptRecords),
            6,
            "5 to 10, above the ceiling"
        );
        assert_eq!(plan.drops.by_worker(), 0);
        assert_eq!(plan.drops.get(DropReason::Secret), 0, "it sent no record");
        assert!(holds(&plan.after));
    }

    #[test]
    fn a_resend_stores_nothing_and_a_gap_is_the_workers() {
        let first = plan_batch(
            &StreamCounters::new(),
            BIG,
            OPEN,
            &sized(&[1, 2, 3], 10),
            None,
        );
        assert_eq!(first.keep, vec![0, 1, 2]);
        let again = plan_batch(&first.after, BIG, OPEN, &sized(&[1, 2, 3], 10), None);
        assert!(again.keep.is_empty());
        assert_eq!(again.after, first.after);
        let gap = plan_batch(&first.after, BIG, OPEN, &sized(&[6], 10), Some(fin(8)));
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
            None,
        );
        assert_eq!(plan.keep, vec![0, 1]);
        assert_eq!(
            plan.marker,
            Some((FIRST_MARKER_SEQUENCE, MarkerReason::AttemptRecords))
        );
        assert_eq!(plan.verdict, PlanVerdict::FinalOnly);
        assert_eq!(plan.after.dropped_by_server, 2);
        assert_eq!(plan.drops.get(DropReason::AttemptRecords), 2);
        assert_eq!(plan.after.ceiling_sequence, Some(4));
        // The worker stops sending; its report's carry, with no record, counts the rest as the server's.
        let last = plan_batch(&plan.after, ceilings, OPEN, &[], Some(fin(10)));
        assert!(last.marker.is_none(), "one marker per stream");
        assert_eq!(last.after.dropped_by_server, 8);
        assert_eq!(
            last.drops.get(DropReason::AttemptRecords),
            6,
            "5 to 10, above the ceiling"
        );
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
            None,
        );
        assert_eq!(plan.keep, vec![0, 1]);
        assert_eq!(
            plan.marker.map(|(_, reason)| reason),
            Some(MarkerReason::AttemptBytes)
        );
        assert_eq!(plan.drops.get(DropReason::AttemptBytes), 1);
        assert_eq!(plan.drops.by_server(), 1);
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
            None,
        );
        assert!(plan.keep.is_empty());
        assert_eq!(plan.drops.get(DropReason::TenantQuota), 2);
        assert_eq!(plan.drops.by_server(), 2);
        assert_eq!(plan.after.ceiling, Some(CeilingReason::TenantQuota));
        assert_eq!(plan.verdict, PlanVerdict::FinalOnly);
        assert!(holds(&plan.after));
        let fits = TenantAllowance {
            remaining: Some(ROW_CHARGE * 2 + 20),
        };
        assert_eq!(
            plan_batch(&StreamCounters::new(), BIG, fits, &sized(&[1, 2], 10), None)
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
            None,
        );
        assert!(plan.keep.is_empty() && plan.marker.is_none());
        assert_eq!(plan.after.ceiling, Some(CeilingReason::TenantQuota));
    }

    #[test]
    fn an_empty_batch_charges_nothing_at_the_tenant_limit() {
        let none_left = TenantAllowance { remaining: Some(0) };
        let plan = plan_batch(&StreamCounters::new(), BIG, none_left, &[], Some(fin(0)));
        assert_eq!(plan.after.ceiling, None);
        assert_eq!(plan.verdict, PlanVerdict::Continue);
    }

    #[test]
    fn a_late_batch_never_lowers_the_last_sequence() {
        let first = plan_batch(&StreamCounters::new(), BIG, OPEN, &sized(&[5], 1), None);
        let late = plan_batch(&first.after, BIG, OPEN, &sized(&[2, 3], 1), None);
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
            Some(fin(MAX_WORKER_SEQUENCE)),
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
        },
        /// The attempt's report: its carry ends the stream.
        Carry {
            sequences: Vec<u64>,
            bytes: u64,
            highest_extra: u64,
            in_order: bool,
            mark_only: Option<MarkOnlyReason>,
        },
        Refused,
    }

    const MARK_ONLY: [MarkOnlyReason; 6] = [
        MarkOnlyReason::Invalid,
        MarkOnlyReason::Secret,
        MarkOnlyReason::ContentFallback,
        MarkOnlyReason::Failed,
        MarkOnlyReason::TenantQuota,
        MarkOnlyReason::ServiceBusy,
    ];

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            4 => (proptest::collection::btree_set(1u64..400, 1..20), 0u64..300)
                .prop_map(|(set, bytes)| Step::Batch { sequences: set.into_iter().collect(), bytes }),
            1 => (
                proptest::collection::btree_set(1u64..400, 0..20),
                0u64..300,
                0u64..30,
                any::<bool>(),
            )
                .prop_map(|(set, bytes, highest_extra, in_order)| Step::Carry {
                    sequences: set.into_iter().collect(),
                    bytes,
                    highest_extra,
                    in_order,
                    mark_only: None,
                }),
            // A mark-only carry's records may repeat, be out of order or lie outside its range.
            1 => (
                proptest::collection::vec(1u64..400, 0..20),
                0u64..300,
                proptest::sample::select(MARK_ONLY.to_vec()),
            )
                .prop_map(|(sequences, highest_extra, reason)| Step::Carry {
                    sequences,
                    bytes: 1,
                    highest_extra,
                    in_order: true,
                    mark_only: Some(reason),
                }),
            1 => Just(Step::Refused),
        ]
    }

    proptest! {
        /// The counting rule of design section 2: over any sequence of batches, carries, `in_order` values and
        /// refusals, lines stored (markers excluded) + dropped by the worker + dropped by the server = last_sequence,
        /// and every server drop carries a reason the rule names for that plan.
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
            let mut by_reason = [0u64; DropReason::ALL.len()];
            for step in steps {
                let (sequences, bytes, carry) = match step {
                    Step::Refused => continue,
                    Step::Batch { sequences, bytes } => (sequences, bytes, None),
                    Step::Carry { sequences, bytes, highest_extra, in_order, mark_only } => {
                        // A full carry's highest is at least its last record's (C2); a mark-only carry's records
                        // may lie above it.
                        let highest = if mark_only.is_some() {
                            stream.last_sequence + highest_extra
                        } else {
                            sequences.iter().copied().max().unwrap_or(0).max(stream.last_sequence) + highest_extra
                        };
                        (sequences, bytes, Some(CarryMark { highest_sequence: highest, in_order, mark_only }))
                    }
                };
                let batch = sized(&sequences, bytes);
                let plan = plan_batch(&stream, ceilings, TenantAllowance { remaining: tenant_left }, &batch, carry);
                let charged: u64 = plan.keep.iter().map(|&i| batch[i].text_bytes + ROW_CHARGE).sum::<u64>()
                    + ROW_CHARGE * u64::from(plan.marker.is_some());
                tenant_left = tenant_left.map(|left| left - charged);
                markers += u64::from(plan.marker.is_some());
                inserted += plan.keep.len() as u64;
                for (reason, count) in plan.drops.iter() {
                    by_reason[reason.index()] += count;
                }
                prop_assert!(plan.after.last_sequence >= stream.last_sequence);
                prop_assert!(plan.after.records <= ceilings.records);
                prop_assert!(plan.after.bytes <= ceilings.bytes);
                if let Some(carry) = carry {
                    prop_assert_eq!(plan.after.last_sequence, carry.highest_sequence.max(stream.last_sequence));
                    if carry.mark_only.is_some() || !carry.in_order {
                        prop_assert!(plan.keep.is_empty() && plan.marker.is_none());
                    }
                    let fresh = sequences.iter().filter(|&&s| s > stream.last_sequence).count() as u64;
                    if carry.mark_only.is_none() && !carry.in_order {
                        prop_assert_eq!(plan.drops.get(DropReason::OutOfOrder), fresh);
                    }
                    if let Some(reason) = carry.mark_only {
                        for (other, count) in plan.drops.iter() {
                            let allowed = other == reason.drop_reason()
                                || other == DropReason::WorkerGap
                                || stream.ceiling.is_some_and(|c| DropReason::from(c) == other);
                            prop_assert!(allowed, "{other:?} {count} in a {reason:?} carry");
                        }
                    }
                } else {
                    prop_assert_eq!(plan.drops.get(DropReason::OutOfOrder), 0);
                }
                // Each server reason is one the rule names: a ceiling met before or by this plan, the tenant quota,
                // out of order for an out-of-order full carry, or the mark-only carry's own reason.
                for (reason, count) in plan.drops.iter() {
                    let ceiling = |c: Option<CeilingReason>| c.is_some_and(|c| DropReason::from(c) == reason);
                    let named = reason == DropReason::WorkerGap
                        || ceiling(stream.ceiling)
                        || ceiling(plan.after.ceiling)
                        || (reason == DropReason::TenantQuota && carry.is_none_or(|c| c.mark_only.is_none()))
                        || (reason == DropReason::OutOfOrder
                            && carry.is_some_and(|c| !c.in_order && c.mark_only.is_none()))
                        || carry.and_then(|c| c.mark_only).is_some_and(|r| r.drop_reason() == reason);
                    prop_assert!(named, "{reason:?} {count}");
                }
                if stream.not_stored_reason.is_none() && plan.after.not_stored_reason.is_some() {
                    prop_assert_eq!(carry.and_then(|c| c.mark_only), Some(MarkOnlyReason::ServiceBusy));
                }
                stream = plan.after;
                // The rows the insert stores are the ones the counters count.
                prop_assert_eq!(inserted, stream.records);
                prop_assert_eq!(
                    inserted + stream.dropped_by_worker + stream.dropped_by_server,
                    stream.last_sequence
                );
                prop_assert_eq!(by_reason[DropReason::WorkerGap.index()], stream.dropped_by_worker);
                prop_assert_eq!(by_reason.iter().sum::<u64>() - stream.dropped_by_worker, stream.dropped_by_server);
                if carry.is_some() {
                    // The carry records the final mark: the stream takes no batch after it.
                    break;
                }
            }
            prop_assert!(markers <= 1);
        }
    }
}

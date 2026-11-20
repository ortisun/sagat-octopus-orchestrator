//! The JSON of a schedule (feature 018): how the spec, the policies and the target of a schedule
//! travel between the REST contract (`contracts/schedule-spec.schema.json`), the database and the
//! domain.
//!
//! Decoding is strict, like the schema: an unknown field is refused where it is, with its path, so a
//! typo is an error and not a setting silently ignored. Encoding is canonical: every default is
//! written out, so the stored document and a digest of it do not depend on what the caller left out.
//! What the domain refuses (a range that runs backwards, a spec with no entry) comes back as the
//! domain's own [`ScheduleError`] at the place of the document it concerns.
//!
//! The input of a run is not part of the target here: it is a payload of its own.

use std::str::FromStr as _;

use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::schedules::{
    Calendar, CalendarFields, CatchupMode, Ceilings, Dst, DstAmbiguous, DstGap, Field, Interval,
    Overlap, Policies, Range, ScheduleError, ScheduleSpec, ScheduleTarget, SpecParts, TargetParts,
    VersionPolicy,
};
use serde_json::{Map, Value, json};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

/// Why a document was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecReason {
    /// A key the contract does not name.
    UnknownField,
    /// A required key is absent.
    Missing,
    /// A value of the wrong JSON kind (a string for a number, a negative or fractional number).
    WrongType,
    /// A string that is not one of the names the contract lists.
    UnknownValue,
    /// A time that is not RFC 3339 with an offset.
    BadTime,
    /// A key that is not allowed with the others the document has.
    Unexpected,
    /// The domain refuses the value.
    Invalid(ScheduleError),
}

/// A refused document: the dotted path of the place and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError {
    pub path: String,
    pub reason: CodecReason,
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {:?}", self.path, self.reason)
    }
}

impl std::error::Error for CodecError {}

fn fail<T>(path: impl Into<String>, reason: CodecReason) -> Result<T, CodecError> {
    Err(CodecError {
        path: path.into(),
        reason,
    })
}

fn at(path: &str, key: &str) -> String {
    format!("{path}.{key}")
}

fn nth(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

// --- reading helpers ----------------------------------------------------------------------------

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, CodecError> {
    value
        .as_object()
        .map_or_else(|| fail(path, CodecReason::WrongType), Ok)
}

fn only(map: &Map<String, Value>, allowed: &[&str], path: &str) -> Result<(), CodecError> {
    match map.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(key) => fail(at(path, key), CodecReason::UnknownField),
        None => Ok(()),
    }
}

fn array<'a>(
    map: &'a Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<&'a [Value], CodecError> {
    match map.get(key) {
        None => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => fail(at(path, key), CodecReason::WrongType),
    }
}

fn unsigned(map: &Map<String, Value>, key: &str, path: &str) -> Result<Option<u64>, CodecError> {
    match map.get(key) {
        None => Ok(None),
        Some(value) => value.as_u64().map_or_else(
            || fail(at(path, key), CodecReason::WrongType),
            |n| Ok(Some(n)),
        ),
    }
}

fn small(map: &Map<String, Value>, key: &str, path: &str) -> Result<Option<u32>, CodecError> {
    match unsigned(map, key, path)? {
        None => Ok(None),
        Some(n) => u32::try_from(n)
            .map(Some)
            .or_else(|_| fail(at(path, key), CodecReason::WrongType)),
    }
}

fn text<'a>(
    map: &'a Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<&'a str>, CodecError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => fail(at(path, key), CodecReason::WrongType),
    }
}

fn flag(map: &Map<String, Value>, key: &str, path: &str) -> Result<Option<bool>, CodecError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => fail(at(path, key), CodecReason::WrongType),
    }
}

fn invalid<T>(path: &str, error: ScheduleError) -> Result<T, CodecError> {
    fail(path, CodecReason::Invalid(error))
}

fn instant(
    map: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<OffsetDateTime>, CodecError> {
    match text(map, key, path)? {
        None => Ok(None),
        Some(s) => OffsetDateTime::parse(s, &Rfc3339)
            .map(|t| Some(t.to_offset(UtcOffset::UTC)))
            .or_else(|_| fail(at(path, key), CodecReason::BadTime)),
    }
}

fn utc(t: OffsetDateTime) -> Value {
    // formatting an in-range instant with the well-known format cannot fail
    Value::String(t.format(&Rfc3339).unwrap_or_default())
}

// --- the spec -----------------------------------------------------------------------------------

const CALENDAR_KEYS: [&str; 8] = [
    "second",
    "minute",
    "hour",
    "dayOfMonth",
    "month",
    "year",
    "dayOfWeek",
    "comment",
];

fn field_from(
    map: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<Field>, CodecError> {
    if !map.contains_key(key) {
        return Ok(None);
    }
    let field_path = at(path, key);
    let mut ranges = Vec::new();
    for (i, item) in array(map, key, path)?.iter().enumerate() {
        let range_path = nth(&field_path, i);
        let range = object(item, &range_path)?;
        only(range, &["start", "end", "step"], &range_path)?;
        let start = small(range, "start", &range_path)?
            .map_or_else(|| fail(at(&range_path, "start"), CodecReason::Missing), Ok)?;
        let end = small(range, "end", &range_path)?;
        let step = small(range, "step", &range_path)?;
        match Range::new(start, end, step) {
            Ok(r) => ranges.push(r),
            Err(e) => return invalid(&range_path, e),
        }
    }
    Field::new(ranges)
        .map(Some)
        .or_else(|e| invalid(&field_path, e))
}

fn calendar_from(value: &Value, path: &str) -> Result<Calendar, CodecError> {
    let map = object(value, path)?;
    only(map, &CALENDAR_KEYS, path)?;
    let fields = CalendarFields {
        second: field_from(map, "second", path)?,
        minute: field_from(map, "minute", path)?,
        hour: field_from(map, "hour", path)?,
        day_of_month: field_from(map, "dayOfMonth", path)?,
        month: field_from(map, "month", path)?,
        year: field_from(map, "year", path)?,
        day_of_week: field_from(map, "dayOfWeek", path)?,
    };
    let comment = text(map, "comment", path)?;
    Calendar::new(fields, comment).or_else(|e| invalid(path, e))
}

fn calendars_from(
    map: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Vec<Calendar>, CodecError> {
    let list_path = at(path, key);
    array(map, key, path)?
        .iter()
        .enumerate()
        .map(|(i, item)| calendar_from(item, &nth(&list_path, i)))
        .collect()
}

fn interval_from(value: &Value, path: &str) -> Result<Interval, CodecError> {
    let map = object(value, path)?;
    only(map, &["everyMs", "offsetMs"], path)?;
    let every = unsigned(map, "everyMs", path)?
        .map_or_else(|| fail(at(path, "everyMs"), CodecReason::Missing), Ok)?;
    let offset = unsigned(map, "offsetMs", path)?.unwrap_or(0);
    Interval::new(every, offset).or_else(|e| invalid(path, e))
}

fn dst_from(value: &Value, path: &str) -> Result<Dst, CodecError> {
    let map = object(value, path)?;
    only(map, &["gap", "ambiguous"], path)?;
    let gap = match text(map, "gap", path)? {
        None | Some("SKIP") => DstGap::Skip,
        Some("SHIFT_FORWARD") => DstGap::ShiftForward,
        Some(_) => return fail(at(path, "gap"), CodecReason::UnknownValue),
    };
    let ambiguous = match text(map, "ambiguous", path)? {
        None | Some("ONCE_EARLIER") => DstAmbiguous::OnceEarlier,
        Some("TWICE") => DstAmbiguous::Twice,
        Some(_) => return fail(at(path, "ambiguous"), CodecReason::UnknownValue),
    };
    Ok(Dst::new(gap, ambiguous))
}

/// Decodes the spec of a schedule.
///
/// # Errors
/// [`CodecError`] naming the place and the reason.
pub fn spec_from_json(value: &Value, ceilings: &Ceilings) -> Result<ScheduleSpec, CodecError> {
    const PATH: &str = "spec";
    let map = object(value, PATH)?;
    only(
        map,
        &[
            "calendars",
            "cron",
            "intervals",
            "exclusions",
            "startAt",
            "endAt",
            "timezone",
            "jitterMs",
            "dst",
        ],
        PATH,
    )?;
    let calendars = calendars_from(map, "calendars", PATH)?;
    let exclusions = calendars_from(map, "exclusions", PATH)?;
    let mut cron = Vec::new();
    for (i, item) in array(map, "cron", PATH)?.iter().enumerate() {
        match item.as_str() {
            Some(s) => cron.push(s.to_owned()),
            None => return fail(nth(&at(PATH, "cron"), i), CodecReason::WrongType),
        }
    }
    let intervals = array(map, "intervals", PATH)?
        .iter()
        .enumerate()
        .map(|(i, item)| interval_from(item, &nth(&at(PATH, "intervals"), i)))
        .collect::<Result<Vec<_>, _>>()?;
    let dst = match map.get("dst") {
        None => Dst::default(),
        Some(v) => dst_from(v, &at(PATH, "dst"))?,
    };
    let jitter_ms = small(map, "jitterMs", PATH)?.unwrap_or(0);
    let parts = SpecParts {
        calendars,
        cron,
        intervals,
        exclusions,
        start_at: instant(map, "startAt", PATH)?,
        end_at: instant(map, "endAt", PATH)?,
        timezone: text(map, "timezone", PATH)?.map(str::to_owned),
        jitter_ms,
        dst,
    };
    ScheduleSpec::new(parts, ceilings).or_else(|e| invalid(PATH, e))
}

fn ranges_to_json(field: &Field) -> Value {
    Value::Array(
        field
            .ranges()
            .iter()
            .map(|r| json!({"start": r.start(), "end": r.end(), "step": r.step()}))
            .collect(),
    )
}

fn calendar_to_json(calendar: &Calendar) -> Value {
    let f = calendar.fields();
    let mut out = Map::new();
    for (key, field) in [
        ("second", &f.second),
        ("minute", &f.minute),
        ("hour", &f.hour),
        ("dayOfMonth", &f.day_of_month),
        ("month", &f.month),
        ("year", &f.year),
        ("dayOfWeek", &f.day_of_week),
    ] {
        if let Some(field) = field {
            out.insert(key.to_owned(), ranges_to_json(field));
        }
    }
    if let Some(comment) = calendar.comment() {
        out.insert("comment".to_owned(), Value::String(comment.to_owned()));
    }
    Value::Object(out)
}

/// The canonical document of a spec: every default written out.
#[must_use]
pub fn spec_to_json(spec: &ScheduleSpec) -> Value {
    let mut out = Map::new();
    if !spec.calendars().is_empty() {
        out.insert(
            "calendars".to_owned(),
            Value::Array(spec.calendars().iter().map(calendar_to_json).collect()),
        );
    }
    if !spec.cron().is_empty() {
        out.insert(
            "cron".to_owned(),
            Value::Array(spec.cron().iter().map(|c| json!(c.expression())).collect()),
        );
    }
    if !spec.intervals().is_empty() {
        out.insert(
            "intervals".to_owned(),
            Value::Array(
                spec.intervals()
                    .iter()
                    .map(|i| json!({"everyMs": i.every_ms(), "offsetMs": i.offset_ms()}))
                    .collect(),
            ),
        );
    }
    if !spec.exclusions().is_empty() {
        out.insert(
            "exclusions".to_owned(),
            Value::Array(spec.exclusions().iter().map(calendar_to_json).collect()),
        );
    }
    if let Some(start) = spec.start_at() {
        out.insert("startAt".to_owned(), utc(start));
    }
    if let Some(end) = spec.end_at() {
        out.insert("endAt".to_owned(), utc(end));
    }
    out.insert("timezone".to_owned(), json!(spec.timezone()));
    out.insert("jitterMs".to_owned(), json!(spec.jitter_ms()));
    out.insert(
        "dst".to_owned(),
        json!({
            "gap": match spec.dst().gap() { DstGap::Skip => "SKIP", DstGap::ShiftForward => "SHIFT_FORWARD" },
            "ambiguous": match spec.dst().ambiguous() {
                DstAmbiguous::OnceEarlier => "ONCE_EARLIER",
                DstAmbiguous::Twice => "TWICE",
            },
        }),
    );
    Value::Object(out)
}

// --- the policies -------------------------------------------------------------------------------

/// Decodes the policies of a schedule; absent settings take their defaults.
///
/// # Errors
/// [`CodecError`] naming the place and the reason.
pub fn policies_from_json(value: &Value, ceilings: &Ceilings) -> Result<Policies, CodecError> {
    const PATH: &str = "policies";
    let map = object(value, PATH)?;
    only(
        map,
        &[
            "overlap",
            "catchupWindowMs",
            "catchupMode",
            "pauseOnFailure",
            "bufferLimit",
        ],
        PATH,
    )?;
    let defaults = Policies::default();
    let overlap = match text(map, "overlap", PATH)? {
        None | Some("SKIP") => Overlap::Skip,
        Some("BUFFER_ONE") => Overlap::BufferOne,
        Some("BUFFER_ALL") => Overlap::BufferAll,
        Some("CANCEL_OTHER") => Overlap::CancelOther,
        Some("ALLOW_ALL") => Overlap::AllowAll,
        Some(_) => return fail(at(PATH, "overlap"), CodecReason::UnknownValue),
    };
    let mode = match text(map, "catchupMode", PATH)? {
        None | Some("ONE") => CatchupMode::One,
        Some("ALL") => CatchupMode::All,
        Some(_) => return fail(at(PATH, "catchupMode"), CodecReason::UnknownValue),
    };
    let window =
        unsigned(map, "catchupWindowMs", PATH)?.unwrap_or_else(|| defaults.catchup_window_ms());
    let pause = flag(map, "pauseOnFailure", PATH)?.unwrap_or(false);
    let buffer = small(map, "bufferLimit", PATH)?.unwrap_or_else(|| defaults.buffer_limit());
    Policies::new(overlap, window, mode, pause, buffer, ceilings).or_else(|e| invalid(PATH, e))
}

/// The canonical document of the policies: every setting written out.
#[must_use]
pub fn policies_to_json(policies: &Policies) -> Value {
    json!({
        "overlap": match policies.overlap() {
            Overlap::Skip => "SKIP",
            Overlap::BufferOne => "BUFFER_ONE",
            Overlap::BufferAll => "BUFFER_ALL",
            Overlap::CancelOther => "CANCEL_OTHER",
            Overlap::AllowAll => "ALLOW_ALL",
        },
        "catchupWindowMs": policies.catchup_window_ms(),
        "catchupMode": match policies.catchup_mode() { CatchupMode::One => "ONE", CatchupMode::All => "ALL" },
        "pauseOnFailure": policies.pause_on_failure(),
        "bufferLimit": policies.buffer_limit(),
    })
}

// --- the target ---------------------------------------------------------------------------------

/// Decodes what a schedule starts.
///
/// # Errors
/// [`CodecError`] naming the place and the reason.
pub fn target_from_json(value: &Value) -> Result<ScheduleTarget, CodecError> {
    const PATH: &str = "action";
    let map = object(value, PATH)?;
    only(
        map,
        &[
            "workflowId",
            "versionPolicy",
            "versionId",
            "nominalTimeField",
            "runTimeoutMs",
            "businessIdPrefix",
            "businessIdConflictPolicy",
            "businessIdReusePolicy",
        ],
        PATH,
    )?;
    let workflow = match text(map, "workflowId", PATH)? {
        None => return fail(at(PATH, "workflowId"), CodecReason::Missing),
        Some(s) => WorkflowId::from_str(s)
            .or_else(|_| fail(at(PATH, "workflowId"), CodecReason::WrongType))?,
    };
    let version_id = match text(map, "versionId", PATH)? {
        None => None,
        Some(s) => Some(
            WorkflowVersionId::from_str(s)
                .or_else(|_| fail(at(PATH, "versionId"), CodecReason::WrongType))?,
        ),
    };
    let version = match (text(map, "versionPolicy", PATH)?, version_id) {
        (None | Some("ACTIVE"), None) => VersionPolicy::Active,
        (None | Some("ACTIVE"), Some(_)) => {
            return fail(at(PATH, "versionId"), CodecReason::Unexpected);
        }
        (Some("PINNED"), Some(id)) => VersionPolicy::Pinned(id),
        (Some("PINNED"), None) => return fail(at(PATH, "versionId"), CodecReason::Missing),
        (Some(_), _) => return fail(at(PATH, "versionPolicy"), CodecReason::UnknownValue),
    };
    let conflict = match text(map, "businessIdConflictPolicy", PATH)? {
        None => ConflictPolicy::default(),
        Some(s) => ConflictPolicy::from_str(s).or_else(|()| {
            fail(
                at(PATH, "businessIdConflictPolicy"),
                CodecReason::UnknownValue,
            )
        })?,
    };
    let reuse = match text(map, "businessIdReusePolicy", PATH)? {
        None => ReusePolicy::default(),
        Some(s) => ReusePolicy::from_str(s)
            .or_else(|()| fail(at(PATH, "businessIdReusePolicy"), CodecReason::UnknownValue))?,
    };
    let parts = TargetParts {
        workflow,
        version,
        nominal_time_field: text(map, "nominalTimeField", PATH)?.map(str::to_owned),
        run_timeout_ms: unsigned(map, "runTimeoutMs", PATH)?,
        business_id_prefix: text(map, "businessIdPrefix", PATH)?.map(str::to_owned),
        business_id_conflict: conflict,
        business_id_reuse: reuse,
    };
    ScheduleTarget::new(parts).or_else(|e| invalid(PATH, e))
}

/// The canonical document of a target (without its input).
#[must_use]
pub fn target_to_json(target: &ScheduleTarget) -> Value {
    let mut out = Map::new();
    out.insert(
        "workflowId".to_owned(),
        json!(target.workflow().to_string()),
    );
    match target.version() {
        VersionPolicy::Active => {
            out.insert("versionPolicy".to_owned(), json!("ACTIVE"));
        }
        VersionPolicy::Pinned(id) => {
            out.insert("versionPolicy".to_owned(), json!("PINNED"));
            out.insert("versionId".to_owned(), json!(id.to_string()));
        }
    }
    if let Some(field) = target.nominal_time_field() {
        out.insert("nominalTimeField".to_owned(), json!(field));
    }
    if let Some(ms) = target.run_timeout_ms() {
        out.insert("runTimeoutMs".to_owned(), json!(ms));
    }
    if let Some(id) = target.business_id() {
        out.insert("businessIdPrefix".to_owned(), json!(id.prefix()));
        out.insert(
            "businessIdConflictPolicy".to_owned(),
            json!(id.conflict().as_str()),
        );
        out.insert(
            "businessIdReusePolicy".to_owned(),
            json!(id.reuse().as_str()),
        );
    }
    Value::Object(out)
}

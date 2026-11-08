//! The attempt input of a lake step, as the server puts it in the lease (feature 035, T035-056, D035-131).
//!
//! The runner reads this document with a strict reader (`sagat-infra-lake`, `AttemptInput::read`, D035-121) and refuses anything that
//! is not exactly what the contract says. This is the other end: a pure function from what the server knows about the step to that
//! document. It reads nothing and stores nothing; what it needs is given, so that a test can hold it to the runner's corpus
//! (`tests/contract/iceberg-writes/fixtures/attempt-input/cases.json`, whose base document this reproduces member for member).
//!
//! **The document carries no credential, no relay and no connection.** The credentials come from `ResolveDatabase`, pinned and
//! audited; the relay is the runner's own. What is here is the effect key, the instant of the first attempt, the five snapshot
//! properties the platform writes, the table, the ref, the mode, the records, the mapping, three flags and the effective limits.
//!
//! The server cannot link the runner's reader, so it checks what it controls itself: a record that is not an object, a blob source
//! of none or of more than the domain's ceiling or one named twice, an effect key the host would refuse, and a document over the
//! runner's bound are refused **here**, with a name, instead of being leased for the runner to refuse as a permanent configuration
//! failure after the step was claimed.

use std::collections::BTreeSet;

use sagat_domain::blobs::BlobId;
use sagat_domain::database::{EffectiveLakeLimits, IcebergWrite, MAX_RECORD_BLOBS, WriteIntent};
use serde_json::{Value, json};
use time::OffsetDateTime;

/// The schema the runner's reader serves.
pub const SCHEMA: &str = "sagat.lake.attempt-input.v1";

/// The longest document the runner reads (`MAX_INPUT_BYTES` of `sagat-infra-lake`; a test in the `tests` crate holds the two equal).
/// The server's own cap on a lease input is larger (8 MiB), so this is the one that matters for a lake step.
pub const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;

/// The longest effect key the host takes.
const MAX_EFFECT_KEY_BYTES: usize = 512;

/// The format of a blob of records, as the host reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobFormat {
    Ndjson,
    Parquet,
}

impl BlobFormat {
    const fn wire(self) -> &'static str {
        match self {
            Self::Ndjson => "ndjson",
            Self::Parquet => "parquet",
        }
    }
}

/// A blob of records an earlier step of the run committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LakeBlob {
    pub id: BlobId,
    pub format: BlobFormat,
}

/// The records of the attempt, already evaluated from the step's expression.
#[derive(Debug, Clone, PartialEq)]
pub enum LakeRecords {
    /// The records themselves: each one a JSON object.
    Inline(Vec<Value>),
    /// One to [`MAX_RECORD_BLOBS`] blobs, each once.
    Blobs(Vec<LakeBlob>),
}

/// An append never allows an empty write. The lease says so and the receipt is held to it, from this one value.
pub const APPEND_ALLOWS_EMPTY: bool = false;

/// What the server knows about this attempt.
#[derive(Debug, Clone, Copy)]
pub struct LakeFacts<'a> {
    /// Stable across the attempts of the step: it is how a retry finds what an earlier attempt already committed.
    pub effect_key: &'a str,
    /// When the first attempt started, by the database's clock.
    pub first_attempt_started_at: OffsetDateTime,
    pub run_id: &'a str,
    pub step_id: &'a str,
    pub workflow: &'a str,
    pub workflow_version: &'a str,
    /// An earlier attempt of this effect started, so it may have written files the host has to look for and clean.
    pub prior_attempt_may_have_left_files: bool,
    /// A write of no record is a success without a commit and not a failure. Only an overwrite names it
    /// (`overwrite.allowEmpty` in the workflow schema, T035-080): an append never allows an empty write, so the caller says `false`.
    pub allow_empty: bool,
}

/// Why a document cannot be built. A closed kind: it names a part and never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LakeInputError {
    #[error("a record of an inline source is not a JSON object")]
    Record,
    #[error("a blob source names none, more than the ceiling, or the same blob twice")]
    Blobs,
    #[error("the effect key is not 1 to 512 characters without a control character")]
    EffectKey,
    #[error("the document is over the bound the runner reads")]
    TooLarge,
}

/// `at` as the host reads an instant: UTC, to the millisecond, `YYYY-MM-DDTHH:MM:SS.mmmZ`.
fn instant(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

fn records_member(records: LakeRecords) -> Result<Value, LakeInputError> {
    match records {
        LakeRecords::Inline(items) => {
            if !items.iter().all(Value::is_object) {
                return Err(LakeInputError::Record);
            }
            Ok(json!({"inline": items}))
        }
        LakeRecords::Blobs(blobs) => {
            let mut seen = BTreeSet::new();
            if !(1..=MAX_RECORD_BLOBS).contains(&blobs.len())
                || !blobs.iter().all(|blob| seen.insert(blob.id.to_string()))
            {
                return Err(LakeInputError::Blobs);
            }
            let list: Vec<Value> = blobs
                .iter()
                .map(|blob| json!({"id": blob.id.to_string(), "format": blob.format.wire()}))
                .collect();
            Ok(json!({"blobs": list}))
        }
    }
}

/// The `input_json` of the lease of a lake step.
///
/// # Errors
/// [`LakeInputError`] for the first part the runner would refuse.
pub fn lake_attempt_input(
    write: &IcebergWrite,
    facts: &LakeFacts<'_>,
    records: LakeRecords,
    limits: EffectiveLakeLimits,
) -> Result<Vec<u8>, LakeInputError> {
    if facts.effect_key.is_empty()
        || facts.effect_key.len() > MAX_EFFECT_KEY_BYTES
        || facts.effect_key.chars().any(char::is_control)
    {
        return Err(LakeInputError::EffectKey);
    }
    let mode = match write.intent() {
        WriteIntent::Append => "append",
    };
    let mapping: Vec<Value> = write
        .mapping()
        .iter()
        .map(|pair| json!({"field": pair.field(), "column": pair.column()}))
        .collect();
    let document = json!({
        "schema": SCHEMA,
        "operation": "icebergWrite",
        "effectKey": facts.effect_key,
        "firstAttemptStartedAt": instant(facts.first_attempt_started_at),
        "snapshotProperties": {
            "sagat.effect-key": facts.effect_key,
            "sagat.run-id": facts.run_id,
            "sagat.step-id": facts.step_id,
            "sagat.workflow": facts.workflow,
            "sagat.workflow-version": facts.workflow_version,
        },
        "table": {"namespace": write.table().namespace(), "name": write.table().name()},
        "ref": write.reference(),
        "mode": mode,
        "records": records_member(records)?,
        "mapping": mapping,
        "ignoreUnknownFields": write.ignore_unknown_fields(),
        "allowEmpty": facts.allow_empty,
        "priorAttemptMayHaveLeftFiles": facts.prior_attempt_may_have_left_files,
        "limits": {
            "maxRecordBytes": limits.max_record_bytes,
            "maxRecords": limits.max_records,
            "timeoutMs": limits.timeout_ms,
        },
    });
    let bytes = serde_json::to_vec(&document).map_err(|_| LakeInputError::TooLarge)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(LakeInputError::TooLarge);
    }
    Ok(bytes)
}

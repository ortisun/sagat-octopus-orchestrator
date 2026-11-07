//! The attempt input of a lake step, as the lease carries it (feature 035, T035-035, T035-056, D035-121).
//!
//! The lease of a lake activity has an `input_json`. It carries what the lake runner needs to run the attempt **and no
//! credential and no relay**: the effect key, the instant the first attempt started, the table, the mode, the mapping, the
//! flags, the limits and the record sources. The credentials come from `ResolveDatabase`, two-phase and pinned, and the relay
//! is the runner's own.
//!
//! The server builds this document and the runner reads it, so the two ends are held to one corpus
//! (`tests/contract/iceberg-writes/fixtures/attempt-input/cases.json`). The reader is strict in the way the host's request is:
//! a document that is not exactly what the contract says is refused whole, because a step that writes a table must not run
//! on a guess about what it meant. What is strict:
//!
//! - an **object**, at most [`MAX_INPUT_BYTES`], with **no name twice at any depth** (`serde_json::Value` keeps the last of a
//!   name written twice and drops the first without a trace, which is how a mapping or a limit could be read as the one the
//!   author did not write);
//! - every member present, no member the contract does not name, each of its own type and within its bounds;
//! - a source of records that is exactly one of `inline` and `blobs`, with at most 32 blobs, each named once;
//! - what this runner does not serve is **refused as unsupported by name**, not read as something else: the modes other than
//!   `append`, the operations other than `icebergWrite`, and a 017 database result as the source (its conversion into rows is
//!   G035-059).
//!
//! An error names the member and never its value: the document holds the author's data.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sagat_domain::blobs::BlobId;
use sagat_domain::database::{
    ColumnMapping, MAX_MAPPED_COLUMNS, MAX_RECORD_BLOBS, TableIdentifier,
};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use super::MAX_EFFECT_KEY_BYTES;
use super::write_body::{
    InputFormat, MAX_PROPERTY_BYTES, MAX_REFERENCE_BYTES, SNAPSHOT_PROPERTIES, is_instant, plain,
};
use super::{Connection, InputFile, Limits, WriteAppend};

/// The longest document the reader takes. A source of records that does not fit comes as blobs.
pub const MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;

/// The schema this reader serves. A document of another version is a different contract, not a variation of this one.
const SCHEMA: &str = "sagat.lake.attempt-input.v1";

/// The operations of the lake profile that have a name in the contract but are not served by this reader yet.
const OTHER_OPERATIONS: [&str; 4] = ["icebergRevert", "objectWrite", "probe", "browse"];

/// The bounds of the node's limits, the same as the domain's (`LakeLimits`, T035-041): a limit outside them was never one
/// the platform could have saved.
const RECORD_BYTES: (u64, u64) = (1024, 2_147_483_648);
const RECORDS: (u64, u64) = (1, 100_000_000);
const TIMEOUT_MS: (u64, u64) = (1000, 3_600_000);

/// Why a document is not an attempt input. The reason names a part of the contract and never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    #[error(
        "the attempt input is not one JSON object of at most 4 MiB in which no name is written twice"
    )]
    Document,
    /// A member is missing, is one the contract does not name, is of another type, or is outside its bounds.
    #[error(
        "the member `{0}` of the attempt input is missing, not allowed, or not what the contract says"
    )]
    Member(&'static str),
    /// A part of the contract that this runner does not serve (yet).
    #[error("the attempt input asks for `{0}`, which this runner does not serve")]
    Unsupported(&'static str),
}

/// A recursive JSON value in which no object has a name twice.
struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Reads;
        impl<'de> Visitor<'de> for Reads {
            type Value = Strict;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON value with no name written twice in an object")
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Strict, E> {
                Ok(Strict(Value::Bool(value)))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Strict, E> {
                Ok(Strict(Value::Number(value.into())))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Strict, E> {
                Ok(Strict(Value::Number(value.into())))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Strict, E> {
                Number::from_f64(value)
                    .map(|number| Strict(Value::Number(number)))
                    .ok_or_else(|| de::Error::custom("a number JSON cannot hold"))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Strict, E> {
                Ok(Strict(Value::String(value.to_owned())))
            }

            fn visit_string<E: de::Error>(self, value: String) -> Result<Strict, E> {
                Ok(Strict(Value::String(value)))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Strict, A::Error> {
                let mut items = Vec::new();
                while let Some(Strict(item)) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Strict(Value::Array(items)))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Strict, A::Error> {
                let mut members = Map::new();
                while let Some(name) = map.next_key::<String>()? {
                    let Strict(value) = map.next_value()?;
                    if members.insert(name, value).is_some() {
                        return Err(de::Error::custom("a name twice"));
                    }
                }
                Ok(Strict(Value::Object(members)))
            }
        }
        deserializer.deserialize_any(Reads)
    }
}

/// The members of one object, taken out by name; whatever is left over is a member the contract does not name.
struct Members {
    left: Map<String, Value>,
    /// What to say when something is wrong at this level: the name of the object, or the document.
    place: &'static str,
}

impl Members {
    fn of(value: Value, place: &'static str) -> Result<Self, InputError> {
        match value {
            Value::Object(left) => Ok(Self { left, place }),
            _ => Err(InputError::Member(place)),
        }
    }

    fn take(&mut self, name: &'static str) -> Result<Value, InputError> {
        self.left.remove(name).ok_or(InputError::Member(name))
    }

    fn text(&mut self, name: &'static str) -> Result<String, InputError> {
        match self.take(name)? {
            Value::String(text) => Ok(text),
            _ => Err(InputError::Member(name)),
        }
    }

    fn flag(&mut self, name: &'static str) -> Result<bool, InputError> {
        self.take(name)?.as_bool().ok_or(InputError::Member(name))
    }

    fn number(&mut self, name: &'static str, (low, high): (u64, u64)) -> Result<u32, InputError> {
        self.take(name)?
            .as_u64()
            .filter(|value| (low..=high).contains(value))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(InputError::Member(name))
    }

    /// Nothing may be left: a member the contract does not name is refused, not ignored.
    fn finish(self) -> Result<(), InputError> {
        if self.left.is_empty() {
            Ok(())
        } else {
            Err(InputError::Member(self.place))
        }
    }
}

/// The limits the step runs under, already the intersection of the node's, the connection's and the platform's (the server
/// computes them as it does for a SQL step). The runner may still lower them with its own deployment ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeLimits {
    pub max_record_bytes: u32,
    pub max_records: u32,
    pub timeout_ms: u32,
}

/// A blob of records an earlier step committed, to be read under the lease with `ReadBlob`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobRef {
    pub id: BlobId,
    pub format: InputFormat,
}

/// Where the records of the attempt come from.
#[derive(Clone, PartialEq)]
pub enum Records {
    /// The records themselves, objects, evaluated by the server.
    Inline(Vec<Value>),
    /// One to [`MAX_RECORD_BLOBS`] blobs, each once.
    Blobs(Vec<BlobRef>),
}

/// The records are the author's data: a `Debug` shows how many.
impl fmt::Debug for Records {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inline(records) => {
                write!(formatter, "Inline(<{} records, not shown>)", records.len())
            }
            Self::Blobs(blobs) => write!(formatter, "Blobs(<{} blobs>)", blobs.len()),
        }
    }
}

/// An `icebergWrite` in the append mode, as the lease carries it.
#[derive(Clone, PartialEq)]
pub struct AttemptInput {
    effect_key: String,
    first_attempt_started_at: String,
    snapshot_properties: BTreeMap<String, String>,
    table: TableIdentifier,
    reference: String,
    records: Records,
    mapping: Vec<ColumnMapping>,
    ignore_unknown_fields: bool,
    allow_empty: bool,
    prior_attempt_may_have_left_files: bool,
    limits: NodeLimits,
}

impl fmt::Debug for AttemptInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttemptInput")
            .field("effect_key", &self.effect_key)
            .field("table", &self.table)
            .field("reference", &self.reference)
            .field("records", &self.records)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl AttemptInput {
    /// Reads the `input_json` of a lake lease.
    ///
    /// # Errors
    /// [`InputError`] for the first part that is not what the contract says, or that this runner does not serve.
    pub fn read(bytes: &[u8]) -> Result<Self, InputError> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(InputError::Document);
        }
        let Strict(document) = serde_json::from_slice(bytes).map_err(|_| InputError::Document)?;
        let Value::Object(_) = document else {
            return Err(InputError::Document);
        };
        let mut members = Members::of(document, "document")?;

        if members.text("schema")? != SCHEMA {
            return Err(InputError::Member("schema"));
        }
        match members.text("operation")?.as_str() {
            "icebergWrite" => {}
            other if OTHER_OPERATIONS.contains(&other) => {
                return Err(InputError::Unsupported("operation"));
            }
            _ => return Err(InputError::Member("operation")),
        }
        let effect_key = members.text("effectKey")?;
        if !plain(&effect_key, MAX_EFFECT_KEY_BYTES) {
            return Err(InputError::Member("effectKey"));
        }
        let first_attempt_started_at = members.text("firstAttemptStartedAt")?;
        if !is_instant(&first_attempt_started_at) {
            return Err(InputError::Member("firstAttemptStartedAt"));
        }
        let snapshot_properties = snapshot_properties(members.take("snapshotProperties")?)?;
        let table = table(members.take("table")?)?;
        let reference = members.text("ref")?;
        if !plain(&reference, MAX_REFERENCE_BYTES) {
            return Err(InputError::Member("ref"));
        }
        match members.text("mode")?.as_str() {
            "append" => {}
            "overwrite" | "upsert" => return Err(InputError::Unsupported("mode")),
            _ => return Err(InputError::Member("mode")),
        }
        let records = records(members.take("records")?)?;
        let mapping = mapping(members.take("mapping")?)?;
        let ignore_unknown_fields = members.flag("ignoreUnknownFields")?;
        let allow_empty = members.flag("allowEmpty")?;
        let prior_attempt_may_have_left_files = members.flag("priorAttemptMayHaveLeftFiles")?;
        let limits = limits(members.take("limits")?)?;
        members.finish()?;

        Ok(Self {
            effect_key,
            first_attempt_started_at,
            snapshot_properties,
            table,
            reference,
            records,
            mapping,
            ignore_unknown_fields,
            allow_empty,
            prior_attempt_may_have_left_files,
            limits,
        })
    }

    #[must_use]
    pub fn effect_key(&self) -> &str {
        &self.effect_key
    }

    /// An RFC 3339 instant in UTC.
    #[must_use]
    pub fn first_attempt_started_at(&self) -> &str {
        &self.first_attempt_started_at
    }

    #[must_use]
    pub const fn table(&self) -> &TableIdentifier {
        &self.table
    }

    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    #[must_use]
    pub const fn records(&self) -> &Records {
        &self.records
    }

    /// Takes the records out, leaving an empty inline list behind. A document can hold millions of small values, so the
    /// caller that stages them moves them instead of making a second copy of the whole tree.
    pub fn take_records(&mut self) -> Records {
        std::mem::replace(&mut self.records, Records::Inline(Vec::new()))
    }

    #[must_use]
    pub fn mapping(&self) -> &[ColumnMapping] {
        &self.mapping
    }

    #[must_use]
    pub const fn limits(&self) -> NodeLimits {
        self.limits
    }

    /// The write the host is asked to do: the input's own table, ref, mapping, flags and snapshot summary, with the parts the
    /// runner owns: the connection it resolved, the record files it staged, and the limits it enforces.
    #[must_use]
    pub fn write_append(
        &self,
        connection: Connection,
        inputs: Vec<InputFile>,
        limits: Limits,
    ) -> WriteAppend {
        let mut write = WriteAppend::new(
            self.table.clone(),
            &self.reference,
            connection,
            inputs,
            limits,
        )
        .ignore_unknown_fields(self.ignore_unknown_fields)
        .allow_empty(self.allow_empty)
        .prior_attempt_may_have_left_files(self.prior_attempt_may_have_left_files);
        for (key, value) in &self.snapshot_properties {
            write = write.snapshot_property(key, value);
        }
        for pair in &self.mapping {
            write = write.map_column(pair.clone());
        }
        write
    }
}

fn snapshot_properties(value: Value) -> Result<BTreeMap<String, String>, InputError> {
    const NAME: &str = "snapshotProperties";
    let Value::Object(found) = value else {
        return Err(InputError::Member(NAME));
    };
    found
        .into_iter()
        .map(|(key, value)| match value {
            Value::String(text)
                if SNAPSHOT_PROPERTIES.contains(&key.as_str())
                    && plain(&text, MAX_PROPERTY_BYTES) =>
            {
                Ok((key, text))
            }
            _ => Err(InputError::Member(NAME)),
        })
        .collect()
}

fn table(value: Value) -> Result<TableIdentifier, InputError> {
    let mut members = Members::of(value, "table")?;
    let Value::Array(levels) = members.take("namespace")? else {
        return Err(InputError::Member("table"));
    };
    let namespace = levels
        .into_iter()
        .map(|level| match level {
            Value::String(text) => Ok(text),
            _ => Err(InputError::Member("table")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let name = members.text("name")?;
    members.finish()?;
    TableIdentifier::new(namespace, &name).map_err(|_| InputError::Member("table"))
}

fn records(value: Value) -> Result<Records, InputError> {
    let Value::Object(mut found) = value else {
        return Err(InputError::Member("records"));
    };
    // exactly one source: two sources would be two answers to where the rows are
    if found.len() != 1 {
        return Err(InputError::Member("records"));
    }
    if let Some(inline) = found.remove("inline") {
        let Value::Array(items) = inline else {
            return Err(InputError::Member("records"));
        };
        if !items.iter().all(Value::is_object) {
            return Err(InputError::Member("records"));
        }
        return Ok(Records::Inline(items));
    }
    if let Some(blobs) = found.remove("blobs") {
        return blob_refs(blobs).map(Records::Blobs);
    }
    if found.contains_key("databaseResult") {
        return Err(InputError::Unsupported("records.databaseResult"));
    }
    Err(InputError::Member("records"))
}

fn blob_refs(value: Value) -> Result<Vec<BlobRef>, InputError> {
    let Value::Array(items) = value else {
        return Err(InputError::Member("records"));
    };
    if !(1..=MAX_RECORD_BLOBS).contains(&items.len()) {
        return Err(InputError::Member("records"));
    }
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .map(|item| {
            let mut members = Members::of(item, "records")?;
            let id = members
                .text("id")?
                .parse::<BlobId>()
                .map_err(|_| InputError::Member("records"))?;
            let format = match members.text("format")?.as_str() {
                "ndjson" => InputFormat::Ndjson,
                "parquet" => InputFormat::Parquet,
                _ => return Err(InputError::Member("records")),
            };
            members.finish()?;
            // a blob twice is a file read twice, and the rows would be written twice
            if !seen.insert(id.to_string()) {
                return Err(InputError::Member("records"));
            }
            Ok(BlobRef { id, format })
        })
        .collect()
}

fn mapping(value: Value) -> Result<Vec<ColumnMapping>, InputError> {
    let Value::Array(items) = value else {
        return Err(InputError::Member("mapping"));
    };
    if items.len() > MAX_MAPPED_COLUMNS {
        return Err(InputError::Member("mapping"));
    }
    let (mut fields, mut columns) = (BTreeSet::new(), BTreeSet::new());
    items
        .into_iter()
        .map(|item| {
            let mut members = Members::of(item, "mapping")?;
            let field = members.text("field")?;
            let column = members.text("column")?;
            members.finish()?;
            // a field or a column twice is two rows of the mapping that disagree about one thing
            if !fields.insert(field.clone()) || !columns.insert(column.clone()) {
                return Err(InputError::Member("mapping"));
            }
            ColumnMapping::new(&field, &column).map_err(|_| InputError::Member("mapping"))
        })
        .collect()
}

fn limits(value: Value) -> Result<NodeLimits, InputError> {
    let mut members = Members::of(value, "limits")?;
    let limits = NodeLimits {
        max_record_bytes: members.number("maxRecordBytes", RECORD_BYTES)?,
        max_records: members.number("maxRecords", RECORDS)?,
        timeout_ms: members.number("timeoutMs", TIMEOUT_MS)?,
    };
    members.finish()?;
    Ok(limits)
}

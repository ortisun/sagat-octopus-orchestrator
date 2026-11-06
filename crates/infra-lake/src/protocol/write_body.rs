//! The body of an `icebergWrite` request (feature 035, T035-035, D035-101, D035-105, D035-109).
//!
//! The envelope of the request line (`protocol`, `operation`, `effectKey`, `firstAttemptStartedAt`, `relay`) belongs to
//! [`super::Request`]. What follows is the part the Python host parses with `runners/lake/sagat_lake/request.py`: the
//! table, the ref, the mode, the pinned connection with the secrets the runner resolved, the record files it staged
//! under `in/`, the mapping, the flags and the limits.
//!
//! The builder takes the domain's own [`TableIdentifier`] and [`ColumnMapping`], which already refuse what the contract
//! refuses, and the connection as the platform stored it (`catalog` and `storage` documents are forwarded untouched, so
//! the storage format stays defined in one place). What the domain does not own it checks here, against the bounds the
//! host enforces: a body the host would refuse is a fault of the runner and is found before a container is started,
//! not as `LAKE_HOST_PROTOCOL` from inside one.
//!
//! The members are spelled once, here, and the fixture `tests/contract/iceberg-writes/fixtures/requests/` is read by a
//! test on each side, so the two halves of the line cannot drift without a test failing.

use std::collections::BTreeMap;
use std::fmt;

use sagat_domain::database::{
    ColumnMapping, MAX_MAPPED_COLUMNS, MAX_RECORD_BLOBS, TableIdentifier,
};
use serde_json::{Map, Value, json};

/// The members of a snapshot summary the platform writes (D035-016); the host accepts no other.
pub const SNAPSHOT_PROPERTIES: [&str; 5] = [
    "sagat.effect-key",
    "sagat.run-id",
    "sagat.step-id",
    "sagat.workflow",
    "sagat.workflow-version",
];

const MAX_COMMIT_RETRIES: u32 = 16;
const MAX_INPUT_NAME_BYTES: usize = 128;
const MAX_REFERENCE_BYTES: usize = 255;
const MAX_LOCATIONS: usize = 16;
const MAX_LOCATION_BYTES: usize = 2048;
const MAX_SECRET_BYTES: usize = 16384;
const MAX_PROPERTY_BYTES: usize = 1024;
const MAX_FRACTION_DIGITS: usize = 6;

/// Why a body cannot be built. The reason names the part and never its value: a body holds secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BodyError {
    #[error(
        "a record file name is not 1 to 128 characters of letters, digits, '.', '_' and '-' that start without a dot"
    )]
    InputName,
    #[error("a write needs 1 to 32 record files")]
    Inputs,
    #[error(
        "a limit is outside the range the host accepts, or the deadline is not an RFC 3339 instant in UTC"
    )]
    Limits,
    #[error("the target ref is not 1 to 255 bytes without control characters")]
    Reference,
    #[error("the connection holds a location or a secret the host would refuse")]
    Connection,
    #[error("a snapshot property is not one the platform writes, or its value is out of bounds")]
    SnapshotProperty,
    #[error("the mapping names more columns than the ceiling")]
    Mapping,
}

/// The format of a record file the runner staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Ndjson,
    Parquet,
}

impl InputFormat {
    const fn wire(self) -> &'static str {
        match self {
            Self::Ndjson => "ndjson",
            Self::Parquet => "parquet",
        }
    }
}

/// A record file under the attempt's `in/` directory. The name is one path component: the host resolves it below
/// `in/` and nothing downstream follows a name out of the attempt's directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputFile {
    name: String,
    format: InputFormat,
}

impl InputFile {
    /// # Errors
    /// [`BodyError::InputName`] for a name that is empty, longer than 128 bytes, starts with a dot, or has a character
    /// outside `A-Z a-z 0-9 . _ -`.
    pub fn new(name: &str, format: InputFormat) -> Result<Self, BodyError> {
        let mut bytes = name.bytes();
        let first = bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
        if !first
            || name.len() > MAX_INPUT_NAME_BYTES
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(BodyError::InputName);
        }
        Ok(Self {
            name: name.to_owned(),
            format,
        })
    }
}

/// The ceilings the host enforces, and the instant after which the attempt is over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    max_commit_retries: u32,
    max_decoded_bytes: u64,
    max_metadata_bytes: u64,
    deadline_at: String,
}

impl Limits {
    /// `deadline_at` is an RFC 3339 instant in UTC (`2026-11-05T10:10:00.000Z`) that the caller formats from a real
    /// clock; only its shape is checked here.
    ///
    /// # Errors
    /// [`BodyError::Limits`] for more than 16 commit retries, a byte ceiling above `i64::MAX`, or a deadline that is not
    /// `YYYY-MM-DDTHH:MM:SS` with up to six fractional digits and a `Z`.
    pub fn new(
        max_commit_retries: u32,
        max_decoded_bytes: u64,
        max_metadata_bytes: u64,
        deadline_at: impl Into<String>,
    ) -> Result<Self, BodyError> {
        let deadline_at = deadline_at.into();
        let ceiling = u64::try_from(i64::MAX).map_err(|_| BodyError::Limits)?;
        if max_commit_retries > MAX_COMMIT_RETRIES
            || max_decoded_bytes > ceiling
            || max_metadata_bytes > ceiling
            || !is_instant(&deadline_at)
        {
            return Err(BodyError::Limits);
        }
        Ok(Self {
            max_commit_retries,
            max_decoded_bytes,
            max_metadata_bytes,
            deadline_at,
        })
    }

    fn to_value(&self) -> Value {
        // the byte ceilings are decimal strings: a JSON number is a double for some readers (D035-101)
        json!({
            "maxCommitRetries": self.max_commit_retries,
            "maxDecodedBytes": self.max_decoded_bytes.to_string(),
            "maxMetadataBytes": self.max_metadata_bytes.to_string(),
            "deadlineAt": self.deadline_at,
        })
    }
}

fn is_instant(text: &str) -> bool {
    let bytes = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|part| part.iter().all(u8::is_ascii_digit))
    };
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')]
        .iter()
        .all(|&(at, expected)| bytes.get(at) == Some(&expected));
    if !(separators
        && digits(0..4)
        && digits(5..7)
        && digits(8..10)
        && digits(11..13)
        && digits(14..16)
        && digits(17..19))
    {
        return false;
    }
    match bytes.get(19..) {
        Some([b'Z']) => true,
        Some([b'.', fraction @ .., b'Z']) => {
            (1..=MAX_FRACTION_DIGITS).contains(&fraction.len())
                && fraction.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    }
}

/// Text of 1 to `limit` bytes without a control character, as the domain's and the host's `plain`.
fn plain(text: &str, limit: usize) -> bool {
    !text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
}

/// The pinned connection of a write: the `catalog` and `storage` documents as the platform stored them
/// (`crates/infra-postgres/src/database/iceberg.rs`), the warehouse locations, and the secrets the runner resolved for
/// the catalog and for the storage (each a document of text to text).
///
/// Nothing of it is shown by [`fmt::Debug`].
#[derive(Clone)]
pub struct Connection {
    catalog: Value,
    storage: Value,
    warehouse_locations: Vec<String>,
    catalog_secrets: BTreeMap<String, String>,
    storage_secrets: BTreeMap<String, String>,
}

impl Connection {
    #[must_use]
    pub const fn new(
        catalog: Value,
        storage: Value,
        warehouse_locations: Vec<String>,
        catalog_secrets: BTreeMap<String, String>,
        storage_secrets: BTreeMap<String, String>,
    ) -> Self {
        Self {
            catalog,
            storage,
            warehouse_locations,
            catalog_secrets,
            storage_secrets,
        }
    }

    fn to_value(&self) -> Result<Value, BodyError> {
        let locations_fit = (1..=MAX_LOCATIONS).contains(&self.warehouse_locations.len())
            && self
                .warehouse_locations
                .iter()
                .all(|location| plain(location, MAX_LOCATION_BYTES));
        // the host's rule for a secret: text of bounded size with no byte below 0x20 and no DEL
        let secret_fits = |secret: &String| {
            secret.len() <= MAX_SECRET_BYTES
                && !secret.chars().any(|c| c < '\u{20}' || c == '\u{7f}')
        };
        let secrets_fit = self
            .catalog_secrets
            .values()
            .chain(self.storage_secrets.values())
            .all(secret_fits);
        if !locations_fit || !secrets_fit {
            return Err(BodyError::Connection);
        }
        let mut credentials = Map::new();
        for (name, document) in [
            ("catalog", &self.catalog_secrets),
            ("storage", &self.storage_secrets),
        ] {
            if !document.is_empty() {
                credentials.insert(name.to_owned(), json!(document));
            }
        }
        Ok(json!({
            "catalog": self.catalog,
            "storage": self.storage,
            "warehouseLocations": self.warehouse_locations,
            "credentials": credentials,
        }))
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Connection")
            .field("warehouse_locations", &self.warehouse_locations.len())
            .finish_non_exhaustive()
    }
}

/// An `icebergWrite` in the append mode, as the host is asked to do it. The sections of the other modes are sent as
/// `null` until the host serves them.
#[derive(Clone)]
pub struct WriteAppend {
    table: TableIdentifier,
    reference: String,
    connection: Connection,
    inputs: Vec<InputFile>,
    limits: Limits,
    snapshot_properties: BTreeMap<String, String>,
    mapping: Vec<ColumnMapping>,
    ignore_unknown_fields: bool,
    allow_empty: bool,
    prior_attempt_may_have_left_files: bool,
}

impl WriteAppend {
    #[must_use]
    pub fn new(
        table: TableIdentifier,
        reference: &str,
        connection: Connection,
        inputs: Vec<InputFile>,
        limits: Limits,
    ) -> Self {
        Self {
            table,
            reference: reference.to_owned(),
            connection,
            inputs,
            limits,
            snapshot_properties: BTreeMap::new(),
            mapping: Vec::new(),
            ignore_unknown_fields: false,
            allow_empty: false,
            prior_attempt_may_have_left_files: false,
        }
    }

    /// A member of the snapshot summary; the key must be one of [`SNAPSHOT_PROPERTIES`] (checked by
    /// [`Self::into_body`]).
    #[must_use]
    pub fn snapshot_property(mut self, key: &str, value: &str) -> Self {
        self.snapshot_properties
            .insert(key.to_owned(), value.to_owned());
        self
    }

    #[must_use]
    pub fn map_column(mut self, mapping: ColumnMapping) -> Self {
        self.mapping.push(mapping);
        self
    }

    #[must_use]
    pub const fn ignore_unknown_fields(mut self, ignore: bool) -> Self {
        self.ignore_unknown_fields = ignore;
        self
    }

    /// Whether a write of no record is a success without a commit (`EMPTY_NO_COMMIT`) and not a failure.
    #[must_use]
    pub const fn allow_empty(mut self, allow: bool) -> Self {
        self.allow_empty = allow;
        self
    }

    /// Whether an earlier attempt of this effect may have written files the host has to look for and clean (the lease
    /// knows; the host cannot).
    #[must_use]
    pub const fn prior_attempt_may_have_left_files(mut self, may: bool) -> Self {
        self.prior_attempt_may_have_left_files = may;
        self
    }

    /// The members of the request that are not the envelope.
    ///
    /// # Errors
    /// A [`BodyError`] for the first part the host would refuse.
    pub fn into_body(self) -> Result<Map<String, Value>, BodyError> {
        if !plain(&self.reference, MAX_REFERENCE_BYTES) {
            return Err(BodyError::Reference);
        }
        if !(1..=MAX_RECORD_BLOBS).contains(&self.inputs.len()) {
            return Err(BodyError::Inputs);
        }
        if self.mapping.len() > MAX_MAPPED_COLUMNS {
            return Err(BodyError::Mapping);
        }
        if !self.snapshot_properties.iter().all(|(key, value)| {
            SNAPSHOT_PROPERTIES.contains(&key.as_str()) && plain(value, MAX_PROPERTY_BYTES)
        }) {
            return Err(BodyError::SnapshotProperty);
        }
        let connection = self.connection.to_value()?;
        let inputs = self
            .inputs
            .iter()
            .map(|input| json!({"file": format!("in/{}", input.name), "format": input.format.wire()}))
            .collect::<Vec<_>>();
        let mapping = self
            .mapping
            .iter()
            .map(|pair| json!({"field": pair.field(), "column": pair.column()}))
            .collect::<Vec<_>>();
        let members = [
            (
                "table",
                json!({"namespace": self.table.namespace(), "name": self.table.name()}),
            ),
            ("ref", json!(self.reference)),
            ("mode", json!("append")),
            ("overwrite", Value::Null),
            ("upsert", Value::Null),
            ("snapshotProperties", json!(self.snapshot_properties)),
            ("connection", connection),
            ("inputs", Value::Array(inputs)),
            ("mapping", Value::Array(mapping)),
            ("ignoreUnknownFields", json!(self.ignore_unknown_fields)),
            ("allowEmpty", json!(self.allow_empty)),
            (
                "priorAttemptMayHaveLeftFiles",
                json!(self.prior_attempt_may_have_left_files),
            ),
            ("limits", self.limits.to_value()),
        ];
        Ok(members
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect())
    }
}

/// The write carries a connection with secrets, so only its shape is shown.
impl fmt::Debug for WriteAppend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriteAppend")
            .field("table", &self.table)
            .field("reference", &self.reference)
            .field("inputs", &self.inputs.len())
            .field("mapping", &self.mapping.len())
            .finish_non_exhaustive()
    }
}

//! The receipts a lake host can report, as typed fields (`contracts/receipts.schema.json`, D035-062).
//!
//! Two readers use them: the lake runner, which reads what its host sent, and the server, which reads what the runner reported
//! (`JsonLakeReceiptCodec`, D035-142). They are here, and not in the lake runtime, so that the server does not link a runner.
//!
//! The runner never forwards what the host sent. It reads each field into a type that accepts only what the contract
//! names (a count is a canonical decimal string, an outcome is one of a closed set, a hash is 64 lowercase hex digits)
//! and builds the receipt again from those fields, so a record value, a file location or an object key cannot ride
//! along in a field the schema does not have. A field outside the contract is refused, and so is a receipt of a kind
//! the host does not produce (the PostgreSQL, ClickHouse and broker receipts are written by the runner itself).

use sagat_application::database::{ExpectedReceipt, InvalidLakeReceipt, LakeReceiptCodec};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const MAX_EFFECT_KEY_CHARS: usize = 512;
const MAX_REF_CHARS: usize = 256;

/// An unsigned integer that crosses the five languages as a decimal string with no leading zero (`0` or up to 19
/// digits), so a number never loses precision on the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count(u64);

impl Count {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// `text` as a canonical unsigned decimal, or `None`.
fn canonical_unsigned(text: &str) -> Option<u64> {
    let digits =
        !text.is_empty() && text.len() <= 19 && text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}

impl Serialize for Count {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Count {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        canonical_unsigned(&text)
            .map(Count)
            .ok_or_else(|| D::Error::custom("not a canonical count"))
    }
}

/// An Iceberg snapshot id: a signed 64-bit integer as a canonical decimal string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotId(i64);

impl SnapshotId {
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl Serialize for SnapshotId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SnapshotId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let (negative, digits) = text
            .strip_prefix('-')
            .map_or((false, text.as_str()), |rest| (true, rest));
        let canonical = !digits.is_empty()
            && digits.len() <= 19
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && (digits.len() == 1 || !digits.starts_with('0'))
            && !(negative && digits == "0");
        if !canonical {
            return Err(D::Error::custom("not a canonical snapshot id"));
        }
        text.parse()
            .map(SnapshotId)
            .map_err(|_| D::Error::custom("snapshot id out of range"))
    }
}

/// `text` of 1 to `max` characters with no control character: a label, never free text.
fn label(text: &str, max: usize) -> bool {
    let length = text.chars().count();
    (1..=max).contains(&length) && !text.chars().any(char::is_control)
}

macro_rules! label_type {
    ($(#[$doc:meta])* $name:ident, $max:expr, $what:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                if label(&text, $max) {
                    Ok(Self(text))
                } else {
                    Err(D::Error::custom($what))
                }
            }
        }
    };
}

label_type!(
    /// The effect key of a step (D035-030): 1 to 512 characters, no control character.
    EffectKey, MAX_EFFECT_KEY_CHARS, "not an effect key"
);
label_type!(
    /// The ref a write targeted: 1 to 256 characters, no control character.
    RefName, MAX_REF_CHARS, "not a ref name"
);

/// A SHA-256 digest as 64 lowercase hexadecimal digits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Sha256Hex(String);

impl<'de> Deserialize<'de> for Sha256Hex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let hex = text.len() == 64
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        if hex {
            Ok(Self(text))
        } else {
            Err(D::Error::custom("not a SHA-256 digest"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IcebergOutcome {
    Committed,
    EmptyNoCommit,
    Reverted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode {
    Append,
    Overwrite,
    Upsert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowLevelMode {
    #[serde(rename = "NONE")]
    NotUsed,
    #[serde(rename = "COPY_ON_WRITE")]
    CopyOnWrite,
}

/// The Iceberg table format versions this delivery writes (format v3 is refused, FR-035-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum FormatVersion {
    V1,
    V2,
}

impl TryFrom<u8> for FormatVersion {
    type Error = &'static str;

    fn try_from(version: u8) -> Result<Self, Self::Error> {
        match version {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            _ => Err("format version outside 1 and 2"),
        }
    }
}

impl From<FormatVersion> for u8 {
    fn from(version: FormatVersion) -> Self {
        match version {
            FormatVersion::V1 => 1,
            FormatVersion::V2 => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObjectOutcome {
    Created,
    AlreadyPresent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectFormat {
    Parquet,
    Ndjson,
}

/// The receipt of an Iceberg write or revert. It holds counts and identifiers of the commit, never a row, a column
/// name of a dropped field or a data file location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IcebergReceipt {
    pub outcome: IcebergOutcome,
    pub effect_key: EffectKey,
    pub found_by_lookup: bool,
    pub mode: WriteMode,
    #[serde(rename = "ref")]
    pub reference: RefName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<SnapshotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence_number: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_version: Option<FormatVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_level_mode: Option<RowLevelMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_added: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_removed: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys_not_found: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_files_added: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_files_removed: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_files_added: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_written: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncommitted_files: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_attempt_may_have_left_files: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored_fields: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_values: Option<Count>,
}

/// The receipt of an object-storage destination write. The object name appears only as its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectStorageReceipt {
    pub outcome: ObjectOutcome,
    pub effect_key: EffectKey,
    pub found_by_lookup: bool,
    pub format: ObjectFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_name_sha256: Option<Sha256Hex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub records: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditional_create: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub records_sha256: Option<Sha256Hex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored_fields: Option<Count>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_values: Option<Count>,
}

/// A receipt a lake host can produce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Receipt {
    // Boxed: it is the largest receipt by far, and every line the decoder reads is an `Event` of this size.
    #[serde(rename = "iceberg")]
    Iceberg(Box<IcebergReceipt>),
    #[serde(rename = "object_storage")]
    ObjectStorage(ObjectStorageReceipt),
}

/// The longest receipt: the longest line a runner reads from its host (16 KiB).
pub const MAX_RECEIPT_BYTES: usize = 16 * 1024;

/// The server's reader of the receipt of an Iceberg write (D035-142): the runner's report is read into typed fields, checked against
/// the step it was reported for, and written again from those fields. What the server stores is that, never what the runner sent.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonLakeReceiptCodec;

const fn mode_of(mode: sagat_domain::database::WriteMode) -> WriteMode {
    match mode {
        sagat_domain::database::WriteMode::Append => WriteMode::Append,
        sagat_domain::database::WriteMode::Overwrite => WriteMode::Overwrite,
        sagat_domain::database::WriteMode::Upsert => WriteMode::Upsert,
    }
}

impl LakeReceiptCodec for JsonLakeReceiptCodec {
    fn rebuild(
        &self,
        receipt: &[u8],
        expected: &ExpectedReceipt<'_>,
    ) -> Result<Vec<u8>, InvalidLakeReceipt> {
        if receipt.len() > MAX_RECEIPT_BYTES {
            return Err(InvalidLakeReceipt::TooLarge);
        }
        let iceberg =
            match serde_json::from_slice(receipt).map_err(|_| InvalidLakeReceipt::Malformed)? {
                Receipt::Iceberg(iceberg) => iceberg,
                Receipt::ObjectStorage(_) => return Err(InvalidLakeReceipt::Kind),
            };
        // The revert of a commit is the receipt of a compensation; an Iceberg write ends in a commit or in nothing to commit.
        if iceberg.outcome == IcebergOutcome::Reverted {
            return Err(InvalidLakeReceipt::Outcome);
        }
        if iceberg.effect_key.as_str() != expected.effect_key
            || iceberg.reference.as_str() != expected.reference
            || iceberg.mode != mode_of(expected.mode)
        {
            return Err(InvalidLakeReceipt::Mismatch);
        }
        serde_json::to_vec(&Receipt::Iceberg(iceberg)).map_err(|_| InvalidLakeReceipt::Malformed)
    }
}

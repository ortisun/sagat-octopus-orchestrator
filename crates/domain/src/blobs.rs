//! Blobs: payloads too large to live in the database (FR-002-008, ADR-0023).
//!
//! The engine never carries a large payload. It carries a small *descriptor* of it (identity, size,
//! digest, media type) as an ordinary payload, and the bytes live in an object store behind a port.
//! This module owns only the vocabulary and the rules that do not depend on any store: what a
//! descriptor is, which media types are acceptable and the order in which a blob may be used.

use thiserror::Error;

use crate::workflow::Digest;

uuid_id!(
    /// Identifier of a blob. Unguessable (a random-looking UUID) and, in the store, always found
    /// under the tenant that owns it, so knowing an id never opens another tenant's bytes.
    BlobId
);

/// Why a blob rule was not met.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BlobError {
    /// The text is not a media type (`type/subtype`) this platform accepts.
    #[error("not an acceptable media type")]
    InvalidContentType,
    /// The blob is not in a state that allows the operation (not uploaded yet, or already used).
    #[error("the blob cannot be used in its current state")]
    NotUsable,
}

/// Longest media type kept (RFC 6838 allows 127 characters for each side; one is plenty).
const MAX_CONTENT_TYPE: usize = 127;

/// A media type such as `application/json`, lower-cased and without parameters.
///
/// It is echoed back to clients as the blob's `Content-Type`, so it is restricted to the characters
/// of RFC 6838 §4.2 and can never carry a line break or a second header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentType(String);

impl ContentType {
    /// Opaque bytes, the type of a blob whose producer said nothing.
    #[must_use]
    pub fn octet_stream() -> Self {
        Self("application/octet-stream".to_owned())
    }

    /// Accepts `type/subtype`, optionally followed by `;` parameters, which are dropped.
    ///
    /// # Errors
    /// [`BlobError::InvalidContentType`] for anything that is not a plain media type.
    pub fn parse(text: &str) -> Result<Self, BlobError> {
        let essence = text.split(';').next().unwrap_or_default().trim();
        if !(3..=MAX_CONTENT_TYPE).contains(&essence.len()) || !essence.is_ascii() {
            return Err(BlobError::InvalidContentType);
        }
        let essence = essence.to_ascii_lowercase();
        let mut sides = essence.split('/');
        let (Some(kind), Some(subtype), None) = (sides.next(), sides.next(), sides.next()) else {
            return Err(BlobError::InvalidContentType);
        };
        let valid_side = |side: &str| {
            let mut chars = side.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
                && side
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "!#$&-^_.+".contains(c))
        };
        if valid_side(kind) && valid_side(subtype) {
            Ok(Self(essence))
        } else {
            Err(BlobError::InvalidContentType)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What identifies a stored blob: enough to find it, to check it and to decide how to serve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobDescriptor {
    id: BlobId,
    size: u64,
    digest: Digest,
    content_type: ContentType,
}

impl BlobDescriptor {
    #[must_use]
    pub const fn new(id: BlobId, size: u64, digest: Digest, content_type: ContentType) -> Self {
        Self {
            id,
            size,
            digest,
            content_type,
        }
    }

    #[must_use]
    pub const fn id(&self) -> BlobId {
        self.id
    }

    /// Length of the content in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// SHA-256 of the content.
    #[must_use]
    pub const fn digest(&self) -> &Digest {
        &self.digest
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }
}

/// Where a blob is in its life. It only moves forward.
///
/// `Uploading` → `Uploaded` when the store confirmed all the bytes; `Uploaded` → `Committed` when a
/// run or a step output that needs them is stored, in the same transaction as that record. An
/// unused blob moves to `Deleting` before the external delete starts; the catalog row remains there
/// until the backend confirms deletion. An upload whose completion was never confirmed retains
/// a deleting tombstone afterward: a late PUT still needs reconciliation after a successful delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlobState {
    Uploading,
    Uploaded,
    Committed,
    Deleting,
}

impl BlobState {
    /// The state after the store confirmed the bytes.
    ///
    /// # Errors
    /// [`BlobError::NotUsable`] from any state but `Uploading`.
    pub const fn uploaded(self) -> Result<Self, BlobError> {
        match self {
            Self::Uploading => Ok(Self::Uploaded),
            Self::Uploaded | Self::Committed | Self::Deleting => Err(BlobError::NotUsable),
        }
    }

    /// The state after something durable started to reference the blob.
    ///
    /// # Errors
    /// [`BlobError::NotUsable`] unless the blob is `Uploaded`: not before its bytes are safe, and
    /// never twice (a blob belongs to one use).
    pub const fn committed(self) -> Result<Self, BlobError> {
        match self {
            Self::Uploaded => Ok(Self::Committed),
            Self::Uploading | Self::Committed | Self::Deleting => Err(BlobError::NotUsable),
        }
    }

    /// Whether the system refers to the blob from a stored record, so it must not be swept.
    #[must_use]
    pub const fn is_referenced(self) -> bool {
        matches!(self, Self::Committed)
    }

    /// The form that is stored.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uploading => "UPLOADING",
            Self::Uploaded => "UPLOADED",
            Self::Committed => "COMMITTED",
            Self::Deleting => "DELETING",
        }
    }

    /// Reads back a stored state; `None` for anything this version does not write.
    #[must_use]
    pub fn parse_stored(text: &str) -> Option<Self> {
        match text {
            "UPLOADING" => Some(Self::Uploading),
            "UPLOADED" => Some(Self::Uploaded),
            "COMMITTED" => Some(Self::Committed),
            "DELETING" => Some(Self::Deleting),
            _ => None,
        }
    }
}

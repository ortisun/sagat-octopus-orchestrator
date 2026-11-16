//! Where the items of a collection can come from when they are not in the run's own data
//! (feature 014, T014-016, D420-D428).
//!
//! An `items` expression normally yields the array itself. When the array is too large to be a run
//! value it is a blob (ADR-0023) and the expression yields its descriptor,
//! `{"$blob": {"id", "size", "sha256", "contentType"}}`. The scheduler then asks a
//! [`CollectionItemSource`] to materialize the array once, at entry. Membership is fixed from that
//! moment: the items are stored under their own digests with the scope, and the blob is never read
//! again for this collection, so a retry, a restart or a replica takeover cannot see a different
//! array.
//!
//! The reader is the only thing that opens bytes on behalf of a run, so it is also where the
//! boundaries are decided:
//!
//! - the blob must be in the run's own tenant **and** namespace, and committed (a blob that nothing
//!   has used yet may still be swept);
//! - a descriptor that an author typed into a JSON input is not a capability: the blob must be this
//!   run's bound input or the committed output of one of this run's steps;
//! - the bytes are checked against the size and digest the catalog recorded while they are read;
//! - the array is bounded before it becomes anything durable: `maxItems`, 256 KiB per item and
//!   4 MiB of items in all (the retained-run-data ceiling).

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use sagat_domain::blobs::{BlobDescriptor, BlobId, BlobState, ContentType};
use sagat_domain::execution::RunId;
use sagat_domain::workflow::Digest;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::blobs::{BlobAccess, BlobIoError, BlobOwner, BlobReadError, BlobService, BoxFuture};
use crate::execution::{MAX_PAYLOAD_BYTES, MAX_RUN_OUTPUT_BYTES};

/// The largest blob a collection reads for its items. Bigger than the items it may keep (4 MiB)
/// because the source may be pretty-printed; the canonical items are bounded separately.
pub const MAX_SOURCE_BLOB_BYTES: u64 = 8 * 1024 * 1024;

/// What an `items` expression selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionSource {
    /// The array itself: canonical JSON of each element, in order.
    Items(Vec<Vec<u8>>),
    /// A blob whose content is the array.
    Blob(BlobDescriptor),
}

/// Why a blob could not become the items of a collection. The first five are permanent for this
/// run; [`Self::Unavailable`] is the only one worth retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionSourceError {
    /// No such blob for this run: missing, in another tenant or namespace, not committed, not the
    /// run's own, or its descriptor disagrees with the catalog. They look the same on purpose.
    NotFound,
    /// The catalog knows the blob and the store lost the bytes.
    Lost,
    /// The bytes read back are not the bytes that were stored.
    Corrupt,
    /// The content is valid for its digest but is not a JSON array.
    NotAnArray,
    /// More elements than the node's `maxItems`.
    TooManyItems,
    /// The blob, an element or the elements together are over a byte ceiling.
    TooLarge,
    /// The store or the catalog could not be used right now.
    Unavailable,
}

impl fmt::Display for CollectionSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "the collection source blob is not available to this run",
            Self::Lost => "the collection source blob has no bytes in the store",
            Self::Corrupt => "the collection source blob does not match its digest",
            Self::NotAnArray => "the collection source blob is not a JSON array",
            Self::TooManyItems => "the collection source has more items than maxItems",
            Self::TooLarge => "the collection source is over a byte ceiling",
            Self::Unavailable => "the collection source could not be read right now",
        })
    }
}

impl std::error::Error for CollectionSourceError {}

/// What to read and for whom.
#[derive(Debug, Clone)]
pub struct CollectionSourceRequest {
    pub access: BlobAccess,
    pub run_id: RunId,
    /// The blob bound as the run's input, if any.
    pub run_input: Option<BlobId>,
    pub descriptor: BlobDescriptor,
    pub max_items: u32,
}

/// The materialized array and the record that pins it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceItems {
    pub items: Vec<Vec<u8>>,
    pub manifest: SourceManifest,
}

/// What a scope keeps in place of the array when the array came from a blob: the descriptor that
/// was read, how many elements it had and the digest of the canonical array they make. Together
/// with the per-item payloads it pins membership without a snapshot payload of the array's size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceManifest {
    blob: BlobDescriptor,
    count: u32,
    items_digest: Digest,
}

const MANIFEST_KEY: &str = "$collectionSource";

impl SourceManifest {
    #[must_use]
    pub const fn new(blob: BlobDescriptor, count: u32, items_digest: Digest) -> Self {
        Self {
            blob,
            count,
            items_digest,
        }
    }

    #[must_use]
    pub const fn blob(&self) -> &BlobDescriptor {
        &self.blob
    }

    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    #[must_use]
    pub const fn items_digest(&self) -> &Digest {
        &self.items_digest
    }

    /// The bytes stored as the scope's snapshot payload.
    #[must_use]
    pub fn to_json(&self) -> Vec<u8> {
        format!(
            "{{\"{MANIFEST_KEY}\":{{\"blob\":{{\"id\":\"{}\",\"size\":{},\"sha256\":\"{}\",\"contentType\":\"{}\"}},\"count\":{},\"itemsSha256\":\"{}\"}}}}",
            self.blob.id(),
            self.blob.size(),
            self.blob.digest().to_hex(),
            self.blob.content_type().as_str(),
            self.count,
            self.items_digest.to_hex(),
        )
        .into_bytes()
    }

    /// Reads a stored manifest; `None` for anything else, including an inline snapshot array.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let object = value.as_object()?;
        if object.len() != 1 {
            return None;
        }
        let body = object.get(MANIFEST_KEY)?.as_object()?;
        if body.len() != 3 {
            return None;
        }
        let blob = descriptor_from_value(body.get("blob")?)?;
        let count = u32::try_from(body.get("count")?.as_u64()?).ok()?;
        let items_digest = Digest::from_hex(body.get("itemsSha256")?.as_str()?)?;
        Some(Self {
            blob,
            count,
            items_digest,
        })
    }
}

const CHAINED_MANIFEST_KEY: &str = "$collectionChainSource";

/// What a scope keeps when its items came from a blob through an operator chain (design 042, FR-042-010, D042-020):
/// the blob that was read with its element count and digest (provenance: the restore never reads the blob again), the
/// chain's digest, and the count and digest of the chain's output, which are the items stored per item. It is under a
/// key of its own, so a reader that only knows [`SourceManifest`] does not take it for one: `SourceManifest::parse`
/// answers `None` and that reader's restore refuses the scope as invalid storage instead of misreading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainedSourceManifest {
    blob: BlobDescriptor,
    source_count: u32,
    source_digest: Digest,
    chain_digest: Digest,
    count: u32,
    items_digest: Digest,
}

impl ChainedSourceManifest {
    #[must_use]
    pub fn new(
        source: &SourceManifest,
        chain_digest: Digest,
        count: u32,
        items_digest: Digest,
    ) -> Self {
        Self {
            blob: source.blob.clone(),
            source_count: source.count,
            source_digest: source.items_digest,
            chain_digest,
            count,
            items_digest,
        }
    }

    #[must_use]
    pub const fn blob(&self) -> &BlobDescriptor {
        &self.blob
    }

    #[must_use]
    pub const fn source_count(&self) -> u32 {
        self.source_count
    }

    #[must_use]
    pub const fn source_digest(&self) -> &Digest {
        &self.source_digest
    }

    #[must_use]
    pub const fn chain_digest(&self) -> &Digest {
        &self.chain_digest
    }

    /// Elements of the chain's output: the items the scope stores.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }

    /// Digest of the canonical array of the chain's output.
    #[must_use]
    pub const fn items_digest(&self) -> &Digest {
        &self.items_digest
    }

    /// The bytes stored as the scope's snapshot payload: its canonical JSON, every key in code-point order.
    #[must_use]
    pub fn to_json(&self) -> Vec<u8> {
        format!(
            "{{\"{CHAINED_MANIFEST_KEY}\":{{\"blob\":{{\"contentType\":\"{}\",\"id\":\"{}\",\"sha256\":\"{}\",\"size\":{}}},\"chainSha256\":\"{}\",\"count\":{},\"itemsSha256\":\"{}\",\"sourceCount\":{},\"sourceSha256\":\"{}\"}}}}",
            self.blob.content_type().as_str(),
            self.blob.id(),
            self.blob.digest().to_hex(),
            self.blob.size(),
            self.chain_digest.to_hex(),
            self.count,
            self.items_digest.to_hex(),
            self.source_count,
            self.source_digest.to_hex(),
        )
        .into_bytes()
    }

    /// Reads a stored chained manifest; `None` for anything else, a plain [`SourceManifest`] included.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let object = value.as_object()?;
        if object.len() != 1 {
            return None;
        }
        let body = object.get(CHAINED_MANIFEST_KEY)?.as_object()?;
        if body.len() != 6 {
            return None;
        }
        let count = |field: &str| u32::try_from(body.get(field)?.as_u64()?).ok();
        let digest = |field: &str| Digest::from_hex(body.get(field)?.as_str()?);
        Some(Self {
            blob: descriptor_from_value(body.get("blob")?)?,
            source_count: count("sourceCount")?,
            source_digest: digest("sourceSha256")?,
            chain_digest: digest("chainSha256")?,
            count: count("count")?,
            items_digest: digest("itemsSha256")?,
        })
    }
}

/// The manifest a blob-sourced scope stores, either kind: what entry and restore check the stored items against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobManifest {
    Source(SourceManifest),
    Chained(ChainedSourceManifest),
}

impl BlobManifest {
    /// `None` for anything that is neither, an inline snapshot array included.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        SourceManifest::parse(bytes)
            .map(Self::Source)
            .or_else(|| ChainedSourceManifest::parse(bytes).map(Self::Chained))
    }

    #[must_use]
    pub const fn blob(&self) -> &BlobDescriptor {
        match self {
            Self::Source(manifest) => manifest.blob(),
            Self::Chained(manifest) => manifest.blob(),
        }
    }

    /// The items the scope stores.
    #[must_use]
    pub const fn count(&self) -> u32 {
        match self {
            Self::Source(manifest) => manifest.count(),
            Self::Chained(manifest) => manifest.count(),
        }
    }

    /// The digest of the items the scope stores.
    #[must_use]
    pub const fn items_digest(&self) -> &Digest {
        match self {
            Self::Source(manifest) => manifest.items_digest(),
            Self::Chained(manifest) => manifest.items_digest(),
        }
    }
}

/// The digest of the canonical array made of `items` (compact JSON, elements in order).
#[must_use]
pub fn items_digest(items: &[Vec<u8>]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"[");
    for (position, item) in items.iter().enumerate() {
        if position > 0 {
            hasher.update(b",");
        }
        hasher.update(item);
    }
    hasher.update(b"]");
    Digest::new(hasher.finalize().into())
}

/// The descriptor in `{"$blob": {...}}`, if `bytes` is exactly that and nothing else.
#[must_use]
pub fn parse_descriptor(bytes: &[u8]) -> Option<BlobDescriptor> {
    descriptor_from_json(&serde_json::from_slice(bytes).ok()?)
}

/// As [`parse_descriptor`], from a value already parsed.
#[must_use]
pub fn descriptor_from_json(value: &Value) -> Option<BlobDescriptor> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    descriptor_from_value(object.get("$blob")?)
}

fn descriptor_from_value(value: &Value) -> Option<BlobDescriptor> {
    let object = value.as_object()?;
    if object.len() != 4 {
        return None;
    }
    Some(BlobDescriptor::new(
        object.get("id")?.as_str()?.parse().ok()?,
        object.get("size")?.as_u64()?,
        Digest::from_hex(object.get("sha256")?.as_str()?)?,
        ContentType::parse(object.get("contentType")?.as_str()?).ok()?,
    ))
}

/// Reads the array a descriptor stands for. Implemented over [`BlobService`]; the scheduler holds
/// it as a trait object so it depends on no store.
pub trait CollectionItemSource: Send + Sync {
    /// # Errors
    /// [`CollectionSourceError`]; only [`CollectionSourceError::Unavailable`] may succeed later.
    fn materialize<'a>(
        &'a self,
        request: &'a CollectionSourceRequest,
    ) -> BoxFuture<'a, Result<SourceItems, CollectionSourceError>>;
}

impl CollectionItemSource for BlobService {
    fn materialize<'a>(
        &'a self,
        request: &'a CollectionSourceRequest,
    ) -> BoxFuture<'a, Result<SourceItems, CollectionSourceError>> {
        Box::pin(async move {
            let record = self
                .catalog()
                .find(request.access, request.descriptor.id())
                .await
                .map_err(|_| CollectionSourceError::Unavailable)?
                .ok_or(CollectionSourceError::NotFound)?;
            let own = request.run_input == Some(record.descriptor.id())
                || matches!(record.owner, BlobOwner::StepOutput(scope) if scope.run_id == request.run_id);
            // A descriptor in a JSON value proves nothing: only what the catalog says about the
            // blob counts, and it must say the run owns it, the blob is used and the numbers agree.
            if record.state != BlobState::Committed
                || !own
                || record.descriptor != request.descriptor
            {
                return Err(CollectionSourceError::NotFound);
            }
            if record.descriptor.size() > MAX_SOURCE_BLOB_BYTES {
                return Err(CollectionSourceError::TooLarge);
            }
            let (descriptor, mut stream) = self
                .read(request.access, record.descriptor.id())
                .await
                .map_err(|error| match error {
                    BlobReadError::NotFound => CollectionSourceError::NotFound,
                    BlobReadError::Lost => CollectionSourceError::Lost,
                    BlobReadError::Unavailable => CollectionSourceError::Unavailable,
                })?;
            let capacity =
                usize::try_from(descriptor.size()).map_err(|_| CollectionSourceError::TooLarge)?;
            let mut bytes = Vec::with_capacity(capacity);
            while let Some(chunk) = next(&mut stream).await {
                match chunk {
                    Ok(chunk) => bytes.extend_from_slice(&chunk),
                    Err(BlobIoError::Corrupt) => return Err(CollectionSourceError::Corrupt),
                    Err(BlobIoError::TooLarge) => return Err(CollectionSourceError::TooLarge),
                    Err(BlobIoError::Interrupted) => {
                        return Err(CollectionSourceError::Unavailable);
                    }
                }
            }
            // Parsing up to 8 MiB is CPU work: off the runtime's workers.
            let max_items = request.max_items;
            let items = tokio::task::spawn_blocking(move || canonical_items(&bytes, max_items))
                .await
                .map_err(|_| CollectionSourceError::Unavailable)??;
            let digest = items_digest(&items);
            let count =
                u32::try_from(items.len()).map_err(|_| CollectionSourceError::TooManyItems)?;
            Ok(SourceItems {
                items,
                manifest: SourceManifest::new(descriptor, count, digest),
            })
        })
    }
}

/// The elements of the JSON array in `bytes`, each as canonical compact JSON.
///
/// The array is read element by element: the count and the byte ceilings are checked as each element is read, and the
/// read stops at the first crossing (at `max_items + 1` elements for the count), so a source far over a ceiling is never
/// parsed in full (design 042 review).
///
/// # Errors
/// [`CollectionSourceError::NotAnArray`], [`CollectionSourceError::TooManyItems`] or
/// [`CollectionSourceError::TooLarge`].
pub fn canonical_items(
    bytes: &[u8],
    max_items: u32,
) -> Result<Vec<Vec<u8>>, CollectionSourceError> {
    use serde::de::{Deserializer as _, Error as _, SeqAccess, Visitor};
    use std::cell::Cell;

    struct Items<'a> {
        max: usize,
        refused: &'a Cell<Option<CollectionSourceError>>,
    }

    impl<'de> Visitor<'de> for Items<'_> {
        type Value = Vec<Vec<u8>>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a JSON array")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let refuse = |error| {
                self.refused.set(Some(error));
                A::Error::custom("refused")
            };
            let mut total = 0usize;
            let mut items = Vec::new();
            while let Some(value) = seq.next_element::<Value>()? {
                if items.len() >= self.max {
                    return Err(refuse(CollectionSourceError::TooManyItems));
                }
                let item = serde_json::to_vec(&value)
                    .map_err(|_| refuse(CollectionSourceError::NotAnArray))?;
                total = total.saturating_add(item.len());
                if item.len() > MAX_PAYLOAD_BYTES || total > MAX_RUN_OUTPUT_BYTES {
                    return Err(refuse(CollectionSourceError::TooLarge));
                }
                items.push(item);
            }
            Ok(items)
        }
    }

    let refused = Cell::new(None);
    let mut reader = serde_json::Deserializer::from_slice(bytes);
    let parsed = reader.deserialize_seq(Items {
        max: usize::try_from(max_items).unwrap_or(usize::MAX),
        refused: &refused,
    });
    if let Some(error) = refused.take() {
        return Err(error);
    }
    let items = parsed.map_err(|_| CollectionSourceError::NotAnArray)?;
    reader
        .end()
        .map_err(|_| CollectionSourceError::NotAnArray)?;
    Ok(items)
}

async fn next(
    stream: &mut Pin<Box<dyn Stream<Item = Result<bytes::Bytes, BlobIoError>> + Send>>,
) -> Option<Result<bytes::Bytes, BlobIoError>> {
    std::future::poll_fn(|cx: &mut Context<'_>| -> Poll<_> { stream.as_mut().poll_next(cx) }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> BlobDescriptor {
        BlobDescriptor::new(
            BlobId::generate(),
            1234,
            Digest::new([7; 32]),
            ContentType::parse("application/json").expect("media type"),
        )
    }

    #[test]
    fn a_manifest_round_trips_and_an_inline_array_is_not_one() {
        let manifest = SourceManifest::new(descriptor(), 3, Digest::new([9; 32]));
        assert_eq!(SourceManifest::parse(&manifest.to_json()), Some(manifest));
        assert_eq!(SourceManifest::parse(b"[1,2,3]"), None);
        assert_eq!(SourceManifest::parse(br#"{"a":1}"#), None);
        assert_eq!(
            SourceManifest::parse(br#"{"$collectionSource":{"count":1}}"#),
            None
        );
    }

    #[test]
    fn a_chained_manifest_round_trips_under_its_own_key_and_an_old_reader_does_not_take_it() {
        let source = SourceManifest::new(descriptor(), 7, Digest::new([1; 32]));
        let chained =
            ChainedSourceManifest::new(&source, Digest::new([2; 32]), 3, Digest::new([3; 32]));
        let bytes = chained.to_json();
        assert_eq!(ChainedSourceManifest::parse(&bytes), Some(chained.clone()));
        assert_eq!(
            BlobManifest::parse(&bytes),
            Some(BlobManifest::Chained(chained.clone()))
        );
        assert_eq!(
            SourceManifest::parse(&bytes),
            None,
            "a reader that knows only $collectionSource does not read a chained manifest as one"
        );
        assert_eq!(ChainedSourceManifest::parse(&source.to_json()), None);
        assert_eq!(
            BlobManifest::parse(&source.to_json()),
            Some(BlobManifest::Source(source))
        );
        let value: Value = serde_json::from_slice(&bytes).expect("json");
        let body = value["$collectionChainSource"]
            .as_object()
            .expect("the chained manifest key");
        assert_eq!(
            body.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "blob",
                "chainSha256",
                "count",
                "itemsSha256",
                "sourceCount",
                "sourceSha256"
            ]
        );
        assert_eq!(value["$collectionChainSource"]["sourceCount"], 7);
        assert_eq!(value["$collectionChainSource"]["count"], 3);
        let canonical = serde_json::to_vec(&value).expect("json");
        assert_eq!(
            canonical, bytes,
            "the manifest is written in its canonical form"
        );
        let mut extra = value.clone();
        extra["$collectionChainSource"]["more"] = Value::Bool(true);
        assert_eq!(
            ChainedSourceManifest::parse(&serde_json::to_vec(&extra).expect("json")),
            None
        );
        assert_eq!(BlobManifest::parse(b"[1,2]"), None);
    }

    #[test]
    fn only_an_exact_descriptor_is_a_blob_reference() {
        let descriptor = descriptor();
        let json = crate::blobs::descriptor_json(&descriptor);
        assert_eq!(parse_descriptor(&json), Some(descriptor.clone()));
        // Anything extra, missing or mistyped is a value, not a reference.
        let mut value: Value = serde_json::from_slice(&json).expect("json");
        value["extra"] = Value::Bool(true);
        assert_eq!(descriptor_from_json(&value), None);
        let mut value: Value = serde_json::from_slice(&json).expect("json");
        value["$blob"]["size"] = Value::String("1".into());
        assert_eq!(descriptor_from_json(&value), None);
        let mut value: Value = serde_json::from_slice(&json).expect("json");
        value["$blob"]["id"] = Value::String("not-a-uuid".into());
        assert_eq!(descriptor_from_json(&value), None);
        assert_eq!(parse_descriptor(b"[]"), None);
    }

    #[test]
    fn items_are_canonical_ordered_and_bounded() {
        let items = canonical_items(br#" [ {"b":1,"a":2}, 3, "x" ] "#, 3).expect("items");
        assert_eq!(
            items,
            vec![
                br#"{"a":2,"b":1}"#.to_vec(),
                b"3".to_vec(),
                br#""x""#.to_vec()
            ]
        );
        assert_eq!(
            canonical_items(b"[1,2,3]", 2),
            Err(CollectionSourceError::TooManyItems)
        );
        assert_eq!(canonical_items(b"[]", 1), Ok(Vec::new()));
        for bad in [&b"{}"[..], b"1", b"null", b"[1,", b"", b"\xff"] {
            assert_eq!(
                canonical_items(bad, 10),
                Err(CollectionSourceError::NotAnArray),
                "{bad:?}"
            );
        }
        let one_big = format!("[\"{}\"]", "x".repeat(MAX_PAYLOAD_BYTES));
        assert_eq!(
            canonical_items(one_big.as_bytes(), 10),
            Err(CollectionSourceError::TooLarge)
        );
        let item = format!("\"{}\"", "y".repeat(MAX_PAYLOAD_BYTES - 16));
        let together = format!("[{}]", vec![item; 17].join(","));
        assert_eq!(
            canonical_items(together.as_bytes(), 100),
            Err(CollectionSourceError::TooLarge),
            "items under 256 KiB each are still bounded together by 4 MiB"
        );
    }

    /// Design 042 review: a source is read element by element and stops at `max_items + 1`, so a blob far over the
    /// count is refused without building the rest of it (what follows the extra element is never parsed).
    #[test]
    fn an_array_over_max_items_is_refused_before_the_rest_is_read() {
        assert_eq!(
            canonical_items(b"[1, 2, 3, this is never read", 2),
            Err(CollectionSourceError::TooManyItems)
        );
        assert_eq!(
            canonical_items(b"[1, 2, oops]", 2),
            Err(CollectionSourceError::NotAnArray),
            "within the count, a malformed element is still malformed"
        );
        assert_eq!(
            canonical_items(b" [ 1 , 2 ] ", 2),
            Ok(vec![b"1".to_vec(), b"2".to_vec()])
        );
        assert_eq!(
            canonical_items(b"[1, 2] trailing", 2),
            Err(CollectionSourceError::NotAnArray)
        );
    }

    #[test]
    fn the_items_digest_pins_order_and_content() {
        let a = vec![b"1".to_vec(), b"2".to_vec()];
        let b = vec![b"2".to_vec(), b"1".to_vec()];
        assert_eq!(items_digest(&a), items_digest(&a.clone()));
        assert_ne!(items_digest(&a), items_digest(&b));
        assert_eq!(
            items_digest(&a),
            Digest::new(Sha256::digest(b"[1,2]").into()),
            "the digest is that of the canonical array"
        );
        assert_eq!(items_digest(&[]), Digest::new(Sha256::digest(b"[]").into()));
    }
}

//! Blobs: payloads too large for the database live in an object store (FR-002-008, ADR-0023).
//!
//! The engine never carries a large payload. A step output or a run input that is too big is written
//! here first and what the journal, the expressions and the next step see is a small *descriptor*
//! (`{"$blob": {"id", "size", "sha256", "contentType"}}`); a worker that needs the bytes asks for them
//! by id. Two ports keep the rest of the system independent of where the bytes are:
//!
//! - [`BlobBackend`] moves bytes: S3, Cloudflare R2, memory, a directory. It knows nothing of
//!   tenants, states or limits.
//! - [`BlobCatalog`] is the record of what exists (PostgreSQL): the tenant, the state, the size and
//!   the digest, written in the transactions that decide whether a blob is used.
//!
//! [`BlobService`] is the use case between them: it reserves, measures while it streams, verifies,
//! scopes every key by tenant and sweeps what was never used. Nothing here buffers a whole blob.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_core::Stream;
use sagat_domain::blobs::{BlobDescriptor, BlobId, BlobState, ContentType};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::Digest;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::error::StoreError;

/// Largest blob accepted unless the operator says otherwise.
pub const DEFAULT_MAX_BLOB_BYTES: u64 = 64 * 1024 * 1024;
/// Most blobs of one tenant that may be uploaded and not yet used by anything.
pub const DEFAULT_MAX_PENDING_BLOBS: u32 = 64;
/// How long a blob that was uploaded and that nothing refers to is kept before the sweep removes it.
pub const DEFAULT_UPLOAD_TTL: Duration = Duration::from_secs(24 * 3600);
/// How long a blob whose upload never finished is kept. Much shorter: nothing can be waiting for it,
/// and a client that keeps dropping its connections would otherwise use up its budget for a day.
pub const DEFAULT_ABANDONED_AFTER: Duration = Duration::from_secs(3600);
/// Delay before retrying an external delete that failed. The catalog records each claim before the
/// delete starts, so a process crash or provider outage leaves durable work for a later sweep.
pub const DEFAULT_DELETE_RETRY: Duration = Duration::from_secs(5 * 60);

/// An owned, boxed future: the ports are used as trait objects because the provider is chosen at
/// start-up from configuration.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A body that is read once, in chunks, never as a whole.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, BlobIoError>> + Send>>;

/// Why moving bytes stopped. Carries no text: the cause is logged where it happened, and nothing
/// that came from a store (an endpoint, a bucket, a message) is ever returned to a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobIoError {
    /// The producer of the body failed or went away before the end.
    Interrupted,
    /// The body is larger than the limit.
    TooLarge,
    /// The bytes read back are not the bytes that were stored.
    Corrupt,
}

impl fmt::Display for BlobIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Interrupted => "the body was interrupted",
            Self::TooLarge => "the body is larger than the limit",
            Self::Corrupt => "the content does not match its digest",
        })
    }
}

impl std::error::Error for BlobIoError {}

/// What a [`BlobBackend`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendFailure {
    /// There is no object under that key.
    NotFound,
    /// The store could not be reached, or answered with a failure that a retry may cure.
    Unavailable,
    /// The store refused the request: credentials, permissions or configuration. A retry will not
    /// help; an operator must.
    Refused,
    /// The body handed to `put` failed; the upload was abandoned.
    Body(BlobIoError),
}

/// Where an object lives in a store. Built only from a tenant id and a blob id, so nothing a client
/// chose can reach it, and the same blob id under two tenants is two different objects.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlobKey(String);

/// Tenant and namespace boundary within which a live worker lease may read a descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobAccess {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
}

/// The attempt that uploaded a blob. Only this attempt may claim it as its output; after commit,
/// later steps may read the descriptor under any live lease in the same [`BlobAccess`] boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlobScope {
    pub access: BlobAccess,
    pub run_id: RunId,
    pub step_id: Uuid,
    pub attempt: u32,
    pub generation: u64,
}

/// The capability that may bind a staged upload. A run input has no run or attempt yet; its
/// uploader is recorded separately by the catalog and checked when a run claims it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlobOwner {
    StepOutput(BlobScope),
    RunInput(BlobAccess),
}

impl BlobOwner {
    #[must_use]
    pub const fn access(self) -> BlobAccess {
        match self {
            Self::StepOutput(scope) => scope.access,
            Self::RunInput(access) => access,
        }
    }
}

impl BlobKey {
    #[must_use]
    pub fn of(access: BlobAccess, id: BlobId) -> Self {
        Self(format!(
            "tenants/{}/namespaces/{}/blobs/{id}",
            access.tenant_id, access.namespace_id
        ))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Moves bytes to and from one object store.
///
/// Implementations are given a key, never a path or a URL, stream in both directions and delete
/// idempotently. They enforce nothing else: sizes, digests, tenants and states are the service's.
pub trait BlobBackend: Send + Sync + fmt::Debug + 'static {
    /// A short name for logs and metrics: `s3`, `r2`, `memory`, `fs`. Never a bucket or an endpoint.
    fn provider(&self) -> &'static str;

    /// Stores the whole body under `key`. A failed upload must not expose a complete object at the
    /// key. A provider may retain incomplete multipart parts until abort or lifecycle cleanup; they
    /// are not readable through `get` and the deployment must expire them.
    fn put<'a>(
        &'a self,
        key: &'a BlobKey,
        body: ByteStream,
    ) -> BoxFuture<'a, Result<(), BackendFailure>>;

    /// The object as a stream.
    fn get<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<ByteStream, BackendFailure>>;

    /// Removes the object; removing one that does not exist is not an error.
    fn delete<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<(), BackendFailure>>;
}

/// What the catalog knows about a blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRecord {
    pub descriptor: BlobDescriptor,
    pub state: BlobState,
    pub owner: BlobOwner,
    pub created_by: String,
}

/// A durable deletion attempt whose backend acknowledgement may finalize only this exact claim.
/// Carrying the scope, blob and monotonically increasing attempt together prevents a delayed
/// response from an older DELETE from releasing bytes covered by a newer cleanup attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlobDeleteClaim {
    pub access: BlobAccess,
    pub id: BlobId,
    pub attempt: u32,
}

/// The record of which blobs exist, per tenant and namespace.
///
/// Every lookup is scoped by both boundaries, so a blob from another tenant or namespace is simply
/// not there. Times are the database's own clock.
pub trait BlobCatalog: Send + Sync + fmt::Debug + 'static {
    /// Reserves `id` and `reservation_bytes` (state `Uploading`) before any byte is stored. The
    /// tenant may hold at most `max_pending` active blobs that nothing uses yet:
    /// [`StoreError::QuotaExceeded`] beyond either bound. Confirmed cleanup tombstones retain
    /// reconciliation evidence without consuming an active slot or releasing their byte
    /// reservation while a PUT may still finish late.
    fn reserve<'a>(
        &'a self,
        owner: BlobOwner,
        id: BlobId,
        content_type: &'a ContentType,
        created_by: &'a str,
        reservation_bytes: u64,
        max_pending: u32,
    ) -> BoxFuture<'a, Result<(), StoreError>>;

    /// Records that the store acknowledged all `size` bytes with this `digest`. This atomically
    /// converts the precharged reservation to measured retained content. `Uploading` becomes
    /// `Uploaded`; a concurrent `Deleting` remains `Deleting` and returns
    /// [`StoreError::MissingReference`] after recording the measurement, so the caller deletes the
    /// acknowledged late PUT without ever exposing it as usable.
    fn mark_uploaded<'a>(
        &'a self,
        access: BlobAccess,
        id: BlobId,
        size: u64,
        digest: &'a Digest,
    ) -> BoxFuture<'a, Result<(), StoreError>>;

    fn find<'a>(
        &'a self,
        access: BlobAccess,
        id: BlobId,
    ) -> BoxFuture<'a, Result<Option<BlobRecord>, StoreError>>;

    /// Claims one unused blob for deletion and records the attempt before external I/O. Returns
    /// that immutable attempt number, or `None` when it is missing, committed, or a previous failed
    /// attempt is not due yet.
    fn claim_delete<'a>(
        &'a self,
        access: BlobAccess,
        id: BlobId,
        retry_after: Duration,
    ) -> BoxFuture<'a, Result<Option<BlobDeleteClaim>, StoreError>>;

    /// Claims and returns up to `limit` blobs that nothing refers to: expired uploads, expired
    /// uploaded blobs, and failed deletes whose retry time arrived. Each row moves to `Deleting`
    /// and records an attempt before external I/O, so it cannot be committed or lost on failure.
    fn claim_expired<'a>(
        &'a self,
        abandoned_after: Duration,
        unused_for: Duration,
        retry_after: Duration,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<BlobDeleteClaim>, StoreError>>;

    /// Records confirmed backend deletion, atomically with its first audit. A finalized upload's
    /// row may be removed. An upload never confirmed as `Uploaded` retains a `Deleting` tombstone
    /// and its retry schedule: a PUT with an unknown outcome may still finish after deletion. A
    /// confirmed tombstone no longer occupies an active upload slot. `attempt` must be the exact
    /// claim whose backend DELETE returned; stale acknowledgements cannot finalize a newer claim.
    /// Idempotent; committed blobs are never changed.
    fn finish_delete<'a>(&'a self, claim: BlobDeleteClaim)
    -> BoxFuture<'a, Result<(), StoreError>>;
}

/// A blob that a record about to be stored refers to (a step output). The store marks it
/// `Committed` in the same transaction as that record, and only if it is still `Uploaded` with
/// exactly this size and digest: the blob cannot be swept, replaced or used twice behind the
/// record's back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobCommit {
    pub id: BlobId,
    pub size: u64,
    pub digest: Digest,
    pub scope: BlobScope,
}

/// How a newly created run acquires its input reference. Inheritance is authorized by the
/// source run's rerun/parent link and exact input digest, never by a caller-selected blob id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunInputBinding {
    Uploaded(BlobDescriptor),
    Inherit(RunId),
}

/// Checks a bounded descriptor against the input schema in an immutable workflow document.
/// Implemented by the existing JSON Schema adapter; errors must contain no payload values.
pub trait RunInputChecker: Send + Sync {
    fn check(&self, workflow: &[u8], input: &[u8]) -> Result<(), &'static str>;
}

impl BlobCommit {
    #[must_use]
    pub fn new(scope: BlobScope, descriptor: &BlobDescriptor) -> Self {
        Self {
            id: descriptor.id(),
            size: descriptor.size(),
            digest: *descriptor.digest(),
            scope,
        }
    }
}

/// Limits the service enforces on every blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobLimits {
    /// Largest blob in bytes.
    pub max_bytes: u64,
    /// Most unused blobs one tenant may hold.
    pub max_pending: u32,
    /// How long an uploaded blob that nothing uses is kept.
    pub upload_ttl: Duration,
    /// How long a blob whose upload never finished is kept.
    pub abandoned_after: Duration,
}

impl Default for BlobLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BLOB_BYTES,
            max_pending: DEFAULT_MAX_PENDING_BLOBS,
            upload_ttl: DEFAULT_UPLOAD_TTL,
            abandoned_after: DEFAULT_ABANDONED_AFTER,
        }
    }
}

/// What a caller says about a blob it is about to upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    pub content_type: ContentType,
    /// If stated, the upload is refused unless the body has exactly this many bytes.
    pub expected_size: Option<u64>,
    /// If stated, the upload is refused unless the body has exactly this SHA-256.
    pub expected_digest: Option<Digest>,
}

/// Why a blob was not stored. Cleanup retains durable evidence of any uncertain external effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobWriteError {
    TooLarge,
    SizeMismatch,
    DigestMismatch,
    /// The tenant already holds as many unused blobs as it may.
    QuotaExceeded,
    /// The producer of the body failed or went away.
    Interrupted,
    /// The store or the catalog could not be used; the caller may retry.
    Unavailable,
    Internal,
}

/// Why a blob could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobReadError {
    /// No such blob for this tenant, or its bytes were never confirmed. The two look the same.
    NotFound,
    Unavailable,
    /// The catalog says the blob exists and the store does not have it.
    Lost,
}

/// Blob use cases over a catalog and a backend.
#[derive(Clone)]
pub struct BlobService {
    catalog: Arc<dyn BlobCatalog>,
    backend: Arc<dyn BlobBackend>,
    limits: BlobLimits,
}

impl fmt::Debug for BlobService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlobService")
            .field("provider", &self.backend.provider())
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl BlobService {
    #[must_use]
    pub fn new(
        catalog: Arc<dyn BlobCatalog>,
        backend: Arc<dyn BlobBackend>,
        limits: BlobLimits,
    ) -> Self {
        Self {
            catalog,
            backend,
            limits,
        }
    }

    #[must_use]
    pub const fn limits(&self) -> &BlobLimits {
        &self.limits
    }

    /// The catalog, for the use cases that decide inside their own transaction whether a blob is
    /// used (a step output, a run input).
    #[must_use]
    pub fn catalog(&self) -> &Arc<dyn BlobCatalog> {
        &self.catalog
    }

    /// Stores `body` as a new blob of `tenant`, measuring and hashing it as it streams.
    ///
    /// # Errors
    /// [`BlobWriteError`]. Cleanup is idempotent; if the provider cannot delete bytes immediately,
    /// the catalog retains a `Deleting` row and the sweep retries it.
    pub async fn write(
        &self,
        scope: BlobScope,
        created_by: &str,
        request: WriteRequest,
        body: ByteStream,
    ) -> Result<BlobDescriptor, BlobWriteError> {
        self.write_owned(BlobOwner::StepOutput(scope), created_by, request, body)
            .await
    }

    /// Stages an input for a future run. The caller must already have authenticated and authorized
    /// `run:start` in `access`; the catalog retains `created_by` as the only uploader that may bind it.
    pub async fn write_input(
        &self,
        access: BlobAccess,
        created_by: &str,
        request: WriteRequest,
        body: ByteStream,
    ) -> Result<BlobDescriptor, BlobWriteError> {
        self.write_owned(BlobOwner::RunInput(access), created_by, request, body)
            .await
    }

    async fn write_owned(
        &self,
        owner: BlobOwner,
        created_by: &str,
        request: WriteRequest,
        body: ByteStream,
    ) -> Result<BlobDescriptor, BlobWriteError> {
        let outcome = self.write_inner(owner, created_by, request, body).await;
        metrics::counter!(
            "blob_writes_total",
            "provider" => self.backend.provider(),
            "outcome" => match &outcome {
                Ok(_) => "ok",
                Err(BlobWriteError::TooLarge) => "too_large",
                Err(BlobWriteError::SizeMismatch | BlobWriteError::DigestMismatch) => "mismatch",
                Err(BlobWriteError::QuotaExceeded) => "quota",
                Err(BlobWriteError::Interrupted) => "interrupted",
                Err(BlobWriteError::Unavailable | BlobWriteError::Internal) => "failed",
            }
        )
        .increment(1);
        if let Ok(descriptor) = &outcome {
            metrics::counter!("blob_bytes_total", "direction" => "write")
                .increment(descriptor.size());
        }
        outcome
    }

    async fn write_inner(
        &self,
        owner: BlobOwner,
        created_by: &str,
        request: WriteRequest,
        body: ByteStream,
    ) -> Result<BlobDescriptor, BlobWriteError> {
        // A declared size over the limit is refused before anything is reserved or read.
        if request
            .expected_size
            .is_some_and(|size| size > self.limits.max_bytes)
        {
            return Err(BlobWriteError::TooLarge);
        }
        let id = BlobId::generate();
        let access = owner.access();
        // A declared size is the exact maximum this upload may consume. Unknown-length streams
        // reserve the configured hard bound. The catalog commits this capacity before backend I/O.
        let reservation_bytes = request.expected_size.unwrap_or(self.limits.max_bytes);
        self.catalog
            .reserve(
                owner,
                id,
                &request.content_type,
                created_by,
                reservation_bytes,
                self.limits.max_pending,
            )
            .await
            .map_err(|error| match error {
                StoreError::QuotaExceeded { .. } => BlobWriteError::QuotaExceeded,
                StoreError::Unavailable { .. } => BlobWriteError::Unavailable,
                other => {
                    tracing::error!(%other, "a blob could not be reserved");
                    BlobWriteError::Internal
                }
            })?;
        let key = BlobKey::of(access, id);

        let (measured, probe) = Measured::new(body, reservation_bytes);
        let stored = self.backend.put(&key, Box::pin(measured)).await;
        if let Err(failure) = stored {
            self.forget(access, id, &key).await;
            return Err(match failure {
                BackendFailure::Body(BlobIoError::TooLarge) => {
                    if request.expected_size.is_some() {
                        BlobWriteError::SizeMismatch
                    } else {
                        BlobWriteError::TooLarge
                    }
                }
                BackendFailure::Body(_) => BlobWriteError::Interrupted,
                BackendFailure::NotFound | BackendFailure::Unavailable => {
                    BlobWriteError::Unavailable
                }
                BackendFailure::Refused => {
                    tracing::error!(
                        provider = self.backend.provider(),
                        "the object store refused an upload; check its credentials and permissions"
                    );
                    BlobWriteError::Unavailable
                }
            });
        }

        let (size, digest) = probe.totals();
        let mismatch = if request.expected_size.is_some_and(|n| n != size) {
            Some(BlobWriteError::SizeMismatch)
        } else if request.expected_digest.is_some_and(|d| d != digest) {
            Some(BlobWriteError::DigestMismatch)
        } else {
            None
        };
        // The provider acknowledged the PUT, so persist its exact measurement before any cleanup.
        // This distinguishes it from an interrupted or ambiguous PUT: once measured, a confirmed
        // delete can release the capacity instead of retaining an unknown-upload reservation.
        let finalized = self.catalog.mark_uploaded(access, id, size, &digest).await;
        if let Some(error) = mismatch {
            self.forget(access, id, &key).await;
            return Err(error);
        }
        if let Err(error) = finalized {
            self.forget(access, id, &key).await;
            return Err(match error {
                StoreError::Unavailable { .. } => BlobWriteError::Unavailable,
                _ => BlobWriteError::Internal,
            });
        }
        Ok(BlobDescriptor::new(id, size, digest, request.content_type))
    }

    /// Opens an uploaded blob in `access`. The stream checks size and digest as it is read and ends
    /// with [`BlobIoError::Corrupt`] if the bytes are not the ones that were stored.
    ///
    /// # Errors
    /// [`BlobReadError`].
    pub async fn read(
        &self,
        access: BlobAccess,
        id: BlobId,
    ) -> Result<(BlobDescriptor, ByteStream), BlobReadError> {
        let record = self
            .catalog
            .find(access, id)
            .await
            .map_err(|_| BlobReadError::Unavailable)?
            .filter(|record| {
                record.state == BlobState::Committed
                    || (record.state == BlobState::Uploaded
                        && matches!(record.owner, BlobOwner::StepOutput(_)))
            })
            .ok_or(BlobReadError::NotFound)?;
        let key = BlobKey::of(access, id);
        let stream = self
            .backend
            .get(&key)
            .await
            .map_err(|failure| match failure {
                BackendFailure::NotFound => {
                    tracing::error!(blob = %id, "the catalog has a blob that the store does not");
                    metrics::counter!("blob_lost_total").increment(1);
                    BlobReadError::Lost
                }
                _ => BlobReadError::Unavailable,
            })?;
        let descriptor = record.descriptor;
        let verified = Verified::new(stream, descriptor.size(), *descriptor.digest());
        Ok((descriptor, Box::pin(verified)))
    }

    /// What is recorded about an uploaded blob in `access`, whether or not something already uses
    /// it (a retried report must still find the blob it used).
    ///
    /// # Errors
    /// [`BlobReadError::NotFound`] for another tenant or namespace, a missing blob, or one whose
    /// bytes were never confirmed: they look the same.
    pub async fn describe(
        &self,
        access: BlobAccess,
        id: BlobId,
    ) -> Result<BlobDescriptor, BlobReadError> {
        self.catalog
            .find(access, id)
            .await
            .map_err(|_| BlobReadError::Unavailable)?
            .filter(|record| matches!(record.state, BlobState::Uploaded | BlobState::Committed))
            .map(|record| record.descriptor)
            .ok_or(BlobReadError::NotFound)
    }

    /// Describes a blob only when it was uploaded by this exact attempt. Completion uses this
    /// before adding a [`BlobCommit`] to the step transaction, so a worker cannot attach a blob
    /// uploaded for another run, step, attempt or generation.
    pub async fn describe_for_commit(
        &self,
        scope: BlobScope,
        id: BlobId,
    ) -> Result<BlobDescriptor, BlobReadError> {
        self.catalog
            .find(scope.access, id)
            .await
            .map_err(|_| BlobReadError::Unavailable)?
            .filter(|record| {
                matches!(record.state, BlobState::Uploaded | BlobState::Committed)
                    && record.owner == BlobOwner::StepOutput(scope)
            })
            .map(|record| record.descriptor)
            .ok_or(BlobReadError::NotFound)
    }

    /// Finds an unused run-input upload for its authenticated creator. The start transaction
    /// repeats this exact owner/state decision before binding it, excluding concurrent GC/use.
    pub async fn describe_input(
        &self,
        access: BlobAccess,
        id: BlobId,
        created_by: &str,
    ) -> Result<BlobDescriptor, BlobReadError> {
        self.catalog
            .find(access, id)
            .await
            .map_err(|_| BlobReadError::Unavailable)?
            .filter(|record| {
                record.state == BlobState::Uploaded
                    && record.owner == BlobOwner::RunInput(access)
                    && record.created_by == created_by
            })
            .map(|record| record.descriptor)
            .ok_or(BlobReadError::NotFound)
    }

    /// Removes up to `limit` blobs that nothing refers to and that outlived their time, with their
    /// bytes. Returns how many backend deletes were confirmed in the catalog. A never-confirmed
    /// upload keeps a tombstone and remains eligible for later deletion reconciliation.
    ///
    /// # Errors
    /// [`StoreError`] if the catalog cannot be used. A failure to delete bytes is counted and
    /// logged; its `Deleting` row remains with the attempt and next retry time.
    pub async fn sweep(&self, limit: u32) -> Result<usize, StoreError> {
        let due = self
            .catalog
            .claim_expired(
                self.limits.abandoned_after,
                self.limits.upload_ttl,
                DEFAULT_DELETE_RETRY,
                limit,
            )
            .await?;
        let mut removed = 0usize;
        for claim in &due {
            match self
                .backend
                .delete(&BlobKey::of(claim.access, claim.id))
                .await
            {
                Ok(()) => {
                    self.catalog.finish_delete(*claim).await?;
                    removed += 1;
                }
                Err(_) => {
                    tracing::warn!(blob = %claim.id, "the bytes of a swept blob could not be deleted; the catalog retained the retry");
                    metrics::counter!("blob_delete_failures_total").increment(1);
                }
            }
        }
        metrics::counter!("blob_swept_total").increment(removed as u64);
        Ok(removed)
    }

    /// Undoes a failed upload. The deletion claim commits before external I/O; a failure therefore
    /// remains visible and retryable instead of becoming an untracked object.
    async fn forget(&self, access: BlobAccess, id: BlobId, key: &BlobKey) {
        match self
            .catalog
            .claim_delete(access, id, DEFAULT_DELETE_RETRY)
            .await
        {
            Ok(Some(claim)) => match self.backend.delete(key).await {
                Ok(()) => {
                    if self.catalog.finish_delete(claim).await.is_err() {
                        tracing::warn!(blob = %id, "deleted blob bytes still have a retryable catalog row");
                    }
                }
                Err(_) => {
                    tracing::warn!(blob = %id, "failed-upload bytes could not be deleted; the catalog retained the retry");
                    metrics::counter!("blob_delete_failures_total").increment(1);
                }
            },
            Ok(None) => {}
            Err(_) => {
                tracing::warn!(blob = %id, "failed-upload cleanup could not be claimed; the catalog retained the blob");
            }
        }
    }
}

/// The JSON a workflow carries in place of a blob: `{"$blob": {...}}`. The key starts with `$` so it
/// can never be mistaken for a field an author named, and every value is made here from validated
/// parts (a UUID, a number, hex, a media type without quotes), never from free text.
#[must_use]
pub fn descriptor_json(descriptor: &BlobDescriptor) -> Vec<u8> {
    format!(
        "{{\"$blob\":{{\"id\":\"{}\",\"size\":{},\"sha256\":\"{}\",\"contentType\":\"{}\"}}}}",
        descriptor.id(),
        descriptor.size(),
        descriptor.digest().to_hex(),
        descriptor.content_type().as_str(),
    )
    .into_bytes()
}

/// A body made of chunks that are already in memory (tests, small payloads).
#[must_use]
pub fn byte_stream(chunks: Vec<Result<Bytes, BlobIoError>>) -> ByteStream {
    Box::pin(Chunks(chunks.into_iter()))
}

struct Chunks(std::vec::IntoIter<Result<Bytes, BlobIoError>>);

impl Stream for Chunks {
    type Item = Result<Bytes, BlobIoError>;

    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.0.next())
    }
}

// ---------------------------------------------------------------------------- measuring streams

#[derive(Default)]
struct Totals {
    size: u64,
    hasher: Sha256,
}

/// Reads what [`Measured`] counted once the backend has consumed the whole body.
struct Probe(Arc<Mutex<Totals>>);

impl Probe {
    fn totals(&self) -> (u64, Digest) {
        // A poisoned lock means a panic while counting; the totals are then untrustworthy and the
        // all-zero digest fails any comparison.
        self.0.lock().map_or((0, Digest::new([0; 32])), |totals| {
            (
                totals.size,
                Digest::new(totals.hasher.clone().finalize().into()),
            )
        })
    }
}

/// Passes a body through while counting and hashing it, and stops it at `max` bytes.
struct Measured {
    inner: ByteStream,
    totals: Arc<Mutex<Totals>>,
    max: u64,
}

impl Measured {
    fn new(inner: ByteStream, max: u64) -> (Self, Probe) {
        let totals = Arc::new(Mutex::new(Totals::default()));
        (
            Self {
                inner,
                totals: Arc::clone(&totals),
                max,
            },
            Probe(totals),
        )
    }
}

impl Stream for Measured {
    type Item = Result<Bytes, BlobIoError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                let Ok(mut totals) = self.totals.lock() else {
                    return Poll::Ready(Some(Err(BlobIoError::Interrupted)));
                };
                totals.size += chunk.len() as u64;
                if totals.size > self.max {
                    return Poll::Ready(Some(Err(BlobIoError::TooLarge)));
                }
                totals.hasher.update(&chunk);
                drop(totals);
                Poll::Ready(Some(Ok(chunk)))
            }
            other => other,
        }
    }
}

/// Passes a stored object through while checking it against what was recorded for it. A mismatch
/// ends the stream with [`BlobIoError::Corrupt`] instead of a clean end.
struct Verified {
    inner: ByteStream,
    expected_size: u64,
    expected_digest: Digest,
    seen: u64,
    hasher: Option<Sha256>,
    done: bool,
}

impl Verified {
    fn new(inner: ByteStream, expected_size: u64, expected_digest: Digest) -> Self {
        Self {
            inner,
            expected_size,
            expected_digest,
            seen: 0,
            hasher: Some(Sha256::new()),
            done: false,
        }
    }
}

impl Stream for Verified {
    type Item = Result<Bytes, BlobIoError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        match self.inner.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(chunk))) => {
                self.seen += chunk.len() as u64;
                if self.seen > self.expected_size {
                    self.done = true;
                    return Poll::Ready(Some(Err(BlobIoError::Corrupt)));
                }
                if let Some(hasher) = &mut self.hasher {
                    hasher.update(&chunk);
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(error))) => {
                self.done = true;
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                self.done = true;
                let digest = self
                    .hasher
                    .take()
                    .map(|hasher| Digest::new(hasher.finalize().into()));
                if self.seen == self.expected_size && digest == Some(self.expected_digest) {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Err(BlobIoError::Corrupt)))
                }
            }
        }
    }
}

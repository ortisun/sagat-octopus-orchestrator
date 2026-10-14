//! The blob use cases against an in-memory backend and catalog (FR-002-008, ADR-0023).
//!
//! The fakes mirror the *contracts* of `BlobBackend` and `BlobCatalog` (tenant/namespace keys, a
//! reservation before the bytes, idempotent deletes, failure injection). What the real adapters
//! guarantee is proven separately: the object store in `crates/infra-blobstore`, the catalog against
//! PostgreSQL in `tests/integration/002-durable-execution`.
#![allow(clippy::expect_used, clippy::significant_drop_tightening)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use sagat_application::blobs::{
    BackendFailure, BlobAccess, BlobBackend, BlobCatalog, BlobDeleteClaim, BlobIoError, BlobKey,
    BlobLimits, BlobOwner, BlobReadError, BlobRecord, BlobScope, BlobService, BlobWriteError,
    BoxFuture, ByteStream, WriteRequest, byte_stream, descriptor_json,
};
use sagat_application::error::StoreError;
use sagat_domain::blobs::{BlobDescriptor, BlobId, BlobState, ContentType};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::Digest;
use sha2::{Digest as _, Sha256};

// ------------------------------------------------------------------------------------- fakes

#[derive(Default)]
struct Objects {
    stored: BTreeMap<String, Vec<u8>>,
    /// Make `put` fail after reading this many bytes.
    fail_put_after: Option<usize>,
    fail_put_outright: bool,
    fail_delete: bool,
    deletes: Vec<String>,
    put_gate: Option<Arc<PutGate>>,
}

#[derive(Default)]
struct PutGate {
    released: AtomicBool,
    waker: futures_util::task::AtomicWaker,
}

impl PutGate {
    async fn wait(&self) {
        std::future::poll_fn(|cx| {
            self.waker.register(cx.waker());
            if self.released.load(Ordering::Acquire) {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        })
        .await;
    }

    fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.waker.wake();
    }
}

#[derive(Clone, Default, Debug)]
struct FakeBackend(Arc<Mutex<Objects>>);

impl std::fmt::Debug for Objects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Objects({})", self.stored.len())
    }
}

impl FakeBackend {
    fn with<R>(&self, f: impl FnOnce(&mut Objects) -> R) -> R {
        f(&mut self.0.lock().expect("lock"))
    }
}

impl BlobBackend for FakeBackend {
    fn provider(&self) -> &'static str {
        "fake"
    }

    fn put<'a>(
        &'a self,
        key: &'a BlobKey,
        mut body: ByteStream,
    ) -> BoxFuture<'a, Result<(), BackendFailure>> {
        Box::pin(async move {
            if self.with(|o| o.fail_put_outright) {
                return Err(BackendFailure::Unavailable);
            }
            let limit = self.with(|o| o.fail_put_after);
            let mut data = Vec::new();
            while let Some(chunk) = body.next().await {
                let chunk = chunk.map_err(BackendFailure::Body)?;
                data.extend_from_slice(&chunk);
                if limit.is_some_and(|n| data.len() >= n) {
                    return Err(BackendFailure::Unavailable);
                }
            }
            if let Some(gate) = self.with(|o| o.put_gate.clone()) {
                gate.wait().await;
            }
            self.with(|o| o.stored.insert(key.as_str().to_owned(), data));
            Ok(())
        })
    }

    fn get<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<ByteStream, BackendFailure>> {
        Box::pin(async move {
            let data = self
                .with(|o| o.stored.get(key.as_str()).cloned())
                .ok_or(BackendFailure::NotFound)?;
            // Two chunks, so the verifying wrapper sees a stream and not one buffer.
            let middle = data.len() / 2;
            Ok(byte_stream(vec![
                Ok(Bytes::copy_from_slice(&data[..middle])),
                Ok(Bytes::copy_from_slice(&data[middle..])),
            ]))
        })
    }

    fn delete<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<(), BackendFailure>> {
        Box::pin(async move {
            if self.with(|o| o.fail_delete) {
                return Err(BackendFailure::Unavailable);
            }
            self.with(|o| {
                o.deletes.push(key.as_str().to_owned());
                o.stored.remove(key.as_str());
            });
            Ok(())
        })
    }
}

#[derive(Clone)]
struct Row {
    record: BlobRecord,
    created_by: String,
    allowed_bytes: u64,
    /// How long ago it was reserved, set by the test (the real catalog uses the database clock).
    age: Duration,
    deletion_attempts: u32,
    retry_due: bool,
    uploaded: bool,
    deletion_confirmed: bool,
}

#[derive(Default)]
struct Rows(BTreeMap<(BlobAccess, BlobId), Row>);

#[derive(Clone, Default, Debug)]
struct FakeCatalog(Arc<Mutex<Rows>>);

impl std::fmt::Debug for Rows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rows({})", self.0.len())
    }
}

impl FakeCatalog {
    fn with<R>(&self, f: impl FnOnce(&mut Rows) -> R) -> R {
        f(&mut self.0.lock().expect("lock"))
    }

    fn pending(&self) -> usize {
        self.with(|rows| {
            rows.0
                .values()
                .filter(|row| !row.record.state.is_referenced() && !row.deletion_confirmed)
                .count()
        })
    }
}

impl BlobCatalog for FakeCatalog {
    fn reserve<'a>(
        &'a self,
        owner: BlobOwner,
        id: BlobId,
        content_type: &'a ContentType,
        created_by: &'a str,
        reservation_bytes: u64,
        max_pending: u32,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            self.with(|rows| {
                let pending = rows
                    .0
                    .iter()
                    .filter(|((access, _), r)| {
                        access.tenant_id == owner.access().tenant_id
                            && !r.record.state.is_referenced()
                            && !r.deletion_confirmed
                    })
                    .count();
                if pending >= max_pending as usize {
                    return Err(StoreError::QuotaExceeded {
                        what: "uncommitted blobs",
                    });
                }
                rows.0.insert(
                    (owner.access(), id),
                    Row {
                        record: BlobRecord {
                            descriptor: BlobDescriptor::new(
                                id,
                                0,
                                Digest::new([0; 32]),
                                content_type.clone(),
                            ),
                            state: BlobState::Uploading,
                            owner,
                            created_by: created_by.to_owned(),
                        },
                        created_by: created_by.to_owned(),
                        allowed_bytes: reservation_bytes,
                        age: Duration::ZERO,
                        deletion_attempts: 0,
                        retry_due: false,
                        uploaded: false,
                        deletion_confirmed: false,
                    },
                );
                Ok(())
            })
        })
    }

    fn mark_uploaded<'a>(
        &'a self,
        access: BlobAccess,
        id: BlobId,
        size: u64,
        digest: &'a Digest,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let deleting = self.with(|rows| {
                let row = rows
                    .0
                    .get_mut(&(access, id))
                    .ok_or(StoreError::MissingReference)?;
                let deleting = row.record.state == BlobState::Deleting;
                if !deleting {
                    row.record.state = row
                        .record
                        .state
                        .uploaded()
                        .map_err(|_| StoreError::MissingReference)?;
                }
                row.record.descriptor = BlobDescriptor::new(
                    id,
                    size,
                    *digest,
                    row.record.descriptor.content_type().clone(),
                );
                row.uploaded = true;
                if deleting {
                    // Retire the DELETE that began before this provider-acknowledged PUT. The next
                    // cleanup receives a later token; an old response can no longer finalize it.
                    row.deletion_attempts += 1;
                    row.retry_due = true;
                }
                Ok(deleting)
            })?;
            if deleting {
                Err(StoreError::MissingReference)
            } else {
                Ok(())
            }
        })
    }

    fn find<'a>(
        &'a self,
        access: BlobAccess,
        id: BlobId,
    ) -> BoxFuture<'a, Result<Option<BlobRecord>, StoreError>> {
        Box::pin(async move {
            Ok(self.with(|rows| rows.0.get(&(access, id)).map(|r| r.record.clone())))
        })
    }

    fn claim_delete<'a>(
        &'a self,
        access: BlobAccess,
        id: BlobId,
        _retry_after: Duration,
    ) -> BoxFuture<'a, Result<Option<BlobDeleteClaim>, StoreError>> {
        Box::pin(async move {
            Ok(self.with(|rows| {
                let row = rows.0.get_mut(&(access, id))?;
                if matches!(row.record.state, BlobState::Uploading | BlobState::Uploaded)
                    || (row.record.state == BlobState::Deleting && row.retry_due)
                {
                    row.record.state = BlobState::Deleting;
                    row.deletion_attempts += 1;
                    row.retry_due = false;
                    Some(BlobDeleteClaim {
                        access,
                        id,
                        attempt: row.deletion_attempts,
                    })
                } else {
                    None
                }
            }))
        })
    }

    fn claim_expired<'a>(
        &'a self,
        abandoned_after: Duration,
        unused_for: Duration,
        _retry_after: Duration,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<BlobDeleteClaim>, StoreError>> {
        Box::pin(async move {
            Ok(self.with(|rows| {
                let due: Vec<_> = rows
                    .0
                    .iter()
                    .filter(|(_, r)| match r.record.state {
                        BlobState::Uploading => r.age >= abandoned_after,
                        BlobState::Uploaded => r.age >= unused_for,
                        BlobState::Deleting => r.retry_due,
                        BlobState::Committed => false,
                    })
                    .map(|(k, _)| *k)
                    .take(limit as usize)
                    .collect();
                for key in &due {
                    let row = rows.0.get_mut(key).expect("selected row");
                    row.record.state = BlobState::Deleting;
                    row.deletion_attempts += 1;
                    row.retry_due = false;
                }
                due.into_iter()
                    .map(|(access, id)| {
                        let attempt = rows
                            .0
                            .get(&(access, id))
                            .expect("claimed row")
                            .deletion_attempts;
                        BlobDeleteClaim {
                            access,
                            id,
                            attempt,
                        }
                    })
                    .collect()
            }))
        })
    }

    fn finish_delete<'a>(
        &'a self,
        claim: BlobDeleteClaim,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            self.with(|rows| {
                let Some(row) = rows.0.get_mut(&(claim.access, claim.id)) else {
                    return;
                };
                if row.record.state != BlobState::Deleting {
                    return;
                }
                if row.deletion_attempts != claim.attempt {
                    return;
                }
                if row.uploaded {
                    rows.0.remove(&(claim.access, claim.id));
                } else {
                    row.deletion_confirmed = true;
                }
            });
            Ok(())
        })
    }
}

// ----------------------------------------------------------------------------------- helpers

struct World {
    service: BlobService,
    backend: FakeBackend,
    catalog: FakeCatalog,
    tenant: TenantId,
    namespace: NamespaceId,
    run: RunId,
    step: uuid::Uuid,
}

fn world_with(limits: BlobLimits) -> World {
    let backend = FakeBackend::default();
    let catalog = FakeCatalog::default();
    World {
        service: BlobService::new(Arc::new(catalog.clone()), Arc::new(backend.clone()), limits),
        backend,
        catalog,
        tenant: TenantId::generate(),
        namespace: NamespaceId::generate(),
        run: RunId::generate(),
        step: uuid::Uuid::new_v4(),
    }
}

impl World {
    fn access(&self) -> BlobAccess {
        BlobAccess {
            tenant_id: self.tenant,
            namespace_id: self.namespace,
        }
    }

    fn scope(&self) -> BlobScope {
        BlobScope {
            access: self.access(),
            run_id: self.run,
            step_id: self.step,
            attempt: 1,
            generation: 1,
        }
    }
}

fn limits() -> BlobLimits {
    BlobLimits {
        max_bytes: 1024,
        max_pending: 4,
        upload_ttl: Duration::from_secs(24 * 3600),
        abandoned_after: Duration::from_secs(3600),
    }
}

fn world() -> World {
    world_with(limits())
}

fn request() -> WriteRequest {
    WriteRequest {
        content_type: ContentType::parse("application/json").expect("valid"),
        expected_size: None,
        expected_digest: None,
    }
}

fn body(chunks: &[&[u8]]) -> ByteStream {
    byte_stream(
        chunks
            .iter()
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect(),
    )
}

fn sha(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

async fn collect(mut stream: ByteStream) -> Result<Vec<u8>, BlobIoError> {
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk?);
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------ writing

#[tokio::test]
async fn a_written_blob_is_measured_stored_under_its_tenant_and_described() {
    let w = world();

    let descriptor = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"hello ", b"world"]))
        .await
        .expect("written");

    assert_eq!(descriptor.size(), 11);
    assert_eq!(descriptor.digest(), &sha(b"hello world"));
    assert_eq!(descriptor.content_type().as_str(), "application/json");
    let key = BlobKey::of(w.access(), descriptor.id());
    assert_eq!(
        w.backend.with(|o| o.stored.get(key.as_str()).cloned()),
        Some(b"hello world".to_vec())
    );
    let row = w
        .catalog
        .with(|r| r.0.get(&(w.access(), descriptor.id())).cloned());
    let row = row.expect("catalogued");
    assert_eq!(
        row.record.state,
        BlobState::Uploaded,
        "not referenced by anything yet"
    );
    assert_eq!(row.created_by, "wendy");
    assert_eq!(
        row.allowed_bytes, 1024,
        "an unknown-length upload reserves the configured maximum"
    );
}

#[tokio::test]
async fn the_key_in_the_store_is_scoped_by_tenant_and_namespace_and_made_only_of_ids() {
    let id = BlobId::generate();
    let a = BlobAccess {
        tenant_id: TenantId::generate(),
        namespace_id: NamespaceId::generate(),
    };
    let b = BlobAccess {
        tenant_id: TenantId::generate(),
        namespace_id: NamespaceId::generate(),
    };
    let other_namespace = BlobAccess {
        tenant_id: a.tenant_id,
        namespace_id: NamespaceId::generate(),
    };

    let key_a = BlobKey::of(a, id);
    let key_b = BlobKey::of(b, id);
    let key_other_namespace = BlobKey::of(other_namespace, id);

    assert_ne!(key_a, key_b, "the same id under two tenants is two objects");
    assert_ne!(
        key_a, key_other_namespace,
        "the same id under two namespaces is two objects"
    );
    assert_eq!(
        key_a.as_str(),
        format!(
            "tenants/{}/namespaces/{}/blobs/{id}",
            a.tenant_id, a.namespace_id
        )
    );
    let parts: Vec<_> = key_a.as_str().split('/').collect();
    assert_eq!(
        parts,
        [
            "tenants",
            &a.tenant_id.to_string(),
            "namespaces",
            &a.namespace_id.to_string(),
            "blobs",
            &id.to_string()
        ]
    );
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_and_leaves_nothing_behind() {
    let w = world();
    let big = vec![7u8; 1025];

    let result = w
        .service
        .write(
            w.scope(),
            "wendy",
            request(),
            body(&[&big[..600], &big[600..]]),
        )
        .await;

    assert_eq!(result, Err(BlobWriteError::TooLarge));
    assert!(w.backend.with(|o| o.stored.is_empty()), "no object");
    assert_eq!(w.catalog.pending(), 0, "no active reservation");
}

#[tokio::test]
async fn a_declared_size_is_the_stream_bound_and_an_oversized_declaration_reserves_nothing() {
    let w = world();
    let mut declared = request();
    declared.expected_size = Some(5);
    assert_eq!(
        w.service
            .write(w.scope(), "wendy", declared, body(&[b"sixsix"]))
            .await,
        Err(BlobWriteError::SizeMismatch),
        "the backend stream stops at the committed reservation"
    );
    let mut oversized = request();
    oversized.expected_size = Some(1025);
    assert_eq!(
        w.service
            .write(w.scope(), "wendy", oversized, body(&[]))
            .await,
        Err(BlobWriteError::TooLarge)
    );
    assert_eq!(w.catalog.pending(), 0);
}

#[tokio::test]
async fn a_body_exactly_at_the_limit_is_accepted() {
    let w = world();
    let exact = vec![1u8; 1024];

    let descriptor = w
        .service
        .write(w.scope(), "wendy", request(), body(&[&exact]))
        .await
        .expect("written");

    assert_eq!(descriptor.size(), 1024);
}

#[tokio::test]
async fn a_stated_size_or_digest_that_does_not_match_the_bytes_is_refused_and_cleaned_up() {
    let w = world();

    let mut wrong_size = request();
    wrong_size.expected_size = Some(12);
    let result = w
        .service
        .write(w.scope(), "wendy", wrong_size, body(&[b"hello world"]))
        .await;
    assert_eq!(result, Err(BlobWriteError::SizeMismatch));

    let mut wrong_digest = request();
    wrong_digest.expected_digest = Some(sha(b"something else"));
    let result = w
        .service
        .write(w.scope(), "wendy", wrong_digest, body(&[b"hello world"]))
        .await;
    assert_eq!(result, Err(BlobWriteError::DigestMismatch));

    assert!(
        w.backend.with(|o| o.stored.is_empty()),
        "the bytes were removed"
    );
    assert_eq!(
        w.catalog.pending(),
        0,
        "the active reservation was released"
    );
    assert!(
        w.catalog.with(|rows| rows.0.is_empty()),
        "provider-acknowledged mismatches are measured before confirmed cleanup"
    );
    assert!(!w.backend.with(|o| o.deletes.is_empty()));

    let mut right = request();
    right.expected_size = Some(11);
    right.expected_digest = Some(sha(b"hello world"));
    let written = w
        .service
        .write(w.scope(), "wendy", right, body(&[b"hello world"]))
        .await
        .expect("matching declared upload");
    assert_eq!(
        w.catalog.with(|rows| rows
            .0
            .get(&(w.access(), written.id()))
            .expect("row")
            .allowed_bytes),
        11
    );
}

#[tokio::test]
async fn a_producer_that_fails_half_way_leaves_nothing_behind() {
    let w = world();
    let broken = byte_stream(vec![
        Ok(Bytes::from_static(b"partial")),
        Err(BlobIoError::Interrupted),
    ]);

    let result = w.service.write(w.scope(), "wendy", request(), broken).await;

    assert_eq!(result, Err(BlobWriteError::Interrupted));
    assert!(w.backend.with(|o| o.stored.is_empty()));
    assert_eq!(w.catalog.pending(), 0);
}

#[tokio::test]
async fn a_store_that_is_down_is_unavailable_and_leaves_no_reservation() {
    let w = world();
    w.backend.with(|o| o.fail_put_outright = true);

    let result = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"hello"]))
        .await;

    assert_eq!(result, Err(BlobWriteError::Unavailable));
    assert_eq!(
        w.catalog.pending(),
        0,
        "a retry is not blocked by a tombstone"
    );
}

#[tokio::test]
async fn too_many_uncommitted_blobs_refuse_the_next_one_before_any_byte_is_stored() {
    let w = world();
    for n in 0..4 {
        w.service
            .write(
                w.scope(),
                "wendy",
                request(),
                body(&[format!("blob {n}").as_bytes()]),
            )
            .await
            .expect("within the limit");
    }

    let result = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"one too many"]))
        .await;

    assert_eq!(result, Err(BlobWriteError::QuotaExceeded));
    assert_eq!(w.backend.with(|o| o.stored.len()), 4);

    // Another tenant has a budget of its own.
    assert!(
        w.service
            .write(
                BlobScope {
                    access: BlobAccess {
                        tenant_id: TenantId::generate(),
                        namespace_id: NamespaceId::generate(),
                    },
                    run_id: RunId::generate(),
                    step_id: uuid::Uuid::new_v4(),
                    attempt: 1,
                    generation: 1,
                },
                "vera",
                request(),
                body(&[b"mine"]),
            )
            .await
            .is_ok()
    );
}

// ------------------------------------------------------------------------------------ reading

#[tokio::test]
async fn a_blob_is_read_back_whole_and_verified() {
    let w = world();
    let written = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"hello ", b"world"]))
        .await
        .expect("written");

    let (descriptor, stream) = w
        .service
        .read(w.access(), written.id())
        .await
        .expect("found");

    assert_eq!(descriptor, written);
    assert_eq!(collect(stream).await.expect("intact"), b"hello world");
}

#[tokio::test]
async fn another_tenants_blob_is_exactly_as_missing_as_one_that_does_not_exist() {
    let w = world();
    let written = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"secret"]))
        .await
        .expect("written");
    let other = BlobAccess {
        tenant_id: TenantId::generate(),
        namespace_id: NamespaceId::generate(),
    };

    let foreign = w.service.read(other, written.id()).await.err();
    let missing = w.service.read(w.access(), BlobId::generate()).await.err();

    assert_eq!(foreign, Some(BlobReadError::NotFound));
    assert_eq!(foreign, missing);
}

#[tokio::test]
async fn completion_can_only_describe_the_exact_uploading_attempt_and_its_replay() {
    let w = world();
    let written = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"result"]))
        .await
        .expect("written");

    assert_eq!(
        w.service.describe_for_commit(w.scope(), written.id()).await,
        Ok(written.clone())
    );
    for foreign in [
        BlobScope {
            attempt: 2,
            ..w.scope()
        },
        BlobScope {
            generation: 2,
            ..w.scope()
        },
        BlobScope {
            step_id: uuid::Uuid::new_v4(),
            ..w.scope()
        },
        BlobScope {
            run_id: RunId::generate(),
            ..w.scope()
        },
    ] {
        assert_eq!(
            w.service.describe_for_commit(foreign, written.id()).await,
            Err(BlobReadError::NotFound)
        );
    }

    w.catalog.with(|rows| {
        rows.0
            .get_mut(&(w.access(), written.id()))
            .expect("row")
            .record
            .state = BlobState::Committed;
    });
    assert_eq!(
        w.service.describe_for_commit(w.scope(), written.id()).await,
        Ok(written),
        "an exactly repeated successful report still resolves its descriptor"
    );
}

#[tokio::test]
async fn a_blob_whose_upload_never_finished_cannot_be_read() {
    let w = world();
    let id = BlobId::generate();
    let content_type = ContentType::octet_stream();
    w.catalog
        .reserve(
            BlobOwner::StepOutput(w.scope()),
            id,
            &content_type,
            "wendy",
            1024,
            4,
        )
        .await
        .expect("reserved");

    let result = w.service.read(w.access(), id).await.err();

    assert_eq!(result, Some(BlobReadError::NotFound));
}

#[tokio::test]
async fn bytes_that_changed_in_the_store_are_reported_not_served_as_good() {
    let w = world();
    let written = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"hello world"]))
        .await
        .expect("written");
    let key = BlobKey::of(w.access(), written.id());

    for (what, replaced) in [
        ("flipped", b"hello_world".to_vec()),
        ("truncated", b"hello".to_vec()),
        ("extended", b"hello world!".to_vec()),
    ] {
        w.backend
            .with(|o| o.stored.insert(key.as_str().to_owned(), replaced));
        let (_, stream) = w
            .service
            .read(w.access(), written.id())
            .await
            .expect("found");
        assert_eq!(
            collect(stream).await,
            Err(BlobIoError::Corrupt),
            "{what} content must end the stream with an error"
        );
    }
}

// ----------------------------------------------------------------------------------- sweeping

#[tokio::test]
async fn a_put_finishing_after_cleanup_remains_tracked_and_is_deleted_again() {
    let w = world_with(BlobLimits {
        max_pending: 1,
        ..limits()
    });
    let gate = Arc::new(PutGate::default());
    w.backend
        .with(|objects| objects.put_gate = Some(Arc::clone(&gate)));
    let upload = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"late bytes"]));
    futures_util::pin_mut!(upload);
    assert!(
        upload
            .as_mut()
            .poll(&mut std::task::Context::from_waker(
                futures_util::task::noop_waker_ref(),
            ))
            .is_pending()
    );
    let id = w.catalog.with(|rows| {
        let (_, row) = rows.0.first_key_value().expect("reservation");
        let id = row.record.descriptor.id();
        rows.0.get_mut(&(w.access(), id)).expect("row").age = Duration::from_secs(7200);
        id
    });
    assert_eq!(w.service.sweep(10).await.expect("initial cleanup"), 1);
    assert!(w.backend.with(|objects| objects.stored.is_empty()));

    gate.release();
    assert_eq!(upload.await, Err(BlobWriteError::Internal));
    assert!(w.backend.with(|objects| objects.stored.is_empty()));
    assert!(
        w.catalog
            .with(|rows| !rows.0.contains_key(&(w.access(), id))),
        "the acknowledged late PUT is measured and deleted by a new attempt"
    );
    assert!(matches!(
        w.service.read(w.access(), id).await,
        Err(BlobReadError::NotFound)
    ));
    assert!(
        w.service
            .write(w.scope(), "wendy", request(), body(&[b"next upload"]))
            .await
            .is_ok(),
        "confirmed tombstones do not consume an active upload slot"
    );
}

#[tokio::test]
async fn the_sweep_removes_uncommitted_blobs_that_are_old_enough_and_their_bytes() {
    let w = world();
    let old = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"old"]))
        .await
        .expect("written");
    let fresh = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"fresh"]))
        .await
        .expect("written");
    w.catalog.with(|rows| {
        rows.0.get_mut(&(w.access(), old.id())).expect("row").age =
            Duration::from_secs(2 * 24 * 3600);
    });

    let removed = w.service.sweep(100).await.expect("swept");

    assert_eq!(removed, 1);
    assert!(w.service.read(w.access(), old.id()).await.is_err());
    assert!(w.service.read(w.access(), fresh.id()).await.is_ok());
    assert_eq!(
        w.backend.with(|o| o.stored.len()),
        1,
        "only the fresh bytes remain"
    );
}

#[tokio::test]
async fn the_sweep_never_touches_a_blob_something_refers_to() {
    let w = world();
    let used = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"in use"]))
        .await
        .expect("written");
    w.catalog.with(|rows| {
        let row = rows.0.get_mut(&(w.access(), used.id())).expect("row");
        row.record.state = BlobState::Committed;
        row.age = Duration::from_secs(10 * 24 * 3600);
    });

    let removed = w.service.sweep(100).await.expect("swept");

    assert_eq!(removed, 0);
    assert!(w.service.read(w.access(), used.id()).await.is_ok());
}

#[tokio::test]
async fn the_sweep_is_bounded_per_pass() {
    let w = world_with(BlobLimits {
        max_pending: 50,
        ..limits()
    });
    for n in 0..5 {
        let d = w
            .service
            .write(
                w.scope(),
                "wendy",
                request(),
                body(&[format!("{n}").as_bytes()]),
            )
            .await
            .expect("written");
        w.catalog.with(|rows| {
            rows.0.get_mut(&(w.access(), d.id())).expect("row").age =
                Duration::from_secs(2 * 24 * 3600);
        });
    }

    assert_eq!(w.service.sweep(2).await.expect("swept"), 2);
    assert_eq!(w.service.sweep(100).await.expect("swept"), 3);
    assert_eq!(w.service.sweep(100).await.expect("swept"), 0);
}

#[tokio::test]
async fn a_failed_backend_delete_retains_attempt_evidence_and_is_retried() {
    let w = world();
    let old = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"retry me"]))
        .await
        .expect("written");
    w.catalog.with(|rows| {
        rows.0.get_mut(&(w.access(), old.id())).expect("row").age =
            Duration::from_secs(2 * 24 * 3600);
    });
    w.backend.with(|objects| objects.fail_delete = true);

    assert_eq!(w.service.sweep(100).await.expect("claimed"), 0);
    let retained = w
        .catalog
        .with(|rows| rows.0.get(&(w.access(), old.id())).cloned())
        .expect("retry evidence retained");
    assert_eq!(retained.record.state, BlobState::Deleting);
    assert_eq!(retained.deletion_attempts, 1);
    assert!(
        w.backend.with(|objects| objects
            .stored
            .contains_key(BlobKey::of(w.access(), old.id()).as_str())),
        "the failed external delete is not pretended complete"
    );
    assert_eq!(
        w.service.sweep(100).await.expect("not due"),
        0,
        "the retry delay prevents a hot loop"
    );

    w.catalog.with(|rows| {
        rows.0
            .get_mut(&(w.access(), old.id()))
            .expect("row")
            .retry_due = true;
    });
    w.backend.with(|objects| objects.fail_delete = false);
    assert_eq!(w.service.sweep(100).await.expect("retried"), 1);
    assert!(
        w.catalog
            .with(|rows| !rows.0.contains_key(&(w.access(), old.id())))
    );
}

// ------------------------------------------------------------------------------- descriptor

#[tokio::test]
async fn the_descriptor_a_workflow_carries_has_one_documented_shape() {
    let id: BlobId = "019b5d6e-0000-7000-8000-000000000001".parse().expect("id");
    let descriptor = BlobDescriptor::new(
        id,
        1_048_576,
        Digest::new([0xab; 32]),
        ContentType::parse("application/json").expect("valid"),
    );

    let json: serde_json::Value =
        serde_json::from_slice(&descriptor_json(&descriptor)).expect("valid JSON");

    assert_eq!(
        json,
        serde_json::json!({
            "$blob": {
                "id": "019b5d6e-0000-7000-8000-000000000001",
                "size": 1_048_576,
                "sha256": "ab".repeat(32),
                "contentType": "application/json"
            }
        })
    );
}

#[tokio::test]
async fn an_abandoned_upload_is_swept_much_sooner_than_an_uploaded_blob_nobody_used() {
    let w = world();
    let finished = w
        .service
        .write(w.scope(), "wendy", request(), body(&[b"finished"]))
        .await
        .expect("written");
    // An upload that never finished: reserved, and nothing else (the connection was dropped).
    let abandoned = BlobId::generate();
    w.catalog
        .reserve(
            BlobOwner::StepOutput(w.scope()),
            abandoned,
            &ContentType::octet_stream(),
            "wendy",
            1024,
            10,
        )
        .await
        .expect("reserved");
    // Both are three hours old: past the time of an abandoned upload (1 h), not of an unused blob (24 h).
    w.catalog.with(|rows| {
        for id in [finished.id(), abandoned] {
            rows.0.get_mut(&(w.access(), id)).expect("row").age = Duration::from_secs(3 * 3600);
        }
    });

    let removed = w.service.sweep(100).await.expect("swept");

    assert_eq!(removed, 1);
    assert!(
        w.service.read(w.access(), finished.id()).await.is_ok(),
        "kept"
    );
    assert!(
        w.catalog.with(
            |rows| rows
                .0
                .get(&(w.access(), abandoned))
                .is_some_and(
                    |row| row.record.state == BlobState::Deleting && row.deletion_confirmed
                )
        ),
        "uncertain upload retains reconciliation evidence"
    );
}

#[tokio::test]
async fn a_run_input_is_private_until_bound_and_only_its_creator_can_bind_it() {
    let w = world();
    let descriptor = w
        .service
        .write_input(w.access(), "wendy", request(), body(&[b"large input"]))
        .await
        .expect("staged without a lease");
    assert_eq!(
        w.service
            .describe_input(w.access(), descriptor.id(), "wendy")
            .await,
        Ok(descriptor.clone())
    );
    assert_eq!(
        w.service
            .describe_input(w.access(), descriptor.id(), "other")
            .await,
        Err(BlobReadError::NotFound)
    );
    assert_eq!(
        w.service
            .describe_for_commit(w.scope(), descriptor.id())
            .await,
        Err(BlobReadError::NotFound)
    );
    assert_eq!(
        w.service.read(w.access(), descriptor.id()).await.err(),
        Some(BlobReadError::NotFound)
    );
    w.catalog.with(|rows| {
        rows.0
            .get_mut(&(w.access(), descriptor.id()))
            .expect("input row")
            .record
            .state = BlobState::Committed
    });
    assert_eq!(
        w.service
            .describe_input(w.access(), descriptor.id(), "wendy")
            .await,
        Err(BlobReadError::NotFound)
    );
    let (_, stream) = w
        .service
        .read(w.access(), descriptor.id())
        .await
        .expect("committed input");
    assert_eq!(
        collect(stream).await.expect("verified bytes"),
        b"large input"
    );
}

//! The blob catalogue in PostgreSQL (FR-002-008, ADR-0023).
//!
//! The store never holds a blob's bytes, only the fact that they exist, who owns them and in which
//! state. Every statement carries the tenant. Time is the database's own clock, like everywhere else.

use std::time::Duration;

use sagat_application::blobs::{
    BlobAccess, BlobCatalog, BlobCommit, BlobDeleteClaim, BlobOwner, BlobRecord, BlobScope,
    BoxFuture,
};
use sagat_application::error::StoreError;
use sagat_domain::audit::{AuditAction, ResourceType};
use sagat_domain::blobs::{BlobDescriptor, BlobId, BlobState, ContentType};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::workflow::Digest;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::identity::{col, store_error};
use crate::pool::BeginSafely;

/// What a tenant may not exceed, named for the quota error that reports it.
const UNUSED_BLOBS: &str = "uploaded blobs not yet used";

/// [`BlobCatalog`] over a connection pool.
#[derive(Debug, Clone)]
pub struct PgBlobCatalog {
    pool: PgPool,
}

impl PgBlobCatalog {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn corrupt(what: &str) -> StoreError {
    StoreError::Internal {
        detail: format!("a stored blob has an invalid {what}"),
    }
}

async fn append_blob_audit(
    conn: &mut PgConnection,
    access: BlobAccess,
    id: BlobId,
    actor: &str,
    action: AuditAction,
    reason: Option<&str>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO audit_records \
         (id, tenant_id, actor, action, resource_type, resource_id, reason, trace_id, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, clock_timestamp())",
    )
    .bind(Uuid::now_v7())
    .bind(access.tenant_id.as_uuid())
    .bind(actor)
    .bind(action.as_str())
    .bind(ResourceType::Blob.as_str())
    .bind(format!("{}/{}", access.namespace_id, id))
    .bind(reason)
    .execute(conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

impl BlobCatalog for PgBlobCatalog {
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
            let access = owner.access();
            let scope = match owner {
                BlobOwner::StepOutput(scope) => Some(scope),
                BlobOwner::RunInput(_) => None,
            };
            let purpose = if scope.is_some() {
                "STEP_OUTPUT"
            } else {
                "RUN_INPUT"
            };
            let reservation_bytes =
                i64::try_from(reservation_bytes).map_err(|_| corrupt("reservation"))?;
            let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
            // Count and insert as one decision: two uploads racing for the last place serialise here
            // and the second one sees the first. Released with the transaction.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!("blobs:{}", access.tenant_id))
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            let unused: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM blobs WHERE tenant_id = $1 AND state <> 'COMMITTED' \
                 AND deletion_confirmed_at IS NULL",
            )
            .bind(access.tenant_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            if u64::try_from(unused).unwrap_or(u64::MAX) >= u64::from(max_pending) {
                return Err(StoreError::QuotaExceeded { what: UNUSED_BLOBS });
            }
            let inserted = sqlx::query(
                "INSERT INTO blobs (tenant_id, namespace_id, id, state, content_type, created_by, \
                                    run_id, step_id, attempt, generation, purpose, reservation_bytes) \
                 SELECT $1, $2, $3, 'UPLOADING', $4, $5, $6, $7, $8, $9, $10, $11 \
                 FROM namespaces WHERE tenant_id = $1 AND id = $2 \
                   AND ($10 = 'RUN_INPUT' OR EXISTS (SELECT 1 FROM steps \
                       WHERE tenant_id = $1 AND namespace_id = $2 AND run_id = $6 AND step_id = $7 \
                         AND attempt = $8 AND generation = $9))",
            )
            .bind(access.tenant_id.as_uuid())
            .bind(access.namespace_id.as_uuid())
            .bind(id.as_uuid())
            .bind(content_type.as_str())
            .bind(created_by)
            .bind(scope.map(|s| s.run_id.as_uuid()))
            .bind(scope.map(|s| s.step_id))
            .bind(
                scope
                    .map(|s| i32::try_from(s.attempt))
                    .transpose()
                    .map_err(|_| corrupt("attempt"))?,
            )
            .bind(
                scope
                    .map(|s| i64::try_from(s.generation))
                    .transpose()
                    .map_err(|_| corrupt("generation"))?,
            )
            .bind(purpose)
            .bind(reservation_bytes)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            if inserted != 1 {
                return Err(StoreError::MissingReference);
            }
            tx.commit().await.map_err(store_error)
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
            let size = i64::try_from(size).map_err(|_| corrupt("size"))?;
            let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
            let row: Option<(String, bool)> = sqlx::query_as(
                "UPDATE blobs SET state = CASE WHEN state = 'UPLOADING' THEN 'UPLOADED' \
                                               ELSE 'DELETING' END, \
                     size_bytes = $4, sha256 = $5, uploaded_at = clock_timestamp(), \
                     delete_after = CASE WHEN state = 'DELETING' THEN clock_timestamp() \
                                         ELSE delete_after END, \
                     deletion_confirmed_at = CASE WHEN state = 'DELETING' THEN NULL \
                                                  ELSE deletion_confirmed_at END, \
                     deletion_attempts = CASE WHEN state = 'DELETING' \
                                              THEN deletion_attempts + 1 \
                                              ELSE deletion_attempts END \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 \
                   AND state IN ('UPLOADING', 'DELETING') AND uploaded_at IS NULL \
                   AND $4 <= reservation_bytes \
                 RETURNING created_by, state = 'DELETING'",
            )
            .bind(access.tenant_id.as_uuid())
            .bind(access.namespace_id.as_uuid())
            .bind(id.as_uuid())
            .bind(size)
            .bind(digest.as_bytes().as_slice())
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            let Some((actor, deleting)) = row else {
                // Already finalized, committed, or never reserved.
                return Err(StoreError::MissingReference);
            };
            append_blob_audit(&mut tx, access, id, &actor, AuditAction::BlobUploaded, None).await?;
            tx.commit().await.map_err(store_error)?;
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
            let row = sqlx::query(
                "SELECT state, size_bytes, sha256, content_type, run_id, step_id, attempt, generation, purpose, created_by \
                 FROM blobs WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3",
            )
            .bind(access.tenant_id.as_uuid())
            .bind(access.namespace_id.as_uuid())
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            let Some(row) = row else {
                return Ok(None);
            };
            let state = BlobState::parse_stored(&col::<String>(&row, "state")?)
                .ok_or_else(|| corrupt("state"))?;
            let size: Option<i64> = col(&row, "size_bytes")?;
            let sha: Option<Vec<u8>> = col(&row, "sha256")?;
            let digest = match sha {
                Some(bytes) => {
                    Digest::new(<[u8; 32]>::try_from(bytes).map_err(|_| corrupt("digest"))?)
                }
                None => Digest::new([0; 32]),
            };
            let content_type = ContentType::parse(&col::<String>(&row, "content_type")?)
                .map_err(|_| corrupt("content type"))?;
            Ok(Some(BlobRecord {
                descriptor: BlobDescriptor::new(
                    id,
                    u64::try_from(size.unwrap_or(0)).map_err(|_| corrupt("size"))?,
                    digest,
                    content_type,
                ),
                state,
                owner: match col::<String>(&row, "purpose")?.as_str() {
                    "RUN_INPUT" => BlobOwner::RunInput(access),
                    "STEP_OUTPUT" => BlobOwner::StepOutput(BlobScope {
                        access,
                        run_id: RunId::new(col::<Uuid>(&row, "run_id")?),
                        step_id: col(&row, "step_id")?,
                        attempt: u32::try_from(col::<i32>(&row, "attempt")?)
                            .map_err(|_| corrupt("attempt"))?,
                        generation: u64::try_from(col::<i64>(&row, "generation")?)
                            .map_err(|_| corrupt("generation"))?,
                    }),
                    _ => return Err(corrupt("purpose")),
                },
                created_by: col(&row, "created_by")?,
            }))
        })
    }

    fn claim_delete<'a>(
        &'a self,
        access: BlobAccess,
        id: BlobId,
        retry_after: Duration,
    ) -> BoxFuture<'a, Result<Option<BlobDeleteClaim>, StoreError>> {
        Box::pin(async move {
            let retry_micros = i64::try_from(retry_after.as_micros()).unwrap_or(i64::MAX);
            // The claim is durable before the object-store call. A retry may reclaim a failed
            // deletion only after its delay; a committed blob is never eligible.
            let attempt: Option<i32> = sqlx::query_scalar(
                "UPDATE blobs SET state = 'DELETING', deletion_attempts = deletion_attempts + 1, \
                     deletion_last_attempt_at = clock_timestamp(), \
                     delete_after = clock_timestamp() + $4 * interval '1 microsecond' \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 \
                   AND (state IN ('UPLOADING', 'UPLOADED') \
                        OR (state = 'DELETING' AND delete_after <= clock_timestamp())) \
                 RETURNING deletion_attempts",
            )
            .bind(access.tenant_id.as_uuid())
            .bind(access.namespace_id.as_uuid())
            .bind(id.as_uuid())
            .bind(retry_micros)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            attempt
                .map(|attempt| {
                    Ok(BlobDeleteClaim {
                        access,
                        id,
                        attempt: u32::try_from(attempt).map_err(|_| corrupt("deletion attempt"))?,
                    })
                })
                .transpose()
        })
    }

    fn claim_expired<'a>(
        &'a self,
        abandoned_after: Duration,
        unused_for: Duration,
        retry_after: Duration,
        limit: u32,
    ) -> BoxFuture<'a, Result<Vec<BlobDeleteClaim>, StoreError>> {
        Box::pin(async move {
            let micros = |d: Duration| i64::try_from(d.as_micros()).unwrap_or(i64::MAX);
            // One statement picks and marks. A blob being committed holds its row lock, so it is
            // skipped; one committed a moment ago no longer matches. A failed external delete is
            // retained as DELETING and becomes eligible again at delete_after.
            let rows = sqlx::query(
                "WITH due AS ( \
                     SELECT tenant_id, namespace_id, id FROM blobs \
                     WHERE (state = 'UPLOADING' \
                              AND created_at < clock_timestamp() - $1 * interval '1 microsecond') \
                        OR (state = 'UPLOADED' \
                              AND uploaded_at < clock_timestamp() - $2 * interval '1 microsecond') \
                        OR (state = 'DELETING' AND delete_after <= clock_timestamp()) \
                     ORDER BY COALESCE(delete_after, uploaded_at, created_at), tenant_id, id LIMIT $3 \
                     FOR UPDATE SKIP LOCKED) \
                 UPDATE blobs b SET state = 'DELETING', \
                     deletion_attempts = b.deletion_attempts + 1, \
                     deletion_last_attempt_at = clock_timestamp(), \
                     delete_after = clock_timestamp() + $4 * interval '1 microsecond' \
                 FROM due \
                 WHERE b.tenant_id = due.tenant_id AND b.namespace_id = due.namespace_id \
                   AND b.id = due.id \
                 RETURNING b.tenant_id, b.namespace_id, b.id, b.deletion_attempts",
            )
            .bind(micros(abandoned_after))
            .bind(micros(unused_for))
            .bind(i64::from(limit))
            .bind(micros(retry_after))
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
            rows.iter()
                .map(|row| {
                    let tenant: Uuid = col(row, "tenant_id")?;
                    let namespace: Uuid = col(row, "namespace_id")?;
                    let id: Uuid = row.try_get("id").map_err(store_error)?;
                    let attempt: i32 = col(row, "deletion_attempts")?;
                    Ok(BlobDeleteClaim {
                        access: BlobAccess {
                            tenant_id: TenantId::new(tenant),
                            namespace_id: NamespaceId::new(namespace),
                        },
                        id: BlobId::new(id),
                        attempt: u32::try_from(attempt).map_err(|_| corrupt("deletion attempt"))?,
                    })
                })
                .collect()
        })
    }

    fn finish_delete<'a>(
        &'a self,
        claim: BlobDeleteClaim,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let attempt = i32::try_from(claim.attempt).map_err(|_| corrupt("deletion attempt"))?;
            let access = claim.access;
            let id = claim.id;
            let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
            // A successful delete does not prove an earlier, unconfirmed PUT can no longer finish.
            // Lock before deciding: only a previously confirmed upload can lose its catalog row.
            let row = sqlx::query(
                "SELECT uploaded_at IS NOT NULL AS finalized, \
                        deletion_confirmed_at IS NOT NULL AS confirmed \
                 FROM blobs WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 \
                 AND state = 'DELETING' AND deletion_attempts = $4 FOR UPDATE",
            )
            .bind(access.tenant_id.as_uuid())
            .bind(access.namespace_id.as_uuid())
            .bind(id.as_uuid())
            .bind(attempt)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            let Some(row) = row else {
                return tx.commit().await.map_err(store_error);
            };
            let finalized: bool = col(&row, "finalized")?;
            let confirmed: bool = col(&row, "confirmed")?;
            let statement = if finalized {
                "DELETE FROM blobs WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3"
            } else {
                // Keep delete_after and the attempt evidence. A later sweep deletes again even if
                // the uploader crashed before it could report its late successful PUT.
                "UPDATE blobs SET deletion_confirmed_at = \
                    COALESCE(deletion_confirmed_at, clock_timestamp()) \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3"
            };
            sqlx::query(statement)
                .bind(access.tenant_id.as_uuid())
                .bind(access.namespace_id.as_uuid())
                .bind(id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            if !confirmed {
                append_blob_audit(
                    &mut tx,
                    access,
                    id,
                    Subject::engine().as_str(),
                    AuditAction::BlobDeleted,
                    Some("unused blob cleanup"),
                )
                .await?;
            }
            tx.commit().await.map_err(store_error)
        })
    }
}

/// Marks a blob `Committed` as part of the transaction that stores the record referring to it.
///
/// Succeeds only for a blob of this tenant that is `Uploaded` with exactly this size and digest.
/// Anything else (never uploaded, swept, already used, another tenant's, a different content) is
/// [`StoreError::MissingReference`], which the callers report as "not found", the same answer for
/// all of them.
pub(crate) async fn commit(
    conn: &mut PgConnection,
    tenant: TenantId,
    blob: &BlobCommit,
) -> Result<(), StoreError> {
    if blob.scope.access.tenant_id != tenant {
        return Err(StoreError::MissingReference);
    }
    let size = i64::try_from(blob.size).map_err(|_| corrupt("size"))?;
    let changed = sqlx::query(
        "UPDATE blobs SET state = 'COMMITTED', committed_at = clock_timestamp() \
         WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 AND state = 'UPLOADED' \
           AND run_id = $4 AND step_id = $5 AND attempt = $6 AND generation = $7 \
           AND size_bytes = $8 AND sha256 = $9 AND content_type = $10",
    )
    .bind(tenant.as_uuid())
    .bind(blob.scope.access.namespace_id.as_uuid())
    .bind(blob.id.as_uuid())
    .bind(blob.scope.run_id.as_uuid())
    .bind(blob.scope.step_id)
    .bind(i32::try_from(blob.scope.attempt).map_err(|_| corrupt("attempt"))?)
    .bind(i64::try_from(blob.scope.generation).map_err(|_| corrupt("generation"))?)
    .bind(size)
    .bind(blob.digest.as_bytes().as_slice())
    .bind(blob.content_type.as_str())
    .execute(conn)
    .await
    .map_err(store_error)?
    .rows_affected();
    if changed == 1 {
        Ok(())
    } else {
        Err(StoreError::MissingReference)
    }
}

/// Binds a staged run input after its run row exists, in that same creation transaction.
pub(crate) async fn commit_input(
    conn: &mut PgConnection,
    run: &sagat_application::execution::RunInfo,
    descriptor: &BlobDescriptor,
) -> Result<(), StoreError> {
    let changed = sqlx::query(
        "UPDATE blobs SET state = 'COMMITTED', committed_at = clock_timestamp(), run_id = $4 \
         WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 \
           AND purpose = 'RUN_INPUT' AND state = 'UPLOADED' AND created_by = $5 \
           AND size_bytes = $6 AND sha256 = $7 AND content_type = $8",
    )
    .bind(run.tenant_id.as_uuid())
    .bind(run.namespace_id.as_uuid())
    .bind(descriptor.id().as_uuid())
    .bind(run.run_id.as_uuid())
    .bind(&run.started_by)
    .bind(i64::try_from(descriptor.size()).map_err(|_| corrupt("size"))?)
    .bind(descriptor.digest().as_bytes().as_slice())
    .bind(descriptor.content_type().as_str())
    .execute(conn)
    .await
    .map_err(store_error)?
    .rows_affected();
    if changed == 1 {
        Ok(())
    } else {
        Err(StoreError::MissingReference)
    }
}

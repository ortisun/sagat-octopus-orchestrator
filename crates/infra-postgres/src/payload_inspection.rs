//! Tenant-scoped, audited payload selection for ADR-0028.

use sagat_application::blobs::descriptor_json;
use sagat_application::error::StoreError;
use sagat_application::payloads::{
    ExternalContentKind, InspectionRun, PayloadInspectionStore, PayloadSelector,
    PreparedInspection, SelectedPayload, StoredPayload, StoredPayloadBytes,
};
use sagat_application::runs::RunStore;
use sagat_domain::audit::AuditRecord;
use sagat_domain::blobs::{BlobDescriptor, BlobId, ContentType};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::Digest;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;

impl PayloadInspectionStore for PgRunStore {
    async fn inspection_run(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<InspectionRun>, StoreError> {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT namespace_id FROM runs WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map(|namespace| {
            namespace.map(|id| InspectionRun {
                namespace_id: NamespaceId::new(id),
            })
        })
        .map_err(store_error)
    }

    async fn run_exists_outside(&self, tenant: TenantId, run: RunId) -> Result<bool, StoreError> {
        RunStore::run_exists_outside(self, tenant, run).await
    }

    async fn record_inspection_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        RunStore::record_denial(self, audit).await
    }

    async fn inspect_and_audit(
        &self,
        prepared: &PreparedInspection,
        audit: &AuditRecord,
    ) -> Result<Option<SelectedPayload>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let Some(run) = sqlx::query(
            "SELECT namespace_id, run_version, input_digest, input_blob_id \
             FROM runs WHERE tenant_id = $1 AND id = $2 FOR SHARE",
        )
        .bind(prepared.tenant_id().as_uuid())
        .bind(prepared.run_id().as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let namespace = NamespaceId::new(col::<Uuid>(&run, "namespace_id")?);
        if namespace != prepared.namespace_id() {
            return Err(corrupt("prepared inspection namespace"));
        }
        let run_version =
            u64::try_from(col::<i64>(&run, "run_version")?).map_err(|_| corrupt("run version"))?;

        let selected = match prepared.selector() {
            PayloadSelector::Input => {
                let digest: Vec<u8> = col(&run, "input_digest")?;
                let content = payload_content(&mut tx, prepared.tenant_id(), &digest).await?;
                let payload = match col::<Option<Uuid>>(&run, "input_blob_id")? {
                    Some(id) => {
                        let blob = load_input_blob(&mut tx, prepared.tenant_id(), namespace, id)
                            .await?
                            .ok_or_else(|| corrupt("committed run input blob"))?;
                        if descriptor_json(&blob.descriptor) != content {
                            return Err(corrupt("run input blob descriptor"));
                        }
                        StoredPayload::External {
                            size_bytes: blob.descriptor.size(),
                            content_kind: content_kind(blob.descriptor.content_type()),
                        }
                    }
                    None => StoredPayload::Inline(StoredPayloadBytes::new(content)),
                };
                SelectedPayload {
                    run_version,
                    node_id: None,
                    attempt: None,
                    payload,
                }
            }
            PayloadSelector::Journal { sequence } => {
                let sequence = i64::try_from(sequence).map_err(|_| corrupt("journal sequence"))?;
                let Some(event) = sqlx::query(
                    "SELECT e.node, e.attempt, e.payload_digest, p.content \
                     FROM run_events e LEFT JOIN payloads p \
                       ON p.tenant_id = e.tenant_id AND p.digest = e.payload_digest \
                     WHERE e.tenant_id = $1 AND e.run_id = $2 AND e.seq = $3",
                )
                .bind(prepared.tenant_id().as_uuid())
                .bind(prepared.run_id().as_uuid())
                .bind(sequence)
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?
                else {
                    return Ok(None);
                };
                let node: Option<String> = col(&event, "node")?;
                let attempt: Option<i32> = col(&event, "attempt")?;
                let content: Option<Vec<u8>> = col(&event, "content")?;
                let payload = match content {
                    None => StoredPayload::NoRetainedPayload,
                    Some(content) => match (&node, attempt) {
                        (Some(node), Some(attempt)) => {
                            let blobs = load_step_blobs(
                                &mut tx,
                                prepared.tenant_id(),
                                namespace,
                                prepared.run_id(),
                                node,
                                attempt,
                            )
                            .await?;
                            match blobs
                                .into_iter()
                                .find(|blob| descriptor_json(&blob.descriptor) == content)
                            {
                                Some(blob) => StoredPayload::External {
                                    size_bytes: blob.descriptor.size(),
                                    content_kind: content_kind(blob.descriptor.content_type()),
                                },
                                _ => StoredPayload::Inline(StoredPayloadBytes::new(content)),
                            }
                        }
                        _ => StoredPayload::Inline(StoredPayloadBytes::new(content)),
                    },
                };
                SelectedPayload {
                    run_version,
                    node_id: node,
                    attempt: attempt
                        .map(u32::try_from)
                        .transpose()
                        .map_err(|_| corrupt("journal attempt"))?,
                    payload,
                }
            }
        };

        insert_audit(&mut tx, audit)
            .await
            .map_err(|error| StoreError::Unavailable {
                detail: format!("payload inspection audit could not be appended: {error}"),
            })?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(selected))
    }
}

async fn payload_content(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    digest: &[u8],
) -> Result<Vec<u8>, StoreError> {
    sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
        .bind(tenant.as_uuid())
        .bind(digest)
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_error)?
        .ok_or_else(|| corrupt("run input payload"))
}

struct CatalogBlob {
    descriptor: BlobDescriptor,
}

async fn load_input_blob(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    id: Uuid,
) -> Result<Option<CatalogBlob>, StoreError> {
    let row = sqlx::query(
        "SELECT id, size_bytes, sha256, content_type FROM blobs \
         WHERE tenant_id = $1 AND namespace_id = $2 AND state = 'COMMITTED' \
           AND id = $3 AND purpose = 'RUN_INPUT'",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    row.as_ref().map(blob_from_row).transpose()
}

async fn load_step_blobs(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    run: RunId,
    node: &str,
    attempt: i32,
) -> Result<Vec<CatalogBlob>, StoreError> {
    let rows = sqlx::query(
        "SELECT b.id, b.size_bytes, b.sha256, b.content_type \
         FROM blobs b JOIN steps s \
           ON s.tenant_id = b.tenant_id AND s.step_id = b.step_id \
         WHERE b.tenant_id = $1 AND b.namespace_id = $2 AND b.state = 'COMMITTED' \
           AND b.purpose = 'STEP_OUTPUT' AND b.run_id = $3 \
           AND s.node = $4 AND b.attempt = $5 \
         ORDER BY b.committed_at DESC LIMIT 64",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(run.as_uuid())
    .bind(node)
    .bind(attempt)
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    rows.iter().map(blob_from_row).collect()
}

fn blob_from_row(row: &sqlx::postgres::PgRow) -> Result<CatalogBlob, StoreError> {
    let sha =
        <[u8; 32]>::try_from(col::<Vec<u8>>(row, "sha256")?).map_err(|_| corrupt("blob digest"))?;
    let size = u64::try_from(col::<i64>(row, "size_bytes")?).map_err(|_| corrupt("blob size"))?;
    let content_type = ContentType::parse(&col::<String>(row, "content_type")?)
        .map_err(|_| corrupt("blob content type"))?;
    Ok(CatalogBlob {
        descriptor: BlobDescriptor::new(
            BlobId::new(col(row, "id")?),
            size,
            Digest::new(sha),
            content_type,
        ),
    })
}

fn content_kind(content_type: &ContentType) -> ExternalContentKind {
    let media_type = content_type.as_str();
    if media_type == "application/json" || media_type.ends_with("+json") {
        ExternalContentKind::Json
    } else if media_type.starts_with("text/") {
        ExternalContentKind::Text
    } else if media_type == "application/octet-stream"
        || media_type.starts_with("image/")
        || media_type.starts_with("audio/")
        || media_type.starts_with("video/")
    {
        ExternalContentKind::Binary
    } else {
        ExternalContentKind::Other
    }
}

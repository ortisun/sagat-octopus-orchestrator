//! Current authorization and retained output identity, fenced by the run and membership locks.
use crate::{
    database::current_permission,
    execution::PgRunStore,
    identity::{col, corrupt, insert_audit, store_error},
    pool::BeginSafely,
};
use sagat_application::{
    blobs::descriptor_json,
    database_results::{
        DatabaseResultContent, DatabaseResultRequest, DatabaseResultStore, SelectedDatabaseResult,
    },
    error::StoreError,
    payloads::StoredPayloadBytes,
};
use sagat_domain::{
    audit::AuditRecord,
    blobs::{BlobDescriptor, BlobId, ContentType},
    identity::Permission,
    workflow::Digest,
};
use sqlx::{PgConnection, postgres::PgRow};

impl DatabaseResultStore for PgRunStore {
    async fn select_database_result(
        &self,
        request: &DatabaseResultRequest,
    ) -> Result<Option<SelectedDatabaseResult>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let Some(row) = selected(&mut tx, request).await? else {
            return Ok(None);
        };
        let digest = digest(&col::<Vec<u8>>(&row, "output_digest")?)?;
        let bytes: Vec<u8> = col(&row, "content")?;
        let blob = sqlx::query(
            "SELECT id,size_bytes,sha256,content_type FROM blobs \
            WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3 AND step_id=$4 \
            AND attempt=$5 AND generation=$6 AND purpose='STEP_OUTPUT' AND state='COMMITTED'",
        )
        .bind(request.principal.tenant_id().as_uuid())
        .bind(request.namespace.as_uuid())
        .bind(request.run.as_uuid())
        .bind(col::<uuid::Uuid>(&row, "step_id")?)
        .bind(col::<i32>(&row, "attempt")?)
        .bind(col::<i64>(&row, "generation")?)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let content = match blob {
            None => DatabaseResultContent::Inline(StoredPayloadBytes::new(bytes)),
            Some(blob) => {
                let descriptor = blob_descriptor(&blob)?;
                if descriptor_json(&descriptor) != bytes {
                    return Err(corrupt("database result blob provenance"));
                }
                DatabaseResultContent::Blob(descriptor)
            }
        };
        tx.commit().await.map_err(store_error)?;
        Ok(Some(SelectedDatabaseResult { digest, content }))
    }
    async fn audit_database_result(
        &self,
        request: &DatabaseResultRequest,
        expected: &Digest,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let row = selected(&mut tx, request)
            .await?
            .ok_or(StoreError::MissingReference)?;
        if digest(&col::<Vec<u8>>(&row, "output_digest")?)? != *expected {
            return Err(StoreError::MissingReference);
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }
    async fn deny_database_result(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }
}

async fn selected(
    conn: &mut PgConnection,
    request: &DatabaseResultRequest,
) -> Result<Option<PgRow>, StoreError> {
    // Match engine lock order: run before current membership. Never keep these across object I/O.
    let exists: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM runs WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR SHARE",
    )
    .bind(request.principal.tenant_id().as_uuid())
    .bind(request.namespace.as_uuid())
    .bind(request.run.as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    if exists.is_none() {
        return Ok(None);
    }
    current_permission(conn, &request.principal, Permission::RunRead).await?;
    current_permission(conn, &request.principal, Permission::DatabaseResultRead).await?;
    sqlx::query("SELECT s.step_id,s.attempt,s.generation,s.output_digest,p.content FROM steps s \
        JOIN run_database_connections d ON d.tenant_id=s.tenant_id AND d.run_id=s.run_id AND d.node_id=s.node \
        JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest \
        WHERE s.tenant_id=$1 AND d.namespace_id=$2 AND s.run_id=$3 AND s.node=$4 \
        AND s.status='SUCCEEDED' FOR SHARE OF s")
        .bind(request.principal.tenant_id().as_uuid()).bind(request.namespace.as_uuid()).bind(request.run.as_uuid())
        .bind(request.node.as_str()).fetch_optional(conn).await.map_err(store_error)
}
fn digest(bytes: &[u8]) -> Result<Digest, StoreError> {
    Ok(Digest::new(
        bytes
            .try_into()
            .map_err(|_| corrupt("database result digest"))?,
    ))
}
fn blob_descriptor(row: &PgRow) -> Result<BlobDescriptor, StoreError> {
    Ok(BlobDescriptor::new(
        BlobId::new(col(row, "id")?),
        u64::try_from(col::<i64>(row, "size_bytes")?)
            .map_err(|_| corrupt("database result size"))?,
        digest(&col::<Vec<u8>>(row, "sha256")?)?,
        ContentType::parse(&col::<String>(row, "content_type")?)
            .map_err(|_| corrupt("database result media type"))?,
    ))
}

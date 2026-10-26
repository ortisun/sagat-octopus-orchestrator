//! Provider cleanup uses short authority transactions around external retirement.
use crate::{
    execution::PgRunStore,
    identity::{col, insert_audit, store_error},
    pool::BeginSafely,
    quotas::lock_secrets,
};
use sagat_application::{
    error::StoreError,
    secret_cleanup::{CleanupClaim, SecretCleanupStore},
    secret_provider::ProviderReference,
};
use sagat_domain::{
    audit::{AuditAction, AuditRecord, ResourceType},
    identity::{NamespaceId, TenantId},
};
use uuid::Uuid;

impl SecretCleanupStore for PgRunStore {
    async fn release_terminal_credential_pins(&self, limit: u32) -> Result<u64, StoreError> {
        let rows:Vec<(Uuid,Uuid)>=sqlx::query_as("SELECT r.tenant_id,r.id FROM runs r
            WHERE r.finished_at<now()-interval '24 hours'
              AND r.status IN ('SUCCEEDED','FAILED','CANCELLED','TIMED_OUT','COMPENSATED','PARTIAL','COMPENSATION_FAILED')
              AND EXISTS(SELECT 1 FROM step_database_credentials p WHERE p.tenant_id=r.tenant_id AND p.run_id=r.id)
            ORDER BY r.finished_at,r.id LIMIT $1").bind(i64::from(limit.min(128))).fetch_all(&self.pool).await.map_err(store_error)?;
        let mut released = 0;
        for (tenant, run) in rows {
            let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
            // Never take another run lock after acquiring this run's namespace secret lock.
            let namespace:Option<Uuid>=sqlx::query_scalar("SELECT namespace_id FROM runs WHERE tenant_id=$1 AND id=$2
                AND finished_at<clock_timestamp()-interval '24 hours'
                AND status IN ('SUCCEEDED','FAILED','CANCELLED','TIMED_OUT','COMPENSATED','PARTIAL','COMPENSATION_FAILED')
                FOR UPDATE SKIP LOCKED").bind(tenant).bind(run).fetch_optional(&mut *tx).await.map_err(store_error)?;
            let Some(namespace) = namespace else {
                continue;
            };
            lock_secrets(&mut tx, TenantId::new(tenant), NamespaceId::new(namespace)).await?;
            let count=sqlx::query("DELETE FROM step_database_credentials WHERE tenant_id=$1 AND namespace_id=$2 AND run_id=$3")
                .bind(tenant).bind(namespace).bind(run).execute(&mut *tx).await.map_err(store_error)?.rows_affected();
            if count > 0 {
                let now = sqlx::query_scalar("SELECT clock_timestamp()")
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(store_error)?;
                insert_audit(
                    &mut tx,
                    &AuditRecord::provisioning(
                        TenantId::new(tenant),
                        AuditAction::SecretAttemptPinsReleased,
                        ResourceType::Run,
                        run.to_string(),
                        "terminal credential retention ended",
                        now,
                    ),
                )
                .await?;
            }
            tx.commit().await.map_err(store_error)?;
            released += count;
        }
        Ok(released)
    }
    async fn provider_cleanup_candidates(
        &self,
        limit: u32,
    ) -> Result<Vec<ProviderReference>, StoreError> {
        let rows:Vec<(Uuid,Uuid,Uuid)>=sqlx::query_as("SELECT o.tenant_id,o.namespace_id,o.id FROM secret_provider_operations o
            WHERE greatest(o.created_at,coalesce(o.activated_at,o.created_at))<now()-interval '24 hours'
              AND coalesce(o.cleanup_expires_at,'-infinity'::timestamptz)<=now()
              AND NOT EXISTS(SELECT 1 FROM secret_versions v WHERE v.tenant_id=o.tenant_id AND v.provider_operation=o.id)
              AND NOT EXISTS(SELECT 1 FROM step_database_credentials p WHERE p.tenant_id=o.tenant_id AND p.namespace_id=o.namespace_id AND p.provider_operation=o.id)
            ORDER BY coalesce(o.cleanup_expires_at,'-infinity'::timestamptz),o.created_at,o.id LIMIT $1")
            .bind(i64::from(limit.min(128))).fetch_all(&self.pool).await.map_err(store_error)?;
        Ok(rows
            .into_iter()
            .map(|(tenant, namespace, operation)| ProviderReference {
                tenant: TenantId::new(tenant),
                namespace: NamespaceId::new(namespace),
                operation,
            })
            .collect())
    }
    async fn claim_provider_cleanup(
        &self,
        reference: &ProviderReference,
    ) -> Result<Option<CleanupClaim>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_secrets(&mut tx, reference.tenant, reference.namespace).await?;
        let row=sqlx::query("SELECT state,clock_timestamp() AS now FROM secret_provider_operations o
            WHERE o.tenant_id=$1 AND o.namespace_id=$2 AND o.id=$3
              AND greatest(o.created_at,coalesce(o.activated_at,o.created_at))<clock_timestamp()-interval '24 hours'
              AND coalesce(o.cleanup_expires_at,'-infinity'::timestamptz)<=clock_timestamp()
              AND NOT EXISTS(SELECT 1 FROM secret_versions v WHERE v.tenant_id=o.tenant_id AND v.provider_operation=o.id)
              AND NOT EXISTS(SELECT 1 FROM step_database_credentials p WHERE p.tenant_id=o.tenant_id AND p.namespace_id=o.namespace_id AND p.provider_operation=o.id)
            FOR UPDATE").bind(reference.tenant.as_uuid()).bind(reference.namespace.as_uuid()).bind(reference.operation)
            .fetch_optional(&mut *tx).await.map_err(store_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let lease = Uuid::new_v4();
        sqlx::query("UPDATE secret_provider_operations SET state='abandoned',cleanup_lease=$4,cleanup_expires_at=clock_timestamp()+interval '90 seconds'
            WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3")
            .bind(reference.tenant.as_uuid()).bind(reference.namespace.as_uuid()).bind(reference.operation).bind(lease)
            .execute(&mut *tx).await.map_err(store_error)?;
        if col::<String>(&row, "state")? != "abandoned" {
            insert_audit(
                &mut tx,
                &AuditRecord::provisioning(
                    reference.tenant,
                    AuditAction::SecretProviderAbandoned,
                    ResourceType::Secret,
                    reference.operation.to_string(),
                    "provider receipt retention ended without retained references",
                    col(&row, "now")?,
                ),
            )
            .await?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(Some(CleanupClaim {
            reference: reference.clone(),
            lease,
        }))
    }
    async fn finish_provider_cleanup(&self, claim: &CleanupClaim) -> Result<bool, StoreError> {
        let r = &claim.reference;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_secrets(&mut tx, r.tenant, r.namespace).await?;
        let removed=sqlx::query("DELETE FROM secret_provider_operations o WHERE o.tenant_id=$1 AND o.namespace_id=$2 AND o.id=$3
            AND o.state='abandoned' AND o.cleanup_lease=$4 AND o.cleanup_expires_at>clock_timestamp()
            AND NOT EXISTS(SELECT 1 FROM secret_versions v WHERE v.tenant_id=o.tenant_id AND v.provider_operation=o.id)
            AND NOT EXISTS(SELECT 1 FROM step_database_credentials p WHERE p.tenant_id=o.tenant_id AND p.namespace_id=o.namespace_id AND p.provider_operation=o.id)")
            .bind(r.tenant.as_uuid()).bind(r.namespace.as_uuid()).bind(r.operation).bind(claim.lease)
            .execute(&mut *tx).await.map_err(store_error)?.rows_affected()>0;
        if removed {
            let now = sqlx::query_scalar("SELECT clock_timestamp()")
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
            insert_audit(
                &mut tx,
                &AuditRecord::provisioning(
                    r.tenant,
                    AuditAction::SecretProviderRetired,
                    ResourceType::Secret,
                    r.operation.to_string(),
                    "provider value retired; metadata tombstone retained",
                    now,
                ),
            )
            .await?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(removed)
    }
}

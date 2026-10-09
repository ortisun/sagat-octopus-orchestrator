//! The egress policy in PostgreSQL (FR-009-008).

use sagat_application::egress::{EgressStore, StoredPolicy};
use sagat_application::error::StoreError;
use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{NamespaceId, TenantId};

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, insert_audit, store_error};

impl EgressStore for PgRunStore {
    async fn egress_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Option<StoredPolicy>, StoreError> {
        let row = sqlx::query(
            "SELECT rules, ports, allow_http, updated_by, updated_at FROM egress_policies \
             WHERE tenant_id = $1 AND namespace_id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        row.map(|row| {
            let ports: Vec<i32> = col(&row, "ports")?;
            Ok(StoredPolicy {
                rules: col(&row, "rules")?,
                ports: ports
                    .into_iter()
                    .map(|p| u16::try_from(p).map_err(|_| corrupt("egress port")))
                    .collect::<Result<_, _>>()?,
                allow_http: col(&row, "allow_http")?,
                updated_by: col(&row, "updated_by")?,
                updated_at: col(&row, "updated_at")?,
            })
        })
        .transpose()
    }

    async fn put_egress_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        policy: &StoredPolicy,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let ports: Vec<i32> = policy.ports.iter().map(|p| i32::from(*p)).collect();
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        sqlx::query(
            "INSERT INTO egress_policies \
             (tenant_id, namespace_id, rules, ports, allow_http, updated_by, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp()) \
             ON CONFLICT (tenant_id, namespace_id) DO UPDATE SET \
                 rules = EXCLUDED.rules, ports = EXCLUDED.ports, allow_http = EXCLUDED.allow_http, \
                 updated_by = EXCLUDED.updated_by, updated_at = EXCLUDED.updated_at",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(&policy.rules)
        .bind(&ports)
        .bind(policy.allow_http)
        .bind(&policy.updated_by)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn delete_egress_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        let removed =
            sqlx::query("DELETE FROM egress_policies WHERE tenant_id = $1 AND namespace_id = $2")
                .bind(tenant.as_uuid())
                .bind(namespace.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?
                .rows_affected()
                > 0;
        if removed {
            insert_audit(&mut tx, audit).await?;
            tx.commit().await.map_err(store_error)?;
        }
        Ok(removed)
    }
}

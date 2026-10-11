//! Worker bindings in PostgreSQL (FR-009-002, gap G31).

use crate::pool::BeginSafely;
use sagat_application::bindings::{BindingStore, Revoked, StoredBinding};
use sagat_application::error::StoreError;
use sagat_domain::audit::AuditRecord;
use sagat_domain::binding::{ActivityPattern, first_uncovered};
use sagat_domain::identity::{NamespaceId, TenantId};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{insert_audit, store_error};

impl BindingStore for PgRunStore {
    async fn bindings(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Vec<StoredBinding>, StoreError> {
        let rows: Vec<(String, Vec<String>)> = sqlx::query_as(
            "SELECT subject, array_agg(pattern ORDER BY pattern) FROM worker_bindings \
             WHERE tenant_id = $1 AND namespace_id = $2 GROUP BY subject ORDER BY subject",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(rows
            .into_iter()
            .map(|(subject, patterns)| StoredBinding { subject, patterns })
            .collect())
    }

    async fn replace_binding(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
        patterns: &[String],
        changed_by: &str,
        audit: &AuditRecord,
    ) -> Result<Option<Revoked>, StoreError> {
        let parsed = patterns
            .iter()
            .map(|text| ActivityPattern::parse(text))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| StoreError::Internal {
                detail: "a binding pattern does not parse".into(),
            })?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // One change at a time per identity and namespace, so two administrators cannot interleave
        // a replace with a revocation.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("binding:{tenant}:{namespace}:{subject}"))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let removed = sqlx::query(
            "DELETE FROM worker_bindings \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if patterns.is_empty() && removed == 0 {
            // Nothing to remove: the transaction ends without a trace.
            return Ok(None);
        }
        if !patterns.is_empty() {
            sqlx::query(
                "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by) \
                 SELECT $1, $2, $3, p, $5 FROM unnest($4::text[]) AS t(p)",
            )
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(subject)
            .bind(patterns)
            .bind(changed_by)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }

        // The sessions of this identity in this namespace that the new binding no longer covers are
        // closed, and the attempts they hold are released: the scheduler reclaims them as lost
        // leases, and the fencing makes the revoked worker's late reports stale.
        let sessions: Vec<(Uuid, Vec<String>)> = sqlx::query_as(
            "SELECT id, activities FROM worker_sessions \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let doomed: Vec<Uuid> = sessions
            .into_iter()
            .filter(|(_, activities)| first_uncovered(&parsed, activities).is_some())
            .map(|(id, _)| id)
            .collect();
        let mut revoked = Revoked::default();
        if !doomed.is_empty() {
            let names: Vec<String> = doomed.iter().map(Uuid::to_string).collect();
            revoked.leases = sqlx::query(
                "UPDATE steps SET lease_expires_at = clock_timestamp(), updated_at = clock_timestamp() \
                 WHERE tenant_id = $1 AND status IN ('LEASED', 'RUNNING') AND leased_by = ANY($2)",
            )
            .bind(tenant.as_uuid())
            .bind(&names)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            revoked.sessions =
                sqlx::query("DELETE FROM worker_sessions WHERE tenant_id = $1 AND id = ANY($2)")
                    .bind(tenant.as_uuid())
                    .bind(&doomed)
                    .execute(&mut *tx)
                    .await
                    .map_err(store_error)?
                    .rows_affected();
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(revoked))
    }
}

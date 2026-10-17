//! PostgreSQL persistence for namespace PII reveal grants.

use sagat_application::error::StoreError;
use sagat_application::reveal_grants::{
    GrantWrite, MAX_GRANTS_PER_NAMESPACE, RevealGrant, RevealGrantStore,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{NamespaceId, Permission, Role, Subject, TenantId};
use sqlx::PgConnection;
use time::OffsetDateTime;

use crate::execution::PgRunStore;
use crate::identity::{insert_audit, store_error};
use crate::pool::BeginSafely;

type GrantRow = (String, String, OffsetDateTime);

fn restore_grant(row: GrantRow) -> Result<RevealGrant, StoreError> {
    let (subject, granted_by, granted_at) = row;
    Ok(RevealGrant {
        subject: Subject::parse(&subject).map_err(|_| corrupt("reveal grant subject"))?,
        granted_by: Subject::parse(&granted_by).map_err(|_| corrupt("reveal grant actor"))?,
        granted_at,
    })
}

fn corrupt(field: &'static str) -> StoreError {
    StoreError::Internal {
        detail: format!("invalid {field} in storage"),
    }
}

/// Checks the authenticated subject's effective namespace grant on the caller's current database
/// transaction/connection.
///
/// Disclosure and event stores call this with the same `PgConnection` that reads the payload and
/// appends its audit record. The current authenticated role must still hold
/// `payload:read-internal`; role permissions never imply `payload:reveal-pii`.
pub async fn has_effective_payload_reveal_grant(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    subject: &Subject,
) -> Result<bool, StoreError> {
    let current_role: Option<String> =
        sqlx::query_scalar("SELECT role FROM public.sagat_lock_memberships($1, ARRAY[$2::text])")
            .bind(tenant.as_uuid())
            .bind(subject.as_str())
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?;
    let Some(current_role) = current_role else {
        return Ok(false);
    };
    let current_role = current_role
        .parse::<Role>()
        .map_err(|_| corrupt("membership role"))?;
    if !role_can_use_reveal_grant(current_role) {
        return Ok(false);
    }
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM payload_reveal_grants \
         WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3)",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(subject.as_str())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)
}

fn role_can_use_reveal_grant(role: Role) -> bool {
    role.grants(Permission::PayloadReadInternal)
}

async fn lock_namespace(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<bool, StoreError> {
    sqlx::query_scalar::<_, String>("SELECT environment FROM public.sagat_lock_namespace($1, $2)")
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_optional(conn)
        .await
        .map(|row| row.is_some())
        .map_err(store_error)
}

async fn lock_membership_role(
    conn: &mut PgConnection,
    tenant: TenantId,
    subject: &Subject,
) -> Result<Option<Role>, StoreError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM public.sagat_lock_memberships($1, ARRAY[$2::text])")
            .bind(tenant.as_uuid())
            .bind(subject.as_str())
            .fetch_optional(conn)
            .await
            .map_err(store_error)?;
    role.map(|value| {
        value
            .parse::<Role>()
            .map_err(|_| corrupt("membership role"))
    })
    .transpose()
}

async fn lock_grant_set(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    let lock_key = format!("payload-reveal-grants:{tenant}:{namespace}");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(lock_key)
        .execute(conn)
        .await
        .map(|_| ())
        .map_err(store_error)
}

impl RevealGrantStore for PgRunStore {
    async fn reveal_grants(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<&Subject>,
        limit: u32,
    ) -> Result<Vec<RevealGrant>, StoreError> {
        let rows: Vec<GrantRow> = sqlx::query_as(
            "SELECT subject, granted_by, granted_at FROM payload_reveal_grants \
             WHERE tenant_id = $1 AND namespace_id = $2 \
               AND ($3::text IS NULL OR subject > $3) \
             ORDER BY subject LIMIT $4",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(after.map(Subject::as_str))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.into_iter().map(restore_grant).collect()
    }

    async fn grant_reveal(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &Subject,
        audit: &AuditRecord,
    ) -> Result<GrantWrite, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;

        // The SECURITY DEFINER helpers supplied by feature 017 take authority read locks without
        // granting the runtime role UPDATE on authority tables. The advisory lock then serialises
        // this namespace's grant set, making the cap exact under concurrent administrators.
        let namespace_exists = lock_namespace(&mut tx, tenant, namespace).await?;
        if !namespace_exists {
            return Err(StoreError::MissingReference);
        }

        let role = lock_membership_role(&mut tx, tenant, subject).await?;
        let Some(role) = role else {
            return Ok(GrantWrite::UnknownSubject);
        };
        if !role.grants(Permission::PayloadReadInternal) {
            return Ok(GrantWrite::RoleCannotReveal);
        }
        lock_grant_set(&mut tx, tenant, namespace).await?;

        let existing: Option<GrantRow> = sqlx::query_as(
            "SELECT subject, granted_by, granted_at FROM payload_reveal_grants \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        if let Some(existing) = existing {
            return restore_grant(existing).map(GrantWrite::Existing);
        }

        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM payload_reveal_grants \
             WHERE tenant_id = $1 AND namespace_id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if count >= MAX_GRANTS_PER_NAMESPACE {
            return Ok(GrantWrite::LimitReached);
        }

        let row: GrantRow = sqlx::query_as(
            "INSERT INTO payload_reveal_grants \
             (tenant_id, namespace_id, subject, granted_by, granted_at) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING subject, granted_by, granted_at",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject.as_str())
        .bind(audit.actor().as_str())
        .bind(audit.created_at())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        restore_grant(row).map(GrantWrite::Created)
    }

    async fn revoke_reveal(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &Subject,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let namespace_exists = lock_namespace(&mut tx, tenant, namespace).await?;
        if !namespace_exists {
            return Ok(false);
        }
        // Lock an existing membership so a concurrent cascade cannot interleave with the explicit
        // revocation audit. Missing membership is already equivalent to an absent grant.
        if lock_membership_role(&mut tx, tenant, subject)
            .await?
            .is_none()
        {
            return Ok(false);
        }
        lock_grant_set(&mut tx, tenant, namespace).await?;
        let removed = sqlx::query(
            "DELETE FROM payload_reveal_grants \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject.as_str())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected()
            == 1;
        if !removed {
            return Ok(false);
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::role_can_use_reveal_grant;
    use sagat_domain::identity::Role;

    #[test]
    fn the_current_authenticated_role_must_read_internal_values() {
        assert!(role_can_use_reveal_grant(Role::Admin));
        assert!(role_can_use_reveal_grant(Role::Developer));
        assert!(role_can_use_reveal_grant(Role::Operator));
        assert!(!role_can_use_reveal_grant(Role::Viewer));
        assert!(!role_can_use_reveal_grant(Role::Worker));
        assert!(!role_can_use_reveal_grant(Role::Runner));
    }
}

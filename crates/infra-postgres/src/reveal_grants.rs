//! PostgreSQL persistence for namespace PII reveal grants.

use sagat_application::error::StoreError;
use sagat_application::reveal_grants::{
    GrantRead, GrantRevoke, GrantWrite, MAX_GRANTS_PER_NAMESPACE, RevealGrant, RevealGrantStore,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{NamespaceId, Permission, Role, Subject, TenantId};
use sqlx::{PgConnection, Postgres, Transaction};
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

/// Checks the authenticated subject's effective namespace grant in the caller's database
/// transaction.
///
/// Disclosure and event stores call this with the transaction that reads the payload and appends
/// its audit record. Requiring a `Transaction` keeps the shared revocation fence alive through that
/// commit. The current authenticated role must still hold `payload:read-internal`; role permissions
/// never imply `payload:reveal-pii`.
pub async fn has_effective_payload_reveal_grant(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    subject: &Subject,
) -> Result<bool, StoreError> {
    let conn = &mut **tx;
    let roles = lock_membership_roles(conn, tenant, &[subject]).await?;
    let Some(current_role) = locked_role(&roles, subject) else {
        return Ok(false);
    };
    if !role_can_use_reveal_grant(current_role) {
        return Ok(false);
    }
    // Revocation takes this namespace key exclusively. Holding it shared through the caller's
    // transaction prevents a revoke from committing after this check but before disclosure audit.
    lock_grant_set_shared(conn, tenant, namespace).await?;
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

async fn lock_membership_roles(
    conn: &mut PgConnection,
    tenant: TenantId,
    subjects: &[&Subject],
) -> Result<Vec<(String, Role)>, StoreError> {
    let subjects: Vec<&str> = subjects.iter().map(|subject| subject.as_str()).collect();
    let roles: Vec<(String, String)> =
        sqlx::query_as("SELECT subject, role FROM public.sagat_lock_memberships($1, $2::text[])")
            .bind(tenant.as_uuid())
            .bind(&subjects)
            .fetch_all(conn)
            .await
            .map_err(store_error)?;
    roles
        .into_iter()
        .map(|(subject, role)| {
            role.parse::<Role>()
                .map(|role| (subject, role))
                .map_err(|_| corrupt("membership role"))
        })
        .collect()
}

fn locked_role(roles: &[(String, Role)], subject: &Subject) -> Option<Role> {
    roles
        .iter()
        .find_map(|(stored, role)| (stored == subject.as_str()).then_some(*role))
}

fn actor_can_manage(roles: &[(String, Role)], actor: &Subject) -> bool {
    locked_role(roles, actor).is_some_and(|role| role.grants(Permission::MembershipManage))
}

fn grant_lock_key(tenant: TenantId, namespace: NamespaceId) -> String {
    format!("payload-reveal-grants:{tenant}:{namespace}")
}

async fn lock_grant_set(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    let lock_key = grant_lock_key(tenant, namespace);
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(lock_key)
        .execute(conn)
        .await
        .map(|_| ())
        .map_err(store_error)
}

async fn lock_grant_set_shared(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    let lock_key = grant_lock_key(tenant, namespace);
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
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
        actor: &Subject,
        after: Option<&Subject>,
        limit: u32,
    ) -> Result<GrantRead, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        if !lock_namespace(&mut tx, tenant, namespace).await? {
            return Err(StoreError::MissingReference);
        }
        let roles = lock_membership_roles(&mut tx, tenant, &[actor]).await?;
        if !actor_can_manage(&roles, actor) {
            return Ok(GrantRead::ActorForbidden);
        }
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
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let grants = rows
            .into_iter()
            .map(restore_grant)
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(store_error)?;
        Ok(GrantRead::Authorized(grants))
    }

    async fn grant_reveal(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        actor: &Subject,
        subject: &Subject,
        audit: &AuditRecord,
    ) -> Result<GrantWrite, StoreError> {
        if audit.actor() != actor {
            return Err(corrupt("reveal grant audit actor"));
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;

        // The SECURITY DEFINER helpers supplied by feature 017 take authority read locks without
        // granting the runtime role UPDATE on authority tables. The advisory lock then serialises
        // this namespace's grant set, making the cap exact under concurrent administrators.
        let namespace_exists = lock_namespace(&mut tx, tenant, namespace).await?;
        if !namespace_exists {
            return Err(StoreError::MissingReference);
        }

        let roles = lock_membership_roles(&mut tx, tenant, &[actor, subject]).await?;
        if !actor_can_manage(&roles, actor) {
            return Ok(GrantWrite::ActorForbidden);
        }
        let Some(role) = locked_role(&roles, subject) else {
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
        actor: &Subject,
        subject: &Subject,
        audit: &AuditRecord,
    ) -> Result<GrantRevoke, StoreError> {
        if audit.actor() != actor {
            return Err(corrupt("reveal revoke audit actor"));
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let namespace_exists = lock_namespace(&mut tx, tenant, namespace).await?;
        if !namespace_exists {
            return Ok(GrantRevoke::Absent);
        }
        // The actor's current authority and the target membership are locked in canonical subject
        // order by the feature-017 helper. A concurrent cascade therefore cannot interleave with
        // an explicit revocation audit.
        let roles = lock_membership_roles(&mut tx, tenant, &[actor, subject]).await?;
        if !actor_can_manage(&roles, actor) {
            return Ok(GrantRevoke::ActorForbidden);
        }
        if locked_role(&roles, subject).is_none() {
            return Ok(GrantRevoke::Absent);
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
            return Ok(GrantRevoke::Absent);
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(GrantRevoke::Removed)
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

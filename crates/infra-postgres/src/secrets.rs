//! The secret store in PostgreSQL (FR-009-002, FR-009-003).
//!
//! Only sealed values are stored. Reads that hand a value to a worker happen inside the worker's
//! run transaction ([`RunTx::secret_inputs`](sagat_application::execution::RunTx::secret_inputs)),
//! so the audit record of a resolution commits with the read.

use crate::pool::BeginSafely;
use sagat_application::error::StoreError;
use sagat_application::secrets::{
    NewVersion, SecretAccessLookup, SecretInput, SecretInputs, SecretState, SecretStore,
    StoredSecret, StoredValue,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{NamespaceId, Role, TenantId};
use sagat_domain::secrets::{SealedSecret, SecretMeta, SecretName};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::definitions::PgDefinitionStore;
use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::quotas::{ensure_room_for_secret, lock_secrets};

/// Versions kept per secret: the current one and the few before it, so a rotation can be undone.
const KEPT_VERSIONS: i32 = 5;

fn name_of(row: &PgRow, column: &str) -> Result<SecretName, StoreError> {
    let text: String = col(row, column)?;
    SecretName::parse(&text).map_err(|_| corrupt("secret name"))
}

pub(crate) fn stored_from(row: &PgRow) -> Result<StoredSecret, StoreError> {
    let version: i32 = col(row, "version")?;
    let key_id: Option<String> = col(row, "key_id")?;
    let remote: Option<Uuid> = col(row, "provider_operation")?;
    Ok(StoredSecret {
        meta: SecretMeta {
            id: col(row, "version_id")?,
            tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
            namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
            name: name_of(row, "name")?,
            version: u32::try_from(version).map_err(|_| corrupt("secret version"))?,
            key_id: key_id.clone(),
            created_by: col(row, "created_by")?,
            created_at: col(row, "created_at")?,
        },
        value: if let Some(operation) = remote {
            StoredValue::OpenBao(sagat_application::secret_provider::ProviderReference {
                tenant: TenantId::new(col(row, "tenant_id")?),
                namespace: NamespaceId::new(col(row, "namespace_id")?),
                operation,
            })
        } else {
            StoredValue::Local(SealedSecret {
                key_id: key_id.ok_or_else(|| corrupt("local secret key"))?,
                nonce: col(row, "nonce")?,
                ciphertext: col(row, "ciphertext")?,
            })
        },
    })
}

/// `SELECT` of the current version of secrets, ending in `$tail`. A macro so the result is a string
/// literal: `sqlx` only accepts literal SQL.
macro_rules! select_current {
    ($tail:literal) => {
        concat!(
            "SELECT s.tenant_id, s.namespace_id, s.name, v.id AS version_id, v.version, v.key_id, ",
            "v.nonce, v.ciphertext, v.provider_operation, v.created_by, v.created_at \
             FROM secrets s JOIN secret_versions v \
               ON v.secret_id = s.id AND v.version = s.current_version ",
            $tail
        )
    };
}

async fn states(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    names: &[SecretName],
    subject: &str,
) -> Result<Vec<(SecretName, SecretState)>, StoreError> {
    let texts: Vec<&str> = names.iter().map(SecretName::as_str).collect();
    let rows: Vec<(String, bool)> = sqlx::query_as(
        "SELECT s.name, EXISTS (SELECT 1 FROM secret_access a \
                                WHERE a.secret_id = s.id AND a.subject = $4) \
         FROM secrets s \
         WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.name = ANY($3)",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(&texts)
    .bind(subject)
    .fetch_all(conn)
    .await
    .map_err(store_error)?;
    Ok(names
        .iter()
        .map(|name| {
            let state = rows
                .iter()
                .find(|(found, _)| found == name.as_str())
                .map_or(SecretState::Missing, |(_, granted)| SecretState::Present {
                    granted: *granted,
                });
            (name.clone(), state)
        })
        .collect())
}

impl SecretAccessLookup for PgRunStore {
    async fn secret_states(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        names: &[SecretName],
        subject: &str,
    ) -> Result<Vec<(SecretName, SecretState)>, StoreError> {
        let mut conn = self.pool.acquire().await.map_err(store_error)?;
        states(&mut conn, tenant, namespace, names, subject).await
    }
}

impl SecretAccessLookup for PgDefinitionStore {
    async fn secret_states(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        names: &[SecretName],
        subject: &str,
    ) -> Result<Vec<(SecretName, SecretState)>, StoreError> {
        let pool: &PgPool = self.pool();
        let mut conn = pool.acquire().await.map_err(store_error)?;
        states(&mut conn, tenant, namespace, names, subject).await
    }
}

/// Every stored version of `names`, newest first. For searching an output for values that were
/// handed out, which may since have been replaced.
pub(crate) async fn all_versions(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    names: &[SecretName],
) -> Result<Vec<StoredSecret>, StoreError> {
    let texts: Vec<&str> = names.iter().map(SecretName::as_str).collect();
    let rows = sqlx::query(
        "SELECT s.tenant_id, s.namespace_id, s.name, v.id AS version_id, v.version, v.key_id, \
                v.nonce, v.ciphertext, v.provider_operation, v.created_by, v.created_at \
         FROM secrets s JOIN secret_versions v ON v.secret_id = s.id \
         WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.name = ANY($3) \
         ORDER BY s.name, v.version DESC",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(&texts)
    .fetch_all(conn)
    .await
    .map_err(store_error)?;
    rows.iter().map(stored_from).collect()
}

/// The `limit` newest stored versions of one secret, newest first. Bounded in the query: a secret
/// rotated many times costs the same to read as one rotated twice, which matters on the path that
/// any caller who knows a webhook URL can drive.
pub(crate) async fn newest_versions(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    name: &SecretName,
    limit: u32,
) -> Result<Vec<StoredSecret>, StoreError> {
    let rows = sqlx::query(
        "SELECT s.tenant_id, s.namespace_id, s.name, v.id AS version_id, v.version, v.key_id, \
                v.nonce, v.ciphertext, v.provider_operation, v.created_by, v.created_at \
         FROM secrets s JOIN secret_versions v ON v.secret_id = s.id \
         WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.name = $3 \
         ORDER BY v.version DESC LIMIT $4",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(name.as_str())
    .bind(i64::from(limit))
    .fetch_all(conn)
    .await
    .map_err(store_error)?;
    rows.iter().map(stored_from).collect()
}

/// The first of `names` that lists worker pools and whose pools `subject` has none of in the
/// namespace (G48). Pool grants and a secret's pools are both scoped by tenant, and the grants by
/// namespace, so a pool name means nothing outside the namespace that granted it.
pub(crate) async fn pool_refusal(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    names: &[SecretName],
    subject: &str,
    lock: bool,
) -> Result<Option<SecretName>, StoreError> {
    let texts: Vec<&str> = names.iter().map(SecretName::as_str).collect();
    if lock {
        // The rows the answer rests on, locked until the caller's transaction ends. Changing a
        // secret's pools takes the secret's row for update, and taking a grant away deletes its
        // row, so either waits for this transaction (and is seen by the next one) instead of
        // committing between this check and the read of the value. A grant that is *added* is not
        // waited for: it can only widen who is entitled. The order, secret rows and then grant
        // rows, is the order nothing else takes in reverse.
        sqlx::query(
            "SELECT id FROM secrets \
             WHERE tenant_id = $1 AND namespace_id = $2 AND name = ANY($3) ORDER BY id FOR SHARE",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(&texts)
        .fetch_all(&mut *conn)
        .await
        .map_err(store_error)?;
        // The grants are not row-locked: a row lock needs UPDATE, which the runtime role does not
        // hold. Replacing or revoking a binding takes this advisory lock exclusively before it
        // touches the grants, so holding it shared has the same effect (G48, G294).
        sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
            .bind(format!("binding:{tenant}:{namespace}:{subject}"))
            .execute(&mut *conn)
            .await
            .map_err(store_error)?;
    }
    let refused: Vec<String> = sqlx::query_scalar(
        "SELECT s.name FROM secrets s \
         WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.name = ANY($3) \
           AND EXISTS (SELECT 1 FROM secret_pools p \
                       WHERE p.tenant_id = s.tenant_id AND p.secret_id = s.id) \
           AND NOT EXISTS (SELECT 1 FROM secret_pools p \
                           JOIN worker_binding_pools g \
                             ON g.tenant_id = p.tenant_id AND g.pool = p.pool \
                            AND g.namespace_id = s.namespace_id AND g.subject = $4 \
                           WHERE p.tenant_id = s.tenant_id AND p.secret_id = s.id)",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(&texts)
    .bind(subject)
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(names
        .iter()
        .find(|name| refused.iter().any(|r| r == name.as_str()))
        .cloned())
}

/// Replaces the pools of a secret on the caller's transaction (which holds the secret's row lock).
pub(crate) async fn replace_pools(
    conn: &mut PgConnection,
    tenant: TenantId,
    secret: Uuid,
    pools: &[String],
    changed_by: &str,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM secret_pools WHERE tenant_id = $1 AND secret_id = $2")
        .bind(tenant.as_uuid())
        .bind(secret)
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    if !pools.is_empty() {
        sqlx::query(
            "INSERT INTO secret_pools (tenant_id, secret_id, pool, granted_by) \
             SELECT $1, $2, p, $4 FROM unnest($3::text[]) AS t(p)",
        )
        .bind(tenant.as_uuid())
        .bind(secret)
        .bind(pools)
        .bind(changed_by)
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

/// What a step's declared secrets resolve to, read on the caller's transaction.
pub(crate) async fn inputs(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    version: Uuid,
    names: &[SecretName],
) -> Result<SecretInputs, StoreError> {
    let publisher: String = sqlx::query_scalar(
        "SELECT published_by FROM workflow_versions WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(version)
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or_else(|| corrupt("workflow version reference"))?;
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE tenant_id = $1 AND subject = $2")
            .bind(tenant.as_uuid())
            .bind(&publisher)
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?;
    let publisher_role = role
        .as_deref()
        .map(str::parse::<Role>)
        .transpose()
        .map_err(|_| corrupt("membership role"))?;
    let texts: Vec<&str> = names.iter().map(SecretName::as_str).collect();
    let rows = sqlx::query(select_current!(
        "WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.name = ANY($3)"
    ))
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(&texts)
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    let found = rows
        .iter()
        .map(stored_from)
        .collect::<Result<Vec<_>, _>>()?;
    let granted: Vec<String> = sqlx::query_scalar(
        "SELECT s.name FROM secrets s JOIN secret_access a ON a.secret_id = s.id \
         WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.name = ANY($3) AND a.subject = $4",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(&texts)
    .bind(&publisher)
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    let items = names
        .iter()
        .map(|name| SecretInput {
            name: name.clone(),
            stored: found.iter().find(|s| &s.meta.name == name).cloned(),
            granted: granted.iter().any(|g| g == name.as_str()),
        })
        .collect();
    Ok(SecretInputs {
        publisher,
        publisher_role,
        items,
    })
}

impl SecretStore for PgRunStore {
    async fn put_version(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        version: &NewVersion,
        secrets_limit: u64,
        audit: &AuditRecord,
    ) -> Result<SecretMeta, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // The lock comes first, so two creators of one name, or a creator and a rotation, are
        // serialised before either looks at what exists.
        lock_secrets(&mut tx, tenant, namespace).await?;
        let managed: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM secrets WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 AND authority='openbao') OR EXISTS (SELECT 1 FROM secret_provider_operations WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 AND state='pending')")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(name.as_str())
            .fetch_one(&mut *tx).await.map_err(store_error)?;
        if managed {
            return Err(StoreError::StateConflict(
                "secret is managed by OpenBao or has a pending provider operation",
            ));
        }

        let existing: Option<(Uuid, i32)> = sqlx::query_as(
            "SELECT id, current_version FROM secrets \
             WHERE tenant_id = $1 AND namespace_id = $2 AND name = $3 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(name.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let (secret_id, number) = if let Some((id, current)) = existing {
            (id, current + 1)
        } else {
            ensure_room_for_secret(&mut tx, tenant, namespace, secrets_limit).await?;
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO secrets (id, tenant_id, namespace_id, name, current_version, created_by) \
                 VALUES ($1, $2, $3, $4, 1, $5)",
            )
            .bind(id)
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(name.as_str())
            .bind(&version.created_by)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            (id, 1)
        };
        let created_at: OffsetDateTime = sqlx::query_scalar(
            "INSERT INTO secret_versions \
             (id, tenant_id, secret_id, version, key_id, nonce, ciphertext, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING created_at",
        )
        .bind(version.row_id)
        .bind(tenant.as_uuid())
        .bind(secret_id)
        .bind(number)
        .bind(&version.sealed.key_id)
        .bind(&version.sealed.nonce)
        .bind(&version.sealed.ciphertext)
        .bind(&version.created_by)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE secrets SET current_version = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(secret_id)
        .bind(number)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query("DELETE FROM secret_versions WHERE secret_id = $1 AND version <= $2")
            .bind(secret_id)
            .bind(number - KEPT_VERSIONS)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        if let Some(pools) = &version.pools {
            replace_pools(&mut tx, tenant, secret_id, pools, &version.created_by).await?;
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(SecretMeta {
            id: version.row_id,
            tenant_id: tenant,
            namespace_id: namespace,
            name: name.clone(),
            version: u32::try_from(number).map_err(|_| corrupt("secret version"))?,
            key_id: Some(version.sealed.key_id.clone()),
            created_by: version.created_by.clone(),
            created_at,
        })
    }

    async fn list_secrets(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Vec<SecretMeta>, StoreError> {
        sqlx::query(select_current!(
            "WHERE s.tenant_id = $1 AND s.namespace_id = $2 ORDER BY s.name"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| stored_from(row).map(|stored| stored.meta))
        .collect()
    }

    async fn delete_secret(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_secrets(&mut tx, tenant, namespace).await?;
        let abandoned = sqlx::query("UPDATE secret_provider_operations SET state='abandoned' WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 AND state='pending'")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(name.as_str()).execute(&mut *tx).await.map_err(store_error)?.rows_affected() > 0;
        let removed = sqlx::query(
            "DELETE FROM secrets WHERE tenant_id = $1 AND namespace_id = $2 AND name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(name.as_str())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected()
            > 0;
        if removed || abandoned {
            insert_audit(&mut tx, audit).await?;
            tx.commit().await.map_err(store_error)?;
        }
        Ok(removed || abandoned)
    }

    async fn grant_access(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        subject: &str,
        granted_by: &str,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM secrets WHERE tenant_id = $1 AND namespace_id = $2 AND name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(name.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(id) = id else {
            return Ok(false);
        };
        sqlx::query(
            "INSERT INTO secret_access (tenant_id, secret_id, subject, granted_by) \
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(subject)
        .bind(granted_by)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(true)
    }

    async fn revoke_access(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        subject: &str,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let removed = sqlx::query(
            "DELETE FROM secret_access a USING secrets s \
             WHERE a.secret_id = s.id AND s.tenant_id = $1 AND s.namespace_id = $2 \
               AND s.name = $3 AND a.subject = $4",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(name.as_str())
        .bind(subject)
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

    async fn names_usable_by(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
    ) -> Result<Vec<String>, StoreError> {
        sqlx::query_scalar(
            "SELECT s.name FROM secrets s JOIN secret_access a ON a.secret_id = s.id \
             WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND a.subject = $3 ORDER BY s.name",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(subject)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn access_list(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
    ) -> Result<Option<Vec<String>>, StoreError> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM secrets WHERE tenant_id = $1 AND namespace_id = $2 AND name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(name.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        let Some(id) = id else {
            return Ok(None);
        };
        let subjects = sqlx::query_scalar(
            "SELECT subject FROM secret_access WHERE secret_id = $1 ORDER BY subject",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(Some(subjects))
    }

    async fn secret_pools(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
    ) -> Result<Option<Vec<String>>, StoreError> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM secrets WHERE tenant_id = $1 AND namespace_id = $2 AND name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(name.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        let Some(id) = id else {
            return Ok(None);
        };
        sqlx::query_scalar(
            "SELECT pool FROM secret_pools WHERE tenant_id = $1 AND secret_id = $2 ORDER BY pool",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .map(Some)
        .map_err(store_error)
    }

    async fn restricted_secrets(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Vec<(String, Vec<String>)>, StoreError> {
        sqlx::query_as(
            "SELECT s.name, array_agg(p.pool ORDER BY p.pool) FROM secrets s \
             JOIN secret_pools p ON p.tenant_id = s.tenant_id AND p.secret_id = s.id \
             WHERE s.tenant_id = $1 AND s.namespace_id = $2 GROUP BY s.name",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn replace_secret_pools(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        pools: &[String],
        changed_by: &str,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // The row lock orders two replacements of one secret's list and a rotation or deletion.
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM secrets WHERE tenant_id = $1 AND namespace_id = $2 AND name = $3 \
             FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(name.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(id) = id else {
            return Ok(false);
        };
        replace_pools(&mut tx, tenant, id, pools, changed_by).await?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(true)
    }

    async fn versions_not_sealed_under(
        &self,
        active_key: &str,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<StoredSecret>, StoreError> {
        sqlx::query(
            "SELECT s.tenant_id, s.namespace_id, s.name, v.id AS version_id, v.version, v.key_id, \
                    v.nonce, v.ciphertext, v.provider_operation, v.created_by, v.created_at \
             FROM secret_versions v JOIN secrets s ON s.id = v.secret_id \
             WHERE v.key_id <> $1 AND ($3::uuid IS NULL OR v.id > $3::uuid) \
             ORDER BY v.id LIMIT $2",
        )
        .bind(active_key)
        .bind(i64::from(limit))
        .bind(after)
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(stored_from)
        .collect()
    }

    async fn replace_sealed(
        &self,
        row_id: Uuid,
        expected_key: &str,
        sealed: &SealedSecret,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let replaced = sqlx::query(
            "UPDATE secret_versions SET key_id = $3, nonce = $4, ciphertext = $5 \
             WHERE id = $1 AND key_id = $2",
        )
        .bind(row_id)
        .bind(expected_key)
        .bind(&sealed.key_id)
        .bind(&sealed.nonce)
        .bind(&sealed.ciphertext)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected()
            == 1;
        if replaced {
            insert_audit(&mut tx, audit).await?;
            tx.commit().await.map_err(store_error)?;
        }
        Ok(replaced)
    }
}

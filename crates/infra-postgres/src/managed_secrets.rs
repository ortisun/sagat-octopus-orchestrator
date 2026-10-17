//! Durable OpenBao intents. No provider I/O occurs while a PostgreSQL transaction is open.
use crate::{
    execution::PgRunStore,
    identity::{col, corrupt, insert_audit, store_error},
    pool::BeginSafely,
    quotas::{ensure_room_for_secret, lock_secrets},
};
use sagat_application::{
    error::StoreError,
    managed_secrets::{ManagedSecretStore, ProviderIntent},
    secret_provider::ProviderReference,
    secrets::{SecretView, StoredSecret},
};
use sagat_domain::{
    audit::AuditRecord,
    identity::{NamespaceId, Principal, TenantId},
    secrets::SecretName,
};
use sqlx::{PgConnection, postgres::PgRow};
use uuid::Uuid;

async fn current_admin(
    conn: &mut PgConnection,
    tenant: TenantId,
    actor: &str,
) -> Result<(), StoreError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM public.sagat_lock_memberships($1, ARRAY[$2::text])")
            .bind(tenant.as_uuid())
            .bind(actor)
            .fetch_optional(conn)
            .await
            .map_err(store_error)?;
    if role.as_deref() == Some("admin") {
        Ok(())
    } else {
        Err(StoreError::MissingReference)
    }
}

fn intent_from(row: &PgRow) -> Result<ProviderIntent, StoreError> {
    Ok(ProviderIntent {
        reference: ProviderReference {
            tenant: TenantId::new(col(row, "tenant_id")?),
            namespace: NamespaceId::new(col(row, "namespace_id")?),
            operation: col(row, "id")?,
        },
        secret_id: col(row, "secret_id")?,
        name: SecretName::parse(&col::<String>(row, "name")?)
            .map_err(|_| corrupt("secret name"))?,
        actor: col(row, "actor")?,
        request_id: col(row, "request_id")?,
        expected_version: u32::try_from(col::<i32>(row, "expected_version")?)
            .map_err(|_| corrupt("secret version"))?,
        binding: col(row, "binding")?,
        value_bytes: u32::try_from(col::<i64>(row, "value_bytes")?)
            .map_err(|_| corrupt("secret size"))?,
        activated: col::<String>(row, "state")? == "active",
        source_version: col(row, "source_version")?,
    })
}

async fn load(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    operation: Uuid,
    actor: &str,
) -> Result<ProviderIntent, StoreError> {
    let row = sqlx::query("SELECT * FROM secret_provider_operations WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 AND actor=$4 AND (state='active' OR (state='pending' AND created_at > clock_timestamp() - interval '24 hours')) FOR UPDATE")
        .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(operation).bind(actor)
        .fetch_optional(conn).await.map_err(store_error)?.ok_or(StoreError::MissingReference)?;
    intent_from(&row)
}

impl ManagedSecretStore for PgRunStore {
    async fn provider_request(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        request: Uuid,
    ) -> Result<Option<ProviderIntent>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        current_admin(&mut tx, principal.tenant_id(), principal.subject().as_str()).await?;
        let row = sqlx::query("SELECT * FROM secret_provider_operations WHERE tenant_id=$1 AND namespace_id=$2 AND actor=$3 AND request_id=$4")
            .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(principal.subject().as_str()).bind(request)
            .fetch_optional(&mut *tx).await.map_err(store_error)?;
        let result = row.as_ref().map(intent_from).transpose()?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    async fn legacy_provider_source(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &SecretName,
        expected_version: u32,
    ) -> Result<StoredSecret, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        current_admin(&mut tx, principal.tenant_id(), principal.subject().as_str()).await?;
        let row = sqlx::query("SELECT s.tenant_id,s.namespace_id,s.name,v.id AS version_id,v.version,v.key_id,v.nonce,v.ciphertext,v.provider_operation,v.created_by,v.created_at FROM secrets s JOIN secret_versions v ON v.tenant_id=s.tenant_id AND v.secret_id=s.id AND v.version=s.current_version WHERE s.tenant_id=$1 AND s.namespace_id=$2 AND s.name=$3 AND s.current_version=$4 AND s.authority='local'")
            .bind(principal.tenant_id().as_uuid()).bind(namespace.as_uuid()).bind(name.as_str()).bind(i64::from(expected_version))
            .fetch_optional(&mut *tx).await.map_err(store_error)?.ok_or(StoreError::MissingReference)?;
        let result = crate::secrets::stored_from(&row)?;
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    async fn prepare_provider_intent(
        &self,
        intent: &ProviderIntent,
        secret_limit: u64,
        audit: &AuditRecord,
    ) -> Result<ProviderIntent, StoreError> {
        let tenant = intent.reference.tenant;
        let namespace = intent.reference.namespace;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_secrets(&mut tx, tenant, namespace).await?;
        current_admin(&mut tx, tenant, &intent.actor).await?;
        let replay = sqlx::query("SELECT *, (created_at <= clock_timestamp() - interval '24 hours') AS expired FROM secret_provider_operations WHERE tenant_id=$1 AND namespace_id=$2 AND actor=$3 AND request_id=$4 FOR UPDATE")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(&intent.actor).bind(intent.request_id)
            .fetch_optional(&mut *tx).await.map_err(store_error)?;
        if let Some(row) = replay {
            let old = intent_from(&row)?;
            if old.binding != intent.binding {
                return Err(StoreError::IdempotencyKeyReused);
            }
            if col::<String>(&row, "state")? == "abandoned"
                || (!old.activated && col::<bool>(&row, "expired")?)
            {
                return Err(StoreError::MissingReference);
            }
            tx.commit().await.map_err(store_error)?;
            return Ok(old);
        }
        let current: Option<(Uuid,i32,String)> = sqlx::query_as("SELECT id,current_version,authority FROM secrets WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 FOR UPDATE")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(intent.name.as_str()).fetch_optional(&mut *tx).await.map_err(store_error)?;
        let version = current.as_ref().map_or(0, |(_, v, _)| *v as u32);
        if version != intent.expected_version {
            return Err(StoreError::RevisionConflict {
                current: u64::from(version),
            });
        }
        if let Some(source) = intent.source_version {
            let source_valid: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM secrets s JOIN secret_versions v ON v.secret_id=s.id AND v.tenant_id=s.tenant_id AND v.version=s.current_version WHERE s.tenant_id=$1 AND s.namespace_id=$2 AND s.name=$3 AND s.authority='local' AND v.id=$4)")
                .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(intent.name.as_str()).bind(source)
                .fetch_one(&mut *tx).await.map_err(store_error)?;
            if !source_valid {
                return Err(StoreError::StateConflict("legacy migration source changed"));
            }
        } else if current
            .as_ref()
            .is_some_and(|(_, _, authority)| authority != "openbao")
        {
            return Err(StoreError::StateConflict(
                "legacy secret requires explicit migration",
            ));
        }
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM secret_provider_operations WHERE tenant_id=$1 AND namespace_id=$2 AND state='pending'")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).fetch_one(&mut *tx).await.map_err(store_error)?;
        if pending >= 32 {
            return Err(StoreError::QuotaExceeded {
                what: "pending secret operations",
            });
        }
        let same_name: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM secret_provider_operations WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 AND state='pending')")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(intent.name.as_str()).fetch_one(&mut *tx).await.map_err(store_error)?;
        if same_name {
            return Err(StoreError::StateConflict(
                "secret has a pending provider operation",
            ));
        }
        let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM secret_provider_operations WHERE tenant_id=$1 AND namespace_id=$2")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).fetch_one(&mut *tx).await.map_err(store_error)?;
        if retained >= 1024 {
            return Err(StoreError::QuotaExceeded {
                what: "retained secret operations",
            });
        }
        if current.is_none() {
            // Pending creations also reserve a catalog slot, so an outage cannot bypass quotas.
            ensure_room_for_secret(
                &mut tx,
                tenant,
                namespace,
                secret_limit.saturating_sub(pending as u64),
            )
            .await?;
        }
        let secret_id = current.map_or(intent.secret_id, |(id, _, _)| id);
        sqlx::query("INSERT INTO secret_provider_operations (id,tenant_id,namespace_id,secret_id,name,actor,request_id,expected_version,binding,value_bytes,source_version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(intent.reference.operation).bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(secret_id)
            .bind(intent.name.as_str()).bind(&intent.actor).bind(intent.request_id).bind(intent.expected_version as i32)
            .bind(&intent.binding).bind(i64::from(intent.value_bytes)).bind(intent.source_version).execute(&mut *tx).await.map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(ProviderIntent {
            secret_id,
            ..intent.clone()
        })
    }

    async fn provider_intent(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        operation: Uuid,
    ) -> Result<ProviderIntent, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        current_admin(&mut tx, principal.tenant_id(), principal.subject().as_str()).await?;
        let intent = load(
            &mut tx,
            principal.tenant_id(),
            namespace,
            operation,
            principal.subject().as_str(),
        )
        .await?;
        tx.commit().await.map_err(store_error)?;
        Ok(intent)
    }

    async fn activate_provider_intent(
        &self,
        input: &ProviderIntent,
        secret_limit: u64,
        audit: &AuditRecord,
    ) -> Result<SecretView, StoreError> {
        let tenant = input.reference.tenant;
        let namespace = input.reference.namespace;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_secrets(&mut tx, tenant, namespace).await?;
        current_admin(&mut tx, tenant, &input.actor).await?;
        let intent = load(
            &mut tx,
            tenant,
            namespace,
            input.reference.operation,
            &input.actor,
        )
        .await?;
        let number = intent.expected_version + 1;
        if !intent.activated {
            let current: Option<(Uuid,i32,String)> = sqlx::query_as("SELECT id,current_version,authority FROM secrets WHERE tenant_id=$1 AND namespace_id=$2 AND name=$3 FOR UPDATE")
                .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(intent.name.as_str()).fetch_optional(&mut *tx).await.map_err(store_error)?;
            if intent.expected_version == 0 && current.is_none() {
                ensure_room_for_secret(&mut tx, tenant, namespace, secret_limit).await?;
                sqlx::query("INSERT INTO secrets (id,tenant_id,namespace_id,name,current_version,created_by,authority) VALUES ($1,$2,$3,$4,1,$5,'openbao')")
                    .bind(intent.secret_id).bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(intent.name.as_str()).bind(&intent.actor)
                    .execute(&mut *tx).await.map_err(store_error)?;
            } else if !current.as_ref().is_some_and(|(id, version, authority)| {
                *id == intent.secret_id
                    && *version as u32 == intent.expected_version
                    && (authority == "openbao"
                        || (intent.source_version.is_some() && authority == "local"))
            }) {
                return Err(StoreError::RevisionConflict {
                    current: current.map_or(0, |(_, version, _)| version as u64),
                });
            }
            sqlx::query("INSERT INTO secret_versions (id,tenant_id,secret_id,version,provider_operation,created_by) VALUES ($1,$2,$3,$4,$1,$5)")
                .bind(intent.reference.operation).bind(tenant.as_uuid()).bind(intent.secret_id).bind(number as i32).bind(&intent.actor)
                .execute(&mut *tx).await.map_err(store_error)?;
            sqlx::query("UPDATE secrets SET authority='openbao',current_version=$3,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
                .bind(tenant.as_uuid()).bind(intent.secret_id).bind(number as i32).execute(&mut *tx).await.map_err(store_error)?;
            sqlx::query("UPDATE secret_provider_operations SET state='active',activated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
                .bind(tenant.as_uuid()).bind(intent.reference.operation).execute(&mut *tx).await.map_err(store_error)?;
            sqlx::query(
                "DELETE FROM secret_versions WHERE tenant_id=$1 AND secret_id=$2 AND version <= $3",
            )
            .bind(tenant.as_uuid())
            .bind(intent.secret_id)
            .bind(number as i32 - 5)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            insert_audit(&mut tx, audit).await?;
        }
        // Deletion or pruning makes an old receipt unavailable; it cannot resurrect authority.
        let updated_at = sqlx::query_scalar("SELECT v.created_at FROM secret_versions v JOIN secrets s ON s.id=v.secret_id AND s.tenant_id=v.tenant_id WHERE s.tenant_id=$1 AND s.namespace_id=$2 AND s.id=$3 AND v.provider_operation=$4")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(intent.secret_id).bind(intent.reference.operation)
            .fetch_optional(&mut *tx).await.map_err(store_error)?.ok_or(StoreError::MissingReference)?;
        tx.commit().await.map_err(store_error)?;
        Ok(SecretView {
            name: intent.name.to_string(),
            version: number,
            key_id: None,
            provider: "openbao",
            created_by: intent.actor,
            updated_at,
        })
    }
}

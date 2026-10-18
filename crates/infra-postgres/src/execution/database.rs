//! Managed executor admission and immutable per-attempt credential evidence. No provider I/O.
use super::*;
use sagat_application::{
    database::DatabaseInput,
    secrets::{StoredSecret, StoredValue},
};
use sagat_domain::secrets::SecretMeta;

impl PgRunTx {
    pub(super) async fn database_role(&mut self, subject: &str) -> Result<bool, StoreError> {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM public.sagat_lock_memberships($1, ARRAY[$2::text])",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(subject)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(role.as_deref() == Some("database-runner"))
    }

    pub(super) async fn database_owner(
        &mut self,
        node: &NodeId,
        subject: &str,
    ) -> Result<bool, StoreError> {
        // Binding mutations lock this key before the worker session row. Follow that order
        // before retaining the delivery grant through capability disclosure.
        sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
            .bind(format!(
                "binding:{}:{}:{subject}",
                self.info.tenant_id, self.info.namespace_id
            ))
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        // Session before catalog/membership locks, as on claiming. The current lease token is
        // independently checked by WorkerService; this also binds the token to its machine owner.
        let session: Option<OffsetDateTime> = sqlx::query_scalar(
            "SELECT w.expires_at FROM steps s JOIN worker_sessions w ON w.tenant_id=s.tenant_id AND w.id::text=s.leased_by
             WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node=$3 AND cardinality(s.instance_path_nodes)=0 AND cardinality(s.instance_path_indices)=0 AND w.namespace_id=$4
               AND w.subject=$5 AND w.accepted_protocol_minor>=6
             FOR SHARE OF w")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.run_id.as_uuid()).bind(node.as_str())
            .bind(self.info.namespace_id.as_uuid()).bind(subject)
            .fetch_optional(&mut *self.tx).await.map_err(store_error)?;
        let Some(expires_at) = session else {
            return Ok(false);
        };
        // An unchanged-row lock wait can outlive expiry without rechecking a WHERE predicate.
        // Observe time after retaining this session guard on the owning run transaction.
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        if expires_at <= now {
            return Ok(false);
        }
        // Same lock order as catalog mutations and current publisher/initiator checks.
        crate::quotas::lock_secrets(&mut self.tx, self.info.tenant_id, self.info.namespace_id)
            .await?;
        self.database_role(subject).await
    }

    pub(super) async fn database_admission(
        &mut self,
        node: &NodeId,
        subject: &str,
    ) -> Result<bool, StoreError> {
        // Serialize all managed lease admissions in a tenant before counting them. Releases can
        // only reduce the count. This lock is retained through the READY -> LEASED commit.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("database-concurrency:{}", self.info.tenant_id))
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        crate::quotas::lock_secrets(&mut self.tx, self.info.tenant_id, self.info.namespace_id)
            .await?;
        if !self.database_role(subject).await? {
            return Ok(false);
        }
        crate::database::admit_node(&mut self.tx, &self.info, &self.def, node).await?;
        let connection = self.database_connection(node).await?;
        if crate::secrets::pool_refusal(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            std::slice::from_ref(connection.config.secret()),
            subject,
            false,
        )
        .await?
        .is_some()
        {
            return Ok(false);
        }
        let (tenant, namespace, pool): (i64,i64,i64) = sqlx::query_as(
            "SELECT count(*), count(*) FILTER (WHERE b.namespace_id=$2),
                    count(*) FILTER (WHERE b.namespace_id=$2 AND b.connection_id=$3)
             FROM steps s JOIN run_database_connections b ON b.tenant_id=s.tenant_id AND b.run_id=s.run_id AND b.node_id=s.node
             WHERE s.tenant_id=$1 AND cardinality(s.instance_path_nodes)=0 AND cardinality(s.instance_path_indices)=0 AND s.status IN ('LEASED','RUNNING')")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(connection.id.as_uuid())
            .fetch_one(&mut *self.tx).await.map_err(store_error)?;
        let requested = self
            .def
            .node(node)
            .and_then(Node::database_operation)
            .ok_or(StoreError::DatabaseUnavailable)?
            .limits;
        let policy = crate::database::policy::read(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
        )
        .await?;
        let effective = requested
            .intersect(connection.config.limits())
            .intersect(policy.limits);
        Ok(tenant < i64::from(policy.tenant_concurrency)
            && namespace < i64::from(policy.namespace_concurrency)
            && pool < i64::from(effective.pool_size))
    }

    pub(super) async fn database_report_owner(
        &mut self,
        node: &NodeId,
        fence: sagat_domain::execution::Fence,
        subject: &str,
    ) -> Result<bool, StoreError> {
        crate::quotas::lock_secrets(&mut self.tx, self.info.tenant_id, self.info.namespace_id)
            .await?;
        if !self.database_role(subject).await? {
            return Ok(false);
        }
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM run_events WHERE tenant_id=$1 AND run_id=$2
             AND node=$3 AND COALESCE(cardinality(instance_path_nodes),0)=0
             AND COALESCE(cardinality(instance_path_indices),0)=0
             AND event_type='step.leased' AND actor=$4
             AND detail->>'attempt'=$5 AND detail->>'generation'=$6)",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .bind(format!("worker:{subject}"))
        .bind(fence.attempt.to_string())
        .bind(fence.generation.to_string())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn database_connection(
        &mut self,
        node: &NodeId,
    ) -> Result<sagat_application::database::ConnectionRecord, StoreError> {
        let row = sqlx::query(
            "SELECT c.id,c.tenant_id,c.namespace_id,c.name,c.status,c.created_at,c.updated_at,r.revision,r.config
             FROM run_database_connections b
             JOIN database_connections c ON c.tenant_id=b.tenant_id AND c.namespace_id=b.namespace_id AND c.id=b.connection_id
             JOIN database_connection_revisions r ON r.tenant_id=b.tenant_id AND r.namespace_id=b.namespace_id AND r.connection_id=b.connection_id AND r.revision=b.revision
             WHERE b.tenant_id=$1 AND b.namespace_id=$2 AND b.run_id=$3 AND b.node_id=$4")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(self.info.run_id.as_uuid()).bind(node.as_str())
            .fetch_optional(&mut *self.tx).await.map_err(store_error)?.ok_or(StoreError::DatabaseUnavailable)?;
        crate::database::from_row(&row)
    }

    pub(super) async fn database_snapshot(
        &mut self,
        node: &NodeId,
        subject: &str,
    ) -> Result<DatabaseInput, StoreError> {
        if !self.database_owner(node, subject).await? {
            return Err(StoreError::DatabaseUnavailable);
        }
        crate::database::admit_node(&mut self.tx, &self.info, &self.def, node).await?;
        let connection = self.database_connection(node).await?;
        if crate::secrets::pool_refusal(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            std::slice::from_ref(connection.config.secret()),
            subject,
            true,
        )
        .await?
        .is_some()
        {
            return Err(StoreError::DatabaseUnavailable);
        }
        let mut operation = self
            .def
            .node(node)
            .and_then(Node::database_operation)
            .ok_or(StoreError::DatabaseUnavailable)?
            .clone();
        let policy = crate::database::policy::read(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
        )
        .await?;
        operation.limits = operation
            .limits
            .intersect(connection.config.limits())
            .intersect(policy.limits);
        operation
            .validate()
            .map_err(|_| StoreError::DatabaseLimit)?;
        let row = sqlx::query(
            "SELECT s.id AS secret_id,s.tenant_id,s.namespace_id,s.name,v.id AS version_id,v.version,v.key_id,v.nonce,v.ciphertext,v.provider_operation,v.created_by,v.created_at
             FROM secrets s JOIN secret_versions v ON v.tenant_id=s.tenant_id AND v.secret_id=s.id AND v.version=s.current_version
             WHERE s.tenant_id=$1 AND s.namespace_id=$2 AND s.name=$3 AND s.authority='openbao'")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(connection.config.secret().as_str())
            .fetch_optional(&mut *self.tx).await.map_err(store_error)?.ok_or(StoreError::DatabaseUnavailable)?;
        let current = crate::secrets::stored_from(&row)?;
        let current_secret_version = current.meta.id;
        let secret = self.database_pinned_secret(node).await?.unwrap_or(current);
        // Reusing a logical name after deletion creates new authority. An old attempt must not
        // regain its previous credential merely because a different secret now has that name.
        let same_authority: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM secret_provider_operations WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 AND secret_id=$4)")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(secret.meta.id)
            .bind(col::<Uuid>(&row,"secret_id")?).fetch_one(&mut *self.tx).await.map_err(store_error)?;
        if !same_authority {
            return Err(StoreError::DatabaseUnavailable);
        }
        if secret.meta.name != *connection.config.secret() {
            return Err(StoreError::DatabaseUnavailable);
        }
        Ok(DatabaseInput {
            connection,
            operation,
            secret,
            current_secret_version,
        })
    }

    pub(super) async fn database_pin(
        &mut self,
        node: &NodeId,
        secret: &StoredSecret,
    ) -> Result<(), StoreError> {
        let StoredValue::OpenBao(reference) = &secret.value else {
            return Err(StoreError::DatabaseUnavailable);
        };
        if reference.tenant != self.info.tenant_id
            || reference.namespace != self.info.namespace_id
            || reference.operation != secret.meta.id
        {
            return Err(StoreError::DatabaseUnavailable);
        }
        let step = self
            .state
            .step(node)
            .ok_or(StoreError::DatabaseUnavailable)?;
        sqlx::query(
            "INSERT INTO step_database_credentials(tenant_id,namespace_id,run_id,node_id,attempt,generation,provider_operation,secret_name,secret_version)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT DO NOTHING")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(self.info.run_id.as_uuid()).bind(node.as_str())
            .bind(i64::from(step.attempt())).bind(i64::try_from(step.generation()).map_err(|_| corrupt("step generation"))?)
            .bind(reference.operation).bind(secret.meta.name.as_str()).bind(i64::from(secret.meta.version))
            .execute(&mut *self.tx).await.map_err(store_error)?;
        let pinned = self
            .database_pinned_secret(node)
            .await?
            .ok_or(StoreError::DatabaseUnavailable)?;
        if pinned.meta.id != secret.meta.id || pinned.meta.version != secret.meta.version {
            return Err(StoreError::DatabaseUnavailable);
        }
        Ok(())
    }

    pub(super) async fn database_output_limits(
        &mut self,
        node: &NodeId,
    ) -> Result<sagat_domain::database::DatabaseLimits, StoreError> {
        let connection = self.database_connection(node).await?;
        let operation = self
            .def
            .node(node)
            .and_then(Node::database_operation)
            .ok_or(StoreError::DatabaseUnavailable)?;
        crate::quotas::lock_secrets(&mut self.tx, self.info.tenant_id, self.info.namespace_id)
            .await?;
        let policy = crate::database::policy::read(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
        )
        .await?;
        Ok(operation
            .limits
            .intersect(connection.config.limits())
            .intersect(policy.limits))
    }

    pub(super) async fn database_pinned_secret(
        &mut self,
        node: &NodeId,
    ) -> Result<Option<StoredSecret>, StoreError> {
        let step = self
            .state
            .step(node)
            .ok_or(StoreError::DatabaseUnavailable)?;
        let row = sqlx::query(
            "SELECT p.provider_operation,p.secret_name,p.secret_version,o.actor,o.created_at
             FROM step_database_credentials p JOIN secret_provider_operations o
               ON o.tenant_id=p.tenant_id AND o.namespace_id=p.namespace_id AND o.id=p.provider_operation
             WHERE p.tenant_id=$1 AND p.namespace_id=$2 AND p.run_id=$3 AND p.node_id=$4 AND p.attempt=$5 AND p.generation=$6")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(self.info.run_id.as_uuid()).bind(node.as_str())
            .bind(i64::from(step.attempt())).bind(i64::try_from(step.generation()).map_err(|_| corrupt("step generation"))?)
            .fetch_optional(&mut *self.tx).await.map_err(store_error)?;
        row.map(|row| {
            let operation = col(&row, "provider_operation")?;
            Ok(StoredSecret {
                meta: SecretMeta {
                    id: operation,
                    tenant_id: self.info.tenant_id,
                    namespace_id: self.info.namespace_id,
                    name: SecretName::parse(&col::<String>(&row, "secret_name")?)
                        .map_err(|_| corrupt("secret name"))?,
                    version: u32::try_from(col::<i32>(&row, "secret_version")?)
                        .map_err(|_| corrupt("secret version"))?,
                    key_id: None,
                    created_by: col(&row, "actor")?,
                    created_at: col(&row, "created_at")?,
                },
                value: StoredValue::OpenBao(
                    sagat_application::secret_provider::ProviderReference {
                        tenant: self.info.tenant_id,
                        namespace: self.info.namespace_id,
                        operation,
                    },
                ),
            })
        })
        .transpose()
    }
}

//! Managed executor admission and immutable per-attempt credential evidence. No provider I/O.
use super::*;
use sagat_application::{
    database::{DatabaseInput, DatabaseWork, SelectedSecret},
    secrets::{StoredSecret, StoredValue},
};
use sagat_domain::{
    database::{ManagedKind, SecretRole},
    secrets::{SecretMeta, SecretName},
};

/// Concurrent lake attempts a tenant and a namespace may hold until the quota rows exist (T035-055, the ceilings of plan.md).
const LAKE_TENANT_CONCURRENCY: i64 = 32;
const LAKE_NAMESPACE_CONCURRENCY: i64 = 8;

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
        let session: Option<(OffsetDateTime, Vec<String>)> = sqlx::query_as(
            "SELECT w.expires_at, w.activities FROM steps s JOIN worker_sessions w ON w.tenant_id=s.tenant_id AND w.id::text=s.leased_by
             WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node=$3 AND cardinality(s.instance_path_nodes)=0 AND cardinality(s.instance_path_indices)=0 AND w.namespace_id=$4
               AND w.subject=$5 AND w.accepted_protocol_minor>=6
             FOR SHARE OF w")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.run_id.as_uuid()).bind(node.as_str())
            .bind(self.info.namespace_id.as_uuid()).bind(subject)
            .fetch_optional(&mut *self.tx).await.map_err(store_error)?;
        let Some((expires_at, offered)) = session else {
            return Ok(false);
        };
        // The fence on the profile: the session offers the managed activities of the profile of this step, and its binding is of
        // that profile alone. The shared binding lock is held, so the rows read here are the ones a binding change would wait for.
        let kind = self
            .def
            .node(node)
            .and_then(Node::managed_kind)
            .ok_or(StoreError::DatabaseUnavailable)?;
        let bound: Vec<String> = sqlx::query_scalar(
            "SELECT profile FROM worker_bindings WHERE tenant_id=$1 AND namespace_id=$2 AND subject=$3",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(subject)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(store_error)?;
        let bound: Option<Vec<sagat_domain::binding::BindingProfile>> = bound
            .iter()
            .map(|text| sagat_domain::binding::BindingProfile::parse(text))
            .collect();
        if !bound
            .is_some_and(|bound| sagat_application::worker::profile_fence(kind, &offered, &bound))
        {
            return Ok(false);
        }
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
        crate::database::admit_node(&mut self.tx, &self.info, &self.def, node, self.lake_enabled)
            .await?;
        let connection = self.database_connection(node).await?;
        let kind = self
            .def
            .node(node)
            .and_then(Node::managed_kind)
            .ok_or(StoreError::DatabaseUnavailable)?;
        // A SQL step uses the one secret of its connection; an Iceberg write uses the catalog's and the storage's, each of
        // which has to be deliverable to this runner.
        let secrets: Vec<SecretName> = match kind {
            ManagedKind::Sql => vec![sql_secret(&connection)?.clone()],
            ManagedKind::Lake => connection.config.secrets().into_iter().cloned().collect(),
        };
        if crate::secrets::pool_refusal(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            &secrets,
            subject,
            false,
        )
        .await?
        .is_some()
        {
            return Ok(false);
        }
        // The budgets of one kind are its own: a lake attempt does not use up the concurrency of the SQL steps (G035-074).
        let (tenant, namespace, pool): (i64,i64,i64) = sqlx::query_as(
            "SELECT count(*), count(*) FILTER (WHERE b.namespace_id=$2),
                    count(*) FILTER (WHERE b.namespace_id=$2 AND b.connection_id=$3)
             FROM steps s JOIN run_database_connections b ON b.tenant_id=s.tenant_id AND b.run_id=s.run_id AND b.node_id=s.node
             WHERE s.tenant_id=$1 AND cardinality(s.instance_path_nodes)=0 AND cardinality(s.instance_path_indices)=0 AND s.status IN ('LEASED','RUNNING')
               AND (COALESCE(s.activity,'') = $4) = $5")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(connection.id.as_uuid())
            .bind(sagat_domain::database::ICEBERG_WRITE_ACTIVITY).bind(kind == ManagedKind::Lake)
            .fetch_one(&mut *self.tx).await.map_err(store_error)?;
        if kind == ManagedKind::Lake {
            return Ok(tenant < LAKE_TENANT_CONCURRENCY && namespace < LAKE_NAMESPACE_CONCURRENCY);
        }
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
            .intersect(sql_limits(&connection)?)
            .intersect(policy.limits);
        Ok(tenant < i64::from(policy.tenant_concurrency)
            && namespace < i64::from(policy.namespace_concurrency)
            && pool < i64::from(effective.pool_size))
    }

    /// What the lease of an Iceberg write needs, read in the lease transaction (D035-138): the limits of the connection at the revision the
    /// run was admitted with, the number of the workflow version, and the instant of the first lease of the step by the database's
    /// clock (the journal's `occurred_at`; none until an attempt has been leased).
    pub(super) async fn lake_facts(
        &mut self,
        node: &NodeId,
    ) -> Result<sagat_application::database::LakeLeaseFacts, StoreError> {
        let connection = self.database_connection(node).await?;
        let connection_limits = connection
            .config
            .as_iceberg()
            .ok_or(StoreError::DatabaseUnavailable)?
            .limits;
        let version: i32 = sqlx::query_scalar(
            "SELECT version_number FROM workflow_versions WHERE tenant_id=$1 AND id=$2",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.version_id.as_uuid())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)?;
        let first_leased_at: Option<OffsetDateTime> = sqlx::query_scalar(
            "SELECT min(occurred_at) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND node=$3
             AND COALESCE(cardinality(instance_path_nodes),0)=0 AND COALESCE(cardinality(instance_path_indices),0)=0
             AND event_type='step.leased'",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(sagat_application::database::LakeLeaseFacts {
            connection_limits,
            workflow_version: u32::try_from(version)
                .map_err(|_| corrupt("workflow version number"))?,
            first_leased_at,
        })
    }

    pub(super) async fn lake_blob_descriptors(
        &mut self,
        ids: &[sagat_domain::blobs::BlobId],
    ) -> Result<Vec<sagat_domain::blobs::BlobDescriptor>, StoreError> {
        // The caller holds the ceiling already; the store does not trust it with an unbounded list.
        if ids.len() > sagat_domain::database::MAX_RECORD_BLOBS {
            return Err(StoreError::DatabaseLimit);
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: Vec<Uuid> = ids.iter().map(|id| id.as_uuid()).collect();
        // The scope is the run's: its tenant, its namespace and itself as the owner of a committed step output. A run input, an upload
        // that was not committed and a blob being deleted are not returned, and nothing tells those apart from an id that is absent.
        let rows = sqlx::query(
            "SELECT id, size_bytes, sha256, content_type FROM blobs \
             WHERE tenant_id = $1 AND namespace_id = $2 AND run_id = $3 \
             AND purpose = 'STEP_OUTPUT' AND state = 'COMMITTED' AND id = ANY($4)",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(&wanted)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(store_error)?;
        rows.iter()
            .map(|row| {
                let size: Option<i64> = col(row, "size_bytes")?;
                let sha: Option<Vec<u8>> = col(row, "sha256")?;
                let (Some(size), Some(sha)) = (size, sha) else {
                    return Err(corrupt("committed blob measurement"));
                };
                Ok(sagat_domain::blobs::BlobDescriptor::new(
                    sagat_domain::blobs::BlobId::new(col::<Uuid>(row, "id")?),
                    u64::try_from(size).map_err(|_| corrupt("blob size"))?,
                    sagat_domain::workflow::Digest::new(
                        <[u8; 32]>::try_from(sha).map_err(|_| corrupt("blob digest"))?,
                    ),
                    sagat_domain::blobs::ContentType::parse(&col::<String>(row, "content_type")?)
                        .map_err(|_| corrupt("blob content type"))?,
                ))
            })
            .collect()
    }

    pub(super) async fn lake_blob_pin(
        &mut self,
        node: &NodeId,
        fence: sagat_domain::execution::Fence,
        ids: &[sagat_domain::blobs::BlobId],
    ) -> Result<(), StoreError> {
        let wanted: BTreeSet<Uuid> = ids.iter().map(|id| id.as_uuid()).collect();
        // All or none: what the catalog does not vouch for for this run (the same word the lease took the document from) is not
        // pinned, and then the blobs that are vouched for are not either.
        if self.lake_blob_descriptors(ids).await?.len() != wanted.len() {
            return Err(StoreError::MissingReference);
        }
        let wanted: Vec<Uuid> = wanted.into_iter().collect();
        sqlx::query(
            "INSERT INTO step_lease_blobs (tenant_id, namespace_id, run_id, node_id, attempt, generation, blob_id) \
             SELECT tenant_id, namespace_id, run_id, $4, $5, $6, id FROM blobs \
             WHERE tenant_id = $1 AND namespace_id = $2 AND run_id = $3 \
             AND purpose = 'STEP_OUTPUT' AND state = 'COMMITTED' AND id = ANY($7) \
             ON CONFLICT DO NOTHING",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .bind(i32::try_from(fence.attempt).map_err(|_| corrupt("step attempt"))?)
        .bind(i64::try_from(fence.generation).map_err(|_| corrupt("step generation"))?)
        .bind(&wanted)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    pub(super) async fn lake_blob_is_pinned(
        &mut self,
        node: &NodeId,
        fence: sagat_domain::execution::Fence,
        id: sagat_domain::blobs::BlobId,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM step_lease_blobs \
             WHERE tenant_id = $1 AND namespace_id = $2 AND run_id = $3 AND node_id = $4 \
             AND attempt = $5 AND generation = $6 AND blob_id = $7)",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .bind(i32::try_from(fence.attempt).map_err(|_| corrupt("step attempt"))?)
        .bind(i64::try_from(fence.generation).map_err(|_| corrupt("step generation"))?)
        .bind(id.as_uuid())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
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
        crate::database::admit_node(&mut self.tx, &self.info, &self.def, node, self.lake_enabled)
            .await?;
        let connection = self.database_connection(node).await?;
        let kind = self
            .def
            .node(node)
            .and_then(Node::managed_kind)
            .ok_or(StoreError::DatabaseUnavailable)?;
        if kind == ManagedKind::Lake {
            return self.lake_snapshot(node, subject, connection).await;
        }
        if crate::secrets::pool_refusal(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            std::slice::from_ref(sql_secret(&connection)?),
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
            .intersect(sql_limits(&connection)?)
            .intersect(policy.limits);
        operation
            .validate()
            .map_err(|_| StoreError::DatabaseLimit)?;
        let secret = self
            .selected_secret(node, SecretRole::Primary, sql_secret(&connection)?)
            .await?;
        Ok(DatabaseInput {
            connection,
            work: DatabaseWork::Sql {
                operation,
                secret: Box::new(secret),
            },
        })
    }

    /// The secrets of a lake step: one per role its connection names, each the version the attempt has pinned, or else the one in
    /// force (feature 035, D035-141). The connection is the revision the run was admitted with, so the roles are the ones it was
    /// judged for. A subject with none of the pools of a secret is refused it, whichever role the secret plays.
    async fn lake_snapshot(
        &mut self,
        node: &NodeId,
        subject: &str,
        connection: sagat_application::database::ConnectionRecord,
    ) -> Result<DatabaseInput, StoreError> {
        // refused before any secret is read: a step of the lake is only ever of an Iceberg connection
        if connection.config.as_iceberg().is_none() {
            return Err(StoreError::DatabaseUnavailable);
        }
        let slots = connection.config.secret_slots();
        // A valid lake connection always names one: with none there is nothing to hand over, and a step that is handed nothing is
        // not one this runner can run.
        if slots.is_empty() {
            return Err(StoreError::DatabaseUnavailable);
        }
        let names: Vec<SecretName> = slots.iter().map(|(_, name)| (*name).clone()).collect();
        if crate::secrets::pool_refusal(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            &names,
            subject,
            true,
        )
        .await?
        .is_some()
        {
            return Err(StoreError::DatabaseUnavailable);
        }
        let mut secrets = Vec::with_capacity(slots.len());
        for (role, name) in slots {
            secrets.push(self.selected_secret(node, role, name).await?);
        }
        Ok(DatabaseInput {
            connection,
            work: DatabaseWork::Lake { secrets },
        })
    }

    /// The secret `name` as the attempt is to use it under `role`: the version it pinned, or else the one in force. Metadata
    /// only; the value is read from the provider by the caller.
    async fn selected_secret(
        &mut self,
        node: &NodeId,
        role: SecretRole,
        name: &SecretName,
    ) -> Result<SelectedSecret, StoreError> {
        let row = sqlx::query(
            "SELECT s.id AS secret_id,s.tenant_id,s.namespace_id,s.name,v.id AS version_id,v.version,v.key_id,v.nonce,v.ciphertext,v.provider_operation,v.created_by,v.created_at
             FROM secrets s JOIN secret_versions v ON v.tenant_id=s.tenant_id AND v.secret_id=s.id AND v.version=s.current_version
             WHERE s.tenant_id=$1 AND s.namespace_id=$2 AND s.name=$3 AND s.authority='openbao'")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(name.as_str())
            .fetch_optional(&mut *self.tx).await.map_err(store_error)?.ok_or(StoreError::DatabaseUnavailable)?;
        let current = crate::secrets::stored_from(&row)?;
        let current_version = current.meta.id;
        let stored = self
            .database_pinned_secret(node, role)
            .await?
            .unwrap_or(current);
        // Reusing a logical name after deletion creates new authority. An old attempt must not
        // regain its previous credential merely because a different secret now has that name.
        let same_authority: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM secret_provider_operations WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 AND secret_id=$4)")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(stored.meta.id)
            .bind(col::<Uuid>(&row,"secret_id")?).fetch_one(&mut *self.tx).await.map_err(store_error)?;
        if !same_authority || stored.meta.name != *name {
            return Err(StoreError::DatabaseUnavailable);
        }
        Ok(SelectedSecret {
            role,
            stored,
            current_version,
        })
    }

    /// Whether the current attempt of a lake step holds a pin for every secret its connection names (feature 035, D035-142).
    pub(super) async fn lake_resolved(&mut self, node: &NodeId) -> Result<bool, StoreError> {
        let connection = self.database_connection(node).await?;
        let slots = connection.config.secret_slots();
        if slots.is_empty() {
            return Ok(false);
        }
        for (role, _) in slots {
            if self.database_pinned_secret(node, role).await?.is_none() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) async fn database_pin(
        &mut self,
        node: &NodeId,
        secrets: &[SelectedSecret],
    ) -> Result<(), StoreError> {
        // One pin per role, all in this transaction: a step that cannot pin every secret it was handed pins none.
        for selected in secrets {
            let secret = &selected.stored;
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
                "INSERT INTO step_database_credentials(tenant_id,namespace_id,run_id,node_id,attempt,generation,provider_operation,secret_name,secret_version,secret_role)
                 VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT DO NOTHING")
                .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(self.info.run_id.as_uuid()).bind(node.as_str())
                .bind(i64::from(step.attempt())).bind(i64::try_from(step.generation()).map_err(|_| corrupt("step generation"))?)
                .bind(reference.operation).bind(secret.meta.name.as_str()).bind(i64::from(secret.meta.version)).bind(selected.role.as_str())
                .execute(&mut *self.tx).await.map_err(store_error)?;
            let pinned = self
                .database_pinned_secret(node, selected.role)
                .await?
                .ok_or(StoreError::DatabaseUnavailable)?;
            if pinned.meta.id != secret.meta.id || pinned.meta.version != secret.meta.version {
                return Err(StoreError::DatabaseUnavailable);
            }
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
            .intersect(sql_limits(&connection)?)
            .intersect(policy.limits))
    }

    pub(super) async fn database_pinned_secret(
        &mut self,
        node: &NodeId,
        role: SecretRole,
    ) -> Result<Option<StoredSecret>, StoreError> {
        let step = self
            .state
            .step(node)
            .ok_or(StoreError::DatabaseUnavailable)?;
        let row = sqlx::query(
            "SELECT p.provider_operation,p.secret_name,p.secret_version,o.actor,o.created_at
             FROM step_database_credentials p JOIN secret_provider_operations o
               ON o.tenant_id=p.tenant_id AND o.namespace_id=p.namespace_id AND o.id=p.provider_operation
             WHERE p.tenant_id=$1 AND p.namespace_id=$2 AND p.run_id=$3 AND p.node_id=$4 AND p.attempt=$5 AND p.generation=$6
               AND p.secret_role=$7")
            .bind(self.info.tenant_id.as_uuid()).bind(self.info.namespace_id.as_uuid()).bind(self.info.run_id.as_uuid()).bind(node.as_str())
            .bind(i64::from(step.attempt())).bind(i64::try_from(step.generation()).map_err(|_| corrupt("step generation"))?)
            .bind(role.as_str())
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

/// The secret of a SQL connection. A lake connection has none, and a SQL step that names one is refused.
fn sql_secret(
    connection: &sagat_application::database::ConnectionRecord,
) -> Result<&sagat_domain::secrets::SecretName, StoreError> {
    connection
        .config
        .sql_secret()
        .ok_or(StoreError::DatabaseUnavailable)
}

/// The SQL limits of a SQL connection, with the same refusal.
fn sql_limits(
    connection: &sagat_application::database::ConnectionRecord,
) -> Result<sagat_domain::database::DatabaseLimits, StoreError> {
    connection
        .config
        .sql_limits()
        .ok_or(StoreError::DatabaseUnavailable)
}

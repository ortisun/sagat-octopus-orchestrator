//! PostgreSQL collection progress adapter.
//!
//! The methods here are intentionally not wired into publication or scheduling. They persist a
//! collection transition together with its journal, projections, usage and outbox. Checkpoint and
//! finish additionally require an immutable definition cache. Runtime orchestration stays gated
//! until the planner owns scoped routing and supplies semantic audit evidence for every transition.

use sagat_application::error::StoreError;
use sagat_application::execution::{DefinitionCache, instance_step_id};
use sagat_application::primitives::{
    CollectionAdmission, CollectionCheckpoint, CollectionEntry, CollectionFinish, CollectionStore,
    CollectionTransition, PrimitiveStoreError, PrimitiveWrite,
};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::ExecutionInstanceId;
use sagat_domain::identity::NamespaceId;
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{NodeKind, WorkflowDefinition};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::identity::{insert_audit, store_error};
use crate::pool::BeginSafely;

/// Storage adapter for collection transition batches.
#[derive(Debug, Clone)]
pub struct PgCollectionStore {
    pool: PgPool,
    definitions: Option<DefinitionCache>,
}

impl PgCollectionStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            definitions: None,
        }
    }

    /// Enables immutable definition checks required before a scoped return can checkpoint.
    #[must_use]
    pub fn with_definition_cache(pool: PgPool, definitions: DefinitionCache) -> Self {
        Self {
            pool,
            definitions: Some(definitions),
        }
    }

    async fn definition(
        &self,
        conn: &mut PgConnection,
        tenant_id: Uuid,
        version_id: WorkflowVersionId,
    ) -> Result<std::sync::Arc<WorkflowDefinition>, PrimitiveStoreError> {
        let cache = self
            .definitions
            .as_ref()
            .ok_or(PrimitiveStoreError::RuntimeNotIntegrated)?;
        if let Some(definition) = cache.get(version_id) {
            return Ok(definition);
        }
        let document: Vec<u8> = sqlx::query_scalar(
            "SELECT document FROM workflow_versions WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant_id)
        .bind(version_id.as_uuid())
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        cache.get_or_parse(version_id, &document).map_err(|error| {
            tracing::error!(%error, %version_id, "stored collection workflow no longer parses");
            invalid_storage("stored workflow definition")
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct LockedRun {
    namespace_id: NamespaceId,
    version_id: WorkflowVersionId,
    run_version: u64,
}

fn path_parts(instance: &ExecutionInstanceId) -> (Vec<String>, Vec<i32>) {
    instance
        .path()
        .segments()
        .iter()
        .map(|segment| {
            (
                segment.scope_node().as_str().to_owned(),
                i32::try_from(segment.iteration_index()).unwrap_or(i32::MAX),
            )
        })
        .unzip()
}

fn instance_json(instance: &ExecutionInstanceId) -> Value {
    json!({
        "path": instance.path().segments().iter().map(|segment| json!({
            "scope": segment.scope_node().as_str(),
            "index": segment.iteration_index(),
        })).collect::<Vec<_>>(),
        "node": instance.node().as_str(),
    })
}

fn payload_matches(content: &[u8], digest: sagat_domain::workflow::Digest) -> bool {
    Sha256::digest(content).as_slice() == digest.as_bytes()
}

fn number(value: u64, field: &'static str) -> Result<i64, PrimitiveStoreError> {
    i64::try_from(value).map_err(|_| {
        PrimitiveStoreError::Storage(StoreError::Internal {
            detail: format!("{field} does not fit PostgreSQL bigint"),
        })
    })
}

fn invalid_storage(detail: &'static str) -> PrimitiveStoreError {
    PrimitiveStoreError::Storage(StoreError::Internal {
        detail: detail.to_owned(),
    })
}

async fn lock_run(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
) -> Result<LockedRun, PrimitiveStoreError> {
    let row = sqlx::query(
        "SELECT namespace_id, workflow_version_id, run_version, status FROM runs \
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .ok_or(PrimitiveStoreError::Missing)?;
    let namespace_id = NamespaceId::new(row.get("namespace_id"));
    let status: String = row.get("status");
    let stored_version: i64 = row.get("run_version");
    let run_version = u64::try_from(stored_version).map_err(|_| invalid_storage("run version"))?;
    if namespace_id != transition.namespace_id()
        || !matches!(status.as_str(), "RUNNING" | "WAITING")
    {
        return Err(PrimitiveStoreError::InvalidTransition);
    }
    Ok(LockedRun {
        namespace_id,
        version_id: WorkflowVersionId::new(row.get("workflow_version_id")),
        run_version,
    })
}

fn check_revision(
    run: LockedRun,
    transition: &CollectionTransition,
) -> Result<(), PrimitiveStoreError> {
    if run.run_version != transition.expected_run_version() {
        return Err(PrimitiveStoreError::RevisionConflict {
            current: run.run_version,
        });
    }
    Ok(())
}

fn check_audit(transition: &CollectionTransition) -> Result<(), PrimitiveStoreError> {
    if transition
        .audit()
        .is_some_and(|audit| audit.tenant_id() != transition.tenant_id())
    {
        return Err(PrimitiveStoreError::AuditTenantMismatch);
    }
    Ok(())
}

async fn append_event(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    run: LockedRun,
    sequence: u64,
    event_type: &str,
    detail: Value,
) -> Result<(), PrimitiveStoreError> {
    let (nodes, indices) = path_parts(transition.owner());
    let instance_id = instance_step_id(run.version_id, transition.run_id(), transition.owner());
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, detail, actor, trace_id, occurred_at, \
          instance_id, instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(sequence, "journal sequence")?)
    .bind(event_type)
    .bind(transition.owner().node().as_str())
    .bind(detail)
    .bind(transition.actor().as_str())
    .bind(transition.trace_id())
    .bind(transition.occurred_at())
    .bind(instance_id)
    .bind(nodes)
    .bind(indices)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn append_instance_event(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    run: LockedRun,
    instance: &ExecutionInstanceId,
    sequence: u64,
    event_type: &str,
    detail: Value,
) -> Result<(), PrimitiveStoreError> {
    let (nodes, indices) = path_parts(instance);
    let instance_id = instance_step_id(run.version_id, transition.run_id(), instance);
    sqlx::query(
        "INSERT INTO run_events \
         (tenant_id, run_id, seq, event_type, node, detail, actor, trace_id, occurred_at, \
          instance_id, instance_path_nodes, instance_path_indices) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(sequence, "journal sequence")?)
    .bind(event_type)
    .bind(instance.node().as_str())
    .bind(detail)
    .bind(transition.actor().as_str())
    .bind(transition.trace_id())
    .bind(transition.occurred_at())
    .bind(instance_id)
    .bind(nodes)
    .bind(indices)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn finish_version(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
    new_version: u64,
) -> Result<(), PrimitiveStoreError> {
    sqlx::query(
        "UPDATE runs SET run_version = $3, updated_at = $4, \
                status = CASE \
                    WHEN EXISTS (SELECT 1 FROM steps \
                                 WHERE tenant_id = $1 AND run_id = $2 \
                                   AND status IN ('PENDING', 'READY', 'LEASED', 'RUNNING', 'RETRY_WAIT')) \
                        THEN 'RUNNING' \
                    WHEN EXISTS (SELECT 1 FROM steps \
                                 WHERE tenant_id = $1 AND run_id = $2 AND status = 'WAITING') \
                        THEN 'WAITING' \
                    ELSE 'RUNNING' \
                END \
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(transition.tenant_id().as_uuid())
    .bind(transition.run_id().as_uuid())
    .bind(number(new_version, "run version")?)
    .bind(transition.occurred_at())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    if let Some(audit) = transition.audit() {
        insert_audit(conn, audit).await?;
    }
    Ok(())
}

async fn shared_policy_fence(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
) -> Result<(), PrimitiveStoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
        .bind(format!("primitive-policy:{}", transition.tenant_id()))
        .execute(&mut *conn)
        .await
        .map_err(store_error)?;
    Ok(())
}

async fn check_scheduler_fence(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
) -> Result<(), PrimitiveStoreError> {
    let authority = transition.scheduler();
    let lease = authority.lease();
    if lease.shard != Shard::of(transition.run_id().as_uuid()) {
        return Err(PrimitiveStoreError::SchedulerFenceLost);
    }
    let held: Option<i16> = sqlx::query_scalar(
        "SELECT shard FROM scheduler_shards \
         WHERE shard = $1 AND generation = $2 AND owner = $3 \
           AND expires_at > clock_timestamp() FOR SHARE",
    )
    .bind(i16::from(lease.shard.index()))
    .bind(number(lease.generation, "scheduler generation")?)
    .bind(authority.owner())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    if held.is_none() {
        return Err(PrimitiveStoreError::SchedulerFenceLost);
    }
    Ok(())
}

async fn tenant_active_limit(
    conn: &mut PgConnection,
    transition: &CollectionTransition,
) -> Result<u32, PrimitiveStoreError> {
    shared_policy_fence(conn, transition).await?;
    let limit: Option<i32> = sqlx::query_scalar(
        "SELECT max_active_iterations FROM tenant_collection_policies \
         WHERE tenant_id = $1 FOR SHARE",
    )
    .bind(transition.tenant_id().as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    limit
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(PrimitiveStoreError::PolicyMissing)
}

#[allow(clippy::too_many_lines)]
impl CollectionStore for PgCollectionStore {
    async fn enter_collection(
        &self,
        entry: &CollectionEntry,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&entry.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &entry.transition).await?;
        check_scheduler_fence(&mut tx, &entry.transition).await?;
        let (owner_nodes, owner_indices) = path_parts(entry.transition.owner());
        if let Some(row) = sqlx::query(
            "SELECT scope_id, snapshot_digest, item_count, max_items, max_concurrency \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 \
               AND owner_path_nodes = $3 AND owner_path_indices = $4 AND owner_node = $5",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(entry.transition.owner().node().as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        {
            let matches = row.get::<Uuid, _>("scope_id") == entry.scope_id.as_uuid()
                && row.get::<Vec<u8>, _>("snapshot_digest")
                    == entry.snapshot_digest.as_bytes().as_slice()
                && row.get::<i32, _>("item_count") == i32::try_from(entry.item_count).unwrap_or(-1)
                && row.get::<i32, _>("max_items")
                    == i32::try_from(entry.limits.max_items()).unwrap_or(-1)
                && row.get::<i32, _>("max_concurrency")
                    == i32::from(entry.limits.max_concurrency());
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: run.run_version,
            });
        }
        check_revision(run, &entry.transition)?;
        tenant_active_limit(&mut tx, &entry.transition).await?;

        let snapshot_content: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
                .bind(entry.transition.tenant_id().as_uuid())
                .bind(entry.snapshot_digest.as_bytes().as_slice())
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?;
        let Some(snapshot_content) = snapshot_content else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        if !payload_matches(&snapshot_content, entry.snapshot_digest) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let snapshot: Value = serde_json::from_slice(&snapshot_content)
            .map_err(|_| PrimitiveStoreError::InvalidTransition)?;
        let Value::Array(items) = snapshot else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        if items.len() != usize::try_from(entry.item_count).unwrap_or(usize::MAX) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let item_payloads = items
            .iter()
            .map(|item| {
                serde_json::to_vec(item)
                    .map(|bytes| (Sha256::digest(&bytes).to_vec(), bytes))
                    .map_err(|_| invalid_storage("collection snapshot item"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let item_digests = item_payloads
            .iter()
            .map(|(digest, _)| digest.clone())
            .collect::<Vec<_>>();
        let item_contents = item_payloads
            .into_iter()
            .map(|(_, content)| content)
            .collect::<Vec<_>>();
        if !item_digests.is_empty() {
            sqlx::query(
                "INSERT INTO payloads (tenant_id, digest, content) \
                 SELECT $1, item.digest, item.content \
                 FROM unnest($2::bytea[], $3::bytea[]) AS item(digest, content) \
                 ON CONFLICT (tenant_id, digest) DO NOTHING",
            )
            .bind(entry.transition.tenant_id().as_uuid())
            .bind(&item_digests)
            .bind(&item_contents)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            let mismatched: bool = sqlx::query_scalar(
                "SELECT EXISTS ( \
                   SELECT 1 FROM unnest($2::bytea[], $3::bytea[]) AS item(digest, content) \
                   JOIN payloads p ON p.tenant_id = $1 AND p.digest = item.digest \
                   WHERE p.content <> item.content)",
            )
            .bind(entry.transition.tenant_id().as_uuid())
            .bind(&item_digests)
            .bind(&item_contents)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            if mismatched {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
        }

        let usage = sqlx::query(
            "SELECT admitted_instances, reserved_instances FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let consumed =
            usage.get::<i32, _>("admitted_instances") + usage.get::<i32, _>("reserved_instances");
        if consumed > i32::try_from(entry.limits.max_instances()).unwrap_or(i32::MAX) {
            return Err(PrimitiveStoreError::InstanceBudgetExceeded);
        }
        sqlx::query(
            "UPDATE run_instance_usage SET max_instances = least(max_instances, $3), \
                    max_depth = least(max_depth, $4) \
             WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .bind(i32::try_from(entry.limits.max_instances()).unwrap_or(i32::MAX))
        .bind(i16::from(entry.limits.max_depth()))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let parked = sqlx::query(
            "UPDATE steps SET status = 'WAITING', waiting_kind = 'COLLECTION', updated_at = $6 \
             WHERE tenant_id = $1 AND run_id = $2 AND instance_path_nodes = $3 \
               AND instance_path_indices = $4 AND node = $5 AND status = 'PENDING'",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(entry.transition.owner().node().as_str())
        .bind(entry.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if parked.rows_affected() != 1 {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        sqlx::query(
            "INSERT INTO iteration_scopes \
             (tenant_id, run_id, scope_id, namespace_id, owner_path_nodes, owner_path_indices, \
              owner_node, kind, lifecycle, snapshot_digest, item_count, max_items, max_concurrency, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'COLLECTION', 'OPEN', $8, $9, $10, $11, $12, $12)",
        )
        .bind(entry.transition.tenant_id().as_uuid())
        .bind(entry.transition.run_id().as_uuid())
        .bind(entry.scope_id.as_uuid())
        .bind(run.namespace_id.as_uuid())
        .bind(owner_nodes)
        .bind(owner_indices)
        .bind(entry.transition.owner().node().as_str())
        .bind(entry.snapshot_digest.as_bytes().as_slice())
        .bind(i32::try_from(entry.item_count).unwrap_or(i32::MAX))
        .bind(i32::try_from(entry.limits.max_items()).unwrap_or(i32::MAX))
        .bind(i32::from(entry.limits.max_concurrency()))
        .bind(entry.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if !item_digests.is_empty() {
            let indices = (0..item_digests.len())
                .map(|index| i32::try_from(index).unwrap_or(i32::MAX))
                .collect::<Vec<_>>();
            sqlx::query(
                "INSERT INTO collection_scope_items \
                 (tenant_id, run_id, scope_id, iteration_index, item_digest) \
                 SELECT $1, $2, $3, item_index, item_digest \
                 FROM unnest($4::integer[], $5::bytea[]) AS item(item_index, item_digest)",
            )
            .bind(entry.transition.tenant_id().as_uuid())
            .bind(entry.transition.run_id().as_uuid())
            .bind(entry.scope_id.as_uuid())
            .bind(indices)
            .bind(item_digests)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        let new_version = run.run_version + 1;
        append_event(
            &mut tx,
            &entry.transition,
            run,
            new_version,
            "collection.entered",
            json!({
                "item_count": entry.item_count,
                "max_items": entry.limits.max_items(),
                "max_concurrency": entry.limits.max_concurrency(),
                "snapshot_digest": entry.snapshot_digest.to_hex(),
            }),
        )
        .await?;
        finish_version(&mut tx, &entry.transition, new_version).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: new_version,
        })
    }

    async fn admit_collection_activity(
        &self,
        admission: &CollectionAdmission,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&admission.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &admission.transition).await?;
        check_scheduler_fence(&mut tx, &admission.transition).await?;
        let (owner_nodes, owner_indices) = path_parts(admission.transition.owner());
        let stored_owner = sqlx::query(
            "SELECT namespace_id, owner_path_nodes, owner_path_indices, owner_node \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if NamespaceId::new(stored_owner.get("namespace_id")) != run.namespace_id
            || stored_owner.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || stored_owner.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || stored_owner.get::<String, _>("owner_node")
                != admission.transition.owner().node().as_str()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        if let Some(row) = sqlx::query(
            "SELECT entry_instance_id, item_digest, activity, reserved_instances, outbox_id \
             FROM collection_iterations WHERE tenant_id = $1 AND run_id = $2 \
               AND scope_id = $3 AND iteration_index = $4",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .bind(i32::try_from(admission.index).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        {
            let expected_id = instance_step_id(
                run.version_id,
                admission.transition.run_id(),
                &admission.entry,
            );
            let matches = row.get::<Uuid, _>("entry_instance_id") == expected_id
                && row.get::<Vec<u8>, _>("item_digest")
                    == admission.item_digest.as_bytes().as_slice()
                && row.get::<String, _>("activity") == admission.activity.as_str()
                && row.get::<i32, _>("reserved_instances")
                    == i32::from(admission.reserved_instances)
                && row.get::<Uuid, _>("outbox_id") == admission.outbox_id;
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: run.run_version,
            });
        }
        check_revision(run, &admission.transition)?;
        let active_limit = tenant_active_limit(&mut tx, &admission.transition).await?;
        let active: i32 = sqlx::query_scalar(
            "SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id = $1 FOR UPDATE",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if u32::try_from(active).unwrap_or(u32::MAX) >= active_limit {
            return Err(PrimitiveStoreError::AtCapacity);
        }
        let usage = sqlx::query(
            "SELECT max_instances, admitted_instances, reserved_instances FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let charge = 1_i64 + i64::from(admission.reserved_instances);
        let consumed = i64::from(usage.get::<i32, _>("admitted_instances"))
            + i64::from(usage.get::<i32, _>("reserved_instances"));
        if consumed + charge > i64::from(usage.get::<i32, _>("max_instances")) {
            return Err(PrimitiveStoreError::InstanceBudgetExceeded);
        }
        let scope = sqlx::query(
            "SELECT lifecycle, item_count, next_admission_index, active_count, max_concurrency \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 FOR UPDATE",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if scope.get::<String, _>("lifecycle") != "OPEN"
            || scope.get::<i32, _>("next_admission_index")
                != i32::try_from(admission.index).unwrap_or(i32::MAX)
            || admission.index >= u32::try_from(scope.get::<i32, _>("item_count")).unwrap_or(0)
            || scope.get::<i32, _>("active_count") >= scope.get::<i32, _>("max_concurrency")
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let expected_item: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT item_digest FROM collection_scope_items \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 AND iteration_index = $4",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .bind(i32::try_from(admission.index).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        if expected_item.as_deref() != Some(admission.item_digest.as_bytes().as_slice()) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let item_content: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
                .bind(admission.transition.tenant_id().as_uuid())
                .bind(admission.item_digest.as_bytes().as_slice())
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?;
        if item_content
            .as_deref()
            .is_none_or(|content| !payload_matches(content, admission.item_digest))
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let (entry_nodes, entry_indices) = path_parts(&admission.entry);
        let entry_id = instance_step_id(
            run.version_id,
            admission.transition.run_id(),
            &admission.entry,
        );
        let admitted_sequence = run.run_version + 1;
        let released_sequence = admitted_sequence + 1;
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id, run_id, node, step_id, namespace_id, status, activity, ready_at, \
              last_hinted_at, hint_count, updated_at, instance_path_nodes, instance_path_indices) \
             VALUES ($1, $2, $3, $4, $5, 'READY', $6, $7, $7, 1, $7, $8, $9)",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.entry.node().as_str())
        .bind(entry_id)
        .bind(run.namespace_id.as_uuid())
        .bind(admission.activity.as_str())
        .bind(admission.transition.occurred_at())
        .bind(&entry_nodes)
        .bind(&entry_indices)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO collection_iterations \
             (tenant_id, run_id, scope_id, iteration_index, entry_path_nodes, entry_path_indices, \
              entry_node, entry_instance_id, activity, item_digest, lifecycle, slot_held, \
              reserved_instances, remaining_reservations, admitted_sequence, outbox_id, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'ADMITTED', true, $11, $11, \
                     $12, $13, $14, $14)",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .bind(i32::try_from(admission.index).unwrap_or(i32::MAX))
        .bind(&entry_nodes)
        .bind(&entry_indices)
        .bind(admission.entry.node().as_str())
        .bind(entry_id)
        .bind(admission.activity.as_str())
        .bind(admission.item_digest.as_bytes().as_slice())
        .bind(i32::from(admission.reserved_instances))
        .bind(number(admitted_sequence, "journal sequence")?)
        .bind(admission.outbox_id)
        .bind(admission.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE iteration_scopes SET next_admission_index = next_admission_index + 1, \
                    admitted_count = admitted_count + 1, active_count = active_count + 1, updated_at = $4 \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.scope_id.as_uuid())
        .bind(admission.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE run_instance_usage SET admitted_instances = admitted_instances + 1, \
                    reserved_instances = reserved_instances + $3 \
             WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(i32::from(admission.reserved_instances))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE tenant_iteration_usage SET active_iterations = active_iterations + 1 WHERE tenant_id = $1",
        )
        .bind(admission.transition.tenant_id().as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        append_event(
            &mut tx,
            &admission.transition,
            run,
            admitted_sequence,
            "collection.iteration_admitted",
            json!({
                "index": admission.index,
                "entry": instance_json(&admission.entry),
                "item_digest": admission.item_digest.to_hex(),
            }),
        )
        .await?;
        append_instance_event(
            &mut tx,
            &admission.transition,
            run,
            &admission.entry,
            released_sequence,
            "step.released",
            json!({}),
        )
        .await?;
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1, $2, $3, 'task.ready', $4, $5)",
        )
        .bind(admission.outbox_id)
        .bind(admission.transition.tenant_id().as_uuid())
        .bind(admission.transition.run_id().as_uuid())
        .bind(admission.activity.as_str())
        .bind(json!({
            "tenantId": admission.transition.tenant_id().to_string(),
            "namespaceId": admission.transition.namespace_id().to_string(),
            "runId": admission.transition.run_id().to_string(),
            "node": admission.entry.node().as_str(),
            "instancePath": instance_json(&admission.entry)["path"],
            "activity": admission.activity.as_str(),
            "traceId": admission.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        finish_version(&mut tx, &admission.transition, released_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: released_sequence,
        })
    }

    async fn checkpoint_collection_iteration(
        &self,
        checkpoint: &CollectionCheckpoint,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&checkpoint.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &checkpoint.transition).await?;
        check_scheduler_fence(&mut tx, &checkpoint.transition).await?;

        let _: i32 = sqlx::query_scalar(
            "SELECT active_iterations FROM tenant_iteration_usage \
             WHERE tenant_id = $1 FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let usage = sqlx::query(
            "SELECT admitted_instances, reserved_instances FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;

        let (owner_nodes, owner_indices) = path_parts(checkpoint.transition.owner());
        let scope = sqlx::query(
            "SELECT namespace_id, owner_path_nodes, owner_path_indices, owner_node, lifecycle, \
                    active_count \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 \
             FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if NamespaceId::new(scope.get("namespace_id")) != run.namespace_id
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != checkpoint.transition.owner().node().as_str()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        let iteration = sqlx::query(
            "SELECT lifecycle, slot_held, remaining_reservations, entry_path_nodes, \
                    entry_path_indices, return_path_nodes, return_path_indices, return_node, \
                    return_instance_id, result_digest, checkpoint_sequence \
             FROM collection_iterations WHERE tenant_id = $1 AND run_id = $2 \
               AND scope_id = $3 AND iteration_index = $4 FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .bind(i32::try_from(checkpoint.index).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;

        let definition = self
            .definition(
                &mut tx,
                checkpoint.transition.tenant_id().as_uuid(),
                run.version_id,
            )
            .await?;
        if !matches!(
            definition
                .node_for_instance(checkpoint.transition.owner())
                .map(sagat_domain::workflow::Node::kind),
            Some(NodeKind::ForEach { .. })
        ) || !matches!(
            definition
                .node_for_instance(&checkpoint.return_instance)
                .map(sagat_domain::workflow::Node::kind),
            Some(NodeKind::Return { .. })
        ) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        let return_id = instance_step_id(
            run.version_id,
            checkpoint.transition.run_id(),
            &checkpoint.return_instance,
        );
        let (return_nodes, return_indices) = path_parts(&checkpoint.return_instance);
        if iteration.get::<String, _>("lifecycle") == "CHECKPOINTED" {
            let saved = sqlx::query(
                "SELECT return_instance_id, result_digest, outbox_id FROM iteration_checkpoints \
                 WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 AND iteration_index = $4",
            )
            .bind(checkpoint.transition.tenant_id().as_uuid())
            .bind(checkpoint.transition.run_id().as_uuid())
            .bind(checkpoint.scope_id.as_uuid())
            .bind(i32::try_from(checkpoint.index).unwrap_or(i32::MAX))
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?
            .ok_or_else(|| invalid_storage("checkpoint projection"))?;
            let matches = saved.get::<Uuid, _>("return_instance_id") == return_id
                && saved.get::<Vec<u8>, _>("result_digest")
                    == checkpoint.result_digest.as_bytes().as_slice()
                && saved.get::<Uuid, _>("outbox_id") == checkpoint.outbox_id
                && iteration.get::<Option<Vec<String>>, _>("return_path_nodes")
                    == Some(return_nodes)
                && iteration.get::<Option<Vec<i32>>, _>("return_path_indices")
                    == Some(return_indices)
                && iteration.get::<Option<String>, _>("return_node")
                    == Some(checkpoint.return_instance.node().as_str().to_owned());
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: run.run_version,
            });
        }

        check_revision(run, &checkpoint.transition)?;
        if scope.get::<String, _>("lifecycle") != "OPEN"
            || scope.get::<i32, _>("active_count") <= 0
            || iteration.get::<String, _>("lifecycle") != "ADMITTED"
            || !iteration.get::<bool, _>("slot_held")
            || iteration.get::<i32, _>("remaining_reservations") <= 0
            || usage.get::<i32, _>("reserved_instances") <= 0
            || iteration.get::<Vec<String>, _>("entry_path_nodes") != return_nodes
            || iteration.get::<Vec<i32>, _>("entry_path_indices") != return_indices
            || iteration
                .get::<Option<i64>, _>("checkpoint_sequence")
                .is_some()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let result_content: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
                .bind(checkpoint.transition.tenant_id().as_uuid())
                .bind(checkpoint.result_digest.as_bytes().as_slice())
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?;
        if result_content
            .as_deref()
            .is_none_or(|content| !payload_matches(content, checkpoint.result_digest))
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        let member_steps = sqlx::query(
            "SELECT node, step_id, status, routed, attempt, generation, lease_token_hash, \
                    waiting_kind \
             FROM steps WHERE tenant_id = $1 AND run_id = $2 \
               AND instance_path_nodes = $3 AND instance_path_indices = $4 \
             ORDER BY node FOR UPDATE",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(&return_nodes)
        .bind(&return_indices)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut found_return = false;
        for member in member_steps {
            let node: String = member.get("node");
            if node == checkpoint.return_instance.node().as_str() {
                let valid_return = !found_return
                    && member.get::<Uuid, _>("step_id") == return_id
                    && member.get::<String, _>("status") == "PENDING"
                    && !member.get::<bool, _>("routed")
                    && member.get::<i32, _>("attempt") == 0
                    && member.get::<i64, _>("generation") == 0
                    && member
                        .get::<Option<Vec<u8>>, _>("lease_token_hash")
                        .is_none()
                    && member.get::<Option<String>, _>("waiting_kind").is_none();
                if !valid_return {
                    return Err(PrimitiveStoreError::InvalidTransition);
                }
                found_return = true;
            } else if !matches!(
                member.get::<String, _>("status").as_str(),
                "SUCCEEDED" | "SKIPPED"
            ) || !member.get::<bool, _>("routed")
            {
                return Err(PrimitiveStoreError::InvalidTransition);
            }
        }
        if !found_return {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        let completed = sqlx::query(
            "UPDATE steps SET status = 'SUCCEEDED', routed = true, updated_at = $8 \
             WHERE tenant_id = $1 AND run_id = $2 AND namespace_id = $3 \
               AND instance_path_nodes = $4 AND instance_path_indices = $5 AND node = $6 \
               AND step_id = $7 AND status = 'PENDING' AND attempt = 0 AND generation = 0 \
               AND NOT routed AND lease_token_hash IS NULL AND waiting_kind IS NULL",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(run.namespace_id.as_uuid())
        .bind(&return_nodes)
        .bind(&return_indices)
        .bind(checkpoint.return_instance.node().as_str())
        .bind(return_id)
        .bind(checkpoint.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if completed.rows_affected() != 1 {
            return Err(PrimitiveStoreError::InvalidTransition);
        }

        let returned_sequence = run.run_version + 1;
        let checkpoint_sequence = returned_sequence + 1;
        sqlx::query(
            "UPDATE collection_iterations SET lifecycle = 'CHECKPOINTED', slot_held = false, \
                    remaining_reservations = remaining_reservations - 1, \
                    return_path_nodes = $5, return_path_indices = $6, return_node = $7, \
                    return_instance_id = $8, result_digest = $9, checkpoint_sequence = $10, \
                    updated_at = $11 \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 AND iteration_index = $4",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .bind(i32::try_from(checkpoint.index).unwrap_or(i32::MAX))
        .bind(&return_nodes)
        .bind(&return_indices)
        .bind(checkpoint.return_instance.node().as_str())
        .bind(return_id)
        .bind(checkpoint.result_digest.as_bytes().as_slice())
        .bind(number(checkpoint_sequence, "journal sequence")?)
        .bind(checkpoint.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO iteration_checkpoints \
             (tenant_id, run_id, scope_id, iteration_index, return_instance_id, result_digest, \
              journal_sequence, outbox_id, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .bind(i32::try_from(checkpoint.index).unwrap_or(i32::MAX))
        .bind(return_id)
        .bind(checkpoint.result_digest.as_bytes().as_slice())
        .bind(number(checkpoint_sequence, "journal sequence")?)
        .bind(checkpoint.outbox_id)
        .bind(checkpoint.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE iteration_scopes SET completed_count = completed_count + 1, \
                    active_count = active_count - 1, updated_at = $4 \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.scope_id.as_uuid())
        .bind(checkpoint.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let tenant_released = sqlx::query(
            "UPDATE tenant_iteration_usage SET active_iterations = active_iterations - 1 \
             WHERE tenant_id = $1 AND active_iterations > 0",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let reservation_consumed = sqlx::query(
            "UPDATE run_instance_usage SET admitted_instances = admitted_instances + 1, \
                    reserved_instances = reserved_instances - 1 \
             WHERE tenant_id = $1 AND run_id = $2 AND reserved_instances > 0",
        )
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if tenant_released.rows_affected() != 1 || reservation_consumed.rows_affected() != 1 {
            return Err(invalid_storage("collection usage projection"));
        }

        append_event(
            &mut tx,
            &checkpoint.transition,
            run,
            returned_sequence,
            "collection.iteration_returned",
            json!({
                "index": checkpoint.index,
                "return_instance": instance_json(&checkpoint.return_instance),
            }),
        )
        .await?;
        append_event(
            &mut tx,
            &checkpoint.transition,
            run,
            checkpoint_sequence,
            "collection.iteration_checkpointed",
            json!({
                "index": checkpoint.index,
                "return_instance": instance_json(&checkpoint.return_instance),
                "result_digest": checkpoint.result_digest.to_hex(),
            }),
        )
        .await?;
        sqlx::query(
            "INSERT INTO event_outbox \
             (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1, $2, $3, 'collection.iteration.checkpointed', $4, $5)",
        )
        .bind(checkpoint.outbox_id)
        .bind(checkpoint.transition.tenant_id().as_uuid())
        .bind(checkpoint.transition.run_id().as_uuid())
        .bind(checkpoint.transition.owner().node().as_str())
        .bind(json!({
            "tenantId": checkpoint.transition.tenant_id().to_string(),
            "namespaceId": checkpoint.transition.namespace_id().to_string(),
            "runId": checkpoint.transition.run_id().to_string(),
            "scopeId": checkpoint.scope_id.as_uuid().to_string(),
            "index": checkpoint.index,
            "traceId": checkpoint.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        finish_version(&mut tx, &checkpoint.transition, checkpoint_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: checkpoint_sequence,
        })
    }

    async fn finish_collection(
        &self,
        finish: &CollectionFinish,
    ) -> Result<PrimitiveWrite, PrimitiveStoreError> {
        check_audit(&finish.transition)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let run = lock_run(&mut tx, &finish.transition).await?;
        check_scheduler_fence(&mut tx, &finish.transition).await?;
        let _: i32 = sqlx::query_scalar(
            "SELECT active_iterations FROM tenant_iteration_usage \
             WHERE tenant_id = $1 FOR UPDATE",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let _: i32 = sqlx::query_scalar(
            "SELECT admitted_instances FROM run_instance_usage \
             WHERE tenant_id = $1 AND run_id = $2 FOR UPDATE",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;

        let (owner_nodes, owner_indices) = path_parts(finish.transition.owner());
        let scope = sqlx::query(
            "SELECT namespace_id, owner_path_nodes, owner_path_indices, owner_node, lifecycle, \
                    item_count, next_admission_index, admitted_count, completed_count, \
                    active_count, result_digest, finish_sequence, finish_outbox_id \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 \
             FOR UPDATE",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        .ok_or(PrimitiveStoreError::Missing)?;
        if NamespaceId::new(scope.get("namespace_id")) != run.namespace_id
            || scope.get::<Vec<String>, _>("owner_path_nodes") != owner_nodes
            || scope.get::<Vec<i32>, _>("owner_path_indices") != owner_indices
            || scope.get::<String, _>("owner_node") != finish.transition.owner().node().as_str()
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let definition = self
            .definition(
                &mut tx,
                finish.transition.tenant_id().as_uuid(),
                run.version_id,
            )
            .await?;
        let Some(owner_definition) = definition.node_for_instance(finish.transition.owner()) else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        if !matches!(owner_definition.kind(), NodeKind::ForEach { .. }) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let successor_node = owner_definition
            .next()
            .cloned()
            .ok_or(PrimitiveStoreError::RuntimeNotIntegrated)?;
        let successor =
            ExecutionInstanceId::in_path(finish.transition.owner().path().clone(), successor_node);
        if definition.node_for_instance(&successor).is_none() {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        if scope.get::<String, _>("lifecycle") == "SUCCEEDED" {
            let matches = scope.get::<Option<Vec<u8>>, _>("result_digest")
                == Some(finish.result_digest.as_bytes().to_vec())
                && scope.get::<Option<Uuid>, _>("finish_outbox_id") == Some(finish.outbox_id)
                && scope.get::<Option<i64>, _>("finish_sequence").is_some();
            if !matches {
                return Err(PrimitiveStoreError::DuplicateMismatch);
            }
            tx.rollback().await.map_err(store_error)?;
            return Ok(PrimitiveWrite {
                applied: false,
                run_version: run.run_version,
            });
        }

        check_revision(run, &finish.transition)?;
        let item_count = scope.get::<i32, _>("item_count");
        if scope.get::<String, _>("lifecycle") != "OPEN"
            || scope.get::<i32, _>("next_admission_index") != item_count
            || scope.get::<i32, _>("admitted_count") != item_count
            || scope.get::<i32, _>("completed_count") != item_count
            || scope.get::<i32, _>("active_count") != 0
        {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let unsettled: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM collection_iterations \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3 \
               AND (lifecycle <> 'CHECKPOINTED' OR slot_held OR checkpoint_sequence IS NULL)",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let checkpoint_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM iteration_checkpoints \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if unsettled != 0 || checkpoint_count != i64::from(item_count) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let aggregate_content: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
                .bind(finish.transition.tenant_id().as_uuid())
                .bind(finish.result_digest.as_bytes().as_slice())
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?;
        let Some(aggregate_content) = aggregate_content else {
            return Err(PrimitiveStoreError::InvalidTransition);
        };
        if !payload_matches(&aggregate_content, finish.result_digest) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let result_rows = sqlx::query(
            "SELECT c.result_digest, p.content FROM iteration_checkpoints c \
             JOIN payloads p ON p.tenant_id = c.tenant_id AND p.digest = c.result_digest \
             WHERE c.tenant_id = $1 AND c.run_id = $2 AND c.scope_id = $3 \
             ORDER BY c.iteration_index FOR UPDATE OF c",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let ordered_results = result_rows
            .iter()
            .map(|row| {
                let digest = row.get::<Vec<u8>, _>("result_digest");
                let content = row.get::<Vec<u8>, _>("content");
                if Sha256::digest(&content).as_slice() != digest {
                    return Err(invalid_storage("checkpoint result payload digest"));
                }
                serde_json::from_slice(&content)
                    .map_err(|_| invalid_storage("checkpoint result payload"))
            })
            .collect::<Result<Vec<Value>, PrimitiveStoreError>>()?;
        let aggregate: Value = serde_json::from_slice(&aggregate_content)
            .map_err(|_| invalid_storage("collection aggregate payload"))?;
        if aggregate != Value::Array(ordered_results) {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let owner_finished = sqlx::query(
            "UPDATE steps SET status = 'SUCCEEDED', waiting_kind = NULL, routed = true, \
                    updated_at = $6 \
             WHERE tenant_id = $1 AND run_id = $2 AND instance_path_nodes = $3 \
               AND instance_path_indices = $4 AND node = $5 \
               AND status = 'WAITING' AND waiting_kind = 'COLLECTION' AND NOT routed",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(finish.transition.owner().node().as_str())
        .bind(finish.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if owner_finished.rows_affected() != 1 {
            return Err(PrimitiveStoreError::InvalidTransition);
        }
        let finish_sequence = run.run_version + 1;
        let routed_sequence = finish_sequence + 1;
        let (successor_nodes, successor_indices) = path_parts(&successor);
        sqlx::query(
            "INSERT INTO steps \
             (tenant_id, run_id, node, step_id, namespace_id, status, updated_at, \
              instance_path_nodes, instance_path_indices) \
             VALUES ($1, $2, $3, $4, $5, 'PENDING', $6, $7, $8)",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(successor.node().as_str())
        .bind(instance_step_id(
            run.version_id,
            finish.transition.run_id(),
            &successor,
        ))
        .bind(run.namespace_id.as_uuid())
        .bind(finish.transition.occurred_at())
        .bind(successor_nodes)
        .bind(successor_indices)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE iteration_scopes SET lifecycle = 'SUCCEEDED', result_digest = $4, \
                    finish_sequence = $5, finish_outbox_id = $6, updated_at = $7 \
             WHERE tenant_id = $1 AND run_id = $2 AND scope_id = $3",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .bind(finish.result_digest.as_bytes().as_slice())
        .bind(number(finish_sequence, "journal sequence")?)
        .bind(finish.outbox_id)
        .bind(finish.transition.occurred_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        append_event(
            &mut tx,
            &finish.transition,
            run,
            finish_sequence,
            "collection.finished",
            json!({
                "outcome": "succeeded",
                "result_digest": finish.result_digest.to_hex(),
            }),
        )
        .await?;
        append_event(
            &mut tx,
            &finish.transition,
            run,
            routed_sequence,
            "step.routed",
            json!({ "activated": [successor.node().as_str()] }),
        )
        .await?;
        sqlx::query(
            "INSERT INTO event_outbox \
             (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1, $2, $3, 'collection.finished', $4, $5)",
        )
        .bind(finish.outbox_id)
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.transition.owner().node().as_str())
        .bind(json!({
            "tenantId": finish.transition.tenant_id().to_string(),
            "namespaceId": finish.transition.namespace_id().to_string(),
            "runId": finish.transition.run_id().to_string(),
            "scopeId": finish.scope_id.as_uuid().to_string(),
            "status": "SUCCEEDED",
            "traceId": finish.transition.trace_id(),
        }))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        finish_version(&mut tx, &finish.transition, routed_sequence).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(PrimitiveWrite {
            applied: true,
            run_version: routed_sequence,
        })
    }
}

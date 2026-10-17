//! PostgreSQL implementation of the definition store port.
//!
//! Every statement carries the tenant filter. Writes that must be atomic with their audit record
//! and their idempotency record run in one transaction; the compare-and-swap of the draft and the
//! numbering of versions are single statements or row-locked sequences, never read-modify-write
//! across round trips.

use crate::pool::BeginSafely;
use sagat_application::declared_operations::PublishedDeclaredState;
use sagat_application::definitions::{
    Applied, DefinitionStore, Idempotency, PublishContext, StoredPlan, StoredPlanEntry,
    VersionRecord, WorkflowRecord,
};
use sagat_application::dispatch::TriggerRegistration;
use sagat_application::error::StoreError;
use sagat_application::execution::compensation_id;
use sagat_domain::audit::AuditRecord;
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersion, WorkflowVersionId};
use sagat_domain::identity::{Namespace, NamespaceId, Principal, Slug, Subject, TenantId};
use sagat_domain::saga::SagaPlan;
use sagat_domain::workflow::{ActivityName, Digest, NodeId, WorkflowDefinition};
use sha2::{Digest as _, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::identity::{
    PgIdentityStore, col, corrupt, insert_audit, namespace_from_row, store_error,
};
use crate::quotas::ensure_room_for_workflow;

/// PostgreSQL-backed definition store.
#[derive(Debug, Clone)]
pub struct PgDefinitionStore {
    pool: PgPool,
    identity: PgIdentityStore,
}

impl PgDefinitionStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            identity: PgIdentityStore::new(pool.clone()),
            pool,
        }
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// `SELECT` of a workflow with its active version, ending in `$tail`. A macro (not a function) so
/// the result is a string literal: `sqlx` only accepts literal SQL, which rules out injection.
macro_rules! select_workflow {
    ($tail:literal) => {
        concat!(
            "SELECT w.id, w.tenant_id, w.namespace_id, w.name, w.draft_revision, w.draft, ",
            "w.active_version_id, w.created_at, w.updated_at, ",
            "v.id AS v_id, v.version_number AS v_number, v.digest AS v_digest, ",
            "v.published_by AS v_published_by, v.published_at AS v_published_at ",
            "FROM (SELECT * FROM workflows WHERE NOT internal) w ",
            "LEFT JOIN workflow_versions v ON v.tenant_id = w.tenant_id AND v.id = w.active_version_id ",
            $tail
        )
    };
}

/// Makes `triggers` the ones in force for the workflow: the triggers of the version it succeeds that
/// are not in the new one stop matching, and one that is kept keeps its id (and so the memory of
/// which events already started a run) and its place in the schedule.
pub(crate) async fn register_triggers(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workflow: &Workflow,
    version: WorkflowVersionId,
    triggers: &[TriggerRegistration],
) -> Result<(), StoreError> {
    let tenant = workflow.tenant_id();
    let kept: Vec<Uuid> = triggers.iter().map(|t| t.id).collect();
    sqlx::query(
        "DELETE FROM workflow_triggers WHERE tenant_id = $1 AND workflow_id = $2 AND id <> ALL($3)",
    )
    .bind(tenant.as_uuid())
    .bind(workflow.id().as_uuid())
    .bind(&kept)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    for trigger in triggers {
        sqlx::query(
            "INSERT INTO workflow_triggers \
             (id, tenant_id, namespace_id, workflow_id, version_id, kind, match_source, match_type, \
              filter, cron, timezone) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
             ON CONFLICT (id) DO UPDATE SET version_id = EXCLUDED.version_id, \
                 filter = EXCLUDED.filter",
        )
        .bind(trigger.id)
        .bind(tenant.as_uuid())
        .bind(workflow.namespace_id().as_uuid())
        .bind(workflow.id().as_uuid())
        .bind(version.as_uuid())
        .bind(trigger.kind.as_str())
        .bind(trigger.source.as_ref().map(Slug::as_str))
        .bind(trigger.event_type.as_ref().map(sagat_domain::workflow::EventType::as_str))
        .bind(trigger.filter.as_ref().map(sagat_domain::workflow::Expression::as_str))
        .bind(trigger.schedule.as_ref().map(sagat_domain::triggers::CronSpec::expression))
        .bind(trigger.schedule.as_ref().map(sagat_domain::triggers::CronSpec::timezone))
        .execute(&mut **tx)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

async fn persist_declared_operations(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    version: WorkflowVersionId,
    state: &PublishedDeclaredState,
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    for operation in state.operations() {
        sqlx::query(
            "INSERT INTO declared_operation_definitions \
             (tenant_id, namespace_id, workflow_version_id, operation_kind, operation_name, \
              declaration_digest, state_schema_digest, result_schema_digest, disclosure, \
              created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(version.as_uuid())
        .bind(operation.kind().as_str())
        .bind(operation.name().as_str())
        .bind(operation.declaration_digest().as_bytes().as_slice())
        .bind(state.state_schema_digest().as_bytes().as_slice())
        .bind(operation.result_schema_digest().as_bytes().as_slice())
        .bind(operation.disclosure().as_str())
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

pub(crate) async fn verify_declared_operations(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    version: WorkflowVersionId,
    state: Option<&PublishedDeclaredState>,
) -> Result<(), StoreError> {
    let Some(state) = state else {
        return Ok(());
    };
    let expected = state.operations().count();
    let actual: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM declared_operation_definitions \
         WHERE tenant_id = $1 AND namespace_id = $2 AND workflow_version_id = $3",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(version.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(store_error)?;
    if usize::try_from(actual).ok() != Some(expected) {
        return Err(StoreError::Internal {
            detail: "published declared-operation metadata is incomplete".to_owned(),
        });
    }
    for operation in state.operations() {
        let matches: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM declared_operation_definitions \
             WHERE tenant_id = $1 AND namespace_id = $2 AND workflow_version_id = $3 \
               AND operation_kind = $4 AND operation_name = $5 \
               AND declaration_digest = $6 AND state_schema_digest = $7 \
               AND result_schema_digest = $8 AND disclosure = $9)",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(version.as_uuid())
        .bind(operation.kind().as_str())
        .bind(operation.name().as_str())
        .bind(operation.declaration_digest().as_bytes().as_slice())
        .bind(state.state_schema_digest().as_bytes().as_slice())
        .bind(operation.result_schema_digest().as_bytes().as_slice())
        .bind(operation.disclosure().as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(store_error)?;
        if !matches {
            return Err(StoreError::Internal {
                detail: "published declared-operation metadata does not match its version"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

impl DefinitionStore for PgDefinitionStore {
    async fn check_database_access(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        definition: &WorkflowDefinition,
    ) -> Result<(), StoreError> {
        if !definition.nodes().any(|n| n.database_operation().is_some()) {
            return Ok(());
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        crate::database::admit_definition(
            &mut tx,
            principal.tenant_id(),
            namespace,
            &[principal.subject().as_str()],
            definition,
            None,
            None,
        )
        .await?;
        tx.commit().await.map_err(store_error)
    }
    async fn find_namespace_by_slug(
        &self,
        tenant: TenantId,
        slug: &Slug,
    ) -> Result<Option<Namespace>, StoreError> {
        sqlx::query(
            "SELECT id, tenant_id, workspace_id, slug, environment, created_at \
             FROM namespaces WHERE tenant_id = $1 AND slug = $2",
        )
        .bind(tenant.as_uuid())
        .bind(slug.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(|row| namespace_from_row(&row))
        .transpose()
    }

    async fn create_workflow(
        &self,
        workflow: &Workflow,
        draft: &[u8],
        audit: &AuditRecord,
        idempotency: &Idempotency,
        max_in_namespace: u64,
    ) -> Result<Applied<()>, StoreError> {
        let draft = parse_json(draft)?;
        let hash = hash(&idempotency.fingerprint);
        // Two attempts: the second only runs when a concurrent request with the same key committed
        // between our lookup and our insert, in which case the lookup now finds it.
        for _ in 0..2 {
            let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
            if let Some(resource) =
                replay(&mut tx, workflow.tenant_id(), idempotency, &hash).await?
            {
                return Ok(Applied::Replayed(resource));
            }
            // After the replay check, so repeating a request that already succeeded never counts
            // against the quota; before the insert, so the count and the insert are one decision.
            ensure_room_for_workflow(
                &mut tx,
                workflow.tenant_id(),
                workflow.namespace_id(),
                max_in_namespace,
            )
            .await?;
            sqlx::query(
                "INSERT INTO workflows \
                 (id, tenant_id, namespace_id, name, draft_revision, draft, active_version_id, \
                  created_at, updated_at, draft_edited_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, $8, $9)",
            )
            .bind(workflow.id().as_uuid())
            .bind(workflow.tenant_id().as_uuid())
            .bind(workflow.namespace_id().as_uuid())
            .bind(workflow.name().as_str())
            .bind(to_i64(workflow.draft_revision())?)
            .bind(&draft)
            .bind(workflow.created_at())
            .bind(workflow.updated_at())
            .bind(audit.actor().as_str())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            insert_audit(&mut tx, audit).await?;
            if remember(
                &mut tx,
                workflow.tenant_id(),
                idempotency,
                &hash,
                &workflow.id().to_string(),
            )
            .await?
            {
                tx.commit().await.map_err(store_error)?;
                return Ok(Applied::Done(()));
            }
        }
        Err(StoreError::Unavailable {
            detail: "idempotency record kept changing under concurrent requests".into(),
        })
    }

    async fn get_workflow(
        &self,
        tenant: TenantId,
        id: WorkflowId,
    ) -> Result<Option<WorkflowRecord>, StoreError> {
        fetch_workflow(&self.pool, tenant, id).await
    }

    async fn find_workflow_by_name(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &Slug,
    ) -> Result<Option<WorkflowRecord>, StoreError> {
        sqlx::query(select_workflow!(
            "WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.name = $3"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(name.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .as_ref()
        .map(workflow_from_row)
        .transpose()
    }

    async fn list_workflows(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<WorkflowId>,
        limit: u32,
    ) -> Result<Vec<WorkflowRecord>, StoreError> {
        sqlx::query(select_workflow!(
            "WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND ($3::uuid IS NULL OR w.id > $3) \
             ORDER BY w.id LIMIT $4"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(after.map(WorkflowId::as_uuid))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(workflow_from_row)
        .collect()
    }

    async fn update_draft(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        expected_revision: u64,
        draft: &[u8],
        audit: &AuditRecord,
        now: OffsetDateTime,
    ) -> Result<Option<WorkflowRecord>, StoreError> {
        let draft = parse_json(draft)?;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // The check and the write are one statement: no other editor can slip in between.
        let updated = sqlx::query(
            "UPDATE workflows SET draft = $4, draft_revision = draft_revision + 1, updated_at = $5, \
                    draft_edited_by = $6 \
             WHERE tenant_id = $1 AND id = $2 AND NOT internal AND draft_revision = $3",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .bind(to_i64(expected_revision)?)
        .bind(&draft)
        .bind(now)
        .bind(audit.actor().as_str())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if updated == 0 {
            let current: Option<i64> = sqlx::query_scalar(
                "SELECT draft_revision FROM workflows WHERE tenant_id = $1 AND id = $2 AND NOT internal",
            )
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            return match current {
                None => Ok(None),
                Some(current) => Err(StoreError::RevisionConflict {
                    current: u64::try_from(current).map_err(|_| corrupt("draft revision"))?,
                }),
            };
        }
        // What was asked to be approved is not what the draft is now.
        let superseded: Vec<Uuid> = sqlx::query_scalar(
            "UPDATE publication_requests SET state = 'SUPERSEDED', updated_at = $3 \
             WHERE tenant_id = $1 AND workflow_id = $2 AND state IN ('PENDING', 'APPROVED') \
             RETURNING id",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .bind(now)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        for request in superseded {
            let ended = audit.follow_up(
                sagat_domain::audit::AuditAction::PublicationSuperseded,
                sagat_domain::audit::ResourceType::PublicationRequest,
                request.to_string(),
                format!(
                    "the draft was saved again at revision {}",
                    expected_revision + 1
                ),
            );
            insert_audit(&mut tx, &ended).await?;
        }
        let record = fetch_workflow(&mut *tx, tenant, id).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(record)
    }

    async fn publish(
        &self,
        principal: &Principal,
        id: WorkflowId,
        draft_revision: u64,
        version_id: WorkflowVersionId,
        digest: Digest,
        document: &[u8],
        declared_state: Option<&PublishedDeclaredState>,
        definition: &WorkflowDefinition,
        release_notes: Option<&str>,
        plans: &[SagaPlan],
        triggers: &[TriggerRegistration],
        context: &PublishContext,
        audit: &AuditRecord,
        idempotency: &Idempotency,
        now: OffsetDateTime,
    ) -> Result<Option<Applied<(WorkflowVersion, bool)>>, StoreError> {
        let tenant = principal.tenant_id();
        let hash = hash(&idempotency.fingerprint);
        for _ in 0..2 {
            let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
            // The row lock serialises publishers of one workflow, so version numbers are gapless.
            let Some(locked) = sqlx::query(
                "SELECT id, tenant_id, namespace_id, name, draft_revision, active_version_id, \
                        created_at, updated_at \
                 FROM workflows WHERE tenant_id = $1 AND id = $2 AND NOT internal FOR UPDATE",
            )
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?
            else {
                return Ok(None);
            };
            let workflow = bare_workflow(&locked)?;
            crate::database::admit_definition(
                &mut tx,
                tenant,
                workflow.namespace_id(),
                &[principal.subject().as_str()],
                definition,
                None,
                None,
            )
            .await?;
            // The same publication, made by a request that committed while this one waited for the
            // lock, is this one: answered as it was, whatever it left behind (a request that is
            // published now, a draft that has moved on).
            if let Some(resource) = replay(&mut tx, tenant, idempotency, &hash).await? {
                return Ok(Some(Applied::Replayed(resource)));
            }
            if workflow.draft_revision() != draft_revision {
                return Err(StoreError::RevisionConflict {
                    current: workflow.draft_revision(),
                });
            }
            let existing = sqlx::query(
                "SELECT id, version_number, published_by, published_at FROM workflow_versions \
                 WHERE tenant_id = $1 AND workflow_id = $2 AND digest = $3",
            )
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .bind(digest.to_hex())
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
            let (version, created) = if let Some(row) = existing {
                (version_from_parts(&row, tenant, id, digest)?, false)
            } else {
                let next: i32 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(version_number), 0) + 1 FROM workflow_versions \
                     WHERE tenant_id = $1 AND workflow_id = $2",
                )
                .bind(tenant.as_uuid())
                .bind(id.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
                let number = u32::try_from(next).map_err(|_| corrupt("version number"))?;
                let version =
                    WorkflowVersion::publish(principal, &workflow, version_id, number, digest, now)
                        .map_err(|_| StoreError::MissingReference)?;
                sqlx::query(
                    "INSERT INTO workflow_versions \
                     (id, tenant_id, workflow_id, version_number, digest, document, release_notes, \
                      published_by, published_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                )
                .bind(version.id().as_uuid())
                .bind(tenant.as_uuid())
                .bind(id.as_uuid())
                .bind(next)
                .bind(digest.to_hex())
                .bind(document)
                .bind(release_notes)
                .bind(version.published_by().as_str())
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                // The compensation plan of every saga of the version, stored once with it.
                for plan in plans {
                    insert_saga_plan(&mut tx, tenant, version.id(), plan).await?;
                }
                if let Some(state) = declared_state {
                    persist_declared_operations(
                        &mut tx,
                        tenant,
                        workflow.namespace_id(),
                        version.id(),
                        state,
                        now,
                    )
                    .await?;
                }
                (version, true)
            };
            verify_declared_operations(
                &mut tx,
                tenant,
                workflow.namespace_id(),
                version.id(),
                declared_state,
            )
            .await?;
            crate::versioning::record_publication(
                &mut tx,
                &crate::versioning::Published {
                    tenant,
                    workflow: id,
                    previous_active: workflow.active_version(),
                    version: version.id(),
                    created,
                    draft_revision,
                    digest,
                    context,
                    actor: principal.subject().as_str(),
                    now,
                },
            )
            .await?;
            sqlx::query(
                "UPDATE workflows SET active_version_id = $3, updated_at = $4 \
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .bind(version.id().as_uuid())
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            register_triggers(&mut tx, &workflow, version.id(), triggers).await?;
            insert_audit(&mut tx, audit).await?;
            if remember(
                &mut tx,
                tenant,
                idempotency,
                &hash,
                &version.number().to_string(),
            )
            .await?
            {
                tx.commit().await.map_err(store_error)?;
                return Ok(Some(Applied::Done((version, created))));
            }
        }
        Err(StoreError::Unavailable {
            detail: "idempotency record kept changing under concurrent requests".into(),
        })
    }

    async fn publication_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Option<sagat_domain::versioning::ApprovalPolicy>, StoreError> {
        crate::versioning::effective_policy(&self.pool, tenant, namespace).await
    }

    async fn get_version(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        number: u32,
    ) -> Result<Option<VersionRecord>, StoreError> {
        sqlx::query(VERSION_SELECT_ONE)
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .bind(i64::from(number))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(|row| version_record(&row))
            .transpose()
    }

    async fn compensation_plans(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
    ) -> Result<Vec<StoredPlan>, StoreError> {
        let plans = sqlx::query(
            "SELECT saga_node, on_compensation_failure, cancel_policy, irreversible \
             FROM saga_plans WHERE tenant_id = $1 AND workflow_version_id = $2 ORDER BY saga_node",
        )
        .bind(tenant.as_uuid())
        .bind(version.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        let entries = sqlx::query(
            "SELECT saga_node, position, node, compensation_id, compensation_activity, after \
             FROM saga_plan_entries WHERE tenant_id = $1 AND workflow_version_id = $2 \
             ORDER BY saga_node, position",
        )
        .bind(tenant.as_uuid())
        .bind(version.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        let node = |text: &str| NodeId::parse(text).map_err(|_| corrupt("plan node"));
        plans
            .iter()
            .map(|row| {
                let saga: String = col(row, "saga_node")?;
                let failure: String = col(row, "on_compensation_failure")?;
                let cancel: String = col(row, "cancel_policy")?;
                let irreversible: Vec<String> = col(row, "irreversible")?;
                let mine = entries
                    .iter()
                    .filter(|e| col::<String>(e, "saga_node").is_ok_and(|s| s == saga))
                    .map(|e| {
                        let after: Vec<String> = col(e, "after")?;
                        let activity: String = col(e, "compensation_activity")?;
                        Ok(StoredPlanEntry {
                            node: node(&col::<String>(e, "node")?)?,
                            compensation_id: col(e, "compensation_id")?,
                            activity: ActivityName::parse(&activity)
                                .map_err(|_| corrupt("plan activity"))?,
                            after: after.iter().map(|n| node(n)).collect::<Result<_, _>>()?,
                        })
                    })
                    .collect::<Result<Vec<_>, StoreError>>()?;
                Ok(StoredPlan {
                    saga: node(&saga)?,
                    on_compensation_failure: failure
                        .parse()
                        .map_err(|_| corrupt("compensation failure policy"))?,
                    cancel_policy: cancel.parse().map_err(|_| corrupt("cancel policy"))?,
                    irreversible: irreversible
                        .iter()
                        .map(|n| node(n))
                        .collect::<Result<_, _>>()?,
                    entries: mine,
                })
            })
            .collect()
    }

    async fn get_version_by_id(
        &self,
        tenant: TenantId,
        id: WorkflowVersionId,
    ) -> Result<Option<VersionRecord>, StoreError> {
        sqlx::query(
            "SELECT id, tenant_id, workflow_id, version_number, digest, document, release_notes, \
                    published_by, published_at \
             FROM workflow_versions WHERE tenant_id = $1 AND id = $2 AND workflow_id IN (SELECT id FROM workflows WHERE tenant_id=$1 AND NOT internal)",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(|row| version_record(&row))
        .transpose()
    }

    async fn list_versions(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        after: Option<u32>,
        limit: u32,
    ) -> Result<Vec<VersionRecord>, StoreError> {
        sqlx::query(
            "SELECT id, tenant_id, workflow_id, version_number, digest, document, release_notes, \
                    published_by, published_at \
             FROM workflow_versions \
             WHERE tenant_id = $1 AND workflow_id = $2 AND workflow_id IN (SELECT id FROM workflows WHERE tenant_id=$1 AND NOT internal) AND version_number > $3 \
             ORDER BY version_number LIMIT $4",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .bind(after.map_or(0, i64::from))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(version_record)
        .collect()
    }

    async fn workflow_exists_outside(
        &self,
        tenant: TenantId,
        id: WorkflowId,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM workflows WHERE id = $1 AND tenant_id <> $2)",
        )
        .bind(id.as_uuid())
        .bind(tenant.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        use sagat_application::identity::IdentityStore;
        self.identity.record_denial(audit).await
    }
}

const VERSION_SELECT_ONE: &str = "SELECT id, tenant_id, workflow_id, version_number, digest, \
     document, release_notes, published_by, published_at \
     FROM workflow_versions WHERE tenant_id = $1 AND workflow_id = $2 AND workflow_id IN (SELECT id FROM workflows WHERE tenant_id=$1 AND NOT internal) AND version_number = $3";

// ----------------------------------------------------------------------------- idempotency

pub(crate) fn hash(fingerprint: &[u8]) -> Vec<u8> {
    Sha256::digest(fingerprint).to_vec()
}

/// Looks for an earlier request with the same key. `Some(resource)` is a replay; a different
/// request under the same key is an error.
pub(crate) async fn replay(
    conn: &mut PgConnection,
    tenant: TenantId,
    idempotency: &Idempotency,
    hash: &[u8],
) -> Result<Option<String>, StoreError> {
    let row = sqlx::query(
        "SELECT request_hash, resource_id FROM command_dedupe \
         WHERE tenant_id = $1 AND scope = $2 AND idempotency_key = $3",
    )
    .bind(tenant.as_uuid())
    .bind(&idempotency.scope)
    .bind(&idempotency.key)
    .fetch_optional(conn)
    .await
    .map_err(store_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let stored: Vec<u8> = col(&row, "request_hash")?;
    if stored == hash {
        Ok(Some(col(&row, "resource_id")?))
    } else {
        Err(StoreError::IdempotencyKeyReused)
    }
}

/// Records the key. `false` means a concurrent request with the same key committed first; the
/// caller rolls back and starts over, which turns this request into a replay.
pub(crate) async fn remember(
    conn: &mut PgConnection,
    tenant: TenantId,
    idempotency: &Idempotency,
    hash: &[u8],
    resource: &str,
) -> Result<bool, StoreError> {
    let inserted = sqlx::query(
        "INSERT INTO command_dedupe (tenant_id, scope, idempotency_key, request_hash, resource_id) \
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .bind(&idempotency.scope)
    .bind(&idempotency.key)
    .bind(hash)
    .bind(resource)
    .execute(conn)
    .await
    .map_err(store_error)?
    .rows_affected();
    Ok(inserted == 1)
}

// ----------------------------------------------------------------------------- decoding

fn to_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Internal {
        detail: "a revision does not fit the column".into(),
    })
}

fn parse_json(bytes: &[u8]) -> Result<serde_json::Value, StoreError> {
    serde_json::from_slice(bytes).map_err(|_| StoreError::Internal {
        detail: "a draft that passed validation is not JSON".into(),
    })
}

async fn fetch_workflow<'e, E>(
    executor: E,
    tenant: TenantId,
    id: WorkflowId,
) -> Result<Option<WorkflowRecord>, StoreError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(select_workflow!("WHERE w.tenant_id = $1 AND w.id = $2"))
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(executor)
        .await
        .map_err(store_error)?
        .as_ref()
        .map(workflow_from_row)
        .transpose()
}

pub(crate) fn bare_workflow(row: &PgRow) -> Result<Workflow, StoreError> {
    let name: String = col(row, "name")?;
    let revision: i64 = col(row, "draft_revision")?;
    Ok(Workflow::restore(
        WorkflowId::new(col::<Uuid>(row, "id")?),
        TenantId::new(col::<Uuid>(row, "tenant_id")?),
        NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        Slug::parse(&name).map_err(|_| corrupt("workflow name"))?,
        u64::try_from(revision).map_err(|_| corrupt("draft revision"))?,
        col::<Option<Uuid>>(row, "active_version_id")?.map(WorkflowVersionId::new),
        col::<OffsetDateTime>(row, "created_at")?,
        col::<OffsetDateTime>(row, "updated_at")?,
    ))
}

fn workflow_from_row(row: &PgRow) -> Result<WorkflowRecord, StoreError> {
    let workflow = bare_workflow(row)?;
    let draft: serde_json::Value = col(row, "draft")?;
    let active = match col::<Option<Uuid>>(row, "v_id")? {
        None => None,
        Some(version) => {
            let digest: String = col(row, "v_digest")?;
            let published_by: String = col(row, "v_published_by")?;
            let number: i32 = col(row, "v_number")?;
            Some(WorkflowVersion::restore(
                WorkflowVersionId::new(version),
                workflow.tenant_id(),
                workflow.id(),
                u32::try_from(number).map_err(|_| corrupt("version number"))?,
                Digest::from_hex(&digest).ok_or_else(|| corrupt("digest"))?,
                Subject::parse(&published_by).map_err(|_| corrupt("subject"))?,
                col::<OffsetDateTime>(row, "v_published_at")?,
            ))
        }
    };
    Ok(WorkflowRecord {
        workflow,
        draft: serde_json::to_vec(&draft).map_err(|_| corrupt("draft"))?,
        active,
    })
}

fn version_from_parts(
    row: &PgRow,
    tenant: TenantId,
    workflow: WorkflowId,
    digest: Digest,
) -> Result<WorkflowVersion, StoreError> {
    let number: i32 = col(row, "version_number")?;
    let published_by: String = col(row, "published_by")?;
    Ok(WorkflowVersion::restore(
        WorkflowVersionId::new(col::<Uuid>(row, "id")?),
        tenant,
        workflow,
        u32::try_from(number).map_err(|_| corrupt("version number"))?,
        digest,
        Subject::parse(&published_by).map_err(|_| corrupt("subject"))?,
        col::<OffsetDateTime>(row, "published_at")?,
    ))
}

fn version_record(row: &PgRow) -> Result<VersionRecord, StoreError> {
    let digest: String = col(row, "digest")?;
    let digest = Digest::from_hex(&digest).ok_or_else(|| corrupt("digest"))?;
    let version = version_from_parts(
        row,
        TenantId::new(col::<Uuid>(row, "tenant_id")?),
        WorkflowId::new(col::<Uuid>(row, "workflow_id")?),
        digest,
    )?;
    Ok(VersionRecord {
        version,
        document: col(row, "document")?,
        release_notes: col(row, "release_notes")?,
    })
}

/// Stores one saga's compensation plan next to the version it belongs to. A plan is a pure function
/// of the definition, so this is a record for those who read it (the API, Studio, an audit), not the
/// source the engine plans from; a test compares the two.
async fn insert_saga_plan(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    version: WorkflowVersionId,
    plan: &SagaPlan,
) -> Result<(), StoreError> {
    let irreversible: Vec<String> = plan
        .irreversible
        .iter()
        .map(|n| n.as_str().to_owned())
        .collect();
    sqlx::query(
        "INSERT INTO saga_plans \
         (tenant_id, workflow_version_id, saga_node, on_compensation_failure, cancel_policy, irreversible) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(tenant.as_uuid())
    .bind(version.as_uuid())
    .bind(plan.saga.as_str())
    .bind(plan.on_compensation_failure.as_str())
    .bind(plan.cancel_policy.as_str())
    .bind(&irreversible)
    .execute(&mut *tx)
    .await
    .map_err(store_error)?;
    for (position, entry) in plan.entries.iter().enumerate() {
        let after: Vec<String> = entry.after.iter().map(|n| n.as_str().to_owned()).collect();
        sqlx::query(
            "INSERT INTO saga_plan_entries \
             (tenant_id, workflow_version_id, saga_node, position, node, compensation_id, \
              compensation_activity, after) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(tenant.as_uuid())
        .bind(version.as_uuid())
        .bind(plan.saga.as_str())
        .bind(i32::try_from(position).map_err(|_| corrupt("plan position"))?)
        .bind(entry.node.as_str())
        .bind(compensation_id(version, &entry.node))
        .bind(entry.activity.as_str())
        .bind(&after)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

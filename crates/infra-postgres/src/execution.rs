//! PostgreSQL implementation of the run store: one locked transaction per command.
//!
//! [`PgRunStore::begin`] takes the run's row lock (`FOR UPDATE`, or `SKIP LOCKED` for workers),
//! loads the immutable definition and the current state from the projection rows, and hands both to
//! the engine. [`PgRunTx::persist`] then writes, in that same transaction: the journal entries, the
//! step rows, the payloads, the outbox, the run row, the audit record and the idempotency record.
//! Nothing outside the transaction ever sees a half-applied command, and a crash before `COMMIT`
//! leaves the previous state intact (`FR-002-003`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use sagat_application::definitions::{
    Applied, DefinitionStore, Idempotency, VersionRecord, WorkflowRecord,
};
use sagat_application::error::StoreError;
use sagat_application::execution::{
    DefinitionCache, Effects, ParentLink, RunData, RunInfo, RunRecord, RunTx, step_id,
};
use sagat_application::runs::{
    GraphSnapshot, JournalEntry, Lock, NewRun, RunFilter, RunStore, StepView,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersion, WorkflowVersionId};
use sagat_domain::execution::{
    CorrelationKey, FailureCode, RunEvent, RunId, RunState, RunStatus, StepState, StepStatus,
    StopReason, WaitingOn,
};
use sagat_domain::identity::{Namespace, NamespaceId, Principal, Slug, TenantId};
use sagat_domain::workflow::{Digest, NodeId, NodeKind, WorkflowDefinition};
use sha2::{Digest as _, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::definitions::{PgDefinitionStore, hash, remember, replay};
use crate::event_codec::{decode, encode};
use crate::identity::{PgIdentityStore, col, corrupt, insert_audit, store_error};

/// PostgreSQL-backed run store.
#[derive(Debug, Clone)]
pub struct PgRunStore {
    pub(crate) pool: PgPool,
    pub(crate) cache: DefinitionCache,
    identity: PgIdentityStore,
    definitions: PgDefinitionStore,
}

impl PgRunStore {
    #[must_use]
    pub fn new(pool: PgPool, cache: DefinitionCache) -> Self {
        Self {
            identity: PgIdentityStore::new(pool.clone()),
            definitions: PgDefinitionStore::new(pool.clone()),
            pool,
            cache,
        }
    }

    /// Whether the execution plane can work: the server answers and every table it reads or writes
    /// exists, so an engine pointed at an unmigrated database is never reported ready.
    ///
    /// # Errors
    /// [`StoreError::Unavailable`] naming how many tables are missing (never their contents).
    pub async fn ready(&self) -> Result<(), StoreError> {
        sagat_application::identity::IdentityStore::ping(&self.identity).await?;
        let missing: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM unnest(ARRAY['workflows', 'workflow_versions', 'runs', 'steps', \
                 'run_events', 'payloads', 'event_outbox', 'worker_sessions', 'command_dedupe']) AS t(name) \
             WHERE to_regclass('public.' || t.name) IS NULL",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)?;
        if missing == 0 {
            Ok(())
        } else {
            Err(StoreError::Unavailable {
                detail: format!(
                    "the execution schema has not been migrated ({missing} tables missing)"
                ),
            })
        }
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The definition of `version`, from the cache or parsed from the stored document.
    async fn definition(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
    ) -> Result<Arc<WorkflowDefinition>, StoreError> {
        if let Some(def) = self.cache.get(version) {
            return Ok(def);
        }
        let document: Vec<u8> = sqlx::query_scalar(
            "SELECT document FROM workflow_versions WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(version.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .ok_or_else(|| corrupt("workflow version reference"))?;
        self.cache
            .get_or_parse(version, &document)
            .map_err(|error| {
                tracing::error!(%error, %version, "a stored workflow version no longer parses");
                StoreError::Internal {
                    detail: "a stored workflow version no longer parses".into(),
                }
            })
    }

    /// Verifies that the stored projections equal a replay of the stored journal. Used by tests and
    /// by operators investigating a suspected inconsistency; never needed on the hot path.
    ///
    /// # Errors
    /// A description of the first difference found.
    pub async fn verify_run(&self, tenant: TenantId, run: RunId) -> Result<(), String> {
        let rows = sqlx::query(
            "SELECT seq, event_type, node, detail FROM run_events \
             WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        let mut events = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let seq: i64 = col(row, "seq").map_err(|e| e.to_string())?;
            if seq != i64::try_from(index + 1).map_err(|e| e.to_string())? {
                return Err(format!("the journal has a gap before sequence {seq}"));
            }
            let event_type: String = col(row, "event_type").map_err(|e| e.to_string())?;
            let node: Option<String> = col(row, "node").map_err(|e| e.to_string())?;
            let detail: serde_json::Value = col(row, "detail").map_err(|e| e.to_string())?;
            events.push(
                decode(&event_type, node.as_deref(), &detail)
                    .ok_or_else(|| format!("event {seq} ('{event_type}') cannot be decoded"))?,
            );
        }
        let replayed = RunState::replay(run, &events).map_err(|e| format!("replay failed: {e}"))?;
        let mut tx = self
            .begin(tenant, run, Lock::Wait)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("the run does not exist")?;
        let stored = tx.parts().1.clone();
        if replayed == stored {
            Ok(())
        } else {
            Err(format!(
                "projection differs from the journal:\n  replayed: {replayed:?}\n  stored:   {stored:?}"
            ))
        }
    }
}

impl RunStore for PgRunStore {
    type Tx = PgRunTx;

    async fn create(&self, new: NewRun) -> Result<Applied<PgRunTx>, StoreError> {
        let tenant = new.info.tenant_id;
        let hash = hash(&new.idempotency.fingerprint);
        let input_digest = Sha256::digest(&new.input).to_vec();
        for _ in 0..2 {
            let mut tx = self.pool.begin().await.map_err(store_error)?;
            if let Some(existing) = replay(&mut tx, tenant, &new.idempotency, &hash).await? {
                return Ok(Applied::Replayed(existing));
            }
            sqlx::query(
                "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(tenant.as_uuid())
            .bind(&input_digest)
            .bind(&new.input)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
            let info = &new.info;
            if let Some(parent) = &info.parent {
                claim_reservation(&mut tx, tenant, parent, info.run_id).await?;
            }
            sqlx::query(
                "INSERT INTO runs (id, tenant_id, namespace_id, workflow_id, workflow_version_id, \
                 status, run_version, paused, input_digest, correlation_id, started_by, trace_id, \
                 deadline_at, created_at, updated_at, parent_run_id, parent_node, cancel_policy, \
                 depth) \
                 VALUES ($1, $2, $3, $4, $5, 'RUNNING', 0, false, $6, $7, $8, $9, $10, $11, $11, \
                         $12, $13, $14, $15)",
            )
            .bind(info.run_id.as_uuid())
            .bind(tenant.as_uuid())
            .bind(info.namespace_id.as_uuid())
            .bind(info.workflow_id.as_uuid())
            .bind(info.version_id.as_uuid())
            .bind(&input_digest)
            .bind(info.correlation_id.as_deref())
            .bind(&info.started_by)
            .bind(info.trace_id.as_deref())
            .bind(info.deadline_at)
            .bind(now)
            .bind(info.parent.as_ref().map(|p| p.run.as_uuid()))
            .bind(info.parent.as_ref().map(|p| p.node.as_str().to_owned()))
            .bind(info.parent.as_ref().map(|p| p.cancel_policy.as_str()))
            .bind(i16::from(info.depth))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            insert_audit(&mut tx, &new.audit).await?;
            if remember(
                &mut tx,
                tenant,
                &new.idempotency,
                &hash,
                &info.run_id.to_string(),
            )
            .await?
            {
                let mut info = info.clone();
                info.created_at = now;
                return Ok(Applied::Done(PgRunTx {
                    tx,
                    info,
                    now,
                    def: Arc::clone(&new.definition),
                    state: RunState::new(new.info.run_id),
                    persisted: 0,
                    data: Some(RunData {
                        run_id: Some(new.info.run_id),
                        input: new.input.clone(),
                        outputs: BTreeMap::new(),
                    }),
                    input_digest,
                    stored: BTreeSet::new(),
                }));
            }
        }
        Err(StoreError::Unavailable {
            detail: "idempotency record kept changing under concurrent requests".into(),
        })
    }

    async fn begin(
        &self,
        tenant: TenantId,
        run: RunId,
        lock: Lock,
    ) -> Result<Option<PgRunTx>, StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        let row = match lock {
            Lock::Wait => sqlx::query(RUN_FOR_UPDATE)
                .bind(tenant.as_uuid())
                .bind(run.as_uuid()),
            Lock::SkipLocked => sqlx::query(RUN_FOR_UPDATE_SKIP_LOCKED)
                .bind(tenant.as_uuid())
                .bind(run.as_uuid()),
        }
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let record = run_record(&row)?;
        let paused: bool = col(&row, "paused")?;
        let stop_code: Option<String> = col(&row, "stop_code")?;
        let now: OffsetDateTime = col(&row, "db_now")?;
        let input_digest: Vec<u8> = col(&row, "input_digest")?;
        let step_rows = sqlx::query(
            "SELECT node, status, attempt, generation, cancel_requested, routed, condition, \
                    timed_out, failure_code, next_attempt_at, waiting_kind, due_at, correlation_key, \
                    child_run_id \
             FROM steps WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut steps = BTreeMap::new();
        let mut stored = BTreeSet::new();
        for row in &step_rows {
            let node: String = col(row, "node")?;
            let node = NodeId::parse(&node).map_err(|_| corrupt("step node"))?;
            stored.insert(node.clone());
            steps.insert(node, step_state(row)?);
        }
        let stop = match record.stop_reason {
            None => None,
            Some(reason) => Some((
                reason,
                stop_code
                    .map(|c| FailureCode::parse(&c))
                    .transpose()
                    .map_err(|_| corrupt("stop code"))?,
            )),
        };
        let state = RunState::restore(run, record.status, stop, paused, steps, record.version);
        let def = self.definition(tenant, record.info.version_id).await?;
        Ok(Some(PgRunTx {
            tx,
            info: record.info,
            now,
            def,
            persisted: state.version(),
            state,
            data: None,
            input_digest,
            stored,
        }))
    }

    async fn get(&self, tenant: TenantId, run: RunId) -> Result<Option<RunRecord>, StoreError> {
        sqlx::query(RUN_SELECT)
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .as_ref()
            .map(run_record)
            .transpose()
    }

    async fn list(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        filter: &RunFilter,
        before: Option<RunId>,
        limit: u32,
    ) -> Result<Vec<RunRecord>, StoreError> {
        sqlx::query(
            "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, run_version, \
                    stop_reason, paused, correlation_id, started_by, trace_id, deadline_at, \
                    created_at, updated_at, finished_at, parent_run_id, parent_node, cancel_policy, \
                    depth \
             FROM runs \
             WHERE tenant_id = $1 AND namespace_id = $2 \
               AND ($3::uuid IS NULL OR id < $3) \
               AND ($4::uuid IS NULL OR workflow_id = $4) \
               AND ($5::text IS NULL OR status = $5) \
             ORDER BY id DESC LIMIT $6",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(before.map(RunId::as_uuid))
        .bind(filter.workflow.map(WorkflowId::as_uuid))
        .bind(filter.status.map(RunStatus::as_str))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(run_record)
        .collect()
    }

    async fn journal(
        &self,
        tenant: TenantId,
        run: RunId,
        after: Option<u64>,
        limit: u32,
    ) -> Result<Option<Vec<JournalEntry>>, StoreError> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM runs WHERE tenant_id = $1 AND id = $2)",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)?;
        if !exists {
            return Ok(None);
        }
        let rows = sqlx::query(
            "SELECT seq, event_type, node, attempt, actor, trace_id, occurred_at, \
                    encode(payload_digest, 'hex') AS payload_ref \
             FROM run_events WHERE tenant_id = $1 AND run_id = $2 AND seq > $3 \
             ORDER BY seq LIMIT $4",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(i64::try_from(after.unwrap_or(0)).unwrap_or(i64::MAX))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter()
            .map(|row| {
                let seq: i64 = col(row, "seq")?;
                let attempt: Option<i32> = col(row, "attempt")?;
                Ok(JournalEntry {
                    seq: u64::try_from(seq).map_err(|_| corrupt("sequence"))?,
                    event_type: col(row, "event_type")?,
                    node: col(row, "node")?,
                    attempt: attempt.and_then(|a| u32::try_from(a).ok()),
                    actor: col(row, "actor")?,
                    trace_id: col(row, "trace_id")?,
                    occurred_at: col(row, "occurred_at")?,
                    payload_ref: col(row, "payload_ref")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()
            .map(Some)
    }

    async fn graph(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<GraphSnapshot>, StoreError> {
        // One snapshot for the run row and its steps, so the graph is self-consistent.
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let Some(row) = sqlx::query(RUN_SELECT)
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        let record = run_record(&row)?;
        let step_rows = sqlx::query(
            "SELECT node, status, attempt, generation, cancel_requested, failure_code, \
                    next_attempt_at, due_at, waiting_kind, child_run_id \
             FROM steps WHERE tenant_id = $1 AND run_id = $2 ORDER BY node",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        let steps = step_rows
            .iter()
            .map(|row| {
                let node: String = col(row, "node")?;
                let status: String = col(row, "status")?;
                let attempt: i32 = col(row, "attempt")?;
                let generation: i64 = col(row, "generation")?;
                let waiting: Option<String> = col(row, "waiting_kind")?;
                Ok(StepView {
                    node: NodeId::parse(&node).map_err(|_| corrupt("step node"))?,
                    status: status.parse().map_err(|_| corrupt("step status"))?,
                    attempt: u32::try_from(attempt).map_err(|_| corrupt("attempt"))?,
                    generation: u64::try_from(generation).map_err(|_| corrupt("generation"))?,
                    cancel_requested: col(row, "cancel_requested")?,
                    failure_code: col(row, "failure_code")?,
                    next_attempt_at: col(row, "next_attempt_at")?,
                    due_at: col(row, "due_at")?,
                    waiting_on: match waiting.as_deref() {
                        Some("TIMER") => Some("timer"),
                        Some("EVENT") => Some("event"),
                        Some("BRANCHES") => Some("branches"),
                        Some("CHILD") => Some("child"),
                        _ => None,
                    },
                    child_run: col::<Option<Uuid>>(row, "child_run_id")?.map(RunId::new),
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let definition = self.definition(tenant, record.info.version_id).await?;
        Ok(Some(GraphSnapshot {
            run: record,
            definition,
            steps,
        }))
    }

    async fn run_exists_outside(&self, tenant: TenantId, run: RunId) -> Result<bool, StoreError> {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM runs WHERE id = $1 AND tenant_id <> $2)")
            .bind(run.as_uuid())
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

/// Creating a child run: confirms, under the parent's row lock, that the parent step still waits
/// for exactly this run and has not created it yet, and marks it created.
///
/// Taking the parent's lock first is the order every writer of a run uses, so this cannot deadlock
/// with a command on the parent; and because the check and the creation share one transaction, a
/// child can never come into being for a step that was cancelled a moment earlier.
async fn claim_reservation(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    parent: &ParentLink,
    child: RunId,
) -> Result<(), StoreError> {
    let locked = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(parent.run.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    if locked.is_none() {
        return Err(StoreError::MissingReference);
    }
    let claimed = sqlx::query(
        "UPDATE steps SET child_started = true, child_input = NULL, updated_at = clock_timestamp() \
         WHERE tenant_id = $1 AND run_id = $2 AND node = $3 AND status = 'WAITING' \
           AND waiting_kind = 'CHILD' AND child_run_id = $4 AND child_started = false",
    )
    .bind(tenant.as_uuid())
    .bind(parent.run.as_uuid())
    .bind(parent.node.as_str())
    .bind(child.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(store_error)?
    .rows_affected();
    if claimed == 1 {
        Ok(())
    } else {
        // Cancelled, finished, or already created by someone else.
        Err(StoreError::MissingReference)
    }
}

const RUN_SELECT: &str = "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, \
     run_version, stop_reason, paused, correlation_id, started_by, trace_id, deadline_at, \
     created_at, updated_at, finished_at, parent_run_id, parent_node, cancel_policy, depth \
     FROM runs WHERE tenant_id = $1 AND id = $2";

const RUN_FOR_UPDATE: &str = "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id, \
     status, run_version, stop_reason, stop_code, paused, input_digest, correlation_id, started_by, \
     trace_id, deadline_at, created_at, updated_at, finished_at, parent_run_id, parent_node, \
     cancel_policy, depth, clock_timestamp() AS db_now \
     FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE";

const RUN_FOR_UPDATE_SKIP_LOCKED: &str = "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id, \
     status, run_version, stop_reason, stop_code, paused, input_digest, correlation_id, started_by, \
     trace_id, deadline_at, created_at, updated_at, finished_at, parent_run_id, parent_node, \
     cancel_policy, depth, clock_timestamp() AS db_now \
     FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE SKIP LOCKED";

fn parent_link(row: &PgRow) -> Result<Option<ParentLink>, StoreError> {
    let run: Option<Uuid> = col(row, "parent_run_id")?;
    let node: Option<String> = col(row, "parent_node")?;
    let policy: Option<String> = col(row, "cancel_policy")?;
    match (run, node, policy) {
        (None, None, None) => Ok(None),
        (Some(run), Some(node), Some(policy)) => Ok(Some(ParentLink {
            run: RunId::new(run),
            node: NodeId::parse(&node).map_err(|_| corrupt("parent node"))?,
            cancel_policy: policy.parse().map_err(|()| corrupt("cancel policy"))?,
        })),
        _ => Err(corrupt("parent link")),
    }
}

fn run_record(row: &PgRow) -> Result<RunRecord, StoreError> {
    let status: String = col(row, "status")?;
    let version: i64 = col(row, "run_version")?;
    let stop: Option<String> = col(row, "stop_reason")?;
    Ok(RunRecord {
        info: RunInfo {
            run_id: RunId::new(col::<Uuid>(row, "id")?),
            tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
            namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
            workflow_id: WorkflowId::new(col::<Uuid>(row, "workflow_id")?),
            version_id: WorkflowVersionId::new(col::<Uuid>(row, "workflow_version_id")?),
            started_by: col(row, "started_by")?,
            correlation_id: col(row, "correlation_id")?,
            trace_id: col(row, "trace_id")?,
            deadline_at: col(row, "deadline_at")?,
            created_at: col(row, "created_at")?,
            parent: parent_link(row)?,
            depth: u8::try_from(col::<i16>(row, "depth")?).map_err(|_| corrupt("run depth"))?,
        },
        status: status.parse().map_err(|_| corrupt("run status"))?,
        version: u64::try_from(version).map_err(|_| corrupt("run version"))?,
        stop_reason: match stop.as_deref() {
            None => None,
            Some("CANCELLED") => Some(StopReason::Cancelled),
            Some("TIMED_OUT") => Some(StopReason::TimedOut),
            Some("FAILED") => Some(StopReason::Failed),
            Some(_) => return Err(corrupt("stop reason")),
        },
        paused: col(row, "paused")?,
        updated_at: col(row, "updated_at")?,
        finished_at: col(row, "finished_at")?,
    })
}

fn step_state(row: &PgRow) -> Result<StepState, StoreError> {
    let status: String = col(row, "status")?;
    let attempt: i32 = col(row, "attempt")?;
    let generation: i64 = col(row, "generation")?;
    let waiting_kind: Option<String> = col(row, "waiting_kind")?;
    let due_at: Option<OffsetDateTime> = col(row, "due_at")?;
    let key: Option<String> = col(row, "correlation_key")?;
    let waiting = match (waiting_kind.as_deref(), due_at) {
        (None, _) => None,
        (Some("BRANCHES"), _) => Some(WaitingOn::Branches),
        (Some("CHILD"), _) => Some(WaitingOn::Child {
            run: RunId::new(
                col::<Option<Uuid>>(row, "child_run_id")?.ok_or_else(|| corrupt("child run"))?,
            ),
        }),
        (Some("TIMER"), Some(due_at)) => Some(WaitingOn::Timer { due_at }),
        (Some("EVENT"), Some(deadline)) => Some(WaitingOn::Event {
            correlation_key: CorrelationKey::parse(&key.ok_or_else(|| corrupt("correlation key"))?)
                .map_err(|_| corrupt("correlation key"))?,
            deadline,
        }),
        _ => return Err(corrupt("waiting step")),
    };
    let failure: Option<String> = col(row, "failure_code")?;
    Ok(StepState::restore(
        status.parse().map_err(|_| corrupt("step status"))?,
        u32::try_from(attempt).map_err(|_| corrupt("attempt"))?,
        u64::try_from(generation).map_err(|_| corrupt("generation"))?,
        col(row, "cancel_requested")?,
        col(row, "routed")?,
        col(row, "next_attempt_at")?,
        waiting,
        failure
            .map(|c| FailureCode::parse(&c))
            .transpose()
            .map_err(|_| corrupt("failure code"))?,
        col(row, "condition")?,
        col(row, "timed_out")?,
    ))
}

// ------------------------------------------------------------------------------ the transaction

/// One locked run inside an open database transaction.
pub struct PgRunTx {
    tx: Transaction<'static, Postgres>,
    info: RunInfo,
    now: OffsetDateTime,
    def: Arc<WorkflowDefinition>,
    state: RunState,
    /// Journal sequence already written; `persist` writes `persisted + 1 ..= state.version()`.
    persisted: u64,
    data: Option<RunData>,
    input_digest: Vec<u8>,
    /// Steps that already have a row. Every write to a run's steps happens under its row lock, so
    /// "insert or update" is decided here, not by `ON CONFLICT` (which checks constraints on the
    /// proposed row before it takes the update path).
    stored: BTreeSet<NodeId>,
}

impl std::fmt::Debug for PgRunTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgRunTx")
            .field("run", &self.info.run_id)
            .finish_non_exhaustive()
    }
}

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

impl RunTx for PgRunTx {
    fn info(&self) -> &RunInfo {
        &self.info
    }

    fn now(&self) -> OffsetDateTime {
        self.now
    }

    fn parts(&mut self) -> (&WorkflowDefinition, &mut RunState) {
        (&self.def, &mut self.state)
    }

    async fn data(&mut self) -> Result<&RunData, StoreError> {
        if self.data.is_none() {
            let tenant = self.info.tenant_id.as_uuid();
            let input: Vec<u8> = sqlx::query_scalar(
                "SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2",
            )
            .bind(tenant)
            .bind(&self.input_digest)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
            let rows = sqlx::query(
                "SELECT s.node, p.content FROM steps s \
                 JOIN payloads p ON p.tenant_id = s.tenant_id AND p.digest = s.output_digest \
                 WHERE s.tenant_id = $1 AND s.run_id = $2",
            )
            .bind(tenant)
            .bind(self.info.run_id.as_uuid())
            .fetch_all(&mut *self.tx)
            .await
            .map_err(store_error)?;
            let mut outputs = BTreeMap::new();
            for row in &rows {
                let node: String = col(row, "node")?;
                outputs.insert(
                    NodeId::parse(&node).map_err(|_| corrupt("step node"))?,
                    col::<Vec<u8>>(row, "content")?,
                );
            }
            self.data = Some(RunData {
                run_id: Some(self.info.run_id),
                input,
                outputs,
            });
        }
        self.data.as_ref().ok_or_else(|| StoreError::Internal {
            detail: "run data was not loaded".into(),
        })
    }

    async fn stage_outputs(&mut self, outputs: &[(NodeId, Vec<u8>)]) -> Result<(), StoreError> {
        self.data().await?;
        if let Some(data) = &mut self.data {
            for (node, bytes) in outputs {
                data.outputs.insert(node.clone(), bytes.clone());
            }
        }
        Ok(())
    }

    async fn lease_expiry(&mut self, node: &NodeId) -> Result<Option<OffsetDateTime>, StoreError> {
        sqlx::query_scalar(
            "SELECT lease_expires_at FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
             AND status IN ('LEASED', 'RUNNING')",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .fetch_optional(&mut *self.tx)
        .await
        .map(Option::flatten)
        .map_err(store_error)
    }

    async fn lease_matches(
        &mut self,
        node: &NodeId,
        token_hash: &[u8],
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
                            AND status IN ('LEASED', 'RUNNING') AND lease_token_hash = $4)",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .bind(token_hash)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn replay(&mut self, idempotency: &Idempotency) -> Result<Option<String>, StoreError> {
        replay(
            &mut self.tx,
            self.info.tenant_id,
            idempotency,
            &hash(&idempotency.fingerprint),
        )
        .await
    }

    async fn persist(&mut self, events: &[RunEvent], effects: Effects) -> Result<(), StoreError> {
        let version = self.state.version();
        let written = u64::try_from(events.len()).map_err(|_| corrupt("event count"))?;
        if version != self.persisted + written {
            return Err(StoreError::Internal {
                detail: "the in-memory state and the events to write diverged".into(),
            });
        }
        let tenant = self.info.tenant_id;
        let run = self.info.run_id;

        // 1. Outputs: content-addressed payloads, remembered per node.
        let mut output_digest: BTreeMap<&NodeId, Vec<u8>> = BTreeMap::new();
        for (node, bytes) in &effects.outputs {
            let digest = Sha256::digest(bytes).to_vec();
            sqlx::query(
                "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(tenant.as_uuid())
            .bind(&digest)
            .bind(bytes)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            output_digest.insert(node, digest);
            if let Some(data) = &mut self.data {
                data.outputs.insert(node.clone(), bytes.clone());
            }
        }

        // 2. The journal, and the nodes the events touched.
        let mut touched: BTreeSet<NodeId> = BTreeSet::new();
        let mut hinted: BTreeSet<NodeId> = BTreeSet::new();
        for (offset, event) in events.iter().enumerate() {
            let seq = i64::try_from(self.persisted + 1 + offset as u64)
                .map_err(|_| corrupt("sequence"))?;
            let encoded = encode(event);
            let payload = event
                .node()
                .filter(|_| {
                    matches!(
                        event,
                        RunEvent::StepCompleted { .. } | RunEvent::WaitSatisfied { .. }
                    )
                })
                .and_then(|n| output_digest.get(n));
            sqlx::query(
                "INSERT INTO run_events \
                 (tenant_id, run_id, seq, event_type, node, attempt, detail, payload_digest, actor, \
                  trace_id, occurred_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(seq)
            .bind(encoded.event_type)
            .bind(encoded.node.as_deref())
            .bind(encoded.attempt)
            .bind(&encoded.detail)
            .bind(payload.map(Vec::as_slice))
            .bind(effects.actor.label())
            .bind(effects.trace_id.as_deref().or(self.info.trace_id.as_deref()))
            .bind(self.now)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;

            if let Some(node) = event.node() {
                touched.insert(node.clone());
            }
            match event {
                RunEvent::RunStarted { entrypoint } => {
                    touched.insert(entrypoint.clone());
                }
                RunEvent::StepRouted { activated, .. } => touched.extend(activated.iter().cloned()),
                RunEvent::ParallelForked { branches, .. } => {
                    touched.extend(branches.iter().cloned())
                }
                RunEvent::StepReleased { node } | RunEvent::StepRetryDue { node } => {
                    hinted.insert(node.clone());
                    self.outbox_task_ready(node).await?;
                }
                RunEvent::ChildFinished { child, .. } => self.settle_child(*child, false).await?,
                RunEvent::StepCancelled { node } => self.release_child(node, true).await?,
                RunEvent::StepCancelRequested { node } => self.release_child(node, false).await?,
                RunEvent::RunFinished { status } => self.outbox_run_finished(*status).await?,
                _ => {}
            }
        }
        if let Some(renewal) = &effects.renewal {
            touched.insert(renewal.node.clone());
        }
        for node in &effects.rehint {
            if self
                .state
                .step(node)
                .is_some_and(|s| s.status() == StepStatus::Ready)
            {
                hinted.insert(node.clone());
                touched.insert(node.clone());
                self.outbox_task_ready(node).await?;
            }
        }

        // 3. The step projections, from the state the events produced.
        for node in &touched {
            self.upsert_step(
                node,
                &effects,
                hinted.contains(node),
                output_digest.get(node),
            )
            .await?;
        }

        // 4. The run row, guarded by the version it was loaded at.
        let (reason, code) = match (self.state.stop_reason(), self.state.stop_code()) {
            (Some(reason), code) => (
                Some(match reason {
                    StopReason::Cancelled => "CANCELLED",
                    StopReason::TimedOut => "TIMED_OUT",
                    StopReason::Failed => "FAILED",
                }),
                code.map(FailureCode::as_str),
            ),
            (None, _) => (None, None),
        };
        if written > 0 {
            let updated = sqlx::query(
                "UPDATE runs SET status = $4, run_version = $5, stop_reason = $6, stop_code = $7, \
                        paused = $8, updated_at = $9, \
                        finished_at = CASE WHEN $10 THEN COALESCE(finished_at, $9) ELSE NULL END \
                 WHERE tenant_id = $1 AND id = $2 AND run_version = $3",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(i64::try_from(self.persisted).map_err(|_| corrupt("run version"))?)
            .bind(self.state.status().as_str())
            .bind(i64::try_from(version).map_err(|_| corrupt("run version"))?)
            .bind(reason)
            .bind(code)
            .bind(self.state.is_paused())
            .bind(self.now)
            .bind(self.state.status().is_terminal())
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            if updated != 1 {
                return Err(StoreError::Internal {
                    detail: "the run changed while its row was locked".into(),
                });
            }
        }
        self.persisted = version;

        // 5. Audit and idempotency commit with everything above.
        if let Some(audit) = &effects.audit {
            insert_audit(&mut self.tx, audit).await?;
        }
        if let Some((idempotency, result)) = &effects.idempotency
            && !remember(
                &mut self.tx,
                tenant,
                idempotency,
                &hash(&idempotency.fingerprint),
                result,
            )
            .await?
        {
            return Err(StoreError::Unavailable {
                detail: "an idempotency record for this command appeared concurrently".into(),
            });
        }
        Ok(())
    }

    async fn commit(self) -> Result<(), StoreError> {
        self.tx.commit().await.map_err(store_error)
    }
}

impl PgRunTx {
    /// The parent is told the outcome (or no longer cares): the child need not notify it again.
    async fn settle_child(&mut self, child: RunId, cancel: bool) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE runs SET parent_settled = true, \
                    parent_cancel = (parent_cancel OR ($3 AND finished_at IS NULL)) \
             WHERE tenant_id = $1 AND id = $2 AND parent_run_id IS NOT NULL",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(child.as_uuid())
        .bind(cancel)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// A `child` step of this run was cancelled (`settle`) or asked to stop: apply the node's cancel
    /// policy to the child run it reserved, if it exists.
    ///
    /// The flag is set by the parent, under the parent's lock; the scheduler then cancels the child
    /// under the child's own lock, so no transaction ever holds both.
    async fn release_child(&mut self, node: &NodeId, settle: bool) -> Result<(), StoreError> {
        let Some(NodeKind::Child { cancel_policy, .. }) =
            self.def.node(node).map(sagat_domain::workflow::Node::kind)
        else {
            return Ok(());
        };
        let cascades = cancel_policy.cascades();
        let child: Option<Option<Uuid>> = sqlx::query_scalar(
            "SELECT child_run_id FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        let Some(Some(child)) = child else {
            // The step never reserved a child (it failed before): nothing to release.
            return Ok(());
        };
        if settle {
            self.settle_child(RunId::new(child), cascades).await
        } else if cascades {
            sqlx::query(
                "UPDATE runs SET parent_cancel = true \
                 WHERE tenant_id = $1 AND id = $2 AND parent_run_id IS NOT NULL",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(child)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            Ok(())
        } else {
            Ok(())
        }
    }

    async fn outbox_task_ready(&mut self, node: &NodeId) -> Result<(), StoreError> {
        let activity = match self.def.node(node).map(sagat_domain::workflow::Node::kind) {
            Some(NodeKind::Activity { activity, .. }) => activity.as_str().to_owned(),
            _ => return Ok(()),
        };
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
             VALUES ($1, $2, $3, 'task.ready', $4)",
        )
        .bind(Uuid::now_v7())
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(serde_json::json!({
            "tenantId": self.info.tenant_id.to_string(),
            "namespaceId": self.info.namespace_id.to_string(),
            "runId": self.info.run_id.to_string(),
            "node": node.as_str(),
            "activity": activity,
            "traceId": self.info.trace_id,
        }))
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn outbox_run_finished(&mut self, status: RunStatus) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
             VALUES ($1, $2, $3, 'run.finished', $4)",
        )
        .bind(Uuid::now_v7())
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(serde_json::json!({
            "tenantId": self.info.tenant_id.to_string(),
            "namespaceId": self.info.namespace_id.to_string(),
            "runId": self.info.run_id.to_string(),
            "status": status.as_str(),
        }))
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// Writes the step row for `node` from the current in-memory state.
    async fn upsert_step(
        &mut self,
        node: &NodeId,
        effects: &Effects,
        hinted: bool,
        output: Option<&Vec<u8>>,
    ) -> Result<(), StoreError> {
        let Some(step) = self.state.step(node) else {
            return Ok(());
        };
        let def_node = self.def.node(node);
        let activity = match def_node.map(sagat_domain::workflow::Node::kind) {
            Some(NodeKind::Activity { activity, .. }) => Some(activity.as_str()),
            _ => None,
        };
        let event_type = match def_node.map(sagat_domain::workflow::Node::kind) {
            Some(NodeKind::WaitEvent { event_type, .. }) => Some(event_type.as_str()),
            _ => None,
        };
        let (waiting_kind, due_at, correlation_key) = match step.waiting_on() {
            None => (None, None, None),
            Some(WaitingOn::Timer { due_at }) => (Some("TIMER"), Some(*due_at), None),
            Some(WaitingOn::Event {
                correlation_key,
                deadline,
            }) => (
                Some("EVENT"),
                Some(*deadline),
                Some(correlation_key.as_str()),
            ),
            Some(WaitingOn::Branches) => (Some("BRANCHES"), None, None),
            Some(WaitingOn::Child { .. }) => (Some("CHILD"), None, None),
        };
        // A `child` step reserved in this transaction: the run id, and the input it starts with.
        let spawn = effects.children.iter().find(|c| &c.node == node);
        let child_run = match step.waiting_on() {
            Some(WaitingOn::Child { run }) => Some(run.as_uuid()),
            _ => spawn.map(|c| c.child.as_uuid()),
        };
        let child_input = spawn.map(|c| c.input.clone());
        let grant = effects.lease.as_ref().filter(|g| &g.node == node);
        let renewal = effects.renewal.as_ref().filter(|r| &r.node == node);
        let leased = matches!(step.status(), StepStatus::Leased | StepStatus::Running);
        let fresh_grant = grant.filter(|_| leased);
        let step_uuid = step_id(self.info.version_id, self.info.run_id, node);
        let status = step.status().as_str();
        let attempt = i32::try_from(step.attempt()).map_err(|_| corrupt("attempt"))?;
        let generation = i64::try_from(step.generation()).map_err(|_| corrupt("generation"))?;
        let inserted = !self.stored.contains(node);
        if inserted {
            if leased {
                return Err(StoreError::Internal {
                    detail: "a step cannot be created already leased".into(),
                });
            }
            sqlx::query(
                "INSERT INTO steps \
                 (tenant_id, run_id, node, step_id, namespace_id, status, attempt, generation, \
                  cancel_requested, routed, condition, timed_out, failure_code, next_attempt_at, \
                  waiting_kind, due_at, event_type, correlation_key, activity, ready_at, \
                  last_hinted_at, output_digest, child_run_id, child_input, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
                         $18, $19, CASE WHEN $6 = 'READY' THEN $20 END, CASE WHEN $21 THEN $20 END, \
                         $22, $23, $24, $20)",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.run_id.as_uuid())
            .bind(node.as_str())
            .bind(step_uuid)
            .bind(self.info.namespace_id.as_uuid())
            .bind(status)
            .bind(attempt)
            .bind(generation)
            .bind(step.cancel_requested())
            .bind(step.routed())
            .bind(step.condition())
            .bind(step.timed_out())
            .bind(step.failure().map(FailureCode::as_str))
            .bind(step.next_attempt_at())
            .bind(waiting_kind)
            .bind(due_at)
            .bind(if waiting_kind == Some("EVENT") { event_type } else { None })
            .bind(correlation_key)
            .bind(activity)
            .bind(self.now)
            .bind(hinted)
            .bind(output.cloned())
            .bind(child_run)
            .bind(child_input)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            self.stored.insert(node.clone());
            return Ok(());
        }
        sqlx::query(
            "UPDATE steps SET \
                status = $6, attempt = $7, generation = $8, cancel_requested = $9, routed = $10, \
                condition = $11, timed_out = $12, failure_code = $13, next_attempt_at = $14, \
                waiting_kind = $15, due_at = $16, event_type = $17, correlation_key = $18, \
                activity = $19, \
                ready_at = CASE WHEN $6 = 'READY' \
                                THEN COALESCE(CASE WHEN status = 'READY' THEN ready_at END, $20) \
                                ELSE ready_at END, \
                last_hinted_at = CASE WHEN $21 THEN $20 ELSE last_hinted_at END, \
                lease_token_hash = CASE WHEN $6 IN ('LEASED', 'RUNNING') \
                                        THEN COALESCE($22, lease_token_hash) END, \
                lease_expires_at = CASE WHEN $6 NOT IN ('LEASED', 'RUNNING') THEN NULL \
                                        WHEN $22::bytea IS NOT NULL THEN $20 + $23 * interval '1 microsecond' \
                                        WHEN $27::bigint IS NOT NULL THEN LEAST( \
                                             $20 + $27 * interval '1 microsecond', \
                                             COALESCE(attempt_deadline_at, 'infinity')) \
                                        ELSE lease_expires_at END, \
                attempt_deadline_at = CASE WHEN $6 NOT IN ('LEASED', 'RUNNING') THEN NULL \
                                           WHEN $22::bytea IS NOT NULL \
                                                THEN CASE WHEN $24::bigint IS NOT NULL \
                                                          THEN $20 + $24 * interval '1 microsecond' END \
                                           ELSE attempt_deadline_at END, \
                leased_by = CASE WHEN $6 NOT IN ('LEASED', 'RUNNING') THEN NULL \
                                 WHEN $22::bytea IS NOT NULL THEN $25 \
                                 ELSE leased_by END, \
                output_digest = COALESCE($26, output_digest), \
                child_run_id = COALESCE($28, child_run_id), \
                child_input = CASE WHEN $29::bytea IS NOT NULL THEN $29 \
                                   WHEN $6 = 'WAITING' THEN child_input ELSE NULL END, \
                updated_at = $20 \
             WHERE tenant_id = $1 AND run_id = $2 AND node = $3",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .bind(step_uuid)
        .bind(self.info.namespace_id.as_uuid())
        .bind(status)
        .bind(attempt)
        .bind(generation)
        .bind(step.cancel_requested())
        .bind(step.routed())
        .bind(step.condition())
        .bind(step.timed_out())
        .bind(step.failure().map(FailureCode::as_str))
        .bind(step.next_attempt_at())
        .bind(waiting_kind)
        .bind(due_at)
        .bind(if waiting_kind == Some("EVENT") { event_type } else { None })
        .bind(correlation_key)
        .bind(activity)
        .bind(self.now)
        .bind(hinted)
        .bind(fresh_grant.map(|g| g.token_hash.clone()))
        .bind(fresh_grant.map_or(0, |g| micros(g.ttl)))
        .bind(fresh_grant.and_then(|g| g.attempt_timeout).map(micros))
        .bind(fresh_grant.map(|g| g.worker_id.clone()))
        .bind(output.cloned())
        .bind(renewal.map(|r| micros(r.ttl)))
        .bind(child_run)
        .bind(child_input)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }
}

/// The run store also serves definitions, so one value can back every service that needs both.
impl DefinitionStore for PgRunStore {
    async fn find_namespace_by_slug(
        &self,
        tenant: TenantId,
        slug: &Slug,
    ) -> Result<Option<Namespace>, StoreError> {
        self.definitions.find_namespace_by_slug(tenant, slug).await
    }

    async fn create_workflow(
        &self,
        workflow: &Workflow,
        draft: &[u8],
        audit: &AuditRecord,
        idempotency: &Idempotency,
    ) -> Result<Applied<()>, StoreError> {
        self.definitions
            .create_workflow(workflow, draft, audit, idempotency)
            .await
    }

    async fn get_workflow(
        &self,
        tenant: TenantId,
        id: WorkflowId,
    ) -> Result<Option<WorkflowRecord>, StoreError> {
        self.definitions.get_workflow(tenant, id).await
    }

    async fn find_workflow_by_name(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &Slug,
    ) -> Result<Option<WorkflowRecord>, StoreError> {
        self.definitions
            .find_workflow_by_name(tenant, namespace, name)
            .await
    }

    async fn list_workflows(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<WorkflowId>,
        limit: u32,
    ) -> Result<Vec<WorkflowRecord>, StoreError> {
        self.definitions
            .list_workflows(tenant, namespace, after, limit)
            .await
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
        self.definitions
            .update_draft(tenant, id, expected_revision, draft, audit, now)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish(
        &self,
        principal: &Principal,
        id: WorkflowId,
        draft_revision: u64,
        version_id: WorkflowVersionId,
        digest: Digest,
        document: &[u8],
        release_notes: Option<&str>,
        audit: &AuditRecord,
        idempotency: &Idempotency,
        now: OffsetDateTime,
    ) -> Result<Option<Applied<(WorkflowVersion, bool)>>, StoreError> {
        self.definitions
            .publish(
                principal,
                id,
                draft_revision,
                version_id,
                digest,
                document,
                release_notes,
                audit,
                idempotency,
                now,
            )
            .await
    }

    async fn get_version(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        number: u32,
    ) -> Result<Option<VersionRecord>, StoreError> {
        self.definitions.get_version(tenant, id, number).await
    }

    async fn get_version_by_id(
        &self,
        tenant: TenantId,
        id: WorkflowVersionId,
    ) -> Result<Option<VersionRecord>, StoreError> {
        self.definitions.get_version_by_id(tenant, id).await
    }

    async fn list_versions(
        &self,
        tenant: TenantId,
        id: WorkflowId,
        after: Option<u32>,
        limit: u32,
    ) -> Result<Vec<VersionRecord>, StoreError> {
        self.definitions
            .list_versions(tenant, id, after, limit)
            .await
    }

    async fn workflow_exists_outside(
        &self,
        tenant: TenantId,
        id: WorkflowId,
    ) -> Result<bool, StoreError> {
        self.definitions.workflow_exists_outside(tenant, id).await
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        DefinitionStore::record_denial(&self.definitions, audit).await
    }
}

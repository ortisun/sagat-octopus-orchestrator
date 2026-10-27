//! PostgreSQL implementation of the run store: one locked transaction per command.
//!
//! [`PgRunStore::begin`] takes the run's row lock (`FOR UPDATE`, or `SKIP LOCKED` for workers),
//! loads the immutable definition and the current state from the projection rows, and hands both to
//! the engine. [`PgRunTx::persist`] then writes, in that same transaction: the journal entries, the
//! step rows, the payloads, the outbox, the run row, the audit record and the idempotency record.
//! Nothing outside the transaction ever sees a half-applied command, and a crash before `COMMIT`
//! leaves the previous state intact (`FR-002-003`).

use crate::pool::BeginSafely;
mod ad_hoc;
mod database;
mod path;
mod path_read;
mod scoped_events;
use scoped_events::EventWaitReadState;
pub(crate) use scoped_events::validate_scoped_event_identity;

// A deterministic control owns no worker, timer, event, child, approval or output capability.
// Reuse the same physical-row predicate in snapshot readers and guarded update writers.
macro_rules! scoped_suspension_metadata_empty {
    () => {
        "s.activity IS NULL AND s.secrets IS NULL \
         AND s.ready_at IS NULL AND s.last_hinted_at IS NULL AND s.hint_count=0 \
         AND s.lease_token_hash IS NULL AND s.lease_expires_at IS NULL \
         AND s.attempt_deadline_at IS NULL AND s.leased_by IS NULL \
         AND s.reported_lease_token_hash IS NULL \
         AND s.child_run_id IS NULL AND NOT s.child_started AND s.child_input IS NULL \
         AND s.approval_request_id IS NULL AND s.kind='FORWARD'"
    };
}
macro_rules! scoped_scheduler_metadata_empty {
    () => {
        concat!(
            $crate::execution::scoped_suspension_metadata_empty!(),
            " AND s.output_digest IS NULL AND s.event_type IS NULL AND s.correlation_key IS NULL \
         AND s.event_since IS NULL AND s.event_scanned_at IS NULL"
        )
    };
}
macro_rules! scoped_control_metadata_empty {
    () => {
        concat!(
            $crate::execution::scoped_scheduler_metadata_empty!(),
            " AND s.waiting_kind IS NULL AND s.due_at IS NULL"
        )
    };
}
macro_rules! scoped_timer_metadata_valid {
    () => { concat!($crate::execution::scoped_scheduler_metadata_empty!(),
        " AND s.attempt=0 AND s.generation=0 AND NOT s.cancel_requested AND NOT s.timed_out \
          AND s.next_attempt_at IS NULL AND s.condition IS NULL AND s.switch_case_id IS NULL \
          AND s.failure_code IS NULL \
          AND ((s.status='WAITING' AND NOT s.routed AND s.waiting_kind IS NOT DISTINCT FROM 'TIMER' AND s.due_at IS NOT NULL) \
            OR (s.status='PENDING' AND NOT s.routed AND s.waiting_kind IS NULL AND s.due_at IS NULL) \
            OR (s.status='SUCCEEDED' AND s.waiting_kind IS NULL AND s.due_at IS NULL) \
            OR (s.status='CANCELLED' AND s.routed AND s.waiting_kind IS NULL AND s.due_at IS NULL))") };
}
macro_rules! scoped_event_metadata_valid {
    () => { concat!($crate::execution::scoped_suspension_metadata_empty!(),
        " AND s.output_digest IS NULL AND s.attempt=0 AND s.generation=0 AND NOT s.cancel_requested \
          AND s.next_attempt_at IS NULL AND s.condition IS NULL AND s.switch_case_id IS NULL \
          AND ((s.status='WAITING' AND NOT s.routed AND NOT s.timed_out AND s.failure_code IS NULL \
                AND s.waiting_kind IS NOT DISTINCT FROM 'EVENT' AND s.due_at IS NOT NULL \
                AND s.event_type IS NOT NULL AND s.correlation_key IS NOT NULL AND s.event_since IS NOT NULL) \
            OR (s.waiting_kind IS NULL AND s.due_at IS NULL AND s.event_type IS NULL \
                AND s.correlation_key IS NULL AND s.event_since IS NULL AND s.event_scanned_at IS NULL \
                AND ((s.status='PENDING' AND NOT s.routed AND NOT s.timed_out AND s.failure_code IS NULL) \
                  OR (s.status='SUCCEEDED' AND s.timed_out AND s.failure_code IS NULL) \
                  OR (s.status='FAILED' AND NOT s.timed_out AND (s.failure_code IS NOT DISTINCT FROM 'EXPRESSION_ERROR' OR s.failure_code IS NOT DISTINCT FROM 'WAIT_TIMED_OUT')) \
                  OR (s.status='CANCELLED' AND s.routed AND NOT s.timed_out AND s.failure_code IS NULL))))") };
}
pub(crate) use scoped_control_metadata_empty;
pub(crate) use scoped_event_metadata_valid;
pub(crate) use scoped_scheduler_metadata_empty;
pub(crate) use scoped_suspension_metadata_empty;
pub(crate) use scoped_timer_metadata_valid;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use sagat_application::chain_progress::{
    ChainInitialization, ChainParent, ChainProgressError, ChainRunTx, LockedChainHead,
};
use sagat_application::definitions::{
    Applied, DefinitionStore, Idempotency, VersionRecord, WorkflowRecord,
};
use sagat_application::error::StoreError;
use sagat_application::execution::{
    BreakerPolicy, DefinitionCache, Effects, LeaseDeadlines, MAX_RUN_OUTPUT_BYTES, ParentLink,
    PathExecutionSnapshot, PathExecutionStore, PathExecutionTx, RunData, RunInfo, RunRecord,
    RunSegment, RunTx, ScopeInput, compensation_id, instance_compensation_id, instance_step_id,
};
use sagat_application::primitives::CollectionFinish;
use sagat_application::runs::{
    ApprovalView, ChainView, GraphSnapshot, JournalEntry, Lock, NewRun, RunFilter, RunStore,
    SagaView, StepView,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersion, WorkflowVersionId};
use sagat_domain::execution::{
    AddressedRunEvent, AncestorExecutionFence, ApprovalRequestId, ChainFinalOutcome, ChainPolicy,
    ChainScope, CollectionDrainReason, CollectionIterationRestore, CollectionPhase,
    ContinuationTransfer, CorrelationKey, ExecutionInstanceId, FailureCode, GlobalExecutionFence,
    InstancePath, LoopContinuation, LoopDrainReason, LoopIterationRestore, LoopKind, LoopPhase,
    PathCollectionState, PathExecutionState, PathLoopState, PathScopeSagaState, RunChainState,
    RunEvent, RunId, RunState, RunStatus, RunWorkSummary, SagaPhase, SagaState, ScopeSagaState,
    SegmentCompletionKind, SegmentIndex, StepState, StepStatus, StopReason, SwitchSelection,
    WaitingOn,
};
use sagat_domain::identity::{Namespace, NamespaceId, Principal, Slug, TenantId};
use sagat_domain::saga::{NodeOrigin, SagaOutcome};
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::{ActivityName, Digest, Node, NodeId, NodeKind, WorkflowDefinition};
use sagat_telemetry::{WorkflowPrimitiveMetric, WorkflowPrimitiveOutcome};
use sha2::{Digest as _, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::collection_outputs;
use crate::definitions::{PgDefinitionStore, hash, remember, replay};
use crate::event_codec::{decode, encode};
use crate::identity::{PgIdentityStore, col, corrupt, insert_audit, store_error};
use crate::quotas::ensure_room_for_run;
use crate::signals::{Signal, of_event};

/// PostgreSQL-backed run store.
#[derive(Debug, Clone)]
pub struct PgRunStore {
    pub(crate) pool: PgPool,
    pub(crate) cache: DefinitionCache,
    identity: PgIdentityStore,
    pub(crate) definitions: PgDefinitionStore,
    breaker: BreakerPolicy,
    pub(crate) scoped_worker_protocol: bool,
}

impl PgRunStore {
    #[must_use]
    pub fn new(pool: PgPool, cache: DefinitionCache) -> Self {
        Self {
            identity: PgIdentityStore::new(pool.clone()),
            definitions: PgDefinitionStore::new(pool.clone()),
            pool,
            cache,
            breaker: BreakerPolicy::default(),
            scoped_worker_protocol: false,
        }
    }

    /// When the compensations of a provider operation stop being leased (FR-003-003).
    #[must_use]
    pub const fn with_breaker(mut self, policy: BreakerPolicy) -> Self {
        self.breaker = policy;
        self
    }

    /// Enables the scoped worker storage path for an explicitly gated runtime or test harness.
    #[must_use]
    pub const fn with_scoped_worker_protocol(mut self) -> Self {
        self.scoped_worker_protocol = true;
        self
    }

    /// Waits until denial audits accepted by this store have reached PostgreSQL.
    pub async fn flush_denial_writes(&self) {
        self.identity.flush_denial_writes().await;
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
                 'run_events', 'payloads', 'event_outbox', 'worker_sessions', 'command_dedupe', 'private_execution_definitions']) AS t(name) \
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
    ///
    /// The document is read on `conn`, never on a connection of its own: a caller that holds a
    /// transaction and waits for a second connection from the same pool starves the pool when enough
    /// callers do it at once (a burst of first requests for a version nobody has cached yet).
    pub(crate) async fn definition(
        &self,
        conn: &mut PgConnection,
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
        .fetch_optional(conn)
        .await
        .map_err(store_error)?
        .ok_or_else(|| corrupt("workflow version reference"))?;
        self.cache
            .get_or_parse(version, &document)
            .map_err(|error| {
                tracing::error!(%error, %version, "a stored workflow version no longer parses");
                StoreError::UnreadableDefinition
            })
    }

    pub(crate) async fn begin_in_transaction(
        &self,
        mut tx: Transaction<'static, Postgres>,
        tenant: TenantId,
        run: RunId,
        lock: Lock,
        chain_lock: Option<LockedChainHead>,
    ) -> Result<Option<PgRunTx>, StoreError> {
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
        // A volatile SELECT target can be evaluated before FOR UPDATE finishes waiting. Sample
        // mutation time only after the row lock is held, so contention cannot backdate deadlines.
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        let input_digest: Vec<u8> = col(&row, "input_digest")?;
        let step_rows = sqlx::query(concat!(
            "SELECT node, instance_path_nodes, instance_path_indices, status, attempt, generation, cancel_requested, routed, condition, switch_case_id, \
                    timed_out, failure_code, next_attempt_at, waiting_kind, due_at, correlation_key, event_type, event_since, \
                    child_run_id, approval_request_id, s.namespace_id, s.step_id, s.node_kind AS stored_node_kind, (",
            scoped_control_metadata_empty!(),
            ") AS control_metadata_empty, (",scoped_timer_metadata_valid!(),") AS timer_metadata_valid, (",scoped_event_metadata_valid!(),") AS event_metadata_valid \
             FROM steps s WHERE tenant_id = $1 AND run_id = $2",
        ))
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let mut steps = BTreeMap::new();
        let mut stored = BTreeSet::new();
        for row in &step_rows {
            let node: String = col(row, "node")?;
            let node = NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?;
            let path = step_instance_path(row)?;
            let instance = ExecutionInstanceId::in_path(path, node);
            stored.insert(instance.clone());
            steps.insert(instance, step_state(row)?);
        }
        let sagas = load_sagas(&mut tx, tenant, run).await?;
        let stop = match record.stop_reason {
            None => None,
            Some(reason) => Some((
                reason,
                stop_code
                    .map(|code| FailureCode::parse(&code))
                    .transpose()
                    .map_err(|_| corrupt("stop code"))?,
            )),
        };
        let mut state = RunState::restore_instances(
            run,
            record.status,
            stop,
            paused,
            steps,
            sagas,
            record.version,
        );
        let definition = self
            .definition(&mut tx, tenant, record.info.version_id)
            .await?;
        load_collections(&mut tx, tenant, run, &definition, &mut state).await?;
        load_loops(&mut tx, tenant, run, &mut state).await?;
        load_scope_sagas(&mut tx, tenant, run, &mut state).await?;
        load_chain(&mut tx, tenant, run, &mut state).await?;
        for row in &step_rows {
            let node = NodeId::parse_stored(&col::<String>(row, "node")?)
                .map_err(|_| corrupt("step node"))?;
            let instance = ExecutionInstanceId::in_path(step_instance_path(row)?, node);
            if !instance.path().is_root() {
                let step = state
                    .instance(&instance)
                    .ok_or_else(|| corrupt("scoped step identity"))?;
                validate_scoped_control_row(definition.node_for_instance(&instance), step, row)?;
                validate_scoped_event_identity(&definition, &record.info, &instance, row)?;
            }
        }
        let timer_read_states = state
            .instances()
            .filter(|(instance, _)| {
                !instance.path().is_root()
                    && matches!(
                        definition.node_for_instance(instance).map(Node::kind),
                        Some(NodeKind::Timer { .. })
                    )
            })
            .map(|(instance, step)| (instance.clone(), TimerReadState::from_step(step)))
            .collect();
        let event_wait_read_states = state
            .instances()
            .filter(|(instance, _)| {
                !instance.path().is_root()
                    && matches!(
                        definition.node_for_instance(instance).map(Node::kind),
                        Some(NodeKind::WaitEvent { .. })
                    )
            })
            .map(|(instance, step)| (instance.clone(), EventWaitReadState::from_step(step)))
            .collect();
        Ok(Some(PgRunTx {
            tx,
            info: record.info,
            now,
            own_scheduler_time: None,
            def: definition,
            persisted: state.version(),
            state,
            data: None,
            input_digest,
            stored,
            timer_read_states,
            event_wait_read_states,
            breaker: self.breaker,
            script_retention: BTreeMap::new(),
            signals: Vec::new(),
            chain_lock,
            initialize_chain: false,
            cancelled_waits: Vec::new(),
        }))
    }

    async fn lock_chain_for_run(
        &self,
        tx: &mut Transaction<'static, Postgres>,
        tenant: TenantId,
        run: RunId,
        lock: Lock,
    ) -> Result<RunChainLock, StoreError> {
        let chain_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT chain_id FROM chain_segments WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_error)?;
        let Some(chain_id) = chain_id else {
            return Ok(RunChainLock::NotMember);
        };
        let query = match lock {
            Lock::Wait => {
                "SELECT namespace_id,current_run_id,revision,cancel_requested,final_outcome \
                           FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 FOR UPDATE"
            }
            Lock::SkipLocked => {
                "SELECT namespace_id,current_run_id,revision,cancel_requested,final_outcome \
                                 FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2 \
                                 FOR UPDATE SKIP LOCKED"
            }
        };
        let row = sqlx::query(query)
            .bind(tenant.as_uuid())
            .bind(chain_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(store_error)?;
        let Some(row) = row else {
            return match lock {
                Lock::Wait => Err(corrupt("chain membership without chain")),
                Lock::SkipLocked => Ok(RunChainLock::Contended),
            };
        };
        Ok(RunChainLock::Locked(LockedChainHead::restore(
            ChainScope::new(tenant, NamespaceId::new(col(&row, "namespace_id")?)),
            RunId::new(chain_id),
            RunId::new(col(&row, "current_run_id")?),
            u64::try_from(col::<i64>(&row, "revision")?).map_err(|_| corrupt("chain revision"))?,
            col(&row, "cancel_requested")?,
            restore_chain_outcome(col::<Option<String>>(&row, "final_outcome")?.as_deref())?,
        )))
    }

    /// A receipt never grants stale database authority, nor resolves a floating reference again.
    async fn check_database_replay(
        &self,
        conn: &mut PgConnection,
        tenant: TenantId,
        subject: &str,
        run: &str,
    ) -> Result<(), StoreError> {
        let run = run.parse::<RunId>().map_err(|_| corrupt("run receipt"))?;
        let row = sqlx::query(RUN_SELECT)
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?
            .ok_or(StoreError::MissingReference)?;
        let info = run_record(&row)?.info;
        let definition = self.definition(conn, tenant, info.version_id).await?;
        if definition.nodes().any(|n| n.database_operation().is_some()) {
            let mut actors = crate::database::run_actors(conn, &info).await?;
            if subject != sagat_domain::identity::Subject::engine().as_str()
                && !actors.iter().any(|s| s == subject)
            {
                actors.push(subject.to_owned());
            }
            crate::database::admit_definition(
                conn,
                tenant,
                info.namespace_id,
                &actors.iter().map(String::as_str).collect::<Vec<_>>(),
                &definition,
                Some(run),
                Some(&info),
            )
            .await?;
        }
        Ok(())
    }

    /// Verifies that the stored projections equal a replay of the stored journal. Used by tests and
    /// by operators investigating a suspected inconsistency; never needed on the hot path.
    ///
    /// # Errors
    /// A description of the first difference found.
    pub async fn verify_run(&self, tenant: TenantId, run: RunId) -> Result<(), String> {
        let mut stored_tx = self
            .begin(tenant, run, Lock::Wait)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("the run does not exist")?;
        let stored = stored_tx.state.clone();
        let rows = sqlx::query(
            "SELECT seq, event_type, node, detail, instance_path_nodes, instance_path_indices \
             FROM run_events \
             WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_all(&mut *stored_tx.tx)
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
            let event = decode(&event_type, node.as_deref(), &detail)
                .ok_or_else(|| format!("event {seq} ('{event_type}') cannot be decoded"))?;
            let path = event_instance_path(row).map_err(|e| e.to_string())?;
            events.push(
                AddressedRunEvent::in_path(path, event).map_err(|e| {
                    format!("event {seq} ('{event_type}') has an invalid path: {e}")
                })?,
            );
        }
        let replayed =
            RunState::replay_addressed(run, &events).map_err(|e| format!("replay failed: {e}"))?;
        if replayed != stored {
            return Err(format!(
                "projection differs from the journal:\n  replayed: {replayed:?}\n  stored:   {stored:?}"
            ));
        }
        drop(stored_tx);
        // The run's result (G21) is the payload its `end.reached` event names.
        let result_agrees: bool = sqlx::query_scalar(
            "SELECT r.result_digest IS NOT DISTINCT FROM \
                    (SELECT e.payload_digest FROM run_events e \
                      WHERE e.tenant_id = r.tenant_id AND e.run_id = r.id \
                        AND e.event_type = 'end.reached' ORDER BY e.seq DESC LIMIT 1) \
             FROM runs r WHERE r.tenant_id = $1 AND r.id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        if result_agrees {
            Ok(())
        } else {
            Err("the run's result is not the payload its end.reached event names".into())
        }
    }
}

/// Serialize a pinned member mutation on its immutable, tenant-scoped chain before its own run.
/// The guard remains in the caller's transaction; it neither forwards nor grants head authority.
/// Initial membership is committed with new-run creation; later membership identity is immutable.
pub(crate) async fn lock_member_chain(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    run: RunId,
) -> Result<(), StoreError> {
    lock_member_chain_sql(tx, tenant, namespace, run)
        .await
        .map_err(store_error)
}

/// The same guard without logging/classification; callers choose their diagnostic boundary.
pub(crate) async fn lock_member_chain_sql(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    run: RunId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT c.chain_id FROM chain_segments s JOIN execution_chains c \
         ON c.tenant_id=s.tenant_id AND c.namespace_id=s.namespace_id AND c.chain_id=s.chain_id \
         WHERE s.tenant_id=$1 AND s.namespace_id=$2 AND s.run_id=$3 FOR UPDATE OF c",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(run.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// Serialize the capacity check and subsequent lease on the owning execution transaction.
// Check the clock only after acquiring the tuple: an unchanged-row wait need not re-evaluate
// a volatile WHERE predicate when the blocker releases its lock without changing the row.
async fn worker_session_room(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
    session_id: Uuid,
    token_hash: &[u8],
    subject: &str,
) -> Result<u32, StoreError> {
    let session: Option<(i32, OffsetDateTime)> = sqlx::query_as(
        "SELECT max_inflight,expires_at FROM worker_sessions \
         WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 AND token_hash=$4 \
           AND subject=$5 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(session_id)
    .bind(token_hash)
    .bind(subject)
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    let Some((max_inflight, expires_at)) = session else {
        return Ok(0);
    };
    let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await
        .map_err(store_error)?;
    if expires_at <= now {
        return Ok(0);
    }
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 \
         AND status IN ('LEASED','RUNNING') AND leased_by=$2",
    )
    .bind(tenant.as_uuid())
    .bind(session_id.to_string())
    .fetch_one(&mut **tx)
    .await
    .map_err(store_error)?;
    let limit = u32::try_from(max_inflight).map_err(|_| corrupt("max inflight"))?;
    Ok(limit.saturating_sub(u32::try_from(active).unwrap_or(u32::MAX)))
}

/// Lock first, then observe expiry. A volatile WHERE predicate can be evaluated before a row-lock
/// wait, and an unchanged row need not be rechecked when that wait ends.
pub(crate) async fn locked_scheduler_time(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
    owner: &str,
    lease: sagat_application::shards::Lease,
) -> Result<Option<OffsetDateTime>, StoreError> {
    locked_scheduler_time_sql(tx, run, owner, lease)
        .await
        .map_err(store_error)
}

/// Preserve the shared lock/clock order while letting sensitive callers redact driver failures.
pub(crate) async fn locked_scheduler_time_sql(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
    owner: &str,
    lease: sagat_application::shards::Lease,
) -> Result<Option<OffsetDateTime>, sqlx::Error> {
    if lease.shard != sagat_domain::sharding::Shard::of(run.as_uuid()) {
        return Ok(None);
    }
    let Ok(generation) = i64::try_from(lease.generation) else {
        return Ok(None);
    };
    let held: Option<i16> = sqlx::query_scalar(
        "SELECT shard FROM scheduler_shards \
         WHERE shard=$1 AND generation=$2 AND owner=$3 FOR SHARE",
    )
    .bind(i16::from(lease.shard.index()))
    .bind(generation)
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await?;
    if held.is_none() {
        return Ok(None);
    }
    // Compare in PostgreSQL: finite lease expiry can exceed OffsetDateTime's year 9999.
    // MATERIALIZED observes one fresh clock only after the SHARE lock, including after waits.
    let now: Option<Option<OffsetDateTime>> = sqlx::query_scalar(
        "WITH tick AS MATERIALIZED (SELECT clock_timestamp() AS now) \
         SELECT CASE WHEN s.expires_at>tick.now THEN tick.now END \
         FROM scheduler_shards s CROSS JOIN tick \
         WHERE s.shard=$1 AND s.generation=$2 AND s.owner=$3",
    )
    .bind(i16::from(lease.shard.index()))
    .bind(generation)
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(now.flatten())
}

enum RunChainLock {
    NotMember,
    Locked(LockedChainHead),
    Contended,
}

fn restore_chain_outcome(value: Option<&str>) -> Result<Option<ChainFinalOutcome>, StoreError> {
    value
        .map(|value| match value {
            "SUCCEEDED" => Ok(ChainFinalOutcome::Succeeded),
            "FAILED" => Ok(ChainFinalOutcome::Failed),
            "CANCELLED" => Ok(ChainFinalOutcome::Cancelled),
            "TIMED_OUT" => Ok(ChainFinalOutcome::TimedOut),
            "COMPENSATED" => Ok(ChainFinalOutcome::Compensated),
            "PARTIAL" => Ok(ChainFinalOutcome::Partial),
            "COMPENSATION_FAILED" => Ok(ChainFinalOutcome::CompensationFailed),
            _ => Err(corrupt("chain final outcome")),
        })
        .transpose()
}

async fn lock_path_revision(
    conn: &mut PgConnection,
    tenant: TenantId,
    run: RunId,
    lock: Lock,
) -> Result<Option<u64>, StoreError> {
    let query = match lock {
        Lock::Wait => "SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
        Lock::SkipLocked => {
            "SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE SKIP LOCKED"
        }
    };
    let row = sqlx::query(query)
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(conn)
        .await
        .map_err(store_error)?;
    row.map(|row| {
        u64::try_from(col::<i64>(&row, "run_version")?).map_err(|_| corrupt("bounded run revision"))
    })
    .transpose()
}

impl PathExecutionStore for PgRunStore {
    type Tx = PgPathExecutionTx;

    async fn load_path(
        &self,
        tenant: TenantId,
        run: RunId,
        path: InstancePath,
    ) -> Result<Option<PathExecutionSnapshot>, StoreError> {
        if path.is_root() {
            return Err(StoreError::Internal {
                detail: "bounded path reads require a scoped path".into(),
            });
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let snapshot = self.read_path_snapshot(&mut tx, tenant, run, path).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(snapshot)
    }

    async fn begin_current_path(
        &self,
        tenant: TenantId,
        run: RunId,
        path: InstancePath,
        lock: Lock,
    ) -> Result<Option<Self::Tx>, StoreError> {
        if path.is_root() {
            return Err(StoreError::Internal {
                detail: "bounded path reads require a scoped path".into(),
            });
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // Match full-run entry: serialize any chain before taking its member's mutation lock.
        // The transaction retains this guard; it grants no extra execution or finality authority.
        if matches!(
            self.lock_chain_for_run(&mut tx, tenant, run, lock).await?,
            RunChainLock::Contended
        ) {
            return Ok(None);
        }
        let Some(current) = lock_path_revision(&mut tx, tenant, run, lock).await? else {
            return Ok(None);
        };
        let Some(snapshot) = self.read_path_snapshot(&mut tx, tenant, run, path).await? else {
            return Ok(None);
        };
        if snapshot.state.global().version() != current {
            return Err(corrupt(
                "current bounded snapshot revision changed under its run lock",
            ));
        }
        let now = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        Ok(Some(PgPathExecutionTx::from_snapshot(
            tx,
            snapshot,
            now,
            self.breaker,
        )))
    }

    async fn begin_path(
        &self,
        snapshot: PathExecutionSnapshot,
        lock: Lock,
    ) -> Result<Option<Self::Tx>, StoreError> {
        let tenant = snapshot.info.tenant_id;
        let run = snapshot.info.run_id;
        let expected = snapshot.state.global().version();
        if snapshot.state.global().run_id() != run {
            return Err(StoreError::Internal {
                detail: "bounded snapshot run identity diverged".into(),
            });
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // Match full-run entry: serialize any chain before taking its member's mutation lock.
        // The transaction retains this guard; it grants no extra execution or finality authority.
        if matches!(
            self.lock_chain_for_run(&mut tx, tenant, run, lock).await?,
            RunChainLock::Contended
        ) {
            return Ok(None);
        }
        let Some(current) = lock_path_revision(&mut tx, tenant, run, lock).await? else {
            return Ok(None);
        };
        if current != expected {
            return Err(StoreError::RevisionConflict { current });
        }
        let now = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        Ok(Some(PgPathExecutionTx::from_snapshot(
            tx,
            snapshot,
            now,
            self.breaker,
        )))
    }
}

/// One exact-path mutation transaction that never restores the full run history.
pub struct PgPathExecutionTx {
    tx: Transaction<'static, Postgres>,
    snapshot: PathExecutionSnapshot,
    now: OffsetDateTime,
    own_scheduler_time: Option<OffsetDateTime>,
    breaker: BreakerPolicy,
    persisted: u64,
    stored: BTreeSet<ExecutionInstanceId>,
    timer_read_states: BTreeMap<ExecutionInstanceId, TimerReadState>,
    event_wait_read_states: BTreeMap<ExecutionInstanceId, EventWaitReadState>,
    signals: Vec<Signal>,
    script_retention: BTreeMap<ExecutionInstanceId, ScriptRetentionPins>,
}

impl PgPathExecutionTx {
    fn from_snapshot(
        tx: Transaction<'static, Postgres>,
        snapshot: PathExecutionSnapshot,
        now: OffsetDateTime,
        breaker: BreakerPolicy,
    ) -> Self {
        let persisted = snapshot.state.global().version();
        let stored = snapshot
            .state
            .local_instances()
            .map(|(node, _)| {
                ExecutionInstanceId::in_path(snapshot.state.path().clone(), node.clone())
            })
            .collect();
        let timer_read_states = snapshot
            .state
            .local_instances()
            .filter_map(|(node, step)| {
                let instance =
                    ExecutionInstanceId::in_path(snapshot.state.path().clone(), node.clone());
                matches!(
                    snapshot
                        .definition
                        .node_for_instance(&instance)
                        .map(Node::kind),
                    Some(NodeKind::Timer { .. })
                )
                .then(|| (instance, TimerReadState::from_step(step)))
            })
            .collect();
        let event_wait_read_states = snapshot
            .state
            .local_instances()
            .filter_map(|(node, step)| {
                let instance =
                    ExecutionInstanceId::in_path(snapshot.state.path().clone(), node.clone());
                matches!(
                    snapshot
                        .definition
                        .node_for_instance(&instance)
                        .map(Node::kind),
                    Some(NodeKind::WaitEvent { .. })
                )
                .then(|| (instance, EventWaitReadState::from_step(step)))
            })
            .collect();
        Self {
            tx,
            snapshot,
            now,
            own_scheduler_time: None,
            breaker,
            persisted,
            stored,
            timer_read_states,
            event_wait_read_states,
            signals: Vec::new(),
            script_retention: BTreeMap::new(),
        }
    }
}

impl std::fmt::Debug for PgPathExecutionTx {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PgPathExecutionTx")
            .field("run", &self.snapshot.info.run_id)
            .field("path", self.snapshot.state.path())
            .finish_non_exhaustive()
    }
}

async fn current_worker_authorized(
    tx: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    subject: &str,
) -> Result<bool, StoreError> {
    crate::quotas::lock_secrets(tx, tenant, namespace).await?;
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM public.sagat_lock_memberships($1, ARRAY[$2::text])")
            .bind(tenant.as_uuid())
            .bind(subject)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?;
    let Some(role) = role else {
        return Ok(false);
    };
    let role: sagat_domain::identity::Role =
        role.parse().map_err(|_| corrupt("membership role"))?;
    Ok(role != sagat_domain::identity::Role::DatabaseRunner
        && role
            .permissions()
            .contains(&sagat_domain::identity::Permission::WorkerExecute))
}

impl RunStore for PgRunStore {
    type Tx = PgRunTx;

    async fn replay_start(
        &self,
        tenant: TenantId,
        subject: &str,
        idempotency: &Idempotency,
    ) -> Result<Option<String>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let result = replay(
            &mut tx,
            tenant,
            idempotency,
            &hash(&idempotency.fingerprint),
        )
        .await?;
        if let Some(run) = &result {
            self.check_database_replay(&mut tx, tenant, subject, run)
                .await?;
        }
        tx.rollback().await.map_err(store_error)?;
        Ok(result)
    }

    async fn create(&self, mut new: NewRun) -> Result<Applied<PgRunTx>, StoreError> {
        let tenant = new.info.tenant_id;
        let hash = hash(&new.idempotency.fingerprint);
        let input_digest = Sha256::digest(&new.input).to_vec();
        for _ in 0..2 {
            let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
            if let Some(existing) = replay(&mut tx, tenant, &new.idempotency, &hash).await? {
                self.check_database_replay(&mut tx, tenant, &new.info.started_by, &existing)
                    .await?;
                return Ok(Applied::Replayed(existing));
            }
            // Advance the rerun FK's stable identity guard before admission locks.
            // This grants no authority over the source run's business state (ADR-0039).
            if let Some(source) = new.info.rerun_of {
                let exists: Option<Uuid> = sqlx::query_scalar(
                    "SELECT id FROM runs WHERE tenant_id=$1 AND id=$2 FOR KEY SHARE",
                )
                .bind(tenant.as_uuid())
                .bind(source.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?;
                exists.ok_or(StoreError::MissingReference)?;
            }
            if let Some(retained) = &new.retained {
                ad_hoc::retain(&mut tx, &mut new.info, retained).await?;
            } else if new.info.origin.is_some() {
                return Err(StoreError::MissingReference);
            }
            let version_row = sqlx::query(
                "SELECT workflow_id, document FROM workflow_versions \
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(new.info.version_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?
            .ok_or(StoreError::MissingReference)?;
            let stored_workflow: Uuid = col(&version_row, "workflow_id")?;
            if stored_workflow != new.info.workflow_id.as_uuid() {
                return Err(StoreError::MissingReference);
            }
            let version_document: Vec<u8> = col(&version_row, "document")?;
            let declared_state = self
                .cache
                .declared_state_publication(&version_document)
                .map_err(|error| {
                    tracing::error!(%error, version = %new.info.version_id, "stored declared state no longer validates");
                    StoreError::Internal {
                        detail: "stored declared state no longer validates".to_owned(),
                    }
                })?;
            let initial_state = self
                .cache
                .initial_declared_state(&version_document, &new.input)
                .map_err(|error| {
                    tracing::warn!(%error, version = %new.info.version_id, "declared initial state evaluation failed");
                    StoreError::Internal {
                        detail: "declared initial state evaluation failed".to_owned(),
                    }
                })?;
            crate::definitions::verify_declared_operations(
                &mut tx,
                tenant,
                new.info.namespace_id,
                new.info.version_id,
                declared_state.as_ref(),
            )
            .await?;
            let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
            let info = &new.info;
            let input_blob =
                match &new.input_blob {
                    Some(sagat_application::blobs::RunInputBinding::Uploaded(descriptor)) => {
                        Some(descriptor.id())
                    }
                    Some(sagat_application::blobs::RunInputBinding::Inherit(source)) => {
                        if info.rerun_of != Some(*source)
                            && info.parent.as_ref().map(|p| p.run) != Some(*source)
                        {
                            return Err(StoreError::MissingReference);
                        }
                        sqlx::query_scalar::<_, Option<Uuid>>(
                        "SELECT input_blob_id FROM runs WHERE tenant_id = $1 AND namespace_id = $2 \
                         AND id = $3 AND input_digest = $4",
                    )
                    .bind(tenant.as_uuid()).bind(info.namespace_id.as_uuid()).bind(source.as_uuid())
                    .bind(&input_digest).fetch_optional(&mut *tx).await.map_err(store_error)?
                    .flatten().map(sagat_domain::blobs::BlobId::new)
                    }
                    None => None,
                };
            // On a refusal the transaction is rolled back at once: its locks (the parent's row, the
            // tree's, the quota scopes') must not wait for the connection to be reused.
            if let Some(parent) = &info.parent
                && let Err(error) =
                    claim_reservation(&mut tx, tenant, parent, info.root, info.run_id).await
            {
                let _ = tx.rollback().await;
                return Err(error);
            }
            let bindings = if new
                .definition
                .nodes()
                .any(|n| n.database_operation().is_some())
            {
                let actors = crate::database::run_actors(&mut tx, &new.info).await?;
                crate::database::admit_definition(
                    &mut tx,
                    tenant,
                    new.info.namespace_id,
                    &actors.iter().map(String::as_str).collect::<Vec<_>>(),
                    &new.definition,
                    None,
                    Some(info),
                )
                .await?
            } else {
                Vec::new()
            };
            if let Err(error) = ensure_room_for_run(
                &mut tx,
                tenant,
                info.namespace_id,
                new.limits.namespace,
                new.limits.tenant,
            )
            .await
            {
                let _ = tx.rollback().await;
                return Err(error);
            }
            // Admission locks precede payload uniqueness checks, as in evaluated handoff.
            // A novel input/state can be retained concurrently by either writer.
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
            if let Some(initial_state) = &initial_state {
                sqlx::query(
                    "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) \
                     ON CONFLICT (tenant_id, digest) DO NOTHING",
                )
                .bind(tenant.as_uuid())
                .bind(initial_state.value().digest().as_bytes().as_slice())
                .bind(initial_state.content())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                let retained: Option<Vec<u8>> = sqlx::query_scalar(
                    "SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2",
                )
                .bind(tenant.as_uuid())
                .bind(initial_state.value().digest().as_bytes().as_slice())
                .fetch_optional(&mut *tx)
                .await
                .map_err(store_error)?;
                if retained.as_deref() != Some(initial_state.content()) {
                    return Err(StoreError::Internal {
                        detail: "retained declared initial state does not match its digest"
                            .to_owned(),
                    });
                }
            }
            sqlx::query(
                "INSERT INTO runs (id, tenant_id, namespace_id, workflow_id, workflow_version_id, \
                 status, run_version, paused, input_digest, correlation_id, started_by, trace_id, \
                 deadline_at, created_at, updated_at, parent_run_id, parent_node, cancel_policy, \
                 depth, root_run_id, trace_parent_span, rerun_of, input_blob_id, origin) \
                 VALUES ($1, $2, $3, $4, $5, 'RUNNING', 0, false, $6, $7, $8, $9, $10, $11, $11, \
                         $12, $13, $14, $15, $16, $17, $18, $19, $20)",
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
            .bind(info.root.as_uuid())
            .bind(info.parent_span.as_deref())
            .bind(info.rerun_of.map(|r| r.as_uuid()))
            .bind(input_blob.map(sagat_domain::blobs::BlobId::as_uuid))
            .bind(info.origin.map(sagat_application::runs::RunOrigin::as_str))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            capture_run_limits(
                &mut tx,
                tenant.as_uuid(),
                info.run_id.as_uuid(),
                new.definition.limits(),
            )
            .await?;
            if let Some(initial_state) = &initial_state {
                sqlx::query(
                    "INSERT INTO workflow_operation_states \
                     (tenant_id, namespace_id, run_id, state_digest, state_schema_digest, \
                      state_size_bytes, state_revision, last_changed_run_version, updated_at) \
                     VALUES ($1,$2,$3,$4,$5,$6,0,0,$7)",
                )
                .bind(tenant.as_uuid())
                .bind(info.namespace_id.as_uuid())
                .bind(info.run_id.as_uuid())
                .bind(initial_state.value().digest().as_bytes().as_slice())
                .bind(initial_state.value().schema_digest().as_bytes().as_slice())
                .bind(
                    i32::try_from(initial_state.value().size_bytes())
                        .map_err(|_| corrupt("declared initial state size"))?,
                )
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            }
            for (node, connection, revision) in bindings {
                sqlx::query("INSERT INTO run_database_connections(tenant_id,namespace_id,run_id,node_id,connection_id,revision) VALUES($1,$2,$3,$4,$5,$6)")
                    .bind(tenant.as_uuid()).bind(info.namespace_id.as_uuid()).bind(info.run_id.as_uuid())
                    .bind(node.as_str()).bind(connection.as_uuid()).bind(i64::from(revision))
                    .execute(&mut *tx).await.map_err(store_error)?;
            }
            if let Some(sagat_application::blobs::RunInputBinding::Uploaded(descriptor)) =
                &new.input_blob
                && let Err(error) = crate::blobs::commit_input(&mut tx, info, descriptor).await
            {
                tx.rollback().await.map_err(store_error)?;
                if matches!(error, StoreError::MissingReference)
                    && let Some(existing) = self
                        .replay_start(tenant, &new.info.started_by, &new.idempotency)
                        .await?
                {
                    return Ok(Applied::Replayed(existing));
                }
                return Err(error);
            }
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
                    timer_read_states: BTreeMap::new(),
                    event_wait_read_states: BTreeMap::new(),
                    info,
                    now,
                    own_scheduler_time: None,
                    def: Arc::clone(&new.definition),
                    state: RunState::new(new.info.run_id),
                    persisted: 0,
                    data: Some(RunData {
                        run_id: Some(new.info.run_id),
                        input: new.input.clone(),
                        input_blob,
                        state: initial_state.as_ref().map(|value| value.content().to_vec()),
                        state_revision: initial_state.as_ref().map(|_| 0),
                        outputs: BTreeMap::new(),
                        scope_inputs: BTreeMap::new(),
                    }),
                    input_digest,
                    stored: BTreeSet::new(),
                    breaker: self.breaker,
                    script_retention: BTreeMap::new(),
                    signals: Vec::new(),
                    chain_lock: None,
                    initialize_chain: true,
                    cancelled_waits: Vec::new(),
                }));
            }
        }
        Err(StoreError::Unavailable {
            detail: "idempotency record kept changing under concurrent requests".into(),
        })
    }

    async fn private_definition(
        &self,
        tenant: TenantId,
        actor: &str,
        run: RunId,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        ad_hoc::private_definition(self, tenant, actor, run).await
    }
    async fn latest_connection_probe(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &Slug,
    ) -> Result<Option<sagat_application::runs::ConnectionProbe>, StoreError> {
        ad_hoc::latest_probe(self, tenant, namespace, name).await
    }

    async fn begin(
        &self,
        tenant: TenantId,
        run: RunId,
        lock: Lock,
    ) -> Result<Option<PgRunTx>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let chain_lock = match self.lock_chain_for_run(&mut tx, tenant, run, lock).await? {
            RunChainLock::NotMember => None,
            RunChainLock::Locked(chain) => Some(chain),
            RunChainLock::Contended => return Ok(None),
        };
        self.begin_in_transaction(tx, tenant, run, lock, chain_lock)
            .await
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
                    created_at, updated_at, finished_at, parent_run_id, root_run_id, parent_node, \
                    cancel_policy, depth, trace_parent_span, rerun_of, origin, \
                    seg_chain_id, seg_index, seg_completion, seg_successor \
             FROM runs LEFT JOIN LATERAL ( \
               SELECT chain_id AS seg_chain_id, segment_index AS seg_index, \
                      completion_kind AS seg_completion, successor_run_id AS seg_successor \
               FROM chain_segments \
               WHERE chain_segments.tenant_id = runs.tenant_id \
                 AND chain_segments.run_id = runs.id) segment ON true \
             WHERE runs.tenant_id = $1 AND runs.namespace_id = $2 \
               AND ($3::uuid IS NULL OR runs.id < $3) \
               AND ($4::uuid IS NULL OR runs.workflow_id = $4) \
               AND ($5::text IS NULL OR runs.status = $5) \
               AND ($7::text IS NULL OR runs.origin = $7) \
             ORDER BY runs.id DESC LIMIT $6",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(before.map(RunId::as_uuid))
        .bind(filter.workflow.map(WorkflowId::as_uuid))
        .bind(filter.status.map(RunStatus::as_str))
        .bind(i64::from(limit))
        .bind(filter.origin.map(sagat_application::runs::RunOrigin::as_str))
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
            "SELECT seq, event_type, node, attempt, actor, trace_id, occurred_at, detail, \
                    encode(payload_digest, 'hex') AS payload_ref, instance_path_nodes, \
                    instance_path_indices \
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
        // The identity of each step, which the span of an attempt is computed from.
        let steps: BTreeMap<ExecutionInstanceId, Uuid> = sqlx::query(
            "SELECT node, step_id, instance_path_nodes, instance_path_indices \
                 FROM steps WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            let node: String = col(row, "node")?;
            Ok((
                ExecutionInstanceId::in_path(
                    step_instance_path(row)?,
                    NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?,
                ),
                col::<Uuid>(row, "step_id")?,
            ))
        })
        .collect::<Result<_, StoreError>>()?;
        rows.iter()
            .map(|row| {
                let seq: i64 = col(row, "seq")?;
                let attempt: Option<i32> = col(row, "attempt")?;
                let event_type: String = col(row, "event_type")?;
                let node: Option<String> = col(row, "node")?;
                let detail: serde_json::Value = col(row, "detail")?;
                // The steps the event changed, not only the one it names. An event this build cannot
                // decode (one written by a newer engine) says nothing of what it changed: `None`, and a
                // change of the graph that holds one is the whole graph.
                let decoded = decode(&event_type, node.as_deref(), &detail);
                let event_path = event_instance_path(row)?;
                let touched = decoded.as_ref().and_then(|event| {
                    AddressedRunEvent::in_path(event_path.clone(), event.clone())
                        .ok()
                        .map(|addressed| addressed.changed_instances())
                });
                let attempt = attempt.and_then(|a| u32::try_from(a).ok());
                let instance = node
                    .as_deref()
                    .map(|node| {
                        Ok(ExecutionInstanceId::in_path(
                            event_path,
                            NodeId::parse_stored(node).map_err(|_| corrupt("journal node"))?,
                        ))
                    })
                    .transpose()?;
                let span_id = attempt.and_then(|attempt| {
                    instance
                        .as_ref()
                        .and_then(|instance| steps.get(instance))
                        .map(|step| sagat_telemetry::attempt_span_id(*step, attempt).to_string())
                });
                Ok(JournalEntry {
                    seq: u64::try_from(seq).map_err(|_| corrupt("sequence"))?,
                    event_type,
                    node,
                    touched,
                    cause: decoded.as_ref().and_then(RunEvent::cause),
                    span_id,
                    attempt,
                    actor: col(row, "actor")?,
                    trace_id: col(row, "trace_id")?,
                    occurred_at: col(row, "occurred_at")?,
                    payload_ref: col(row, "payload_ref")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()
            .map(Some)
    }

    async fn rerun_source(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<sagat_application::runs::RerunSource>, StoreError> {
        let Some(row) = sqlx::query(
            "SELECT n.slug AS namespace, p.content AS input \
             FROM runs r \
             JOIN namespaces n ON n.tenant_id = r.tenant_id AND n.id = r.namespace_id \
             JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest \
             WHERE r.tenant_id = $1 AND r.id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        Ok(Some(sagat_application::runs::RerunSource {
            namespace: col(&row, "namespace")?,
            input: col(&row, "input")?,
        }))
    }

    async fn graph(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<GraphSnapshot>, StoreError> {
        // One snapshot for the run row and its steps, so the graph is self-consistent.
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
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
            "SELECT s.node, s.instance_path_nodes, s.instance_path_indices, s.status, s.attempt, \
                    s.generation, s.cancel_requested, s.failure_code, s.next_attempt_at, s.due_at, \
                    s.waiting_kind, s.child_run_id, s.switch_case_id, \
                    a.status AS approval_status, a.routed_node AS approval_routed \
             FROM steps s LEFT JOIN approval_requests a \
               ON a.tenant_id = s.tenant_id AND a.run_id = s.run_id \
              AND a.owner_path_nodes = s.instance_path_nodes \
              AND a.owner_path_indices = s.instance_path_indices \
              AND a.owner_node = s.node \
             WHERE s.tenant_id = $1 AND s.run_id = $2 \
             ORDER BY s.instance_path_nodes, s.instance_path_indices, s.node",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let saga_rows = sqlx::query(
            "SELECT node, phase, cause_code, outcome, irreversible FROM run_sagas \
             WHERE tenant_id = $1 AND run_id = $2 ORDER BY node",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let chain_row = sqlx::query(
            "SELECT s.chain_id, s.segment_index, s.predecessor_run_id, s.successor_run_id, \
                    s.completion_kind, c.current_run_id, c.segment_count, c.final_outcome \
             FROM chain_segments s JOIN execution_chains c \
               ON c.tenant_id = s.tenant_id AND c.namespace_id = s.namespace_id \
              AND c.chain_id = s.chain_id \
             WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.run_id = $3",
        )
        .bind(tenant.as_uuid())
        .bind(record.info.namespace_id.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        let chain = chain_row
            .as_ref()
            .map(|row| {
                Ok(ChainView {
                    chain_id: RunId::new(col::<Uuid>(row, "chain_id")?),
                    segment_index: u32::try_from(col::<i32>(row, "segment_index")?)
                        .map_err(|_| corrupt("chain segment index"))?,
                    segment_count: u32::try_from(col::<i32>(row, "segment_count")?)
                        .map_err(|_| corrupt("chain segment count"))?,
                    current_run_id: RunId::new(col::<Uuid>(row, "current_run_id")?),
                    predecessor_run_id: col::<Option<Uuid>>(row, "predecessor_run_id")?
                        .map(RunId::new),
                    successor_run_id: col::<Option<Uuid>>(row, "successor_run_id")?.map(RunId::new),
                    completion_kind: col(row, "completion_kind")?,
                    final_outcome: col(row, "final_outcome")?,
                })
            })
            .transpose()?;
        let sagas = saga_rows
            .iter()
            .map(|row| {
                let node: String = col(row, "node")?;
                let irreversible: Vec<String> = col(row, "irreversible")?;
                Ok(SagaView {
                    node: NodeId::parse_stored(&node).map_err(|_| corrupt("saga node"))?,
                    phase: col(row, "phase")?,
                    cause: col(row, "cause_code")?,
                    outcome: col(row, "outcome")?,
                    irreversible: irreversible
                        .iter()
                        .map(|n| NodeId::parse_stored(n).map_err(|_| corrupt("irreversible node")))
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let steps = step_rows
            .iter()
            .map(|row| {
                let node: String = col(row, "node")?;
                let status: String = col(row, "status")?;
                let attempt: i32 = col(row, "attempt")?;
                let generation: i64 = col(row, "generation")?;
                let waiting: Option<String> = col(row, "waiting_kind")?;
                let node = NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?;
                Ok(StepView {
                    instance: ExecutionInstanceId::in_path(step_instance_path(row)?, node.clone()),
                    node,
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
                        Some("SAGA") => Some("saga"),
                        Some("RECONCILIATION") => Some("reconciliation"),
                        Some("OPERATOR") => Some("operator"),
                        Some("LOOP") => Some("loop"),
                        Some("APPROVAL") => Some("approval"),
                        Some("COLLECTION") => Some("collection"),
                        _ => None,
                    },
                    child_run: col::<Option<Uuid>>(row, "child_run_id")?.map(RunId::new),
                    switch_selection: switch_selection(row)?,
                    approval: approval_view(row)?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let definition = match self.cache.get(record.info.version_id) {
            Some(cached) => cached,
            None => {
                // No transaction is open here, so a connection of its own is fine.
                let mut conn = self.pool.acquire().await.map_err(store_error)?;
                self.definition(&mut conn, tenant, record.info.version_id)
                    .await?
            }
        };
        Ok(Some(GraphSnapshot {
            run: record,
            definition,
            steps,
            sagas,
            chain,
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

/// Pins the effective instance and depth ceilings of a run that was just inserted.
///
/// The row of `run_instance_usage` starts at the platform ceilings (100000 and 8). The author's
/// `limits` (D297) can only lower them, so the stored value is the smaller of the two, and a
/// tenant policy that already lowered the row is never raised by a later, laxer definition.
pub(crate) async fn capture_run_limits(
    tx: &mut sqlx::PgConnection,
    tenant: Uuid,
    run: Uuid,
    limits: sagat_domain::workflow::WorkflowLimits,
) -> Result<(), StoreError> {
    let updated = sqlx::query(
        "UPDATE run_instance_usage SET max_instances = least(max_instances, $3), \
                max_depth = least(max_depth, $4) \
         WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant)
    .bind(run)
    .bind(i32::try_from(limits.max_instances()).unwrap_or(i32::MAX))
    .bind(i16::from(limits.max_depth()))
    .execute(&mut *tx)
    .await
    .map_err(store_error)?;
    if updated.rows_affected() != 1 {
        return Err(corrupt("run instance usage"));
    }
    Ok(())
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
    root: RunId,
    child: RunId,
) -> Result<(), StoreError> {
    let locked = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM runs WHERE tenant_id = $1 AND id = $2 FOR NO KEY UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(parent.run.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    if locked.is_none() {
        return Err(StoreError::MissingReference);
    }
    // One tree, one budget. The advisory lock serialises only the creators of this tree's runs (each
    // already holds its own parent's write lock), so the count cannot be overshot by racing
    // siblings. NO KEY UPDATE still excludes parent commands and another reservation, but lets
    // another creator/child handoff validate a root FK with KEY SHARE while holding this tree
    // lock. An UPDATE-mode parent lock would deadlock with that late FK at depth two.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("run-tree:{tenant}:{root}"))
        .execute(&mut **tx)
        .await
        .map_err(store_error)?;
    let members: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id = $1 AND root_run_id = $2")
            .bind(tenant.as_uuid())
            .bind(root.as_uuid())
            .fetch_one(&mut **tx)
            .await
            .map_err(store_error)?;
    if usize::try_from(members).unwrap_or(usize::MAX) >= sagat_application::execution::MAX_TREE_RUNS
    {
        return Err(StoreError::QuotaExceeded {
            what: sagat_application::execution::TREE_LIMIT,
        });
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

// `get` and `list` also read the run's chain segment as four extra columns (`run_record` reads them
// when present): one lateral index probe (`chain_segments_run_unique`) that yields nothing for a run
// outside any chain.
const RUN_SELECT: &str = "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, \
     run_version, stop_reason, paused, correlation_id, started_by, trace_id, deadline_at, \
     created_at, updated_at, finished_at, parent_run_id, root_run_id, parent_node, cancel_policy, depth, \
     trace_parent_span, rerun_of, origin, seg_chain_id, seg_index, seg_completion, seg_successor \
     FROM runs LEFT JOIN LATERAL ( \
       SELECT chain_id AS seg_chain_id, segment_index AS seg_index, \
              completion_kind AS seg_completion, successor_run_id AS seg_successor \
       FROM chain_segments \
       WHERE chain_segments.tenant_id = runs.tenant_id AND chain_segments.run_id = runs.id) segment \
     ON true \
     WHERE runs.tenant_id = $1 AND runs.id = $2";

/// The columns a command transaction reads about a run; `$tail` is the lock clause, if any.
macro_rules! run_columns_sql {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id, \
     status, run_version, stop_reason, stop_code, paused, input_digest, correlation_id, started_by, \
     trace_id, deadline_at, created_at, updated_at, finished_at, parent_run_id, root_run_id, parent_node, \
     cancel_policy, depth, trace_parent_span, rerun_of, origin, clock_timestamp() AS db_now \
     FROM runs WHERE tenant_id = $1 AND id = $2",
            $tail
        )
    };
}

const RUN_FOR_UPDATE: &str = run_columns_sql!(" FOR UPDATE");

const RUN_FOR_UPDATE_SKIP_LOCKED: &str = run_columns_sql!(" FOR UPDATE SKIP LOCKED");

/// The same read without a lock, for the bounded snapshot of a scope transition (D523).
pub(crate) const RUN_SNAPSHOT: &str = run_columns_sql!("");

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

pub(crate) fn run_record(row: &PgRow) -> Result<RunRecord, StoreError> {
    let status: String = col(row, "status")?;
    let version: i64 = col(row, "run_version")?;
    let stop: Option<String> = col(row, "stop_reason")?;
    Ok(RunRecord {
        info: RunInfo {
            origin: col::<Option<String>>(row, "origin")?
                .map(|s| s.parse().map_err(|()| corrupt("run origin")))
                .transpose()?,
            run_id: RunId::new(col::<Uuid>(row, "id")?),
            tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
            namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
            workflow_id: WorkflowId::new(col::<Uuid>(row, "workflow_id")?),
            version_id: WorkflowVersionId::new(col::<Uuid>(row, "workflow_version_id")?),
            started_by: col(row, "started_by")?,
            correlation_id: col(row, "correlation_id")?,
            trace_id: col(row, "trace_id")?,
            parent_span: col(row, "trace_parent_span")?,
            rerun_of: col::<Option<Uuid>>(row, "rerun_of")?.map(RunId::new),
            deadline_at: col(row, "deadline_at")?,
            created_at: col(row, "created_at")?,
            parent: parent_link(row)?,
            depth: u8::try_from(col::<i16>(row, "depth")?).map_err(|_| corrupt("run depth"))?,
            root: RunId::new(col::<Uuid>(row, "root_run_id")?),
        },
        status: status.parse().map_err(|_| corrupt("run status"))?,
        version: u64::try_from(version).map_err(|_| corrupt("run version"))?,
        stop_reason: match stop.as_deref() {
            None => None,
            Some(reason) => Some(
                reason
                    .parse::<StopReason>()
                    .map_err(|()| corrupt("stop reason"))?,
            ),
        },
        paused: col(row, "paused")?,
        updated_at: col(row, "updated_at")?,
        finished_at: col(row, "finished_at")?,
        chain: run_segment(row)?,
    })
}

/// The segment columns of `get` and `list`; a row from a command transaction has none.
fn run_segment(row: &PgRow) -> Result<Option<RunSegment>, StoreError> {
    let Ok(chain) = sqlx::Row::try_get::<Option<Uuid>, _>(row, "seg_chain_id") else {
        return Ok(None);
    };
    let Some(chain) = chain else {
        return Ok(None);
    };
    let index: i32 = col(row, "seg_index")?;
    let completion: Option<String> = col(row, "seg_completion")?;
    Ok(Some(RunSegment {
        chain_id: RunId::new(chain),
        index: u16::try_from(index).map_err(|_| corrupt("chain segment index"))?,
        completion_kind: match completion.as_deref() {
            None => None,
            Some("CONTINUED") => Some(sagat_domain::execution::SegmentCompletionKind::Continued),
            Some("FINAL") => Some(sagat_domain::execution::SegmentCompletionKind::Final),
            Some(_) => return Err(corrupt("chain completion kind")),
        },
        successor: col::<Option<Uuid>>(row, "seg_successor")?.map(RunId::new),
    }))
}

pub(crate) fn instance_path(
    path_nodes: Vec<String>,
    path_indices: Vec<i32>,
    what: &str,
) -> Result<InstancePath, StoreError> {
    if path_nodes.len() != path_indices.len() {
        return Err(corrupt(what));
    }
    let mut path = InstancePath::root();
    for (scope, index) in path_nodes.into_iter().zip(path_indices) {
        let scope = NodeId::parse_stored(&scope).map_err(|_| corrupt(what))?;
        let index = u32::try_from(index).map_err(|_| corrupt(what))?;
        path = path.child(scope, index).map_err(|_| corrupt(what))?;
    }
    Ok(path)
}

pub(crate) fn stored_path_parts(path: &InstancePath) -> (Vec<String>, Vec<i32>) {
    path.segments()
        .iter()
        .map(|segment| {
            (
                segment.scope_node().as_str().to_owned(),
                i32::try_from(segment.iteration_index()).unwrap_or(i32::MAX),
            )
        })
        .unzip()
}

pub(crate) fn step_instance_path(row: &PgRow) -> Result<InstancePath, StoreError> {
    instance_path(
        col(row, "instance_path_nodes")?,
        col(row, "instance_path_indices")?,
        "step instance path",
    )
}

fn event_instance_path(row: &PgRow) -> Result<InstancePath, StoreError> {
    match (
        col::<Option<Vec<String>>>(row, "instance_path_nodes")?,
        col::<Option<Vec<i32>>>(row, "instance_path_indices")?,
    ) {
        (None, None) => Ok(InstancePath::root()),
        (Some(nodes), Some(indices)) => instance_path(nodes, indices, "journal instance path"),
        _ => Err(corrupt("journal instance path")),
    }
}

pub(crate) fn stored_digest(bytes: Vec<u8>, what: &str) -> Result<Digest, StoreError> {
    <[u8; 32]>::try_from(bytes)
        .map(Digest::new)
        .map_err(|_| corrupt(what))
}

fn restored_return_instance(
    path_nodes: Option<Vec<String>>,
    path_indices: Option<Vec<i32>>,
    node: Option<String>,
    instance_id: Option<Uuid>,
    projected_id: Option<Uuid>,
) -> Result<ExecutionInstanceId, StoreError> {
    let (Some(path_nodes), Some(path_indices), Some(node), Some(instance_id), Some(projected_id)) =
        (path_nodes, path_indices, node, instance_id, projected_id)
    else {
        return Err(corrupt("collection return identity"));
    };
    if instance_id != projected_id {
        return Err(corrupt("collection return identity"));
    }
    Ok(ExecutionInstanceId::in_path(
        instance_path(path_nodes, path_indices, "collection return path")?,
        NodeId::parse_stored(&node).map_err(|_| corrupt("collection return node"))?,
    ))
}

pub(crate) fn step_state(row: &PgRow) -> Result<StepState, StoreError> {
    let status: String = col(row, "status")?;
    let attempt: i32 = col(row, "attempt")?;
    let generation: i64 = col(row, "generation")?;
    let waiting_kind: Option<String> = col(row, "waiting_kind")?;
    let due_at: Option<OffsetDateTime> = col(row, "due_at")?;
    let key: Option<String> = col(row, "correlation_key")?;
    let waiting = match (waiting_kind.as_deref(), due_at) {
        (None, _) => None,
        (Some("BRANCHES"), _) => Some(WaitingOn::Branches),
        (Some("COLLECTION"), _) => Some(WaitingOn::Collection),
        (Some("LOOP"), _) => Some(WaitingOn::Loop),
        (Some("SAGA"), _) => Some(WaitingOn::Saga),
        (Some("RECONCILIATION"), _) => Some(WaitingOn::Reconciliation),
        (Some("OPERATOR"), _) => Some(WaitingOn::Operator {
            code: FailureCode::parse(
                &col::<Option<String>>(row, "failure_code")?
                    .ok_or_else(|| corrupt("operator wait"))?,
            )
            .map_err(|_| corrupt("operator wait"))?,
        }),
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
        (Some("APPROVAL"), _) => Some(WaitingOn::Approval {
            request_id: ApprovalRequestId::new(
                col::<Option<Uuid>>(row, "approval_request_id")?
                    .ok_or_else(|| corrupt("approval request"))?,
            ),
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
    )
    .with_restored_switch_selection(switch_selection(row)?))
}

fn approval_view(row: &PgRow) -> Result<Option<ApprovalView>, StoreError> {
    use sagat_domain::execution::ApprovalStatus;
    let Some(status) = col::<Option<String>>(row, "approval_status")? else {
        return Ok(None);
    };
    let status = match status.as_str() {
        "PENDING" => ApprovalStatus::Pending,
        "APPROVED" => ApprovalStatus::Approved,
        "REJECTED" => ApprovalStatus::Rejected,
        "EXPIRED" => ApprovalStatus::Expired,
        "CANCELLED" => ApprovalStatus::Cancelled,
        _ => return Err(corrupt("approval request status")),
    };
    let routed_node = col::<Option<String>>(row, "approval_routed")?
        .map(|node| NodeId::parse_stored(&node).map_err(|_| corrupt("approval route")))
        .transpose()?;
    Ok(Some(ApprovalView {
        status,
        routed_node,
    }))
}

fn switch_selection(row: &PgRow) -> Result<Option<SwitchSelection>, StoreError> {
    match col::<Option<String>>(row, "switch_case_id")? {
        None => Ok(None),
        Some(case) if case.is_empty() => Ok(Some(SwitchSelection::Default)),
        Some(case) => NodeId::parse(&case)
            .map(SwitchSelection::Case)
            .map(Some)
            .map_err(|_| corrupt("switch case id")),
    }
}

fn stored_switch_case_id(step: &StepState) -> Option<&str> {
    step.switch_selection().map(|selection| match selection {
        SwitchSelection::Case(id) => id.as_str(),
        SwitchSelection::Default => "",
    })
}

pub(crate) fn validate_scoped_control_projection(
    node: Option<&Node>,
    step: &StepState,
) -> Result<(), StoreError> {
    let valid = match node.map(Node::kind) {
        Some(NodeKind::Timer { .. }) => step.is_valid_scoped_timer(),
        Some(NodeKind::WaitEvent { .. }) => {
            node.is_some_and(|node| step.is_valid_scoped_event_wait(node))
        }
        _ if matches!(
            step.waiting_on(),
            Some(WaitingOn::Timer { .. } | WaitingOn::Event { .. })
        ) =>
        {
            false
        }
        _ => node.map_or_else(
            || step.condition().is_none() && step.switch_selection().is_none(),
            |node| step.is_valid_scoped_control(node),
        ),
    };
    if !valid {
        return Err(corrupt("scoped control projection"));
    }
    Ok(())
}

pub(crate) fn validate_scoped_control_row(
    node: Option<&Node>,
    step: &StepState,
    row: &PgRow,
) -> Result<(), StoreError> {
    validate_scoped_control_projection(node, step)?;
    if let Some(node) = node
        && matches!(node.kind(), NodeKind::WaitEvent { .. })
    {
        scoped_events::validate_event_row(node, step, row)?;
    }
    if let Some(node) = node
        && matches!(node.kind(), NodeKind::Timer { .. })
        && (!col::<bool>(row, "timer_metadata_valid")?
            || col::<String>(row, "stored_node_kind")? != "timer")
    {
        return Err(corrupt("scoped timer physical metadata"));
    }
    if let Some(node) = node
        && matches!(
            node.kind(),
            NodeKind::Condition { .. } | NodeKind::Switch { .. }
        )
        && (!col::<bool>(row, "control_metadata_empty")?
            || col::<String>(row, "stored_node_kind")? != node.kind().label())
    {
        return Err(corrupt("scoped control physical metadata"));
    }
    Ok(())
}

/// The varying fields of a validated timer row, retained until its guarded write.
/// All other timer lifecycle fields are fixed by `scoped_timer_metadata_valid!`.
#[derive(Clone, Copy)]
struct TimerReadState {
    status: StepStatus,
    routed: bool,
    due_at: Option<OffsetDateTime>,
}

impl TimerReadState {
    fn from_step(step: &StepState) -> Self {
        Self {
            status: step.status(),
            routed: step.routed(),
            due_at: match step.waiting_on() {
                Some(WaitingOn::Timer { due_at }) => Some(*due_at),
                _ => None,
            },
        }
    }
}

struct RestoredCollectionRows {
    iterations: Vec<CollectionIterationRestore>,
    completed_count: i32,
    active_count: i32,
}

/// Dispatch is a projection of the pinned entry kind, never an independent worker capability.
pub(crate) fn validate_collection_entry_dispatch(
    definition: &WorkflowDefinition,
    entry: &ExecutionInstanceId,
    row: &PgRow,
) -> Result<(), StoreError> {
    let dispatch: String = col(row, "entry_dispatch")?;
    let activity: Option<String> = col(row, "activity")?;
    let outbox: Option<Uuid> = col(row, "outbox_id")?;
    let journal: Option<String> = col(row, "journal_dispatch")?;
    let valid = match definition.node_for_instance(entry).map(Node::kind) {
        Some(NodeKind::Activity {
            activity: expected, ..
        }) => {
            dispatch == "ACTIVITY"
                && activity.as_deref() == Some(expected.as_str())
                && outbox.is_some()
                && journal.as_deref().is_none_or(|kind| kind == "ACTIVITY")
        }
        Some(NodeKind::Condition { .. } | NodeKind::Switch { .. } | NodeKind::Timer { .. }) => {
            dispatch == "SCHEDULER"
                && activity.is_none()
                && outbox.is_none()
                && journal.as_deref() == Some("SCHEDULER")
        }
        _ => false,
    };
    if !valid {
        return Err(corrupt("collection entry dispatch"));
    }
    Ok(())
}

async fn load_collections(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    run: RunId,
    definition: &WorkflowDefinition,
    state: &mut RunState,
) -> Result<(), StoreError> {
    let scopes = sqlx::query(
        "SELECT scope_id, owner_path_nodes, owner_path_indices, owner_node, kind, lifecycle, \
                snapshot_digest, item_count, max_items, max_concurrency, next_admission_index, \
                admitted_count, completed_count, active_count, result_digest, finish_sequence, \
                finish_outbox_id, drain_kind, drain_iteration, drain_failure_code, drain_sequence, \
                drain_outbox_id \
         FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 \
         ORDER BY owner_path_nodes, owner_path_indices, owner_node",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    if scopes.is_empty() {
        return Ok(());
    }

    let iteration_rows = sqlx::query(
        "SELECT i.scope_id, i.iteration_index, i.entry_path_nodes, i.entry_path_indices, \
                i.entry_node, i.entry_instance_id, i.item_digest, i.lifecycle, i.slot_held, \
                i.remaining_reservations, i.return_path_nodes, i.return_path_indices, \
                i.return_node, i.return_instance_id, i.result_digest, i.checkpoint_sequence, \
                i.drained_sequence, i.drain_outbox_id, i.entry_dispatch,i.activity,i.outbox_id, \
                admission.detail->>'entry_dispatch' AS journal_dispatch, \
                entry_step.step_id AS projected_entry_id, \
                return_step.step_id AS projected_return_id \
         FROM collection_iterations i \
         LEFT JOIN run_events admission ON admission.tenant_id=i.tenant_id AND admission.run_id=i.run_id \
          AND admission.seq=i.admitted_sequence AND admission.event_type='collection.iteration_admitted' \
         LEFT JOIN steps entry_step \
           ON entry_step.tenant_id = i.tenant_id AND entry_step.run_id = i.run_id \
          AND entry_step.instance_path_nodes = i.entry_path_nodes \
          AND entry_step.instance_path_indices = i.entry_path_indices \
          AND entry_step.node = i.entry_node \
         LEFT JOIN steps return_step \
           ON return_step.tenant_id = i.tenant_id AND return_step.run_id = i.run_id \
          AND return_step.instance_path_nodes = i.return_path_nodes \
          AND return_step.instance_path_indices = i.return_path_indices \
          AND return_step.node = i.return_node \
         WHERE i.tenant_id = $1 AND i.run_id = $2 \
         ORDER BY i.scope_id, i.iteration_index",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    let checkpoint_rows = sqlx::query(
        "SELECT scope_id, iteration_index, return_instance_id, result_digest, journal_sequence \
         FROM iteration_checkpoints WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    let mut checkpoints = BTreeMap::new();
    for row in &checkpoint_rows {
        let key = (
            col::<Uuid>(row, "scope_id")?,
            u32::try_from(col::<i32>(row, "iteration_index")?)
                .map_err(|_| corrupt("checkpoint iteration index"))?,
        );
        if checkpoints
            .insert(
                key,
                (
                    col::<Uuid>(row, "return_instance_id")?,
                    stored_digest(col(row, "result_digest")?, "checkpoint result digest")?,
                    col::<i64>(row, "journal_sequence")?,
                ),
            )
            .is_some()
        {
            return Err(corrupt("duplicate iteration checkpoint"));
        }
    }
    let mut iterations: BTreeMap<Uuid, RestoredCollectionRows> = BTreeMap::new();
    for row in &iteration_rows {
        let scope_id: Uuid = col(row, "scope_id")?;
        let index = u32::try_from(col::<i32>(row, "iteration_index")?)
            .map_err(|_| corrupt("collection iteration index"))?;
        let lifecycle: String = col(row, "lifecycle")?;
        let return_nodes = col::<Option<Vec<String>>>(row, "return_path_nodes")?;
        let return_indices = col::<Option<Vec<i32>>>(row, "return_path_indices")?;
        let return_node = col::<Option<String>>(row, "return_node")?;
        let return_id = col::<Option<Uuid>>(row, "return_instance_id")?;
        let result_digest = col::<Option<Vec<u8>>>(row, "result_digest")?;
        let checkpoint_sequence = col::<Option<i64>>(row, "checkpoint_sequence")?;
        let drained_sequence = col::<Option<i64>>(row, "drained_sequence")?;
        let drain_outbox_id = col::<Option<Uuid>>(row, "drain_outbox_id")?;
        if col::<Uuid>(row, "entry_instance_id")?
            != col::<Option<Uuid>>(row, "projected_entry_id")?
                .ok_or_else(|| corrupt("collection entry projection"))?
        {
            return Err(corrupt("collection entry identity"));
        }
        let path = instance_path(
            col(row, "entry_path_nodes")?,
            col(row, "entry_path_indices")?,
            "collection entry path",
        )?;
        let node: String = col(row, "entry_node")?;
        let entry = ExecutionInstanceId::in_path(
            path,
            NodeId::parse_stored(&node).map_err(|_| corrupt("collection entry node"))?,
        );
        validate_collection_entry_dispatch(definition, &entry, row)?;
        let item_digest = stored_digest(col(row, "item_digest")?, "collection item digest")?;
        let (restored, completed) = match lifecycle.as_str() {
            "ADMITTED"
                if col::<bool>(row, "slot_held")?
                    && col::<i32>(row, "remaining_reservations")? > 0
                    && return_nodes.is_none()
                    && return_indices.is_none()
                    && return_node.is_none()
                    && return_id.is_none()
                    && result_digest.is_none()
                    && checkpoint_sequence.is_none()
                    && drained_sequence.is_none()
                    && drain_outbox_id.is_none() =>
            {
                (
                    CollectionIterationRestore::admitted(index, entry, item_digest),
                    false,
                )
            }
            "CHECKPOINTED"
                if !col::<bool>(row, "slot_held")?
                    && drained_sequence.is_none()
                    && drain_outbox_id.is_none() =>
            {
                let return_id_value =
                    return_id.ok_or_else(|| corrupt("collection checkpoint return identity"))?;
                let result = stored_digest(
                    result_digest.ok_or_else(|| corrupt("collection checkpoint result digest"))?,
                    "collection checkpoint result digest",
                )?;
                let sequence =
                    checkpoint_sequence.ok_or_else(|| corrupt("collection checkpoint sequence"))?;
                let checkpoint = checkpoints
                    .remove(&(scope_id, index))
                    .ok_or_else(|| corrupt("collection checkpoint projection"))?;
                // What is left reserved is the derived work (compensations and lookups) the body
                // declares and the iteration never needed: it stays counted against the run's
                // lifetime budget (D384).
                if checkpoint != (return_id_value, result, sequence)
                    || col::<i32>(row, "remaining_reservations")? < 0
                {
                    return Err(corrupt("collection checkpoint projection"));
                }
                (
                    CollectionIterationRestore::checkpointed(
                        index,
                        entry,
                        item_digest,
                        restored_return_instance(
                            return_nodes,
                            return_indices,
                            return_node,
                            Some(return_id_value),
                            col(row, "projected_return_id")?,
                        )?,
                        result,
                    ),
                    true,
                )
            }
            "DRAINED"
                if !col::<bool>(row, "slot_held")?
                    && col::<i32>(row, "remaining_reservations")? >= 0
                    && return_nodes.is_none()
                    && return_indices.is_none()
                    && return_node.is_none()
                    && return_id.is_none()
                    && result_digest.is_none()
                    && checkpoint_sequence.is_none()
                    && drained_sequence.is_some()
                    && drain_outbox_id.is_some() =>
            {
                (
                    CollectionIterationRestore::drained(index, entry, item_digest),
                    true,
                )
            }
            _ => return Err(corrupt("unsupported collection iteration projection")),
        };
        let rows = iterations
            .entry(scope_id)
            .or_insert_with(|| RestoredCollectionRows {
                iterations: Vec::new(),
                completed_count: 0,
                active_count: 0,
            });
        rows.iterations.push(restored);
        if completed {
            rows.completed_count += 1;
        } else {
            rows.active_count += 1;
        }
    }
    if !checkpoints.is_empty() {
        return Err(corrupt("orphan iteration checkpoint"));
    }

    for row in &scopes {
        let scope_id: Uuid = col(row, "scope_id")?;
        let kind: String = col(row, "kind")?;
        let lifecycle: String = col(row, "lifecycle")?;
        let admitted = iterations
            .remove(&scope_id)
            .unwrap_or(RestoredCollectionRows {
                iterations: Vec::new(),
                completed_count: 0,
                active_count: 0,
            });
        let admitted_count = i32::try_from(admitted.iterations.len()).unwrap_or(i32::MAX);
        let stored_result_digest = col::<Option<Vec<u8>>>(row, "result_digest")?
            .map(|digest| stored_digest(digest, "collection result digest"))
            .transpose()?;
        let owner_path = instance_path(
            col(row, "owner_path_nodes")?,
            col(row, "owner_path_indices")?,
            "collection owner path",
        )?;
        let owner_node: String = col(row, "owner_node")?;
        let parsed_owner_node =
            NodeId::parse_stored(&owner_node).map_err(|_| corrupt("collection owner node"))?;
        let drain_reason = match (
            col::<Option<String>>(row, "drain_kind")?.as_deref(),
            col::<Option<i32>>(row, "drain_iteration")?,
            col::<Option<String>>(row, "drain_failure_code")?,
            col::<Option<i64>>(row, "drain_sequence")?,
            col::<Option<Uuid>>(row, "drain_outbox_id")?,
        ) {
            (None, None, None, None, None) => None,
            (Some("FAILED"), Some(iteration), Some(code), Some(_), Some(_)) => {
                Some(CollectionDrainReason::Failed {
                    iteration: u32::try_from(iteration)
                        .map_err(|_| corrupt("collection drain iteration"))?,
                    code: FailureCode::parse(&code)
                        .map_err(|_| corrupt("collection drain failure code"))?,
                })
            }
            (Some("RUN_DATA_LIMIT"), None, Some(code), Some(sequence), Some(_))
                if code == FailureCode::run_data_limit().as_str() && sequence > 0 =>
            {
                Some(CollectionDrainReason::RunDataLimit)
            }
            (Some("STORAGE_QUOTA"), None, Some(code), Some(_), Some(_))
                if code == FailureCode::storage_quota_exceeded().as_str() =>
            {
                Some(CollectionDrainReason::StorageQuotaExceeded)
            }
            (Some("CANCELLED"), None, None, Some(_), Some(_)) => {
                Some(CollectionDrainReason::Cancelled)
            }
            _ => return Err(corrupt("collection drain projection")),
        };
        let phase = match lifecycle.as_str() {
            "OPEN"
                if stored_result_digest.is_none()
                    && col::<Option<i64>>(row, "finish_sequence")?.is_none()
                    && col::<Option<Uuid>>(row, "finish_outbox_id")?.is_none()
                    && drain_reason.is_none() =>
            {
                CollectionPhase::Open
            }
            "SUCCEEDED"
                if stored_result_digest.is_some()
                    && col::<Option<i64>>(row, "finish_sequence")?.is_some()
                    && col::<Option<Uuid>>(row, "finish_outbox_id")?.is_some()
                    && drain_reason.is_none() =>
            {
                CollectionPhase::Succeeded
            }
            "DRAINING"
                if stored_result_digest.is_none()
                    && col::<Option<i64>>(row, "finish_sequence")?.is_none()
                    && col::<Option<Uuid>>(row, "finish_outbox_id")?.is_none()
                    && drain_reason.is_some() =>
            {
                CollectionPhase::Draining
            }
            "FAILED"
                if stored_result_digest.is_none()
                    && col::<Option<i64>>(row, "finish_sequence")?
                        .is_some_and(|sequence| sequence > 0)
                    && col::<Option<Uuid>>(row, "finish_outbox_id")?.is_some()
                    && matches!(&drain_reason, Some(CollectionDrainReason::RunDataLimit)) =>
            {
                CollectionPhase::Failed
            }
            "FAILED"
                if stored_result_digest.is_none()
                    && col::<Option<i64>>(row, "finish_sequence")?.is_some()
                    && col::<Option<Uuid>>(row, "finish_outbox_id")?.is_some()
                    && matches!(
                        &drain_reason,
                        Some(
                            CollectionDrainReason::Failed { .. }
                                | CollectionDrainReason::StorageQuotaExceeded
                        )
                    ) =>
            {
                CollectionPhase::Failed
            }
            "CANCELLED"
                if stored_result_digest.is_none()
                    && col::<Option<i64>>(row, "finish_sequence")?.is_some()
                    && col::<Option<Uuid>>(row, "finish_outbox_id")?.is_some()
                    && matches!(&drain_reason, Some(CollectionDrainReason::Cancelled)) =>
            {
                CollectionPhase::Cancelled
            }
            _ => return Err(corrupt("unsupported collection scope projection")),
        };
        if kind != "COLLECTION"
            || col::<i32>(row, "next_admission_index")? != admitted_count
            || col::<i32>(row, "admitted_count")? != admitted_count
            || col::<i32>(row, "completed_count")? != admitted.completed_count
            || col::<i32>(row, "active_count")? != admitted.active_count
        {
            return Err(corrupt("unsupported collection scope projection"));
        }
        state
            .restore_collection_progress(
                ExecutionInstanceId::in_path(owner_path, parsed_owner_node),
                phase,
                u32::try_from(col::<i32>(row, "item_count")?)
                    .map_err(|_| corrupt("collection item count"))?,
                u32::try_from(col::<i32>(row, "max_items")?)
                    .map_err(|_| corrupt("collection item limit"))?,
                u16::try_from(col::<i32>(row, "max_concurrency")?)
                    .map_err(|_| corrupt("collection concurrency limit"))?,
                stored_digest(col(row, "snapshot_digest")?, "collection snapshot digest")?,
                stored_result_digest,
                drain_reason,
                admitted.iterations,
            )
            .map_err(|_| corrupt("collection projection"))?;
    }
    if !iterations.is_empty() {
        return Err(corrupt("collection iteration owner"));
    }
    Ok(())
}

#[derive(Debug)]
struct RestoredLoopRows {
    iterations: Vec<LoopIterationRestore>,
    active_iteration: Option<u32>,
    last_checkpoint_sequence: Option<i64>,
}

#[derive(Debug)]
struct StoredLoopCheckpoint {
    return_instance_id: Uuid,
    result_digest: Digest,
    carry_digest: Digest,
    condition: bool,
    checked_at: OffsetDateTime,
    continuation: LoopContinuation,
    journal_sequence: i64,
}

#[derive(Debug)]
struct StoredLoopTimer {
    timer_id: Uuid,
    next_iteration: u32,
    due_at: OffsetDateTime,
    lifecycle: String,
    scheduled_sequence: i64,
    scheduled_outbox_id: Uuid,
    fired_sequence: Option<i64>,
    fired_outbox_id: Option<Uuid>,
}

fn stored_loop_continuation(
    kind: &str,
    due_at: Option<OffsetDateTime>,
    timer_id: Option<Uuid>,
) -> Result<LoopContinuation, StoreError> {
    match (kind, due_at, timer_id) {
        ("CONTINUE", None, None) => Ok(LoopContinuation::Continue),
        ("DELAY", Some(due_at), Some(_)) => Ok(LoopContinuation::DelayUntil(due_at)),
        ("COMPLETE", None, None) => Ok(LoopContinuation::Complete),
        ("ITERATION_LIMIT", None, None) => Ok(LoopContinuation::IterationLimit),
        ("INSTANCE_LIMIT", None, None) => Ok(LoopContinuation::InstanceLimit),
        _ => Err(corrupt("loop checkpoint continuation")),
    }
}

#[allow(clippy::too_many_lines)]
async fn load_loops(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    run: RunId,
    state: &mut RunState,
) -> Result<(), StoreError> {
    let scopes = sqlx::query(
        "SELECT scope.owner_instance_id,scope.owner_path_nodes,scope.owner_path_indices, \
                scope.owner_node,scope.kind,scope.lifecycle,scope.max_iterations, \
                scope.deadline_at,scope.initial_carry_digest,scope.current_carry_digest, \
                scope.initial_condition,scope.next_iteration,scope.active_iteration, \
                scope.delay_due_at,scope.entered_sequence,scope.last_checkpoint_sequence, \
                scope.drain_kind,scope.drain_iteration,scope.drain_failure_code, \
                scope.drain_sequence,scope.drain_outbox_id,scope.finish_sequence, \
                scope.finish_outbox_id,scope.created_at,owner.step_id AS projected_owner_id, \
                scope.namespace_id = runs.namespace_id AS namespace_matches \
         FROM loop_scopes scope \
         JOIN runs ON runs.tenant_id=scope.tenant_id AND runs.id=scope.run_id \
         LEFT JOIN steps owner \
           ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
          AND owner.instance_path_nodes=scope.owner_path_nodes \
          AND owner.instance_path_indices=scope.owner_path_indices \
          AND owner.node=scope.owner_node \
         WHERE scope.tenant_id=$1 AND scope.run_id=$2 \
         ORDER BY scope.owner_path_nodes,scope.owner_path_indices,scope.owner_node",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    if scopes.is_empty() {
        return Ok(());
    }

    let checkpoint_rows = sqlx::query(
        "SELECT owner_instance_id,iteration_index,return_instance_id,result_digest, \
                carry_digest,condition,checked_at,continuation,delay_due_at,timer_id, \
                journal_sequence,outbox_id \
         FROM loop_checkpoints WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    let timer_rows = sqlx::query(
        "SELECT owner_instance_id,checkpoint_iteration,timer_id,next_iteration,due_at,lifecycle, \
                scheduled_sequence,scheduled_outbox_id,fired_sequence,fired_outbox_id \
         FROM loop_timers WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    let mut timers = BTreeMap::new();
    for row in &timer_rows {
        let key = (
            col::<Uuid>(row, "owner_instance_id")?,
            u32::try_from(col::<i32>(row, "checkpoint_iteration")?)
                .map_err(|_| corrupt("loop timer checkpoint iteration"))?,
        );
        let timer = StoredLoopTimer {
            timer_id: col(row, "timer_id")?,
            next_iteration: u32::try_from(col::<i32>(row, "next_iteration")?)
                .map_err(|_| corrupt("loop timer next iteration"))?,
            due_at: col(row, "due_at")?,
            lifecycle: col(row, "lifecycle")?,
            scheduled_sequence: col(row, "scheduled_sequence")?,
            scheduled_outbox_id: col(row, "scheduled_outbox_id")?,
            fired_sequence: col(row, "fired_sequence")?,
            fired_outbox_id: col(row, "fired_outbox_id")?,
        };
        if timers.insert(key, timer).is_some() {
            return Err(corrupt("duplicate loop timer"));
        }
    }
    let mut checkpoints = BTreeMap::new();
    let mut waiting_timers = BTreeMap::new();
    for row in &checkpoint_rows {
        let owner = col::<Uuid>(row, "owner_instance_id")?;
        let index = u32::try_from(col::<i32>(row, "iteration_index")?)
            .map_err(|_| corrupt("loop checkpoint iteration"))?;
        let continuation_name: String = col(row, "continuation")?;
        let due_at = col::<Option<OffsetDateTime>>(row, "delay_due_at")?;
        let timer_id = col::<Option<Uuid>>(row, "timer_id")?;
        let journal_sequence = col::<i64>(row, "journal_sequence")?;
        let outbox_id = col::<Uuid>(row, "outbox_id")?;
        let continuation = stored_loop_continuation(&continuation_name, due_at, timer_id)?;
        if let Some(expected_timer_id) = timer_id {
            let timer = timers
                .remove(&(owner, index))
                .ok_or_else(|| corrupt("loop checkpoint timer"))?;
            let next = index
                .checked_add(1)
                .ok_or_else(|| corrupt("loop timer next iteration"))?;
            if timer.timer_id != expected_timer_id
                || timer.next_iteration != next
                || Some(timer.due_at) != due_at
                || timer.scheduled_sequence != journal_sequence
                || timer.scheduled_outbox_id != outbox_id
            {
                return Err(corrupt("loop checkpoint timer"));
            }
            match (
                timer.lifecycle.as_str(),
                timer.fired_sequence,
                timer.fired_outbox_id,
            ) {
                ("WAITING", None, None) => {
                    if waiting_timers.insert(owner, index).is_some() {
                        return Err(corrupt("duplicate waiting loop timer"));
                    }
                }
                ("FIRED", Some(fired), Some(_)) if fired > timer.scheduled_sequence => {}
                ("CANCELLED", None, None) => {}
                _ => return Err(corrupt("loop timer lifecycle")),
            }
        } else if timers.contains_key(&(owner, index)) {
            return Err(corrupt("unexpected loop timer"));
        }
        let checkpoint = StoredLoopCheckpoint {
            return_instance_id: col(row, "return_instance_id")?,
            result_digest: stored_digest(col(row, "result_digest")?, "loop result digest")?,
            carry_digest: stored_digest(col(row, "carry_digest")?, "loop carry digest")?,
            condition: col(row, "condition")?,
            checked_at: col(row, "checked_at")?,
            continuation,
            journal_sequence,
        };
        if checkpoints.insert((owner, index), checkpoint).is_some() {
            return Err(corrupt("duplicate loop checkpoint"));
        }
    }
    if !timers.is_empty() {
        return Err(corrupt("orphan loop timer"));
    }

    let iteration_rows = sqlx::query(
        "SELECT iteration.owner_instance_id,iteration.iteration_index, \
                iteration.entry_instance_id,iteration.entry_path_nodes, \
                iteration.entry_path_indices,iteration.entry_node,iteration.lifecycle, \
                iteration.slot_held,iteration.remaining_reservations, \
                iteration.return_instance_id,iteration.return_node,iteration.result_digest, \
                iteration.carry_digest,iteration.condition,iteration.checked_at, \
                iteration.checkpoint_sequence,iteration.drained_sequence, \
                iteration.drain_outbox_id,entry.step_id AS projected_entry_id, \
                returned.step_id AS projected_return_id \
         FROM loop_iterations iteration \
         LEFT JOIN steps entry \
           ON entry.tenant_id=iteration.tenant_id AND entry.run_id=iteration.run_id \
          AND entry.instance_path_nodes=iteration.entry_path_nodes \
          AND entry.instance_path_indices=iteration.entry_path_indices \
          AND entry.node=iteration.entry_node \
         LEFT JOIN steps returned \
           ON returned.tenant_id=iteration.tenant_id AND returned.run_id=iteration.run_id \
          AND returned.instance_path_nodes=iteration.entry_path_nodes \
          AND returned.instance_path_indices=iteration.entry_path_indices \
          AND returned.node=iteration.return_node \
         WHERE iteration.tenant_id=$1 AND iteration.run_id=$2 \
         ORDER BY iteration.owner_instance_id,iteration.iteration_index",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    let mut iterations: BTreeMap<Uuid, RestoredLoopRows> = BTreeMap::new();
    for row in &iteration_rows {
        let owner = col::<Uuid>(row, "owner_instance_id")?;
        let index = u32::try_from(col::<i32>(row, "iteration_index")?)
            .map_err(|_| corrupt("loop iteration index"))?;
        let entry_id = col::<Uuid>(row, "entry_instance_id")?;
        if Some(entry_id) != col::<Option<Uuid>>(row, "projected_entry_id")? {
            return Err(corrupt("loop entry identity"));
        }
        let entry_path = instance_path(
            col(row, "entry_path_nodes")?,
            col(row, "entry_path_indices")?,
            "loop entry path",
        )?;
        let entry_node: String = col(row, "entry_node")?;
        let entry = ExecutionInstanceId::in_path(
            entry_path.clone(),
            NodeId::parse_stored(&entry_node).map_err(|_| corrupt("loop entry node"))?,
        );
        let lifecycle: String = col(row, "lifecycle")?;
        let slot_held = col::<bool>(row, "slot_held")?;
        let remaining = col::<i32>(row, "remaining_reservations")?;
        let return_id = col::<Option<Uuid>>(row, "return_instance_id")?;
        let return_node = col::<Option<String>>(row, "return_node")?;
        let result = col::<Option<Vec<u8>>>(row, "result_digest")?;
        let carry = col::<Option<Vec<u8>>>(row, "carry_digest")?;
        let condition = col::<Option<bool>>(row, "condition")?;
        let checked_at = col::<Option<OffsetDateTime>>(row, "checked_at")?;
        let sequence = col::<Option<i64>>(row, "checkpoint_sequence")?;
        let drained_sequence = col::<Option<i64>>(row, "drained_sequence")?;
        let drain_outbox_id = col::<Option<Uuid>>(row, "drain_outbox_id")?;
        let rows = iterations.entry(owner).or_insert_with(|| RestoredLoopRows {
            iterations: Vec::new(),
            active_iteration: None,
            last_checkpoint_sequence: None,
        });
        match lifecycle.as_str() {
            "ADMITTED"
                if slot_held
                    && remaining > 0
                    && return_id.is_none()
                    && return_node.is_none()
                    && result.is_none()
                    && carry.is_none()
                    && condition.is_none()
                    && checked_at.is_none()
                    && sequence.is_none()
                    && drained_sequence.is_none()
                    && drain_outbox_id.is_none() =>
            {
                if checkpoints.contains_key(&(owner, index))
                    || rows.active_iteration.replace(index).is_some()
                {
                    return Err(corrupt("loop active iteration"));
                }
                rows.iterations
                    .push(LoopIterationRestore::admitted(index, entry));
            }
            "CHECKPOINTED"
                if !slot_held
                    && remaining >= 0
                    && drained_sequence.is_none()
                    && drain_outbox_id.is_none() =>
            {
                let stored = checkpoints
                    .remove(&(owner, index))
                    .ok_or_else(|| corrupt("loop checkpoint projection"))?;
                let return_id = return_id.ok_or_else(|| corrupt("loop return identity"))?;
                let return_node = return_node.ok_or_else(|| corrupt("loop return node"))?;
                let result = stored_digest(
                    result.ok_or_else(|| corrupt("loop result digest"))?,
                    "loop result digest",
                )?;
                let carry = stored_digest(
                    carry.ok_or_else(|| corrupt("loop carry digest"))?,
                    "loop carry digest",
                )?;
                let condition = condition.ok_or_else(|| corrupt("loop condition"))?;
                let checked_at = checked_at.ok_or_else(|| corrupt("loop checked at"))?;
                let sequence = sequence.ok_or_else(|| corrupt("loop checkpoint sequence"))?;
                if return_id != stored.return_instance_id
                    || Some(return_id) != col::<Option<Uuid>>(row, "projected_return_id")?
                    || result != stored.result_digest
                    || carry != stored.carry_digest
                    || condition != stored.condition
                    || checked_at != stored.checked_at
                    || sequence != stored.journal_sequence
                {
                    return Err(corrupt("loop checkpoint projection"));
                }
                let return_instance = ExecutionInstanceId::in_path(
                    entry_path,
                    NodeId::parse_stored(&return_node).map_err(|_| corrupt("loop return node"))?,
                );
                rows.last_checkpoint_sequence = Some(sequence);
                rows.iterations.push(LoopIterationRestore::checkpointed(
                    index,
                    entry,
                    return_instance,
                    result,
                    carry,
                    condition,
                    checked_at,
                    stored.continuation,
                ));
            }
            "DRAINED"
                if !slot_held
                    && remaining >= 0
                    && return_id.is_none()
                    && return_node.is_none()
                    && result.is_none()
                    && carry.is_none()
                    && condition.is_none()
                    && checked_at.is_none()
                    && sequence.is_none()
                    && drained_sequence.is_some_and(|value| value >= 1)
                    && drain_outbox_id.is_some() =>
            {
                if checkpoints.contains_key(&(owner, index)) {
                    return Err(corrupt("drained loop checkpoint"));
                }
                rows.iterations
                    .push(LoopIterationRestore::drained(index, entry));
            }
            _ => return Err(corrupt("unsupported loop iteration projection")),
        }
    }
    if !checkpoints.is_empty() {
        return Err(corrupt("orphan loop checkpoint"));
    }

    for row in &scopes {
        let owner_id = col::<Uuid>(row, "owner_instance_id")?;
        if Some(owner_id) != col::<Option<Uuid>>(row, "projected_owner_id")?
            || !col::<bool>(row, "namespace_matches")?
        {
            return Err(corrupt("loop owner identity"));
        }
        let owner_path = instance_path(
            col(row, "owner_path_nodes")?,
            col(row, "owner_path_indices")?,
            "loop owner path",
        )?;
        let owner_node: String = col(row, "owner_node")?;
        let owner = ExecutionInstanceId::in_path(
            owner_path,
            NodeId::parse_stored(&owner_node).map_err(|_| corrupt("loop owner node"))?,
        );
        let kind = match col::<String>(row, "kind")?.as_str() {
            "WHILE" => LoopKind::While,
            "REPEAT_UNTIL" => LoopKind::RepeatUntil,
            _ => return Err(corrupt("loop kind")),
        };
        let phase = match col::<String>(row, "lifecycle")?.as_str() {
            "READY" => LoopPhase::Ready,
            "RUNNING" => LoopPhase::Running,
            "DELAYING" => LoopPhase::Delaying,
            "SUCCEEDED" => LoopPhase::Succeeded,
            "FAILED" => LoopPhase::Failed,
            "DRAINING" => LoopPhase::Draining,
            "CANCELLED" => LoopPhase::Cancelled,
            _ => return Err(corrupt("loop lifecycle")),
        };
        let restored = iterations.remove(&owner_id).unwrap_or(RestoredLoopRows {
            iterations: Vec::new(),
            active_iteration: None,
            last_checkpoint_sequence: None,
        });
        let projected_active = col::<Option<i32>>(row, "active_iteration")?
            .map(|value| u32::try_from(value).map_err(|_| corrupt("loop active iteration")))
            .transpose()?;
        if projected_active != restored.active_iteration
            || col::<Option<i64>>(row, "last_checkpoint_sequence")?
                != restored.last_checkpoint_sequence
            || col::<i64>(row, "entered_sequence")? < 1
        {
            return Err(corrupt("loop scope projection"));
        }
        let next_iteration = u32::try_from(col::<i32>(row, "next_iteration")?)
            .map_err(|_| corrupt("loop next iteration"))?;
        let delay_due_at = col::<Option<OffsetDateTime>>(row, "delay_due_at")?;
        let drain_kind = col::<Option<String>>(row, "drain_kind")?;
        let drain_iteration = col::<Option<i32>>(row, "drain_iteration")?;
        let drain_failure_code = col::<Option<String>>(row, "drain_failure_code")?;
        let drain_sequence = col::<Option<i64>>(row, "drain_sequence")?;
        let drain_outbox_id = col::<Option<Uuid>>(row, "drain_outbox_id")?;
        let finish_sequence = col::<Option<i64>>(row, "finish_sequence")?;
        let finish_outbox_id = col::<Option<Uuid>>(row, "finish_outbox_id")?;
        let drain_reason = match (
            drain_kind.as_deref(),
            drain_iteration,
            drain_failure_code.as_deref(),
            drain_sequence,
            drain_outbox_id,
        ) {
            (None, None, None, None, None) => None,
            (Some("FAILED"), Some(iteration), Some(code), Some(sequence), Some(_))
                if sequence >= 1 =>
            {
                Some(LoopDrainReason::Failed {
                    iteration: u32::try_from(iteration)
                        .map_err(|_| corrupt("loop drain iteration"))?,
                    code: FailureCode::parse(code)
                        .map_err(|_| corrupt("loop drain failure code"))?,
                })
            }
            (Some("TIMED_OUT"), None, Some("RUN_TIMED_OUT"), Some(sequence), Some(_))
                if sequence >= 1 =>
            {
                Some(LoopDrainReason::TimedOut)
            }
            (Some("CANCELLED"), None, None, Some(sequence), Some(_)) if sequence >= 1 => {
                Some(LoopDrainReason::Cancelled)
            }
            _ => return Err(corrupt("loop drain reason")),
        };
        let finish_matches = match phase {
            LoopPhase::Draining => finish_sequence.is_none() && finish_outbox_id.is_none(),
            LoopPhase::Failed | LoopPhase::Cancelled if drain_reason.is_some() => {
                finish_sequence.is_some_and(|value| value >= 1) && finish_outbox_id.is_some()
            }
            _ => finish_sequence.is_none() && finish_outbox_id.is_none(),
        };
        if !finish_matches {
            return Err(corrupt("loop drain finish receipt"));
        }
        match (phase, waiting_timers.remove(&owner_id)) {
            (LoopPhase::Delaying, Some(index)) if index.checked_add(1) == Some(next_iteration) => {}
            (LoopPhase::Delaying, _) | (_, Some(_)) => {
                return Err(corrupt("loop waiting timer projection"));
            }
            _ => {}
        }
        state
            .restore_loop(
                owner,
                kind,
                phase,
                u32::try_from(col::<i32>(row, "max_iterations")?)
                    .map_err(|_| corrupt("loop iteration limit"))?,
                col(row, "created_at")?,
                col(row, "deadline_at")?,
                stored_digest(col(row, "initial_carry_digest")?, "loop initial carry")?,
                stored_digest(col(row, "current_carry_digest")?, "loop current carry")?,
                col(row, "initial_condition")?,
                next_iteration,
                delay_due_at,
                drain_reason,
                restored.iterations,
            )
            .map_err(|_| corrupt("loop projection"))?;
    }
    if !iterations.is_empty() || !waiting_timers.is_empty() {
        return Err(corrupt("loop iteration owner"));
    }
    Ok(())
}

async fn load_chain(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    run: RunId,
    state: &mut RunState,
) -> Result<(), StoreError> {
    let row = sqlx::query(
        "SELECT ec.chain_id,ec.max_segments,ec.original_deadline_at,cs.segment_index, \
                cs.predecessor_run_id,cs.successor_run_id,cs.completion_kind, \
                CASE WHEN cs.completion_kind='CONTINUED' THEN cs.transfer_input_digest \
                     ELSE incoming.transfer_input_digest END AS transfer_input_digest, \
                CASE WHEN cs.completion_kind='CONTINUED' THEN cs.transfer_state_digest \
                     ELSE incoming.transfer_state_digest END AS transfer_state_digest \
         FROM chain_segments cs \
         JOIN execution_chains ec ON ec.tenant_id=cs.tenant_id AND ec.chain_id=cs.chain_id \
         LEFT JOIN chain_segments incoming ON incoming.tenant_id=cs.tenant_id \
              AND incoming.chain_id=cs.chain_id AND incoming.successor_run_id=cs.run_id \
         WHERE cs.tenant_id=$1 AND cs.run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    let Some(row) = row else {
        return Ok(());
    };
    let max_segments = u16::try_from(col::<i32>(&row, "max_segments")?)
        .map_err(|_| corrupt("chain segment limit"))?;
    let segment_index = u16::try_from(col::<i32>(&row, "segment_index")?)
        .map_err(|_| corrupt("chain segment index"))?;
    if max_segments == 0 || segment_index >= max_segments {
        return Err(corrupt("chain segment bounds"));
    }
    let segment_index =
        SegmentIndex::new(segment_index).map_err(|_| corrupt("chain segment index"))?;
    let predecessor = col::<Option<Uuid>>(&row, "predecessor_run_id")?.map(RunId::new);
    let successor = col::<Option<Uuid>>(&row, "successor_run_id")?.map(RunId::new);
    let input = col::<Option<Vec<u8>>>(&row, "transfer_input_digest")?
        .map(|value| stored_digest(value, "chain transfer input"))
        .transpose()?;
    let transfer = input.map(|input| {
        col::<Option<Vec<u8>>>(&row, "transfer_state_digest")
            .and_then(|state| {
                state
                    .map(|value| stored_digest(value, "chain transfer state"))
                    .transpose()
            })
            .map(|state| ContinuationTransfer::new(input, state))
    });
    let transfer = transfer.transpose()?;
    let completion = match col::<Option<String>>(&row, "completion_kind")?.as_deref() {
        None if successor.is_none() => None,
        Some("FINAL") if successor.is_none() => Some(SegmentCompletionKind::Final),
        Some("CONTINUED") if successor.is_some() && transfer.is_some() => {
            Some(SegmentCompletionKind::Continued)
        }
        _ => return Err(corrupt("chain segment completion")),
    };
    state.restore_chain(RunChainState::restore(
        RunId::new(col(&row, "chain_id")?),
        predecessor,
        successor,
        segment_index,
        max_segments,
        col(&row, "original_deadline_at")?,
        completion,
        transfer,
    ));
    Ok(())
}

/// Whether the event names a collection or loop scope's compensation rather than one step.
fn is_scope_saga_event(event: &RunEvent) -> bool {
    matches!(
        event,
        RunEvent::ScopeCompensationPlanned { .. } | RunEvent::ScopeCompensated { .. }
    )
}

/// The compensation of the collection and loop scopes of a run, rebuilt from their rows. The scopes
/// themselves are restored first: a compensation without its scope is a corrupt projection.
async fn load_scope_sagas(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    run: RunId,
    state: &mut RunState,
) -> Result<(), StoreError> {
    let sagas = sqlx::query(
        "SELECT owner_path_nodes, owner_path_indices, owner_node, phase, cause_code, outcome, \
                during_stop \
         FROM scope_sagas WHERE tenant_id = $1 AND run_id = $2 \
         ORDER BY owner_path_nodes, owner_path_indices, owner_node",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    if sagas.is_empty() {
        return Ok(());
    }
    let entries = sqlx::query(
        "SELECT owner_path_nodes, owner_path_indices, owner_node, iteration_index, node, kind, \
                after_iterations, after_nodes \
         FROM scope_saga_entries WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    type Planned = (
        BTreeMap<ExecutionInstanceId, Vec<ExecutionInstanceId>>,
        Vec<ExecutionInstanceId>,
    );
    let mut planned: BTreeMap<ExecutionInstanceId, Planned> = BTreeMap::new();
    for row in &entries {
        let owner = scope_owner_instance(row)?;
        let iteration = u32::try_from(col::<i32>(row, "iteration_index")?)
            .map_err(|_| corrupt("scope compensation iteration"))?;
        let member = |iteration: u32, node: &str| -> Result<ExecutionInstanceId, StoreError> {
            Ok(ExecutionInstanceId::in_path(
                owner
                    .path()
                    .child(owner.node().clone(), iteration)
                    .map_err(|_| corrupt("scope compensation member path"))?,
                NodeId::parse_stored(node).map_err(|_| corrupt("scope compensation member"))?,
            ))
        };
        let forward = member(iteration, &col::<String>(row, "node")?)?;
        let slot = planned.entry(owner.clone()).or_default();
        match col::<String>(row, "kind")?.as_str() {
            "COMPENSATE" => {
                let iterations = col::<Vec<i32>>(row, "after_iterations")?;
                let nodes = col::<Vec<String>>(row, "after_nodes")?;
                let after = iterations
                    .iter()
                    .zip(&nodes)
                    .map(|(iteration, node)| {
                        member(
                            u32::try_from(*iteration)
                                .map_err(|_| corrupt("scope compensation dependency"))?,
                            node,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                slot.0.insert(forward, after);
            }
            "IRREVERSIBLE" => slot.1.push(forward),
            _ => return Err(corrupt("scope compensation entry kind")),
        }
    }
    for row in &sagas {
        let owner = scope_owner_instance(row)?;
        let (entries, irreversible) = planned.remove(&owner).unwrap_or_default();
        let saga = ScopeSagaState::restore(
            col::<String>(row, "phase")?
                .parse::<SagaPhase>()
                .map_err(|_| corrupt("scope compensation phase"))?,
            FailureCode::parse(&col::<String>(row, "cause_code")?)
                .map_err(|_| corrupt("scope compensation cause"))?,
            entries,
            irreversible,
            col::<Option<String>>(row, "outcome")?
                .map(|outcome| outcome.parse::<SagaOutcome>())
                .transpose()
                .map_err(|()| corrupt("scope compensation outcome"))?,
            col(row, "during_stop")?,
        );
        state
            .restore_scope_saga(owner, saga)
            .map_err(|_| corrupt("scope compensation owner"))?;
    }
    if !planned.is_empty() {
        return Err(corrupt("orphan scope compensation entry"));
    }
    Ok(())
}

fn scope_owner_instance(row: &PgRow) -> Result<ExecutionInstanceId, StoreError> {
    Ok(ExecutionInstanceId::in_path(
        instance_path(
            col(row, "owner_path_nodes")?,
            col(row, "owner_path_indices")?,
            "scope compensation owner path",
        )?,
        NodeId::parse_stored(&col::<String>(row, "owner_node")?)
            .map_err(|_| corrupt("scope compensation owner"))?,
    ))
}

/// The sagas of a run, rebuilt from their rows.
async fn load_sagas(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    run: RunId,
) -> Result<BTreeMap<NodeId, SagaState>, StoreError> {
    let rows = sqlx::query(
        "SELECT node, phase, cause_code, outcome, during_stop, irreversible \
         FROM run_sagas WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    if rows.is_empty() {
        return Ok(BTreeMap::new());
    }
    let entry_rows = sqlx::query(
        "SELECT saga_node, node, after FROM run_saga_entries WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(store_error)?;
    let mut entries: BTreeMap<NodeId, BTreeMap<NodeId, Vec<NodeId>>> = BTreeMap::new();
    for row in &entry_rows {
        let saga: String = col(row, "saga_node")?;
        let node: String = col(row, "node")?;
        let after: Vec<String> = col(row, "after")?;
        let after = after
            .iter()
            .map(|n| NodeId::parse_stored(n))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| corrupt("saga entry"))?;
        entries
            .entry(NodeId::parse_stored(&saga).map_err(|_| corrupt("saga node"))?)
            .or_default()
            .insert(
                NodeId::parse_stored(&node).map_err(|_| corrupt("saga entry"))?,
                after,
            );
    }
    let mut sagas = BTreeMap::new();
    for row in &rows {
        let node: String = col(row, "node")?;
        let node = NodeId::parse_stored(&node).map_err(|_| corrupt("saga node"))?;
        let phase: String = col(row, "phase")?;
        let cause: Option<String> = col(row, "cause_code")?;
        let outcome: Option<String> = col(row, "outcome")?;
        let irreversible: Vec<String> = col(row, "irreversible")?;
        let state = SagaState::restore(
            phase
                .parse::<SagaPhase>()
                .map_err(|_| corrupt("saga phase"))?,
            cause
                .map(|c| FailureCode::parse(&c))
                .transpose()
                .map_err(|_| corrupt("saga cause"))?,
            entries.remove(&node).unwrap_or_default(),
            irreversible
                .iter()
                .map(|n| NodeId::parse_stored(n))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| corrupt("saga irreversible"))?,
            outcome
                .map(|o| o.parse::<SagaOutcome>())
                .transpose()
                .map_err(|()| corrupt("saga outcome"))?,
            col(row, "during_stop")?,
        );
        sagas.insert(node, state);
    }
    Ok(sagas)
}

// ------------------------------------------------------------------------------ the transaction

/// One locked run inside an open database transaction.
pub struct PgRunTx {
    tx: Transaction<'static, Postgres>,
    info: RunInfo,
    now: OffsetDateTime,
    own_scheduler_time: Option<OffsetDateTime>,
    def: Arc<WorkflowDefinition>,
    state: RunState,
    /// Journal sequence already written; `persist` writes `persisted + 1 ..= state.version()`.
    persisted: u64,
    data: Option<RunData>,
    input_digest: Vec<u8>,
    /// Steps that already have a row. Every write to a run's steps happens under its row lock, so
    /// "insert or update" is decided here, not by `ON CONFLICT` (which checks constraints on the
    /// proposed row before it takes the update path).
    stored: BTreeSet<ExecutionInstanceId>,
    timer_read_states: BTreeMap<ExecutionInstanceId, TimerReadState>,
    event_wait_read_states: BTreeMap<ExecutionInstanceId, EventWaitReadState>,
    breaker: BreakerPolicy,
    /// Pins selected under the retention-policy fence before a lease is granted. `persist` adds
    /// them to the matching immutable `step.leased` detail in this transaction.
    script_retention: BTreeMap<NodeId, ScriptRetentionPins>,
    /// Counted when (and only when) the transaction commits.
    signals: Vec<Signal>,
    chain_lock: Option<LockedChainHead>,
    initialize_chain: bool,
    /// How long each approval request this transaction cancelled had waited, recorded as
    /// `workflow_approval_wait_seconds{outcome="cancelled"}` when (and only when) it commits.
    cancelled_waits: Vec<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ScriptRetentionPins {
    script: Option<u16>,
    compensation: Option<u16>,
    reconciliation: Option<u16>,
}

enum ScriptRetentionDecision {
    NotScript,
    Refused,
    Pinned(u16),
}

impl std::fmt::Debug for PgRunTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgRunTx")
            .field("run", &self.info.run_id)
            .finish_non_exhaustive()
    }
}

impl PgRunTx {
    async fn ensure_node_kind(
        &mut self,
        instance: &ExecutionInstanceId,
        node_kind: &str,
    ) -> Result<(), StoreError> {
        let (path_nodes, path_indices) = stored_path_parts(instance.path());
        let updated = sqlx::query(
            "UPDATE steps SET node_kind=$6 \
             WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
               AND instance_path_indices=$4 AND node=$5 \
               AND node_kind IN ('legacyControl',$6)",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(path_nodes)
        .bind(path_indices)
        .bind(instance.node().as_str())
        .bind(node_kind)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        if updated.rows_affected() != 1 {
            return Err(corrupt("immutable step node kind"));
        }
        Ok(())
    }

    async fn charge_logical_output_bytes(
        &mut self,
        outputs: &[(ExecutionInstanceId, usize)],
        introduced_outputs: &BTreeSet<ExecutionInstanceId>,
    ) -> Result<(), StoreError> {
        let mut seen = BTreeSet::new();
        let mut charge = 0_i64;
        for (instance, bytes) in outputs {
            if !seen.insert(instance) {
                return Err(StoreError::Internal {
                    detail: "one logical step staged more than one output".into(),
                });
            }
            let (path_nodes, path_indices) = stored_path_parts(instance.path());
            let retained: Option<bool> = sqlx::query_scalar(
                "SELECT output_digest IS NOT NULL FROM steps \
                 WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
                   AND instance_path_indices=$4 AND node=$5 FOR UPDATE",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.run_id.as_uuid())
            .bind(path_nodes)
            .bind(path_indices)
            .bind(instance.node().as_str())
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(store_error)?;
            let should_charge = match retained {
                Some(true) => false,
                Some(false) => true,
                None if introduced_outputs.contains(instance) => true,
                None => return Err(corrupt("output step projection")),
            };
            if should_charge {
                charge = charge
                    .checked_add(i64::try_from(*bytes).map_err(|_| corrupt("output bytes"))?)
                    .ok_or_else(|| corrupt("output byte ledger"))?;
            }
        }
        if charge == 0 {
            return Ok(());
        }
        let effective = collection_outputs::inspect(&mut self.tx, &self.info, &self.def).await?;
        let charge = u64::try_from(charge).map_err(|_| corrupt("output byte charge"))?;
        let after = effective
            .total()
            .checked_add(charge)
            .ok_or_else(|| corrupt("effective output byte total"))?;
        if after > u64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| corrupt("run output limit"))? {
            return Err(StoreError::QuotaExceeded {
                what: "output retained by one run",
            });
        }
        let updated = sqlx::query(
            "UPDATE run_instance_usage \
             SET logical_output_bytes=logical_output_bytes+$4 \
             WHERE tenant_id=$1 AND run_id=$2 \
               AND logical_output_bytes=$3 \
               AND logical_output_bytes+$4+$5 <= $6",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(i64::try_from(effective.ledger()).map_err(|_| corrupt("output byte ledger"))?)
        .bind(i64::try_from(charge).map_err(|_| corrupt("output byte charge"))?)
        .bind(i64::try_from(effective.legacy_bytes()?).map_err(|_| corrupt("legacy output bytes"))?)
        .bind(i64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| corrupt("run output limit"))?)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        if updated.rows_affected() != 1 {
            return Err(corrupt("run output byte ledger changed under its run lock"));
        }
        Ok(())
    }

    /// Closes the collection projection and records its one notification inside this Run
    /// transaction. The caller persists the matching `CollectionFinished` event before commit.
    pub(crate) async fn project_collection_finish(
        &mut self,
        finish: &CollectionFinish,
        finish_sequence: u64,
    ) -> Result<(), StoreError> {
        let updated = sqlx::query(
            "UPDATE iteration_scopes SET lifecycle='SUCCEEDED',result_digest=$4, \
                    finish_sequence=$5,finish_outbox_id=$6,updated_at=$7 \
             WHERE tenant_id=$1 AND run_id=$2 AND scope_id=$3 AND lifecycle='OPEN'",
        )
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.scope_id.as_uuid())
        .bind(finish.result_digest.as_bytes().as_slice())
        .bind(i64::try_from(finish_sequence).map_err(|_| corrupt("journal sequence"))?)
        .bind(finish.outbox_id)
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        if updated.rows_affected() != 1 {
            return Err(StoreError::Internal {
                detail: "the collection projection changed while its Run was locked".into(),
            });
        }
        sqlx::query(
            "INSERT INTO event_outbox \
             (id,tenant_id,aggregate_id,event_type,routing_key,payload) \
             VALUES ($1,$2,$3,'collection.finished',$4,$5)",
        )
        .bind(finish.outbox_id)
        .bind(finish.transition.tenant_id().as_uuid())
        .bind(finish.transition.run_id().as_uuid())
        .bind(finish.transition.owner().node().as_str())
        .bind(serde_json::json!({
            "tenantId": finish.transition.tenant_id().to_string(),
            "namespaceId": finish.transition.namespace_id().to_string(),
            "runId": finish.transition.run_id().to_string(),
            "scopeId": finish.scope_id.as_uuid().to_string(),
            "status": "SUCCEEDED",
            "traceId": finish.transition.trace_id(),
        }))
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }
}

/// Charges the aggregate retained by a successful root collection whose non-End successor keeps
/// using the dedicated collection route. End successors use ordinary `PgRunTx::persist` charging.
pub(crate) async fn charge_collection_owner_output(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    run: RunId,
    owner: &ExecutionInstanceId,
    effective: &collection_outputs::EffectiveOutputBytes,
    bytes: usize,
) -> Result<(), StoreError> {
    let (path_nodes, path_indices) = stored_path_parts(owner.path());
    let retained: Option<bool> = sqlx::query_scalar(
        "SELECT output_digest IS NOT NULL FROM steps \
         WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes=$3 \
           AND instance_path_indices=$4 AND node=$5 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(path_nodes)
    .bind(path_indices)
    .bind(owner.node().as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    if retained != Some(false) {
        return Err(corrupt("collection owner output projection"));
    }
    let charge = u64::try_from(bytes).map_err(|_| corrupt("output bytes"))?;
    let after = effective
        .total()
        .checked_add(charge)
        .ok_or_else(|| corrupt("effective output byte total"))?;
    if after > u64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| corrupt("run output limit"))? {
        return Err(StoreError::QuotaExceeded {
            what: "output retained by one run",
        });
    }
    let updated = sqlx::query(
        "UPDATE run_instance_usage \
         SET logical_output_bytes=logical_output_bytes+$4 \
         WHERE tenant_id=$1 AND run_id=$2 \
           AND logical_output_bytes=$3 \
           AND logical_output_bytes+$4+$5 <= $6",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(i64::try_from(effective.ledger()).map_err(|_| corrupt("output byte ledger"))?)
    .bind(i64::try_from(charge).map_err(|_| corrupt("output byte charge"))?)
    .bind(i64::try_from(effective.legacy_bytes()?).map_err(|_| corrupt("legacy output bytes"))?)
    .bind(i64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| corrupt("run output limit"))?)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    if updated.rows_affected() != 1 {
        return Err(corrupt("run output byte ledger changed under its run lock"));
    }
    Ok(())
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

    async fn required_worker_protocol_minor(&mut self) -> Result<u32, StoreError> {
        let needs_blob_protocol: bool = sqlx::query_scalar(
            "SELECT r.input_blob_id IS NOT NULL OR EXISTS ( \
                 SELECT 1 FROM blobs b \
                 WHERE b.tenant_id = r.tenant_id AND b.namespace_id = r.namespace_id \
                   AND b.run_id = r.id AND b.purpose = 'STEP_OUTPUT' \
                   AND b.state = 'COMMITTED') \
             FROM runs r WHERE r.tenant_id = $1 AND r.id = $2",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(if needs_blob_protocol {
            sagat_application::worker::BLOB_PROTOCOL_MINOR
        } else {
            0
        })
    }

    async fn data(&mut self) -> Result<&RunData, StoreError> {
        if self.data.is_none() {
            // Validate and count legacy collection owners before transferring any payload body.
            // Historical writers may have retained more than the current Run ceiling.
            let effective =
                collection_outputs::inspect(&mut self.tx, &self.info, &self.def).await?;
            if effective.total()
                > u64::try_from(MAX_RUN_OUTPUT_BYTES).map_err(|_| corrupt("run output limit"))?
            {
                return Err(StoreError::QuotaExceeded {
                    what: "output retained by one run",
                });
            }
            let legacy_outputs =
                collection_outputs::hydrate_missing(&mut self.tx, &self.info, &effective).await?;
            let tenant = self.info.tenant_id.as_uuid();
            let input_row = sqlx::query(
                "SELECT p.content, r.input_blob_id FROM runs r JOIN payloads p \
                 ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest \
                 WHERE r.tenant_id = $1 AND r.id = $2 AND p.digest = $3",
            )
            .bind(tenant)
            .bind(self.info.run_id.as_uuid())
            .bind(&self.input_digest)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
            let rows = sqlx::query(
                "SELECT s.node, s.instance_path_nodes, s.instance_path_indices, p.content FROM steps s \
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
                if outputs
                    .insert(
                        ExecutionInstanceId::in_path(
                            step_instance_path(row)?,
                            NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?,
                        ),
                        col::<Vec<u8>>(row, "content")?,
                    )
                    .is_some()
                {
                    return Err(corrupt("duplicate step output"));
                }
            }
            for (instance, content) in legacy_outputs {
                if outputs.insert(instance, content).is_some() {
                    return Err(corrupt("legacy collection owner already has an output"));
                }
            }
            let loaded_output_bytes = outputs.values().try_fold(0_u64, |total, output| {
                total
                    .checked_add(
                        u64::try_from(output.len())
                            .map_err(|_| corrupt("output payload length"))?,
                    )
                    .ok_or_else(|| corrupt("effective output byte total"))
            })?;
            if loaded_output_bytes != effective.total() {
                return Err(corrupt("effective output byte total"));
            }
            let state_row = sqlx::query(
                "SELECT p.content, s.state_revision FROM workflow_operation_states s JOIN payloads p \
                 ON p.tenant_id=s.tenant_id AND p.digest=s.state_digest \
                 WHERE s.tenant_id=$1 AND s.run_id=$2",
            )
            .bind(tenant)
            .bind(self.info.run_id.as_uuid())
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(store_error)?;
            let state = state_row
                .as_ref()
                .map(|row| col::<Vec<u8>>(row, "content"))
                .transpose()?;
            let state_revision = state_row
                .as_ref()
                .map(|row| {
                    u64::try_from(col::<i64>(row, "state_revision")?)
                        .map_err(|_| corrupt("operation state revision"))
                })
                .transpose()?;
            let collection_inputs = sqlx::query(
                "SELECT i.entry_path_nodes, i.entry_path_indices, i.iteration_index, p.content \
                 FROM collection_iterations i JOIN payloads p \
                   ON p.tenant_id=i.tenant_id AND p.digest=i.item_digest \
                 WHERE i.tenant_id=$1 AND i.run_id=$2 \
                   AND (i.lifecycle='ADMITTED' OR EXISTS ( \
                        SELECT 1 FROM steps open_step \
                         WHERE open_step.tenant_id=i.tenant_id AND open_step.run_id=i.run_id \
                           AND open_step.instance_path_nodes=i.entry_path_nodes \
                           AND open_step.instance_path_indices=i.entry_path_indices \
                           AND open_step.kind='COMPENSATION' \
                           AND open_step.status NOT IN ('SUCCEEDED','FAILED','CANCELLED','SKIPPED')))",
            )
            .bind(tenant)
            .bind(self.info.run_id.as_uuid())
            .fetch_all(&mut *self.tx)
            .await
            .map_err(store_error)?;
            // The carry an iteration received: the one the loop holds for the iteration that runs,
            // and, for an iteration with a compensation still to run, the carry it started from
            // (the loop's initial one, or the one its predecessor produced).
            let loop_inputs = sqlx::query(
                "SELECT i.entry_path_nodes, i.entry_path_indices, i.iteration_index, p.content \
                 FROM loop_iterations i JOIN loop_scopes s \
                   ON s.tenant_id=i.tenant_id AND s.run_id=i.run_id \
                  AND s.owner_instance_id=i.owner_instance_id \
                 JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.current_carry_digest \
                 WHERE i.tenant_id=$1 AND i.run_id=$2 AND i.lifecycle='ADMITTED' \
                   AND s.lifecycle='RUNNING' AND s.active_iteration=i.iteration_index \
                 UNION ALL \
                 SELECT i.entry_path_nodes, i.entry_path_indices, i.iteration_index, p.content \
                 FROM loop_iterations i JOIN loop_scopes s \
                   ON s.tenant_id=i.tenant_id AND s.run_id=i.run_id \
                  AND s.owner_instance_id=i.owner_instance_id \
                 JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest = CASE \
                      WHEN i.iteration_index = 0 THEN s.initial_carry_digest \
                      ELSE (SELECT before.carry_digest FROM loop_iterations before \
                             WHERE before.tenant_id=i.tenant_id AND before.run_id=i.run_id \
                               AND before.owner_instance_id=i.owner_instance_id \
                               AND before.iteration_index=i.iteration_index-1) END \
                 WHERE i.tenant_id=$1 AND i.run_id=$2 AND i.lifecycle IN ('CHECKPOINTED','DRAINED') \
                   AND EXISTS ( \
                        SELECT 1 FROM steps open_step \
                         WHERE open_step.tenant_id=i.tenant_id AND open_step.run_id=i.run_id \
                           AND open_step.instance_path_nodes=i.entry_path_nodes \
                           AND open_step.instance_path_indices=i.entry_path_indices \
                           AND open_step.kind='COMPENSATION' \
                           AND open_step.status NOT IN ('SUCCEEDED','FAILED','CANCELLED','SKIPPED'))",
            )
            .bind(tenant)
            .bind(self.info.run_id.as_uuid())
            .fetch_all(&mut *self.tx)
            .await
            .map_err(store_error)?;
            let mut scope_inputs = BTreeMap::new();
            for row in collection_inputs {
                let path = instance_path(
                    col(&row, "entry_path_nodes")?,
                    col(&row, "entry_path_indices")?,
                    "collection input path",
                )?;
                let index = u32::try_from(col::<i32>(&row, "iteration_index")?)
                    .map_err(|_| corrupt("collection iteration index"))?;
                if scope_inputs
                    .insert(
                        path,
                        ScopeInput::Collection {
                            index,
                            item: col(&row, "content")?,
                        },
                    )
                    .is_some()
                {
                    return Err(corrupt("duplicate scoped input path"));
                }
            }
            for row in loop_inputs {
                let path = instance_path(
                    col(&row, "entry_path_nodes")?,
                    col(&row, "entry_path_indices")?,
                    "loop input path",
                )?;
                let iteration = u32::try_from(col::<i32>(&row, "iteration_index")?)
                    .map_err(|_| corrupt("loop iteration index"))?;
                if scope_inputs
                    .insert(
                        path,
                        ScopeInput::Loop {
                            iteration,
                            carry: col(&row, "content")?,
                        },
                    )
                    .is_some()
                {
                    return Err(corrupt("duplicate scoped input path"));
                }
            }
            self.data = Some(RunData {
                run_id: Some(self.info.run_id),
                input: col(&input_row, "content")?,
                input_blob: col::<Option<Uuid>>(&input_row, "input_blob_id")?
                    .map(sagat_domain::blobs::BlobId::new),
                state,
                state_revision,
                outputs,
                scope_inputs,
            });
        }
        self.data.as_ref().ok_or_else(|| StoreError::Internal {
            detail: "run data was not loaded".into(),
        })
    }

    async fn logical_output_bytes(&mut self) -> Result<u64, StoreError> {
        Ok(
            collection_outputs::inspect(&mut self.tx, &self.info, &self.def)
                .await?
                .total(),
        )
    }

    async fn stage_outputs(&mut self, outputs: &[(NodeId, Vec<u8>)]) -> Result<(), StoreError> {
        self.data().await?;
        if let Some(data) = &mut self.data {
            for (node, bytes) in outputs {
                data.insert_root_output(node.clone(), bytes.clone());
            }
        }
        Ok(())
    }

    async fn stage_instance_outputs(
        &mut self,
        outputs: &[(ExecutionInstanceId, Vec<u8>)],
    ) -> Result<(), StoreError> {
        self.data().await?;
        if let Some(data) = &mut self.data {
            for (instance, bytes) in outputs {
                data.insert_output(instance.clone(), bytes.clone());
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

    async fn lease_expiry_instance(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<Option<OffsetDateTime>, StoreError> {
        let (nodes, indices) = stored_path_parts(instance.path());
        sqlx::query_scalar(
            "SELECT lease_expires_at FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
             AND instance_path_nodes = $4 AND instance_path_indices = $5 \
             AND status IN ('LEASED', 'RUNNING')",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(nodes)
        .bind(indices)
        .fetch_optional(&mut *self.tx)
        .await
        .map(Option::flatten)
        .map_err(store_error)
    }

    async fn lease_deadlines(
        &mut self,
        node: &NodeId,
    ) -> Result<Option<LeaseDeadlines>, StoreError> {
        let row = sqlx::query(
            "SELECT lease_expires_at, attempt_deadline_at FROM steps \
             WHERE tenant_id = $1 AND run_id = $2 AND node = $3 AND status IN ('LEASED', 'RUNNING')",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        row.map(|row| {
            Ok(LeaseDeadlines {
                expires_at: col(&row, "lease_expires_at")?,
                attempt_deadline_at: col(&row, "attempt_deadline_at")?,
            })
        })
        .transpose()
    }

    async fn lease_deadlines_instance(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<Option<LeaseDeadlines>, StoreError> {
        let (nodes, indices) = stored_path_parts(instance.path());
        let row = sqlx::query(
            "SELECT lease_expires_at, attempt_deadline_at FROM steps \
             WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
               AND instance_path_nodes = $4 AND instance_path_indices = $5 \
               AND status IN ('LEASED', 'RUNNING')",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(nodes)
        .bind(indices)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        row.map(|row| {
            Ok(LeaseDeadlines {
                expires_at: col(&row, "lease_expires_at")?,
                attempt_deadline_at: col(&row, "attempt_deadline_at")?,
            })
        })
        .transpose()
    }

    async fn database_executor_owns(
        &mut self,
        node: &NodeId,
        subject: &str,
    ) -> Result<bool, StoreError> {
        self.database_owner(node, subject).await
    }
    async fn database_report_owns(
        &mut self,
        node: &NodeId,
        fence: sagat_domain::execution::Fence,
        subject: &str,
    ) -> Result<bool, StoreError> {
        self.database_report_owner(node, fence, subject).await
    }
    async fn database_claim(&mut self, node: &NodeId, subject: &str) -> Result<bool, StoreError> {
        self.database_admission(node, subject).await
    }
    async fn database_input(
        &mut self,
        node: &NodeId,
        subject: &str,
    ) -> Result<sagat_application::database::DatabaseInput, StoreError> {
        self.database_snapshot(node, subject).await
    }
    async fn pin_database_credential(
        &mut self,
        node: &NodeId,
        secret: &sagat_application::secrets::StoredSecret,
    ) -> Result<(), StoreError> {
        self.database_pin(node, secret).await
    }
    async fn database_result_limits(
        &mut self,
        node: &NodeId,
    ) -> Result<sagat_domain::database::DatabaseLimits, StoreError> {
        self.database_output_limits(node).await
    }
    async fn database_credential(
        &mut self,
        node: &NodeId,
    ) -> Result<Option<sagat_application::secrets::StoredSecret>, StoreError> {
        self.database_pinned_secret(node).await
    }

    async fn current_worker_authorized(&mut self, subject: &str) -> Result<bool, StoreError> {
        current_worker_authorized(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            subject,
        )
        .await
    }

    async fn secret_versions(
        &mut self,
        names: &[SecretName],
    ) -> Result<Vec<sagat_application::secrets::StoredSecret>, StoreError> {
        crate::secrets::all_versions(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            names,
        )
        .await
    }

    async fn secret_pool_refusal(
        &mut self,
        names: &[SecretName],
        subject: &str,
        check: sagat_application::execution::PoolCheck,
    ) -> Result<Option<SecretName>, StoreError> {
        crate::secrets::pool_refusal(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            names,
            subject,
            check == sagat_application::execution::PoolCheck::Delivery,
        )
        .await
    }

    async fn secret_inputs(
        &mut self,
        names: &[SecretName],
    ) -> Result<sagat_application::secrets::SecretInputs, StoreError> {
        crate::secrets::inputs(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            self.info.version_id.as_uuid(),
            names,
        )
        .await
    }

    async fn holds_shard(
        &mut self,
        owner: &str,
        lease: sagat_application::shards::Lease,
    ) -> Result<bool, StoreError> {
        self.own_scheduler_time = None;
        let Some(now) = locked_scheduler_time(&mut self.tx, self.info.run_id, owner, lease).await?
        else {
            return Ok(false);
        };
        self.now = now;
        self.own_scheduler_time = Some(now);
        Ok(true)
    }

    async fn holds_parent_shard(
        &mut self,
        owner: &str,
        lease: sagat_application::shards::Lease,
    ) -> Result<bool, StoreError> {
        // Only create() constructs this uncommitted zero-event transaction. Reopening a child
        // clears initialize_chain; its parent's authority cannot authorize later child changes.
        if !self.initialize_chain || self.persisted != 0 || self.state.version() != 0 {
            return Ok(false);
        }
        let Some(parent) = &self.info.parent else {
            return Ok(false);
        };
        let Some(now) = locked_scheduler_time(&mut self.tx, parent.run, owner, lease).await? else {
            return Ok(false);
        };
        self.now = now;
        Ok(true)
    }

    async fn worker_drained(&mut self, subject: &str) -> Result<bool, StoreError> {
        sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
            .bind(crate::operations::drain_lock_key(
                self.info.tenant_id,
                self.info.namespace_id,
                subject,
            ))
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM worker_drains \
                            WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3)",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(subject)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn worker_session_room(
        &mut self,
        session_id: Uuid,
        token_hash: &[u8],
        subject: &str,
    ) -> Result<u32, StoreError> {
        worker_session_room(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            session_id,
            token_hash,
            subject,
        )
        .await
    }

    async fn bound_patterns(&mut self, subject: &str) -> Result<Vec<String>, StoreError> {
        // Keep the authority observed by the final capability check through its audit commit.
        // A binding replacement takes this same key exclusively before changing grants or leases.
        crate::bindings::hold_worker_binding(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            subject,
        )
        .await?;
        sqlx::query_scalar(
            "SELECT pattern FROM worker_bindings \
             WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(subject)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn lease_unexpired(
        &mut self,
        node: &NodeId,
        token_hash: &[u8],
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
                            AND status IN ('LEASED', 'RUNNING') AND lease_token_hash = $4 \
                            AND lease_expires_at > clock_timestamp())",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .bind(token_hash)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn lease_unexpired_instance(
        &mut self,
        instance: &ExecutionInstanceId,
        token_hash: &[u8],
    ) -> Result<bool, StoreError> {
        let (nodes, indices) = stored_path_parts(instance.path());
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
                            AND instance_path_nodes = $4 AND instance_path_indices = $5 \
                            AND status IN ('LEASED', 'RUNNING') AND lease_token_hash = $6 \
                            AND lease_expires_at > clock_timestamp())",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(nodes)
        .bind(indices)
        .bind(token_hash)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn compensation_admitted(&mut self, activity: &ActivityName) -> Result<bool, StoreError> {
        crate::breaker::admit(
            &mut self.tx,
            self.breaker,
            self.info.tenant_id.as_uuid(),
            self.info.namespace_id.as_uuid(),
            activity.as_str(),
        )
        .await
    }

    async fn activity_admitted(&mut self, activity: &ActivityName) -> Result<bool, StoreError> {
        crate::quotas::activity_admitted(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            activity,
        )
        .await
    }

    async fn script_execution_admitted(
        &mut self,
        node: &NodeId,
        activity: &ActivityName,
    ) -> Result<bool, StoreError> {
        let origin = self
            .def
            .node(node)
            .map(Node::origin)
            .cloned()
            .ok_or_else(|| corrupt("leased node"))?;
        match origin {
            NodeOrigin::Compensation(forward) | NodeOrigin::Reconciliation(forward) => {
                if sagat_domain::scripts::digest_of_activity(activity.as_str()).is_none() {
                    return Ok(true);
                }
                let field = if node.is_compensation() {
                    "compensationScriptRetentionDays"
                } else {
                    "reconciliationScriptRetentionDays"
                };
                let days = self
                    .inherited_script_retention(&forward, field, activity)
                    .await?;
                self.script_retention.insert(
                    node.clone(),
                    ScriptRetentionPins {
                        script: Some(days),
                        ..ScriptRetentionPins::default()
                    },
                );
                Ok(true)
            }
            NodeOrigin::Declared => {
                let compensation = self
                    .def
                    .node(&node.compensation())
                    .and_then(|node| match node.kind() {
                        NodeKind::Activity { activity, .. } => Some(activity.clone()),
                        _ => None,
                    });
                let reconciliation =
                    self.def
                        .node(&node.reconciliation())
                        .and_then(|node| match node.kind() {
                            NodeKind::Activity { activity, .. } => Some(activity.clone()),
                            _ => None,
                        });
                let mut pins = ScriptRetentionPins::default();
                for (role, candidate) in [
                    (0_u8, Some(activity.clone())),
                    (1_u8, compensation),
                    (2_u8, reconciliation),
                ] {
                    let Some(candidate) = candidate else {
                        continue;
                    };
                    match self.current_script_retention(&candidate).await? {
                        ScriptRetentionDecision::NotScript => {}
                        ScriptRetentionDecision::Refused => return Ok(false),
                        ScriptRetentionDecision::Pinned(days) => match role {
                            0 => pins.script = Some(days),
                            1 => pins.compensation = Some(days),
                            _ => pins.reconciliation = Some(days),
                        },
                    }
                }
                if pins.script.is_some()
                    || pins.compensation.is_some()
                    || pins.reconciliation.is_some()
                {
                    self.script_retention.insert(node.clone(), pins);
                }
                Ok(true)
            }
        }
    }

    async fn script_execution_admitted_instance(
        &mut self,
        instance: &ExecutionInstanceId,
        activity: &ActivityName,
    ) -> Result<bool, StoreError> {
        if instance.path().is_root() {
            return self
                .script_execution_admitted(instance.node(), activity)
                .await;
        }
        Ok(sagat_domain::scripts::digest_of_activity(activity.as_str()).is_none())
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

    async fn lease_matches_instance(
        &mut self,
        instance: &ExecutionInstanceId,
        token_hash: &[u8],
    ) -> Result<bool, StoreError> {
        let (nodes, indices) = stored_path_parts(instance.path());
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
                            AND instance_path_nodes = $4 AND instance_path_indices = $5 \
                            AND status IN ('LEASED', 'RUNNING') AND lease_token_hash = $6)",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(nodes)
        .bind(indices)
        .bind(token_hash)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)
    }

    async fn reported_by(
        &mut self,
        instance: &ExecutionInstanceId,
        token_hash: &[u8],
    ) -> Result<bool, StoreError> {
        let (nodes, indices) = stored_path_parts(instance.path());
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
                            AND instance_path_nodes = $4 AND instance_path_indices = $5 \
                            AND status NOT IN ('LEASED', 'RUNNING') \
                            AND reported_lease_token_hash = $6)",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(nodes)
        .bind(indices)
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

    async fn latch_chain_cancel(&mut self) -> Result<(), StoreError> {
        self.latch_chain_cancel_inner().await
    }

    async fn persist(&mut self, events: &[RunEvent], effects: Effects) -> Result<(), StoreError> {
        if !effects.rehint_instances.is_empty() {
            return Err(StoreError::Internal {
                detail: "a scoped rehint used root persistence".into(),
            });
        }
        let version = self.state.version();
        let written = u64::try_from(events.len()).map_err(|_| corrupt("event count"))?;
        if version != self.persisted + written {
            return Err(StoreError::Internal {
                detail: "the in-memory state and the events to write diverged".into(),
            });
        }
        let tenant = self.info.tenant_id;
        let run = self.info.run_id;

        // 1. Outputs: content-addressed payloads, remembered per root node.
        let output_charges = effects
            .outputs
            .iter()
            .map(|(node, bytes)| (ExecutionInstanceId::root(node.clone()), bytes.len()))
            .collect::<Vec<_>>();
        let introduced_outputs = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    RunEvent::StepCompleted { .. }
                        | RunEvent::WaitSatisfied { .. }
                        | RunEvent::ReconciliationConcluded { .. }
                        | RunEvent::EndReached { .. }
                        | RunEvent::ChildFinished { .. }
                )
            })
            .filter_map(|event| event.node().cloned().map(ExecutionInstanceId::root))
            .collect::<BTreeSet<_>>();
        self.charge_logical_output_bytes(&output_charges, &introduced_outputs)
            .await?;
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
                data.insert_root_output(node.clone(), bytes.clone());
            }
        }

        // 1b. The blobs those outputs are descriptors of, used for good in this very transaction: a
        // blob that was swept, already used or altered makes the whole report fail, never half of it.
        for blob in &effects.blobs {
            crate::blobs::commit(&mut self.tx, tenant, blob).await?;
        }

        // 2. The journal, and the nodes the events touched.
        let mut touched: BTreeSet<NodeId> = BTreeSet::new();
        // Sagas whose state changed, to be written after the steps.
        let mut sagas: BTreeSet<NodeId> = BTreeSet::new();
        let mut hinted: BTreeSet<NodeId> = BTreeSet::new();
        // Hints written again because the first one may have been lost (they are counted: a step
        // nobody claims is announced a bounded number of times, not forever).
        let mut rehinted: BTreeSet<NodeId> = BTreeSet::new();
        for (offset, event) in events.iter().enumerate() {
            let seq = i64::try_from(self.persisted + 1 + offset as u64)
                .map_err(|_| corrupt("sequence"))?;
            let mut encoded = encode(event);
            if let RunEvent::StepLeased { node, .. } = event
                && let Some(pins) = self.script_retention.get(node)
            {
                let detail =
                    encoded
                        .detail
                        .as_object_mut()
                        .ok_or_else(|| StoreError::Internal {
                            detail: "a lease event did not encode as an object".into(),
                        })?;
                detail.insert("scriptRetentionPolicyVersion".into(), serde_json::json!(1));
                for (name, value) in [
                    ("scriptRetentionDays", pins.script),
                    ("compensationScriptRetentionDays", pins.compensation),
                    ("reconciliationScriptRetentionDays", pins.reconciliation),
                ] {
                    if let Some(days) = value {
                        detail.insert(name.into(), serde_json::json!(days));
                    }
                }
            }
            let payload = event
                .node()
                .filter(|_| {
                    matches!(
                        event,
                        RunEvent::StepCompleted { .. }
                            | RunEvent::WaitSatisfied { .. }
                            | RunEvent::ReconciliationConcluded { .. }
                            // A run's result (G21) and the result a child hands its parent: the
                            // journal names the payload, not only the step projection.
                            | RunEvent::EndReached { .. }
                            | RunEvent::ChildFinished { .. }
                    )
                })
                .and_then(|n| output_digest.get(n));
            let root_instance_id = event.node().map(|node| {
                let instance = ExecutionInstanceId::root(node.clone());
                instance_step_id(self.info.version_id, run, &instance)
            });
            let insert = sqlx::query(
                "INSERT INTO run_events \
                 (tenant_id, run_id, seq, event_type, node, attempt, detail, payload_digest, actor, \
                  trace_id, occurred_at, instance_id, instance_path_nodes, instance_path_indices) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            );
            insert
                .bind(tenant.as_uuid())
                .bind(run.as_uuid())
                .bind(seq)
                .bind(encoded.event_type)
                .bind(encoded.node.as_deref())
                .bind(encoded.attempt)
                .bind(&encoded.detail)
                .bind(payload.map(Vec::as_slice))
                .bind(effects.actor.label())
                .bind(
                    effects
                        .trace_id
                        .as_deref()
                        .or(self.info.trace_id.as_deref()),
                )
                .bind(self.now)
                .bind(root_instance_id)
                .bind(root_instance_id.map(|_| Vec::<String>::new()))
                .bind(root_instance_id.map(|_| Vec::<i32>::new()))
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;

            // The steps the event created or changed, from the one definition of them the platform has
            // (the Studio's changes of the graph are made of the same list).
            touched.extend(event.changed_nodes());
            self.signals.extend(of_event(event));
            match event {
                RunEvent::SagaEntered { node, .. }
                | RunEvent::SagaCompleted { node }
                | RunEvent::SagaAborting { node, .. }
                | RunEvent::SagaCompensated { node, .. }
                | RunEvent::CompensationPlanned { node, .. } => {
                    sagas.insert(node.clone());
                }
                RunEvent::StepReleased { node }
                | RunEvent::StepRetryDue { node }
                | RunEvent::StepRetryRequested { node } => {
                    hinted.insert(node.clone());
                    self.outbox_task_ready(node).await?;
                }
                RunEvent::StepCompleted { node, .. } if node.is_compensation() => {
                    self.breaker_outcome(node, true).await?;
                }
                // An attempt of a compensation that ended without success (its lease ran out and
                // a timeout count too: the event does not say why, and a provider that does not
                // answer is exactly what the breaker is for).
                RunEvent::StepRetryScheduled { node, .. }
                | RunEvent::StepFailed {
                    node,
                    fence: Some(_),
                    ..
                }
                | RunEvent::StepNeedsReview {
                    node,
                    fence: Some(_),
                    ..
                } if node.is_compensation() => self.breaker_outcome(node, false).await?,
                RunEvent::ChildFinished { child, .. } => {
                    self.settle_child(*child, false, true).await?;
                }
                RunEvent::StepCancelled { node } => {
                    self.release_child(node, true).await?;
                    if matches!(
                        self.def.node(node).map(sagat_domain::workflow::Node::kind),
                        Some(NodeKind::Approval { .. })
                    ) {
                        self.cancel_approval(
                            node,
                            u64::try_from(seq)
                                .map_err(|_| corrupt("approval cancellation sequence"))?,
                            &effects,
                        )
                        .await?;
                    }
                }
                RunEvent::StepCancelRequested { node } => self.release_child(node, false).await?,
                RunEvent::RunFinished { status } => self.outbox_run_finished(*status).await?,
                _ => {}
            }
        }
        if !events.is_empty() {
            // The journal moved: the telemetry projection will turn what is new into spans. The write
            // is one upsert of a row of its own; nothing the projector does waits for this
            // transaction or makes it wait (D160).
            let head = i64::try_from(self.persisted + events.len() as u64)
                .map_err(|_| corrupt("sequence"))?;
            sqlx::query(
                "INSERT INTO telemetry_pending (tenant_id, run_id, head_seq, applied_seq, behind_since) \
                 VALUES ($1, $2, $3, 0, clock_timestamp()) \
                 ON CONFLICT (tenant_id, run_id) DO UPDATE \
                   SET behind_since = CASE WHEN telemetry_pending.applied_seq >= telemetry_pending.head_seq \
                                           THEN clock_timestamp() ELSE telemetry_pending.behind_since END, \
                       head_seq = GREATEST(telemetry_pending.head_seq, EXCLUDED.head_seq)",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(head)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        }
        if let Some(renewal) = &effects.renewal {
            if !renewal.instance.path().is_root() {
                return Err(StoreError::Internal {
                    detail: "a scoped renewal used the root persistence path".into(),
                });
            }
            touched.insert(renewal.instance.node().clone());
        }
        for node in &effects.rehint {
            if self
                .state
                .step(node)
                .is_some_and(|s| s.status() == StepStatus::Ready)
            {
                hinted.insert(node.clone());
                rehinted.insert(node.clone());
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
                rehinted.contains(node),
                output_digest.get(node),
            )
            .await?;
        }

        // 3b. The sagas the events touched, and the compensations a saga planned.
        for node in &sagas {
            self.upsert_saga(node, events).await?;
        }

        // 4. The run row, guarded by the version it was loaded at.
        let (reason, code) = match (self.state.stop_reason(), self.state.stop_code()) {
            (Some(reason), code) => (Some(reason.as_str()), code.map(FailureCode::as_str)),
            (None, _) => (None, None),
        };
        // The run's result (G21): the output its `end` step was given in this transaction.
        let result = events.iter().find_map(|event| match event {
            RunEvent::EndReached { node } => output_digest.get(node).cloned(),
            _ => None,
        });
        if written > 0 {
            let updated = sqlx::query(
                "UPDATE runs SET status = $4, run_version = $5, stop_reason = $6, stop_code = $7, \
                        paused = $8, updated_at = $9, \
                        finished_at = CASE WHEN $10 THEN COALESCE(finished_at, $9) ELSE NULL END, \
                        result_digest = COALESCE($11, result_digest) \
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
            .bind(result)
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

        if written > 0 && self.state.status().is_terminal() && self.chain_lock.is_some() {
            self.project_terminal_chain_inner().await?;
        }

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

    async fn persist_addressed(
        &mut self,
        events: &[AddressedRunEvent],
        outputs: &[(ExecutionInstanceId, Vec<u8>)],
        mut effects: Effects,
    ) -> Result<(), StoreError> {
        // The events that name a scope are addressed at its owner, which may be a root step, but
        // their effects (the compensations they create) are scoped.
        if !events
            .iter()
            .any(|event| is_scope_saga_event(event.event()))
            && events.iter().all(|event| event.path().is_root())
            && outputs
                .iter()
                .all(|(instance, _)| instance.path().is_root())
            && effects.rehint_instances.is_empty()
            && effects
                .lease
                .as_ref()
                .is_none_or(|grant| grant.instance.path().is_root())
            && effects
                .renewal
                .as_ref()
                .is_none_or(|renewal| renewal.instance.path().is_root())
        {
            let root_events = events
                .iter()
                .cloned()
                .map(AddressedRunEvent::into_event)
                .collect::<Vec<_>>();
            effects.outputs.extend(
                outputs
                    .iter()
                    .map(|(instance, bytes)| (instance.node().clone(), bytes.clone())),
            );
            return self.persist(&root_events, effects).await;
        }
        self.persist_scoped_addressed(events, outputs, effects)
            .await
    }

    async fn record_delivery(
        &mut self,
        delivery: &sagat_application::dispatch::DeliveryRef,
    ) -> Result<bool, StoreError> {
        // The primary key is (event, consumer): the second delivery of the same event to the same
        // consumer inserts nothing, and the caller then does not repeat what the delivery did.
        let inserted = sqlx::query(
            "INSERT INTO event_deliveries \
             (tenant_id, namespace_id, source, event_id, consumer, run_id, node, trigger_id, \
              delivered_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, clock_timestamp()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(delivery.namespace_id.as_uuid())
        .bind(&delivery.source)
        .bind(&delivery.event_id)
        .bind(&delivery.consumer)
        .bind(delivery.run_id.map(RunId::as_uuid))
        .bind(delivery.node.as_deref())
        .bind(delivery.trigger_id)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected()
            > 0;
        Ok(inserted)
    }

    async fn commit(mut self) -> Result<(), StoreError> {
        if self.initialize_chain
            && let Some(policy) = self.def.continuation_policy()
        {
            let chain_policy =
                ChainPolicy::new(policy.max_segments()).map_err(|_| StoreError::Internal {
                    detail: "validated continuation policy became invalid".to_owned(),
                })?;
            let mut initialization = ChainInitialization::new(
                ChainScope::new(self.info.tenant_id, self.info.namespace_id),
                self.info.run_id,
                chain_policy,
            );
            if let Some(parent) = &self.info.parent {
                initialization =
                    initialization.with_parent(ChainParent::new(parent.run, parent.node.clone()));
            }
            crate::chain_progress::initialize_chain_in_transaction(&mut self.tx, &initialization)
                .await
                .map_err(|error| match error {
                    ChainProgressError::Storage(error) => error,
                    other => StoreError::Internal {
                        detail: format!("continuation chain initialization failed: {other}"),
                    },
                })?;
        }
        self.tx.commit().await.map_err(store_error)?;
        for waited in self.cancelled_waits {
            metrics::histogram!(
                WorkflowPrimitiveMetric::ApprovalWait.as_str(),
                "outcome" => WorkflowPrimitiveOutcome::Cancelled.as_str()
            )
            .record(waited);
        }
        for signal in self.signals {
            signal.emit();
        }
        Ok(())
    }
}

impl ChainRunTx for PgRunTx {
    fn locked_chain(&self) -> Result<&LockedChainHead, StoreError> {
        self.chain_lock
            .as_ref()
            .ok_or_else(|| StoreError::Internal {
                detail: "run transaction is not bound to a locked chain".to_owned(),
            })
    }
}

impl PgRunTx {
    async fn cancel_approval(
        &mut self,
        owner: &NodeId,
        sequence: u64,
        effects: &Effects,
    ) -> Result<(), StoreError> {
        let row = sqlx::query(
            "SELECT step.status,step.waiting_kind, \
                    step.approval_request_id AS projected_request_id,request.request_id \
             FROM steps step LEFT JOIN approval_requests request \
               ON request.tenant_id=step.tenant_id \
              AND request.request_id=step.approval_request_id \
              AND request.run_id=step.run_id \
              AND request.owner_node=step.node \
              AND request.owner_path_nodes=step.instance_path_nodes \
              AND request.owner_path_indices=step.instance_path_indices \
              AND request.status='PENDING' \
             WHERE step.tenant_id=$1 AND step.run_id=$2 AND step.node=$3 \
               AND cardinality(step.instance_path_nodes)=0 \
             FOR UPDATE OF step",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(owner.as_str())
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?
        .ok_or_else(|| corrupt("cancelled approval owner"))?;
        let stored_status: String = col(&row, "status")?;
        let waiting_kind: Option<String> = col(&row, "waiting_kind")?;
        let projected_request: Option<Uuid> = col(&row, "projected_request_id")?;
        let request_id: Option<Uuid> = col(&row, "request_id")?;
        let Some(request_id) = request_id else {
            if stored_status == "PENDING" && waiting_kind.is_none() && projected_request.is_none() {
                return Ok(());
            }
            return Err(corrupt("cancelled approval owner"));
        };
        if stored_status != "WAITING"
            || waiting_kind.as_deref() != Some("APPROVAL")
            || projected_request != Some(request_id)
        {
            return Err(corrupt("cancelled approval owner"));
        }
        let outbox_id = Uuid::now_v7();
        let updated = sqlx::query(
            "UPDATE approval_requests \
             SET status='CANCELLED',outcome_sequence=$4,settled_at=$5, \
                 outcome_outbox_id=$6,updated_at=$5 \
             WHERE tenant_id=$1 AND run_id=$2 AND request_id=$3 AND status='PENDING' \
             RETURNING created_at",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(request_id)
        .bind(i64::try_from(sequence).map_err(|_| corrupt("approval cancellation sequence"))?)
        .bind(self.now)
        .bind(outbox_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        let Some(updated) = updated else {
            return Err(corrupt("cancelled approval request"));
        };
        let opened_at: OffsetDateTime = col(&updated, "created_at")?;
        self.cancelled_waits
            .push((self.now - opened_at).as_seconds_f64().max(0.0));
        let timer_updated = sqlx::query(
            "UPDATE approval_timers SET state='CANCELLED',updated_at=$3 \
             WHERE tenant_id=$1 AND request_id=$2 AND state='PENDING'",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(request_id)
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if timer_updated != 1 {
            return Err(corrupt("cancelled approval timer"));
        }
        sqlx::query(
            "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
             VALUES ($1,$2,$3,'approval.cancelled',$4)",
        )
        .bind(outbox_id)
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(serde_json::json!({
            "tenantId": self.info.tenant_id.to_string(),
            "namespaceId": self.info.namespace_id.to_string(),
            "runId": self.info.run_id.to_string(),
            "requestId": ApprovalRequestId::new(request_id).to_string(),
            "outcomeSequence": sequence,
            "traceId": effects.trace_id.as_deref().or(self.info.trace_id.as_deref()),
        }))
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        // An operator's cancel brings the audit of the run with it; a stop the platform makes by
        // itself (the run's or the chain's deadline, a parent that cancels its child) has no
        // principal, and the request is audited as the engine's own act in the same transaction.
        let request_resource = ApprovalRequestId::new(request_id).to_string();
        let audit = match effects.audit.as_ref() {
            Some(run_audit) => run_audit.follow_up(
                AuditAction::ApprovalCancelled,
                ResourceType::ApprovalRequest,
                request_resource,
                "approval cancelled with its run",
            ),
            None => AuditRecord::engine(
                self.info.tenant_id,
                AuditAction::ApprovalCancelled,
                ResourceType::ApprovalRequest,
                request_resource,
                "approval cancelled with its run",
                effects
                    .trace_id
                    .clone()
                    .or_else(|| self.info.trace_id.clone()),
                self.now,
            ),
        };
        insert_audit(&mut self.tx, &audit).await?;
        Ok(())
    }

    async fn latch_chain_cancel_inner(&mut self) -> Result<(), StoreError> {
        let Some(chain) = self.chain_lock else {
            return Ok(());
        };
        // Only the open head of an open chain can latch it. A run-addressed cancel of an earlier
        // segment, or of a chain that is already latched or finished, latches nothing: the reducer
        // then refuses it as it refuses any run that has ended, instead of this being "corruption".
        if chain.head() != self.info.run_id
            || chain.final_outcome().is_some()
            || chain.cancel_requested()
        {
            return Ok(());
        }
        let revision = i64::try_from(chain.revision()).map_err(|_| corrupt("chain revision"))?;
        let next: Option<i64> = sqlx::query_scalar(
            "UPDATE execution_chains SET cancel_requested=true,revision=revision+1, \
                    updated_at=clock_timestamp() \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 AND current_run_id=$4 \
               AND revision=$5 AND cancel_requested=false AND final_outcome IS NULL \
             RETURNING revision",
        )
        .bind(chain.scope().tenant_id().as_uuid())
        .bind(chain.scope().namespace_id().as_uuid())
        .bind(chain.chain_id().as_uuid())
        .bind(chain.head().as_uuid())
        .bind(revision)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        let next = next.ok_or_else(|| corrupt("locked chain cancellation latch"))?;
        self.chain_lock = Some(LockedChainHead::restore(
            chain.scope(),
            chain.chain_id(),
            chain.head(),
            u64::try_from(next).map_err(|_| corrupt("chain revision"))?,
            true,
            chain.final_outcome(),
        ));
        Ok(())
    }

    async fn project_terminal_chain_inner(&mut self) -> Result<(), StoreError> {
        let status = self.state.status();
        if !status.is_terminal() {
            return Ok(());
        }
        let chain = self.chain_lock.ok_or_else(|| StoreError::Internal {
            detail: "terminal chain projection is missing its chain lock".to_owned(),
        })?;
        let revision = i64::try_from(chain.revision()).map_err(|_| corrupt("chain revision"))?;
        let completed = sqlx::query(
            "UPDATE chain_segments SET completion_kind='FINAL',completed_at=$5 \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 AND run_id=$4 \
               AND completion_kind IS NULL",
        )
        .bind(chain.scope().tenant_id().as_uuid())
        .bind(chain.scope().namespace_id().as_uuid())
        .bind(chain.chain_id().as_uuid())
        .bind(chain.head().as_uuid())
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        let final_outcome = sqlx::query(
            "UPDATE execution_chains SET final_outcome=$5,revision=revision+1,updated_at=$6 \
             WHERE tenant_id=$1 AND namespace_id=$2 AND chain_id=$3 AND current_run_id=$4 \
               AND revision=$7 AND final_outcome IS NULL",
        )
        .bind(chain.scope().tenant_id().as_uuid())
        .bind(chain.scope().namespace_id().as_uuid())
        .bind(chain.chain_id().as_uuid())
        .bind(chain.head().as_uuid())
        .bind(status.as_str())
        .bind(self.now)
        .bind(revision)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if completed != 1 || final_outcome != 1 {
            return Err(corrupt("locked chain terminal projection"));
        }
        sqlx::query(
            "UPDATE child_chain_links SET final_outcome=$4,updated_at=$5 \
             WHERE tenant_id=$1 AND namespace_id=$2 AND child_chain_id=$3 \
               AND final_outcome IS NULL AND settled=false",
        )
        .bind(chain.scope().tenant_id().as_uuid())
        .bind(chain.scope().namespace_id().as_uuid())
        .bind(chain.chain_id().as_uuid())
        .bind(status.as_str())
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO event_outbox (id,tenant_id,aggregate_id,event_type,payload) \
             VALUES ($1,$2,$3,'chain.finished',$4)",
        )
        .bind(Uuid::now_v7())
        .bind(chain.scope().tenant_id().as_uuid())
        .bind(chain.chain_id().as_uuid())
        .bind(serde_json::json!({
            "tenantId": chain.scope().tenant_id().to_string(),
            "namespaceId": chain.scope().namespace_id().to_string(),
            "chainId": chain.chain_id().to_string(),
            "headRunId": chain.head().to_string(),
            "status": status.as_str(),
            "traceId": self.info.trace_id.clone(),
        }))
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }
    async fn persist_scoped_addressed(
        &mut self,
        events: &[AddressedRunEvent],
        outputs: &[(ExecutionInstanceId, Vec<u8>)],
        effects: Effects,
    ) -> Result<(), StoreError> {
        if events
            .iter()
            .any(|event| event.path().is_root() && !is_scope_saga_event(event.event()))
            || outputs
                .iter()
                .any(|(instance, _)| instance.path().is_root())
            || !effects.outputs.is_empty()
            || !effects.rehint.is_empty()
            || !effects.children.is_empty()
            || !effects.blobs.is_empty()
        {
            return Err(StoreError::Internal {
                detail: "a scoped addressed write contains root-only effects".into(),
            });
        }
        if effects
            .rehint_instances
            .iter()
            .any(|instance| instance.path().is_root())
        {
            return Err(StoreError::Internal {
                detail: "a root rehint used addressed storage".into(),
            });
        }
        if effects
            .lease
            .as_ref()
            .is_some_and(|grant| grant.instance.path().is_root())
            || effects
                .renewal
                .as_ref()
                .is_some_and(|renewal| renewal.instance.path().is_root())
        {
            return Err(StoreError::Internal {
                detail: "a root lease effect used addressed storage".into(),
            });
        }
        scoped_events::require_scoped_event_clock(
            &self.def,
            events,
            &self.event_wait_read_states,
            self.own_scheduler_time,
            self.now,
        )?;
        for addressed in events {
            if !matches!(
                addressed.event(),
                RunEvent::StepLeased { .. }
                    | RunEvent::StepStarted { .. }
                    | RunEvent::StepCompleted { .. }
                    | RunEvent::StepFailed { .. }
                    | RunEvent::ConditionEvaluated { .. }
                    | RunEvent::SwitchEvaluated { .. }
                    | RunEvent::WaitStarted { .. }
                    | RunEvent::WaitTimedOut { .. }
                    | RunEvent::TimerStarted { .. }
                    | RunEvent::TimerFired { .. }
                    | RunEvent::StepRouted { .. }
                    | RunEvent::StepRetryScheduled { .. }
                    | RunEvent::StepRetryDue { .. }
                    | RunEvent::StepReleased { .. }
                    | RunEvent::StepCancelRequested { .. }
                    | RunEvent::StepCancelled { .. }
                    | RunEvent::ReconciliationRequested { .. }
                    | RunEvent::ReconciliationConcluded { .. }
                    | RunEvent::StepNeedsReview { .. }
                    | RunEvent::StepRetryRequested { .. }
                    | RunEvent::StepSkipped { .. }
                    | RunEvent::StepResolved { .. }
                    | RunEvent::ScopeCompensationPlanned { .. }
                    | RunEvent::ScopeCompensated { .. }
            ) {
                return Err(StoreError::Internal {
                    detail: "the scoped event is not supported by addressed storage".into(),
                });
            }
        }

        let version = self.state.version();
        let written = u64::try_from(events.len()).map_err(|_| corrupt("event count"))?;
        if version != self.persisted + written {
            return Err(StoreError::Internal {
                detail: "the in-memory state and the addressed events to write diverged".into(),
            });
        }
        let tenant = self.info.tenant_id;
        let run = self.info.run_id;
        let completed = events
            .iter()
            .filter_map(|addressed| {
                matches!(
                    addressed.event(),
                    RunEvent::StepCompleted { .. } | RunEvent::ReconciliationConcluded { .. }
                )
                .then(|| addressed.instance())
                .flatten()
            })
            .collect::<BTreeSet<_>>();
        let mut output_charges = Vec::with_capacity(outputs.len());
        let mut charged_instances = BTreeSet::new();
        for (instance, bytes) in outputs {
            if !completed.contains(instance) || !charged_instances.insert(instance) {
                return Err(StoreError::Internal {
                    detail: "a scoped output has no unique completion event".into(),
                });
            }
            output_charges.push((instance.clone(), bytes.len()));
        }
        self.charge_logical_output_bytes(&output_charges, &completed)
            .await?;
        let mut output_digest = BTreeMap::new();
        for (instance, bytes) in outputs {
            if !completed.contains(instance) || output_digest.contains_key(instance) {
                return Err(StoreError::Internal {
                    detail: "a scoped output has no unique completion event".into(),
                });
            }
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
            output_digest.insert(instance.clone(), digest);
            if let Some(data) = &mut self.data {
                data.insert_output(instance.clone(), bytes.clone());
            }
        }
        let mut touched = BTreeSet::new();
        let mut hinted = BTreeSet::new();
        let mut rehinted = BTreeSet::new();
        let mut scope_owners: BTreeSet<ExecutionInstanceId> = BTreeSet::new();
        for (offset, addressed) in events.iter().enumerate() {
            let instance = addressed.instance().ok_or_else(|| StoreError::Internal {
                detail: "a scoped event has no logical instance".into(),
            })?;
            let event = addressed.event();
            let encoded = encode(event);
            let seq = i64::try_from(self.persisted + 1 + offset as u64)
                .map_err(|_| corrupt("sequence"))?;
            let (path_nodes, path_indices) = stored_path_parts(instance.path());
            let payload = matches!(
                event,
                RunEvent::StepCompleted { .. } | RunEvent::ReconciliationConcluded { .. }
            )
            .then(|| output_digest.get(&instance))
            .flatten();
            sqlx::query(
                "INSERT INTO run_events \
                 (tenant_id, run_id, seq, event_type, node, attempt, detail, payload_digest, actor, \
                  trace_id, occurred_at, instance_id, instance_path_nodes, instance_path_indices) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
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
            .bind(instance_step_id(self.info.version_id, run, &instance))
            .bind(path_nodes)
            .bind(path_indices)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            // The owner of a scope is a step the collection or loop store writes; the events that
            // name it change the compensations they plan, not the owner row.
            if is_scope_saga_event(event) {
                if let RunEvent::ScopeCompensationPlanned { entries, .. } = event {
                    touched.extend(entries.iter().map(|entry| entry.instance.compensation()));
                }
                scope_owners.insert(instance);
            } else {
                touched.extend(addressed.changed_instances());
                if matches!(
                    event,
                    RunEvent::StepReleased { .. }
                        | RunEvent::StepRetryDue { .. }
                        | RunEvent::StepRetryRequested { .. }
                ) {
                    hinted.insert(instance.clone());
                }
                match event {
                    RunEvent::StepCompleted { .. } if instance.node().is_compensation() => {
                        self.breaker_outcome_instance(&instance, true).await?;
                    }
                    RunEvent::StepRetryScheduled { .. }
                    | RunEvent::StepFailed { fence: Some(_), .. }
                    | RunEvent::StepNeedsReview { fence: Some(_), .. }
                        if instance.node().is_compensation() =>
                    {
                        self.breaker_outcome_instance(&instance, false).await?;
                    }
                    _ => {}
                }
            }
            self.signals.extend(of_event(event));
        }

        if let Some(grant) = &effects.lease {
            touched.insert(grant.instance.clone());
        }
        if let Some(renewal) = &effects.renewal {
            touched.insert(renewal.instance.clone());
        }
        for instance in &effects.rehint_instances {
            if self
                .state
                .instance(instance)
                .is_some_and(|step| step.status() == StepStatus::Ready)
            {
                touched.insert(instance.clone());
                hinted.insert(instance.clone());
                rehinted.insert(instance.clone());
            }
        }

        if !events.is_empty() {
            let head = i64::try_from(version).map_err(|_| corrupt("sequence"))?;
            sqlx::query(
                "INSERT INTO telemetry_pending (tenant_id, run_id, head_seq, applied_seq, behind_since) \
                 VALUES ($1, $2, $3, 0, clock_timestamp()) \
                 ON CONFLICT (tenant_id, run_id) DO UPDATE \
                   SET behind_since = CASE WHEN telemetry_pending.applied_seq >= telemetry_pending.head_seq \
                                           THEN clock_timestamp() ELSE telemetry_pending.behind_since END, \
                       head_seq = GREATEST(telemetry_pending.head_seq, EXCLUDED.head_seq)",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(head)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        }

        for instance in &touched {
            self.upsert_scoped_instance(
                instance,
                hinted.contains(instance),
                rehinted.contains(instance),
                output_digest.get(instance),
                &effects,
            )
            .await?;
        }
        for instance in &hinted {
            self.outbox_task_ready_instance(instance).await?;
        }
        for owner in &scope_owners {
            self.upsert_scope_saga(owner, events).await?;
        }

        if written > 0 {
            let updated = sqlx::query(
                "UPDATE runs SET status = $4, run_version = $5, updated_at = $6 \
                 WHERE tenant_id = $1 AND id = $2 AND run_version = $3",
            )
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .bind(i64::try_from(self.persisted).map_err(|_| corrupt("run version"))?)
            .bind(self.state.status().as_str())
            .bind(i64::try_from(version).map_err(|_| corrupt("run version"))?)
            .bind(self.now)
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

    async fn ready_script_retention(
        &mut self,
        activity: &ActivityName,
    ) -> Result<Option<u16>, StoreError> {
        let Some(digest) = sagat_domain::scripts::digest_of_activity(activity.as_str()) else {
            return Ok(None);
        };
        let days: Option<i16> = sqlx::query_scalar(
            "SELECT max(retention_days) FROM script_versions \
             WHERE tenant_id = $1 AND artifact_digest = $2 AND status = 'READY'",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(digest.as_str())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)?;
        let days = days.ok_or_else(|| StoreError::Internal {
            detail: "a workflow script has no READY version".into(),
        })?;
        u16::try_from(days)
            .ok()
            .filter(|days| (1..=365).contains(days))
            .ok_or_else(|| StoreError::Internal {
                detail: "a READY script has invalid retention".into(),
            })
            .map(Some)
    }

    async fn current_script_retention(
        &mut self,
        activity: &ActivityName,
    ) -> Result<ScriptRetentionDecision, StoreError> {
        if sagat_domain::scripts::digest_of_activity(activity.as_str()).is_none() {
            return Ok(ScriptRetentionDecision::NotScript);
        }
        // Fence the effective policy before selecting the current READY-version maximum. A policy
        // edit that completed first governs this lease; one that starts later waits for commit.
        let cap = match crate::content_usage::script_execution_retention(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            365,
        )
        .await
        {
            Ok(days) => days,
            Err(StoreError::QuotaExceeded { what })
                if what == sagat_domain::quota::QuotaResource::ScriptRetentionDays.limit_name() =>
            {
                return Ok(ScriptRetentionDecision::Refused);
            }
            Err(error) => return Err(error),
        };
        let requested = self
            .ready_script_retention(activity)
            .await?
            .ok_or_else(|| StoreError::Internal {
                detail: "a script activity lost its digest".into(),
            })?;
        Ok(ScriptRetentionDecision::Pinned(requested.min(cap)))
    }

    async fn inherited_script_retention(
        &mut self,
        forward: &NodeId,
        field: &'static str,
        activity: &ActivityName,
    ) -> Result<u16, StoreError> {
        let attempt = self
            .state
            .step(forward)
            .map(sagat_domain::execution::StepState::attempt)
            .ok_or_else(|| StoreError::Internal {
                detail: "a derived script has no originating forward step".into(),
            })?;
        let detail: serde_json::Value = sqlx::query_scalar(
            "SELECT detail FROM run_events \
             WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
               AND event_type = 'step.leased' AND attempt = $4 \
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(forward.as_str())
        .bind(i32::try_from(attempt).map_err(|_| corrupt("attempt"))?)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?
        .ok_or_else(|| StoreError::Internal {
            detail: "a derived script has no originating lease".into(),
        })?;
        match versioned_script_retention(&detail, field)? {
            None => {
                self.ready_script_retention(activity)
                    .await?
                    .ok_or_else(|| StoreError::Internal {
                        detail: "a legacy script obligation has no READY version".into(),
                    })
            }
            Some(days) => Ok(days),
        }
    }

    /// An attempt of the compensation `node` ended: tells the breaker of its activity.
    async fn breaker_outcome(&mut self, node: &NodeId, succeeded: bool) -> Result<(), StoreError> {
        let Some(NodeKind::Activity { activity, .. }) = self.def.node(node).map(Node::kind) else {
            return Ok(());
        };
        self.breaker_outcome_of(activity.as_str().to_owned(), succeeded)
            .await
    }

    /// An attempt of a scoped compensation ended: tells the breaker of its activity, which is the
    /// same breaker the root compensations of that activity share (per tenant, namespace, name).
    async fn breaker_outcome_instance(
        &mut self,
        instance: &ExecutionInstanceId,
        succeeded: bool,
    ) -> Result<(), StoreError> {
        let Some(NodeKind::Activity { activity, .. }) =
            self.def.node_for_instance(instance).map(Node::kind)
        else {
            return Ok(());
        };
        self.breaker_outcome_of(activity.as_str().to_owned(), succeeded)
            .await
    }

    async fn breaker_outcome_of(
        &mut self,
        activity: String,
        succeeded: bool,
    ) -> Result<(), StoreError> {
        let (tenant, namespace) = (
            self.info.tenant_id.as_uuid(),
            self.info.namespace_id.as_uuid(),
        );
        if succeeded {
            crate::breaker::record_success(
                &mut self.tx,
                self.breaker,
                tenant,
                namespace,
                &activity,
            )
            .await?;
        } else if crate::breaker::record_failure(
            &mut self.tx,
            self.breaker,
            tenant,
            namespace,
            &activity,
        )
        .await?
        {
            self.signals.push(Signal::BreakerOpened);
        }
        Ok(())
    }

    /// A chained child answers through its link. The owning parent transaction already locks
    /// the parent, excluding initial child creation/link insertion; no match uses legacy handling.
    /// Never combine a matching link mutation with an UPDATE of the existing child's run row.
    async fn update_child_chain_link(
        &mut self,
        child: RunId,
        cancel: bool,
        finished: bool,
    ) -> Result<bool, StoreError> {
        let changed = sqlx::query(
            "UPDATE child_chain_links SET cancel_requested = (cancel_requested OR $4), \
                    settled = (settled OR ($5 AND final_outcome IS NOT NULL)), updated_at = $6 \
             WHERE tenant_id = $1 AND parent_run_id = $2 AND child_chain_id = $3",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(child.as_uuid())
        .bind(cancel)
        .bind(finished)
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        Ok(changed != 0)
    }

    /// The parent is told the outcome (or no longer cares): the child need not notify it again.
    /// `finished` says the outcome was applied to the parent's step (a child chain's link is then
    /// settled); otherwise the parent only stopped caring and the link is merely latched.
    async fn settle_child(
        &mut self,
        child: RunId,
        cancel: bool,
        finished: bool,
    ) -> Result<(), StoreError> {
        if self
            .update_child_chain_link(child, cancel, finished)
            .await?
        {
            return Ok(());
        }
        sqlx::query(
            "UPDATE runs SET parent_settled = true, \
                    parent_cancel = (parent_cancel OR ($4 AND finished_at IS NULL)) \
             WHERE tenant_id = $1 AND id = $2 AND parent_run_id = $3",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(child.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(cancel)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// A `child` step of this run was cancelled (`settle`) or asked to stop: apply the node's cancel
    /// policy to the child run it reserved, if it exists.
    ///
    /// A chained child's link latch is set under the parent's lock; the scheduler then cancels
    /// the child under its own authority. Only children without a link use the legacy row flags.
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
            self.settle_child(RunId::new(child), cascades, false).await
        } else if cascades {
            if self
                .update_child_chain_link(RunId::new(child), true, false)
                .await?
            {
                return Ok(());
            }
            sqlx::query(
                "UPDATE runs SET parent_cancel = true \
                 WHERE tenant_id = $1 AND id = $2 AND parent_run_id = $3",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(child)
            .bind(self.info.run_id.as_uuid())
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
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1, $2, $3, 'task.ready', $4, $5)",
        )
        .bind(Uuid::now_v7())
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(&activity)
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

    async fn outbox_task_ready_instance(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<(), StoreError> {
        let (path_nodes, path_indices) = stored_path_parts(instance.path());
        let activity: Option<String> = sqlx::query_scalar(
            "SELECT activity FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
               AND instance_path_nodes = $4 AND instance_path_indices = $5",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(&path_nodes)
        .bind(&path_indices)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?
        .flatten();
        let Some(activity) = activity else {
            return Err(StoreError::Internal {
                detail: "a scoped ready step has no activity".into(),
            });
        };
        let path = instance
            .path()
            .segments()
            .iter()
            .map(|segment| {
                serde_json::json!({
                    "scope": segment.scope_node().as_str(),
                    "index": segment.iteration_index(),
                })
            })
            .collect::<Vec<_>>();
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, routing_key, payload) \
             VALUES ($1, $2, $3, 'task.ready', $4, $5)",
        )
        .bind(Uuid::now_v7())
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(&activity)
        .bind(serde_json::json!({
            "tenantId": self.info.tenant_id.to_string(),
            "namespaceId": self.info.namespace_id.to_string(),
            "runId": self.info.run_id.to_string(),
            "node": instance.node().as_str(),
            "instancePath": path,
            "activity": activity,
            "traceId": self.info.trace_id,
        }))
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn outbox_run_finished(&mut self, status: RunStatus) -> Result<(), StoreError> {
        let message = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
             VALUES ($1, $2, $3, 'run.finished', $4)",
        )
        .bind(message)
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(serde_json::json!({
            "tenantId": self.info.tenant_id.to_string(),
            "namespaceId": self.info.namespace_id.to_string(),
            "runId": self.info.run_id.to_string(),
            "status": status.as_str(),
            "traceId": self.info.trace_id,
        }))
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        // What subscribers are told, owed to each of them from this very commit: the event and the
        // obligation to deliver it are written together or not at all. Identifiers and the status,
        // never data of the run.
        let body = serde_json::json!({
            "id": message.to_string(),
            "type": "run.finished",
            "time": self
                .now
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
            "data": {
                "tenantId": self.info.tenant_id.to_string(),
                "namespaceId": self.info.namespace_id.to_string(),
                "runId": self.info.run_id.to_string(),
                "workflow": self.def.name().as_str(),
                "status": status.as_str(),
                // So a subscriber can put what it does about the run in the run's trace.
                "traceId": self.info.trace_id,
            },
        });
        crate::outbound::fan_out(
            &mut self.tx,
            self.info.tenant_id,
            self.info.namespace_id,
            sagat_domain::outbound::OutboundKind::RunFinished,
            &message.to_string(),
            &serde_json::to_vec(&body).unwrap_or_default(),
            self.now,
        )
        .await?;
        Ok(())
    }

    /// Writes the step row for `node` from the current in-memory state.
    /// Writes the state of one saga, and the compensations it planned in `events`.
    async fn upsert_saga(&mut self, node: &NodeId, events: &[RunEvent]) -> Result<(), StoreError> {
        let Some(saga) = self.state.saga(node) else {
            return Ok(());
        };
        let irreversible: Vec<String> = saga
            .irreversible()
            .iter()
            .map(|n| n.as_str().to_owned())
            .collect();
        sqlx::query(
            "INSERT INTO run_sagas \
             (tenant_id, run_id, node, phase, cause_code, outcome, during_stop, irreversible, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (tenant_id, run_id, node) DO UPDATE SET \
                 phase = EXCLUDED.phase, cause_code = EXCLUDED.cause_code, \
                 outcome = EXCLUDED.outcome, during_stop = EXCLUDED.during_stop, \
                 irreversible = EXCLUDED.irreversible, updated_at = EXCLUDED.updated_at",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .bind(saga.phase().as_str())
        .bind(saga.cause().map(FailureCode::as_str))
        .bind(saga.outcome().map(SagaOutcome::as_str))
        .bind(saga.during_stop())
        .bind(&irreversible)
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        for event in events {
            let RunEvent::CompensationPlanned {
                node: planned,
                entries,
                ..
            } = event
            else {
                continue;
            };
            if planned != node {
                continue;
            }
            for entry in entries {
                let after: Vec<String> =
                    entry.after.iter().map(|n| n.as_str().to_owned()).collect();
                sqlx::query(
                    "INSERT INTO run_saga_entries \
                     (tenant_id, run_id, saga_node, node, compensation_id, after) \
                     VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(node.as_str())
                .bind(entry.node.as_str())
                .bind(compensation_id(self.info.version_id, &entry.node))
                .bind(&after)
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;
            }
        }
        Ok(())
    }

    /// Writes the compensation of the scope `owner` and the plan it recorded in `events`.
    async fn upsert_scope_saga(
        &mut self,
        owner: &ExecutionInstanceId,
        events: &[AddressedRunEvent],
    ) -> Result<(), StoreError> {
        let Some(saga) = self.state.scope_saga(owner).cloned() else {
            return Err(corrupt("scope compensation projection"));
        };
        let (path_nodes, path_indices) = stored_path_parts(owner.path());
        let previous = self.persisted;
        let sequence_at = |index: usize| -> Result<i64, StoreError> {
            let offset = u64::try_from(index).map_err(|_| corrupt("scope event sequence"))?;
            previous
                .checked_add(offset)
                .and_then(|value| value.checked_add(1))
                .and_then(|value| i64::try_from(value).ok())
                .ok_or_else(|| corrupt("scope event sequence"))
        };
        let planned = events
            .iter()
            .position(|event| {
                matches!(event.event(), RunEvent::ScopeCompensationPlanned { .. })
                    && event.instance().as_ref() == Some(owner)
            })
            .map(sequence_at)
            .transpose()?;
        let ended = events
            .iter()
            .position(|event| {
                matches!(event.event(), RunEvent::ScopeCompensated { .. })
                    && event.instance().as_ref() == Some(owner)
            })
            .map(sequence_at)
            .transpose()?;
        if let Some(planned_sequence) = planned {
            sqlx::query(
                "INSERT INTO scope_sagas \
                 (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node, phase, \
                  cause_code, outcome, during_stop, planned_sequence, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.run_id.as_uuid())
            .bind(&path_nodes)
            .bind(&path_indices)
            .bind(owner.node().as_str())
            .bind(SagaPhase::Compensating.as_str())
            .bind(saga.cause().as_str())
            .bind(None::<&str>)
            .bind(saga.during_stop())
            .bind(planned_sequence)
            .bind(self.now)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            for (forward, after) in saga.entries() {
                self.insert_scope_saga_entry(
                    owner,
                    forward,
                    Some(instance_compensation_id(self.info.version_id, forward)),
                    after,
                )
                .await?;
            }
            for forward in saga.irreversible() {
                self.insert_scope_saga_entry(owner, forward, None, &[])
                    .await?;
            }
        }
        if saga.outcome().is_some() {
            let ended_sequence = ended.ok_or_else(|| corrupt("scope compensation end event"))?;
            let updated = sqlx::query(
                "UPDATE scope_sagas SET phase = $6, outcome = $7, during_stop = $8, \
                        ended_sequence = $9, updated_at = $10 \
                 WHERE tenant_id = $1 AND run_id = $2 AND owner_path_nodes = $3 \
                   AND owner_path_indices = $4 AND owner_node = $5 AND phase = 'COMPENSATING'",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.run_id.as_uuid())
            .bind(&path_nodes)
            .bind(&path_indices)
            .bind(owner.node().as_str())
            .bind(saga.phase().as_str())
            .bind(saga.outcome().map(SagaOutcome::as_str))
            .bind(saga.during_stop())
            .bind(ended_sequence)
            .bind(self.now)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            if updated != 1 {
                return Err(corrupt("scope compensation projection"));
            }
        }
        Ok(())
    }

    async fn insert_scope_saga_entry(
        &mut self,
        owner: &ExecutionInstanceId,
        forward: &ExecutionInstanceId,
        compensation_id: Option<Uuid>,
        after: &[ExecutionInstanceId],
    ) -> Result<(), StoreError> {
        let (path_nodes, path_indices) = stored_path_parts(owner.path());
        let iteration = |instance: &ExecutionInstanceId| {
            instance
                .path()
                .segments()
                .last()
                .map(|segment| i32::try_from(segment.iteration_index()))
                .and_then(Result::ok)
                .ok_or_else(|| corrupt("scope compensation member"))
        };
        let after_iterations = after.iter().map(iteration).collect::<Result<Vec<_>, _>>()?;
        let after_nodes: Vec<String> = after
            .iter()
            .map(|instance| instance.node().as_str().to_owned())
            .collect();
        sqlx::query(
            "INSERT INTO scope_saga_entries \
             (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node, \
              iteration_index, node, kind, compensation_id, after_iterations, after_nodes) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .bind(owner.node().as_str())
        .bind(iteration(forward)?)
        .bind(forward.node().as_str())
        .bind(if compensation_id.is_some() {
            "COMPENSATE"
        } else {
            "IRREVERSIBLE"
        })
        .bind(compensation_id)
        .bind(&after_iterations)
        .bind(&after_nodes)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn upsert_scoped_instance(
        &mut self,
        instance: &ExecutionInstanceId,
        hinted: bool,
        rehinted: bool,
        output: Option<&Vec<u8>>,
        effects: &Effects,
    ) -> Result<(), StoreError> {
        let step = self
            .state
            .instance(instance)
            .cloned()
            .ok_or_else(|| corrupt("scoped step projection"))?;
        // A scoped step can wait for the answer of its lookup or for a person (D386, D387); the
        // other waits belong to the stores of their own primitive.
        let (waiting_kind, due_at) = match step.waiting_on() {
            None => (None, None),
            Some(WaitingOn::Reconciliation) => (Some("RECONCILIATION"), None),
            Some(WaitingOn::Operator { .. }) => (Some("OPERATOR"), None),
            Some(WaitingOn::Timer { due_at }) => (Some("TIMER"), Some(*due_at)),
            Some(WaitingOn::Event { deadline, .. }) => (Some("EVENT"), Some(*deadline)),
            Some(_) => {
                return Err(StoreError::Internal {
                    detail: "the scoped step projection is not supported by addressed storage"
                        .into(),
                });
            }
        };
        validate_scoped_control_projection(self.def.node_for_instance(instance), &step)?;
        let kind = self
            .def
            .node_for_instance(instance)
            .map_or("FORWARD", |node| node.origin().label());
        let node_kind = self
            .def
            .node_for_instance(instance)
            .map_or("activity", |node| node.kind().label());
        let pinned = self.def.node_for_instance(instance);
        let next_event = EventWaitReadState::from_step(&step);
        let old_event = self.event_wait_read_states.get(instance).cloned();
        let event_type = next_event.event_type(pinned).map(str::to_owned);
        let event_since = if node_kind == "waitEvent" {
            next_event.since(pinned)?
        } else {
            None
        };
        let old_event_type = old_event
            .as_ref()
            .and_then(|read| read.event_type(pinned))
            .map(str::to_owned);
        let old_event_since = if node_kind == "waitEvent" {
            old_event
                .as_ref()
                .map(|read| read.since(pinned))
                .transpose()?
                .flatten()
        } else {
            None
        };
        let status = step.status().as_str();
        let attempt = i32::try_from(step.attempt()).map_err(|_| corrupt("attempt"))?;
        let generation = i64::try_from(step.generation()).map_err(|_| corrupt("generation"))?;
        let grant = effects
            .lease
            .as_ref()
            .filter(|grant| &grant.instance == instance);
        let renewal = effects
            .renewal
            .as_ref()
            .filter(|renewal| &renewal.instance == instance);
        let reported = effects
            .reported
            .as_ref()
            .filter(|reported| &reported.instance == instance);
        let leased = matches!(step.status(), StepStatus::Leased | StepStatus::Running);
        let fresh_grant = grant.filter(|_| leased);
        let (path_nodes, path_indices) = stored_path_parts(instance.path());
        let step_uuid = instance_step_id(self.info.version_id, self.info.run_id, instance);
        if !self.stored.contains(instance) {
            if !matches!(step.status(), StepStatus::Pending | StepStatus::Ready) {
                return Err(StoreError::Internal {
                    detail: "a scoped step cannot be created after it started".into(),
                });
            }
            let activity =
                self.def
                    .node_for_instance(instance)
                    .and_then(|node| match node.kind() {
                        NodeKind::Activity { activity, .. } => Some(activity.as_str().to_owned()),
                        _ => None,
                    });
            if step.status() == StepStatus::Ready && activity.is_none() {
                return Err(StoreError::Internal {
                    detail: "a scoped ready step has no declared activity".into(),
                });
            }
            // The secrets the step declares, so the claim query can leave out the steps a worker is
            // not entitled to receive (G48). None when it declares none.
            let declared: Option<Vec<String>> = self
                .def
                .node_for_instance(instance)
                .map(|n| {
                    n.secrets()
                        .iter()
                        .map(|s| s.as_str().to_owned())
                        .collect::<Vec<_>>()
                })
                .filter(|names| !names.is_empty());
            match self.def.node_for_instance(instance).map(Node::kind) {
                Some(NodeKind::Return { .. }) => {}
                Some(_) => self.consume_scoped_reservation(instance).await?,
                None => {}
            }
            sqlx::query(
                "INSERT INTO steps \
                 (tenant_id, run_id, node, step_id, namespace_id, status, attempt, generation, \
                  cancel_requested, routed, timed_out, failure_code, next_attempt_at, activity, \
                  ready_at, last_hinted_at, output_digest, updated_at, kind, \
                  instance_path_nodes, instance_path_indices, waiting_kind, secrets, node_kind) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, \
                         CASE WHEN $6 = 'READY' THEN $15 END, CASE WHEN $16 THEN $15 END, $17, \
                         $15, $20, $18, $19, $21, $22, $23)",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.run_id.as_uuid())
            .bind(instance.node().as_str())
            .bind(step_uuid)
            .bind(self.info.namespace_id.as_uuid())
            .bind(status)
            .bind(attempt)
            .bind(generation)
            .bind(step.cancel_requested())
            .bind(step.routed())
            .bind(step.timed_out())
            .bind(step.failure().map(FailureCode::as_str))
            .bind(step.next_attempt_at())
            .bind(activity.as_deref())
            .bind(self.now)
            .bind(hinted)
            .bind(output.cloned())
            .bind(path_nodes)
            .bind(path_indices)
            .bind(kind)
            .bind(waiting_kind)
            .bind(declared)
            .bind(node_kind)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            self.stored.insert(instance.clone());
            if node_kind == "timer" {
                self.timer_read_states
                    .insert(instance.clone(), TimerReadState::from_step(&step));
            }
            if node_kind == "waitEvent" {
                self.event_wait_read_states
                    .insert(instance.clone(), next_event);
            }
            return Ok(());
        }

        if !matches!(node_kind, "timer" | "waitEvent") {
            self.ensure_node_kind(instance, node_kind).await?;
        }

        let updated = sqlx::query(concat!(
            "UPDATE steps AS s SET status = $6, attempt = $7, generation = $8, \
                    cancel_requested = $9, routed = $10, timed_out = $11, failure_code = $12, \
                    next_attempt_at = $13, \
                    ready_at = CASE WHEN $6 = 'READY' \
                                    THEN COALESCE(CASE WHEN status = 'READY' THEN ready_at END, $14) \
                                    ELSE ready_at END, \
                    last_hinted_at = CASE WHEN $15 THEN $14 ELSE last_hinted_at END, \
                    hint_count = CASE WHEN $24 THEN hint_count + 1 WHEN $15 THEN 0 ELSE hint_count END, \
                    lease_token_hash = CASE WHEN $6 IN ('LEASED', 'RUNNING') \
                                            THEN COALESCE($19, lease_token_hash) END, \
                    lease_expires_at = CASE WHEN $6 NOT IN ('LEASED', 'RUNNING') THEN NULL \
                                            WHEN $19::bytea IS NOT NULL \
                                                 THEN $14 + $20 * interval '1 microsecond' \
                                            WHEN $23::bigint IS NOT NULL THEN LEAST( \
                                                 $14 + $23 * interval '1 microsecond', \
                                                 COALESCE(attempt_deadline_at, 'infinity')) \
                                            ELSE lease_expires_at END, \
                    attempt_deadline_at = CASE WHEN $6 NOT IN ('LEASED', 'RUNNING') THEN NULL \
                                               WHEN $19::bytea IS NOT NULL \
                                                    THEN CASE WHEN $21::bigint IS NOT NULL \
                                                              THEN $14 + $21 * interval '1 microsecond' END \
                                               ELSE attempt_deadline_at END, \
                    leased_by = CASE WHEN $6 NOT IN ('LEASED', 'RUNNING') THEN NULL \
                                     WHEN $19::bytea IS NOT NULL THEN $22 ELSE leased_by END, \
                    reported_lease_token_hash = CASE WHEN $6 IN ('LEASED', 'RUNNING') THEN NULL \
                                                     WHEN $25::bytea IS NOT NULL THEN $25 \
                                                     ELSE reported_lease_token_hash END, \
                    output_digest = COALESCE($16, output_digest), waiting_kind = $26, \
                    updated_at = $14, condition = $27, switch_case_id = $28,due_at=$31, \
                    event_type=CASE WHEN $30='waitEvent' THEN $35 ELSE s.event_type END, \
                    correlation_key=CASE WHEN $30='waitEvent' THEN $36 ELSE s.correlation_key END, \
                    event_since=CASE WHEN $30='waitEvent' THEN $37 ELSE s.event_since END, \
                    event_scanned_at=CASE WHEN $30='waitEvent' THEN NULL ELSE s.event_scanned_at END \
             WHERE tenant_id = $1 AND run_id = $2 AND node = $3 AND step_id = $4 \
               AND namespace_id = $5 AND instance_path_nodes = $17 AND instance_path_indices = $18 \
               AND (NOT $29 OR (s.node_kind=$30 AND (($30='timer' AND (",
            scoped_timer_metadata_valid!(),
            ")) OR ($30 IN ('condition','switch') AND (",
            scoped_control_metadata_empty!(),
            ")) OR ($30='waitEvent' AND (",
            scoped_event_metadata_valid!(),
            "))))) AND ($30<>'timer' OR (s.due_at IS NOT DISTINCT FROM $32 \
              AND s.status=$33 AND s.routed=$34)) \
               AND ($30<>'waitEvent' OR (s.due_at IS NOT DISTINCT FROM $38 AND s.status=$39 AND s.routed=$40 \
                 AND s.event_type IS NOT DISTINCT FROM $41 AND s.correlation_key IS NOT DISTINCT FROM $42 \
                 AND s.event_since IS NOT DISTINCT FROM $43))",
        ))
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(instance.node().as_str())
        .bind(step_uuid)
        .bind(self.info.namespace_id.as_uuid())
        .bind(status)
        .bind(attempt)
        .bind(generation)
        .bind(step.cancel_requested())
        .bind(step.routed())
        .bind(step.timed_out())
        .bind(step.failure().map(FailureCode::as_str))
        .bind(step.next_attempt_at())
        .bind(self.now)
        .bind(hinted)
        .bind(output.cloned())
        .bind(path_nodes)
        .bind(path_indices)
        .bind(fresh_grant.map(|grant| grant.token_hash.clone()))
        .bind(fresh_grant.map_or(0, |grant| micros(grant.ttl)))
        .bind(
            fresh_grant
                .and_then(|grant| grant.attempt_timeout)
                .map(micros),
        )
        .bind(fresh_grant.map(|grant| grant.worker_id.clone()))
        .bind(renewal.map(|renewal| micros(renewal.ttl)))
        .bind(rehinted)
        .bind(reported.map(|reported| reported.token_hash.clone()))
        .bind(waiting_kind)
        .bind(step.condition())
        .bind(stored_switch_case_id(&step))
        .bind(matches!(node_kind, "condition" | "switch" | "timer" | "waitEvent"))
        .bind(node_kind)
        .bind(due_at)
        .bind(self.timer_read_states.get(instance).and_then(|read| read.due_at))
        .bind(self.timer_read_states.get(instance).map(|read| read.status.as_str()))
        .bind(self.timer_read_states.get(instance).map(|read| read.routed))
        .bind(event_type.as_deref())
        .bind(next_event.key.as_ref().map(CorrelationKey::as_str))
        .bind(event_since)
        .bind(old_event.as_ref().and_then(|read|read.deadline))
        .bind(old_event.as_ref().map(|read|read.status.as_str()))
        .bind(old_event.as_ref().map(|read|read.routed))
        .bind(old_event_type.as_deref())
        .bind(old_event.as_ref().and_then(|read|read.key.as_ref()).map(CorrelationKey::as_str))
        .bind(old_event_since)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if updated != 1 {
            return Err(corrupt("scoped step identity"));
        }
        if node_kind == "timer" {
            self.timer_read_states
                .insert(instance.clone(), TimerReadState::from_step(&step));
        }
        if node_kind == "waitEvent" {
            self.event_wait_read_states
                .insert(instance.clone(), next_event);
        }
        Ok(())
    }

    async fn consume_scoped_reservation(
        &mut self,
        instance: &ExecutionInstanceId,
    ) -> Result<(), StoreError> {
        let (path_nodes, path_indices) = stored_path_parts(instance.path());
        let collection = sqlx::query(
            "UPDATE collection_iterations SET remaining_reservations=remaining_reservations-1, \
                    updated_at=$5 \
             WHERE tenant_id=$1 AND run_id=$2 AND entry_path_nodes=$3 AND entry_path_indices=$4 \
               AND lifecycle IN ('ADMITTED','CHECKPOINTED','DRAINED') AND remaining_reservations>0",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(&path_nodes)
        .bind(&path_indices)
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        let loop_iteration = sqlx::query(
            "UPDATE loop_iterations SET remaining_reservations=remaining_reservations-1, \
                    updated_at=$5 \
             WHERE tenant_id=$1 AND run_id=$2 AND entry_path_nodes=$3 AND entry_path_indices=$4 \
               AND lifecycle IN ('ADMITTED','CHECKPOINTED','DRAINED') AND remaining_reservations>0",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(path_nodes)
        .bind(path_indices)
        .bind(self.now)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if collection + loop_iteration != 1 {
            return Err(corrupt("scoped instance reservation"));
        }
        let usage = sqlx::query(
            "UPDATE run_instance_usage SET admitted_instances=admitted_instances+1, \
                    reserved_instances=reserved_instances-1 \
             WHERE tenant_id=$1 AND run_id=$2 AND reserved_instances>0",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        if usage.rows_affected() != 1 {
            return Err(corrupt("scoped instance usage"));
        }
        Ok(())
    }

    async fn upsert_step(
        &mut self,
        node: &NodeId,
        effects: &Effects,
        hinted: bool,
        rehinted: bool,
        output: Option<&Vec<u8>>,
    ) -> Result<(), StoreError> {
        let Some(step) = self.state.step(node).cloned() else {
            return Ok(());
        };
        let def_node = self.def.node(node);
        let activity = match def_node.map(sagat_domain::workflow::Node::kind) {
            Some(NodeKind::Activity { activity, .. }) => Some(activity.as_str().to_owned()),
            _ => None,
        };
        let event_type = match def_node.map(sagat_domain::workflow::Node::kind) {
            Some(NodeKind::WaitEvent { event_type, .. }) => Some(event_type.as_str().to_owned()),
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
            Some(WaitingOn::Saga) => (Some("SAGA"), None, None),
            Some(WaitingOn::Reconciliation) => (Some("RECONCILIATION"), None, None),
            Some(WaitingOn::Operator { .. }) => (Some("OPERATOR"), None, None),
            Some(WaitingOn::Collection) => {
                return Err(StoreError::Internal {
                    detail: "collection projection is not persisted".into(),
                });
            }
            Some(WaitingOn::Loop) => {
                return Err(StoreError::Internal {
                    detail: "loop projection is not persisted".into(),
                });
            }
            Some(WaitingOn::Approval { .. }) => {
                return Err(StoreError::Internal {
                    detail: "approval projection is not persisted by the legacy run transaction"
                        .into(),
                });
            }
        };
        // How far back an event wait accepts events, beyond the moment it began waiting.
        let lookback = match def_node.map(sagat_domain::workflow::Node::kind) {
            Some(NodeKind::WaitEvent { lookback, .. }) => micros(*lookback),
            _ => 0,
        };
        let kind = def_node.map_or("FORWARD", |n| n.origin().label());
        let node_kind = def_node.map_or("legacyControl", |node| node.kind().label());
        // A `child` step reserved in this transaction: the run id, and the input it starts with.
        let spawn = effects.children.iter().find(|c| &c.node == node);
        let child_run = match step.waiting_on() {
            Some(WaitingOn::Child { run }) => Some(run.as_uuid()),
            _ => spawn.map(|c| c.child.as_uuid()),
        };
        let child_input = spawn.map(|c| c.input.clone());
        let grant = effects
            .lease
            .as_ref()
            .filter(|grant| grant.instance.path().is_root() && grant.instance.node() == node);
        let renewal = effects
            .renewal
            .as_ref()
            .filter(|renewal| renewal.instance.path().is_root() && renewal.instance.node() == node);
        let reported = effects
            .reported
            .as_ref()
            .filter(|r| r.instance.path().is_root() && r.instance.node() == node);
        let leased = matches!(step.status(), StepStatus::Leased | StepStatus::Running);
        let fresh_grant = grant.filter(|_| leased);
        let instance = ExecutionInstanceId::root(node.clone());
        let step_uuid = instance_step_id(self.info.version_id, self.info.run_id, &instance);
        let status = step.status().as_str();
        let switch_case_id = stored_switch_case_id(&step);
        let attempt = i32::try_from(step.attempt()).map_err(|_| corrupt("attempt"))?;
        let generation = i64::try_from(step.generation()).map_err(|_| corrupt("generation"))?;
        let inserted = !self.stored.contains(&instance);
        // The secrets the step declares, copied onto its row so that the claim query can leave out
        // the steps a worker is not entitled to receive (G48). None when it declares none.
        let declared: Option<Vec<String>> = def_node
            .map(|n| {
                n.secrets()
                    .iter()
                    .map(|s| s.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
            .filter(|names| !names.is_empty());
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
                  last_hinted_at, output_digest, child_run_id, child_input, updated_at, kind, \
                  event_since, switch_case_id, secrets, node_kind) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
                         $18, $19, CASE WHEN $6 = 'READY' THEN $20 END, CASE WHEN $21 THEN $20 END, \
                         $22, $23, $24, $20, $25, \
                         CASE WHEN $15 = 'EVENT' THEN $20 - $26 * interval '1 microsecond' END, $27, $28, $29)",
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
            .bind(if waiting_kind == Some("EVENT") {
                event_type.as_deref()
            } else {
                None
            })
            .bind(correlation_key)
            .bind(activity.as_deref())
            .bind(self.now)
            .bind(hinted)
            .bind(output.cloned())
            .bind(child_run)
            .bind(child_input)
            .bind(kind)
            .bind(lookback)
            .bind(switch_case_id)
            .bind(declared)
            .bind(node_kind)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
            self.stored.insert(instance);
            return Ok(());
        }
        self.ensure_node_kind(&instance, node_kind).await?;
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
                hint_count = CASE WHEN $30 THEN hint_count + 1 WHEN $21 THEN 0 ELSE hint_count END, \
                lease_token_hash = CASE WHEN $6 IN ('LEASED', 'RUNNING') \
                                        THEN COALESCE($22, lease_token_hash) END, \
                reported_lease_token_hash = CASE WHEN $6 IN ('LEASED', 'RUNNING') THEN NULL \
                                                 WHEN $32::bytea IS NOT NULL THEN $32 \
                                                 ELSE reported_lease_token_hash END, \
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
                approval_request_id = CASE WHEN $15 = 'APPROVAL' \
                                           THEN approval_request_id ELSE NULL END, \
                event_since = CASE WHEN $15::text IS DISTINCT FROM 'EVENT' THEN NULL \
                                   ELSE COALESCE(CASE WHEN waiting_kind = 'EVENT' THEN event_since END, \
                                                 $20 - $31 * interval '1 microsecond') END, \
                event_scanned_at = CASE WHEN $15 = 'EVENT' AND waiting_kind = 'EVENT' \
                                        THEN event_scanned_at END, \
                updated_at = $20, switch_case_id = $33 \
             WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
               AND cardinality(instance_path_nodes) = 0",
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
        .bind(if waiting_kind == Some("EVENT") {
            event_type.as_deref()
        } else {
            None
        })
        .bind(correlation_key)
        .bind(activity.as_deref())
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
        .bind(rehinted)
        .bind(lookback)
        .bind(reported.map(|r| r.token_hash.clone()))
        .bind(switch_case_id)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }
}

fn versioned_script_retention(
    detail: &serde_json::Value,
    field: &'static str,
) -> Result<Option<u16>, StoreError> {
    match detail.get("scriptRetentionPolicyVersion") {
        None => Ok(None),
        Some(marker) if marker.as_u64() == Some(1) => detail
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .and_then(|days| u16::try_from(days).ok())
            .filter(|days| (1..=365).contains(days))
            .map(Some)
            .ok_or_else(|| StoreError::Internal {
                detail: "a versioned script lease is missing its retention pin".into(),
            }),
        _ => Err(StoreError::Internal {
            detail: "a script lease has an unknown retention policy version".into(),
        }),
    }
}

/// The run store also serves definitions, so one value can back every service that needs both.
impl DefinitionStore for PgRunStore {
    async fn check_database_access(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        definition: &WorkflowDefinition,
    ) -> Result<(), StoreError> {
        self.definitions
            .check_database_access(principal, namespace, definition)
            .await
    }
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
        max_in_namespace: u64,
    ) -> Result<Applied<()>, StoreError> {
        self.definitions
            .create_workflow(workflow, draft, audit, idempotency, max_in_namespace)
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
        declared_state: Option<&sagat_application::declared_operations::PublishedDeclaredState>,
        definition: &WorkflowDefinition,
        release_notes: Option<&str>,
        plans: &[sagat_domain::saga::SagaPlan],
        triggers: &[sagat_application::dispatch::TriggerRegistration],
        context: &sagat_application::definitions::PublishContext,
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
                declared_state,
                definition,
                release_notes,
                plans,
                triggers,
                context,
                audit,
                idempotency,
                now,
            )
            .await
    }

    async fn publication_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Option<sagat_domain::versioning::ApprovalPolicy>, StoreError> {
        self.definitions.publication_policy(tenant, namespace).await
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

    async fn compensation_plans(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
    ) -> Result<Vec<sagat_application::definitions::StoredPlan>, StoreError> {
        self.definitions.compensation_plans(tenant, version).await
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

#[cfg(test)]
mod tests {
    use sagat_application::error::StoreError;
    use serde_json::json;

    use super::versioned_script_retention;

    #[test]
    fn script_retention_pin_parser_grandfathers_only_unversioned_leases() {
        assert_eq!(
            versioned_script_retention(&json!({"attempt": 1}), "scriptRetentionDays")
                .expect("legacy lease"),
            None
        );
        assert_eq!(
            versioned_script_retention(
                &json!({
                    "scriptRetentionPolicyVersion": 1,
                    "scriptRetentionDays": 7
                }),
                "scriptRetentionDays",
            )
            .expect("versioned pin"),
            Some(7)
        );
        for corrupt in [
            json!({"scriptRetentionPolicyVersion": 1}),
            json!({"scriptRetentionPolicyVersion": 1, "scriptRetentionDays": 0}),
            json!({"scriptRetentionPolicyVersion": 2, "scriptRetentionDays": 7}),
        ] {
            assert!(matches!(
                versioned_script_retention(&corrupt, "scriptRetentionDays"),
                Err(StoreError::Internal { .. })
            ));
        }
    }
}

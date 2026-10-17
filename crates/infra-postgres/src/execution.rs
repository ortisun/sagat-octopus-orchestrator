//! PostgreSQL implementation of the run store: one locked transaction per command.
//!
//! [`PgRunStore::begin`] takes the run's row lock (`FOR UPDATE`, or `SKIP LOCKED` for workers),
//! loads the immutable definition and the current state from the projection rows, and hands both to
//! the engine. [`PgRunTx::persist`] then writes, in that same transaction: the journal entries, the
//! step rows, the payloads, the outbox, the run row, the audit record and the idempotency record.
//! Nothing outside the transaction ever sees a half-applied command, and a crash before `COMMIT`
//! leaves the previous state intact (`FR-002-003`).

use crate::pool::BeginSafely;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use sagat_application::definitions::{
    Applied, DefinitionStore, Idempotency, VersionRecord, WorkflowRecord,
};
use sagat_application::error::StoreError;
use sagat_application::execution::{
    BreakerPolicy, DefinitionCache, Effects, LeaseDeadlines, ParentLink, RunData, RunInfo,
    RunRecord, RunTx, compensation_id, step_id,
};
use sagat_application::runs::{
    GraphSnapshot, JournalEntry, Lock, NewRun, RunFilter, RunStore, SagaView, StepView,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersion, WorkflowVersionId};
use sagat_domain::execution::{
    CorrelationKey, FailureCode, RunEvent, RunId, RunState, RunStatus, SagaPhase, SagaState,
    StepState, StepStatus, StopReason, WaitingOn,
};
use sagat_domain::identity::{Namespace, NamespaceId, Principal, Slug, TenantId};
use sagat_domain::saga::{NodeOrigin, SagaOutcome};
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::{ActivityName, Digest, Node, NodeId, NodeKind, WorkflowDefinition};
use sha2::{Digest as _, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

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
        }
    }

    /// When the compensations of a provider operation stop being leased (FR-003-003).
    #[must_use]
    pub const fn with_breaker(mut self, policy: BreakerPolicy) -> Self {
        self.breaker = policy;
        self
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
    ///
    /// The document is read on `conn`, never on a connection of its own: a caller that holds a
    /// transaction and waits for a second connection from the same pool starves the pool when enough
    /// callers do it at once (a burst of first requests for a version nobody has cached yet).
    async fn definition(
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
        if replayed != stored {
            return Err(format!(
                "projection differs from the journal:\n  replayed: {replayed:?}\n  stored:   {stored:?}"
            ));
        }
        drop(tx);
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

impl RunStore for PgRunStore {
    type Tx = PgRunTx;

    async fn replay_start(
        &self,
        tenant: TenantId,
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
        tx.rollback().await.map_err(store_error)?;
        Ok(result)
    }

    async fn create(&self, new: NewRun) -> Result<Applied<PgRunTx>, StoreError> {
        let tenant = new.info.tenant_id;
        let hash = hash(&new.idempotency.fingerprint);
        let input_digest = Sha256::digest(&new.input).to_vec();
        for _ in 0..2 {
            let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
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
            sqlx::query(
                "INSERT INTO runs (id, tenant_id, namespace_id, workflow_id, workflow_version_id, \
                 status, run_version, paused, input_digest, correlation_id, started_by, trace_id, \
                 deadline_at, created_at, updated_at, parent_run_id, parent_node, cancel_policy, \
                 depth, root_run_id, trace_parent_span, rerun_of, input_blob_id) \
                 VALUES ($1, $2, $3, $4, $5, 'RUNNING', 0, false, $6, $7, $8, $9, $10, $11, $11, \
                         $12, $13, $14, $15, $16, $17, $18, $19)",
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
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            if let Some(sagat_application::blobs::RunInputBinding::Uploaded(descriptor)) =
                &new.input_blob
                && let Err(error) = crate::blobs::commit_input(&mut tx, info, descriptor).await
            {
                tx.rollback().await.map_err(store_error)?;
                if matches!(error, StoreError::MissingReference)
                    && let Some(existing) = self.replay_start(tenant, &new.idempotency).await?
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
                    info,
                    now,
                    def: Arc::clone(&new.definition),
                    state: RunState::new(new.info.run_id),
                    persisted: 0,
                    data: Some(RunData {
                        run_id: Some(new.info.run_id),
                        input: new.input.clone(),
                        input_blob,
                        outputs: BTreeMap::new(),
                    }),
                    input_digest,
                    stored: BTreeSet::new(),
                    breaker: self.breaker,
                    script_retention: BTreeMap::new(),
                    signals: Vec::new(),
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
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
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
            let node = NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?;
            stored.insert(node.clone());
            steps.insert(node, step_state(row)?);
        }
        let sagas = load_sagas(&mut tx, tenant, run).await?;
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
        let state = RunState::restore(
            run,
            record.status,
            stop,
            paused,
            steps,
            sagas,
            record.version,
        );
        let def = self
            .definition(&mut tx, tenant, record.info.version_id)
            .await?;
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
            breaker: self.breaker,
            script_retention: BTreeMap::new(),
            signals: Vec::new(),
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
                    created_at, updated_at, finished_at, parent_run_id, root_run_id, parent_node, \
                    cancel_policy, depth, trace_parent_span, rerun_of \
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
            "SELECT seq, event_type, node, attempt, actor, trace_id, occurred_at, detail, \
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
        // The identity of each step, which the span of an attempt is computed from.
        let steps: std::collections::HashMap<String, Uuid> =
            sqlx::query("SELECT node, step_id FROM steps WHERE tenant_id = $1 AND run_id = $2")
                .bind(tenant.as_uuid())
                .bind(run.as_uuid())
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .iter()
                .map(|r| Ok((col::<String>(r, "node")?, col::<Uuid>(r, "step_id")?)))
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
                let touched = decoded.as_ref().map(|event| {
                    event
                        .changed_nodes()
                        .iter()
                        .map(|n| n.as_str().to_owned())
                        .collect()
                });
                let attempt = attempt.and_then(|a| u32::try_from(a).ok());
                let span_id = attempt.and_then(|a| {
                    node.as_deref()
                        .and_then(|n| steps.get(n))
                        .map(|step| sagat_telemetry::attempt_span_id(*step, a).to_string())
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
            "SELECT node, status, attempt, generation, cancel_requested, failure_code, \
                    next_attempt_at, due_at, waiting_kind, child_run_id \
             FROM steps WHERE tenant_id = $1 AND run_id = $2 ORDER BY node",
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
        tx.commit().await.map_err(store_error)?;
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
                Ok(StepView {
                    node: NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?,
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
                        _ => None,
                    },
                    child_run: col::<Option<Uuid>>(row, "child_run_id")?.map(RunId::new),
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
    root: RunId,
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
    // One tree, one budget. The advisory lock serialises only the creators of this tree's runs (each
    // already holds its own parent's lock, and none waits for anything else while holding it), so the
    // count cannot be overshot by racing siblings and no row lock of the root is ever needed, which
    // would deadlock against the root's own commands.
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

const RUN_SELECT: &str = "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id, status, \
     run_version, stop_reason, paused, correlation_id, started_by, trace_id, deadline_at, \
     created_at, updated_at, finished_at, parent_run_id, root_run_id, parent_node, cancel_policy, depth, \
     trace_parent_span, rerun_of \
     FROM runs WHERE tenant_id = $1 AND id = $2";

const RUN_FOR_UPDATE: &str = "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id, \
     status, run_version, stop_reason, stop_code, paused, input_digest, correlation_id, started_by, \
     trace_id, deadline_at, created_at, updated_at, finished_at, parent_run_id, root_run_id, parent_node, \
     cancel_policy, depth, trace_parent_span, rerun_of, clock_timestamp() AS db_now \
     FROM runs WHERE tenant_id = $1 AND id = $2 FOR UPDATE";

const RUN_FOR_UPDATE_SKIP_LOCKED: &str = "SELECT id, tenant_id, namespace_id, workflow_id, workflow_version_id, \
     status, run_version, stop_reason, stop_code, paused, input_digest, correlation_id, started_by, \
     trace_id, deadline_at, created_at, updated_at, finished_at, parent_run_id, root_run_id, parent_node, \
     cancel_policy, depth, trace_parent_span, rerun_of, clock_timestamp() AS db_now \
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
    breaker: BreakerPolicy,
    /// Pins selected under the retention-policy fence before a lease is granted. `persist` adds
    /// them to the matching immutable `step.leased` detail in this transaction.
    script_retention: BTreeMap<NodeId, ScriptRetentionPins>,
    /// Counted when (and only when) the transaction commits.
    signals: Vec<Signal>,
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
                    NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?,
                    col::<Vec<u8>>(row, "content")?,
                );
            }
            self.data = Some(RunData {
                run_id: Some(self.info.run_id),
                input: col(&input_row, "content")?,
                input_blob: col::<Option<Uuid>>(&input_row, "input_blob_id")?
                    .map(sagat_domain::blobs::BlobId::new),
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
        // FOR SHARE: a takeover of the shard (an UPDATE of this row) waits for this transaction, and a
        // transaction that starts after the takeover finds the new generation and no row.
        let held: Option<i16> = sqlx::query_scalar(
            "SELECT shard FROM scheduler_shards \
             WHERE shard = $1 AND generation = $2 AND owner = $3 AND expires_at > clock_timestamp() \
             FOR SHARE",
        )
        .bind(i16::from(lease.shard.index()))
        .bind(i64::try_from(lease.generation).unwrap_or(i64::MAX))
        .bind(owner)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(held.is_some())
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
        // This row lock serialises claims for the session across runs and replicas. Replacement
        // and binding revocation update or lock this same row, so stale authority cannot lease
        // after either operation commits. It is taken after the drain fence and before activity
        // policy locks to preserve a single lock order.
        let max_inflight: Option<i32> = sqlx::query_scalar(
            "SELECT max_inflight FROM worker_sessions \
             WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 AND token_hash = $4 \
               AND subject = $5 AND expires_at > clock_timestamp() FOR UPDATE",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(session_id)
        .bind(token_hash)
        .bind(subject)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(store_error)?;
        let Some(max_inflight) = max_inflight else {
            return Ok(0);
        };
        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM steps WHERE tenant_id = $1 \
             AND status IN ('LEASED', 'RUNNING') AND leased_by = $2",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(session_id.to_string())
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)?;
        let limit = u32::try_from(max_inflight).map_err(|_| corrupt("max inflight"))?;
        Ok(limit.saturating_sub(u32::try_from(active).unwrap_or(u32::MAX)))
    }

    async fn bound_patterns(&mut self, subject: &str) -> Result<Vec<String>, StoreError> {
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

    async fn reported_by(&mut self, node: &NodeId, token_hash: &[u8]) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM steps WHERE tenant_id = $1 AND run_id = $2 AND node = $3 \
                            AND status NOT IN ('LEASED', 'RUNNING') \
                            AND reported_lease_token_hash = $4)",
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
                RunEvent::ChildFinished { child, .. } => self.settle_child(*child, false).await?,
                RunEvent::StepCancelled { node } => self.release_child(node, true).await?,
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
            touched.insert(renewal.node.clone());
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

    async fn commit(self) -> Result<(), StoreError> {
        self.tx.commit().await.map_err(store_error)?;
        for signal in self.signals {
            signal.emit();
        }
        Ok(())
    }
}

impl PgRunTx {
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
        let activity = activity.as_str().to_owned();
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

    async fn upsert_step(
        &mut self,
        node: &NodeId,
        effects: &Effects,
        hinted: bool,
        rehinted: bool,
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
            Some(WaitingOn::Saga) => (Some("SAGA"), None, None),
            Some(WaitingOn::Reconciliation) => (Some("RECONCILIATION"), None, None),
            Some(WaitingOn::Operator { .. }) => (Some("OPERATOR"), None, None),
        };
        // How far back an event wait accepts events, beyond the moment it began waiting.
        let lookback = match def_node.map(sagat_domain::workflow::Node::kind) {
            Some(NodeKind::WaitEvent { lookback, .. }) => micros(*lookback),
            _ => 0,
        };
        let kind = def_node.map_or("FORWARD", |n| n.origin().label());
        // A `child` step reserved in this transaction: the run id, and the input it starts with.
        let spawn = effects.children.iter().find(|c| &c.node == node);
        let child_run = match step.waiting_on() {
            Some(WaitingOn::Child { run }) => Some(run.as_uuid()),
            _ => spawn.map(|c| c.child.as_uuid()),
        };
        let child_input = spawn.map(|c| c.input.clone());
        let grant = effects.lease.as_ref().filter(|g| &g.node == node);
        let renewal = effects.renewal.as_ref().filter(|r| &r.node == node);
        let reported = effects.reported.as_ref().filter(|r| &r.node == node);
        let leased = matches!(step.status(), StepStatus::Leased | StepStatus::Running);
        let fresh_grant = grant.filter(|_| leased);
        let step_uuid = step_id(self.info.version_id, self.info.run_id, node);
        let status = step.status().as_str();
        let attempt = i32::try_from(step.attempt()).map_err(|_| corrupt("attempt"))?;
        let generation = i64::try_from(step.generation()).map_err(|_| corrupt("generation"))?;
        let inserted = !self.stored.contains(node);
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
                  event_since, secrets) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
                         $18, $19, CASE WHEN $6 = 'READY' THEN $20 END, CASE WHEN $21 THEN $20 END, \
                         $22, $23, $24, $20, $25, \
                         CASE WHEN $15 = 'EVENT' THEN $20 - $26 * interval '1 microsecond' END, $27)",
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
            .bind(kind)
            .bind(lookback)
            .bind(declared)
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
                event_since = CASE WHEN $15::text IS DISTINCT FROM 'EVENT' THEN NULL \
                                   ELSE COALESCE(CASE WHEN waiting_kind = 'EVENT' THEN event_since END, \
                                                 $20 - $31 * interval '1 microsecond') END, \
                event_scanned_at = CASE WHEN $15 = 'EVENT' AND waiting_kind = 'EVENT' \
                                        THEN event_scanned_at END, \
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
        .bind(rehinted)
        .bind(lookback)
        .bind(reported.map(|r| r.token_hash.clone()))
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

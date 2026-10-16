//! The telemetry projection's store (FR-008-001, FR-008-002).
//!
//! The write path leaves a note per run in `telemetry_pending` (see `execution.rs`); this module
//! reads the notes, loads what the application needs to derive spans from a run's journal, and
//! records how far the projection has got. Nothing here takes a lock on a run: the journal is
//! append-only and read at one point in time, so the projector never makes a transition wait.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::telemetry::{
    Backlog, CONTEXT_EVENTS, JournalEntry, Pending, RunFacts, RunTelemetry, TelemetryStore,
    ToProject,
};
use sagat_domain::execution::{ExecutionInstanceId, InstancePath, RunId};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::NodeId;
use sagat_telemetry::{SpanId, TraceId};
use sqlx::Row;
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::event_codec::decode;
use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, store_error};

/// The name of the projector's lease.
pub const PROJECTOR_LEASE: &str = "telemetry-projector";

/// How many new journal entries one run gives the projector at a time.
const BATCH_ENTRIES: i64 = 5000;

/// Who holds a lease in this process: stable for its life, different from every other process's.
fn holder() -> &'static str {
    static HOLDER: OnceLock<String> = OnceLock::new();
    HOLDER.get_or_init(|| Uuid::new_v4().to_string())
}

fn seq(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| corrupt("journal sequence"))
}

fn pending_from_row(row: &PgRow) -> Result<Pending, StoreError> {
    Ok(Pending {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        run_id: RunId::new(col::<Uuid>(row, "run_id")?),
        head_seq: seq(col(row, "head_seq")?)?,
        applied_seq: seq(col(row, "applied_seq")?)?,
        behind_since: col(row, "behind_since")?,
    })
}

impl PgRunStore {
    /// Takes, or renews, the lease `name` for this process for `ttl`. `false` when another process
    /// holds a lease that has not run out. The expiry is the database's clock, so processes with
    /// skewed clocks agree.
    ///
    /// # Errors
    /// [`StoreError`] if the database cannot be reached.
    pub async fn acquire_lease(&self, name: &str, ttl: Duration) -> Result<bool, StoreError> {
        let taken = sqlx::query(
            "INSERT INTO engine_leases (name, holder, expires_at) \
             VALUES ($1, $2, clock_timestamp() + make_interval(secs => $3)) \
             ON CONFLICT (name) DO UPDATE \
               SET holder = EXCLUDED.holder, expires_at = EXCLUDED.expires_at \
               WHERE engine_leases.holder = EXCLUDED.holder \
                  OR engine_leases.expires_at < clock_timestamp() \
             RETURNING holder",
        )
        .bind(name)
        .bind(holder())
        .bind(ttl.as_secs_f64())
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        Ok(taken.is_some())
    }

    /// Lets go of the lease `name` if this process holds it, so another can take it at once.
    ///
    /// # Errors
    /// [`StoreError`] if the database cannot be reached.
    pub async fn release_lease(&self, name: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM engine_leases WHERE name = $1 AND holder = $2")
            .bind(name)
            .bind(holder())
            .execute(self.pool())
            .await
            .map_err(store_error)?;
        Ok(())
    }

    /// Removes the notes of runs that finished and were projected more than `retention` ago.
    /// Returns how many.
    ///
    /// # Errors
    /// [`StoreError`] if the database cannot be reached.
    pub async fn purge_projected(
        &self,
        retention: Duration,
        limit: u32,
    ) -> Result<u64, StoreError> {
        let removed = sqlx::query(
            "DELETE FROM telemetry_pending WHERE (tenant_id, run_id) IN ( \
                 SELECT p.tenant_id, p.run_id FROM telemetry_pending p \
                 JOIN runs r ON r.tenant_id = p.tenant_id AND r.id = p.run_id \
                 WHERE p.applied_seq >= p.head_seq AND r.finished_at IS NOT NULL \
                   AND r.finished_at < clock_timestamp() - make_interval(secs => $1) \
                 LIMIT $2)",
        )
        .bind(retention.as_secs_f64())
        .bind(i64::from(limit))
        .execute(self.pool())
        .await
        .map_err(store_error)?;
        Ok(removed.rows_affected())
    }
}

impl TelemetryStore for PgRunStore {
    async fn acquire_projector(&self) -> Result<bool, StoreError> {
        self.acquire_lease(PROJECTOR_LEASE, Duration::from_secs(30))
            .await
    }

    async fn pending_runs(&self, limit: u32) -> Result<Vec<Pending>, StoreError> {
        sqlx::query(
            "SELECT tenant_id, run_id, head_seq, applied_seq, behind_since FROM telemetry_pending \
             WHERE applied_seq < head_seq ORDER BY behind_since LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(pending_from_row)
        .collect()
    }

    async fn load_for_projection(
        &self,
        pending: &Pending,
    ) -> Result<Option<ToProject>, StoreError> {
        let tenant = pending.tenant_id.as_uuid();
        let run = pending.run_id.as_uuid();
        let Some(row) = sqlx::query(
            "SELECT r.namespace_id, r.trace_id, r.trace_parent_span, r.parent_run_id, r.created_at, \
                    w.name AS workflow, v.version_number \
             FROM runs r \
             JOIN workflows w ON w.tenant_id = r.tenant_id AND w.id = r.workflow_id \
             JOIN workflow_versions v ON v.tenant_id = r.tenant_id AND v.id = r.workflow_version_id \
             WHERE r.tenant_id = $1 AND r.id = $2",
        )
        .bind(tenant)
        .bind(run)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        // A run created before runs were given a trace has none to belong to.
        let Some(trace) = col::<Option<String>>(&row, "trace_id")?
            .as_deref()
            .and_then(TraceId::parse)
        else {
            return Ok(None);
        };
        let parent_span = col::<Option<String>>(&row, "trace_parent_span")?
            .as_deref()
            .and_then(SpanId::parse);
        let steps: HashMap<ExecutionInstanceId, Uuid> = sqlx::query(
            "SELECT node, step_id, instance_path_nodes, instance_path_indices \
             FROM steps WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(tenant)
        .bind(run)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|r| {
            let node = NodeId::parse_stored(&col::<String>(r, "node")?)
                .map_err(|_| corrupt("step node"))?;
            Ok((
                ExecutionInstanceId::in_path(instance_path(r)?, node),
                col::<Uuid>(r, "step_id")?,
            ))
        })
        .collect::<Result<_, StoreError>>()?;

        let applied = i64::try_from(pending.applied_seq).map_err(|_| corrupt("sequence"))?;
        let (entries, last_seen) =
            journal_rows(self, tenant, run, "seq > $3", applied, Some(BATCH_ENTRIES)).await?;
        let head = last_seen.unwrap_or(pending.applied_seq);
        let finishes = entries.iter().any(|e| {
            matches!(
                e.event,
                sagat_domain::execution::RunEvent::RunFinished { .. }
            )
        });
        // What the new entries need from the part already projected: the leases they close and,
        // when the run is over, the whole journal for the run's own span.
        let context = if finishes {
            journal_rows(self, tenant, run, "seq <= $3", applied, None)
                .await?
                .0
        } else {
            let types: Vec<String> = CONTEXT_EVENTS.iter().map(|t| (*t).to_owned()).collect();
            sqlx::query(JOURNAL_CONTEXT)
                .bind(tenant)
                .bind(run)
                .bind(applied)
                .bind(&types)
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .iter()
                .filter_map(|r| entry_from_row(r).transpose())
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut all = context;
        all.extend(entries);
        all.sort_by_key(|e| e.seq);

        let mut facts = RunFacts {
            tenant_id: pending.tenant_id,
            namespace_id: NamespaceId::new(col::<Uuid>(&row, "namespace_id")?),
            run_id: pending.run_id,
            workflow: col(&row, "workflow")?,
            version: u32::try_from(col::<i32>(&row, "version_number")?)
                .map_err(|_| corrupt("version"))?,
            trace_id: trace,
            parent_span,
            parent_run: col::<Option<Uuid>>(&row, "parent_run_id")?.map(RunId::new),
            created_at: col(&row, "created_at")?,
            steps,
        };
        // A child hangs from its parent's span, not from the request that started the parent.
        if facts.parent_run.is_some() {
            facts.parent_span = None;
        }
        Ok(Some(ToProject {
            facts,
            entries: all,
            head_seq: head,
            applied_seq: pending.applied_seq,
        }))
    }

    async fn mark_projected(&self, pending: &Pending, upto: u64) -> Result<bool, StoreError> {
        // What is left after `upto` is behind since its first entry was written, which is not when
        // the run first fell behind: a run that is written to all the time would otherwise never
        // look current.
        let moved = sqlx::query(
            "UPDATE telemetry_pending \
             SET applied_seq = $3, \
                 behind_since = CASE WHEN head_seq > $3 THEN coalesce( \
                     (SELECT occurred_at FROM run_events \
                      WHERE tenant_id = $1 AND run_id = $2 AND seq = $3 + 1), behind_since) \
                 ELSE NULL END \
             WHERE tenant_id = $1 AND run_id = $2 AND applied_seq < $3",
        )
        .bind(pending.tenant_id.as_uuid())
        .bind(pending.run_id.as_uuid())
        .bind(i64::try_from(upto).map_err(|_| corrupt("sequence"))?)
        .execute(self.pool())
        .await
        .map_err(store_error)?;
        Ok(moved.rows_affected() > 0)
    }

    async fn run_telemetry(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<RunTelemetry>, StoreError> {
        let Some(row) = sqlx::query(
            "SELECT r.trace_id IS NOT NULL AS traced, r.finished_at IS NOT NULL AS finished, \
                    p.head_seq, p.applied_seq, \
                    coalesce(extract(epoch FROM clock_timestamp() - p.behind_since), 0)::float8 AS behind \
             FROM runs r LEFT JOIN telemetry_pending p ON p.tenant_id = r.tenant_id AND p.run_id = r.id \
             WHERE r.tenant_id = $1 AND r.id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let optional = |name: &str| -> Result<Option<u64>, StoreError> {
            col::<Option<i64>>(&row, name)?.map(seq).transpose()
        };
        Ok(Some(RunTelemetry {
            traced: col(&row, "traced")?,
            finished: col(&row, "finished")?,
            head_seq: optional("head_seq")?,
            applied_seq: optional("applied_seq")?,
            behind_seconds: col::<f64>(&row, "behind")?.max(0.0),
        }))
    }

    async fn backlog(&self) -> Result<Backlog, StoreError> {
        let row = sqlx::query(
            "SELECT count(*) AS runs, \
                    coalesce(extract(epoch FROM clock_timestamp() - min(behind_since)), 0)::float8 AS oldest \
             FROM telemetry_pending WHERE applied_seq < head_seq",
        )
        .fetch_one(self.pool())
        .await
        .map_err(store_error)?;
        Ok(Backlog {
            runs: u64::try_from(row.try_get::<i64, _>("runs").map_err(store_error)?).unwrap_or(0),
            oldest_seconds: row
                .try_get::<f64, _>("oldest")
                .map_err(store_error)?
                .max(0.0),
        })
    }
}

const JOURNAL_CONTEXT: &str = "SELECT seq, event_type, node, detail, actor, occurred_at, \
     instance_path_nodes, instance_path_indices FROM run_events \
     WHERE tenant_id = $1 AND run_id = $2 AND seq <= $3 AND event_type = ANY($4) ORDER BY seq";

async fn journal_rows(
    store: &PgRunStore,
    tenant: Uuid,
    run: Uuid,
    condition: &str,
    applied: i64,
    limit: Option<i64>,
) -> Result<(Vec<JournalEntry>, Option<u64>), StoreError> {
    let sql = format!(
        "SELECT seq, event_type, node, detail, actor, occurred_at, \
         instance_path_nodes, instance_path_indices FROM run_events \
         WHERE tenant_id = $1 AND run_id = $2 AND {condition} ORDER BY seq LIMIT $4"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(tenant)
        .bind(run)
        .bind(applied)
        .bind(limit.unwrap_or(i64::MAX))
        .fetch_all(store.pool())
        .await
        .map_err(store_error)?;
    let last_seen = rows.last().map(|row| seq(col(row, "seq")?)).transpose()?;
    let entries = rows
        .iter()
        .filter_map(|r| entry_from_row(r).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    Ok((entries, last_seen))
}

/// One journal row as an entry; `None` for an event this version does not know (a journal written
/// by a newer one), which is skipped rather than allowed to stop the projection of the run.
fn entry_from_row(row: &PgRow) -> Result<Option<JournalEntry>, StoreError> {
    let event_type: String = col(row, "event_type")?;
    let node: Option<String> = col(row, "node")?;
    let detail: serde_json::Value = col(row, "detail")?;
    let Some(event) = decode(&event_type, node.as_deref(), &detail) else {
        metrics::counter!("telemetry_projection_skipped_total", "reason" => "unknown_event")
            .increment(1);
        return Ok(None);
    };
    let occurred_at: OffsetDateTime = col(row, "occurred_at")?;
    Ok(Some(JournalEntry {
        seq: seq(col(row, "seq")?)?,
        event,
        instance_path: instance_path(row)?,
        actor: col(row, "actor")?,
        occurred_at,
    }))
}

fn instance_path(row: &PgRow) -> Result<InstancePath, StoreError> {
    let nodes: Option<Vec<String>> = col(row, "instance_path_nodes")?;
    let indices: Option<Vec<i32>> = col(row, "instance_path_indices")?;
    let (nodes, indices) = match (nodes, indices) {
        (None, None) => return Ok(InstancePath::root()),
        (Some(nodes), Some(indices)) if nodes.len() == indices.len() => (nodes, indices),
        _ => return Err(corrupt("journal instance path")),
    };
    let mut path = InstancePath::root();
    for (node, index) in nodes.into_iter().zip(indices) {
        let node = NodeId::parse_stored(&node).map_err(|_| corrupt("scope node"))?;
        let index = u32::try_from(index).map_err(|_| corrupt("scope index"))?;
        path = path
            .child(node, index)
            .map_err(|_| corrupt("journal instance path"))?;
    }
    Ok(path)
}

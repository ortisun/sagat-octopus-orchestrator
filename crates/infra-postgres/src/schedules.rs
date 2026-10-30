//! The schedule store (feature 018): schedules, their immutable revisions, the actions taken on
//! their nominal times, backfills and the owed cancellations of `CANCEL_OTHER`.
//!
//! This file owns the columns of those tables in the storage classification. So far it creates and
//! reads schedules; the changes (update, pause, resume, delete) and the pass's statements arrive
//! with their tasks (T018-014..).

use sagat_application::error::{ConflictKind, StoreError};
use sagat_application::quotas::QuotaLookup;
use sagat_application::schedule_codec::{
    policies_from_json, policies_to_json, spec_from_json, spec_to_json, target_from_json,
    target_to_json,
};
use sagat_application::schedules::{
    ActionDedupe, ActionView, BufferedAction, DueSchedule, NewSchedule, Reconcile, RecordedAction,
    Revise, ScheduleActivity, ScheduleAdmission, ScheduleDecision, ScheduleOrigin, ScheduleRecord,
    ScheduleState, ScheduleStore, ShardFence, StateChange, TargetCheck,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::execution::{RunId, RunStatus};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::schedules::{
    ActionKind, Ceilings, Outcome, ScheduleId, ScheduleName, ScheduleTarget, VersionPolicy,
};
use sagat_domain::sharding::Shard;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::PgConnection;
use sqlx::PgPool;
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;
use crate::quotas::{effective, ensure_room_for_schedule, lock_schedules};

/// What the pass holds when it acts for a schedule, checked in the transaction that acts: the shard
/// lease, then the schedule row under `FOR UPDATE` with the compare-and-swap on the state the
/// decision was made on (ADR-0050 decision 7 puts this before every other admission lock). A
/// refusal is a [`StoreError::StateConflict`] and commits nothing.
async fn lock_schedule(
    conn: &mut PgConnection,
    tenant: TenantId,
    schedule: ScheduleId,
    fence: Option<&ShardFence>,
    expected_state_version: u64,
    allow_paused: bool,
) -> Result<(), StoreError> {
    if let Some(fence) = fence {
        let shard = Shard::of(schedule.as_uuid());
        let generation = i64::try_from(fence.lease.generation).ok();
        let (true, Some(generation)) = (fence.lease.shard == shard, generation) else {
            return Err(StoreError::StateConflict(
                "the shard lease is not the one of this schedule",
            ));
        };
        let held: Option<i16> = sqlx::query_scalar(
            "SELECT shard FROM scheduler_shards \
             WHERE shard = $1 AND generation = $2 AND owner = $3 FOR SHARE",
        )
        .bind(i16::from(shard.index()))
        .bind(generation)
        .bind(&fence.owner)
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?;
        // expiry is observed only after the lock: a volatile predicate could run before a lock wait
        let current: Option<Option<OffsetDateTime>> = if held.is_some() {
            sqlx::query_scalar(
                "WITH tick AS MATERIALIZED (SELECT clock_timestamp() AS now) \
                 SELECT CASE WHEN s.expires_at > tick.now THEN tick.now END \
                 FROM scheduler_shards s CROSS JOIN tick \
                 WHERE s.shard = $1 AND s.generation = $2 AND s.owner = $3",
            )
            .bind(i16::from(shard.index()))
            .bind(generation)
            .bind(&fence.owner)
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?
        } else {
            None
        };
        if current.flatten().is_none() {
            return Err(StoreError::StateConflict(
                "the shard lease is no longer held",
            ));
        }
    }
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT state, state_version FROM schedules WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(schedule.as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let Some((state, version)) = row else {
        return Err(StoreError::MissingReference);
    };
    let acts = state == ScheduleState::Active.as_str()
        || (allow_paused && state == ScheduleState::Paused.as_str());
    if !acts {
        return Err(StoreError::StateConflict("the schedule is not active"));
    }
    if u64::try_from(version).ok() != Some(expected_state_version) {
        return Err(StoreError::StateConflict(
            "the schedule changed since the decision was made",
        ));
    }
    Ok(())
}

/// The first half of admitting the run of a schedule's action: see [`lock_schedule`].
pub(crate) async fn lock_for_admission(
    conn: &mut PgConnection,
    tenant: TenantId,
    admission: &ScheduleAdmission,
) -> Result<(), StoreError> {
    lock_schedule(
        conn,
        tenant,
        admission.schedule,
        admission.fence.as_ref(),
        admission.expected_state_version,
        // trigger-now works while the schedule is paused
        admission.kind == ActionKind::Triggered,
    )
    .await
}

fn conflict_on_duplicate(error: sqlx::Error) -> StoreError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            StoreError::StateConflict("the schedule already acted on that time")
        }
        _ => store_error(error),
    }
}

/// Inserts an action row that has no run.
async fn insert_recorded(
    conn: &mut PgConnection,
    tenant: TenantId,
    schedule: ScheduleId,
    action: &RecordedAction,
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO schedule_actions \
         (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, reason, decided_at, \
          summary_count, summary_last_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(tenant.as_uuid())
    .bind(schedule.as_uuid())
    .bind(action.kind.origin().as_str())
    .bind(action.nominal_at)
    .bind(action.kind.as_str())
    .bind(i32::try_from(action.revision).unwrap_or(i32::MAX))
    .bind(action.outcome.as_str())
    .bind(action.reason)
    .bind(now)
    .bind(
        action
            .summary
            .map(|(n, _)| i32::try_from(n).unwrap_or(i32::MAX)),
    )
    .bind(action.summary.map(|(_, last)| last))
    .execute(conn)
    .await
    .map_err(conflict_on_duplicate)?;
    Ok(())
}

/// The counters of a schedule after rows were recorded: one per outcome, a summary counting the
/// times it stands for.
fn bumped(counters: &mut Value, outcome: Outcome, by: u64) {
    if let Value::Object(map) = counters {
        let key = outcome.as_str().to_owned();
        let now = map
            .get(&key)
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .saturating_add(by);
        map.insert(key, Value::from(now));
    }
}

/// The second half, once the run row exists: the action is recorded with the run (a waiting action
/// becomes the started one), the summaries ride along, and the schedule moves to the state the
/// decision chose. A second action for the same nominal time is refused by its identity.
pub(crate) async fn record_start(
    conn: &mut PgConnection,
    tenant: TenantId,
    run: RunId,
    admission: &ScheduleAdmission,
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    if let Some(manual) = &admission.manual {
        remember_request(conn, tenant, &manual.dedupe).await?;
        insert_audit(conn, &manual.audit).await?;
    }
    let released = sqlx::query(
        "UPDATE schedule_actions SET outcome = 'STARTED', run_id = $5, actual_at = $6, decided_at = $6 \
         WHERE tenant_id = $1 AND schedule_id = $2 AND origin = $3 AND nominal_at = $4 \
           AND outcome = 'BUFFERED'",
    )
    .bind(tenant.as_uuid())
    .bind(admission.schedule.as_uuid())
    .bind(admission.kind.origin().as_str())
    .bind(admission.nominal_at)
    .bind(run.as_uuid())
    .bind(now)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?
    .rows_affected()
        == 1;
    if !released {
        sqlx::query(
            "INSERT INTO schedule_actions \
             (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, actual_at, decided_at, run_id) \
             VALUES ($1, $2, $3, $4, $5, $6, 'STARTED', $7, $7, $8)",
        )
        .bind(tenant.as_uuid())
        .bind(admission.schedule.as_uuid())
        .bind(admission.kind.origin().as_str())
        .bind(admission.nominal_at)
        .bind(admission.kind.as_str())
        .bind(i32::try_from(admission.revision).unwrap_or(i32::MAX))
        .bind(now)
        .bind(run.as_uuid())
        .execute(&mut *conn)
        .await
        .map_err(conflict_on_duplicate)?;
    }
    // the row is locked: its counters can be read, changed here and written back
    let mut counters: Value =
        sqlx::query_scalar("SELECT counters FROM schedules WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.as_uuid())
            .bind(admission.schedule.as_uuid())
            .fetch_one(&mut *conn)
            .await
            .map_err(store_error)?;
    bumped(&mut counters, Outcome::Started, 1);
    for action in &admission.after.extra {
        insert_recorded(conn, tenant, admission.schedule, action, now).await?;
        bumped(
            &mut counters,
            action.outcome,
            action.summary.map_or(1, |(n, _)| u64::from(n)),
        );
    }
    sqlx::query(
        "UPDATE schedules SET state_version = state_version + 1, next_action_at = $3, \
         next_actual_at = $4, retry_at = CASE WHEN $10 THEN retry_at ELSE NULL END, \
         remaining_actions = $5, tracked_runs = array_append(tracked_runs, $6), \
         buffered = GREATEST(buffered - $8, 0), counters = $9, updated_at = $7, \
         recheck_at = CASE WHEN GREATEST(buffered - $8, 0) > 0 THEN $7 + interval '5 seconds' ELSE NULL END \
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(admission.schedule.as_uuid())
    .bind(admission.after.next_action_at)
    .bind(admission.after.next_actual_at)
    .bind(
        admission
            .after
            .remaining_actions
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX)),
    )
    .bind(run.as_uuid())
    .bind(now)
    .bind(i32::from(released))
    .bind(counters)
    // a manual action is not the time that is owed
    .bind(admission.kind == ActionKind::Triggered)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

/// Records the key of a manual request in the transaction that makes its action. `StateConflict`
/// when the key is already recorded: a request with the same key committed first, and the caller
/// starts over, which turns this one into a replay.
async fn remember_request(
    conn: &mut PgConnection,
    tenant: TenantId,
    dedupe: &ActionDedupe,
) -> Result<(), StoreError> {
    let hash = crate::definitions::hash(&dedupe.idempotency.fingerprint);
    if crate::definitions::remember(conn, tenant, &dedupe.idempotency, &hash, &dedupe.resource)
        .await?
    {
        Ok(())
    } else {
        Err(StoreError::StateConflict(
            "a request with this key was already applied",
        ))
    }
}

/// An action row as `ActionView`.
fn action_view(row: &PgRow) -> Result<ActionView, StoreError> {
    let kind: ActionKind = col::<String>(row, "kind")?
        .parse()
        .map_err(|()| corrupt("action kind"))?;
    let count: Option<i32> = col(row, "summary_count")?;
    Ok(ActionView {
        nominal_at: col(row, "nominal_at")?,
        actual_at: col(row, "actual_at")?,
        kind,
        origin: kind.origin(),
        revision: u32::try_from(col::<i32>(row, "revision")?)
            .map_err(|_| corrupt("action revision"))?,
        outcome: col(row, "outcome")?,
        reason: col(row, "reason")?,
        run_id: col(row, "run_id")?,
        backfill_id: col(row, "backfill_id")?,
        summary_count: count
            .map(|n| u32::try_from(n).map_err(|_| corrupt("summary count")))
            .transpose()?,
        summary_last_nominal_at: col(row, "summary_last_at")?,
        decided_at: col(row, "decided_at")?,
    })
}

/// The schedule store over PostgreSQL.
#[derive(Clone)]
pub struct PgScheduleStore {
    pool: PgPool,
}

impl PgScheduleStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

macro_rules! select_schedules {
    ($tail:literal) => {
        concat!(
            "SELECT s.tenant_id, s.namespace_id, s.id, s.name, s.origin, s.trigger_id, s.state, ",
            "s.head_revision, s.state_version, s.note, s.note_by, s.note_at, s.authorized_by, ",
            "s.remaining_actions, s.next_action_at, s.next_actual_at, s.retry_at, s.recheck_at, s.tracked_runs, ",
            "s.buffered, s.created_at, s.updated_at, r.spec, r.policies, r.action, r.input_digest ",
            "FROM schedules s ",
            "JOIN schedule_revisions r ON r.tenant_id = s.tenant_id AND r.schedule_id = s.id ",
            "AND r.revision = s.head_revision ",
            $tail
        )
    };
}

fn record_from(row: &PgRow) -> Result<ScheduleRecord, StoreError> {
    // What was stored is read back under the hard ceilings: a namespace that lowered its own
    // ceiling afterwards does not make an existing schedule unreadable.
    let ceilings = Ceilings::HARD;
    let name: String = col(row, "name")?;
    let origin: String = col(row, "origin")?;
    let state: String = col(row, "state")?;
    let spec: Value = col(row, "spec")?;
    let policies: Value = col(row, "policies")?;
    let action: Value = col(row, "action")?;
    let digest: Option<Vec<u8>> = col(row, "input_digest")?;
    let revision: i32 = col(row, "head_revision")?;
    let state_version: i64 = col(row, "state_version")?;
    let remaining: Option<i32> = col(row, "remaining_actions")?;
    let buffered: i32 = col(row, "buffered")?;
    Ok(ScheduleRecord {
        id: ScheduleId::new(col::<Uuid>(row, "id")?),
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        name: ScheduleName::new(&name).map_err(|_| corrupt("schedule name"))?,
        origin: origin
            .parse::<ScheduleOrigin>()
            .map_err(|()| corrupt("schedule origin"))?,
        trigger_id: col(row, "trigger_id")?,
        state: state
            .parse::<ScheduleState>()
            .map_err(|()| corrupt("schedule state"))?,
        revision: u32::try_from(revision).map_err(|_| corrupt("schedule revision"))?,
        state_version: u64::try_from(state_version)
            .map_err(|_| corrupt("schedule state version"))?,
        spec: spec_from_json(&spec, &ceilings).map_err(|_| corrupt("schedule spec"))?,
        policies: policies_from_json(&policies, &ceilings)
            .map_err(|_| corrupt("schedule policies"))?,
        target: target_from_json(&action).map_err(|_| corrupt("schedule action"))?,
        input_digest: match digest {
            None => None,
            Some(bytes) => Some(
                <[u8; 32]>::try_from(bytes.as_slice())
                    .map_err(|_| corrupt("schedule input digest"))?,
            ),
        },
        note: col(row, "note")?,
        note_by: col(row, "note_by")?,
        note_at: col::<Option<OffsetDateTime>>(row, "note_at")?,
        authorized_by: col(row, "authorized_by")?,
        remaining_actions: match remaining {
            None => None,
            Some(n) => Some(u32::try_from(n).map_err(|_| corrupt("remaining actions"))?),
        },
        next_action_at: col(row, "next_action_at")?,
        next_actual_at: col(row, "next_actual_at")?,
        retry_at: col(row, "retry_at")?,
        recheck_at: col(row, "recheck_at")?,
        tracked_runs: col(row, "tracked_runs")?,
        buffered: u32::try_from(buffered).map_err(|_| corrupt("buffered count"))?,
        created_at: col(row, "created_at")?,
        updated_at: col(row, "updated_at")?,
    })
}

impl QuotaLookup for PgScheduleStore {
    async fn effective_limit(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> Result<u64, StoreError> {
        effective(&self.pool, tenant, namespace, resource).await
    }
}

impl ScheduleStore for PgScheduleStore {
    async fn namespace_exists(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM namespaces WHERE tenant_id = $1 AND id = $2)",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn target_check(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        target: &ScheduleTarget,
    ) -> Result<TargetCheck, StoreError> {
        let workflow: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM workflows WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3)",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(target.workflow().as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)?;
        if !workflow {
            return Ok(TargetCheck::WorkflowMissing);
        }
        if let VersionPolicy::Pinned(version) = target.version() {
            let found: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM workflow_versions \
                 WHERE tenant_id = $1 AND workflow_id = $2 AND id = $3)",
            )
            .bind(tenant.as_uuid())
            .bind(target.workflow().as_uuid())
            .bind(version.as_uuid())
            .fetch_one(&self.pool)
            .await
            .map_err(store_error)?;
            if !found {
                return Ok(TargetCheck::VersionMissing);
            }
        }
        Ok(TargetCheck::Found)
    }

    async fn create_schedule(
        &self,
        new: &NewSchedule,
        limit: u64,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let r = &new.record;
        if let Some(input) = &new.input {
            // payloads are content-addressed: the bytes must be the ones the digest names
            if Sha256::digest(&input.bytes).as_slice() != input.digest.as_slice()
                || r.input_digest != Some(input.digest)
            {
                return Err(StoreError::StateConflict(
                    "the input of a schedule must match its digest",
                ));
            }
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_schedules(&mut tx, r.tenant_id, r.namespace_id).await?;
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM schedules \
             WHERE tenant_id = $1 AND namespace_id = $2 AND name = $3 AND state <> 'DELETED')",
        )
        .bind(r.tenant_id.as_uuid())
        .bind(r.namespace_id.as_uuid())
        .bind(r.name.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if taken {
            return Err(StoreError::Conflict(ConflictKind::ScheduleName));
        }
        ensure_room_for_schedule(&mut tx, r.tenant_id, r.namespace_id, limit).await?;
        if let Some(input) = &new.input {
            sqlx::query(
                "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
                 ON CONFLICT (tenant_id,digest) DO NOTHING",
            )
            .bind(r.tenant_id.as_uuid())
            .bind(input.digest.as_slice())
            .bind(input.bytes.as_slice())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        sqlx::query(
            "INSERT INTO schedules \
             (tenant_id, namespace_id, id, name, origin, trigger_id, state, head_revision, state_version, \
              note, note_by, note_at, authorized_by, remaining_actions, next_action_at, next_actual_at, \
              retry_at, tracked_runs, buffered, created_at, updated_at, recheck_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22)",
        )
        .bind(r.tenant_id.as_uuid())
        .bind(r.namespace_id.as_uuid())
        .bind(r.id.as_uuid())
        .bind(r.name.as_str())
        .bind(r.origin.as_str())
        .bind(r.trigger_id)
        .bind(r.state.as_str())
        .bind(i32::try_from(r.revision).unwrap_or(i32::MAX))
        .bind(i64::try_from(r.state_version).unwrap_or(i64::MAX))
        .bind(r.note.as_deref())
        .bind(r.note_by.as_deref())
        .bind(r.note_at)
        .bind(&r.authorized_by)
        .bind(r.remaining_actions.map(|n| i32::try_from(n).unwrap_or(i32::MAX)))
        .bind(r.next_action_at)
        .bind(r.next_actual_at)
        .bind(r.retry_at)
        .bind(&r.tracked_runs)
        .bind(i32::try_from(r.buffered).unwrap_or(i32::MAX))
        .bind(r.created_at)
        .bind(r.updated_at)
        .bind(r.recheck_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO schedule_revisions \
             (tenant_id, schedule_id, revision, spec, policies, action, input_digest, actor, request_id, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(r.tenant_id.as_uuid())
        .bind(r.id.as_uuid())
        .bind(i32::try_from(r.revision).unwrap_or(i32::MAX))
        .bind(spec_to_json(&r.spec))
        .bind(policies_to_json(&r.policies))
        .bind(target_to_json(&r.target))
        .bind(r.input_digest.as_ref().map(<[u8; 32]>::as_slice))
        .bind(&r.authorized_by)
        .bind(new.request_id.as_deref())
        .bind(r.created_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn schedule(
        &self,
        tenant: TenantId,
        id: ScheduleId,
    ) -> Result<Option<ScheduleRecord>, StoreError> {
        let row = sqlx::query(select_schedules!("WHERE s.tenant_id = $1 AND s.id = $2"))
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
        row.as_ref().map(record_from).transpose()
    }

    async fn schedules(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        after: Option<ScheduleId>,
        limit: u32,
    ) -> Result<Vec<ScheduleRecord>, StoreError> {
        let rows = sqlx::query(select_schedules!(
            "WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.state <> 'DELETED' \
             AND ($3::uuid IS NULL OR s.id < $3) ORDER BY s.id DESC LIMIT $4"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(after.map(ScheduleId::as_uuid))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(record_from).collect()
    }

    async fn revise_schedule(
        &self,
        revise: &Revise,
        audit: &AuditRecord,
    ) -> Result<Option<ScheduleRecord>, StoreError> {
        let r = &revise.record;
        if r.revision != revise.expected_revision.saturating_add(1)
            || !matches!(r.state, ScheduleState::Active | ScheduleState::Paused)
        {
            return Err(StoreError::StateConflict(
                "a revision follows the one the caller saw and leaves the schedule active or paused",
            ));
        }
        if let Some(input) = &revise.input
            && (Sha256::digest(&input.bytes).as_slice() != input.digest.as_slice()
                || r.input_digest != Some(input.digest))
        {
            return Err(StoreError::StateConflict(
                "the input of a schedule must match its digest",
            ));
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // the fence first (a rename claims a name of the namespace), then the row
        lock_schedules(&mut tx, r.tenant_id, r.namespace_id).await?;
        let head: Option<(i32, String)> = sqlx::query_as(
            "SELECT head_revision, state FROM schedules \
             WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 FOR UPDATE",
        )
        .bind(r.tenant_id.as_uuid())
        .bind(r.namespace_id.as_uuid())
        .bind(r.id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some((head_revision, state)) = head else {
            return Ok(None);
        };
        if state == ScheduleState::Deleted.as_str() {
            return Ok(None);
        }
        let current = u32::try_from(head_revision).map_err(|_| corrupt("schedule revision"))?;
        if current != revise.expected_revision {
            return Err(StoreError::RevisionConflict {
                current: u64::from(current),
            });
        }
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM schedules WHERE tenant_id = $1 AND namespace_id = $2 \
             AND name = $3 AND state <> 'DELETED' AND id <> $4)",
        )
        .bind(r.tenant_id.as_uuid())
        .bind(r.namespace_id.as_uuid())
        .bind(r.name.as_str())
        .bind(r.id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if taken {
            return Err(StoreError::Conflict(ConflictKind::ScheduleName));
        }
        if let Some(input) = &revise.input {
            sqlx::query(
                "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
                 ON CONFLICT (tenant_id,digest) DO NOTHING",
            )
            .bind(r.tenant_id.as_uuid())
            .bind(input.digest.as_slice())
            .bind(input.bytes.as_slice())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        sqlx::query(
            "INSERT INTO schedule_revisions \
             (tenant_id, schedule_id, revision, spec, policies, action, input_digest, actor, request_id, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(r.tenant_id.as_uuid())
        .bind(r.id.as_uuid())
        .bind(i32::try_from(r.revision).unwrap_or(i32::MAX))
        .bind(spec_to_json(&r.spec))
        .bind(policies_to_json(&r.policies))
        .bind(target_to_json(&r.target))
        .bind(r.input_digest.as_ref().map(<[u8; 32]>::as_slice))
        .bind(&r.authorized_by)
        .bind(revise.request_id.as_deref())
        .bind(r.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE schedules SET name = $3, head_revision = $4, state_version = state_version + 1, \
             state = $5, authorized_by = $6, remaining_actions = $7, next_action_at = $8, \
             next_actual_at = $9, retry_at = NULL, updated_at = $10 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(r.tenant_id.as_uuid())
        .bind(r.id.as_uuid())
        .bind(r.name.as_str())
        .bind(i32::try_from(r.revision).unwrap_or(i32::MAX))
        .bind(r.state.as_str())
        .bind(&r.authorized_by)
        .bind(r.remaining_actions.map(|n| i32::try_from(n).unwrap_or(i32::MAX)))
        .bind(r.next_action_at)
        .bind(r.next_actual_at)
        .bind(r.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        self.schedule(r.tenant_id, r.id).await
    }

    async fn set_schedule_state(
        &self,
        change: &StateChange,
        audit: &AuditRecord,
    ) -> Result<Option<ScheduleRecord>, StoreError> {
        if !matches!(change.state, ScheduleState::Active | ScheduleState::Paused) {
            return Err(StoreError::StateConflict(
                "a schedule is paused or resumed, not deleted, here",
            ));
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM schedules WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(change.tenant_id.as_uuid())
        .bind(change.id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        match state.as_deref() {
            None => return Ok(None),
            Some(s) if s == ScheduleState::Deleted.as_str() => return Ok(None),
            Some(_) => {}
        }
        sqlx::query(
            "UPDATE schedules SET state = $3, state_version = state_version + 1, \
             note = COALESCE($4, note), note_by = COALESCE($5, note_by), note_at = COALESCE($6, note_at), \
             authorized_by = COALESCE($7, authorized_by), next_action_at = $8, next_actual_at = $9, \
             retry_at = NULL, updated_at = $10 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(change.tenant_id.as_uuid())
        .bind(change.id.as_uuid())
        .bind(change.state.as_str())
        .bind(change.note.as_ref().map(|n| n.text.as_str()))
        .bind(change.note.as_ref().map(|n| n.by.as_str()))
        .bind(change.note.as_ref().map(|n| n.at))
        .bind(change.authorized_by.as_deref())
        .bind(change.next_action_at)
        .bind(change.next_actual_at)
        .bind(change.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        self.schedule(change.tenant_id, change.id).await
    }

    async fn delete_schedule(
        &self,
        tenant: TenantId,
        id: ScheduleId,
        expected_revision: Option<u32>,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let head: Option<(String, i32)> = sqlx::query_as(
            "SELECT state, head_revision FROM schedules \
             WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some((state, head_revision)) = head else {
            return Ok(false);
        };
        if state == ScheduleState::Deleted.as_str() {
            return Ok(false);
        }
        let current = u32::try_from(head_revision).map_err(|_| corrupt("schedule revision"))?;
        if expected_revision.is_some_and(|expected| expected != current) {
            return Err(StoreError::RevisionConflict {
                current: u64::from(current),
            });
        }
        // the actions that were still waiting will never start
        sqlx::query(
            "UPDATE schedule_actions SET outcome = 'DROPPED_DELETED', decided_at = $3 \
             WHERE tenant_id = $1 AND schedule_id = $2 AND outcome = 'BUFFERED'",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE schedules SET state = 'DELETED', deleted_at = $3, buffered = 0, \
             next_action_at = NULL, next_actual_at = NULL, retry_at = NULL, \
             state_version = state_version + 1, updated_at = $3 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(true)
    }

    async fn trigger_replay(
        &self,
        tenant: TenantId,
        idempotency: &sagat_application::definitions::Idempotency,
    ) -> Result<Option<ActionView>, StoreError> {
        let mut conn = self.pool.acquire().await.map_err(store_error)?;
        let hash = crate::definitions::hash(&idempotency.fingerprint);
        let Some(resource) =
            crate::definitions::replay(&mut conn, tenant, idempotency, &hash).await?
        else {
            return Ok(None);
        };
        // the resource is the identity of the action: `manual-{epoch µs}`
        let micros: i128 = resource
            .strip_prefix("manual-")
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| corrupt("manual action identity"))?;
        let nominal = OffsetDateTime::from_unix_timestamp_nanos(micros * 1_000)
            .map_err(|_| corrupt("manual action time"))?;
        let schedule = idempotency
            .scope
            .strip_prefix("schedule-trigger:")
            .and_then(|id| id.parse::<ScheduleId>().ok())
            .ok_or_else(|| corrupt("manual action scope"))?;
        let row = sqlx::query(
            "SELECT nominal_at, actual_at, kind, revision, outcome, reason, run_id, backfill_id, \
                    summary_count, summary_last_at, decided_at \
             FROM schedule_actions \
             WHERE tenant_id = $1 AND schedule_id = $2 AND origin = 'MANUAL' AND nominal_at = $3",
        )
        .bind(tenant.as_uuid())
        .bind(schedule.as_uuid())
        .bind(nominal)
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?;
        row.as_ref().map(action_view).transpose()
    }

    async fn activity(
        &self,
        tenant: TenantId,
        id: ScheduleId,
        recent: u32,
    ) -> Result<ScheduleActivity, StoreError> {
        let counters: Option<Value> =
            sqlx::query_scalar("SELECT counters FROM schedules WHERE tenant_id = $1 AND id = $2")
                .bind(tenant.as_uuid())
                .bind(id.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(store_error)?;
        let counters = match counters {
            Some(Value::Object(map)) => map
                .into_iter()
                .map(|(name, value)| {
                    value
                        .as_u64()
                        .map(|n| (name, n))
                        .ok_or_else(|| corrupt("schedule counter"))
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(corrupt("schedule counters")),
            None => return Ok(ScheduleActivity::default()),
        };
        let rows = sqlx::query(
            "SELECT nominal_at, actual_at, kind, revision, outcome, reason, run_id, backfill_id, \
                    summary_count, summary_last_at, decided_at \
             FROM schedule_actions WHERE tenant_id = $1 AND schedule_id = $2 \
             ORDER BY decided_at DESC, nominal_at DESC LIMIT $3",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .bind(i64::from(recent))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        let recent = rows
            .iter()
            .map(action_view)
            .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(ScheduleActivity { counters, recent })
    }

    async fn due_schedules(
        &self,
        shards: &[Shard],
        limit: u32,
    ) -> Result<Vec<DueSchedule>, StoreError> {
        let shards: Vec<i16> = shards.iter().map(|s| i16::from(s.index())).collect();
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT tenant_id, id FROM schedules \
             WHERE shard = ANY($1) AND state = 'ACTIVE' AND next_wake_at <= clock_timestamp() \
             ORDER BY next_wake_at, id LIMIT $2",
        )
        .bind(shards)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(rows
            .into_iter()
            .map(|(tenant, id)| DueSchedule {
                tenant_id: TenantId::new(tenant),
                id: ScheduleId::new(id),
            })
            .collect())
    }

    async fn run_statuses(
        &self,
        tenant: TenantId,
        runs: &[Uuid],
    ) -> Result<Vec<(Uuid, RunStatus)>, StoreError> {
        let rows: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id, status FROM runs WHERE tenant_id = $1 AND id = ANY($2)")
                .bind(tenant.as_uuid())
                .bind(runs)
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?;
        rows.into_iter()
            .map(|(id, status)| {
                status
                    .parse::<RunStatus>()
                    .map(|s| (id, s))
                    .map_err(|_| corrupt("run status"))
            })
            .collect()
    }

    async fn oldest_buffered(
        &self,
        tenant: TenantId,
        id: ScheduleId,
    ) -> Result<Option<BufferedAction>, StoreError> {
        let row: Option<(OffsetDateTime, String, i32)> = sqlx::query_as(
            "SELECT nominal_at, kind, revision FROM schedule_actions \
             WHERE tenant_id = $1 AND schedule_id = $2 AND outcome = 'BUFFERED' \
             ORDER BY nominal_at LIMIT 1",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        row.map(|(nominal_at, kind, revision)| {
            Ok(BufferedAction {
                nominal_at,
                kind: kind
                    .parse::<ActionKind>()
                    .map_err(|()| corrupt("action kind"))?,
                revision: u32::try_from(revision).map_err(|_| corrupt("action revision"))?,
            })
        })
        .transpose()
    }

    async fn input(
        &self,
        tenant: TenantId,
        digest: &[u8; 32],
    ) -> Result<Option<Vec<u8>>, StoreError> {
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(tenant.as_uuid())
            .bind(digest.as_slice())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)
    }

    async fn namespace_slug(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Option<String>, StoreError> {
        sqlx::query_scalar("SELECT slug FROM namespaces WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)
    }

    async fn record_decision(
        &self,
        decision: &ScheduleDecision,
        audit: Option<&AuditRecord>,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // a manual action works on a paused schedule too, and is not the time that is owed
        let manual = !decision.actions.is_empty()
            && decision
                .actions
                .iter()
                .all(|a| a.kind == ActionKind::Triggered);
        lock_schedule(
            &mut tx,
            decision.tenant_id,
            decision.schedule,
            decision.fence.as_ref(),
            decision.expected_state_version,
            manual,
        )
        .await?;
        if let Some(dedupe) = &decision.dedupe {
            remember_request(&mut tx, decision.tenant_id, dedupe).await?;
        }
        let mut counters: Value =
            sqlx::query_scalar("SELECT counters FROM schedules WHERE tenant_id = $1 AND id = $2")
                .bind(decision.tenant_id.as_uuid())
                .bind(decision.schedule.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
        let mut waiting: i32 = 0;
        for action in decision.actions.iter().chain(&decision.after.extra) {
            insert_recorded(
                &mut tx,
                decision.tenant_id,
                decision.schedule,
                action,
                decision.at,
            )
            .await?;
            bumped(
                &mut counters,
                action.outcome,
                action.summary.map_or(1, |(n, _)| u64::from(n)),
            );
            if action.outcome == Outcome::Buffered {
                waiting += 1;
            }
        }
        sqlx::query(
            "UPDATE schedules SET state_version = state_version + 1, next_action_at = $3, \
             next_actual_at = $4, retry_at = CASE WHEN $10 THEN retry_at ELSE NULL END, \
             remaining_actions = $5, buffered = buffered + $6, counters = $7, \
             recheck_at = CASE WHEN buffered + $6 > 0 THEN $9 + interval '5 seconds' ELSE NULL END, \
             state = CASE WHEN $8::text IS NULL THEN state ELSE 'PAUSED' END, \
             note = COALESCE($8, note), note_by = CASE WHEN $8::text IS NULL THEN note_by ELSE 'system' END, \
             note_at = CASE WHEN $8::text IS NULL THEN note_at ELSE $9 END, updated_at = $9 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(decision.tenant_id.as_uuid())
        .bind(decision.schedule.as_uuid())
        .bind(decision.after.next_action_at)
        .bind(decision.after.next_actual_at)
        .bind(
            decision
                .after
                .remaining_actions
                .map(|n| i32::try_from(n).unwrap_or(i32::MAX)),
        )
        .bind(waiting)
        .bind(counters)
        .bind(decision.pause.as_ref().map(|p| p.note.as_str()))
        .bind(decision.at)
        .bind(manual)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if let Some(audit) = audit {
            insert_audit(&mut tx, audit).await?;
        }
        tx.commit().await.map_err(store_error)
    }

    async fn reconcile_schedule(
        &self,
        reconcile: &Reconcile,
        audit: Option<&AuditRecord>,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_schedule(
            &mut tx,
            reconcile.tenant_id,
            reconcile.schedule,
            reconcile.fence.as_ref(),
            reconcile.expected_state_version,
            false,
        )
        .await?;
        sqlx::query(
            "UPDATE schedules SET state_version = state_version + 1, \
             tracked_runs = COALESCE((SELECT array_agg(t) FROM unnest(tracked_runs) AS t WHERE t <> ALL($3)), '{}'), \
             state = CASE WHEN $4::text IS NULL THEN state ELSE 'PAUSED' END, \
             note = COALESCE($4, note), note_by = CASE WHEN $4::text IS NULL THEN note_by ELSE 'system' END, \
             note_at = CASE WHEN $4::text IS NULL THEN note_at ELSE $5 END, updated_at = $5 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(reconcile.tenant_id.as_uuid())
        .bind(reconcile.schedule.as_uuid())
        .bind(&reconcile.forget)
        .bind(reconcile.pause.as_ref().map(|p| p.note.as_str()))
        .bind(reconcile.at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if let Some(audit) = audit {
            insert_audit(&mut tx, audit).await?;
        }
        tx.commit().await.map_err(store_error)
    }

    async fn recheck_schedule(
        &self,
        tenant: TenantId,
        id: ScheduleId,
        expected_state_version: u64,
        at: Option<OffsetDateTime>,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_schedule(&mut tx, tenant, id, None, expected_state_version, false).await?;
        sqlx::query(
            "UPDATE schedules SET state_version = state_version + 1, recheck_at = $3 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .bind(at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }

    async fn defer_schedule(
        &self,
        tenant: TenantId,
        id: ScheduleId,
        expected_state_version: u64,
        retry_at: OffsetDateTime,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_schedule(&mut tx, tenant, id, None, expected_state_version, false).await?;
        sqlx::query(
            "UPDATE schedules SET state_version = state_version + 1, retry_at = $3, next_actual_at = $3 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .bind(retry_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
}

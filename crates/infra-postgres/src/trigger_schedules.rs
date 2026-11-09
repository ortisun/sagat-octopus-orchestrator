//! Trigger-owned schedules in PostgreSQL (feature 018, US07, `T018-060`, `T018-061`).
//!
//! Three writers keep one invariant, "each cron trigger of the version in force has exactly one `TRIGGER` schedule":
//!
//! * **publication** ([`publish`]), in the transaction that registers the triggers of a version: a schedule for a new
//!   trigger, none touched for a kept one (it keeps its place, its pause and its next time), a tombstone for a dropped
//!   one, and a revival, as the same schedule, for a trigger that comes back. It never refuses a publication: a schedule
//!   that cannot be made is left to the reconciliation, and a taken name is stepped around.
//! * **the reconciliation** ([`TriggerScheduleStore`] for [`PgScheduleStore`]), which reads how the two differ in one
//!   query per round for the shards the replica holds and writes one change per transaction under the lease of the
//!   shard of the trigger's id.
//! * **the firing**, which removes the schedule of a trigger it finds gone (`remove_trigger_schedule`).
//!
//! A schedule is never hard-deleted (the runtime role cannot): removing is the tombstone the schedule API makes, with
//! its waiting actions `DROPPED_DELETED`, its backfills cancelled and its history kept. The unique index on
//! `trigger_id` spans tombstones, so a trigger that returns can only be the same schedule: it is revived, not made again.
//!
//! The runtime role does everything here with the grants it has: it reads `workflow_triggers`, reads and writes
//! `schedules`, `schedule_revisions`, `schedule_actions` and `schedule_backfills`, and takes the row locks it may (it
//! can `UPDATE` all of them).

use sagat_application::dispatch::TriggerRegistration;
use sagat_application::error::StoreError;
use sagat_application::schedule_codec::{policies_to_json, spec_to_json, target_to_json};
use sagat_application::schedule_triggers::{
    Change, Convergence, Drift, DriftPage, DriftWindow, HandoverState, ScanCursor, TriggerEntry,
    TriggerScheduleStore, TriggerSource, trigger_policies, trigger_schedule_name, trigger_spec,
    trigger_target,
};
use sagat_application::schedules::ShardFence;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::schedules::ScheduleId;
use sagat_domain::triggers::TriggerKind;
use sqlx::PgConnection;
use sqlx::Row as _;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;
use crate::schedules::{PgScheduleStore, check_fence};

/// The longest a transaction of the reconciliation waits for a row lock (milliseconds): a schedule that somebody holds
/// is an entry that fails and backs off, so that one held row does not keep the round, and the tenants behind it, waiting.
const LOCK_WAIT_MS: &str = "1000";

/// Starts a transaction of the reconciliation, with its bounded lock wait.
async fn begin(
    pool: &sqlx::PgPool,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, StoreError> {
    let mut tx = pool.begin_safely().await.map_err(store_error)?;
    sqlx::query("SELECT set_config('lock_timeout', $1, true)")
        .bind(LOCK_WAIT_MS)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
    Ok(tx)
}

/// How many names a schedule that cannot have its own is tried under before the attempt is given up (the
/// reconciliation tries again). A tenant that took `trigger-<hex>` in advance can make it two.
const NAME_ATTEMPTS: u32 = 4;

/// What a trigger-owned schedule's revision says about the trigger it was made for.
struct Revision {
    spec: serde_json::Value,
    policies: serde_json::Value,
    action: serde_json::Value,
}

fn unreadable<E>(_: E) -> StoreError {
    StoreError::Internal {
        detail: "a cron trigger's expression or zone is not one this build can read".to_owned(),
    }
}

fn revision_of(workflow: WorkflowId, entry: &TriggerEntry) -> Result<Revision, StoreError> {
    let cron = entry.cron_spec().map_err(unreadable)?;
    let cron = &cron;
    let spec = trigger_spec(cron).map_err(|_| StoreError::Internal {
        detail: "a cron trigger's schedule does not accept its spec".to_owned(),
    })?;
    let policies = trigger_policies().map_err(|_| StoreError::Internal {
        detail: "the policies of a trigger-owned schedule are not accepted".to_owned(),
    })?;
    let target = trigger_target(workflow).map_err(|_| StoreError::Internal {
        detail: "the target of a trigger-owned schedule is not accepted".to_owned(),
    })?;
    Ok(Revision {
        spec: spec_to_json(&spec),
        policies: policies_to_json(&policies),
        action: target_to_json(&target),
    })
}

fn engine_audit(
    tenant: TenantId,
    action: AuditAction,
    schedule: ScheduleId,
    reason: &'static str,
    now: OffsetDateTime,
) -> AuditRecord {
    AuditRecord::engine(
        tenant,
        action,
        ResourceType::Schedule,
        schedule.to_string(),
        reason,
        None,
        now,
    )
}

/// The database's clock, so that every instant of one transaction is the same.
async fn now(conn: &mut PgConnection) -> Result<OffsetDateTime, StoreError> {
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(conn)
        .await
        .map_err(store_error)
}

/// How a trigger's schedule stands.
enum Standing {
    None,
    Tombstone(ScheduleId),
    Alive,
}

async fn standing(
    conn: &mut PgConnection,
    tenant: TenantId,
    trigger: Uuid,
) -> Result<Standing, StoreError> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, state FROM schedules WHERE tenant_id = $1 AND trigger_id = $2 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(trigger)
    .fetch_optional(conn)
    .await
    .map_err(store_error)?;
    Ok(match row {
        None => Standing::None,
        Some((id, state)) if state == "DELETED" => Standing::Tombstone(ScheduleId::new(id)),
        Some(_) => Standing::Alive,
    })
}

/// Makes the schedule of a trigger that has none. `Ok(None)` when a schedule appeared meanwhile (a concurrent
/// reconciliation or publication made it) or when every name was taken. `exhausted` is the calendar's answer that the trigger
/// has no occurrence at all (no next time and no actions left, D2069); a publication, which has no calendar, passes `false` and
/// leaves the schedule to be planned.
async fn insert(
    conn: &mut PgConnection,
    entry: &TriggerEntry,
    next: Option<OffsetDateTime>,
    exhausted: bool,
    now: OffsetDateTime,
) -> Result<Option<ScheduleId>, StoreError> {
    let revision = revision_of(entry.workflow, entry)?;
    let base = trigger_schedule_name(entry.trigger);
    for attempt in 0..NAME_ATTEMPTS {
        let name = if attempt == 0 {
            base.as_str().to_owned()
        } else {
            format!("{}-{attempt}", base.as_str())
        };
        let id = ScheduleId::generate();
        // No target: a taken name and a schedule made meanwhile for this trigger are both "nothing inserted", and
        // neither aborts the transaction (a publication must not be lost to either).
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO schedules \
             (tenant_id, namespace_id, id, name, origin, trigger_id, state, head_revision, state_version, \
              authorized_by, next_action_at, next_actual_at, remaining_actions, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'TRIGGER', $5, 'ACTIVE', 1, 1, $6, $7, $7, $9, $8, $8) \
             ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(entry.tenant_id.as_uuid())
        .bind(entry.namespace_id.as_uuid())
        .bind(id.as_uuid())
        .bind(&name)
        .bind(entry.trigger)
        .bind(Subject::engine().as_str())
        .bind(next)
        .bind(now)
        .bind(exhausted.then_some(0_i32))
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?;
        if inserted.is_none() {
            if matches!(
                standing(conn, entry.tenant_id, entry.trigger).await?,
                Standing::None
            ) {
                // the name is taken: the next attempt takes another
                continue;
            }
            return Ok(None);
        }
        insert_revision(conn, entry.tenant_id, id, 1, &revision, now).await?;
        insert_audit(
            conn,
            &engine_audit(
                entry.tenant_id,
                AuditAction::ScheduleCreated,
                id,
                "schedule of a cron trigger created",
                now,
            ),
        )
        .await?;
        return Ok(Some(id));
    }
    tracing::warn!(
        tenant = %entry.tenant_id,
        trigger = %entry.trigger,
        "no schedule could be made for a cron trigger: every name it may have is taken; the reconciliation tries again"
    );
    metrics::counter!("trigger_schedules_unmade_total", "reason" => "names_taken").increment(1);
    Ok(None)
}

async fn insert_revision(
    conn: &mut PgConnection,
    tenant: TenantId,
    schedule: ScheduleId,
    revision: i32,
    body: &Revision,
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO schedule_revisions \
         (tenant_id, schedule_id, revision, spec, policies, action, input_digest, actor, request_id, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, NULL, $8)",
    )
    .bind(tenant.as_uuid())
    .bind(schedule.as_uuid())
    .bind(revision)
    .bind(&body.spec)
    .bind(&body.policies)
    .bind(&body.action)
    .bind(Subject::engine().as_str())
    .bind(now)
    .execute(conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

/// Brings a tombstone back as the schedule it was: active, with a new revision of the trigger's spec and no history of
/// pauses, notes or owed times. `false` when it was not a tombstone any more.
async fn revive(
    conn: &mut PgConnection,
    entry: &TriggerEntry,
    schedule: ScheduleId,
    next: Option<OffsetDateTime>,
    exhausted: bool,
    now: OffsetDateTime,
) -> Result<bool, StoreError> {
    let revision = revision_of(entry.workflow, entry)?;
    let head: Option<i32> = sqlx::query_scalar(
        "UPDATE schedules SET state = 'ACTIVE', deleted_at = NULL, head_revision = head_revision + 1, \
             state_version = state_version + 1, next_action_at = $3, next_actual_at = $3, remaining_actions = $6, \
             retry_at = NULL, recheck_at = NULL, recovery_until = NULL, recovery_acts = NULL, note = NULL, note_by = NULL, \
             note_at = NULL, authorized_by = $4, buffered = 0, tracked_runs = '{}', updated_at = $5, \
             name = CASE WHEN EXISTS (SELECT 1 FROM schedules o WHERE o.tenant_id = schedules.tenant_id \
                                        AND o.namespace_id = schedules.namespace_id \
                                        AND o.name = schedules.name AND o.state <> 'DELETED') \
                         THEN left(schedules.name, 100) || '-' || substr(md5(random()::text), 1, 8) \
                         ELSE schedules.name END \
         WHERE tenant_id = $1 AND id = $2 AND state = 'DELETED' \
         RETURNING head_revision",
    )
    .bind(entry.tenant_id.as_uuid())
    .bind(schedule.as_uuid())
    .bind(next)
    .bind(Subject::engine().as_str())
    .bind(now)
    .bind(exhausted.then_some(0_i32))
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let Some(head) = head else {
        return Ok(false);
    };
    insert_revision(conn, entry.tenant_id, schedule, head, &revision, now).await?;
    insert_audit(
        conn,
        &engine_audit(
            entry.tenant_id,
            AuditAction::ScheduleCreated,
            schedule,
            "schedule of a cron trigger that came back",
            now,
        ),
    )
    .await?;
    Ok(true)
}

/// Turns a schedule into a tombstone: waiting actions become `DROPPED_DELETED`, backfills are cancelled, history stays.
/// The caller holds the schedule row (or has just updated it).
async fn tombstone(
    conn: &mut PgConnection,
    tenant: TenantId,
    schedule: ScheduleId,
    now: OffsetDateTime,
    reason: &'static str,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE schedule_actions SET outcome = 'DROPPED_DELETED', decided_at = $3 \
         WHERE tenant_id = $1 AND schedule_id = $2 AND outcome = 'BUFFERED'",
    )
    .bind(tenant.as_uuid())
    .bind(schedule.as_uuid())
    .bind(now)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    sqlx::query(
        "UPDATE schedule_backfills SET state = 'CANCELLED', finished_at = $3 \
         WHERE tenant_id = $1 AND schedule_id = $2 AND state IN ('PENDING', 'RUNNING')",
    )
    .bind(tenant.as_uuid())
    .bind(schedule.as_uuid())
    .bind(now)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    insert_audit(
        conn,
        &engine_audit(tenant, AuditAction::ScheduleDeleted, schedule, reason, now),
    )
    .await
}

/// The tombstone itself: no next time, nothing owed, the state version moved.
const SOFT_DELETE: &str = "UPDATE schedules SET state = 'DELETED', deleted_at = $3, buffered = 0, \
     next_action_at = NULL, next_actual_at = NULL, retry_at = NULL, recheck_at = NULL, \
     recovery_until = NULL, recovery_acts = NULL, state_version = state_version + 1, updated_at = $3 \
     WHERE tenant_id = $1 AND id = $2 AND state <> 'DELETED'";

/// Removes the schedules of triggers a publication dropped, in the publishing transaction.
async fn remove_dropped(
    conn: &mut PgConnection,
    tenant: TenantId,
    dropped: &[Uuid],
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    if dropped.is_empty() {
        return Ok(());
    }
    let gone: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE schedules SET state = 'DELETED', deleted_at = $3, buffered = 0, \
             next_action_at = NULL, next_actual_at = NULL, retry_at = NULL, recheck_at = NULL, \
             recovery_until = NULL, recovery_acts = NULL, state_version = state_version + 1, updated_at = $3 \
         WHERE tenant_id = $1 AND trigger_id = ANY($2) AND state <> 'DELETED' RETURNING id",
    )
    .bind(tenant.as_uuid())
    .bind(dropped)
    .bind(now)
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    for id in gone {
        tombstone(
            conn,
            tenant,
            ScheduleId::new(id),
            now,
            "its cron trigger was removed by a publication",
        )
        .await?;
    }
    Ok(())
}

/// What a publication does to the schedules of a workflow's triggers, in its own transaction: `registered` are the
/// triggers of the version now in force, `dropped` the ids of the triggers it removed.
///
/// A publication is never refused for what happens here beyond a failure of the database itself: a trigger whose
/// schedule cannot be made (every name taken, an expression this build cannot turn into a spec) is left to the
/// reconciliation, which tells the operator.
pub(crate) async fn publish(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    workflow: WorkflowId,
    version: WorkflowVersionId,
    registered: &[TriggerRegistration],
    dropped: &[Uuid],
) -> Result<(), StoreError> {
    let at = now(conn).await?;
    remove_dropped(conn, tenant, dropped, at).await?;
    for trigger in registered {
        if trigger.kind != TriggerKind::Cron {
            continue;
        }
        let Some(cron) = trigger.schedule.clone() else {
            continue;
        };
        let entry = TriggerEntry {
            tenant_id: tenant,
            namespace_id: namespace,
            workflow,
            version,
            trigger: trigger.id,
            cron: cron.expression().to_owned(),
            timezone: cron.timezone().to_owned(),
            next_fire_at: None,
        };
        // A kept trigger keeps its schedule untouched: its place, its pause, its next time. A new one has no next time
        // yet (publication has no calendar; the pass that plans it, or the reconciliation, does).
        match standing(conn, tenant, trigger.id).await? {
            Standing::Alive => {}
            Standing::Tombstone(schedule) => {
                revive(conn, &entry, schedule, None, false, at).await?;
            }
            Standing::None => {
                insert(conn, &entry, None, false, at).await?;
            }
        }
    }
    Ok(())
}

// --- the reconciliation -------------------------------------------------------------------------------------------

fn entry_of(row: &sqlx::postgres::PgRow) -> Result<TriggerEntry, StoreError> {
    Ok(TriggerEntry {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        workflow: WorkflowId::new(col::<Uuid>(row, "workflow_id")?),
        version: WorkflowVersionId::new(col::<Uuid>(row, "version_id")?),
        trigger: col(row, "trigger_id")?,
        cron: col(row, "cron")?,
        timezone: col(row, "timezone")?,
        next_fire_at: col(row, "next_fire_at")?,
    })
}

fn drift_of(row: &sqlx::postgres::PgRow) -> Result<Drift, StoreError> {
    let kind: String = col(row, "kind")?;
    let schedule = || -> Result<ScheduleId, StoreError> {
        Ok(ScheduleId::new(col::<Uuid>(row, "schedule_id")?))
    };
    let state_version = || -> Result<u64, StoreError> {
        let version: i64 = col(row, "state_version")?;
        u64::try_from(version).map_err(|_| corrupt("schedule state version"))
    };
    match kind.as_str() {
        "MISSING" => Ok(Drift::Missing(entry_of(row)?)),
        "REVIVED" => Ok(Drift::Revived {
            entry: entry_of(row)?,
            schedule: schedule()?,
        }),
        "ORPHAN" => Ok(Drift::Orphan {
            tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
            schedule: schedule()?,
            trigger: col(row, "trigger_id")?,
            state_version: state_version()?,
        }),
        "RESPEC" => {
            let revision: i32 = col(row, "revision")?;
            Ok(Drift::Respec {
                entry: entry_of(row)?,
                schedule: schedule()?,
                state_version: state_version()?,
                revision: u32::try_from(revision).map_err(|_| corrupt("schedule revision"))?,
            })
        }
        "NO_NEXT_TIME" => Ok(Drift::NoNextTime {
            entry: entry_of(row)?,
            schedule: schedule()?,
            state_version: state_version()?,
        }),
        "PAUSED" => Ok(Drift::Paused {
            entry: entry_of(row)?,
            schedule: schedule()?,
            state_version: state_version()?,
        }),
        _ => Err(corrupt("reconciliation drift")),
    }
}

/// The five ways the schedules differ from the triggers, in one statement over the shards of the window: every
/// trigger is looked at once (it is joined to its schedule, not looked up), the entries that are backing off are left
/// out, and each tenant is given at most `$3` of the `$4` the round may take, interleaved by tenant so that one tenant
/// with a great many does not hold the others back.
const DRIFT: &str = "\
WITH found AS ( \
    SELECT d.kind, d.trigger_id, d.tenant_id, d.namespace_id, d.workflow_id, d.version_id, d.cron, d.timezone, \
           d.next_fire_at, d.schedule_id, d.state_version, d.revision \
      FROM schedule_trigger_drift d \
     WHERE run_shard(d.trigger_id) = ANY($1) AND d.trigger_id <> ALL($2) \
       AND (run_shard(d.trigger_id), d.trigger_id) > ($3::smallint, $4::uuid) \
       AND ($5::smallint IS NULL OR (run_shard(d.trigger_id), d.trigger_id) <= ($5::smallint, $6::uuid)) \
    UNION ALL \
    SELECT 'ORPHAN', s.trigger_id, s.tenant_id, s.namespace_id, NULL::uuid, NULL::uuid, NULL::text, NULL::text, \
           NULL::timestamptz, s.id, s.state_version, s.head_revision \
      FROM schedules s \
     WHERE s.trigger_id IS NOT NULL AND s.state <> 'DELETED' \
       AND run_shard(s.trigger_id) = ANY($1) AND s.trigger_id <> ALL($2) \
       AND (run_shard(s.trigger_id), s.trigger_id) > ($3::smallint, $4::uuid) \
       AND ($5::smallint IS NULL OR (run_shard(s.trigger_id), s.trigger_id) <= ($5::smallint, $6::uuid)) \
       AND NOT EXISTS (SELECT 1 FROM workflow_triggers t WHERE t.id = s.trigger_id AND t.kind = 'cron') \
), ranked AS ( \
    SELECT f.*, row_number() OVER (PARTITION BY f.tenant_id ORDER BY f.trigger_id) AS rn FROM found f \
) \
SELECT kind, trigger_id, tenant_id, namespace_id, workflow_id, version_id, cron, timezone, next_fire_at, \
       schedule_id, state_version, revision \
  FROM ranked WHERE rn <= $7 ORDER BY rn, tenant_id, trigger_id LIMIT $8";

/// The end of a window: the key of the `scan`-th cron trigger after the cursor among the shards held, and whether another
/// follows it. Two rows mean "the window ends at the first, and there is more"; fewer mean the window reaches the last trigger.
/// An index range of at most `scan + 1` entries (`workflow_triggers_cron_shard_idx` is ordered by the same key).
const WINDOW_END: &str = "\
SELECT run_shard(t.id)::smallint AS shard, t.id AS trigger_id FROM workflow_triggers t \
 WHERE t.kind = 'cron' AND run_shard(t.id) = ANY($1) AND (run_shard(t.id), t.id) > ($2::smallint, $3::uuid) \
 ORDER BY run_shard(t.id), t.id OFFSET $4 LIMIT 2";

/// The trigger is still the one the drift was read for: in force, cron, with the expression and zone the change writes.
async fn trigger_is(conn: &mut PgConnection, entry: &TriggerEntry) -> Result<bool, StoreError> {
    let row: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM workflow_triggers WHERE id = $1 AND tenant_id = $2 AND kind = 'cron' \
         AND cron = $3 AND timezone = $4 FOR SHARE",
    )
    .bind(entry.trigger)
    .bind(entry.tenant_id.as_uuid())
    .bind(&entry.cron)
    .bind(&entry.timezone)
    .fetch_optional(conn)
    .await
    .map_err(store_error)?;
    Ok(row.is_some())
}

/// Locks the schedule row and checks it is the state the change was made from.
async fn lock(
    conn: &mut PgConnection,
    tenant: TenantId,
    schedule: ScheduleId,
    state_version: u64,
) -> Result<String, StoreError> {
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT state, state_version FROM schedules WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(schedule.as_uuid())
    .fetch_optional(conn)
    .await
    .map_err(store_error)?;
    let Some((state, version)) = row else {
        return Err(StoreError::MissingReference);
    };
    if u64::try_from(version).ok() != Some(state_version) {
        return Err(StoreError::StateConflict(
            "the schedule changed since the drift was read",
        ));
    }
    Ok(state)
}

impl PgScheduleStore {
    /// Removes the schedule of a trigger that is gone, as a tombstone, under a fence and the compare-and-swap of the
    /// state it was read in. `StateConflict` when the trigger is not gone after all, the lease is not held or the
    /// schedule changed.
    pub(crate) async fn remove_gone(
        &self,
        tenant: TenantId,
        schedule: ScheduleId,
        trigger: Option<Uuid>,
        state_version: u64,
        fence: Option<&ShardFence>,
        fence_of: Uuid,
    ) -> Result<(), StoreError> {
        let mut tx = begin(self.pool()).await?;
        check_fence(&mut tx, fence_of, fence).await?;
        // the identity of a schedule never changes: its trigger is read before any lock, so that the trigger's row is taken
        // before the schedule's, the order a publication takes them in (D2050)
        let trigger_id: Option<Uuid> =
            sqlx::query_scalar("SELECT trigger_id FROM schedules WHERE tenant_id = $1 AND id = $2")
                .bind(tenant.as_uuid())
                .bind(schedule.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
        let Some(trigger_id) = trigger_id else {
            return Err(StoreError::StateConflict("the schedule is not a trigger's"));
        };
        if trigger.is_some_and(|expected| expected != trigger_id) {
            return Err(StoreError::StateConflict(
                "the schedule is another trigger's",
            ));
        }
        // under the trigger's row lock when it is there: a trigger that is in force is not removed from under a publication
        let alive: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM workflow_triggers \
              WHERE id = $1 AND tenant_id = $2 AND kind = 'cron' FOR SHARE",
        )
        .bind(trigger_id)
        .bind(tenant.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        if alive.is_some() {
            return Err(StoreError::StateConflict("the trigger is still in force"));
        }
        let state = lock(&mut tx, tenant, schedule, state_version).await?;
        if state == "DELETED" {
            return Err(StoreError::StateConflict("the schedule is already removed"));
        }
        let at = now(&mut tx).await?;
        sqlx::query(SOFT_DELETE)
            .bind(tenant.as_uuid())
            .bind(schedule.as_uuid())
            .bind(at)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        tombstone(
            &mut tx,
            tenant,
            schedule,
            at,
            "its cron trigger is no longer in force",
        )
        .await?;
        tx.commit().await.map_err(store_error)
    }

    /// The cron trigger as it is now, for the firing.
    pub(crate) async fn read_trigger_source(
        &self,
        tenant: TenantId,
        trigger: Uuid,
    ) -> Result<Option<TriggerSource>, StoreError> {
        let row: Option<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT workflow_id, version_id FROM workflow_triggers \
             WHERE tenant_id = $1 AND id = $2 AND kind = 'cron'",
        )
        .bind(tenant.as_uuid())
        .bind(trigger)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        Ok(row.map(|(workflow, version)| TriggerSource {
            trigger,
            workflow: WorkflowId::new(workflow),
            version: WorkflowVersionId::new(version),
        }))
    }

    /// The run a tick already started under feature 004's key.
    pub(crate) async fn read_tick_run(
        &self,
        tenant: TenantId,
        trigger: Uuid,
        nominal: OffsetDateTime,
    ) -> Result<Option<Uuid>, StoreError> {
        let identity = sagat_application::schedule_triggers::tick_identity(trigger, nominal);
        let run: Option<String> = sqlx::query_scalar(
            "SELECT resource_id FROM command_dedupe \
             WHERE tenant_id = $1 AND scope = $2 AND idempotency_key = $3",
        )
        .bind(tenant.as_uuid())
        .bind(&identity.scope)
        .bind(&identity.key)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        run.map(|text| Uuid::parse_str(&text).map_err(|_| corrupt("the run of a tick")))
            .transpose()
    }

    /// Where the cron triggers fire from.
    pub(crate) async fn read_handover(&self) -> Result<HandoverState, StoreError> {
        handover_state(self.pool()).await
    }

    /// Whether the control of the schedules of the triggers is open: handed over, and the engines have had time to learn of it.
    pub(crate) async fn read_control_open(&self) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT state = 'HANDED_OVER' AND changed_at + make_interval(secs => $1::float8) <= clock_timestamp() \
               FROM schedule_trigger_handover",
        )
        .bind(sagat_application::schedule_triggers::HandoverCache::CONTROL_WAIT_SECONDS)
        .fetch_one(self.pool())
        .await
        .map_err(store_error)
    }
}

/// Where the cron triggers fire from: the one row, read by the services, the reconciliation and the legacy cron pass alike.
pub(crate) async fn handover_state(pool: &sqlx::PgPool) -> Result<HandoverState, StoreError> {
    let state: String = sqlx::query_scalar("SELECT state FROM schedule_trigger_handover")
        .fetch_one(pool)
        .await
        .map_err(store_error)?;
    Ok(HandoverState::from_stored(&state))
}

impl TriggerScheduleStore for PgScheduleStore {
    async fn handover(&self) -> Result<HandoverState, StoreError> {
        self.read_handover().await
    }

    async fn drift(&self, window: &DriftWindow) -> Result<DriftPage, StoreError> {
        let shards: Vec<i16> = window
            .shards
            .iter()
            .map(|shard| i16::from(shard.index()))
            .collect();
        let after = window.after.unwrap_or(ScanCursor {
            shard: -1,
            trigger: Uuid::nil(),
        });
        // the window and its drift are two statements of one read, and a bound on both: a slow scan is a failed round, never a
        // statement that runs for ever (D2047)
        let mut tx = begin(self.pool()).await?;
        sqlx::query("SELECT set_config('statement_timeout', '4000', true)")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let ends: Vec<(i16, Uuid)> = sqlx::query_as(WINDOW_END)
            .bind(&shards)
            .bind(after.shard)
            .bind(after.trigger)
            .bind(i64::from(window.scan.max(1)) - 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
        let (end, more) = if ends.len() == 2 {
            (Some(ends[0]), true)
        } else {
            (None, false)
        };
        let rows = sqlx::query(DRIFT)
            .bind(&shards)
            .bind(&window.exclude)
            .bind(after.shard)
            .bind(after.trigger)
            .bind(end.map(|(shard, _)| shard))
            .bind(end.map(|(_, trigger)| trigger))
            .bind(i64::from(window.per_tenant))
            .bind(i64::from(window.limit))
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
        tx.rollback().await.map_err(store_error)?;
        // everything the window found fits the round: go on after it; the round's limit cut it short: stay, the rest is found
        // once what was written is no longer drift
        let fits = u32::try_from(rows.len()).unwrap_or(u32::MAX) < window.limit;
        let next = if fits {
            end.map(|(shard, trigger)| ScanCursor { shard, trigger })
        } else {
            window.after
        };
        let mut drift = Vec::with_capacity(rows.len());
        for row in &rows {
            match drift_of(row) {
                Ok(one) => drift.push(one),
                Err(_) => {
                    // a row another build wrote: skipped, said by its trigger id alone, counted; the rest of the round goes on
                    let trigger: Option<Uuid> = row.try_get("trigger_id").ok();
                    tracing::warn!(
                        ?trigger,
                        "a row of the reconciliation could not be decoded and was skipped"
                    );
                    metrics::counter!("trigger_schedule_rows_undecodable_total").increment(1);
                }
            }
        }
        Ok(DriftPage {
            drift,
            next,
            more: more || !fits,
        })
    }

    async fn convergence(&self) -> Result<Convergence, StoreError> {
        let mut tx = begin(self.pool()).await?;
        sqlx::query("SELECT set_config('statement_timeout', '4000', true)")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let (count, examples): (i32, Vec<Uuid>) = sqlx::query_as(
            "SELECT out_count, out_examples FROM schedule_trigger_convergence(100000)",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.rollback().await.map_err(store_error)?;
        Ok(Convergence {
            unconverged: u32::try_from(count).map_err(|_| corrupt("unconverged count"))?,
            examples,
        })
    }

    async fn legacy_fires(
        &self,
        since: Option<OffsetDateTime>,
    ) -> Result<Vec<(Uuid, OffsetDateTime)>, StoreError> {
        sqlx::query_as(
            "SELECT t.id, t.last_fire_at FROM workflow_triggers t CROSS JOIN schedule_trigger_handover h \
              WHERE t.kind = 'cron' AND h.state = 'HANDED_OVER' \
                AND t.last_fire_at > h.changed_at + interval '10 seconds' \
                AND ($1::timestamptz IS NULL OR t.last_fire_at > $1) \
              ORDER BY t.last_fire_at DESC LIMIT 20",
        )
        .bind(since)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)
    }

    async fn apply(&self, change: &Change, fence: Option<&ShardFence>) -> Result<(), StoreError> {
        match change {
            Change::Create { entry, next } => {
                let mut tx = begin(self.pool()).await?;
                check_fence(&mut tx, entry.trigger, fence).await?;
                if !trigger_is(&mut tx, entry).await? {
                    return Err(StoreError::StateConflict(
                        "the trigger changed since the drift was read",
                    ));
                }
                let at = now(&mut tx).await?;
                if insert(&mut tx, entry, *next, next.is_none(), at)
                    .await?
                    .is_none()
                {
                    return Err(StoreError::StateConflict(
                        "a schedule of the trigger exists already, or its names are taken",
                    ));
                }
                tx.commit().await.map_err(store_error)
            }
            Change::Revive {
                entry,
                schedule,
                next,
            } => {
                let mut tx = begin(self.pool()).await?;
                check_fence(&mut tx, entry.trigger, fence).await?;
                if !trigger_is(&mut tx, entry).await? {
                    return Err(StoreError::StateConflict(
                        "the trigger changed since the drift was read",
                    ));
                }
                let at = now(&mut tx).await?;
                if !revive(&mut tx, entry, *schedule, *next, next.is_none(), at).await? {
                    return Err(StoreError::StateConflict(
                        "the schedule is not a tombstone any more",
                    ));
                }
                tx.commit().await.map_err(store_error)
            }
            Change::Remove {
                tenant_id,
                schedule,
                trigger,
                expected_state_version,
            } => {
                self.remove_gone(
                    *tenant_id,
                    *schedule,
                    Some(*trigger),
                    *expected_state_version,
                    fence,
                    *trigger,
                )
                .await
            }
            Change::Respec {
                entry,
                schedule,
                expected_state_version,
                expected_revision,
                next,
            } => {
                let mut tx = begin(self.pool()).await?;
                check_fence(&mut tx, entry.trigger, fence).await?;
                // the trigger's row first, then the schedule's: the order of a publication (a deadlock here could abort one)
                if !trigger_is(&mut tx, entry).await? {
                    return Err(StoreError::StateConflict(
                        "the trigger or the schedule changed since the drift was read",
                    ));
                }
                let state =
                    lock(&mut tx, entry.tenant_id, *schedule, *expected_state_version).await?;
                if state == "DELETED" {
                    return Err(StoreError::StateConflict(
                        "the trigger or the schedule changed since the drift was read",
                    ));
                }
                let head: i32 = sqlx::query_scalar(
                    "SELECT head_revision FROM schedules WHERE tenant_id = $1 AND id = $2",
                )
                .bind(entry.tenant_id.as_uuid())
                .bind(schedule.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
                if u32::try_from(head).ok() != Some(*expected_revision) {
                    return Err(StoreError::StateConflict(
                        "the schedule has another revision now",
                    ));
                }
                let at = now(&mut tx).await?;
                let revision = revision_of(entry.workflow, entry)?;
                let new_head = head.saturating_add(1);
                // a paused schedule stays paused and has no next time; an active one is planned again
                sqlx::query(
                    "UPDATE schedules SET head_revision = $3, state_version = state_version + 1, \
                         next_action_at = CASE WHEN state = 'ACTIVE' THEN $4 END, \
                         next_actual_at = CASE WHEN state = 'ACTIVE' THEN $4 END, \
                         remaining_actions = CASE WHEN state = 'ACTIVE' AND $4::timestamptz IS NULL THEN 0 END, \
                         retry_at = NULL, recovery_until = NULL, recovery_acts = NULL, updated_at = $5 \
                     WHERE tenant_id = $1 AND id = $2",
                )
                .bind(entry.tenant_id.as_uuid())
                .bind(schedule.as_uuid())
                .bind(new_head)
                .bind(*next)
                .bind(at)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                insert_revision(&mut tx, entry.tenant_id, *schedule, new_head, &revision, at)
                    .await?;
                insert_audit(
                    &mut tx,
                    &engine_audit(
                        entry.tenant_id,
                        AuditAction::ScheduleUpdated,
                        *schedule,
                        "schedule of a cron trigger brought back to its trigger",
                        at,
                    ),
                )
                .await?;
                tx.commit().await.map_err(store_error)
            }
            Change::SetNext {
                entry,
                schedule,
                expected_state_version,
                next,
            } => {
                let mut tx = begin(self.pool()).await?;
                check_fence(&mut tx, entry.trigger, fence).await?;
                if !trigger_is(&mut tx, entry).await? {
                    return Err(StoreError::StateConflict(
                        "the trigger or the schedule changed since the drift was read",
                    ));
                }
                let state =
                    lock(&mut tx, entry.tenant_id, *schedule, *expected_state_version).await?;
                if state != "ACTIVE" {
                    return Err(StoreError::StateConflict(
                        "the trigger or the schedule changed since the drift was read",
                    ));
                }
                let at = now(&mut tx).await?;
                sqlx::query(
                    "UPDATE schedules SET next_action_at = $3, next_actual_at = $3, remaining_actions = NULL, \
                         state_version = state_version + 1, updated_at = $4 \
                     WHERE tenant_id = $1 AND id = $2",
                )
                .bind(entry.tenant_id.as_uuid())
                .bind(schedule.as_uuid())
                .bind(*next)
                .bind(at)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                tx.commit().await.map_err(store_error)
            }
            Change::Exhaust {
                entry,
                schedule,
                expected_state_version,
            } => {
                let mut tx = begin(self.pool()).await?;
                check_fence(&mut tx, entry.trigger, fence).await?;
                if !trigger_is(&mut tx, entry).await? {
                    return Err(StoreError::StateConflict(
                        "the trigger or the schedule changed since the drift was read",
                    ));
                }
                let state =
                    lock(&mut tx, entry.tenant_id, *schedule, *expected_state_version).await?;
                if state != "ACTIVE" {
                    return Err(StoreError::StateConflict(
                        "the trigger or the schedule changed since the drift was read",
                    ));
                }
                let at = now(&mut tx).await?;
                sqlx::query(
                    "UPDATE schedules SET next_action_at = NULL, next_actual_at = NULL, remaining_actions = 0, \
                         state_version = state_version + 1, updated_at = $3 \
                     WHERE tenant_id = $1 AND id = $2",
                )
                .bind(entry.tenant_id.as_uuid())
                .bind(schedule.as_uuid())
                .bind(at)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                tracing::warn!(
                    tenant = %entry.tenant_id,
                    trigger = %entry.trigger,
                    "the calendar of a cron trigger has no occurrence at all: its schedule is marked exhausted and the trigger never fires"
                );
                tx.commit().await.map_err(store_error)
            }
            Change::Resume {
                entry,
                schedule,
                expected_state_version,
                next,
            } => {
                let mut tx = begin(self.pool()).await?;
                check_fence(&mut tx, entry.trigger, fence).await?;
                if !trigger_is(&mut tx, entry).await? {
                    return Err(StoreError::StateConflict(
                        "the trigger or the schedule changed since the drift was read",
                    ));
                }
                let state =
                    lock(&mut tx, entry.tenant_id, *schedule, *expected_state_version).await?;
                if state != "PAUSED" {
                    return Err(StoreError::StateConflict(
                        "the trigger or the schedule changed since the drift was read",
                    ));
                }
                let at = now(&mut tx).await?;
                sqlx::query(
                    "UPDATE schedules SET state = 'ACTIVE', next_action_at = $3, next_actual_at = $3, \
                         remaining_actions = CASE WHEN $3::timestamptz IS NULL THEN 0 END, retry_at = NULL, \
                         recheck_at = NULL, recovery_until = NULL, recovery_acts = NULL, note = NULL, note_by = NULL, \
                         note_at = NULL, state_version = state_version + 1, updated_at = $4 \
                     WHERE tenant_id = $1 AND id = $2",
                )
                .bind(entry.tenant_id.as_uuid())
                .bind(schedule.as_uuid())
                .bind(*next)
                .bind(at)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                insert_audit(
                    &mut tx,
                    &engine_audit(
                        entry.tenant_id,
                        AuditAction::ScheduleResumed,
                        *schedule,
                        "schedule of a cron trigger resumed: it was paused before the handover, when nothing may pause it",
                        at,
                    ),
                )
                .await?;
                tx.commit().await.map_err(store_error)
            }
        }
    }
}

/// What the handover write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoverWritten {
    /// The state moved from `LEGACY` to `HANDED_OVER` in this transaction.
    HandedOver,
    /// It was handed over before: nothing was written.
    Already,
}

/// The write of the handover, the step of the activation of `schedules-v1` (run inside its transaction, with the
/// schema-owning role): the definer function moves the state, records the operator and the database role, on the
/// database's clock, and refuses unless the requirement is recorded.
///
/// # Errors
/// The store's.
pub async fn complete_handover(
    conn: &mut PgConnection,
    operator: &str,
) -> Result<HandoverWritten, StoreError> {
    let outcome: String = sqlx::query_scalar("SELECT schedule_trigger_handover_complete($1)")
        .bind(operator)
        .fetch_one(conn)
        .await
        .map_err(store_error)?;
    match outcome.as_str() {
        "HANDED_OVER" => Ok(HandoverWritten::HandedOver),
        "ALREADY_HANDED_OVER" => Ok(HandoverWritten::Already),
        _ => Err(corrupt("handover outcome")),
    }
}

/// What the handover command is told of the fleet and of the triggers, in one statement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Census {
    /// Execution engines seen within 30 seconds.
    pub live: u32,
    /// How many of them do not list `schedules-v1`, and up to sixteen of their ids.
    pub lacking: u32,
    pub lacking_examples: Vec<Uuid>,
    /// Cron triggers in force with no schedule equal to them (D2046), and up to sixteen of their ids.
    pub unconverged: u32,
    pub unconverged_examples: Vec<Uuid>,
}

/// The execution engines the registry counts live (seen within 30 seconds), how many of them do not list `schedules-v1`
/// (an engine lists it only when its configuration runs the schedule pass, `SAGAT_SCHEDULES_ENABLED`) and up to sixteen of
/// their ids, in one statement. Read through the owner's function: the runtime role reads the registry by column.
///
/// # Errors
/// The store's.
pub async fn engine_census(conn: &mut PgConnection, lock: bool) -> Result<Census, StoreError> {
    let row: (i32, i32, Vec<Uuid>, i32, Vec<Uuid>) = sqlx::query_as(
        "SELECT out_live, out_lacking, out_examples, out_unconverged, out_unconverged_examples \
           FROM schedule_trigger_handover_census($1)",
    )
    .bind(lock)
    .fetch_one(conn)
    .await
    .map_err(store_error)?;
    Ok(Census {
        live: u32::try_from(row.0).map_err(|_| corrupt("live engines"))?,
        lacking: u32::try_from(row.1).map_err(|_| corrupt("lacking engines"))?,
        lacking_examples: row.2,
        unconverged: u32::try_from(row.3).map_err(|_| corrupt("unconverged triggers"))?,
        unconverged_examples: row.4,
    })
}

/// The part of the activation that is the feature's own, as the registry runs it with the requirement epoch held: the census under
/// the lock of the cron triggers' table, handed to `judge` (which says whether the handover must be refused, and keeps its own
/// reasons), and, if it does not refuse, the move of the state. The connection stays inside this crate (D2077).
pub fn handover_step<J>(
    actor: String,
    judge: J,
) -> impl for<'c> FnOnce(&'c mut PgConnection) -> crate::FeatureStep<'c> + Send
where
    J: FnOnce(&Census) -> bool + Send + 'static,
{
    move |conn| {
        Box::pin(async move {
            let census = engine_census(conn, true).await?;
            if judge(&census) {
                return Err(StoreError::StateConflict(
                    "the handover was refused under the lock",
                ));
            }
            complete_handover(conn, &actor).await.map(|_| ())
        })
    }
}

/// The epoch of the instance registry: what an activation moved it to, for the line the command prints.
///
/// # Errors
/// The store's.
pub async fn read_epoch(conn: &mut PgConnection) -> Result<i64, StoreError> {
    sqlx::query_scalar("SELECT epoch FROM platform_requirement_epoch")
        .fetch_one(conn)
        .await
        .map_err(store_error)
}

/// The state of the handover and who made it, for the operator command and for `--dry-run`.
///
/// # Errors
/// The store's.
pub async fn read_handover_row(
    conn: &mut PgConnection,
) -> Result<(HandoverState, Option<String>, Option<String>), StoreError> {
    let row: (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT state, changed_by, changed_db_role FROM schedule_trigger_handover")
            .fetch_one(conn)
            .await
            .map_err(store_error)?;
    Ok((HandoverState::from_stored(&row.0), row.1, row.2))
}

/// Whether the role of this session may run the handover: execute the two functions of the activation. The runtime role
/// may not (a compromised API must not be able to brick the fleet); the schema-owning role may.
///
/// # Errors
/// The store's.
pub async fn may_hand_over(conn: &mut PgConnection) -> Result<bool, StoreError> {
    sqlx::query_scalar(
        "SELECT has_function_privilege('platform_requirement_activate(text, text[], text)', 'EXECUTE') \
            AND has_function_privilege('schedule_trigger_handover_complete(text)', 'EXECUTE') \
            AND has_function_privilege('schedule_trigger_handover_census(boolean)', 'EXECUTE')",
    )
    .fetch_one(conn)
    .await
    .map_err(store_error)
}

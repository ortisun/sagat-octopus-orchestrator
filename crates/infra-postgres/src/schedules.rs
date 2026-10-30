//! The schedule store (feature 018): schedules, their immutable revisions, the actions taken on
//! their nominal times, backfills and the owed cancellations of `CANCEL_OTHER`.
//!
//! This file owns the columns of those tables in the storage classification. So far it creates and
//! reads schedules; the changes (update, pause, resume, delete) and the pass's statements arrive
//! with their tasks (T018-014..).

use sagat_application::error::{ConflictKind, StoreError};
use sagat_application::schedule_codec::{
    policies_from_json, policies_to_json, spec_from_json, spec_to_json, target_from_json,
    target_to_json,
};
use sagat_application::schedules::{
    NewSchedule, Revise, ScheduleOrigin, ScheduleRecord, ScheduleState, ScheduleStore, StateChange,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::schedules::{Ceilings, ScheduleId, ScheduleName};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;
use crate::quotas::{ensure_room_for_schedule, lock_schedules};

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
            "s.remaining_actions, s.next_action_at, s.next_actual_at, s.retry_at, s.tracked_runs, ",
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
        tracked_runs: col(row, "tracked_runs")?,
        buffered: u32::try_from(buffered).map_err(|_| corrupt("buffered count"))?,
        created_at: col(row, "created_at")?,
        updated_at: col(row, "updated_at")?,
    })
}

impl ScheduleStore for PgScheduleStore {
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
              retry_at, tracked_runs, buffered, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21)",
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
             AND ($3::uuid IS NULL OR s.id > $3) ORDER BY s.id LIMIT $4"
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
        if let Some(input) = &revise.input {
            if Sha256::digest(&input.bytes).as_slice() != input.digest.as_slice()
                || r.input_digest != Some(input.digest)
            {
                return Err(StoreError::StateConflict(
                    "the input of a schedule must match its digest",
                ));
            }
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
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM schedules WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        match state.as_deref() {
            None => return Ok(false),
            Some(s) if s == ScheduleState::Deleted.as_str() => return Ok(false),
            Some(_) => {}
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
}

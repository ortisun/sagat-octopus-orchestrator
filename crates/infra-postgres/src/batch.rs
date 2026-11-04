//! The batch store (feature 018): jobs, their items and the previews that count the runs a selector
//! would pick.
//!
//! This file owns the columns of `batch_jobs` and `batch_items` in the storage classification. It
//! creates, reads, lists and cancels jobs and reads their items; the selection of the runs and the
//! application of the action to each are the executor's statements (`T018-046`).

use sagat_application::batch::{
    ActiveJobLimits, BatchStore, Counts, Drained, DueJob, ItemOutcome, ItemRecord, JobParams,
    JobRecord, NewJob, NextItem, Preview, Selected, failure_reason_from, item_state_from,
    item_state_name, selector_from_json, selector_to_json,
};
use sagat_application::definitions::Applied;
use sagat_application::error::StoreError;
use sagat_application::quotas::QuotaLookup;
use sagat_application::schedules::ShardFence;
use sagat_domain::audit::AuditRecord;
use sagat_domain::batch::{
    Action, FailureReason, ItemState, JobState, Rate, Selector, SelectorKind, next_slot,
};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::sharding::Shard;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;
use crate::quotas::{effective, ensure_room_for_batch, lock_batch_jobs};
use crate::schedules::check_fence;

/// The batch store over PostgreSQL.
#[derive(Clone)]
pub struct PgBatchStore {
    pool: PgPool,
}

impl PgBatchStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl QuotaLookup for PgBatchStore {
    async fn effective_limit(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> Result<u64, StoreError> {
        effective(&self.pool, tenant, namespace, resource).await
    }
}

macro_rules! select_jobs {
    ($tail:literal) => {
        concat!(
            "SELECT tenant_id, namespace_id, id, action, params, payload_digest, selector, reason, ",
            "created_by, state, failure_reason, max_items, rate_milli, selected, succeeded, skipped, ",
            "failed, cancelled, deferred, select_cursor, selection_bound_at, next_slot_at, created_at, ",
            "started_at, finished_at, expires_at FROM batch_jobs ",
            $tail
        )
    };
}

fn count(row: &PgRow, name: &str) -> Result<u32, StoreError> {
    u32::try_from(col::<i32>(row, name)?).map_err(|_| corrupt("a batch counter"))
}

fn job_from(row: &PgRow) -> Result<JobRecord, StoreError> {
    let action: String = col(row, "action")?;
    let state: String = col(row, "state")?;
    let params: Value = col(row, "params")?;
    // the parameters were written by the service; a document this build cannot read is corrupt
    JobParams::from_json(&params).map_err(|_| corrupt("batch parameters"))?;
    let selector: Value = col(row, "selector")?;
    let digest: Option<Vec<u8>> = col(row, "payload_digest")?;
    let failure: Option<String> = col(row, "failure_reason")?;
    Ok(JobRecord {
        id: col(row, "id")?,
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        action: action
            .parse::<Action>()
            .map_err(|_| corrupt("batch action"))?,
        params,
        payload_digest: digest
            .map(|bytes| <[u8; 32]>::try_from(bytes.as_slice()))
            .transpose()
            .map_err(|_| corrupt("batch payload digest"))?,
        selector: selector_from_json(&selector).map_err(|_| corrupt("batch selector"))?,
        reason: col(row, "reason")?,
        created_by: col(row, "created_by")?,
        state: state
            .parse::<JobState>()
            .map_err(|_| corrupt("batch state"))?,
        failure_reason: failure
            .map(|name| failure_reason_from(&name))
            .transpose()
            .map_err(|_| corrupt("batch failure reason"))?,
        max_items: count(row, "max_items")?,
        rate: Rate::from_milli(count(row, "rate_milli")?).map_err(|_| corrupt("batch rate"))?,
        counts: Counts {
            selected: count(row, "selected")?,
            succeeded: count(row, "succeeded")?,
            skipped: count(row, "skipped")?,
            failed: count(row, "failed")?,
            cancelled: count(row, "cancelled")?,
            deferred: count(row, "deferred")?,
        },
        select_cursor: col(row, "select_cursor")?,
        selection_bound_at: col(row, "selection_bound_at")?,
        next_slot_at: col(row, "next_slot_at")?,
        created_at: col(row, "created_at")?,
        started_at: col(row, "started_at")?,
        finished_at: col(row, "finished_at")?,
        expires_at: col(row, "expires_at")?,
    })
}

fn item_from(row: &PgRow) -> Result<ItemRecord, StoreError> {
    let state: String = col(row, "state")?;
    Ok(ItemRecord {
        ordinal: count(row, "ordinal")?,
        run_id: col(row, "run_id")?,
        state: item_state_from(&state).map_err(|_| corrupt("batch item state"))?,
        reason: col(row, "reason")?,
        error_code: col(row, "error_code")?,
        attempts: u32::try_from(col::<i16>(row, "attempts")?)
            .map_err(|_| corrupt("batch item attempts"))?,
        next_attempt_at: col(row, "next_attempt_at")?,
        first_refused_at: col(row, "first_refused_at")?,
        new_run_id: col(row, "new_run_id")?,
        updated_at: col(row, "updated_at")?,
    })
}

/// The runs a selector matches in a namespace, created at or before `$11`, followed by `$tail`. The
/// parameters are fixed: tenant, namespace, workflow, version, statuses, origin, schedule, created
/// after, created before, run ids and the bound (`$1` to `$11`, `NULL` meaning "no restriction").
/// The macro takes one trailing literal, the only shape the runtime-privilege audit expands.
macro_rules! matching_runs {
    ($tail:literal) => {
        concat!(
            "SELECT r.id FROM runs r \
             WHERE r.tenant_id = $1 AND r.namespace_id = $2 AND r.created_at <= $11 \
              AND ($3::uuid IS NULL OR r.workflow_id = $3) \
              AND ($4::uuid IS NULL OR r.workflow_version_id = $4) \
              AND (cardinality($5::text[]) = 0 OR r.status = ANY($5)) \
              AND ($6::text IS NULL OR r.origin = $6) \
              AND ($7::uuid IS NULL OR r.schedule_id = $7) \
              AND ($8::timestamptz IS NULL OR r.created_at > $8) \
              AND ($9::timestamptz IS NULL OR r.created_at < $9) \
              AND ($10::uuid[] IS NULL OR r.id = ANY($10)) ",
            $tail
        )
    };
}

/// The filter of a selector as the statements take it: every parameter is optional, and the
/// statement is one fixed text (`NULL` means "no restriction").
struct Bind {
    workflow: Option<Uuid>,
    version: Option<Uuid>,
    statuses: Vec<String>,
    origin: Option<String>,
    schedule: Option<Uuid>,
    after: Option<OffsetDateTime>,
    before: Option<OffsetDateTime>,
    ids: Option<Vec<Uuid>>,
}

impl Bind {
    fn of(selector: &Selector) -> Self {
        match selector.kind() {
            SelectorKind::RunIds(ids) => Self {
                workflow: None,
                version: None,
                statuses: Vec::new(),
                origin: None,
                schedule: None,
                after: None,
                before: None,
                ids: Some(ids.clone()),
            },
            SelectorKind::Filter(filter) => Self {
                workflow: filter.workflow_id,
                version: filter.version_id,
                statuses: filter
                    .statuses
                    .iter()
                    .map(|s| s.as_str().to_owned())
                    .collect(),
                origin: filter.origin.clone(),
                schedule: filter.schedule_id,
                after: filter.created_after,
                before: filter.created_before,
                ids: None,
            },
        }
    }
}

impl BatchStore for PgBatchStore {
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

    async fn create_job(
        &self,
        new: &NewJob,
        limits: ActiveJobLimits,
        audit: &AuditRecord,
        idempotency: &sagat_application::definitions::Idempotency,
    ) -> Result<Applied<()>, StoreError> {
        let r = &new.record;
        if let Some(payload) = &new.payload {
            // payloads are content-addressed: the bytes must be the ones the digest names
            if Sha256::digest(&payload.bytes).as_slice() != payload.digest.as_slice()
                || r.payload_digest != Some(payload.digest)
            {
                return Err(StoreError::StateConflict(
                    "the payload of a batch job must match its digest",
                ));
            }
        }
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let hash = crate::definitions::hash(&idempotency.fingerprint);
        if let Some(existing) =
            crate::definitions::replay(&mut tx, r.tenant_id, idempotency, &hash).await?
        {
            return Ok(Applied::Replayed(existing));
        }
        lock_batch_jobs(&mut tx, r.tenant_id, r.namespace_id).await?;
        ensure_room_for_batch(
            &mut tx,
            r.tenant_id,
            r.namespace_id,
            limits.namespace,
            limits.tenant,
        )
        .await?;
        if let Some(payload) = &new.payload {
            sqlx::query(
                "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) \
                 ON CONFLICT (tenant_id, digest) DO NOTHING",
            )
            .bind(r.tenant_id.as_uuid())
            .bind(payload.digest.as_slice())
            .bind(payload.bytes.as_slice())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        // The bound, the horizon and the first slot come from the database: the clock it keeps and
        // the identity of this very transaction. The selection starts once every transaction that
        // had an identity now has ended (`pg_snapshot_xmin` is past the horizon).
        sqlx::query(
            "INSERT INTO batch_jobs \
             (tenant_id, namespace_id, id, action, params, payload_digest, selector, reason, created_by, \
              state, max_items, rate_milli, selection_bound_at, selection_horizon, next_slot_at, \
              created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'SELECTING', $10, $11, clock_timestamp(), \
                     pg_current_xact_id(), clock_timestamp(), $12, $13)",
        )
        .bind(r.tenant_id.as_uuid())
        .bind(r.namespace_id.as_uuid())
        .bind(r.id)
        .bind(r.action.as_str())
        .bind(&r.params)
        .bind(r.payload_digest.as_ref().map(<[u8; 32]>::as_slice))
        .bind(selector_to_json(&r.selector))
        .bind(&r.reason)
        .bind(&r.created_by)
        .bind(i32::try_from(r.max_items).unwrap_or(i32::MAX))
        .bind(i32::try_from(r.rate.milli()).unwrap_or(i32::MAX))
        .bind(r.created_at)
        .bind(r.expires_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        if !crate::definitions::remember(
            &mut tx,
            r.tenant_id,
            idempotency,
            &hash,
            &r.id.to_string(),
        )
        .await?
        {
            // a request with this key committed between the look and the write: this one starts over
            // and finds it
            return Err(StoreError::StateConflict(
                "a request with this key was already applied",
            ));
        }
        tx.commit().await.map_err(store_error)?;
        Ok(Applied::Done(()))
    }

    async fn job(&self, tenant: TenantId, id: Uuid) -> Result<Option<JobRecord>, StoreError> {
        let row = sqlx::query(select_jobs!("WHERE tenant_id = $1 AND id = $2"))
            .bind(tenant.as_uuid())
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
        row.as_ref().map(job_from).transpose()
    }

    async fn jobs(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        state: Option<JobState>,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<JobRecord>, StoreError> {
        let rows = sqlx::query(select_jobs!(
            "WHERE tenant_id = $1 AND namespace_id = $2 AND ($3::text IS NULL OR state = $3) \
             AND ($4::uuid IS NULL OR id < $4) ORDER BY id DESC LIMIT $5"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(state.map(JobState::as_str))
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(job_from).collect()
    }

    async fn items(
        &self,
        tenant: TenantId,
        job: Uuid,
        state: Option<ItemState>,
        after: Option<u32>,
        limit: u32,
    ) -> Result<Vec<ItemRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT ordinal, run_id, state, reason, error_code, attempts, next_attempt_at, \
                    first_refused_at, new_run_id, updated_at \
             FROM batch_items \
             WHERE tenant_id = $1 AND job_id = $2 AND ($3::text IS NULL OR state = $3) \
               AND ($4::int IS NULL OR ordinal > $4) \
             ORDER BY ordinal LIMIT $5",
        )
        .bind(tenant.as_uuid())
        .bind(job)
        .bind(state.map(item_state_name))
        .bind(after.map(|n| i32::try_from(n).unwrap_or(i32::MAX)))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(item_from).collect()
    }

    async fn request_cancel(
        &self,
        tenant: TenantId,
        id: Uuid,
        at: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<Option<JobRecord>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM batch_jobs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(state) = state else {
            return Ok(None);
        };
        // a job that is cancelling or over stays as it is: asking twice is asking once
        if matches!(state.as_str(), "SELECTING" | "RUNNING") {
            sqlx::query(
                "UPDATE batch_jobs SET state = 'CANCELLING', next_slot_at = LEAST(next_slot_at, $3) \
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(id)
            .bind(at)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            insert_audit(&mut tx, audit).await?;
        }
        let row = sqlx::query(select_jobs!("WHERE tenant_id = $1 AND id = $2"))
            .bind(tenant.as_uuid())
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        let record = job_from(&row)?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(record))
    }

    async fn due_jobs(&self, shards: &[Shard], limit: u32) -> Result<Vec<DueJob>, StoreError> {
        let shards: Vec<i16> = shards.iter().map(|s| i16::from(s.index())).collect();
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT tenant_id, id FROM batch_jobs \
             WHERE shard = ANY($1) AND state IN ('SELECTING', 'RUNNING', 'CANCELLING') \
               AND next_slot_at <= clock_timestamp() \
             ORDER BY next_slot_at, id LIMIT $2",
        )
        .bind(shards)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(rows
            .into_iter()
            .map(|(tenant, id)| DueJob {
                tenant_id: TenantId::new(tenant),
                id,
            })
            .collect())
    }

    async fn select_round(
        &self,
        tenant: TenantId,
        id: Uuid,
        fence: Option<&ShardFence>,
        page: u32,
    ) -> Result<Selected, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        check_fence(&mut tx, id, fence).await?;
        // The snapshot of this statement against the horizon of the job: every transaction that was
        // open when the job was created has ended once the oldest one still open is not older.
        let job = sqlx::query(
            "SELECT namespace_id, state, select_cursor, selected, max_items, selection_bound_at, selector, \
                    pg_snapshot_xmin(pg_current_snapshot()) > selection_horizon AS ready \
             FROM batch_jobs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(job) = job else {
            return Err(StoreError::MissingReference);
        };
        let state: String = col(&job, "state")?;
        if state != JobState::Selecting.as_str() {
            return Ok(Selected::NotSelecting);
        }
        if !col::<bool>(&job, "ready")? {
            return Ok(Selected::Waiting);
        }
        let namespace: Uuid = col(&job, "namespace_id")?;
        let cursor: Option<Uuid> = col(&job, "select_cursor")?;
        let selected: i32 = col(&job, "selected")?;
        let max_items: i32 = col(&job, "max_items")?;
        let bound: OffsetDateTime = col(&job, "selection_bound_at")?;
        let selector: Value = col(&job, "selector")?;
        let selector = selector_from_json(&selector).map_err(|_| corrupt("batch selector"))?;
        let bind = Bind::of(&selector);
        let take = i64::from(page).min(i64::from(max_items - selected)).max(0);
        let ids: Vec<Uuid> = sqlx::query_scalar(matching_runs!(
            "AND ($12::uuid IS NULL OR r.id > $12) ORDER BY r.id LIMIT $13"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace)
        .bind(bind.workflow)
        .bind(bind.version)
        .bind(&bind.statuses)
        .bind(&bind.origin)
        .bind(bind.schedule)
        .bind(bind.after)
        .bind(bind.before)
        .bind(&bind.ids)
        .bind(bound)
        .bind(cursor)
        .bind(take)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let picked = i32::try_from(ids.len()).unwrap_or(i32::MAX);
        if !ids.is_empty() {
            sqlx::query(
                "INSERT INTO batch_items (tenant_id, job_id, ordinal, run_id, state, updated_at) \
                 SELECT $1, $2, $3 + t.n, t.run_id, 'PENDING', clock_timestamp() \
                 FROM unnest($4::uuid[]) WITH ORDINALITY AS t(run_id, n) \
                 ON CONFLICT (tenant_id, job_id, run_id) DO NOTHING",
            )
            .bind(tenant.as_uuid())
            .bind(id)
            .bind(selected)
            .bind(&ids)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        // the selection is over when a page comes back short or the job has all it asked for
        let over = i64::from(picked) < take || selected + picked >= max_items;
        sqlx::query(
            "UPDATE batch_jobs SET selected = selected + $3, \
                    select_cursor = COALESCE($4, select_cursor), \
                    state = CASE WHEN $5 THEN 'RUNNING' ELSE state END, \
                    started_at = CASE WHEN $5 THEN clock_timestamp() ELSE started_at END \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(picked)
        .bind(ids.last().copied())
        .bind(over)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(Selected::Picked {
            items: u32::try_from(picked).unwrap_or(0),
        })
    }

    async fn next_item(
        &self,
        tenant: TenantId,
        id: Uuid,
        at: OffsetDateTime,
    ) -> Result<NextItem, StoreError> {
        let row = sqlx::query(
            "SELECT ordinal, run_id, state, reason, error_code, attempts, next_attempt_at, \
                    first_refused_at, new_run_id, updated_at \
             FROM batch_items \
             WHERE tenant_id = $1 AND job_id = $2 \
               AND (state = 'PENDING' OR (state = 'DEFERRED' AND next_attempt_at <= $3)) \
             ORDER BY ordinal LIMIT 1",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(at)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        if let Some(row) = row {
            return Ok(NextItem::Item(item_from(&row)?));
        }
        let (selected, ended, deferred): (i32, i32, i32) = sqlx::query_as(
            "SELECT selected, succeeded + skipped + failed + cancelled, deferred FROM batch_jobs \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(if ended == selected && deferred == 0 {
            NextItem::Drained
        } else {
            NextItem::Waiting
        })
    }

    async fn record_item(
        &self,
        tenant: TenantId,
        id: Uuid,
        ordinal: u32,
        outcome: &ItemOutcome,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
    ) -> Result<JobRecord, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        check_fence(&mut tx, id, fence).await?;
        let job = sqlx::query(
            "SELECT state, selected, succeeded + skipped + failed + cancelled AS ended, deferred, \
                    rate_milli, next_slot_at \
             FROM batch_jobs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(job) = job else {
            return Err(StoreError::MissingReference);
        };
        let state: String = col(&job, "state")?;
        if !matches!(state.as_str(), "RUNNING" | "CANCELLING") {
            return Err(StoreError::StateConflict("the batch job is over"));
        }
        let item = sqlx::query(
            "SELECT state, attempts FROM batch_items \
             WHERE tenant_id = $1 AND job_id = $2 AND ordinal = $3 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(item) = item else {
            return Err(StoreError::MissingReference);
        };
        let was: String = col(&item, "state")?;
        let was = item_state_from(&was).map_err(|_| corrupt("batch item state"))?;
        if was.is_terminal() {
            return Err(StoreError::StateConflict("the batch item already ended"));
        }
        // what the item becomes, and what moves in the counters of the job
        let (to, reason, error, new_run, retry_at): (
            ItemState,
            Option<&'static str>,
            Option<&'static str>,
            Option<Uuid>,
            Option<OffsetDateTime>,
        ) = match outcome {
            ItemOutcome::Succeeded { new_run } => {
                (ItemState::Succeeded, None, None, *new_run, None)
            }
            ItemOutcome::Skipped(reason) => {
                (ItemState::Skipped, Some(reason.as_str()), None, None, None)
            }
            ItemOutcome::Failed { code } => (ItemState::Failed, None, Some(*code), None, None),
            ItemOutcome::Deferred { cause, retry_at } => (
                ItemState::Deferred,
                Some(cause.as_str()),
                None,
                None,
                Some(*retry_at),
            ),
        };
        let refusal = to == ItemState::Deferred;
        sqlx::query(
            "UPDATE batch_items SET state = $4, reason = $5, error_code = $6, new_run_id = $7, \
                    attempts = attempts + $8, \
                    first_refused_at = CASE WHEN $9 THEN COALESCE(first_refused_at, $10) ELSE first_refused_at END, \
                    next_attempt_at = $11, updated_at = $10 \
             WHERE tenant_id = $1 AND job_id = $2 AND ordinal = $3",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
        .bind(item_state_name(to))
        .bind(reason)
        .bind(error)
        .bind(new_run)
        .bind(i16::from(refusal))
        .bind(refusal)
        .bind(at)
        .bind(retry_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let delta = |state: ItemState| i32::from(to == state);
        let deferred_delta = i32::from(to == ItemState::Deferred && was == ItemState::Pending)
            - i32::from(to != ItemState::Deferred && was == ItemState::Deferred);
        let rate_milli: i32 = col(&job, "rate_milli")?;
        let slot: OffsetDateTime = col(&job, "next_slot_at")?;
        let slot = next_slot(
            slot,
            at,
            Rate::from_milli(u32::try_from(rate_milli).unwrap_or(1))
                .map_err(|_| corrupt("batch rate"))?,
        );
        let selected: i32 = col(&job, "selected")?;
        let ended: i32 = col::<i32>(&job, "ended")? + i32::from(to.is_terminal());
        let deferred: i32 = col::<i32>(&job, "deferred")? + deferred_delta;
        // a running job whose items have all ended is complete in the same transaction
        let complete = state == "RUNNING" && ended == selected && deferred == 0;
        sqlx::query(
            "UPDATE batch_jobs SET succeeded = succeeded + $3, skipped = skipped + $4, \
                    failed = failed + $5, deferred = deferred + $6, next_slot_at = $7, \
                    state = CASE WHEN $8 THEN 'COMPLETED' ELSE state END, \
                    finished_at = CASE WHEN $8 THEN $9 ELSE finished_at END \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(delta(ItemState::Succeeded))
        .bind(delta(ItemState::Skipped))
        .bind(delta(ItemState::Failed))
        .bind(deferred_delta)
        .bind(slot)
        .bind(complete)
        .bind(at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        if complete {
            insert_audit(&mut tx, finished).await?;
        }
        let row = sqlx::query(select_jobs!("WHERE tenant_id = $1 AND id = $2"))
            .bind(tenant.as_uuid())
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        let record = job_from(&row)?;
        tx.commit().await.map_err(store_error)?;
        Ok(record)
    }

    async fn complete_job(
        &self,
        tenant: TenantId,
        id: Uuid,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        check_fence(&mut tx, id, fence).await?;
        let moved = sqlx::query(
            "UPDATE batch_jobs SET state = 'COMPLETED', finished_at = $3 \
             WHERE tenant_id = $1 AND id = $2 AND state = 'RUNNING' AND deferred = 0 \
               AND succeeded + skipped + failed + cancelled = selected",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if moved == 0 {
            return Err(StoreError::StateConflict("the batch job has items left"));
        }
        insert_audit(&mut tx, finished).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn drain_cancel(
        &self,
        tenant: TenantId,
        id: Uuid,
        limit: u32,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
    ) -> Result<Drained, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        check_fence(&mut tx, id, fence).await?;
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM batch_jobs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(state) = state else {
            return Err(StoreError::MissingReference);
        };
        if state != "CANCELLING" {
            return Ok(Drained {
                cancelled: 0,
                finished: state == "CANCELLED",
            });
        }
        // the next items that have not ended, with how many of them were deferred
        let rows: Vec<(i32, String)> = sqlx::query_as(
            "SELECT ordinal, state FROM batch_items \
             WHERE tenant_id = $1 AND job_id = $2 AND state IN ('PENDING', 'DEFERRED') \
             ORDER BY ordinal LIMIT $3 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        let ordinals: Vec<i32> = rows.iter().map(|(n, _)| *n).collect();
        let was_deferred =
            i32::try_from(rows.iter().filter(|(_, s)| s == "DEFERRED").count()).unwrap_or(i32::MAX);
        let cancelled = i32::try_from(ordinals.len()).unwrap_or(i32::MAX);
        if cancelled > 0 {
            sqlx::query(
                "UPDATE batch_items SET state = 'CANCELLED', updated_at = $4 \
                 WHERE tenant_id = $1 AND job_id = $2 AND ordinal = ANY($3)",
            )
            .bind(tenant.as_uuid())
            .bind(id)
            .bind(&ordinals)
            .bind(at)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            sqlx::query(
                "UPDATE batch_jobs SET cancelled = cancelled + $3, deferred = deferred - $4 \
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(id)
            .bind(cancelled)
            .bind(was_deferred)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        }
        let left: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM batch_items \
             WHERE tenant_id = $1 AND job_id = $2 AND state IN ('PENDING', 'DEFERRED'))",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if !left {
            sqlx::query(
                "UPDATE batch_jobs SET state = 'CANCELLED', finished_at = $3 \
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(id)
            .bind(at)
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            insert_audit(&mut tx, finished).await?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(Drained {
            cancelled: u32::try_from(cancelled).unwrap_or(0),
            finished: !left,
        })
    }

    async fn fail_job(
        &self,
        tenant: TenantId,
        id: Uuid,
        reason: FailureReason,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        check_fence(&mut tx, id, fence).await?;
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM batch_jobs WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(state) = state else {
            return Err(StoreError::MissingReference);
        };
        if !matches!(state.as_str(), "SELECTING" | "RUNNING") {
            return Err(StoreError::StateConflict("the batch job cannot fail now"));
        }
        // what had not ended is cancelled, so the counters equal the outcomes of the items
        let cancelled: i64 = sqlx::query_scalar(
            "WITH moved AS (UPDATE batch_items SET state = 'CANCELLED', updated_at = $3 \
                 WHERE tenant_id = $1 AND job_id = $2 AND state IN ('PENDING', 'DEFERRED') \
                 RETURNING 1) \
             SELECT count(*) FROM moved",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(at)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        // the deferred ones were counted as such: they leave that count when they are cancelled
        sqlx::query(
            "UPDATE batch_jobs SET state = 'FAILED', failure_reason = $3, finished_at = $4, \
                    cancelled = cancelled + $5, deferred = 0 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(reason.as_str())
        .bind(at)
        .bind(i32::try_from(cancelled).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, finished).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn payload(
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

    async fn preview(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        selector: &Selector,
        max_items: u32,
        sample: usize,
    ) -> Result<Preview, StoreError> {
        let bind = Bind::of(selector);
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // a preview is a read that must finish in a bounded time
        sqlx::query("SET LOCAL statement_timeout = '5s'")
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        // one more than the job may take, to know whether the selector matches more
        let found: Vec<Uuid> = sqlx::query_scalar(matching_runs!("ORDER BY r.id LIMIT $12"))
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(bind.workflow)
            .bind(bind.version)
            .bind(&bind.statuses)
            .bind(&bind.origin)
            .bind(bind.schedule)
            .bind(bind.after)
            .bind(bind.before)
            .bind(&bind.ids)
            .bind(now)
            .bind(i64::from(max_items) + 1)
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
        tx.rollback().await.map_err(store_error)?;
        let matched = u32::try_from(found.len()).unwrap_or(u32::MAX);
        Ok(Preview {
            matched: matched.min(max_items.saturating_add(1)),
            capped: matched > max_items,
            sample: found.into_iter().take(sample).collect(),
        })
    }
}

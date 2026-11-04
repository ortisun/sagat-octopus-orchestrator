//! The batch store (feature 018): jobs, their items and the previews that count the runs a selector
//! would pick.
//!
//! This file owns the columns of `batch_jobs` and `batch_items` in the storage classification. It
//! creates, reads, lists and cancels jobs and reads their items; the selection of the runs and the
//! application of the action to each are the executor's statements (`T018-046`).

use sagat_application::batch::{
    ActiveJobLimits, BatchStore, Counts, ItemRecord, JobParams, JobRecord, NewJob, Preview,
    failure_reason_from, item_state_from, item_state_name, selector_from_json, selector_to_json,
};
use sagat_application::definitions::Applied;
use sagat_application::error::StoreError;
use sagat_application::quotas::QuotaLookup;
use sagat_domain::audit::AuditRecord;
use sagat_domain::batch::{Action, ItemState, JobState, Rate, Selector, SelectorKind};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::quota::QuotaResource;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;
use crate::quotas::{effective, ensure_room_for_batch, lock_batch_jobs};

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
        new_run_id: col(row, "new_run_id")?,
        updated_at: col(row, "updated_at")?,
    })
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
        // the snapshot of this very transaction. The selection starts once every transaction that
        // was open now has ended (`pg_snapshot_xmin` has reached the horizon).
        sqlx::query(
            "INSERT INTO batch_jobs \
             (tenant_id, namespace_id, id, action, params, payload_digest, selector, reason, created_by, \
              state, max_items, rate_milli, selection_bound_at, selection_horizon, next_slot_at, \
              created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'SELECTING', $10, $11, clock_timestamp(), \
                     pg_snapshot_xmax(pg_current_snapshot()), clock_timestamp(), $12, $13)",
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
            "SELECT ordinal, run_id, state, reason, error_code, attempts, next_attempt_at, new_run_id, \
                    updated_at \
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
        let (matched, ids): (i64, Option<Vec<Uuid>>) = sqlx::query_as(
            "WITH m AS (SELECT r.id FROM runs r \
                 WHERE r.tenant_id = $1 AND r.namespace_id = $2 AND r.created_at <= clock_timestamp() \
                   AND ($3::uuid IS NULL OR r.workflow_id = $3) \
                   AND ($4::uuid IS NULL OR r.workflow_version_id = $4) \
                   AND (cardinality($5::text[]) = 0 OR r.status = ANY($5)) \
                   AND ($6::text IS NULL OR r.origin = $6) \
                   AND ($7::uuid IS NULL OR r.schedule_id = $7) \
                   AND ($8::timestamptz IS NULL OR r.created_at > $8) \
                   AND ($9::timestamptz IS NULL OR r.created_at < $9) \
                   AND ($10::uuid[] IS NULL OR r.id = ANY($10)) \
                 ORDER BY r.id LIMIT $11) \
             SELECT (SELECT count(*) FROM m), \
                    (SELECT array_agg(s.id ORDER BY s.id) FROM (SELECT id FROM m ORDER BY id LIMIT $12) s)",
        )
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
        .bind(i64::from(max_items) + 1)
        .bind(i64::try_from(sample).unwrap_or(i64::MAX))
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.rollback().await.map_err(store_error)?;
        let matched = u32::try_from(matched).unwrap_or(u32::MAX);
        Ok(Preview {
            matched: matched.min(max_items.saturating_add(1)),
            capped: matched > max_items,
            sample: ids.unwrap_or_default(),
        })
    }
}

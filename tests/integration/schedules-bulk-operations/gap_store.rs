//! A batch store with a gap in it: the real store over the real database, which, the first time its scan of the due jobs
//! returns, makes a job's slot come before it hands the answer back. That is the time between two statements of one round
//! (G2043, the lost wake-up), held where a test can put something in it, with no sleep: the store does the move itself,
//! after the scan has answered and before the caller has seen the answer.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sagat_application::batch::{
    ActiveJobLimits, BatchStore, Drained, DueJobs, ItemOutcome, ItemRecord, JobRecord, NewJob,
    NextItem, Preview, Selected,
};
use sagat_application::definitions::{Applied, Idempotency};
use sagat_application::error::StoreError;
use sagat_application::quotas::QuotaLookup;
use sagat_application::schedules::ShardFence;
use sagat_domain::audit::AuditRecord;
use sagat_domain::batch::{FailureReason, ItemState, JobState, Selector};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::PgBatchStore;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// The real store, and what to do to `job` once, right after the first scan of the due jobs has returned: its slot comes
/// (a millisecond ago, by the clock of the database), as it would have in the time the scan and the next statement take.
#[derive(Clone)]
pub(crate) struct SlotComesBetween {
    inner: PgBatchStore,
    owner: PgPool,
    job: Uuid,
    armed: Arc<AtomicBool>,
}

impl SlotComesBetween {
    pub(crate) fn new(owner: PgPool, job: Uuid) -> Self {
        Self {
            inner: PgBatchStore::new(owner.clone()),
            owner,
            job,
            armed: Arc::new(AtomicBool::new(true)),
        }
    }

    async fn the_slot_comes(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            sqlx::query(
                "UPDATE batch_jobs SET next_slot_at = clock_timestamp() - interval '1 millisecond' WHERE id = $1",
            )
            .bind(self.job)
            .execute(&self.owner)
            .await
            .expect("the slot comes");
        }
    }
}

impl QuotaLookup for SlotComesBetween {
    async fn effective_limit(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> Result<u64, StoreError> {
        self.inner
            .effective_limit(tenant, namespace, resource)
            .await
    }
}

impl BatchStore for SlotComesBetween {
    async fn namespace_exists(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<bool, StoreError> {
        self.inner.namespace_exists(tenant, namespace).await
    }

    async fn create_job(
        &self,
        new: &NewJob,
        limits: ActiveJobLimits,
        audit: &AuditRecord,
        idempotency: &Idempotency,
    ) -> Result<Applied<()>, StoreError> {
        self.inner.create_job(new, limits, audit, idempotency).await
    }

    async fn job(&self, tenant: TenantId, id: Uuid) -> Result<Option<JobRecord>, StoreError> {
        self.inner.job(tenant, id).await
    }

    async fn jobs(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        state: Option<JobState>,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<JobRecord>, StoreError> {
        self.inner
            .jobs(tenant, namespace, state, after, limit)
            .await
    }

    async fn items(
        &self,
        tenant: TenantId,
        job: Uuid,
        state: Option<ItemState>,
        after: Option<u32>,
        limit: u32,
    ) -> Result<Vec<ItemRecord>, StoreError> {
        self.inner.items(tenant, job, state, after, limit).await
    }

    async fn request_cancel(
        &self,
        tenant: TenantId,
        id: Uuid,
        at: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<Option<JobRecord>, StoreError> {
        self.inner.request_cancel(tenant, id, at, audit).await
    }

    async fn ensure_live(
        &self,
        tenant: TenantId,
        id: Uuid,
        ordinal: u32,
        fence: Option<&ShardFence>,
    ) -> Result<(), StoreError> {
        self.inner.ensure_live(tenant, id, ordinal, fence).await
    }

    async fn due_jobs(&self, shards: &[Shard], limit: u32) -> Result<DueJobs, StoreError> {
        let due = self.inner.due_jobs(shards, limit).await;
        self.the_slot_comes().await;
        due
    }

    async fn select_round(
        &self,
        tenant: TenantId,
        id: Uuid,
        fence: Option<&ShardFence>,
        page: u32,
    ) -> Result<Selected, StoreError> {
        self.inner.select_round(tenant, id, fence, page).await
    }

    async fn next_item(
        &self,
        tenant: TenantId,
        id: Uuid,
        at: OffsetDateTime,
    ) -> Result<NextItem, StoreError> {
        self.inner.next_item(tenant, id, at).await
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
        self.inner
            .record_item(tenant, id, ordinal, outcome, at, fence, finished)
            .await
    }

    async fn complete_job(
        &self,
        tenant: TenantId,
        id: Uuid,
        at: OffsetDateTime,
        fence: Option<&ShardFence>,
        finished: &AuditRecord,
    ) -> Result<(), StoreError> {
        self.inner
            .complete_job(tenant, id, at, fence, finished)
            .await
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
        self.inner
            .drain_cancel(tenant, id, limit, at, fence, finished)
            .await
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
        self.inner
            .fail_job(tenant, id, reason, at, fence, finished)
            .await
    }

    async fn payload(
        &self,
        tenant: TenantId,
        digest: &[u8; 32],
    ) -> Result<Option<Vec<u8>>, StoreError> {
        self.inner.payload(tenant, digest).await
    }

    async fn preview(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        selector: &Selector,
        max_items: u32,
        sample: usize,
    ) -> Result<Preview, StoreError> {
        self.inner
            .preview(tenant, namespace, selector, max_items, sample)
            .await
    }
}

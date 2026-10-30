//! The schedule use cases the HTTP layer holds (feature 018): the administration of a schedule
//! ([`ScheduleService`]) and the actions taken on it ([`ScheduleFiring`], which is also what the
//! scheduler's pass runs), behind one trait object.

use sagat_domain::identity::{NamespaceId, Principal};
use sagat_domain::schedules::{Note, Overlap, ScheduleId};

use crate::clock::Clock;
use crate::identity::{IdentityStore, RequestContext};
use crate::quotas::QuotaLookup;
use crate::schedule_firing::{ScheduleFiring, ScheduledStarts, TriggerRequest};
use crate::schedules::{
    ActionView, BackfillRecord, BackfillRequest, ScheduleCalendar, ScheduleDescription,
    ScheduleDocument, ScheduleFuture, ScheduleRecord, ScheduleService, ScheduleStore,
    ScheduleUseCase, decode_document,
};
use uuid::Uuid;

/// The use cases of schedules over one store, one calendar and one run service.
pub struct ScheduleApi<S, K, C, I, R> {
    service: ScheduleService<S, K, C>,
    firing: ScheduleFiring<S, K, C, I, R>,
}

impl<S, K, C, I, R> ScheduleApi<S, K, C, I, R> {
    #[must_use]
    pub const fn new(
        service: ScheduleService<S, K, C>,
        firing: ScheduleFiring<S, K, C, I, R>,
    ) -> Self {
        Self { service, firing }
    }
}

impl<S, K, C, I, R> ScheduleUseCase for ScheduleApi<S, K, C, I, R>
where
    S: ScheduleStore + QuotaLookup + 'static,
    K: ScheduleCalendar + 'static,
    C: Clock + 'static,
    I: IdentityStore + 'static,
    R: ScheduledStarts + 'static,
{
    fn namespace_of<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
    ) -> ScheduleFuture<'a, NamespaceId> {
        Box::pin(self.service.namespace_of(principal, id))
    }

    fn create<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        document: ScheduleDocument,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ScheduleRecord> {
        Box::pin(async move {
            let write = decode_document(document, self.service.ceilings())?;
            self.service.create(principal, namespace, write, ctx).await
        })
    }

    fn describe<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        next: u32,
        recent: u32,
    ) -> ScheduleFuture<'a, ScheduleDescription> {
        Box::pin(self.service.describe(principal, id, next, recent))
    }

    fn list<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        after: Option<ScheduleId>,
        limit: u32,
    ) -> ScheduleFuture<'a, Vec<ScheduleRecord>> {
        Box::pin(self.service.list(principal, namespace, after, limit))
    }

    fn update<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        document: ScheduleDocument,
        expected_revision: u32,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ScheduleRecord> {
        Box::pin(async move {
            let write = decode_document(document, self.service.ceilings())?;
            self.service
                .update(principal, id, write, expected_revision, ctx)
                .await
        })
    }

    fn delete<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        expected_revision: u32,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ()> {
        Box::pin(
            self.service
                .delete(principal, id, Some(expected_revision), ctx),
        )
    }

    fn pause<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        note: Note,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ScheduleRecord> {
        Box::pin(self.service.pause(principal, id, note, ctx))
    }

    fn resume<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        note: Note,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ScheduleRecord> {
        Box::pin(self.service.resume(principal, id, note, ctx))
    }

    fn create_backfill<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        request: BackfillRequest,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, BackfillRecord> {
        Box::pin(async move {
            self.service
                .create_backfill(principal, id, &request, ctx)
                .await
        })
    }

    fn backfills<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
    ) -> ScheduleFuture<'a, Vec<BackfillRecord>> {
        Box::pin(self.service.backfills(principal, id))
    }

    fn cancel_backfill<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        backfill: Uuid,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, BackfillRecord> {
        Box::pin(self.service.cancel_backfill(principal, id, backfill, ctx))
    }

    fn trigger<'a>(
        &'a self,
        principal: &'a Principal,
        id: ScheduleId,
        overlap: Option<Overlap>,
        key: &'a str,
        ctx: &'a RequestContext,
    ) -> ScheduleFuture<'a, ActionView> {
        Box::pin(async move {
            self.firing
                .trigger(principal, id, &TriggerRequest { overlap, key }, ctx)
                .await
        })
    }
}

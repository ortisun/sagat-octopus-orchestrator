//! The batch use cases the HTTP layer holds (feature 018): the administration of a job
//! ([`BatchService`]) behind one trait object, so that the routes need no new generic parameter.

use std::future::Future;
use std::pin::Pin;

use sagat_domain::batch::Selector;
use sagat_domain::batch::{ItemState, JobState};
use sagat_domain::identity::{NamespaceId, Principal};
use uuid::Uuid;

use crate::batch::{BatchService, BatchStore, BatchWrite, ItemRecord, JobRecord, Preview};
use crate::clock::Clock;
use crate::error::AppError;
use crate::identity::{Page, PageRequest, RequestContext};

/// The future of a batch use case.
pub type BatchFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

/// What the routes of batch jobs need, object-safe.
pub trait BatchUseCase: Send + Sync {
    /// The namespace of a job of the caller's tenant: a job of any other tenant is as absent as one
    /// that never existed. Needs no permission, so a route can place the job before it authorizes.
    fn namespace_of<'a>(
        &'a self,
        principal: &'a Principal,
        id: Uuid,
    ) -> BatchFuture<'a, NamespaceId>;

    fn preview<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        selector: &'a Selector,
        max_items: Option<u32>,
    ) -> BatchFuture<'a, Preview>;

    fn create<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        write: BatchWrite,
        key: &'a str,
        ctx: &'a RequestContext,
    ) -> BatchFuture<'a, JobRecord>;

    fn get<'a>(&'a self, principal: &'a Principal, id: Uuid) -> BatchFuture<'a, JobRecord>;

    fn list<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        state: Option<JobState>,
        page: &'a PageRequest,
    ) -> BatchFuture<'a, Page<JobRecord>>;

    fn items<'a>(
        &'a self,
        principal: &'a Principal,
        id: Uuid,
        state: Option<ItemState>,
        page: &'a PageRequest,
    ) -> BatchFuture<'a, Page<ItemRecord>>;

    fn cancel<'a>(
        &'a self,
        principal: &'a Principal,
        id: Uuid,
        reason: &'a str,
        ctx: &'a RequestContext,
    ) -> BatchFuture<'a, JobRecord>;
}

impl<S, C> BatchUseCase for BatchService<S, C>
where
    S: BatchStore + 'static,
    C: Clock + 'static,
{
    fn namespace_of<'a>(
        &'a self,
        principal: &'a Principal,
        id: Uuid,
    ) -> BatchFuture<'a, NamespaceId> {
        Box::pin(self.namespace_of(principal, id))
    }

    fn preview<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        selector: &'a Selector,
        max_items: Option<u32>,
    ) -> BatchFuture<'a, Preview> {
        Box::pin(self.preview(principal, namespace, selector, max_items))
    }

    fn create<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        write: BatchWrite,
        key: &'a str,
        ctx: &'a RequestContext,
    ) -> BatchFuture<'a, JobRecord> {
        Box::pin(self.create(principal, namespace, write, key, ctx))
    }

    fn get<'a>(&'a self, principal: &'a Principal, id: Uuid) -> BatchFuture<'a, JobRecord> {
        Box::pin(self.get(principal, id))
    }

    fn list<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        state: Option<JobState>,
        page: &'a PageRequest,
    ) -> BatchFuture<'a, Page<JobRecord>> {
        Box::pin(self.list(principal, namespace, state, page))
    }

    fn items<'a>(
        &'a self,
        principal: &'a Principal,
        id: Uuid,
        state: Option<ItemState>,
        page: &'a PageRequest,
    ) -> BatchFuture<'a, Page<ItemRecord>> {
        Box::pin(self.items(principal, id, state, page))
    }

    fn cancel<'a>(
        &'a self,
        principal: &'a Principal,
        id: Uuid,
        reason: &'a str,
        ctx: &'a RequestContext,
    ) -> BatchFuture<'a, JobRecord> {
        Box::pin(self.cancel(principal, id, reason, ctx))
    }
}

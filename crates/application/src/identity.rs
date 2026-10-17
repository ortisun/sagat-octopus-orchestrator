//! Identity use cases: authenticate a caller, manage workspaces and namespaces.
//!
//! Every use case receives an authenticated [`Principal`] and derives the tenant from it; the
//! storage port is always called with that tenant, so a query can never reach another tenant's rows
//! (US04, NFR-001-SEC). Authorization runs before any storage access.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{
    Environment, Membership, Namespace, NamespaceId, Permission, Principal, Slug, Subject, Tenant,
    TenantId, Workspace, WorkspaceId,
};

use crate::clock::Clock;
use crate::definitions::{Applied, Idempotency, fingerprint};
use crate::error::{AccessDenial, AppError, StoreError};

/// Default and maximum page sizes of list operations.
pub const DEFAULT_PAGE_SIZE: u32 = 50;
pub const MAX_PAGE_SIZE: u32 = 200;

/// Correlation data of the current request, copied into audit records.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// W3C trace id (32 lowercase hex characters) of the request, when known.
    pub trace_id: Option<String>,
    /// The span of the request itself (16 lowercase hex characters), when it has one: what the work
    /// the request starts hangs from in the trace.
    pub span_id: Option<String>,
}

impl RequestContext {
    /// Where the work this request starts belongs in a trace, when the request has one.
    #[must_use]
    pub fn link(&self) -> Option<TraceLink> {
        self.trace_id.as_ref().map(|trace_id| TraceLink {
            trace_id: trace_id.clone(),
            parent_span: self.span_id.clone(),
        })
    }
}

/// Where a piece of work belongs in a trace: the trace, and the span it hangs from. Kept with an
/// event admitted to the inbox so the run it starts continues the trace of whoever published it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceLink {
    pub trace_id: String,
    pub parent_span: Option<String>,
}

/// Cursor pagination input. The cursor is opaque to clients.
#[derive(Debug, Clone, Default)]
pub struct PageRequest {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

impl PageRequest {
    /// Page size clamped to `1..=MAX_PAGE_SIZE`.
    #[must_use]
    pub fn effective_limit(&self) -> u32 {
        self.limit
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE)
    }
}

/// One page of results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Pass back as `cursor` to fetch the next page; `None` on the last page.
    pub next_cursor: Option<String>,
}

/// An authenticated caller together with the tenant it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub principal: Principal,
    pub tenant: Tenant,
}

/// Storage port for the identity context. Implemented by `sagat-infra-postgres`.
///
/// Every method that reads or writes tenant data takes the tenant explicitly; adapters MUST apply
/// it as a filter on every statement.
pub trait IdentityStore: Send + Sync {
    /// Resolves a tenant by slug. The only lookup that is not tenant-scoped.
    fn find_tenant_by_slug(
        &self,
        slug: &Slug,
    ) -> impl Future<Output = Result<Option<Tenant>, StoreError>> + Send;

    fn find_membership(
        &self,
        tenant: TenantId,
        subject: &Subject,
    ) -> impl Future<Output = Result<Option<Membership>, StoreError>> + Send;

    /// Persists the workspace and its audit record in one transaction.
    fn create_workspace(
        &self,
        workspace: &Workspace,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Like [`Self::create_workspace`], recording `idempotency` in the same transaction. A request
    /// that repeats a recorded one (same key, same fingerprint) writes nothing and is
    /// [`Applied::Replayed`] with the id it created; the same key with a different request is
    /// [`StoreError::IdempotencyKeyReused`]. Racing requests with one key create exactly one.
    fn create_workspace_idempotent(
        &self,
        workspace: &Workspace,
        audit: &AuditRecord,
        idempotency: &Idempotency,
    ) -> impl Future<Output = Result<Applied<()>, StoreError>> + Send;

    fn get_workspace(
        &self,
        tenant: TenantId,
        id: WorkspaceId,
    ) -> impl Future<Output = Result<Option<Workspace>, StoreError>> + Send;

    /// Workspaces of `tenant` with `id > after`, ascending by id, at most `limit`.
    fn list_workspaces(
        &self,
        tenant: TenantId,
        after: Option<WorkspaceId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Workspace>, StoreError>> + Send;

    /// Persists the namespace and its audit record in one transaction.
    fn create_namespace(
        &self,
        namespace: &Namespace,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The namespace counterpart of [`Self::create_workspace_idempotent`].
    fn create_namespace_idempotent(
        &self,
        namespace: &Namespace,
        audit: &AuditRecord,
        idempotency: &Idempotency,
    ) -> impl Future<Output = Result<Applied<()>, StoreError>> + Send;

    fn get_namespace(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> impl Future<Output = Result<Option<Namespace>, StoreError>> + Send;

    /// Namespaces of `tenant` with `id > after`, ascending by id, at most `limit`.
    fn list_namespaces(
        &self,
        tenant: TenantId,
        after: Option<NamespaceId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Namespace>, StoreError>> + Send;

    /// Whether the workspace exists in a tenant *other than* `tenant`. Used only to decide whether
    /// a miss deserves a cross-tenant audit record; the answer is never returned to clients.
    fn workspace_exists_outside(
        &self,
        tenant: TenantId,
        id: WorkspaceId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Same as [`Self::workspace_exists_outside`] for namespaces.
    fn namespace_exists_outside(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Records that a caller tried to reach a resource of another tenant.
    ///
    /// Best effort by contract: implementations MUST NOT make the caller wait for the write (the
    /// response must take the same time whether or not the identifier exists in another tenant),
    /// MUST collapse identical events from the same actor within a window (an attacker repeating a
    /// request must not grow the trail), and MUST bound their own resource use. A dropped event is
    /// logged. The denial itself never depends on this call.
    fn record_denial(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Appends one record of a permission denial (`403`), always: no collapsing of repeats and no
    /// dropping under load. The caller awaits it; its volume is bounded by the per-subject request
    /// budget that every authenticated call is charged to before it gets this far.
    fn record_permission_denial(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Cheap round trip used by readiness probes.
    fn ping(&self) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Identity use cases over a storage port and a clock.
#[derive(Debug, Clone)]
pub struct IdentityService<S, C> {
    store: S,
    clock: C,
}

impl<S: IdentityStore, C: Clock> IdentityService<S, C> {
    pub const fn new(store: S, clock: C) -> Self {
        Self { store, clock }
    }

    /// Readiness probe: succeeds when storage answers.
    ///
    /// # Errors
    /// [`AppError::Unavailable`] when storage cannot be reached.
    pub async fn ping(&self) -> Result<(), AppError> {
        Ok(self.store.ping().await?)
    }

    /// Turns the claims of an already *verified* token into a [`Session`].
    ///
    /// Unknown tenant, malformed values and a missing membership all yield the same
    /// [`AppError::Unauthenticated`]; a storage outage is reported as such (never as a denial).
    ///
    /// # Errors
    /// [`AppError::Unauthenticated`], [`AppError::Unavailable`] or [`AppError::Internal`].
    pub async fn authenticate(
        &self,
        tenant_slug: &str,
        subject: &str,
    ) -> Result<Session, AppError> {
        let (Ok(slug), Ok(subject)) = (Slug::parse(tenant_slug), Subject::parse(subject)) else {
            return Err(AppError::Unauthenticated);
        };
        let Some(tenant) = self.store.find_tenant_by_slug(&slug).await? else {
            return Err(AppError::Unauthenticated);
        };
        let Some(membership) = self.store.find_membership(tenant.id(), &subject).await? else {
            return Err(AppError::Unauthenticated);
        };
        Ok(Session {
            principal: membership.principal(),
            tenant,
        })
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `workspace:create`, [`AppError::Invalid`] for a bad slug,
    /// [`AppError::Conflict`] when the slug is taken inside the tenant.
    pub async fn create_workspace(
        &self,
        principal: &Principal,
        slug: &str,
        ctx: &RequestContext,
    ) -> Result<Workspace, AppError> {
        self.create_workspace_as(principal, slug, None, ctx).await
    }

    /// [`Self::create_workspace`] that is safe to retry: a repeat with the same `key` and slug
    /// returns the workspace the first call created (nothing is written, nothing is audited again).
    ///
    /// # Errors
    /// As [`Self::create_workspace`]; [`AppError::IdempotencyKeyReused`] when `key` was already
    /// used, in this tenant, for a different workspace.
    pub async fn create_workspace_idempotent(
        &self,
        principal: &Principal,
        slug: &str,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<Workspace, AppError> {
        self.create_workspace_as(principal, slug, Some(key), ctx)
            .await
    }

    async fn create_workspace_as(
        &self,
        principal: &Principal,
        slug: &str,
        key: Option<&str>,
        ctx: &RequestContext,
    ) -> Result<Workspace, AppError> {
        principal.authorize(Permission::WorkspaceCreate)?;
        let slug = Slug::parse(slug)?;
        let now = self.clock.now();
        let workspace = Workspace::create(principal, WorkspaceId::generate(), slug, now)?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::WorkspaceCreated,
            ResourceType::Workspace,
            workspace.id().to_string(),
            ctx.trace_id.clone(),
            now,
        );
        let Some(key) = key else {
            self.store.create_workspace(&workspace, &audit).await?;
            return Ok(workspace);
        };
        let idempotency = Idempotency {
            scope: "workspace.create".to_owned(),
            key: key.to_owned(),
            fingerprint: fingerprint(&[workspace.slug().as_str().as_bytes()]),
        };
        match self
            .store
            .create_workspace_idempotent(&workspace, &audit, &idempotency)
            .await?
        {
            Applied::Done(()) => Ok(workspace),
            Applied::Replayed(id) => {
                let id = id.parse::<WorkspaceId>().map_err(|_| AppError::Internal)?;
                self.store
                    .get_workspace(principal.tenant_id(), id)
                    .await?
                    .ok_or(AppError::Internal)
            }
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `workspace:read`; [`AppError::NotFound`] when the workspace
    /// does not exist *in the caller's tenant* (a miss on another tenant's workspace is audited).
    pub async fn get_workspace(
        &self,
        principal: &Principal,
        id: WorkspaceId,
        ctx: &RequestContext,
    ) -> Result<Workspace, AppError> {
        principal.authorize(Permission::WorkspaceRead)?;
        match self.store.get_workspace(principal.tenant_id(), id).await? {
            Some(workspace) => Ok(workspace),
            None => Err(self.missing_workspace(principal, id, ctx).await),
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `workspace:read`; [`AppError::Invalid`] for a bad cursor.
    pub async fn list_workspaces(
        &self,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<Workspace>, AppError> {
        principal.authorize(Permission::WorkspaceRead)?;
        let after = page
            .cursor
            .as_deref()
            .map(str::parse::<WorkspaceId>)
            .transpose()?;
        let limit = page.effective_limit();
        let rows = self
            .store
            .list_workspaces(principal.tenant_id(), after, limit + 1)
            .await?;
        Ok(paginate(rows, limit, |w| w.id().to_string()))
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `namespace:create`; [`AppError::Invalid`] for a bad slug or
    /// environment; [`AppError::NotFound`] when the workspace is not in the caller's tenant;
    /// [`AppError::Conflict`] when the slug is taken inside the tenant.
    pub async fn create_namespace(
        &self,
        principal: &Principal,
        workspace_id: WorkspaceId,
        slug: &str,
        environment: &str,
        ctx: &RequestContext,
    ) -> Result<Namespace, AppError> {
        self.create_namespace_as(principal, workspace_id, slug, environment, None, ctx)
            .await
    }

    /// [`Self::create_namespace`] that is safe to retry; see [`Self::create_workspace_idempotent`].
    ///
    /// # Errors
    /// As [`Self::create_namespace`]; [`AppError::IdempotencyKeyReused`] when `key` was already
    /// used, in this tenant and workspace, for a different namespace or environment.
    pub async fn create_namespace_idempotent(
        &self,
        principal: &Principal,
        workspace_id: WorkspaceId,
        slug: &str,
        environment: &str,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<Namespace, AppError> {
        self.create_namespace_as(principal, workspace_id, slug, environment, Some(key), ctx)
            .await
    }

    async fn create_namespace_as(
        &self,
        principal: &Principal,
        workspace_id: WorkspaceId,
        slug: &str,
        environment: &str,
        key: Option<&str>,
        ctx: &RequestContext,
    ) -> Result<Namespace, AppError> {
        principal.authorize(Permission::NamespaceCreate)?;
        let slug = Slug::parse(slug)?;
        let environment = environment.parse::<Environment>()?;
        let Some(workspace) = self
            .store
            .get_workspace(principal.tenant_id(), workspace_id)
            .await?
        else {
            return Err(self.missing_workspace(principal, workspace_id, ctx).await);
        };
        let now = self.clock.now();
        let namespace = Namespace::create(
            principal,
            &workspace,
            NamespaceId::generate(),
            slug,
            environment,
            now,
        )?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::NamespaceCreated,
            ResourceType::Namespace,
            namespace.id().to_string(),
            ctx.trace_id.clone(),
            now,
        );
        let Some(key) = key else {
            self.store.create_namespace(&namespace, &audit).await?;
            return Ok(namespace);
        };
        let idempotency = Idempotency {
            scope: format!("namespace.create:{workspace_id}"),
            key: key.to_owned(),
            fingerprint: fingerprint(&[
                namespace.slug().as_str().as_bytes(),
                environment.as_str().as_bytes(),
            ]),
        };
        match self
            .store
            .create_namespace_idempotent(&namespace, &audit, &idempotency)
            .await?
        {
            Applied::Done(()) => Ok(namespace),
            Applied::Replayed(id) => {
                let id = id.parse::<NamespaceId>().map_err(|_| AppError::Internal)?;
                self.store
                    .get_namespace(principal.tenant_id(), id)
                    .await?
                    .ok_or(AppError::Internal)
            }
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `namespace:read`; [`AppError::NotFound`] when the namespace
    /// does not exist *in the caller's tenant* (a miss on another tenant's namespace is audited).
    pub async fn get_namespace(
        &self,
        principal: &Principal,
        id: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Namespace, AppError> {
        principal.authorize(Permission::NamespaceRead)?;
        match self.store.get_namespace(principal.tenant_id(), id).await? {
            Some(namespace) => Ok(namespace),
            None => Err(self.missing_namespace(principal, id, ctx).await),
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `namespace:read`; [`AppError::Invalid`] for a bad cursor.
    pub async fn list_namespaces(
        &self,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<Namespace>, AppError> {
        principal.authorize(Permission::NamespaceRead)?;
        let after = page
            .cursor
            .as_deref()
            .map(str::parse::<NamespaceId>)
            .transpose()?;
        let limit = page.effective_limit();
        let rows = self
            .store
            .list_namespaces(principal.tenant_id(), after, limit + 1)
            .await?;
        Ok(paginate(rows, limit, |n| n.id().to_string()))
    }

    /// Records that `principal` was refused a call with a `403` / `PERMISSION_DENIED`, naming the
    /// call by its route template or RPC name and the permission (or refusal code), never a path,
    /// a query, a body or an identifier the caller chose (gap G9, decision of 2026-10-16).
    ///
    /// # Errors
    /// A store failure. The caller is refused whatever the outcome; it logs and counts a failure.
    pub async fn audit_access_denied(
        &self,
        principal: &Principal,
        call: &str,
        denial: AccessDenial,
        ctx: &RequestContext,
    ) -> Result<(), StoreError> {
        let record = AuditRecord::by(
            principal,
            AuditAction::PermissionDenied,
            ResourceType::Endpoint,
            call,
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(denial.reason());
        self.store.record_permission_denial(&record).await
    }

    async fn missing_workspace(
        &self,
        principal: &Principal,
        id: WorkspaceId,
        ctx: &RequestContext,
    ) -> AppError {
        if let Ok(true) = self
            .store
            .workspace_exists_outside(principal.tenant_id(), id)
            .await
        {
            self.audit_cross_tenant(principal, ResourceType::Workspace, id.to_string(), ctx)
                .await;
        }
        AppError::NotFound
    }

    async fn missing_namespace(
        &self,
        principal: &Principal,
        id: NamespaceId,
        ctx: &RequestContext,
    ) -> AppError {
        if let Ok(true) = self
            .store
            .namespace_exists_outside(principal.tenant_id(), id)
            .await
        {
            self.audit_cross_tenant(principal, ResourceType::Namespace, id.to_string(), ctx)
                .await;
        }
        AppError::NotFound
    }

    /// Records an attempt to reach another tenant's resource. The denial never depends on this
    /// succeeding: a failure to write the record is logged and the caller still gets "not found".
    async fn audit_cross_tenant(
        &self,
        principal: &Principal,
        resource: ResourceType,
        resource_id: String,
        ctx: &RequestContext,
    ) {
        let record = AuditRecord::by(
            principal,
            AuditAction::CrossTenantAccessDenied,
            resource,
            resource_id,
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason("resource not accessible from this tenant");
        if let Err(error) = self.store.record_denial(&record).await {
            tracing::error!(%error, action = %record.action(), "failed to record audit entry");
        }
    }
}

/// Trims the extra row fetched to detect a next page and derives the cursor.
fn paginate<T>(mut rows: Vec<T>, limit: u32, id_of: impl Fn(&T) -> String) -> Page<T> {
    let limit = limit as usize;
    if rows.len() > limit {
        rows.truncate(limit);
        let next_cursor = rows.last().map(id_of);
        Page {
            items: rows,
            next_cursor,
        }
    } else {
        Page {
            items: rows,
            next_cursor: None,
        }
    }
}

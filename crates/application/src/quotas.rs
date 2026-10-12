//! Quotas: reading the limits in force and changing them (FR-009-006, US04).
//!
//! *Enforcing* a quota is not done here. The stores that create the things a quota counts (runs,
//! workflows, worker sessions, secrets) take the limit from [`QuotaLookup`] and refuse the creation
//! with `StoreError::QuotaExceeded` from inside the transaction that would create it, under a lock
//! on the counted scope, so two replicas racing for the last place cannot both get it. This service
//! is the administrative side: it shows the limit and the usage and lets an administrator change it.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Permission, Principal, TenantId};
use sagat_domain::quota::{self, QuotaError, QuotaResource, QuotaScope, QuotaSource};

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;

/// The limit in force for a resource, which every creating store consults.
///
/// `namespace` is `None` for a resource counted across the tenant.
pub trait QuotaLookup: Send + Sync {
    fn effective_limit(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;
}

/// The limits a run is created under, resolved just before the creation and judged by the store, in
/// the transaction that inserts the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimits {
    /// Runs that have not finished in the run's namespace.
    pub namespace: u64,
    /// Runs that have not finished in the whole tenant.
    pub tenant: u64,
}

impl RunLimits {
    /// No limit in practice; for tests of other things.
    pub const UNLIMITED: Self = Self {
        namespace: quota::MAX_LIMIT,
        tenant: quota::MAX_LIMIT,
    };

    /// The limits in force for a run of `namespace`.
    ///
    /// # Errors
    /// [`StoreError`] when a limit cannot be read.
    pub async fn of<Q: QuotaLookup>(
        store: &Q,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            namespace: store
                .effective_limit(tenant, Some(namespace), QuotaResource::ActiveRuns)
                .await?,
            tenant: store
                .effective_limit(tenant, None, QuotaResource::TenantActiveRuns)
                .await?,
        })
    }
}

/// A limit somebody set, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredLimit {
    pub resource: QuotaResource,
    pub value: u64,
}

/// Storage port of the quota administration. Implemented by `sagat-infra-postgres`.
pub trait QuotaStore: QuotaLookup + NamespaceDirectory {
    /// What was set at one level: `namespace == None` is the tenant-wide level.
    fn settings(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
    ) -> impl Future<Output = Result<Vec<StoredLimit>, StoreError>> + Send;

    /// The ceilings an operator granted the tenant; a resource without one has the platform default.
    fn ceilings(
        &self,
        tenant: TenantId,
    ) -> impl Future<Output = Result<Vec<StoredLimit>, StoreError>> + Send;

    /// Sets (`Some`) or withdraws (`None`) the weight of a tenant in the scheduler's passes
    /// (`FR-010-002`), with its audit record, in one transaction. Reached only from the operator's
    /// command line.
    fn put_scheduling_weight(
        &self,
        tenant: TenantId,
        weight: Option<u16>,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Grants (`Some`) or withdraws (`None`) a ceiling, with its audit record, in one transaction.
    /// Reached only from the operator's command line, never from the API.
    fn put_ceiling(
        &self,
        tenant: TenantId,
        resource: QuotaResource,
        value: Option<u64>,
        granted_by: &str,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Stores (`Some`) or removes (`None`) a setting, with its audit record, in one transaction.
    fn put_setting(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
        value: Option<u64>,
        changed_by: &str,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// How much of the resource is used: in the namespace for a namespace-scoped resource, in the
    /// tenant for a tenant-scoped one (`namespace` is ignored for those).
    fn usage(
        &self,
        tenant: TenantId,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;
}

/// One line of the quota view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaView {
    pub resource: QuotaResource,
    pub limit: u64,
    pub source: QuotaSource,
    /// The most the tenant may set for this resource (and the most that can be in force).
    pub ceiling: u64,
    /// `None` when the view is the tenant-wide default of a resource that is counted per namespace.
    pub used: Option<u64>,
}

/// Reads and changes quotas.
pub struct QuotaService<S, C> {
    store: S,
    clock: C,
}

impl<S, C> QuotaService<S, C>
where
    S: QuotaStore,
    C: Clock,
{
    #[must_use]
    pub const fn new(store: S, clock: C) -> Self {
        Self { store, clock }
    }

    /// The limits in force and their use, for the tenant (`namespace == None`) or one namespace.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`] for a namespace the tenant does not have
    /// (including one that belongs to another tenant, which is audited).
    pub async fn list(
        &self,
        principal: &Principal,
        namespace: Option<NamespaceId>,
        ctx: &RequestContext,
    ) -> Result<Vec<QuotaView>, AppError> {
        principal.authorize(Permission::QuotaRead)?;
        let tenant = principal.tenant_id();
        self.check_namespace(principal, namespace, ctx).await?;
        let tenant_level = self.store.settings(tenant, None).await?;
        let granted = self.store.ceilings(tenant).await?;
        let namespace_level = match namespace {
            Some(id) => self.store.settings(tenant, Some(id)).await?,
            None => Vec::new(),
        };
        let setting = |rows: &[StoredLimit], resource| {
            rows.iter()
                .find(|r| r.resource == resource)
                .map(|r| r.value)
        };
        let mut views = Vec::new();
        for resource in QuotaResource::ALL {
            // A namespace view shows what is counted there; a tenant view shows everything, but a
            // per-namespace resource has no single usage at that level.
            if namespace.is_some() && resource.scope() == QuotaScope::Tenant {
                continue;
            }
            let ceiling = quota::ceiling(resource, setting(&granted, resource));
            let (limit, source) = quota::resolve(
                resource,
                setting(&namespace_level, resource),
                setting(&tenant_level, resource),
                ceiling,
            );
            let used = match (resource.scope(), namespace) {
                (QuotaScope::Tenant, _) | (QuotaScope::Namespace, Some(_)) => {
                    Some(self.store.usage(tenant, namespace, resource).await?)
                }
                (QuotaScope::Namespace, None) => None,
            };
            views.push(QuotaView {
                resource,
                limit,
                source,
                ceiling,
                used,
            });
        }
        Ok(views)
    }

    /// Sets the limit of `resource` at the tenant level or for one namespace.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`]; [`AppError::NotAllowed`] for a value out of
    /// range or a namespace-level value for a tenant-wide resource.
    pub async fn set(
        &self,
        principal: &Principal,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
        value: u64,
        ctx: &RequestContext,
    ) -> Result<QuotaView, AppError> {
        principal.authorize(Permission::QuotaManage)?;
        self.check_namespace(principal, namespace, ctx).await?;
        let granted = self
            .store
            .ceilings(principal.tenant_id())
            .await?
            .into_iter()
            .find(|c| c.resource == resource)
            .map(|c| c.value);
        let ceiling = quota::ceiling(resource, granted);
        let value =
            quota::validate(resource, namespace.is_some(), value, ceiling).map_err(invalid)?;
        self.write(principal, namespace, resource, Some(value), ctx)
            .await?;
        self.view(principal, namespace, resource).await
    }

    /// Removes a setting, so the next level (or the default) applies again.
    ///
    /// # Errors
    /// As [`Self::set`].
    pub async fn clear(
        &self,
        principal: &Principal,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
        ctx: &RequestContext,
    ) -> Result<QuotaView, AppError> {
        principal.authorize(Permission::QuotaManage)?;
        self.check_namespace(principal, namespace, ctx).await?;
        self.write(principal, namespace, resource, None, ctx)
            .await?;
        self.view(principal, namespace, resource).await
    }

    async fn write(
        &self,
        principal: &Principal,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
        value: Option<u64>,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        let target = namespace.map_or_else(|| "tenant".to_owned(), |n| n.to_string());
        let reason = match value {
            Some(v) => format!("{resource} = {v}"),
            None => format!("{resource} reset"),
        };
        let audit = AuditRecord::by(
            principal,
            AuditAction::QuotaChanged,
            ResourceType::Quota,
            target,
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(reason);
        self.store
            .put_setting(
                principal.tenant_id(),
                namespace,
                resource,
                value,
                principal.subject().as_str(),
                &audit,
            )
            .await?;
        Ok(())
    }

    async fn view(
        &self,
        principal: &Principal,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> Result<QuotaView, AppError> {
        let tenant = principal.tenant_id();
        let ceiling = quota::ceiling(
            resource,
            self.store
                .ceilings(tenant)
                .await?
                .into_iter()
                .find(|c| c.resource == resource)
                .map(|c| c.value),
        );
        let (limit, source) = quota::resolve(
            resource,
            match namespace {
                Some(id) => self
                    .store
                    .settings(tenant, Some(id))
                    .await?
                    .into_iter()
                    .find(|r| r.resource == resource)
                    .map(|r| r.value),
                None => None,
            },
            self.store
                .settings(tenant, None)
                .await?
                .into_iter()
                .find(|r| r.resource == resource)
                .map(|r| r.value),
            ceiling,
        );
        let used = match (resource.scope(), namespace) {
            (QuotaScope::Tenant, _) | (QuotaScope::Namespace, Some(_)) => {
                Some(self.store.usage(tenant, namespace, resource).await?)
            }
            (QuotaScope::Namespace, None) => None,
        };
        Ok(QuotaView {
            resource,
            limit,
            source,
            ceiling,
            used,
        })
    }

    /// Grants the tenant a ceiling for `resource` (`Some`), or withdraws what was granted so the
    /// platform default applies again (`None`). For the operator's command line: a ceiling is the
    /// platform's decision about a tenant and no tenant principal can make it.
    ///
    /// # Errors
    /// [`AppError::NotAllowed`] for a value out of range; [`AppError::NotFound`] for a tenant that
    /// does not exist.
    pub async fn grant_ceiling(
        &self,
        tenant: TenantId,
        resource: QuotaResource,
        value: Option<u64>,
    ) -> Result<(), AppError> {
        let value = value
            .map(quota::validate_ceiling)
            .transpose()
            .map_err(invalid)?;
        let reason = match value {
            Some(v) => format!("ceiling of {resource} = {v}"),
            None => format!("ceiling of {resource} withdrawn"),
        };
        let audit = AuditRecord::operator(
            tenant,
            AuditAction::QuotaChanged,
            ResourceType::Quota,
            "tenant",
            reason,
            self.clock.now(),
        );
        self.store
            .put_ceiling(
                tenant,
                resource,
                value,
                sagat_domain::identity::Subject::operator().as_str(),
                &audit,
            )
            .await?;
        Ok(())
    }

    /// Sets the weight of a tenant in the scheduler's passes, or withdraws it (the default is 1),
    /// and audits it. A tenant of weight `w` gets `w` times the share of a pass that a tenant of
    /// weight 1 gets, when both have more work due than a pass takes; nothing changes for a tenant
    /// that has less.
    ///
    /// # Errors
    /// [`AppError::NotAllowed`] for a weight outside 1 to 16; [`AppError::NotFound`] for a tenant
    /// that does not exist.
    pub async fn set_scheduling_weight(
        &self,
        tenant: TenantId,
        weight: Option<u16>,
    ) -> Result<(), AppError> {
        if weight.is_some_and(|w| !(1..=sagat_domain::sharding::MAX_WEIGHT).contains(&w)) {
            return Err(AppError::NotAllowed("a scheduling weight is 1 to 16"));
        }
        let reason = match weight {
            Some(w) => format!("scheduling weight = {w}"),
            None => "scheduling weight withdrawn".to_owned(),
        };
        let audit = AuditRecord::operator(
            tenant,
            AuditAction::QuotaChanged,
            ResourceType::Quota,
            "scheduling-weight",
            reason,
            self.clock.now(),
        );
        self.store
            .put_scheduling_weight(tenant, weight, &audit)
            .await?;
        Ok(())
    }

    /// A namespace of another tenant looks exactly like one that does not exist, and is audited.
    async fn check_namespace(
        &self,
        principal: &Principal,
        namespace: Option<NamespaceId>,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        match namespace {
            Some(id) => require_namespace(&self.store, &self.clock, principal, id, ctx).await,
            None => Ok(()),
        }
    }
}

fn invalid(error: QuotaError) -> AppError {
    AppError::NotAllowed(match error {
        QuotaError::TooLarge => "a quota limit is at most 1000000000",
        QuotaError::NotPerNamespace => "this quota is counted for the whole tenant",
        QuotaError::AboveCeiling => {
            "the value is above the ceiling the platform grants this tenant (see `ceiling`)"
        }
    })
}

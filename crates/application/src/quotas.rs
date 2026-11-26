//! Quotas: reading the limits in force and changing them (FR-009-006, US04).
//!
//! *Enforcing* a quota is not done here. The stores that create the things a quota counts (runs,
//! workflows, worker sessions, secrets) take the limit from [`QuotaLookup`] and refuse the creation
//! with `StoreError::QuotaExceeded` from inside the transaction that would create it, under a lock
//! on the counted scope, so two replicas racing for the last place cannot both get it. This service
//! is the administrative side: it shows the limit and the usage and lets an administrator change it.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Subject, TenantId};
use sagat_domain::quota::{self, QuotaError, QuotaResource, QuotaScope, QuotaSource};
use sagat_domain::workflow::ActivityName;

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

/// Exact retained logical bytes charged to a tenant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetainedUsage {
    pub content_bytes: u64,
    pub content_reserved_bytes: u64,
    pub log_bytes: u64,
}

/// Result of consuming one durable namespace-rate token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDecision {
    Admitted,
    Refused { retry_after_secs: u32 },
}

/// Authoritative admission for one authenticated, already-authorized namespace API request.
pub trait NamespaceApiAdmission: Send + Sync {
    fn admit_namespace_request(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<RateDecision, StoreError>> + Send;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredActivityLimit {
    pub activity: ActivityName,
    pub value: u64,
    pub used: u64,
}

/// Storage port of the quota administration. Implemented by `sagat-infra-postgres`.
pub trait QuotaStore: QuotaLookup + NamespaceDirectory {
    fn activity_settings(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Vec<StoredActivityLimit>, StoreError>> + Send;

    fn put_activity_setting(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        activity: &ActivityName,
        value: Option<u64>,
        changed_by: &str,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    fn activity_usage(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        activity: &ActivityName,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

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

    /// The retained step-log bytes of `tenant`, or `None` unless `subject` holds the PII reveal grant in every namespace
    /// of `tenant`, each read as `has_effective_payload_reveal_grant` reads it (its current membership role may use a
    /// grant, and the grant row exists). A tenant with no namespace, or a subject with no membership, answers `None`
    /// (fail-closed; design 041, decision D041-133). The grant and the usage are read in one snapshot, so a read that
    /// runs after a revoke commits never answers `Some` (decision D041-141).
    fn step_log_usage_if_revealed(
        &self,
        tenant: TenantId,
        subject: &Subject,
    ) -> impl Future<Output = Result<Option<u64>, StoreError>> + Send;

    /// The usage of `resource` (one of [`QuotaResource::reveals_key_sharing`]) in `namespace`, or `None` unless
    /// `subject`'s current role may use a reveal grant and it holds one for the namespace (feature 022, decision D022-133;
    /// the rule of the key listings, D022-060). The right and the usage are read in one snapshot, so a read that runs
    /// after a revoke commits never answers `Some`.
    fn flow_control_usage_if_revealed(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &Subject,
        resource: QuotaResource,
    ) -> impl Future<Output = Result<Option<u64>, StoreError>> + Send;
}

/// One line of the quota view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaView {
    pub resource: QuotaResource,
    pub limit: u64,
    pub source: QuotaSource,
    /// The most the tenant may set for this resource (and the most that can be in force).
    pub ceiling: u64,
    /// `None` when the view is the tenant-wide default of a resource that is counted per namespace, or when the usage is
    /// withheld.
    pub used: Option<u64>,
    /// True when the usage is withheld: for `tenant_step_log_bytes`, the caller lacks `run:logs:read` or the PII reveal
    /// grant in some namespace of the tenant (decision D041-133, OD-2026-11-20-19); for `concurrency_live_keys`,
    /// `rate_limit_live_keys` and `flow_control_key_waiters`, the caller may not reveal personal data in the namespace
    /// (decision D022-133).
    pub usage_withheld: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityQuotaView {
    pub activity: ActivityName,
    pub limit: u64,
    pub used: u64,
    pub ceiling: u64,
    pub overridden: bool,
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
    /// Activity-specific concurrency overrides stored for a namespace.
    pub async fn list_activity_limits(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Vec<ActivityQuotaView>, AppError> {
        principal.authorize(Permission::QuotaRead)?;
        self.check_namespace(principal, Some(namespace), ctx)
            .await?;
        let tenant = principal.tenant_id();
        let ceiling = self.activity_ceiling(tenant).await?;
        let mut views = Vec::new();
        for stored in self.store.activity_settings(tenant, namespace).await? {
            views.push(ActivityQuotaView {
                activity: stored.activity,
                limit: stored.value.min(ceiling),
                used: stored.used,
                ceiling,
                overridden: true,
            });
        }
        Ok(views)
    }

    pub async fn set_activity_limit(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        activity: ActivityName,
        value: u64,
        ctx: &RequestContext,
    ) -> Result<ActivityQuotaView, AppError> {
        principal.authorize(Permission::QuotaManage)?;
        self.check_namespace(principal, Some(namespace), ctx)
            .await?;
        let ceiling = self.activity_ceiling(principal.tenant_id()).await?;
        let value = quota::validate(QuotaResource::ActivityConcurrency, true, value, ceiling)
            .map_err(invalid)?;
        self.write_activity(principal, namespace, &activity, Some(value), ctx)
            .await?;
        // A platform ceiling may have changed while this write waited for its policy fence.
        // The stored override remains subject to that ceiling, just like every admission.
        let ceiling = self.activity_ceiling(principal.tenant_id()).await?;
        Ok(ActivityQuotaView {
            used: self
                .store
                .activity_usage(principal.tenant_id(), namespace, &activity)
                .await?,
            activity,
            limit: value.min(ceiling),
            ceiling,
            overridden: true,
        })
    }

    pub async fn clear_activity_limit(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        activity: ActivityName,
        ctx: &RequestContext,
    ) -> Result<ActivityQuotaView, AppError> {
        principal.authorize(Permission::QuotaManage)?;
        self.check_namespace(principal, Some(namespace), ctx)
            .await?;
        self.write_activity(principal, namespace, &activity, None, ctx)
            .await?;
        let tenant = principal.tenant_id();
        Ok(ActivityQuotaView {
            limit: self
                .store
                .effective_limit(tenant, Some(namespace), QuotaResource::ActivityConcurrency)
                .await?,
            used: self
                .store
                .activity_usage(tenant, namespace, &activity)
                .await?,
            ceiling: self.activity_ceiling(tenant).await?,
            activity,
            overridden: false,
        })
    }

    async fn activity_ceiling(&self, tenant: TenantId) -> Result<u64, AppError> {
        Ok(quota::ceiling(
            QuotaResource::ActivityConcurrency,
            self.store
                .ceilings(tenant)
                .await?
                .into_iter()
                .find(|c| c.resource == QuotaResource::ActivityConcurrency)
                .map(|c| c.value),
        ))
    }

    async fn write_activity(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        activity: &ActivityName,
        value: Option<u64>,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        let reason = value.map_or_else(
            || format!("activity {} concurrency reset", activity.as_str()),
            |v| format!("activity {} concurrency = {v}", activity.as_str()),
        );
        let audit = AuditRecord::by(
            principal,
            AuditAction::QuotaChanged,
            ResourceType::Quota,
            format!("{namespace}/{}", activity.as_str()),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(reason);
        self.store
            .put_activity_setting(
                principal.tenant_id(),
                namespace,
                activity,
                value,
                principal.subject().as_str(),
                &audit,
            )
            .await?;
        Ok(())
    }

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
            let (used, usage_withheld) = self.used(principal, namespace, resource).await?;
            views.push(QuotaView {
                resource,
                limit,
                source,
                ceiling,
                used,
                usage_withheld,
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
        let (used, usage_withheld) = self.used(principal, namespace, resource).await?;
        Ok(QuotaView {
            resource,
            limit,
            source,
            ceiling,
            used,
            usage_withheld,
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
            .map(|value| quota::validate_ceiling(resource, value))
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

    /// The usage a view shows for `resource`, and whether it is withheld. The retained step-log bytes of a tenant move
    /// with what its runs log, so they are shown only to a caller who holds `run:logs:read` and the PII reveal grant in
    /// every namespace of the tenant, and `run:read` as ADR-0085 rule 6 states (decision D041-133, OD-2026-11-20-19,
    /// replacing D041-066's permission-only rule). The store reads the grant and the usage in one snapshot, so a view
    /// whose read runs after a revoke commits withholds the usage (decision D041-141). A store error fails the whole
    /// view rather than show the usage (fail-closed; decision D041-140).
    async fn used(
        &self,
        principal: &Principal,
        namespace: Option<NamespaceId>,
        resource: QuotaResource,
    ) -> Result<(Option<u64>, bool), AppError> {
        let tenant = principal.tenant_id();
        if resource == QuotaResource::TenantStepLogBytes {
            let permitted = principal.authorize(Permission::RunRead).is_ok()
                && principal.authorize(Permission::RunLogsRead).is_ok();
            let used = if permitted {
                self.store
                    .step_log_usage_if_revealed(tenant, principal.subject())
                    .await?
            } else {
                None
            };
            return Ok((used, used.is_none()));
        }
        // the live-key counts and the busiest key's waiters change exactly when a key's last run ends, so they tell which
        // runs share a key: shown only to a caller who may reveal personal data in the namespace, the right and the usage
        // read in one snapshot (feature 022, decision D022-133)
        if resource.reveals_key_sharing()
            && let Some(namespace) = namespace
        {
            let used = self
                .store
                .flow_control_usage_if_revealed(tenant, namespace, principal.subject(), resource)
                .await?;
            return Ok((used, used.is_none()));
        }
        let used = match (resource.has_usage(), resource.scope(), namespace) {
            (false, _, _) | (true, QuotaScope::Namespace, None) => None,
            (true, QuotaScope::Tenant, _) | (true, QuotaScope::Namespace, Some(_)) => {
                Some(self.store.usage(tenant, namespace, resource).await?)
            }
        };
        Ok((used, false))
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
        QuotaError::TooLarge => "the quota limit is above the maximum for this resource",
        QuotaError::OutsideRange => "the quota limit is outside the valid range for this resource",
        QuotaError::NotPerNamespace => "this quota is counted for the whole tenant",
        QuotaError::AboveCeiling => {
            "the value is above the ceiling the platform grants this tenant (see `ceiling`)"
        }
    })
}

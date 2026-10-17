//! Namespace-scoped grants that allow an eligible tenant member to reveal `pii` payload values.
//!
//! Role permissions and grants are deliberately separate. A built-in role never grants
//! `payload:reveal-pii`; this resource records the additional namespace decision, while the
//! caller's current role must still hold `payload:read-internal` when the grant is used.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Subject, TenantId};
use time::OffsetDateTime;

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;

/// Default and maximum number of grants returned by one page.
pub const DEFAULT_PAGE_SIZE: u32 = 50;
pub const MAX_PAGE_SIZE: u32 = 200;

/// Hard namespace cap from FR-016-011. The store enforces it while holding the namespace lock.
pub const MAX_GRANTS_PER_NAMESPACE: i64 = 256;

/// One persisted namespace reveal grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealGrant {
    pub subject: Subject,
    pub granted_by: Subject,
    pub granted_at: OffsetDateTime,
}

/// Result of the atomic grant transaction.
///
/// The store owns these decisions because membership, the namespace cap, insertion and audit must
/// be observed and committed under one database transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantWrite {
    Created(RevealGrant),
    Existing(RevealGrant),
    UnknownSubject,
    RoleCannotReveal,
    LimitReached,
    ActorForbidden,
}

/// Result of a list transaction that revalidates the actor's current membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantRead {
    Authorized(Vec<RevealGrant>),
    ActorForbidden,
}

/// Result of an idempotent revoke transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantRevoke {
    Removed,
    Absent,
    ActorForbidden,
}

/// Closed application failures of grant management.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RevealGrantError {
    #[error(transparent)]
    Application(#[from] AppError),
    #[error("the subject is not a member of this tenant")]
    UnknownSubject,
    #[error("the subject's current role cannot read internal payload values")]
    RoleCannotReveal,
    #[error("the namespace has reached its reveal grant limit")]
    GrantLimit,
}

impl From<StoreError> for RevealGrantError {
    fn from(error: StoreError) -> Self {
        Self::Application(AppError::from(error))
    }
}

/// A page whose continuation is still a typed subject. The HTTP adapter makes it opaque on wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealGrantPage {
    pub items: Vec<RevealGrant>,
    pub next_after: Option<Subject>,
}

/// A namespace already authorized and resolved inside the authenticated tenant.
///
/// HTTP performs durable namespace admission after `prepare` and before calling a read or write.
/// Keeping the principal here prevents a handler from replacing the tenant or audit actor later.
#[derive(Debug, Clone)]
pub struct RevealGrantScope {
    principal: Principal,
    namespace: NamespaceId,
}

impl RevealGrantScope {
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.principal.tenant_id()
    }

    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace
    }
}

/// Persistence boundary for reveal-grant management.
pub trait RevealGrantStore: NamespaceDirectory {
    /// Returns grants ordered by subject, strictly after `after`, at most `limit` rows.
    fn reveal_grants(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        actor: &Subject,
        after: Option<&Subject>,
        limit: u32,
    ) -> impl Future<Output = Result<GrantRead, StoreError>> + Send;

    /// Idempotently inserts a grant. Membership/role validation, the concurrent-safe cap and the
    /// audit insert share the transaction. `Existing` writes no second audit record.
    fn grant_reveal(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        actor: &Subject,
        subject: &Subject,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<GrantWrite, StoreError>> + Send;

    /// Removes a grant and writes `audit` in the same transaction. `false` is an idempotent no-op.
    fn revoke_reveal(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        actor: &Subject,
        subject: &Subject,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<GrantRevoke, StoreError>> + Send;
}

/// Authorizes and manages reveal grants without transport-specific types.
pub struct RevealGrantService<S, C> {
    store: S,
    clock: C,
}

impl<S, C> RevealGrantService<S, C>
where
    S: RevealGrantStore,
    C: Clock,
{
    #[must_use]
    pub const fn new(store: S, clock: C) -> Self {
        Self { store, clock }
    }

    /// Authorizes membership management and resolves the namespace in the caller's tenant.
    ///
    /// The returned scope is safe to pass through durable API admission before the operation.
    pub async fn prepare(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<RevealGrantScope, AppError> {
        principal.authorize(Permission::MembershipManage)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(RevealGrantScope {
            principal: principal.clone(),
            namespace,
        })
    }

    /// Lists one deterministic page.
    pub async fn list(
        &self,
        scope: &RevealGrantScope,
        after: Option<&Subject>,
        limit: u32,
    ) -> Result<RevealGrantPage, RevealGrantError> {
        let limit = limit.clamp(1, MAX_PAGE_SIZE);
        let read = self
            .store
            .reveal_grants(
                scope.tenant_id(),
                scope.namespace_id(),
                scope.principal.subject(),
                after,
                limit.saturating_add(1),
            )
            .await?;
        let GrantRead::Authorized(mut items) = read else {
            return Err(actor_forbidden());
        };
        let next_after = if items.len() > limit as usize {
            items.truncate(limit as usize);
            items.last().map(|grant| grant.subject.clone())
        } else {
            None
        };
        Ok(RevealGrantPage { items, next_after })
    }

    /// Idempotently grants reveal to an eligible member, with the audit in the write transaction.
    pub async fn grant(
        &self,
        scope: &RevealGrantScope,
        raw_subject: &str,
        ctx: &RequestContext,
    ) -> Result<RevealGrant, RevealGrantError> {
        let subject = Subject::parse(raw_subject).map_err(|_| RevealGrantError::UnknownSubject)?;
        let audit = self.audit(
            scope,
            &subject,
            AuditAction::PayloadRevealGranted,
            "granted",
            ctx,
        );
        match self
            .store
            .grant_reveal(
                scope.tenant_id(),
                scope.namespace_id(),
                scope.principal.subject(),
                &subject,
                &audit,
            )
            .await?
        {
            GrantWrite::Created(grant) => {
                metrics::counter!("payload_reveal_grant_changes_total", "op" => "grant")
                    .increment(1);
                Ok(grant)
            }
            GrantWrite::Existing(grant) => Ok(grant),
            GrantWrite::UnknownSubject => Err(RevealGrantError::UnknownSubject),
            GrantWrite::RoleCannotReveal => Err(RevealGrantError::RoleCannotReveal),
            GrantWrite::LimitReached => Err(RevealGrantError::GrantLimit),
            GrantWrite::ActorForbidden => Err(actor_forbidden()),
        }
    }

    /// Idempotently revokes a grant. A malformed or absent subject is already not granted.
    pub async fn revoke(
        &self,
        scope: &RevealGrantScope,
        raw_subject: &str,
        ctx: &RequestContext,
    ) -> Result<bool, RevealGrantError> {
        let Ok(subject) = Subject::parse(raw_subject) else {
            return Ok(false);
        };
        let audit = self.audit(
            scope,
            &subject,
            AuditAction::PayloadRevealRevoked,
            "revoked",
            ctx,
        );
        let outcome = self
            .store
            .revoke_reveal(
                scope.tenant_id(),
                scope.namespace_id(),
                scope.principal.subject(),
                &subject,
                &audit,
            )
            .await?;
        let removed = match outcome {
            GrantRevoke::Removed => true,
            GrantRevoke::Absent => false,
            GrantRevoke::ActorForbidden => return Err(actor_forbidden()),
        };
        if removed {
            metrics::counter!("payload_reveal_grant_changes_total", "op" => "revoke").increment(1);
        }
        Ok(removed)
    }

    fn audit(
        &self,
        scope: &RevealGrantScope,
        subject: &Subject,
        action: AuditAction,
        reason: &'static str,
        ctx: &RequestContext,
    ) -> AuditRecord {
        AuditRecord::by(
            &scope.principal,
            action,
            ResourceType::PayloadRevealGrant,
            format!("{}/{}", scope.namespace_id(), subject.as_str()),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(reason)
    }
}

fn actor_forbidden() -> RevealGrantError {
    RevealGrantError::Application(AppError::Forbidden {
        permission: Permission::MembershipManage,
    })
}

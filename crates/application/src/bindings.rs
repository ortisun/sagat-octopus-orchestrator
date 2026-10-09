//! Worker bindings: which namespaces and activities a worker identity may serve (FR-009-002, G31).
//!
//! `worker:execute` says an identity is a worker; a binding says what work it may take. Without a
//! binding a worker is refused at registration (fail closed). Removing or narrowing a binding takes
//! effect at once: the sessions that are no longer covered are closed and the attempts they hold are
//! released to the scheduler, whose fencing makes the revoked worker's late reports stale.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::binding::{self, BindingError, MAX_PATTERNS};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Subject, TenantId};

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;

/// What changing a binding did to the workers that were using it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Revoked {
    /// Sessions that were closed because the new binding no longer covers what they advertise.
    pub sessions: u64,
    /// Attempts those sessions held, released for the scheduler to reclaim.
    pub leases: u64,
}

/// A binding as stored: one identity, one namespace, its activity patterns in canonical text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBinding {
    pub subject: String,
    pub patterns: Vec<String>,
}

/// Storage port of the bindings. Implemented by `sagat-infra-postgres`.
pub trait BindingStore: NamespaceDirectory {
    /// The bindings of a namespace, by subject.
    fn bindings(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Vec<StoredBinding>, StoreError>> + Send;

    /// Replaces the patterns of one identity in one namespace (`patterns` empty removes the binding),
    /// closes the sessions the result no longer covers and releases their attempts, and writes the
    /// audit record, all in one transaction. `Ok(None)` when there was nothing to remove.
    fn replace_binding(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
        patterns: &[String],
        changed_by: &str,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Option<Revoked>, StoreError>> + Send;
}

/// A binding with what changing it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingView {
    pub subject: String,
    pub patterns: Vec<String>,
    pub revoked: Revoked,
}

/// Reads and changes bindings.
pub struct BindingService<S, C> {
    store: S,
    clock: C,
}

impl<S, C> BindingService<S, C>
where
    S: BindingStore,
    C: Clock,
{
    #[must_use]
    pub const fn new(store: S, clock: C) -> Self {
        Self { store, clock }
    }

    /// The bindings of a namespace.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`] for a namespace the tenant does not have.
    pub async fn list(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Vec<BindingView>, AppError> {
        principal.authorize(Permission::WorkerBind)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .bindings(principal.tenant_id(), namespace)
            .await?
            .into_iter()
            .map(|b| BindingView {
                subject: b.subject,
                patterns: b.patterns,
                revoked: Revoked::default(),
            })
            .collect())
    }

    /// Binds `subject` to `activities` in `namespace`, replacing what it had there.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`]; [`AppError::NotAllowed`] for a subject or a
    /// pattern that cannot be used.
    pub async fn bind(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        subject: &str,
        activities: &[String],
        ctx: &RequestContext,
    ) -> Result<BindingView, AppError> {
        principal.authorize(Permission::WorkerBind)?;
        let subject = Subject::parse(subject)
            .map_err(|_| AppError::NotAllowed("the subject is not a valid identity"))?;
        let patterns = binding::parse_patterns(activities).map_err(|error| {
            AppError::NotAllowed(match error {
                BindingError::PatternCount => "a binding names between 1 and 64 activity patterns",
                BindingError::InvalidPattern => {
                    "an activity pattern is an activity name, a prefix ending in `.*`, or `*`"
                }
            })
        })?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let texts: Vec<String> = patterns.iter().map(|p| p.as_text()).collect();
        debug_assert!(texts.len() <= MAX_PATTERNS);
        let audit = self.audit(
            principal,
            AuditAction::WorkerBound,
            namespace,
            format!("{subject}: {}", texts.join(", ")),
            ctx,
        );
        let revoked = self
            .store
            .replace_binding(
                principal.tenant_id(),
                namespace,
                subject.as_str(),
                &texts,
                principal.subject().as_str(),
                &audit,
            )
            .await?
            .unwrap_or_default();
        Ok(BindingView {
            subject: subject.to_string(),
            patterns: texts,
            revoked,
        })
    }

    /// Removes the binding of `subject` in `namespace`.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`] for a namespace the tenant does not have or
    /// an identity that has no binding there.
    pub async fn unbind(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        subject: &str,
        ctx: &RequestContext,
    ) -> Result<Revoked, AppError> {
        principal.authorize(Permission::WorkerBind)?;
        let subject = Subject::parse(subject).map_err(|_| AppError::NotFound)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let audit = self.audit(
            principal,
            AuditAction::WorkerUnbound,
            namespace,
            subject.to_string(),
            ctx,
        );
        self.store
            .replace_binding(
                principal.tenant_id(),
                namespace,
                subject.as_str(),
                &[],
                principal.subject().as_str(),
                &audit,
            )
            .await?
            .ok_or(AppError::NotFound)
    }

    fn audit(
        &self,
        principal: &Principal,
        action: AuditAction,
        namespace: NamespaceId,
        reason: String,
        ctx: &RequestContext,
    ) -> AuditRecord {
        AuditRecord::by(
            principal,
            action,
            ResourceType::WorkerBinding,
            namespace.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(reason)
    }
}

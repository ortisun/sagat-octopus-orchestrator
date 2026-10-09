//! Authorization helpers shared by the services that act on a namespace addressed by id.
//!
//! The rule they share: a namespace that exists only in another tenant is indistinguishable, to the
//! caller, from one that does not exist at all (`NotFound`), and the attempt is recorded against the
//! caller's tenant so a probe leaves a trace (FR-009-001, FR-009-004).

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Principal, TenantId};

use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;

/// What a store must answer to check a namespace id on behalf of a caller.
pub trait NamespaceDirectory: Send + Sync {
    /// Whether the namespace exists in `tenant`.
    fn namespace_exists(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Whether the namespace exists in a tenant *other than* `tenant` (only to decide whether a
    /// miss deserves a cross-tenant audit record; never shown to clients).
    fn namespace_exists_outside(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Records an access to a resource of another tenant (best effort, off the response path,
    /// collapsed when repeated).
    fn record_denial(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Confirms the namespace belongs to the caller's tenant.
///
/// # Errors
/// [`AppError::NotFound`] when it does not (whether it exists elsewhere or nowhere);
/// a store failure otherwise.
pub async fn require_namespace<D: NamespaceDirectory, C: Clock>(
    directory: &D,
    clock: &C,
    principal: &Principal,
    namespace: NamespaceId,
    ctx: &RequestContext,
) -> Result<(), AppError> {
    if directory
        .namespace_exists(principal.tenant_id(), namespace)
        .await?
    {
        return Ok(());
    }
    if let Ok(true) = directory
        .namespace_exists_outside(principal.tenant_id(), namespace)
        .await
    {
        let record = AuditRecord::by(
            principal,
            AuditAction::CrossTenantAccessDenied,
            ResourceType::Namespace,
            namespace.to_string(),
            ctx.trace_id.clone(),
            clock.now(),
        )
        .with_reason("resource not accessible from this tenant");
        if let Err(error) = directory.record_denial(&record).await {
            tracing::error!(%error, action = %record.action(), "failed to record audit entry");
        }
    }
    Err(AppError::NotFound)
}

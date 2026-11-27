//! The one way a lookup that found nothing in the caller's tenant is denied and audited (feature 009, decision D60; gap
//! G041-028).
//!
//! Another tenant's resource is indistinguishable from a missing one (constitution, principle V), in the answer and in
//! the audit the caller's tenant can read. So every path that, after a lookup found nothing in the caller's tenant, checks
//! whether the identifier names another tenant's resource hands that check to [`deny_unresolved`] (or its two variants),
//! which turns it into one [`BoundaryDenial`]: the same record for the caller's tenant in both cases, and the detail for the
//! platform's security audit only when the resource is another tenant's. `scripts/check-architecture.py` fails when a
//! path calls such a check anywhere else or names the records itself.

use std::future::Future;

use sagat_domain::audit::{BoundaryDenial, ResourceType};
use sagat_domain::identity::Principal;
use time::OffsetDateTime;

use crate::error::StoreError;

/// Where a [`BoundaryDenial`] is written. Implemented once by the PostgreSQL adapter, and a supertrait of every store whose
/// paths look for another tenant's resource.
pub trait BoundaryAudit: Send + Sync {
    /// Records `denial`: its tenant record always, its probe when it has one. Off the answer path and bounded, so that
    /// neither the time of the answer nor the tenant's record depends on what the check found; a failure is the trail's,
    /// never the caller's answer.
    ///
    /// # Errors
    /// A store failure, which the callers log and drop.
    fn record_boundary_denial(
        &self,
        denial: &BoundaryDenial,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Whose a resource named by an identifier is, as one statement reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tenancy {
    /// The caller's tenant holds it: no lookup across the boundary.
    Caller,
    /// Another tenant holds it.
    Other,
    /// No tenant holds it.
    Nobody,
}

/// Denies `principal`'s lookup of `resource_id` (a `resource_type`) that found nothing in its tenant: awaits `elsewhere`,
/// the check for another tenant's resource (run whatever the outcome, so that the database work before the answer is the
/// same for a missing and a foreign identifier), and records the [`BoundaryDenial`]. A failed check is logged, counted
/// (`tenant_boundary_check_failures_total`) and taken as "nobody's": the tenant's record is written all the same.
pub async fn deny_unresolved<A, F>(
    audit: &A,
    principal: &Principal,
    resource_type: ResourceType,
    resource_id: impl Into<String>,
    elsewhere: F,
    trace_id: Option<String>,
    now: OffsetDateTime,
) where
    A: BoundaryAudit + ?Sized,
    F: Future<Output = Result<bool, StoreError>>,
{
    let resource_id = resource_id.into();
    deny_unresolved_among(
        audit,
        principal,
        resource_type,
        async move { elsewhere.await.map(|found| found.then_some(resource_id)) },
        trace_id,
        now,
    )
    .await;
}

/// As [`deny_unresolved`], for a call that named several resources of one kind and found some of them nowhere in its
/// tenant (the acknowledgement of several leases): `elsewhere` checks them all at once and yields the identifier of one
/// that another tenant holds, if any, which the probe names. The tenant's record is the same whichever it finds.
pub async fn deny_unresolved_among<A, F>(
    audit: &A,
    principal: &Principal,
    resource_type: ResourceType,
    elsewhere: F,
    trace_id: Option<String>,
    now: OffsetDateTime,
) where
    A: BoundaryAudit + ?Sized,
    F: Future<Output = Result<Option<String>, StoreError>>,
{
    let found = match elsewhere.await {
        Ok(found) => found,
        Err(error) => {
            check_failed(resource_type, &error);
            None
        }
    };
    let elsewhere = found.is_some();
    let denial = BoundaryDenial::new(
        principal,
        resource_type,
        found.unwrap_or_default(),
        elsewhere,
        trace_id,
        now,
    );
    if let Err(error) = audit.record_boundary_denial(&denial).await {
        tracing::error!(%error, resource_type = %resource_type, "failed to record a lookup across the tenant boundary");
    }
}

/// As [`deny_unresolved`], for a check that can also find the resource in the caller's own tenant (a lease no longer held
/// on a run of the tenant answers as a missing lease does): nothing is recorded then, since nothing was looked up across
/// the boundary.
pub async fn deny_unless_own<A, F>(
    audit: &A,
    principal: &Principal,
    resource_type: ResourceType,
    resource_id: impl Into<String>,
    tenancy: F,
    trace_id: Option<String>,
    now: OffsetDateTime,
) where
    A: BoundaryAudit + ?Sized,
    F: Future<Output = Result<Tenancy, StoreError>>,
{
    let tenancy = tenancy.await;
    if matches!(tenancy, Ok(Tenancy::Caller)) {
        return;
    }
    deny_unresolved(
        audit,
        principal,
        resource_type,
        resource_id,
        std::future::ready(tenancy.map(|found| found == Tenancy::Other)),
        trace_id,
        now,
    )
    .await;
}

fn check_failed(resource_type: ResourceType, error: &StoreError) {
    tracing::warn!(
        kind = error.kind(),
        sqlstate = error.sqlstate(),
        resource_type = %resource_type,
        "the check for another tenant's resource failed; the lookup is recorded as a missing one"
    );
    metrics::counter!("tenant_boundary_check_failures_total", "resource_type" => resource_type.as_str())
        .increment(1);
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use sagat_domain::audit::AuditAction;
    use sagat_domain::identity::{Role, Subject, TenantId};

    use super::*;

    #[derive(Default)]
    struct Recorded(Mutex<Vec<BoundaryDenial>>);

    impl BoundaryAudit for Recorded {
        async fn record_boundary_denial(&self, denial: &BoundaryDenial) -> Result<(), StoreError> {
            self.0.lock().expect("lock").push(denial.clone());
            Ok(())
        }
    }

    fn caller() -> Principal {
        Principal::new(
            TenantId::generate(),
            Subject::parse("prober").expect("subject"),
            Role::Admin,
        )
    }

    async fn deny(tenancy: Result<Tenancy, StoreError>) -> Vec<BoundaryDenial> {
        let audit = Recorded::default();
        deny_unless_own(
            &audit,
            &caller(),
            ResourceType::Run,
            "0190a5b2-0000-7000-8000-00000000f0e1",
            std::future::ready(tenancy),
            None,
            OffsetDateTime::UNIX_EPOCH,
        )
        .await;
        audit.0.into_inner().expect("lock")
    }

    #[tokio::test]
    async fn a_foreign_a_missing_and_an_unknown_resource_give_the_tenant_one_record_and_only_the_foreign_a_probe()
     {
        let foreign = deny(Ok(Tenancy::Other)).await;
        let missing = deny(Ok(Tenancy::Nobody)).await;
        let failed = deny(Err(StoreError::DatabaseUnavailable)).await;
        for denials in [&foreign, &missing, &failed] {
            assert_eq!(denials.len(), 1);
            let record = denials[0].tenant_record();
            assert_eq!(record.action(), AuditAction::ResourceNotFoundDenied);
            assert_eq!(record.resource_id(), "");
        }
        assert!(foreign[0].probe().is_some());
        assert!(missing[0].probe().is_none());
        assert!(failed[0].probe().is_none());
    }

    #[tokio::test]
    async fn a_resource_of_the_callers_own_tenant_is_not_recorded() {
        assert!(deny(Ok(Tenancy::Caller)).await.is_empty());
    }
}

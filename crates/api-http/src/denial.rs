//! Audit of refused calls (gap G9, decided 2026-10-16): every `403` / `PERMISSION_DENIED` leaves one
//! audit record naming the tenant, the subject, the permission (or refusal code) and the call by its
//! route template or RPC name, with the trace id.
//!
//! The answer to the caller is built before and independently of this: a failure to write the
//! record is logged and counted, and the caller still gets the same `403`. That is still failing
//! closed, because the request was refused already.
//!
//! Volume is bounded by the per-subject budget, which [`crate::extract::authenticate_bearer`]
//! charges before anything is authorized: a request the budget refuses is a `429` and never reaches
//! this point, so no second limiter is needed.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use sagat_application::error::AccessDenial;
use sagat_application::identity::RequestContext;
use sagat_domain::identity::Principal;

use crate::core::Core;

/// How long the write may take before the answer goes out without it.
const WRITE_BUDGET: Duration = Duration::from_secs(5);

/// Response extension: this answer is a denial of the given kind.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Denied(pub AccessDenial);

type Pending<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// A caller that was authenticated, able to audit its own denial.
pub(crate) trait AuditsDenial: Send + Sync {
    fn audit<'a>(&'a self, call: &'a str, denial: AccessDenial) -> Pending<'a>;
}

struct Bound<St> {
    state: St,
    principal: Principal,
    ctx: RequestContext,
}

impl<St: Core> AuditsDenial for Bound<St> {
    fn audit<'a>(&'a self, call: &'a str, denial: AccessDenial) -> Pending<'a> {
        Box::pin(audit_access_denied(
            &self.state,
            &self.principal,
            call,
            denial,
            &self.ctx,
        ))
    }
}

/// Where the extractor leaves the authenticated caller for the middleware that sees the answer.
#[derive(Clone, Default)]
pub(crate) struct DenialSlot(Arc<Mutex<Option<Arc<dyn AuditsDenial>>>>);

impl DenialSlot {
    pub(crate) fn bind<St: Core>(&self, state: &St, principal: &Principal, ctx: &RequestContext) {
        let bound: Arc<dyn AuditsDenial> = Arc::new(Bound {
            state: state.clone(),
            principal: principal.clone(),
            ctx: ctx.clone(),
        });
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(bound);
    }

    pub(crate) fn take(&self) -> Option<Arc<dyn AuditsDenial>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// Writes the record of a denial. Never fails and never changes the answer: a failure is logged
/// (without the caller's data) and counted by outcome, a closed set.
pub async fn audit_access_denied<St: Core>(
    state: &St,
    principal: &Principal,
    call: &str,
    denial: AccessDenial,
    ctx: &RequestContext,
) {
    let written = tokio::time::timeout(
        WRITE_BUDGET,
        state.record_access_denied(principal, call, denial, ctx),
    )
    .await;
    match written {
        Ok(Ok(())) => {
            metrics::counter!("access_denial_audits_total", "outcome" => "recorded").increment(1);
        }
        Ok(Err(error)) => {
            metrics::counter!("access_denial_audits_total", "outcome" => "failed").increment(1);
            tracing::error!(%error, call, "failed to record the audit entry of a denied call");
        }
        Err(_) => {
            metrics::counter!("access_denial_audits_total", "outcome" => "failed").increment(1);
            tracing::error!(call, "timed out recording the audit entry of a denied call");
        }
    }
}

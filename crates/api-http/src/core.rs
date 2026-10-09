//! What every authenticated route needs, whatever else its state carries.
//!
//! [`Caller`](crate::extract::Caller) authenticates a request through this trait, so the identity
//! routes (state: [`AppState`](crate::AppState)) and the execution routes (state:
//! [`ExecState`](crate::exec::ExecState)) share one implementation of token verification, the
//! per-subject and per-tenant budgets and the per-tenant concurrency cap.

use std::future::Future;
use std::sync::Arc;

use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::identity::{IdentityStore, Session};

use crate::concurrency::TenantConcurrency;
use crate::limits::RateLimiter;
use crate::{AppState, TokenVerifier};

pub trait Core: Clone + Send + Sync + 'static {
    fn verifier(&self) -> &TokenVerifier;

    /// Budget per verified subject, charged before any database lookup.
    fn subject_limiter(&self) -> &RateLimiter<String>;

    /// Budget per tenant, charged after authentication.
    fn tenant_limiter(&self) -> &RateLimiter;

    fn tenant_concurrency(&self) -> &Arc<TenantConcurrency>;

    /// Turns verified claims into a session (tenant and role come from the database).
    fn authenticate(
        &self,
        tenant: &str,
        subject: &str,
    ) -> impl Future<Output = Result<Session, AppError>> + Send;
}

impl<S, C> Core for AppState<S, C>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    fn verifier(&self) -> &TokenVerifier {
        &self.verifier
    }

    fn subject_limiter(&self) -> &RateLimiter<String> {
        &self.subject_limiter
    }

    fn tenant_limiter(&self) -> &RateLimiter {
        &self.limiter
    }

    fn tenant_concurrency(&self) -> &Arc<TenantConcurrency> {
        &self.tenant_concurrency
    }

    async fn authenticate(&self, tenant: &str, subject: &str) -> Result<Session, AppError> {
        self.identity.authenticate(tenant, subject).await
    }
}

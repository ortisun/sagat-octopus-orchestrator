//! The state of the governance routes: quotas, and (as they are built) secrets, worker bindings and
//! egress policy.

use std::sync::Arc;

use sagat_application::clock::Clock;
use sagat_application::identity::IdentityStore;
use sagat_application::quotas::{QuotaService, QuotaStore};

use crate::concurrency::TenantConcurrency;
use crate::core::Core;
use crate::limits::RateLimiter;
use crate::{AppState, TokenVerifier};

/// Everything the governance handlers need, behind one type so handlers have one generic parameter.
pub trait Governance: Core {
    type Store: QuotaStore + 'static;
    type Clock: Clock + 'static;

    fn quotas(&self) -> &QuotaService<Self::Store, Self::Clock>;
}

/// The identity state plus the governance services, which share one store `G`.
pub struct GovState<S, C, G> {
    pub core: AppState<S, C>,
    quotas: Arc<QuotaService<G, C>>,
}

impl<S, C, G> GovState<S, C, G> {
    #[must_use]
    pub fn new(core: AppState<S, C>, quotas: QuotaService<G, C>) -> Self {
        Self {
            core,
            quotas: Arc::new(quotas),
        }
    }
}

impl<S, C, G> Clone for GovState<S, C, G> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            quotas: Arc::clone(&self.quotas),
        }
    }
}

impl<S, C, G> Core for GovState<S, C, G>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
    G: Send + Sync + 'static,
{
    fn verifier(&self) -> &TokenVerifier {
        self.core.verifier()
    }

    fn subject_limiter(&self) -> &RateLimiter<String> {
        self.core.subject_limiter()
    }

    fn tenant_limiter(&self) -> &RateLimiter {
        self.core.tenant_limiter()
    }

    fn tenant_concurrency(&self) -> &Arc<TenantConcurrency> {
        self.core.tenant_concurrency()
    }

    async fn authenticate(
        &self,
        tenant: &str,
        subject: &str,
    ) -> Result<sagat_application::identity::Session, sagat_application::error::AppError> {
        self.core.authenticate(tenant, subject).await
    }
}

impl<S, C, G> Governance for GovState<S, C, G>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
    G: QuotaStore + 'static,
{
    type Store = G;
    type Clock = C;

    fn quotas(&self) -> &QuotaService<G, C> {
        &self.quotas
    }
}

//! The state of the governance routes: quotas, and (as they are built) secrets, worker bindings and
//! egress policy.

use std::sync::Arc;

use sagat_application::bindings::{BindingService, BindingStore};
use sagat_application::clock::Clock;
use sagat_application::identity::IdentityStore;
use sagat_application::quotas::{QuotaService, QuotaStore};
use sagat_application::secrets::{SecretService, SecretStore};

use crate::concurrency::TenantConcurrency;
use crate::core::Core;
use crate::limits::RateLimiter;
use crate::{AppState, TokenVerifier};

/// Everything the governance handlers need, behind one type so handlers have one generic parameter.
pub trait Governance: Core {
    type Store: QuotaStore + BindingStore + SecretStore + 'static;
    type Clock: Clock + 'static;

    fn quotas(&self) -> &QuotaService<Self::Store, Self::Clock>;
    fn bindings(&self) -> &BindingService<Self::Store, Self::Clock>;
    /// `None` when the instance has no key ring: the secret routes are then not served at all.
    fn secrets(&self) -> Option<&SecretService<Self::Store, Self::Clock>>;
}

/// The identity state plus the governance services, which share one store `G`.
pub struct GovState<S, C, G> {
    pub core: AppState<S, C>,
    quotas: Arc<QuotaService<G, C>>,
    bindings: Arc<BindingService<G, C>>,
    secrets: Option<Arc<SecretService<G, C>>>,
}

impl<S, C, G> GovState<S, C, G> {
    #[must_use]
    pub fn new(
        core: AppState<S, C>,
        quotas: QuotaService<G, C>,
        bindings: BindingService<G, C>,
    ) -> Self {
        Self {
            core,
            quotas: Arc::new(quotas),
            bindings: Arc::new(bindings),
            secrets: None,
        }
    }

    /// Serves the secret routes, backed by `secrets`.
    #[must_use]
    pub fn with_secrets(mut self, secrets: SecretService<G, C>) -> Self {
        self.secrets = Some(Arc::new(secrets));
        self
    }
}

impl<S, C, G> Clone for GovState<S, C, G> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            quotas: Arc::clone(&self.quotas),
            bindings: Arc::clone(&self.bindings),
            secrets: self.secrets.clone(),
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
    G: QuotaStore + BindingStore + SecretStore + 'static,
{
    type Store = G;
    type Clock = C;

    fn quotas(&self) -> &QuotaService<G, C> {
        &self.quotas
    }

    fn bindings(&self) -> &BindingService<G, C> {
        &self.bindings
    }

    fn secrets(&self) -> Option<&SecretService<G, C>> {
        self.secrets.as_deref()
    }
}

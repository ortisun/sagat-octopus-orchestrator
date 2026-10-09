//! The state of the governance routes: quotas, and (as they are built) secrets, worker bindings and
//! egress policy.

use std::sync::Arc;

use sagat_application::bindings::{BindingService, BindingStore};
use sagat_application::clock::Clock;
use sagat_application::connectors::{ConnectorService, ConnectorStore};
use sagat_application::egress::{EgressService, EgressStore};
use sagat_application::events::{EventService, EventStore};
use sagat_application::identity::IdentityStore;
use sagat_application::outbound::{OutboundService, OutboundStore};
use sagat_application::quotas::{QuotaService, QuotaStore};
use sagat_application::secrets::{SecretService, SecretStore};

use crate::concurrency::TenantConcurrency;
use crate::core::Core;
use crate::limits::RateLimiter;
use crate::{AppState, TokenVerifier};

/// Everything the governance handlers need, behind one type so handlers have one generic parameter.
pub trait Governance: Core {
    type Store: QuotaStore
        + BindingStore
        + SecretStore
        + EgressStore
        + EventStore
        + OutboundStore
        + ConnectorStore
        + 'static;
    type Clock: Clock + 'static;

    fn quotas(&self) -> &QuotaService<Self::Store, Self::Clock>;
    fn bindings(&self) -> &BindingService<Self::Store, Self::Clock>;
    fn egress(&self) -> &EgressService<Self::Store, Self::Clock>;
    /// `None` when the instance has no key ring: the secret routes are then not served at all.
    fn secrets(&self) -> Option<&SecretService<Self::Store, Self::Clock>>;
    /// `None` when the instance has no key ring (a webhook cannot be verified without one): the
    /// event routes are then not served at all.
    fn events(&self) -> Option<&EventService<Self::Store, Self::Clock>>;
    /// `None` when the instance has no key ring (deliveries are signed): the subscription routes are
    /// then not served at all.
    fn outbound(&self) -> Option<&OutboundService<Self::Store, Self::Clock>>;
    /// `None` when the instance has no key ring (connectors read brokers with secrets): the
    /// connector routes are then not served at all.
    fn connectors(&self) -> Option<&ConnectorService<Self::Store, Self::Clock>>;
}

/// The identity state plus the governance services, which share one store `G`.
pub struct GovState<S, C, G> {
    pub core: AppState<S, C>,
    quotas: Arc<QuotaService<G, C>>,
    bindings: Arc<BindingService<G, C>>,
    egress: Arc<EgressService<G, C>>,
    secrets: Option<Arc<SecretService<G, C>>>,
    events: Option<Arc<EventService<G, C>>>,
    outbound: Option<Arc<OutboundService<G, C>>>,
    connectors: Option<Arc<ConnectorService<G, C>>>,
}

impl<S, C, G> GovState<S, C, G> {
    #[must_use]
    pub fn new(
        core: AppState<S, C>,
        quotas: QuotaService<G, C>,
        bindings: BindingService<G, C>,
        egress: EgressService<G, C>,
    ) -> Self {
        Self {
            core,
            quotas: Arc::new(quotas),
            bindings: Arc::new(bindings),
            egress: Arc::new(egress),
            secrets: None,
            events: None,
            outbound: None,
            connectors: None,
        }
    }

    /// Serves the secret routes, backed by `secrets`.
    #[must_use]
    pub fn with_secrets(mut self, secrets: SecretService<G, C>) -> Self {
        self.secrets = Some(Arc::new(secrets));
        self
    }

    /// Serves the connector routes, backed by `connectors`.
    #[must_use]
    pub fn with_connectors(mut self, connectors: ConnectorService<G, C>) -> Self {
        self.connectors = Some(Arc::new(connectors));
        self
    }

    /// Serves the subscription routes, backed by `outbound`.
    #[must_use]
    pub fn with_outbound(mut self, outbound: OutboundService<G, C>) -> Self {
        self.outbound = Some(Arc::new(outbound));
        self
    }

    /// Serves the event routes (endpoints, publishing, the public webhook URL), backed by `events`.
    #[must_use]
    pub fn with_events(mut self, events: EventService<G, C>) -> Self {
        self.events = Some(Arc::new(events));
        self
    }
}

impl<S, C, G> Clone for GovState<S, C, G> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            quotas: Arc::clone(&self.quotas),
            bindings: Arc::clone(&self.bindings),
            egress: Arc::clone(&self.egress),
            secrets: self.secrets.clone(),
            events: self.events.clone(),
            outbound: self.outbound.clone(),
            connectors: self.connectors.clone(),
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
    G: QuotaStore
        + BindingStore
        + SecretStore
        + EgressStore
        + EventStore
        + OutboundStore
        + ConnectorStore
        + 'static,
{
    type Store = G;
    type Clock = C;

    fn quotas(&self) -> &QuotaService<G, C> {
        &self.quotas
    }

    fn bindings(&self) -> &BindingService<G, C> {
        &self.bindings
    }

    fn egress(&self) -> &EgressService<G, C> {
        &self.egress
    }

    fn secrets(&self) -> Option<&SecretService<G, C>> {
        self.secrets.as_deref()
    }

    fn events(&self) -> Option<&EventService<G, C>> {
        self.events.as_deref()
    }

    fn outbound(&self) -> Option<&OutboundService<G, C>> {
        self.outbound.as_deref()
    }

    fn connectors(&self) -> Option<&ConnectorService<G, C>> {
        self.connectors.as_deref()
    }
}

//! The state of the governance routes: quotas, and (as they are built) secrets, worker bindings and
//! egress policy.

use std::sync::Arc;

use sagat_application::api_admission::{ApiAdmissionService, ApiAdmissionStore};
use sagat_application::bindings::{BindingService, BindingStore};
use sagat_application::clock::Clock;
use sagat_application::connectors::{ConnectorService, ConnectorStore};
use sagat_application::egress::{EgressService, EgressStore};
use sagat_application::events::{EventService, EventStore};
use sagat_application::identity::IdentityStore;
use sagat_application::managed_secrets::{ManagedSecretService, ManagedSecretStore};
use sagat_application::operations::{AlertReader, AuditStore, OperationsService, WorkerAdmin};
use sagat_application::outbound::{OutboundService, OutboundStore};
use sagat_application::quotas::{QuotaService, QuotaStore};
use sagat_application::scripts::{RunnerService, RunnerStore, ScriptService, ScriptStore};
use sagat_application::secrets::{SecretService, SecretStore};

use crate::admission::Admission;
use crate::concurrency::TenantConcurrency;
use crate::core::Core;
use crate::limits::RateLimiter;
use crate::{AppState, TokenVerifier};

/// Everything the governance handlers need, behind one type so handlers have one generic parameter.
pub trait Governance: Core + Admission {
    type Store: QuotaStore
        + BindingStore
        + SecretStore
        + ManagedSecretStore
        + EgressStore
        + EventStore
        + OutboundStore
        + ConnectorStore
        + ScriptStore
        + RunnerStore
        + AlertReader
        + WorkerAdmin
        + AuditStore
        + 'static;
    type Clock: Clock + 'static;

    fn quotas(&self) -> &QuotaService<Self::Store, Self::Clock>;
    fn bindings(&self) -> &BindingService<Self::Store, Self::Clock>;
    fn egress(&self) -> &EgressService<Self::Store, Self::Clock>;
    /// `None` when no secret authority is configured.
    fn secrets(&self) -> Option<&SecretService<Self::Store, Self::Clock>>;
    fn managed_secrets(&self) -> Option<&ManagedSecretService<Self::Store, Self::Clock>>;
    /// `None` when no reader is configured to verify webhook secrets.
    fn events(&self) -> Option<&EventService<Self::Store, Self::Clock>>;
    /// `None` when the instance has no key ring (deliveries are signed): the subscription routes are
    /// then not served at all.
    fn outbound(&self) -> Option<&OutboundService<Self::Store, Self::Clock>>;
    /// `None` when the instance has no key ring (connectors read brokers with secrets): the
    /// connector routes are then not served at all.
    fn connectors(&self) -> Option<&ConnectorService<Self::Store, Self::Clock>>;
    /// `None` when the instance has no signing key (an artifact cannot be trusted without one): the
    /// script routes are then not served at all.
    fn scripts(&self) -> Option<&ScriptService<Self::Store, Self::Clock>>;
    /// `None` when the instance has no signing key: the runner routes are then not served at all.
    fn runner(&self) -> Option<&RunnerService<Self::Store, Self::Clock>>;
    /// `None` when the instance does not run the monitor's reads: the alert routes are then not
    /// served at all.
    fn operations(&self) -> Option<&OperationsService<Self::Store, Self::Clock>>;
}

/// The identity state plus the governance services, which share one store `G`.
pub struct GovState<S, C, G> {
    pub core: AppState<S, C>,
    admission: Arc<ApiAdmissionService<G, C>>,
    quotas: Arc<QuotaService<G, C>>,
    bindings: Arc<BindingService<G, C>>,
    egress: Arc<EgressService<G, C>>,
    secrets: Option<Arc<SecretService<G, C>>>,
    managed_secrets: Option<Arc<ManagedSecretService<G, C>>>,
    events: Option<Arc<EventService<G, C>>>,
    outbound: Option<Arc<OutboundService<G, C>>>,
    connectors: Option<Arc<ConnectorService<G, C>>>,
    scripts: Option<Arc<ScriptService<G, C>>>,
    runner: Option<Arc<RunnerService<G, C>>>,
    operations: Option<Arc<OperationsService<G, C>>>,
}

impl<S, C, G> GovState<S, C, G> {
    #[must_use]
    pub fn new(
        core: AppState<S, C>,
        admission: ApiAdmissionService<G, C>,
        quotas: QuotaService<G, C>,
        bindings: BindingService<G, C>,
        egress: EgressService<G, C>,
    ) -> Self {
        Self {
            core,
            admission: Arc::new(admission),
            quotas: Arc::new(quotas),
            bindings: Arc::new(bindings),
            egress: Arc::new(egress),
            secrets: None,
            managed_secrets: None,
            events: None,
            outbound: None,
            connectors: None,
            scripts: None,
            runner: None,
            operations: None,
        }
    }

    /// Serves the alert routes, backed by `operations`.
    #[must_use]
    pub fn with_operations(mut self, operations: OperationsService<G, C>) -> Self {
        self.operations = Some(Arc::new(operations));
        self
    }

    /// Serves the secret routes, backed by `secrets`.
    #[must_use]
    pub fn with_secrets(mut self, secrets: SecretService<G, C>) -> Self {
        self.secrets = Some(Arc::new(secrets));
        self
    }

    pub fn with_managed_secrets(mut self, service: ManagedSecretService<G, C>) -> Self {
        self.managed_secrets = Some(Arc::new(service));
        self
    }

    /// Serves the connector routes, backed by `connectors`.
    #[must_use]
    pub fn with_connectors(mut self, connectors: ConnectorService<G, C>) -> Self {
        self.connectors = Some(Arc::new(connectors));
        self
    }

    /// Serves the script routes (drafts, versions, test runs, executions) and, with `runner`, the
    /// routes of the sandbox's runner.
    #[must_use]
    pub fn with_scripts(
        mut self,
        scripts: ScriptService<G, C>,
        runner: RunnerService<G, C>,
    ) -> Self {
        self.scripts = Some(Arc::new(scripts));
        self.runner = Some(Arc::new(runner));
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
            admission: Arc::clone(&self.admission),
            quotas: Arc::clone(&self.quotas),
            bindings: Arc::clone(&self.bindings),
            egress: Arc::clone(&self.egress),
            secrets: self.secrets.clone(),
            managed_secrets: self.managed_secrets.clone(),
            events: self.events.clone(),
            outbound: self.outbound.clone(),
            connectors: self.connectors.clone(),
            scripts: self.scripts.clone(),
            runner: self.runner.clone(),
            operations: self.operations.clone(),
        }
    }
}

impl<S, C, G> Admission for GovState<S, C, G>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
    G: ApiAdmissionStore + 'static,
{
    type AdmissionStore = G;
    type AdmissionClock = C;

    fn api_admission(&self) -> &ApiAdmissionService<G, C> {
        &self.admission
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
        + ApiAdmissionStore
        + BindingStore
        + SecretStore
        + ManagedSecretStore
        + EgressStore
        + EventStore
        + OutboundStore
        + ConnectorStore
        + ScriptStore
        + RunnerStore
        + AlertReader
        + WorkerAdmin
        + AuditStore
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

    fn managed_secrets(&self) -> Option<&ManagedSecretService<G, C>> {
        self.managed_secrets.as_deref()
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

    fn scripts(&self) -> Option<&ScriptService<G, C>> {
        self.scripts.as_deref()
    }

    fn runner(&self) -> Option<&RunnerService<G, C>> {
        self.runner.as_deref()
    }

    fn operations(&self) -> Option<&OperationsService<G, C>> {
        self.operations.as_deref()
    }
}

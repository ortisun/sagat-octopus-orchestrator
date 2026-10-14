//! The state of the definition and run routes.

use std::sync::Arc;

use sagat_application::api_admission::{ApiAdmissionService, ApiAdmissionStore};
use sagat_application::clock::Clock;
use sagat_application::definitions::{DefinitionService, DefinitionStore, IrCodec};
use sagat_application::execution::ExpressionEvaluator;
use sagat_application::identity::IdentityStore;
use sagat_application::payloads::PayloadInspectionUseCase;
use sagat_application::publishing::{PublishingService, PublishingStore};
use sagat_application::runs::{RunService, RunStore};

use crate::admission::Admission;
use crate::blobs::{BlobTransfers, HttpBlobTransfer};
use crate::concurrency::TenantConcurrency;
use crate::core::Core;
use crate::limits::RateLimiter;
use crate::streams::{StreamLimits, StreamRegistry};
use crate::{AppState, TokenVerifier};

/// Everything the definition and run handlers need, behind one type so handlers have one generic
/// parameter instead of five.
pub trait Exec: Core + Admission {
    type Store: DefinitionStore + RunStore + PublishingStore + Clone + 'static;
    type Clock: Clock + Clone + 'static;
    type Codec: IrCodec + Clone + 'static;
    type Eval: ExpressionEvaluator + 'static;

    fn definitions(&self) -> &DefinitionService<Self::Store, Self::Clock, Self::Codec>;
    fn runs(&self) -> &RunService<Self::Store, Self::Clock, Self::Eval>;

    /// `None` when the instance does not serve approvals, the active version and promotion: those
    /// routes are then not served at all.
    fn publishing(&self) -> Option<&PublishingService<Self::Store, Self::Clock, Self::Codec>>;

    /// The open server-sent streams (D142).
    fn streams(&self) -> &Arc<StreamRegistry>;
    fn blob_transfers(&self) -> &Arc<BlobTransfers>;
    fn payload_inspections(&self) -> Option<&Arc<dyn PayloadInspectionUseCase>>;
}

/// The identity state plus the definition and run services.
pub struct ExecState<S, C, P, X, E> {
    pub core: AppState<S, C>,
    admission: Arc<ApiAdmissionService<P, C>>,
    definitions: Arc<DefinitionService<P, C, X>>,
    publishing: Option<Arc<PublishingService<P, C, X>>>,
    runs: Arc<RunService<P, C, E>>,
    streams: Arc<StreamRegistry>,
    blob_transfers: Arc<BlobTransfers>,
    payload_inspections: Option<Arc<dyn PayloadInspectionUseCase>>,
}

impl<S, C, P, X, E> ExecState<S, C, P, X, E> {
    #[must_use]
    pub fn new(
        core: AppState<S, C>,
        admission: ApiAdmissionService<P, C>,
        definitions: DefinitionService<P, C, X>,
        runs: RunService<P, C, E>,
    ) -> Self {
        Self {
            core,
            admission: Arc::new(admission),
            definitions: Arc::new(definitions),
            publishing: None,
            runs: Arc::new(runs),
            streams: StreamRegistry::new(StreamLimits::default()),
            blob_transfers: BlobTransfers::new(HttpBlobTransfer::default()),
            payload_inspections: None,
        }
    }

    /// Serves the approval, active-version and promotion routes, backed by `publishing`.
    #[must_use]
    pub fn with_publishing(mut self, publishing: PublishingService<P, C, X>) -> Self {
        self.publishing = Some(Arc::new(publishing));
        self
    }

    /// Replaces the limits of server-sent streams (the defaults are in [`StreamLimits`]).
    #[must_use]
    pub fn with_stream_limits(mut self, limits: StreamLimits) -> Self {
        self.streams = StreamRegistry::new(limits);
        self
    }

    #[must_use]
    pub fn with_blob_transfer(mut self, limits: HttpBlobTransfer) -> Self {
        self.blob_transfers = BlobTransfers::new(limits);
        self
    }

    #[must_use]
    pub fn with_payload_inspections(
        mut self,
        payload_inspections: Arc<dyn PayloadInspectionUseCase>,
    ) -> Self {
        self.payload_inspections = Some(payload_inspections);
        self
    }
}

impl<S, C, P, X, E> Clone for ExecState<S, C, P, X, E> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            admission: Arc::clone(&self.admission),
            definitions: Arc::clone(&self.definitions),
            publishing: self.publishing.clone(),
            runs: Arc::clone(&self.runs),
            streams: Arc::clone(&self.streams),
            blob_transfers: Arc::clone(&self.blob_transfers),
            payload_inspections: self.payload_inspections.clone(),
        }
    }
}

impl<S, C, P, X, E> Admission for ExecState<S, C, P, X, E>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
    P: ApiAdmissionStore + 'static,
    X: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    type AdmissionStore = P;
    type AdmissionClock = C;

    fn api_admission(&self) -> &ApiAdmissionService<P, C> {
        &self.admission
    }
}

impl<S, C, P, X, E> Core for ExecState<S, C, P, X, E>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
    P: Send + Sync + 'static,
    X: Send + Sync + 'static,
    E: Send + Sync + 'static,
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

impl<S, C, P, X, E> Exec for ExecState<S, C, P, X, E>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
    C: Clone,
    P: DefinitionStore + RunStore + PublishingStore + ApiAdmissionStore + Clone + 'static,
    X: IrCodec + Clone + 'static,
    E: ExpressionEvaluator + 'static,
{
    type Store = P;
    type Clock = C;
    type Codec = X;
    type Eval = E;

    fn definitions(&self) -> &DefinitionService<P, C, X> {
        &self.definitions
    }

    fn runs(&self) -> &RunService<P, C, E> {
        &self.runs
    }

    fn publishing(&self) -> Option<&PublishingService<P, C, X>> {
        self.publishing.as_deref()
    }

    fn streams(&self) -> &Arc<StreamRegistry> {
        &self.streams
    }

    fn blob_transfers(&self) -> &Arc<BlobTransfers> {
        &self.blob_transfers
    }

    fn payload_inspections(&self) -> Option<&Arc<dyn PayloadInspectionUseCase>> {
        self.payload_inspections.as_ref()
    }
}

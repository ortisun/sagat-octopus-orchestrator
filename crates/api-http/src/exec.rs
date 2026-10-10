//! The state of the definition and run routes.

use std::sync::Arc;

use sagat_application::clock::Clock;
use sagat_application::definitions::{DefinitionService, DefinitionStore, IrCodec};
use sagat_application::execution::ExpressionEvaluator;
use sagat_application::identity::IdentityStore;
use sagat_application::runs::{RunService, RunStore};

use crate::concurrency::TenantConcurrency;
use crate::core::Core;
use crate::limits::RateLimiter;
use crate::streams::{StreamLimits, StreamRegistry};
use crate::{AppState, TokenVerifier};

/// Everything the definition and run handlers need, behind one type so handlers have one generic
/// parameter instead of five.
pub trait Exec: Core {
    type Store: DefinitionStore + RunStore + 'static;
    type Clock: Clock + 'static;
    type Codec: IrCodec + 'static;
    type Eval: ExpressionEvaluator + 'static;

    fn definitions(&self) -> &DefinitionService<Self::Store, Self::Clock, Self::Codec>;
    fn runs(&self) -> &RunService<Self::Store, Self::Clock, Self::Eval>;

    /// The open server-sent streams (D142).
    fn streams(&self) -> &Arc<StreamRegistry>;
}

/// The identity state plus the definition and run services.
pub struct ExecState<S, C, P, X, E> {
    pub core: AppState<S, C>,
    definitions: Arc<DefinitionService<P, C, X>>,
    runs: Arc<RunService<P, C, E>>,
    streams: Arc<StreamRegistry>,
}

impl<S, C, P, X, E> ExecState<S, C, P, X, E> {
    #[must_use]
    pub fn new(
        core: AppState<S, C>,
        definitions: DefinitionService<P, C, X>,
        runs: RunService<P, C, E>,
    ) -> Self {
        Self {
            core,
            definitions: Arc::new(definitions),
            runs: Arc::new(runs),
            streams: StreamRegistry::new(StreamLimits::default()),
        }
    }

    /// Replaces the limits of server-sent streams (the defaults are in [`StreamLimits`]).
    #[must_use]
    pub fn with_stream_limits(mut self, limits: StreamLimits) -> Self {
        self.streams = StreamRegistry::new(limits);
        self
    }
}

impl<S, C, P, X, E> Clone for ExecState<S, C, P, X, E> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            definitions: Arc::clone(&self.definitions),
            runs: Arc::clone(&self.runs),
            streams: Arc::clone(&self.streams),
        }
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
    P: DefinitionStore + RunStore + 'static,
    X: IrCodec + 'static,
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

    fn streams(&self) -> &Arc<StreamRegistry> {
        &self.streams
    }
}

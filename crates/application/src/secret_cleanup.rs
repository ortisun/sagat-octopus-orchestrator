//! Operator-only retirement of expired provider objects. No plaintext retry queue.
use crate::{
    error::{AppError, StoreError},
    secret_provider::{ProviderReference, SecretProvider},
};
use std::{future::Future, sync::Arc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CleanupClaim {
    pub reference: ProviderReference,
    pub lease: Uuid,
}

pub trait SecretCleanupStore: Send + Sync {
    /// One terminal run per transaction; preserve every live/manual-review attempt pin.
    fn release_terminal_credential_pins(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;
    fn provider_cleanup_candidates(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<ProviderReference>, StoreError>> + Send;
    /// Recheck age/references and commit abandonment/audit plus a unique bounded cleanup lease.
    fn claim_provider_cleanup(
        &self,
        reference: &ProviderReference,
    ) -> impl Future<Output = Result<Option<CleanupClaim>, StoreError>> + Send;
    /// Only after confirmed provider retirement; recheck claim and references, audit and refund bytes.
    fn finish_provider_cleanup(
        &self,
        claim: &CleanupClaim,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

#[derive(Debug, Default)]
pub struct CleanupReport {
    pub pins_released: u64,
    pub retired: u32,
    pub deferred: u32,
}

/// Only composition roots/operator commands construct this service. No tenant HTTP route exposes it.
pub struct SecretCleanupService<S> {
    store: S,
    provider: Arc<dyn SecretProvider>,
}
impl<S: SecretCleanupStore> SecretCleanupService<S> {
    pub fn new(store: S, provider: Arc<dyn SecretProvider>) -> Self {
        Self { store, provider }
    }
    #[tracing::instrument(name = "secret.cleanup", skip_all)]
    pub async fn run_batch(&self, limit: u32) -> Result<CleanupReport, AppError> {
        if !(1..=128).contains(&limit) {
            return Err(AppError::NotAllowed(
                "secret cleanup batch must be between 1 and 128",
            ));
        }
        let started = std::time::Instant::now();
        // The operator composition root enforces the whole-command deadline. The application
        // stays runtime-independent; cancelled claims remain recoverable after their lease.
        let result = self.run_inner(limit).await;
        metrics::histogram!("secret_cleanup_seconds").record(started.elapsed().as_secs_f64());
        result
    }
    async fn run_inner(&self, limit: u32) -> Result<CleanupReport, AppError> {
        let mut report = CleanupReport {
            pins_released: self.store.release_terminal_credential_pins(limit).await?,
            ..CleanupReport::default()
        };
        metrics::counter!("secret_cleanup_pins_released_total").increment(report.pins_released);
        for reference in self.store.provider_cleanup_candidates(limit).await? {
            let Some(claim) = self.store.claim_provider_cleanup(&reference).await? else {
                continue;
            };
            if self.provider.delete(&claim.reference).await.is_err() {
                report.deferred += 1;
                metrics::counter!("secret_cleanup_operations_total","outcome"=>"deferred")
                    .increment(1);
                continue;
            }
            if self.store.finish_provider_cleanup(&claim).await? {
                report.retired += 1;
                metrics::counter!("secret_cleanup_operations_total","outcome"=>"retired")
                    .increment(1);
            } else {
                report.deferred += 1;
                metrics::counter!("secret_cleanup_operations_total","outcome"=>"superseded")
                    .increment(1);
            }
        }
        Ok(report)
    }
}

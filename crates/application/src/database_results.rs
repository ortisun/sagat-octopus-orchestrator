//! Explicit, bounded disclosure of one retained database step result (ADR-0035).
//! Object I/O holds no database lock. Current authority and the selected immutable digest are
//! checked again in the transaction that appends the disclosure audit, before bytes are returned.
use std::{future::Future, pin::Pin, time::Instant};

use crate::{
    blobs::{BlobAccess, BlobService},
    clock::Clock,
    database::DatabaseResultValidator,
    error::{AppError, StoreError},
    payloads::StoredPayloadBytes,
};
use sagat_domain::{
    audit::{AuditAction, AuditRecord, ResourceType},
    blobs::BlobDescriptor,
    database::DatabaseLimits,
    execution::RunId,
    identity::{NamespaceId, Permission, Principal},
    workflow::{Digest, NodeId},
};
use sha2::{Digest as _, Sha256};

pub struct DatabaseResultRequest {
    pub principal: Principal,
    pub namespace: NamespaceId,
    pub run: RunId,
    pub node: NodeId,
    pub trace_id: Option<String>,
}

/// Provenance comes from the committed blob catalog, never from a payload's JSON shape.
pub enum DatabaseResultContent {
    Inline(StoredPayloadBytes),
    Blob(BlobDescriptor),
}
pub struct SelectedDatabaseResult {
    pub digest: Digest,
    pub content: DatabaseResultContent,
}
pub struct PreparedDatabaseResult {
    request: DatabaseResultRequest,
    selected: SelectedDatabaseResult,
}
impl PreparedDatabaseResult {
    #[must_use]
    pub const fn namespace(&self) -> NamespaceId {
        self.request.namespace
    }
}

pub trait DatabaseResultStore: Send + Sync {
    fn select_database_result(
        &self,
        request: &DatabaseResultRequest,
    ) -> impl Future<Output = Result<Option<SelectedDatabaseResult>, StoreError>> + Send;
    /// Must recheck current role, scope and output identity in the audit transaction.
    fn audit_database_result(
        &self,
        request: &DatabaseResultRequest,
        digest: &Digest,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn deny_database_result(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

pub type ResultFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;
pub trait DatabaseResultUseCase: Send + Sync {
    fn prepare(&self, request: DatabaseResultRequest) -> ResultFuture<'_, PreparedDatabaseResult>;
    /// The caller must bound total duration and concurrent buffered reads through response delivery.
    fn read(&self, prepared: PreparedDatabaseResult) -> ResultFuture<'_, StoredPayloadBytes>;
}
pub struct DatabaseResultService<S, C, V> {
    store: S,
    clock: C,
    validator: V,
    blobs: Option<BlobService>,
}
impl<S, C, V> DatabaseResultService<S, C, V> {
    #[must_use]
    pub const fn new(store: S, clock: C, validator: V, blobs: Option<BlobService>) -> Self {
        Self {
            store,
            clock,
            validator,
            blobs,
        }
    }
}
impl<S: DatabaseResultStore, C: Clock, V: DatabaseResultValidator> DatabaseResultService<S, C, V> {
    async fn denial(&self, request: &DatabaseResultRequest) -> Result<(), AppError> {
        let audit = AuditRecord::by(
            &request.principal,
            AuditAction::DatabaseDenied,
            ResourceType::Run,
            request.run.to_string(),
            request.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason("result_read");
        self.store.deny_database_result(&audit).await?;
        Ok(())
    }
    async fn prepare_inner(
        &self,
        request: DatabaseResultRequest,
    ) -> Result<PreparedDatabaseResult, AppError> {
        for permission in [Permission::RunRead, Permission::DatabaseResultRead] {
            if let Err(error) = request.principal.authorize(permission) {
                self.denial(&request).await?;
                return Err(error.into());
            }
        }
        let selected = match self.store.select_database_result(&request).await {
            Ok(Some(selected)) => selected,
            Ok(None) | Err(StoreError::MissingReference) => {
                self.denial(&request).await?;
                return Err(AppError::NotFound);
            }
            Err(error) => return Err(error.into()),
        };
        Ok(PreparedDatabaseResult { request, selected })
    }
    async fn read_inner(
        &self,
        prepared: PreparedDatabaseResult,
    ) -> Result<StoredPayloadBytes, AppError> {
        let PreparedDatabaseResult { request, selected } = prepared;
        let bytes = match selected.content {
            DatabaseResultContent::Inline(bytes) => {
                if Digest::new(Sha256::digest(bytes.as_slice()).into()) != selected.digest {
                    return Err(AppError::Unavailable);
                }
                bytes
            }
            DatabaseResultContent::Blob(descriptor) => {
                if Digest::new(Sha256::digest(crate::blobs::descriptor_json(&descriptor)).into())
                    != selected.digest
                {
                    return Err(AppError::Unavailable);
                }
                if descriptor.size() > u64::from(DatabaseLimits::MAX.max_result_bytes)
                    || descriptor.content_type().as_str() != "application/json"
                {
                    return Err(AppError::Unavailable);
                }
                let access = BlobAccess {
                    tenant_id: request.principal.tenant_id(),
                    namespace_id: request.namespace,
                };
                let (current, mut stream) = self
                    .blobs
                    .as_ref()
                    .ok_or(AppError::Unavailable)?
                    .read(access, descriptor.id())
                    .await
                    .map_err(|_| AppError::Unavailable)?;
                if current != descriptor {
                    return Err(AppError::Unavailable);
                }
                let mut bytes = Vec::new();
                // The verifier can emit a corruption error at EOF; never return partial content.
                while let Some(chunk) =
                    std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
                {
                    let chunk = chunk.map_err(|_| AppError::Unavailable)?;
                    if chunk.len() > (descriptor.size() as usize).saturating_sub(bytes.len()) {
                        return Err(AppError::Unavailable);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                if bytes.len() != descriptor.size() as usize {
                    return Err(AppError::Unavailable);
                }
                StoredPayloadBytes::new(bytes)
            }
        };
        self.validator
            .validate(bytes.as_slice(), DatabaseLimits::MAX)
            .map_err(|_| AppError::Unavailable)?;
        let audit = AuditRecord::by(
            &request.principal,
            AuditAction::DatabaseResultRead,
            ResourceType::Run,
            request.run.to_string(),
            request.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(format!("node:{}", request.node.as_str()));
        match self
            .store
            .audit_database_result(&request, &selected.digest, &audit)
            .await
        {
            Ok(()) => Ok(bytes),
            Err(StoreError::MissingReference) => {
                self.denial(&request).await?;
                Err(AppError::NotFound)
            }
            Err(error) => Err(error.into()),
        }
    }
}
impl<S: DatabaseResultStore + 'static, C: Clock + 'static, V: DatabaseResultValidator + 'static>
    DatabaseResultUseCase for DatabaseResultService<S, C, V>
{
    fn prepare(&self, request: DatabaseResultRequest) -> ResultFuture<'_, PreparedDatabaseResult> {
        Box::pin(async move {
            let prepared = self.prepare_inner(request).await;
            if let Err(error) = &prepared {
                metrics::counter!("database_result_reads_total", "outcome" => read_outcome(error))
                    .increment(1);
            }
            prepared
        })
    }
    fn read(&self, prepared: PreparedDatabaseResult) -> ResultFuture<'_, StoredPayloadBytes> {
        use tracing::Instrument;
        let span = tracing::info_span!("database_result_read", tenant_id = %prepared.request.principal.tenant_id(),
            run_id = %prepared.request.run, outcome = tracing::field::Empty);
        Box::pin(
            async move {
                let started = Instant::now();
                let result = self.read_inner(prepared).await;
                let outcome = result.as_ref().map_or_else(read_outcome, |_| "success");
                tracing::Span::current().record("outcome", outcome);
                metrics::counter!("database_result_reads_total", "outcome" => outcome).increment(1);
                metrics::histogram!("database_result_read_duration_seconds")
                    .record(started.elapsed().as_secs_f64());
                result
            }
            .instrument(span),
        )
    }
}

fn read_outcome(error: &AppError) -> &'static str {
    if matches!(error, AppError::NotFound | AppError::Forbidden { .. }) {
        "denied"
    } else {
        "error"
    }
}

//! Authorized structural payload inspection (ADR-0028, FR-007-005).
//!
//! The ordinary run and journal APIs remain payload-free. This use case authorizes a separate
//! audited read, resolves exactly one immutable input or journal event, and hands inline bytes to a
//! projector that can reveal only bounded structure. The store returns bytes only after its audit
//! transaction commits.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Permission, Principal, TenantId};

use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;
use tracing::Instrument;

pub const PAYLOAD_INSPECTION_POLICY: &str = "structure-only-v1";
pub const MAX_INSPECTION_INPUT_BYTES: usize = 256 * 1024;
pub const MAX_INSPECTION_DEPTH: usize = 16;
pub const MAX_INSPECTION_NODES: usize = 256;
pub const MAX_INSPECTION_CHILDREN: usize = 32;
pub const MAX_INSPECTION_RESPONSE_BYTES: usize = 64 * 1024;

/// The immutable payload selected by the caller. The HTTP body is a closed representation of this
/// enum, so no digest, blob id, JSON path or policy override can be supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadSelector {
    Input,
    Journal { sequence: u64 },
}

impl PayloadSelector {
    fn audit_reason(self) -> String {
        match self {
            Self::Input => format!("{PAYLOAD_INSPECTION_POLICY}:input"),
            Self::Journal { sequence } => {
                format!("{PAYLOAD_INSPECTION_POLICY}:journal:{sequence}")
            }
        }
    }
}

/// The run scope found before the separate durable namespace admission is charged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectionRun {
    pub namespace_id: NamespaceId,
}

/// An authorized, tenant-scoped run and selector. Its private fields make it impossible for an
/// adapter to replace the run or selector after admission.
#[derive(Debug, Clone)]
pub struct PreparedInspection {
    principal: Principal,
    run_id: RunId,
    namespace_id: NamespaceId,
    selector: PayloadSelector,
    trace_id: Option<String>,
}

impl PreparedInspection {
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.principal.tenant_id()
    }

    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace_id
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn selector(&self) -> PayloadSelector {
        self.selector
    }
}

/// Original bytes of an inline immutable payload. Debug output is intentionally non-revealing.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredPayloadBytes(Vec<u8>);

impl StoredPayloadBytes {
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for StoredPayloadBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredPayloadBytes")
            .field("size_bytes", &self.0.len())
            .finish_non_exhaustive()
    }
}

/// Closed media category derived from trusted catalog metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalContentKind {
    Json,
    Text,
    Binary,
    Other,
}

/// What the selected immutable record stores. External bytes never cross this port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredPayload {
    Inline(StoredPayloadBytes),
    External {
        size_bytes: u64,
        content_kind: ExternalContentKind,
    },
    NoRetainedPayload,
}

/// The selected record and its provenance, read consistently with the run version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedPayload {
    pub run_version: u64,
    pub node_id: Option<String>,
    pub attempt: Option<u32>,
    pub payload: StoredPayload,
}

/// Storage boundary. `inspect_and_audit` must select the immutable record and commit `audit` in one
/// short transaction, returning inline bytes only after that commit succeeds.
pub trait PayloadInspectionStore: Send + Sync {
    fn inspection_run(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<InspectionRun>, StoreError>> + Send;

    fn run_exists_outside(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    fn record_inspection_denial(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    fn inspect_and_audit(
        &self,
        prepared: &PreparedInspection,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Option<SelectedPayload>, StoreError>> + Send;
}

/// A type in the redacted tree. There is deliberately no representation for a key or scalar value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureType {
    Object,
    Array,
    String,
    Number,
    Boolean,
    Null,
}

/// Bounded structure returned by a projector. Container children are ordinal only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadStructure {
    Object {
        child_count: u32,
        children: Vec<Self>,
        omitted_children: u32,
    },
    Array {
        child_count: u32,
        children: Vec<Self>,
        omitted_children: u32,
    },
    String,
    Number,
    Boolean,
    Null,
}

impl PayloadStructure {
    #[must_use]
    pub const fn kind(&self) -> StructureType {
        match self {
            Self::Object { .. } => StructureType::Object,
            Self::Array { .. } => StructureType::Array,
            Self::String => StructureType::String,
            Self::Number => StructureType::Number,
            Self::Boolean => StructureType::Boolean,
            Self::Null => StructureType::Null,
        }
    }

    #[must_use]
    pub fn children(&self) -> &[Self] {
        match self {
            Self::Object { children, .. } | Self::Array { children, .. } => children,
            Self::String | Self::Number | Self::Boolean | Self::Null => &[],
        }
    }

    #[must_use]
    pub const fn child_count(&self) -> Option<u32> {
        match self {
            Self::Object { child_count, .. } | Self::Array { child_count, .. } => {
                Some(*child_count)
            }
            Self::String | Self::Number | Self::Boolean | Self::Null => None,
        }
    }

    #[must_use]
    pub const fn omitted_children(&self) -> Option<u32> {
        match self {
            Self::Object {
                omitted_children, ..
            }
            | Self::Array {
                omitted_children, ..
            } => Some(*omitted_children),
            Self::String | Self::Number | Self::Boolean | Self::Null => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadProjection {
    pub tree: PayloadStructure,
    pub truncated: bool,
}

/// Closed failure: parser text or payload excerpts never cross the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedPayloadFormat;

pub trait PayloadProjector: Send + Sync {
    fn project(&self, bytes: &[u8]) -> Result<PayloadProjection, UnsupportedPayloadFormat>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionProvenance {
    pub run_id: RunId,
    pub run_version: u64,
    pub selection: PayloadSelector,
    pub node_id: Option<String>,
    pub attempt: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailablePayloadReason {
    NoRetainedPayload,
    UnsupportedFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectionResult {
    InlineJson {
        size_bytes: u64,
        projection: PayloadProjection,
    },
    External {
        size_bytes: u64,
        content_kind: ExternalContentKind,
    },
    Unavailable {
        reason: UnavailablePayloadReason,
        size_bytes: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadInspection {
    pub provenance: InspectionProvenance,
    pub result: InspectionResult,
}

/// Object-safe boundary used by the HTTP state without adding another generic to every router.
pub type InspectionFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

pub trait PayloadInspectionUseCase: Send + Sync {
    fn prepare<'a>(
        &'a self,
        principal: &'a Principal,
        run: RunId,
        selector: PayloadSelector,
        ctx: &'a RequestContext,
    ) -> InspectionFuture<'a, PreparedInspection>;

    fn inspect<'a>(
        &'a self,
        prepared: PreparedInspection,
    ) -> InspectionFuture<'a, PayloadInspection>;
}

pub struct PayloadInspectionService<S, C, P> {
    store: S,
    clock: C,
    projector: P,
}

impl<S, C, P> PayloadInspectionService<S, C, P> {
    #[must_use]
    pub const fn new(store: S, clock: C, projector: P) -> Self {
        Self {
            store,
            clock,
            projector,
        }
    }
}

impl<S, C, P> PayloadInspectionService<S, C, P>
where
    S: PayloadInspectionStore,
    C: Clock,
    P: PayloadProjector,
{
    async fn prepare_inner(
        &self,
        principal: &Principal,
        run: RunId,
        selector: PayloadSelector,
        ctx: &RequestContext,
    ) -> Result<PreparedInspection, AppError> {
        for permission in [Permission::RunRead, Permission::RunPayloadInspect] {
            if let Err(error) = principal.authorize(permission) {
                metrics::counter!("payload_inspections_total", "outcome" => "denied").increment(1);
                let denial = AuditRecord::by(
                    principal,
                    AuditAction::PayloadInspectionDenied,
                    ResourceType::Run,
                    run.to_string(),
                    ctx.trace_id.clone(),
                    self.clock.now(),
                )
                .with_reason("permission");
                if let Err(store_error) = self.store.record_inspection_denial(&denial).await {
                    tracing::error!(error = %store_error, action = %denial.action(), "failed to record audit entry");
                }
                return Err(error.into());
            }
        }

        let Some(scope) = self
            .store
            .inspection_run(principal.tenant_id(), run)
            .await?
        else {
            if let Ok(true) = self
                .store
                .run_exists_outside(principal.tenant_id(), run)
                .await
            {
                let denial = AuditRecord::by(
                    principal,
                    AuditAction::CrossTenantAccessDenied,
                    ResourceType::Run,
                    run.to_string(),
                    ctx.trace_id.clone(),
                    self.clock.now(),
                )
                .with_reason("resource not accessible from this tenant");
                if let Err(store_error) = self.store.record_inspection_denial(&denial).await {
                    tracing::error!(error = %store_error, action = %denial.action(), "failed to record audit entry");
                }
            }
            return Err(AppError::NotFound);
        };
        Ok(PreparedInspection {
            principal: principal.clone(),
            run_id: run,
            namespace_id: scope.namespace_id,
            selector,
            trace_id: ctx.trace_id.clone(),
        })
    }

    async fn inspect_inner(
        &self,
        prepared: PreparedInspection,
    ) -> Result<PayloadInspection, AppError> {
        let started = Instant::now();
        let audit = AuditRecord::by(
            &prepared.principal,
            AuditAction::PayloadInspected,
            ResourceType::Run,
            prepared.run_id.to_string(),
            prepared.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(prepared.selector.audit_reason());
        let selected = match self.store.inspect_and_audit(&prepared, &audit).await {
            Ok(Some(selected)) => selected,
            Ok(None) => {
                finish_metrics(started, "error", false);
                tracing::Span::current().record("outcome", "not_found");
                return Err(AppError::NotFound);
            }
            Err(error) => {
                finish_metrics(started, "error", false);
                tracing::Span::current().record("outcome", "error");
                return Err(error.into());
            }
        };

        let result = match selected.payload {
            StoredPayload::Inline(bytes) => {
                let size_bytes = u64::try_from(bytes.as_slice().len()).unwrap_or(u64::MAX);
                if bytes.as_slice().len() > MAX_INSPECTION_INPUT_BYTES {
                    InspectionResult::Unavailable {
                        reason: UnavailablePayloadReason::UnsupportedFormat,
                        size_bytes: Some(size_bytes),
                    }
                } else {
                    match self.projector.project(bytes.as_slice()) {
                        Ok(projection) => InspectionResult::InlineJson {
                            size_bytes,
                            projection,
                        },
                        Err(UnsupportedPayloadFormat) => InspectionResult::Unavailable {
                            reason: UnavailablePayloadReason::UnsupportedFormat,
                            size_bytes: Some(size_bytes),
                        },
                    }
                }
            }
            StoredPayload::External {
                size_bytes,
                content_kind,
            } => InspectionResult::External {
                size_bytes,
                content_kind,
            },
            StoredPayload::NoRetainedPayload => InspectionResult::Unavailable {
                reason: UnavailablePayloadReason::NoRetainedPayload,
                size_bytes: None,
            },
        };
        let outcome = match &result {
            InspectionResult::InlineJson { .. } => "inline_json",
            InspectionResult::External { .. } => "external",
            InspectionResult::Unavailable { .. } => "unavailable",
        };
        let truncated = matches!(
            &result,
            InspectionResult::InlineJson { projection, .. } if projection.truncated
        );
        finish_metrics(started, outcome, truncated);
        tracing::Span::current().record("outcome", outcome);
        tracing::Span::current().record("truncated", truncated);
        Ok(PayloadInspection {
            provenance: InspectionProvenance {
                run_id: prepared.run_id,
                run_version: selected.run_version,
                selection: prepared.selector,
                node_id: selected.node_id,
                attempt: selected.attempt,
            },
            result,
        })
    }
}

impl<S, C, P> PayloadInspectionUseCase for PayloadInspectionService<S, C, P>
where
    S: PayloadInspectionStore + 'static,
    C: Clock + 'static,
    P: PayloadProjector + 'static,
{
    fn prepare<'a>(
        &'a self,
        principal: &'a Principal,
        run: RunId,
        selector: PayloadSelector,
        ctx: &'a RequestContext,
    ) -> InspectionFuture<'a, PreparedInspection> {
        Box::pin(self.prepare_inner(principal, run, selector, ctx))
    }

    fn inspect<'a>(
        &'a self,
        prepared: PreparedInspection,
    ) -> InspectionFuture<'a, PayloadInspection> {
        let source = match prepared.selector {
            PayloadSelector::Input => "input",
            PayloadSelector::Journal { .. } => "journal",
        };
        let span = tracing::info_span!(
            "payload_inspection",
            policy = PAYLOAD_INSPECTION_POLICY,
            tenant_id = %prepared.tenant_id(),
            run_id = %prepared.run_id,
            source,
            outcome = tracing::field::Empty,
            truncated = tracing::field::Empty,
        );
        Box::pin(self.inspect_inner(prepared).instrument(span))
    }
}

fn finish_metrics(started: Instant, outcome: &'static str, truncated: bool) {
    metrics::counter!("payload_inspections_total", "outcome" => outcome).increment(1);
    if truncated {
        metrics::counter!("payload_inspection_truncated_total").increment(1);
    }
    metrics::histogram!("payload_inspection_duration_seconds")
        .record(started.elapsed().as_secs_f64());
}

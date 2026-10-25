//! Classified, tenant-scoped run payload disclosure (ADR-0033, FR-016-006..012).
//!
//! The store keeps selection, current-role/grant checks, projection, audit and commit in one short
//! transaction. It invokes [`DisclosureEvaluator`] synchronously while that transaction is open and
//! returns no projected value until the required audit has committed.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::classification::{Audience, ClassificationRules};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Permission, Principal, Role, Subject, TenantId};
use tracing::Instrument;

use crate::classification::{
    CarrierPointer, ClassificationCacheKey, ClassificationInterpreter, ClassifiedNode,
    ClassifiedPayload, DisclosedLevels, PiiRevealFacts, ProjectionFailure, ProjectionRequest,
};
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;
use crate::payloads::{ExternalContentKind, PayloadProjection, PayloadProjector, StoredPayload};

pub const PAYLOAD_DISCLOSURE_POLICY: &str = "classified-v1";

/// The closed record selector. Digests, blob identifiers and value paths cannot cross this port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisclosureSelector {
    Input,
    Result,
    Journal { sequence: u64 },
}

impl DisclosureSelector {
    #[must_use]
    pub const fn source(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Result => "result",
            Self::Journal { .. } => "journal",
        }
    }

    fn audit_reason(self, reveal: bool) -> String {
        let selected = match self {
            Self::Input => "input".to_owned(),
            Self::Result => "result".to_owned(),
            Self::Journal { sequence } => format!("journal:{sequence}"),
        };
        if reveal {
            format!("{PAYLOAD_DISCLOSURE_POLICY}:{selected}:reveal=pii")
        } else {
            format!("{PAYLOAD_DISCLOSURE_POLICY}:{selected}")
        }
    }
}

/// The run scope resolved before canonical namespace admission is charged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisclosureRun {
    pub namespace_id: NamespaceId,
}

/// A trusted run, selector and caller. Private fields stop adapters replacing any of them after
/// admission.
#[derive(Debug, Clone)]
pub struct PreparedDisclosure {
    principal: Principal,
    run_id: RunId,
    namespace_id: NamespaceId,
    selector: DisclosureSelector,
    reveal_pii: bool,
    trace_id: Option<String>,
}

impl PreparedDisclosure {
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
    pub const fn selector(&self) -> DisclosureSelector {
        self.selector
    }

    #[must_use]
    pub const fn reveal_pii(&self) -> bool {
        self.reveal_pii
    }

    #[must_use]
    pub fn subject(&self) -> &Subject {
        self.principal.subject()
    }
}

/// Immutable workflow bytes. Debug output retains only their size.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredDefinition(Vec<u8>);

impl StoredDefinition {
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for StoredDefinition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredDefinition")
            .field("size_bytes", &self.0.len())
            .finish_non_exhaustive()
    }
}

/// Trusted records selected by storage. External content carries metadata only; its bytes are never
/// loaded by this use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedDisclosure {
    pub run_version: u64,
    pub workflow_version: WorkflowVersionId,
    pub version_number: u32,
    pub classification_rules: ClassificationRules,
    /// `None` when the selected journal event is not one of the closed carrier-producing events.
    pub carrier: Option<CarrierPointer>,
    pub node_id: Option<String>,
    pub attempt: Option<u32>,
    pub definition: StoredDefinition,
    pub payload: StoredPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisclosureSchemaCarrier {
    InputSchema,
    OutputSchema,
    NodeOutputSchema,
}

impl From<&CarrierPointer> for DisclosureSchemaCarrier {
    fn from(carrier: &CarrierPointer) -> Self {
        match carrier {
            CarrierPointer::Input => Self::InputSchema,
            CarrierPointer::Result => Self::OutputSchema,
            CarrierPointer::NodeOutput(_) | CarrierPointer::BodyNodeOutput { .. } => {
                Self::NodeOutputSchema
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisclosureSchema {
    pub carrier: DisclosureSchemaCarrier,
    pub version_number: u32,
    pub rules: ClassificationRules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisclosureCaller {
    pub disclosed_levels: DisclosedLevels,
    pub reveal_granted: bool,
    pub revealed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureOnlyReason {
    NoSchema,
    SchemaMismatch,
    LegacyVersion,
    SchemaUnusable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableDisclosureReason {
    NoRetainedPayload,
    UnsupportedFormat,
    NotDisclosable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadDisclosureResult {
    Disclosed {
        schema: DisclosureSchema,
        size_bytes: u64,
        projection: ClassifiedPayload,
    },
    StructureOnly {
        schema: Option<DisclosureSchema>,
        size_bytes: u64,
        reason: StructureOnlyReason,
        projection: PayloadProjection,
    },
    External {
        size_bytes: u64,
        content_kind: ExternalContentKind,
    },
    Unavailable {
        reason: UnavailableDisclosureReason,
        size_bytes: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadDisclosure {
    pub run_id: RunId,
    pub run_version: u64,
    pub selection: DisclosureSelector,
    pub node_id: Option<String>,
    pub attempt: Option<u32>,
    pub caller: DisclosureCaller,
    pub result: PayloadDisclosureResult,
}

/// Closed transaction result. `Missing` also covers a selector that has no retained record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisclosureRead {
    Missing,
    Refused(DisclosureFailure),
    Ready(PayloadDisclosure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisclosureAuditKind {
    Internal,
    Pii,
}

/// Server-built records available to the transactional adapter.
#[derive(Debug, Clone)]
pub struct DisclosureAudits {
    pub internal: AuditRecord,
    pub pii: AuditRecord,
    pub reveal_denied: AuditRecord,
}

impl DisclosureAudits {
    #[must_use]
    pub const fn for_kind(&self, kind: DisclosureAuditKind) -> &AuditRecord {
        match kind {
            DisclosureAuditKind::Internal => &self.internal,
            DisclosureAuditKind::Pii => &self.pii,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisclosureDecision {
    pub disclosure: PayloadDisclosure,
    pub audit: Option<DisclosureAuditKind>,
}

/// Value-free refusals made against current authority inside the read transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisclosureFailure {
    RunReadNotGranted,
    PiiRevealNotGranted,
}

/// What an adapter knows about the carrier a disclosure will use before it opens its transaction.
#[derive(Debug, Clone, Copy)]
pub struct CarrierWarmup<'a> {
    pub workflow_version: WorkflowVersionId,
    pub carrier: &'a CarrierPointer,
    pub rules: ClassificationRules,
    pub definition: &'a [u8],
}

/// Synchronous policy callback used while the adapter's read transaction remains open.
pub trait DisclosureEvaluator: Send + Sync {
    /// Called once, before the transaction opens and while no lock is held, so that CPU-heavy
    /// preparation of the carrier (compiling its validator) never happens under those locks.
    /// It must not decide anything: `evaluate` repeats every check against current authority.
    fn prepare(&self, warmup: CarrierWarmup<'_>) {
        let _ = warmup;
    }

    fn evaluate(
        &self,
        selected: SelectedDisclosure,
        current_role: Role,
        reveal_granted: bool,
    ) -> Result<DisclosureDecision, DisclosureFailure>;
}

/// Persistence boundary. Implementations must resolve the selected immutable record, recheck the
/// current role and effective namespace grant, call `evaluator`, append the selected audit and
/// commit before returning `Ready`. Reveal denial appends `audits.reveal_denied` before `Refused`.
pub trait PayloadDisclosureStore: Send + Sync {
    fn disclosure_run(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<DisclosureRun>, StoreError>> + Send;

    fn run_exists_outside(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    fn record_disclosure_denial(
        &self,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    fn disclose_and_audit(
        &self,
        prepared: &PreparedDisclosure,
        evaluator: &dyn DisclosureEvaluator,
        audits: &DisclosureAudits,
    ) -> impl Future<Output = Result<DisclosureRead, StoreError>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PayloadDisclosureError {
    #[error(transparent)]
    Application(#[from] AppError),
    #[error("PII reveal is not granted in this namespace")]
    PiiRevealNotGranted,
}

pub type PrepareDisclosureFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PreparedDisclosure, AppError>> + Send + 'a>>;
pub type DisclosureFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PayloadDisclosure, PayloadDisclosureError>> + Send + 'a>>;

pub trait PayloadDisclosureUseCase: Send + Sync {
    fn prepare<'a>(
        &'a self,
        principal: &'a Principal,
        run: RunId,
        selector: DisclosureSelector,
        reveal_pii: bool,
        ctx: &'a RequestContext,
    ) -> PrepareDisclosureFuture<'a>;

    fn disclose<'a>(&'a self, prepared: PreparedDisclosure) -> DisclosureFuture<'a>;
}

pub struct PayloadDisclosureService<S, C, I, P> {
    store: S,
    clock: C,
    interpreter: I,
    structure_projector: P,
}

impl<S, C, I, P> PayloadDisclosureService<S, C, I, P> {
    #[must_use]
    pub const fn new(store: S, clock: C, interpreter: I, structure_projector: P) -> Self {
        Self {
            store,
            clock,
            interpreter,
            structure_projector,
        }
    }
}

impl<S, C, I, P> PayloadDisclosureService<S, C, I, P>
where
    S: PayloadDisclosureStore,
    C: Clock,
    I: ClassificationInterpreter,
    P: PayloadProjector,
{
    async fn prepare_inner(
        &self,
        principal: &Principal,
        run: RunId,
        selector: DisclosureSelector,
        reveal_pii: bool,
        ctx: &RequestContext,
    ) -> Result<PreparedDisclosure, AppError> {
        principal.authorize(Permission::RunRead)?;
        let Some(scope) = self
            .store
            .disclosure_run(principal.tenant_id(), run)
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
                if let Err(error) = self.store.record_disclosure_denial(&denial).await {
                    tracing::error!(error = %error, action = %denial.action(), "failed to record audit entry");
                }
            }
            return Err(AppError::NotFound);
        };
        Ok(PreparedDisclosure {
            principal: principal.clone(),
            run_id: run,
            namespace_id: scope.namespace_id,
            selector,
            reveal_pii,
            trace_id: ctx.trace_id.clone(),
        })
    }

    async fn disclose_inner(
        &self,
        prepared: PreparedDisclosure,
    ) -> Result<PayloadDisclosure, PayloadDisclosureError> {
        let started = Instant::now();
        let audits = disclosure_audits(&prepared, &self.clock);
        let evaluator = ServiceEvaluator {
            prepared: &prepared,
            interpreter: &self.interpreter,
            structure_projector: &self.structure_projector,
        };
        let read = self
            .store
            .disclose_and_audit(&prepared, &evaluator, &audits)
            .await
            .map_err(AppError::from)?;
        let result = match read {
            DisclosureRead::Missing => Err(AppError::NotFound.into()),
            DisclosureRead::Refused(DisclosureFailure::RunReadNotGranted) => {
                Err(AppError::Forbidden {
                    permission: Permission::RunRead,
                }
                .into())
            }
            DisclosureRead::Refused(DisclosureFailure::PiiRevealNotGranted) => {
                metrics::counter!("payload_reveal_total", "outcome" => "denied").increment(1);
                Err(PayloadDisclosureError::PiiRevealNotGranted)
            }
            DisclosureRead::Ready(disclosure) => {
                if let PayloadDisclosureResult::Disclosed { projection, .. } = &disclosure.result {
                    count_disclosed_nodes(&projection.tree);
                }
                Ok(disclosure)
            }
        };
        finish_metrics(started, &result);
        result
    }
}

impl<S, C, I, P> PayloadDisclosureUseCase for PayloadDisclosureService<S, C, I, P>
where
    S: PayloadDisclosureStore + 'static,
    C: Clock + 'static,
    I: ClassificationInterpreter + 'static,
    P: PayloadProjector + 'static,
{
    fn prepare<'a>(
        &'a self,
        principal: &'a Principal,
        run: RunId,
        selector: DisclosureSelector,
        reveal_pii: bool,
        ctx: &'a RequestContext,
    ) -> PrepareDisclosureFuture<'a> {
        Box::pin(self.prepare_inner(principal, run, selector, reveal_pii, ctx))
    }

    fn disclose<'a>(&'a self, prepared: PreparedDisclosure) -> DisclosureFuture<'a> {
        let source = prepared.selector.source();
        let span = tracing::info_span!(
            "payload_disclosure",
            policy = PAYLOAD_DISCLOSURE_POLICY,
            tenant_id = %prepared.tenant_id(),
            run_id = %prepared.run_id,
            source,
            outcome = tracing::field::Empty,
            truncated = tracing::field::Empty,
            revealed = tracing::field::Empty,
        );
        Box::pin(self.disclose_inner(prepared).instrument(span))
    }
}

struct ServiceEvaluator<'a, I, P> {
    prepared: &'a PreparedDisclosure,
    interpreter: &'a I,
    structure_projector: &'a P,
}

impl<I, P> DisclosureEvaluator for ServiceEvaluator<'_, I, P>
where
    I: ClassificationInterpreter,
    P: PayloadProjector,
{
    fn prepare(&self, warmup: CarrierWarmup<'_>) {
        if warmup.rules == ClassificationRules::NONE {
            return;
        }
        self.interpreter.prepare(
            &ClassificationCacheKey {
                version: warmup.workflow_version,
                carrier: warmup.carrier.clone(),
            },
            warmup.rules,
            warmup.definition,
        );
    }

    fn evaluate(
        &self,
        selected: SelectedDisclosure,
        current_role: Role,
        reveal_granted: bool,
    ) -> Result<DisclosureDecision, DisclosureFailure> {
        if !current_role.grants(Permission::RunRead) {
            return Err(DisclosureFailure::RunReadNotGranted);
        }
        let internal = current_role.grants(Permission::PayloadReadInternal);
        let reveal_granted = reveal_granted && internal;
        if self.prepared.reveal_pii && !reveal_granted {
            return Err(DisclosureFailure::PiiRevealNotGranted);
        }

        let inspect = current_role.grants(Permission::RunPayloadInspect);
        let revealed = self.prepared.reveal_pii && reveal_granted;
        let mut disclosed_levels = DisclosedLevels::PUBLIC;
        if internal {
            disclosed_levels = disclosed_levels.union(DisclosedLevels::INTERNAL);
        }
        if revealed {
            disclosed_levels = disclosed_levels.union(DisclosedLevels::PII);
        }
        let caller = DisclosureCaller {
            disclosed_levels,
            reveal_granted,
            revealed,
        };
        let run_version = selected.run_version;
        let node_id = selected.node_id.clone();
        let attempt = selected.attempt;
        let (result, audit) = match selected.payload {
            StoredPayload::Inline(bytes) => {
                let size_bytes = u64::try_from(bytes.as_slice().len()).unwrap_or(u64::MAX);
                let Some(carrier) = selected.carrier else {
                    let result = if inspect {
                        match self.structure_projector.project(bytes.as_slice()) {
                            Ok(projection) => PayloadDisclosureResult::StructureOnly {
                                schema: None,
                                size_bytes,
                                reason: StructureOnlyReason::NoSchema,
                                projection,
                            },
                            Err(_) => PayloadDisclosureResult::Unavailable {
                                reason: UnavailableDisclosureReason::UnsupportedFormat,
                                size_bytes: Some(size_bytes),
                            },
                        }
                    } else {
                        PayloadDisclosureResult::Unavailable {
                            reason: UnavailableDisclosureReason::NotDisclosable,
                            size_bytes: Some(size_bytes),
                        }
                    };
                    return Ok(DisclosureDecision {
                        disclosure: PayloadDisclosure {
                            run_id: self.prepared.run_id,
                            run_version,
                            selection: self.prepared.selector,
                            node_id,
                            attempt,
                            caller,
                            result,
                        },
                        audit: None,
                    });
                };
                let schema = DisclosureSchema {
                    carrier: (&carrier).into(),
                    version_number: selected.version_number,
                    rules: selected.classification_rules,
                };
                let request = ProjectionRequest {
                    cache_key: ClassificationCacheKey {
                        version: selected.workflow_version,
                        carrier,
                    },
                    rules: selected.classification_rules,
                    document: selected.definition.as_slice(),
                    payload: bytes.as_slice(),
                    audience: Audience::new(internal, revealed, inspect),
                    reveal: PiiRevealFacts {
                        granted: reveal_granted,
                        requested: self.prepared.reveal_pii,
                    },
                };
                match self.interpreter.project(request) {
                    Ok(projection) => {
                        let audit = audit_for_levels(projection.disclosed);
                        (
                            PayloadDisclosureResult::Disclosed {
                                schema,
                                size_bytes,
                                projection,
                            },
                            audit,
                        )
                    }
                    Err(
                        failure @ (ProjectionFailure::NoCarrier
                        | ProjectionFailure::LegacyVersion
                        | ProjectionFailure::SchemaUnusable
                        | ProjectionFailure::SchemaMismatch),
                    ) if inspect => {
                        let (reason, schema) = structure_reason(failure, schema);
                        match self.structure_projector.project(bytes.as_slice()) {
                            Ok(projection) => (
                                PayloadDisclosureResult::StructureOnly {
                                    schema,
                                    size_bytes,
                                    reason,
                                    projection,
                                },
                                None,
                            ),
                            Err(_) => (
                                PayloadDisclosureResult::Unavailable {
                                    reason: UnavailableDisclosureReason::UnsupportedFormat,
                                    size_bytes: Some(size_bytes),
                                },
                                None,
                            ),
                        }
                    }
                    Err(
                        ProjectionFailure::UnsupportedPayload | ProjectionFailure::PayloadTooLarge,
                    ) => (
                        PayloadDisclosureResult::Unavailable {
                            reason: UnavailableDisclosureReason::UnsupportedFormat,
                            size_bytes: Some(size_bytes),
                        },
                        None,
                    ),
                    Err(_) => (
                        PayloadDisclosureResult::Unavailable {
                            reason: UnavailableDisclosureReason::NotDisclosable,
                            size_bytes: Some(size_bytes),
                        },
                        None,
                    ),
                }
            }
            StoredPayload::External {
                size_bytes,
                content_kind,
            } => (
                PayloadDisclosureResult::External {
                    size_bytes,
                    content_kind,
                },
                None,
            ),
            StoredPayload::NoRetainedPayload => (
                PayloadDisclosureResult::Unavailable {
                    reason: UnavailableDisclosureReason::NoRetainedPayload,
                    size_bytes: None,
                },
                None,
            ),
        };
        Ok(DisclosureDecision {
            disclosure: PayloadDisclosure {
                run_id: self.prepared.run_id,
                run_version,
                selection: self.prepared.selector,
                node_id,
                attempt,
                caller,
                result,
            },
            audit,
        })
    }
}

fn structure_reason(
    failure: ProjectionFailure,
    schema: DisclosureSchema,
) -> (StructureOnlyReason, Option<DisclosureSchema>) {
    match failure {
        ProjectionFailure::NoCarrier => (StructureOnlyReason::NoSchema, None),
        ProjectionFailure::LegacyVersion => (StructureOnlyReason::LegacyVersion, None),
        ProjectionFailure::SchemaMismatch => (StructureOnlyReason::SchemaMismatch, Some(schema)),
        ProjectionFailure::SchemaUnusable => (StructureOnlyReason::SchemaUnusable, Some(schema)),
        _ => unreachable!("caller limits failures to structural fallback reasons"),
    }
}

fn audit_for_levels(levels: DisclosedLevels) -> Option<DisclosureAuditKind> {
    if levels.contains(sagat_domain::classification::Level::Pii) {
        Some(DisclosureAuditKind::Pii)
    } else if levels.contains(sagat_domain::classification::Level::Internal) {
        Some(DisclosureAuditKind::Internal)
    } else {
        None
    }
}

fn disclosure_audits<C: Clock>(prepared: &PreparedDisclosure, clock: &C) -> DisclosureAudits {
    let record = |action, reason| {
        AuditRecord::by(
            &prepared.principal,
            action,
            ResourceType::Run,
            prepared.run_id.to_string(),
            prepared.trace_id.clone(),
            clock.now(),
        )
        .with_reason(reason)
    };
    DisclosureAudits {
        internal: record(
            AuditAction::PayloadDisclosed,
            prepared.selector.audit_reason(false),
        ),
        pii: record(
            AuditAction::PayloadPiiRevealed,
            prepared.selector.audit_reason(true),
        ),
        reveal_denied: record(AuditAction::PayloadRevealDenied, "permission".to_owned()),
    }
}

fn count_disclosed_nodes(node: &ClassifiedNode) {
    match node {
        ClassifiedNode::Value { level, .. } | ClassifiedNode::Container { level, .. } => {
            metrics::counter!("payload_disclosed_values_total", "level" => level.as_str())
                .increment(1);
        }
        ClassifiedNode::Withheld { .. } => {}
    }
    if let ClassifiedNode::Container { children, .. } = node {
        for child in children {
            count_disclosed_nodes(&child.node);
        }
    }
}

fn finish_metrics(started: Instant, result: &Result<PayloadDisclosure, PayloadDisclosureError>) {
    let (outcome, truncated, revealed) = match result {
        Ok(disclosure) => match &disclosure.result {
            PayloadDisclosureResult::Disclosed { projection, .. } => (
                "disclosed",
                projection.truncated,
                disclosure.caller.revealed,
            ),
            PayloadDisclosureResult::StructureOnly { projection, .. } => {
                ("structure_only", projection.truncated, false)
            }
            PayloadDisclosureResult::External { .. } => ("external", false, false),
            PayloadDisclosureResult::Unavailable { .. } => ("unavailable", false, false),
        },
        Err(PayloadDisclosureError::PiiRevealNotGranted) => ("denied", false, false),
        Err(PayloadDisclosureError::Application(_)) => ("error", false, false),
    };
    metrics::counter!("payload_disclosures_total", "outcome" => outcome).increment(1);
    if truncated {
        metrics::counter!("payload_disclosure_truncated_total").increment(1);
    }
    if revealed {
        metrics::counter!("payload_reveal_total", "outcome" => "granted").increment(1);
    }
    metrics::histogram!("payload_disclosure_duration_seconds")
        .record(started.elapsed().as_secs_f64());
    tracing::Span::current().record("outcome", outcome);
    tracing::Span::current().record("truncated", truncated);
    tracing::Span::current().record("revealed", revealed);
}

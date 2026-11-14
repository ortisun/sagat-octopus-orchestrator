//! Classified disclosure of the input of a schedule (feature 018, `FR-018-042`, `T018-033`).
//!
//! The input of a schedule is customer content: an ordinary read of a schedule returns its digest and
//! size, never its values. The values come out only through this read, which projects them under the
//! classification of the input schema of the version the schedule would start now, for the audiences of
//! feature 016 (`public` to every reader, `internal` with `payload:read-internal`, `pii` only when asked
//! for and granted), and is audited in the transaction that reads them (`schedule.input_disclosed`, and `payload.pii_revealed` for a
//! reveal of personal data, which never collapses with another), so that nothing leaves before the record exists. The policy is the one `discloseRunPayload` applies
//! ([`crate::payload_disclosure`]); only the owner of the payload and the permission that reads it differ.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Permission, Principal, Subject, TenantId};
use sagat_domain::schedules::ScheduleId;
use tracing::Instrument;

use crate::classification::ClassificationInterpreter;
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;
use crate::payload_disclosure::{
    DisclosureAudits, DisclosureCaller, DisclosureEvaluator, DisclosureFailure, DisclosureSelector,
    DisclosureSubject, PAYLOAD_DISCLOSURE_POLICY, PayloadDisclosureError, PayloadDisclosureResult,
    ServiceEvaluator,
};
use crate::payloads::PayloadProjector;

/// A trusted request: who asks, for which schedule of which namespace, and whether the PII reveal was
/// asked for. Private fields stop an adapter replacing any of them after the checks.
#[derive(Debug, Clone)]
pub struct ScheduleInputRequest {
    principal: Principal,
    namespace_id: NamespaceId,
    schedule: ScheduleId,
    reveal_pii: bool,
}

impl ScheduleInputRequest {
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.principal.tenant_id()
    }

    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace_id
    }

    #[must_use]
    pub const fn schedule(&self) -> ScheduleId {
        self.schedule
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

/// What was disclosed of the input of a schedule: the revision it belongs to, who saw which levels, and
/// the projection (or the structure, the external reference, the reason there is nothing to show).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleInputDisclosure {
    pub schedule: ScheduleId,
    /// The head revision of the schedule whose input this is.
    pub revision: u32,
    pub caller: DisclosureCaller,
    pub result: PayloadDisclosureResult,
}

/// The closed outcome of the transaction. `Missing` is a schedule that is not in the tenant, not in the
/// namespace or deleted; they are all the same answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleInputRead {
    Missing,
    Refused(DisclosureFailure),
    Ready(ScheduleInputDisclosure),
}

/// Persistence boundary. An implementation resolves the head revision of the schedule and the version it
/// would start now, rechecks the current role and the namespace's reveal grant, calls `evaluator` while
/// its transaction is open, appends the audit the evaluator chose and commits before it returns `Ready`.
/// A reveal denial appends `audits.reveal_denied` before it returns `Refused`.
pub trait ScheduleInputStore: Send + Sync {
    fn disclose_and_audit(
        &self,
        request: &ScheduleInputRequest,
        evaluator: &dyn DisclosureEvaluator,
        audits: &DisclosureAudits,
    ) -> impl Future<Output = Result<ScheduleInputRead, StoreError>> + Send;
}

pub type ScheduleInputFuture<'a> = Pin<
    Box<dyn Future<Output = Result<ScheduleInputDisclosure, PayloadDisclosureError>> + Send + 'a>,
>;

/// What the API calls: one read, authorized, audited and projected.
pub trait ScheduleInputDisclosureUseCase: Send + Sync {
    fn disclose_input<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        schedule: ScheduleId,
        reveal_pii: bool,
        ctx: &'a RequestContext,
    ) -> ScheduleInputFuture<'a>;
}

pub struct ScheduleInputDisclosureService<S, C, I, P> {
    store: S,
    clock: C,
    interpreter: I,
    structure_projector: P,
}

impl<S, C, I, P> fmt::Debug for ScheduleInputDisclosureService<S, C, I, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScheduleInputDisclosureService")
            .finish_non_exhaustive()
    }
}

impl<S, C, I, P> ScheduleInputDisclosureService<S, C, I, P> {
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

/// The labels the audit of a disclosure carries; none of them is a value.
fn reason(suffix: &str) -> String {
    format!("{PAYLOAD_DISCLOSURE_POLICY}:schedule-input{suffix}")
}

fn audits<C: Clock>(
    principal: &Principal,
    schedule: ScheduleId,
    trace_id: Option<String>,
    clock: &C,
) -> DisclosureAudits {
    let record = |action, reason: String| {
        AuditRecord::by(
            principal,
            action,
            ResourceType::Schedule,
            schedule.to_string(),
            trace_id.clone(),
            clock.now(),
        )
        .with_reason(reason)
    };
    DisclosureAudits {
        internal: record(AuditAction::ScheduleInputDisclosed, reason("")),
        // a reveal of personal data is never collapsed with another: it is the 016 record of one
        pii: record(AuditAction::PayloadPiiRevealed, reason(":reveal=pii")),
        inspected: record(AuditAction::ScheduleInputDisclosed, reason(":structure")),
        reveal_denied: record(AuditAction::PayloadRevealDenied, "permission".to_owned()),
    }
}

impl<S, C, I, P> ScheduleInputDisclosureService<S, C, I, P>
where
    S: ScheduleInputStore,
    C: Clock,
    I: ClassificationInterpreter,
    P: PayloadProjector,
{
    async fn disclose_inner(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        schedule: ScheduleId,
        reveal_pii: bool,
        ctx: &RequestContext,
    ) -> Result<ScheduleInputDisclosure, PayloadDisclosureError> {
        let started = Instant::now();
        principal
            .authorize(Permission::ScheduleRead)
            .map_err(AppError::from)?;
        let request = ScheduleInputRequest {
            principal: principal.clone(),
            namespace_id: namespace,
            schedule,
            reveal_pii,
        };
        let audits = audits(principal, schedule, ctx.trace_id.clone(), &self.clock);
        let evaluator = ServiceEvaluator {
            subject: DisclosureSubject {
                reveal_pii,
                // the policy labels its answer with a run; the schedule's identifier stands there and
                // none of the labels reaches the caller of this read
                run_id: RunId::new(schedule.as_uuid()),
                selector: DisclosureSelector::Input,
                read: Permission::ScheduleRead,
            },
            interpreter: &self.interpreter,
            structure_projector: &self.structure_projector,
        };
        let read = self
            .store
            .disclose_and_audit(&request, &evaluator, &audits)
            .await
            .map_err(AppError::from)?;
        let result = match read {
            ScheduleInputRead::Missing => Err(AppError::NotFound.into()),
            ScheduleInputRead::Refused(DisclosureFailure::RunReadNotGranted) => {
                Err(AppError::Forbidden {
                    permission: Permission::ScheduleRead,
                }
                .into())
            }
            ScheduleInputRead::Refused(DisclosureFailure::PiiRevealNotGranted) => {
                metrics::counter!("payload_reveal_total", "outcome" => "denied").increment(1);
                Err(PayloadDisclosureError::PiiRevealNotGranted)
            }
            ScheduleInputRead::Ready(disclosure) => Ok(disclosure),
        };
        metrics::counter!(
            "payload_disclosures_total",
            "outcome" => match &result {
                Ok(disclosure) => match disclosure.result {
                    PayloadDisclosureResult::Disclosed { .. } => "disclosed",
                    PayloadDisclosureResult::StructureOnly { .. } => "structure_only",
                    PayloadDisclosureResult::External { .. } => "external",
                    PayloadDisclosureResult::Unavailable { .. } => "unavailable",
                },
                Err(PayloadDisclosureError::PiiRevealNotGranted) => "denied",
                Err(PayloadDisclosureError::Application(_)) => "error",
            }
        )
        .increment(1);
        metrics::histogram!("payload_disclosure_duration_seconds")
            .record(started.elapsed().as_secs_f64());
        result
    }
}

impl<S, C, I, P> ScheduleInputDisclosureUseCase for ScheduleInputDisclosureService<S, C, I, P>
where
    S: ScheduleInputStore + 'static,
    C: Clock + 'static,
    I: ClassificationInterpreter + 'static,
    P: PayloadProjector + 'static,
{
    fn disclose_input<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        schedule: ScheduleId,
        reveal_pii: bool,
        ctx: &'a RequestContext,
    ) -> ScheduleInputFuture<'a> {
        let span = tracing::info_span!(
            "schedule_input_disclosure",
            policy = PAYLOAD_DISCLOSURE_POLICY,
            tenant_id = %principal.tenant_id(),
            schedule_id = %schedule,
        );
        Box::pin(
            self.disclose_inner(principal, namespace, schedule, reveal_pii, ctx)
                .instrument(span),
        )
    }
}
